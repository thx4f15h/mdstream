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
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect()
}

#[test]
fn rejected_header_lazy_tabs_keep_columns_and_scope() {
    for (first, prefix, spaces) in [
        ("-  - a|b\n", "\t", 1),
        ("- > a|b\n", "\t", 2),
        (">  - a|b\n", ">\t", 2),
        ("> -  > a|b\n", "> \t\t", 3),
    ] {
        for tail in ["中🙂 &amp;", "cell|tail", "-|-", "body!"] {
            let source = format!("{first}{prefix}{tail}\n");
            let doc = parse(&source);
            assert_eq!(
                visible(&doc),
                format!("a|b\n{}{tail}\n", " ".repeat(spaces)).replace("&amp;", "&"),
                "{source:?}"
            );
            assert!(
                doc.tables().is_empty(),
                "lazy cannot confirm or start tables"
            );
            assert_eq!(doc.blocks()[0].container, doc.blocks()[1].container);
            let tab = source.rfind('\t').unwrap();
            let runs: Vec<_> = doc
                .display_runs_in(Span::new(tab, tab + 1))
                .unwrap()
                .collect();
            assert_eq!(runs.len(), 1);
            assert_eq!(runs[0].replacement, Some(Span::new(tab, tab + 1)));
            assert_eq!(runs[0].text, " ".repeat(spaces));
        }
    }
}

#[test]
fn pending_header_lazy_matches_independent_columns() {
    let mut cases = 0;
    for depth in 0..8 {
        let parent = "> ".repeat(depth);
        for marker in ["- ", "-  ", "1. ", "12. "] {
            let target = parent.len() + marker.len();
            for leading in 0..=1 {
                let mut prefix = format!("{parent}{}", " ".repeat(leading));
                let mut column = parent.len() + leading;
                while column < target {
                    prefix.push('\t');
                    column += 4 - column % 4;
                }
                if column == target {
                    continue;
                }
                for omitted in ["> ", "123. ", "> - > "] {
                    let first = format!("{parent}{marker}{omitted}a|b\n");
                    for tail in ["body", "中🙂 &amp;", "cell|tail", "-|-]", "body!"] {
                        for end in ["\n", "\r\n", ""] {
                            let source = format!("{first}{prefix}{tail}{end}");
                            let doc = parse(&source);
                            let model = parse(&format!(
                                "{first}{parent}{}{tail}{end}",
                                " ".repeat(column - parent.len())
                            ));
                            assert_eq!(visible(&doc), visible(&model), "{source:?}");
                            assert!(doc.tables().is_empty());
                            assert_eq!(doc.container_count(), model.container_count());
                            assert_eq!(
                                doc.blocks()[0].container,
                                doc.blocks().last().unwrap().container
                            );
                            assert!(doc.work().container_bytes <= 30 * source.len() as u64);
                            cases += 1;
                        }
                    }
                }
            }
        }
    }
    assert_eq!(cases, 2160);
    eprintln!("{cases} independent column models");
}

#[test]
fn pending_header_lazy_chunks_prefixes_and_eof() {
    let mut splits = 0;
    let mut prefixes = 0;
    for source in [
        "-  - *a|b\n\t中🙂* &amp;\n",
        "- > ` a|b\n\t中 `\r\n",
        ">  - [a|b\n>\t中](u)\n",
        "- > a|b\n\t-|-\n",
        "-  - a|b\n\ttail|cell\n\t-|--\n",
        "-  - a|b\n\t14. body\n",
        "-  - a|b\n\t1. body\n",
        "- > a|b\n\t# heading\n",
        "- > a|b\n\t\n",
        "- > a|b\n\t",
        "- > a|b\n\t---\n",
        "- > a|b\n\t$$\n",
    ] {
        let expected = support::snapshot(&parse(source));
        for split in (0..=source.len()).filter(|&at| source.is_char_boundary(at)) {
            let mut doc = Document::new();
            doc.append(&source[..split]).unwrap();
            doc.append(&source[split..]).unwrap();
            doc.finish();
            assert_eq!(
                support::snapshot(&doc),
                expected,
                "{source:?} split {split}"
            );
            splits += 1;
        }
        let mut doc = Document::new();
        for (at, ch) in source.char_indices() {
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            let prefix = &source[..at + ch.len_utf8()];
            let mut bulk = Document::new();
            bulk.append(prefix).unwrap();
            assert_eq!(support::snapshot(&doc), support::snapshot(&bulk));
            assert_eq!(doc.pending(), bulk.pending());
            let mut eof = Document::new();
            for ch in prefix.chars() {
                eof.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            }
            eof.finish();
            assert_eq!(support::snapshot(&eof), support::snapshot(&parse(prefix)));
            prefixes += 1;
        }
    }
    eprintln!("{splits} splits, {prefixes} prefixes with EOF");
}

