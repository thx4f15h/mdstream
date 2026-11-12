use mdstream::*;

#[path = "support/snapshot.rs"]
mod support;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn visible(doc: &Document) -> String {
    doc.display_runs_in(Span::new(0, doc.source().len()))
        .unwrap()
        .filter(|run| !run.style.contains(Style::HIDDEN))
        .map(|run| run.text)
        .collect()
}

#[test]
fn long_delimiters_match_expanded_physical_columns() {
    let mut cases = 0;
    for outer in ["", "> ", ">> ", "   > "] {
        for marker in ["- ", "1. "] {
            for depth in 1..=6 {
                let opening = format!("{outer}{}a|b\n", marker.repeat(depth));
                let target = outer.len() + marker.len() * depth;
                for leading in 0..4 {
                    let mut col = outer.len() + leading;
                    let mut prefix = format!("{outer}{}", " ".repeat(leading));
                    while col < target {
                        prefix.push('\t');
                        col = (col / 4 + 1) * 4;
                    }
                    let expanded = format!("{outer}{}", " ".repeat(col - outer.len()));
                    for (body, supported) in [
                        (format!("{}|-", "-".repeat(1100)), true),
                        (format!("- | :{}:", "-".repeat(1100)), true),
                        (
                            format!("|:{}|{}:|", "-".repeat(1100), "-".repeat(1100)),
                            true,
                        ),
                        (format!("{}x|-", "-".repeat(1100)), false),
                        (format!("{}|-|-", "-".repeat(1100)), false),
                    ] {
                        let source = format!("{opening}{prefix}{body}\n");
                        let doc = parse(&source);
                        let model = parse(&format!("{opening}{expanded}{body}\n"));
                        assert_eq!(doc.tables().len(), usize::from(supported), "{source:?}");
                        assert_eq!(doc.tables().len(), model.tables().len());
                        assert_eq!(visible(&doc), visible(&model));
                        assert_eq!(doc.container_count(), model.container_count());
                        for i in 0..doc.container_count() {
                            let a = doc.container(ContainerId(i)).unwrap();
                            let b = model.container(ContainerId(i)).unwrap();
                            assert_eq!((a.kind, a.parent, a.depth), (b.kind, b.parent, b.depth));
                        }
                        if supported {
                            let table = &doc.tables()[0];
                            assert_eq!(table.columns, model.tables()[0].columns);
                            assert_eq!(table.delimiter, Span::new(opening.len(), source.len()));
                            assert_eq!(table.block, BlockId(0));
                        }
                        assert!(doc.work().container_bytes < 20 * source.len() as u64);
                        assert!(doc.work().classified_bytes < 12 * source.len() as u64);
                        cases += 1;
                    }
                }
            }
        }
    }
    assert_eq!(cases, 960);
}

#[test]
fn late_success_and_failure_preserve_identity_and_recover_once() {
    for tail in ["|-\n", "|-", "x|-\n", "|-|-\n", "", "|:"] {
        let mut doc = Document::new();
        doc.append("- [old](u)|b\n").unwrap();
        let scope = doc.blocks()[0].container;
        doc.append(&format!("\t{}", "-".repeat(2048))).unwrap();
        assert!(doc.pending().len() <= BLOCK_LOOKAHEAD + 3);
        let update = doc.append(tail).unwrap();
        doc.finish();
        let success = tail == "|-\n" || tail == "|-";
        assert_eq!(doc.tables().len(), usize::from(success));
        assert!(update.closed_containers.is_empty());
        assert_eq!(doc.blocks()[0].container, scope);
        assert_eq!(doc.link_destination(LinkId(0)), Some("u"));
        if success {
            assert_eq!(doc.tables()[0].block, BlockId(0));
            assert_eq!(visible(&doc), "oldb\n");
        } else {
            if tail.is_empty() {
                assert_eq!(doc.headings().entries().len(), 1);
                assert_eq!(visible(&doc), "old|b\n");
            } else {
                assert!(visible(&doc).contains(&format!("  {}", "-".repeat(2048))));
            }
        }
        assert!(doc.work().inline_bytes <= doc.source().len() as u64);
    }
    let mut doc = Document::new();
    doc.append(&format!("- *old|b\n\t{}x", "-".repeat(2048)))
        .unwrap();
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("-").unwrap();
    }
    assert_eq!(
        doc.work().table_candidate_bytes,
        before.table_candidate_bytes
    );
    assert_eq!(doc.work().classified_bytes, before.classified_bytes);
    assert_eq!(doc.work().container_steps, before.container_steps);
    assert_eq!(doc.work().inline_bytes - before.inline_bytes, 1000);
    doc.append("*\n\tc|d\n\t-| -\n").unwrap();
    doc.finish();
    assert_eq!(doc.tables().len(), 1);
    assert_eq!(
        doc.text(doc.tables()[0].cell(0, 0).unwrap().source),
        Some("c")
    );
}

#[test]
fn long_partial_lines_are_chunk_and_eof_invariant() {
    for (opening, prefix, tail) in [
        ("- *中🙂*|[a](u)\r\n", "\t", "|-:\r\n\tc|d\r\n"),
        ("- a|b\n", "\t", "|-"),
        ("- a|b\n", "\t- | ", "x\n\tc|d\n\t-|-\n"),
        ("- a|b\n", "\t", "|:"),
        ("- > a|b\n", "\t", "|-\n"),
    ] {
        let source = format!("{opening}{prefix}{}{tail}", "-".repeat(1030));
        let expected = support::snapshot(&parse(&source));
        for split in (0..=source.len()).filter(|&at| source.is_char_boundary(at)) {
            let mut doc = Document::new();
            doc.append(&source[..split]).unwrap();
            doc.append(&source[split..]).unwrap();
            doc.finish();
            assert_eq!(support::snapshot(&doc), expected, "split {split}");
        }
        let mut doc = Document::new();
        for (at, ch) in source.char_indices() {
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            let prefix = &source[..at + ch.len_utf8()];
            let mut bulk = Document::new();
            bulk.append(prefix).unwrap();
            assert_eq!(
                support::snapshot(&doc),
                support::snapshot(&bulk),
                "prefix {at}"
            );
            assert_eq!(doc.pending(), bulk.pending());
            // Check EOF on both sides of the classification boundary and tail.
            if (1018..=1032).contains(&prefix.len()) || prefix.len() > source.len() - 12 {
                let mut eof = Document::new();
                for ch in prefix.chars() {
                    eof.append(ch.encode_utf8(&mut [0; 4])).unwrap();
                }
                eof.finish();
                assert_eq!(support::snapshot(&eof), support::snapshot(&parse(prefix)));
            }
        }
    }
}

#[test]
fn partial_long_line_mapping_and_shared_budget() {
    for tail in ["|-\n\t&amp;|`x`\n", "x|-\n", "|:"] {
        let source = format!("- *中🙂*|[a](u)\n\t{}{tail}", "-".repeat(1100));
        let doc = parse(&source);
        let full = Span::new(0, source.len());
        assert_eq!(
            doc.runs_in(full)
                .unwrap()
                .map(|r| r.text)
                .collect::<String>(),
            source
        );
        let scalars: String = source
            .char_indices()
            .flat_map(|(at, ch)| {
                doc.display_runs_in(Span::new(at, at + ch.len_utf8()))
                    .unwrap()
            })
            .filter(|r| !r.style.contains(Style::HIDDEN))
            .map(|r| r.text)
            .collect();
        let tiled: String = (0..doc.tile_count())
            .flat_map(|i| doc.display_runs(TileId(i)).unwrap())
            .filter(|r| !r.style.contains(Style::HIDDEN))
            .map(|r| r.text)
            .collect();
        assert_eq!(scalars, visible(&doc));
        assert_eq!(tiled, scalars);
        let tab = source.find('\t').unwrap();
        let shown: String = doc
            .display_runs_in(Span::new(tab, tab + 1))
            .unwrap()
            .filter(|r| !r.style.contains(Style::HIDDEN))
            .map(|r| r.text)
            .collect();
        assert_eq!(shown, if tail.starts_with("|-\n") { "" } else { "  " });
        let mut cursor = doc.display_cursor(full).unwrap();
        let mut second = doc.display_cursor(full).unwrap();
        let mut output = String::new();
        loop {
            let position = cursor.source_position();
            let mut empty = Budget::new(WorkLimit::default());
            if position < source.len() {
                assert!(matches!(
                    cursor.next(&doc, &mut empty).unwrap(),
                    DisplayStep::BudgetExhausted
                ));
                assert_eq!(cursor.source_position(), position);
            }
            let mut budget = Budget::new(WorkLimit {
                queries: 1,
                objects: 1,
                bytes: 6,
                ..Default::default()
            });
            match cursor.next(&doc, &mut budget).unwrap() {
                DisplayStep::Run(run) => {
                    assert_eq!(run.source.start, position);
                    if !run.style.contains(Style::HIDDEN) {
                        output.push_str(run.text);
                    }
                    assert!(matches!(
                        second.next(&doc, &mut budget).unwrap(),
                        DisplayStep::BudgetExhausted
                    ));
                    assert_eq!(second.source_position(), 0);
                }
                DisplayStep::Complete => break,
                DisplayStep::BudgetExhausted => panic!("one scalar fits"),
            }
            assert!(budget.used().bytes <= 6);
        }
        assert_eq!(output, visible(&doc));
    }
}