#[test]
fn header_fallback_preserves_identity_and_interruption_boundaries() {
    let mut doc = Document::new();
    doc.append("-  - [old](u)\n     *a|b\n").unwrap();
    let first = doc.block_at(0).unwrap();
    let scope = doc.blocks()[0].container;
    let info = *doc.link_info(LinkId(0)).unwrap();
    let header = doc.block_at(doc.source().find('*').unwrap()).unwrap();
    let update = doc.append("\tnew*\n").unwrap();
    assert!(update.new_containers.is_empty());
    assert!(update.closed_containers.is_empty());
    assert_eq!(doc.block_at(0), Some(first));
    assert_eq!(doc.block_at(doc.source().find('*').unwrap()), Some(header));
    assert_eq!(doc.blocks().last().unwrap().container, scope);
    assert_eq!(doc.link_info(LinkId(0)), Some(&info));
    doc.finish();
    assert_eq!(visible(&doc), "old\na|b\n new\n");
    let tab = doc.source().find('\t').unwrap();
    assert!(
        doc.display_runs_in(Span::new(tab, tab + 1))
            .unwrap()
            .all(|r| r.style.contains(Style::EMPHASIS))
    );
    for tail in ["+ sibling\n", "1. sibling\n", "14. sibling\n"] {
        let doc = parse(&format!("-  - a|b\n\t{tail}"));
        assert_ne!(doc.blocks()[0].container, doc.blocks()[1].container);
        assert!(doc.tables().is_empty());
    }
    for tail in [
        "# h\n",
        "---\n",
        "```\n",
        "$$\n",
        "[^f]: note\n",
        "\n",
        "\r\n",
        "",
    ] {
        let doc = parse(&format!("- > a|b\n\t{tail}"));
        let tab = doc.source().find('\t').unwrap();
        assert!(
            doc.display_runs_in(Span::new(tab, tab + 1))
                .unwrap()
                .all(|r| r.replacement.is_none()),
            "{tail:?}"
        );
    }
    assert_eq!(visible(&parse("- > a|b\n\t[r]: /u\n")), "a|b\n  [r]: /u\n");
    // Complete paths still confirm tables; four columns still start code.
    assert_eq!(parse("- a|b\n\t-|-\n").tables().len(), 1);
    let code = parse("- a|b\n\t\ttext\n");
    assert!(
        code.blocks()
            .iter()
            .any(|b| matches!(b.kind, BlockKind::Code { .. }))
    );
}

#[test]
fn slices_tiles_and_shared_six_byte_budget_preserve_tab_mapping() {
    let source = format!("-  - {}|中🙂\n\tordinary body &amp;\n", "a".repeat(240));
    let doc = parse(&source);
    let full = Span::new(0, source.len());
    assert_eq!(
        doc.runs_in(full)
            .unwrap()
            .map(|r| r.text)
            .collect::<String>(),
        source
    );
    let expected = visible(&doc);
    assert!(expected.contains("\n ordinary body &\n"));
    let scalars: String = source
        .char_indices()
        .flat_map(|(at, ch)| {
            doc.display_runs_in(Span::new(at, at + ch.len_utf8()))
                .unwrap()
        })
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    assert_eq!(scalars, expected);
    let tiles: String = (0..doc.tile_count())
        .flat_map(|i| doc.display_runs(TileId(i)).unwrap())
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    assert_eq!(tiles, expected);
    let mut cursor = doc.display_cursor(full).unwrap();
    let mut second = doc.display_cursor(full).unwrap();
    let mut output = String::new();
    loop {
        let position = cursor.source_position();
        if position < full.end {
            assert_eq!(
                cursor
                    .next(&doc, &mut Budget::new(WorkLimit::default()))
                    .unwrap(),
                DisplayStep::BudgetExhausted
            );
            assert_eq!(cursor.source_position(), position);
        }
        let mut budget = Budget::new(WorkLimit {
            bytes: 6,
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        match cursor.next(&doc, &mut budget).unwrap() {
            DisplayStep::Run(run) => {
                assert_eq!(run.source.start, position);
                if !run.style.contains(Style::HIDDEN) {
                    output.push_str(run.text);
                }
                assert_eq!(
                    second.next(&doc, &mut budget).unwrap(),
                    DisplayStep::BudgetExhausted
                );
                assert_eq!(second.source_position(), 0);
            }
            DisplayStep::Complete => break,
            DisplayStep::BudgetExhausted => panic!("one scalar fits"),
        }
        assert!(budget.used().bytes <= 6);
    }
    assert_eq!(output, expected);
}

#[test]
fn header_lazy_probe_work_is_independent_of_missing_depth_and_history() {
    for depth in [1, 100, 480] {
        let mut doc = Document::new();
        doc.append(&format!("-  {}a|b\n", "- ".repeat(depth)))
            .unwrap();
        let before = doc.work();
        doc.append("\tlazy\n").unwrap();
        assert_eq!(doc.container_count(), 2 * (depth + 1));
        assert!(doc.work().container_steps - before.container_steps <= 2);
        assert!(doc.work().container_bytes - before.container_bytes <= 100);
        doc.append(&format!(
            "{}{}|b\n",
            " ".repeat(3 + depth * 2),
            "a".repeat(2048)
        ))
        .unwrap();
        doc.append(&format!("\t{}", "a".repeat(2048))).unwrap();
        let before = doc.work();
        for _ in 0..1000 {
            doc.append(&"a".repeat(1000)).unwrap();
        }
        let after = doc.work();
        assert_eq!(after.container_steps, before.container_steps);
        assert_eq!(after.container_bytes, before.container_bytes);
        assert_eq!(after.classified_bytes, before.classified_bytes);
        assert_eq!(after.inline_bytes - before.inline_bytes, 1_000_000);
        doc.finish();
    }
    let mut doc = Document::new();
    doc.append("-  - a|b\n").unwrap();
    for _ in 0..4096 {
        doc.append("\tlazy\n     a|b\n").unwrap();
    }
    let before = doc.work();
    doc.append("\tlast\n").unwrap();
    assert_eq!(doc.container_count(), 4);
    assert_eq!(doc.work().container_steps - before.container_steps, 2);
    assert!(doc.work().container_bytes - before.container_bytes <= 100);
}