#[test]
fn incomplete_paths_siblings_and_four_column_partial_indent_remain_excluded() {
    for (header, prefix) in [
        ("- > a|b\n", "\t"),
        ("- a|b\n", "- "),
        ("- a|b\n", "\t  "),
        ("- a|b\n", "\t\t"),
        ("- a|b\n", "\t> "),
        ("- parent\n\t- a|b\n", "\t"),
    ] {
        let source = format!("{header}{prefix}{}|-\n", "-".repeat(1100));
        assert!(parse(&source).tables().is_empty(), "{source:?}");
    }
    // Preserve the old byte-aligned long-probe whitespace dialect.
    for prefix in ["    ", ">\t    "] {
        let header = if prefix.starts_with('>') {
            "> a|b\n"
        } else {
            "a|b\n"
        };
        assert_eq!(
            parse(&format!("{header}{prefix}{}|-\n", "-".repeat(1100)))
                .tables()
                .len(),
            1
        );
    }
}

#[test]
fn deep_million_byte_probe_only_consumes_new_candidate_bytes() {
    let outer = ">".repeat(980);
    for success in [true, false] {
        let mut doc = Document::new();
        doc.append(&format!(
            "{outer}- [old](u)|b\n{outer}\t{}",
            "-".repeat(2048)
        ))
        .unwrap();
        let scope = doc.blocks()[0].container;
        let before = doc.work();
        for _ in 0..1000 {
            doc.append(&"-".repeat(1000)).unwrap();
            assert!(doc.pending().len() <= BLOCK_LOOKAHEAD + 3);
        }
        let after = doc.work();
        assert_eq!(
            after.table_candidate_bytes - before.table_candidate_bytes,
            1_000_000
        );
        assert_eq!(after.container_steps, before.container_steps);
        assert_eq!(after.container_bytes, before.container_bytes);
        assert_eq!(after.classified_bytes, before.classified_bytes);
        assert_eq!(after.inline_bytes, before.inline_bytes);
        assert_eq!(after.heading_steps, before.heading_steps);
        let update = doc.append(if success { "|-\n" } else { "|-x\n" }).unwrap();
        assert_eq!(update.new_tables, 0..usize::from(success));
        assert!(update.closed_containers.is_empty());
        assert_eq!(doc.blocks()[0].container, scope);
        assert_eq!(doc.link_destination(LinkId(0)), Some("u"));
        if success {
            assert_eq!(doc.work().container_steps, after.container_steps);
            assert_eq!(doc.tables()[0].block, BlockId(0));
        }
        doc.finish();
        assert_eq!(doc.container_count(), 982);
        assert!(doc.work().inline_bytes <= doc.source().len() as u64);
        assert!(doc.work().classified_bytes < 12 * doc.source().len() as u64);
    }
}
