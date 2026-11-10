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
fn missing_item_keeps_consumed_columns_and_original_scope() {
    for (first, prefix, spaces, depth) in [
        ("-  - first\n", "\t", 1, 0),
        ("1. - first\n", "\t", 1, 0),
        ("- - - first\n", "\t", 0, 0),
        (">  - first\n", ">\t", 2, 1),
        (" >  - first\n", " >\t", 1, 1),
        ("-  - > first\n", "\t", 1, 0),
        ("> -  123. first\n", "> \t\t", 3, 1),
    ] {
        let source = format!("{first}{prefix}中🙂 &amp;\n");
        let doc = parse(&source);
        assert_eq!(
            visible(&doc),
            format!("first\n{}中🙂 &\n", " ".repeat(spaces)),
            "{source:?}"
        );
        assert_eq!(doc.blocks()[0].container, doc.blocks()[1].container);
        assert_eq!(
            doc.blocks()[1].kind,
            if depth == 0 {
                BlockKind::Paragraph
            } else {
                BlockKind::Quote { depth }
            }
        );
        let tab = source.rfind('\t').unwrap();
        let runs: Vec<_> = doc
            .display_runs_in(Span::new(tab, tab + 1))
            .unwrap()
            .collect();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].source, Span::new(tab, tab + 1));
        if spaces > 0 {
            assert_eq!(runs[0].replacement, Some(runs[0].source));
            assert_eq!(runs[0].text, " ".repeat(spaces));
        } else {
            assert!(runs[0].style.contains(Style::HIDDEN));
        }
    }
}

#[test]
fn missing_items_match_independent_physical_column_model() {
    let mut cases = 0;
    for parent_depth in 0..8 {
        let parent = "> ".repeat(parent_depth);
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
                for missing in [1, 2, 5] {
                    // A five-column child cannot be satisfied by the tab's 1..3 residue.
                    let first = format!("{parent}{marker}123. {}first\n", "- ".repeat(missing - 1));
                    for tail in ["body", "中🙂 &amp;", "[x](u)", "*body*"] {
                        for end in ["\n", "\r\n", ""] {
                            let doc = parse(&format!("{first}{prefix}{tail}{end}"));
                            let model = parse(&format!(
                                "{first}{parent}{}{tail}{end}",
                                " ".repeat(column - parent.len())
                            ));
                            assert_eq!(
                                visible(&doc),
                                visible(&model),
                                "{first:?} {prefix:?} {tail:?}"
                            );
                            assert_eq!(doc.container_count(), model.container_count());
                            assert_eq!(
                                doc.blocks().last().unwrap().container,
                                doc.blocks()[0].container
                            );
                            cases += 1;
                        }
                    }
                }
            }
        }
    }
    assert_eq!(cases, 1728);
    eprintln!("{cases} independent column models");
}

#[test]
fn lazy_tab_mapping_is_chunk_prefix_and_eof_invariant() {
    let mut splits = 0;
    let mut prefixes = 0;
    for source in [
        ">  - *first\n>\t中🙂* &amp;\n",
        "-  - ` first\n\t中 `\r\n",
        "-  - $first\n\t中$\r\n",
        "-  - first\n\t1.\n",
        "-  - first\n\t\ttail\n",
        "1. - [first\n\t中](u)\n",
        ">  - first\n>\t14. body\n",
        ">  - first\n>\t1. body\n",
        ">  - first\n>\t\n",
        ">  - first\n>\t",
        ">  - first\n>\t# heading\n",
        ">  - a|b\n>\ttail\n",
        "-  - first\n\ttail|cell\n\t-|--\n",
    ] {
        let expected = support::snapshot(&parse(source));
        for split in (0..=source.len()).filter(|&at| source.is_char_boundary(at)) {
            splits += 1;
            let mut doc = Document::new();
            doc.append(&source[..split]).unwrap();
            doc.append(&source[split..]).unwrap();
            doc.finish();
            assert_eq!(
                support::snapshot(&doc),
                expected,
                "{source:?} split {split}"
            );
        }
        let mut doc = Document::new();
        for (at, ch) in source.char_indices() {
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            prefixes += 1;
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
        }
    }
    eprintln!("{splits} splits and {prefixes} prefixes with EOF");
}

#[test]
fn siblings_and_nonparagraph_boundaries_keep_existing_rules() {
    // Same/different marker siblings take precedence over lazy fallback,
    // including non-one ordered markers; do not retain the old deepest Item.
    for tail in [
        "- sibling\n",
        "+ sibling\n",
        "1. sibling\n",
        "14. sibling\n",
    ] {
        let doc = parse(&format!("-  - old\n\t{tail}"));
        assert_ne!(
            doc.blocks()[0].container,
            doc.blocks()[1].container,
            "{tail:?}"
        );
        assert_eq!(visible(&doc), "old\nsibling\n");
    }
    // A rejected header now retains the partial-tab lazy mapping too.
    assert_eq!(visible(&parse("-  - a|b\n\ttext\n")), "a|b\n text\n");
    for first in ["-  - # h\n", "-  - old\n\n"] {
        let source = format!("{first}\ttext\n");
        let doc = parse(&source);
        let tab = source.find('\t').unwrap();
        assert!(
            doc.display_runs_in(Span::new(tab, tab + 1))
                .unwrap()
                .all(|r| r.replacement.is_none()),
            "{first:?}"
        );
    }
    for tail in ["# h\n", "---\n", "```\n", "$$\n", "[^f]: note\n"] {
        let doc = parse(&format!("-  - old\n\t{tail}"));
        let tab = doc.source().find('\t').unwrap();
        assert!(
            doc.display_runs_in(Span::new(tab, tab + 1))
                .unwrap()
                .all(|r| r.replacement.is_none()),
            "{tail:?}"
        );
    }
    assert_eq!(visible(&parse("-  - old\n\t[r]: /u\n")), "old\n r: /u\n");
    for tail in ["\n", "\r\n", ""] {
        let source = format!("-  - old\n\t{tail}");
        let doc = parse(&source);
        let tab = source.find('\t').unwrap();
        assert!(
            doc.display_runs_in(Span::new(tab, tab + 1))
                .unwrap()
                .all(|r| r.replacement.is_none())
        );
    }
}

#[test]
fn lazy_mapping_preserves_inline_objects_identity_and_notifications() {
    let mut doc = Document::new();
    doc.append("-  - *[old](u)\n").unwrap();
    let block = doc.block_at(0).unwrap();
    let scope = doc.blocks()[0].container;
    let info = *doc.link_info(LinkId(0)).unwrap();
    let update = doc.append("\tnew*\n").unwrap();
    assert!(update.new_containers.is_empty());
    assert!(update.closed_containers.is_empty());
    assert_eq!(doc.block_at(0), Some(block));
    assert_eq!(doc.blocks().last().unwrap().container, scope);
    assert_eq!(doc.link_info(LinkId(0)), Some(&info));
    assert_eq!(doc.link_destination(LinkId(0)), Some("u"));
    let tab = doc.source().find('\t').unwrap();
    let run = doc
        .display_runs_in(Span::new(tab, tab + 1))
        .unwrap()
        .next()
        .unwrap();
    assert!(run.style.contains(Style::EMPHASIS));
    assert_eq!(run.text, " ");
    doc.finish();
    assert_eq!(visible(&doc), "old\n new\n");
    for (source, expected) in [
        ("-  - ` first\n\t中 `\n", "first  中\n"),
        ("-  - [first\n\t中](u)\n", "first\n 中\n"),
        (
            "-  - first\n\tcell|tail\n\t-|--\n",
            "first\n cell|tail\n -|--\n",
        ),
    ] {
        let doc = parse(source);
        assert_eq!(visible(&doc), expected);
        assert!(doc.tables().is_empty());
    }
}

#[test]
fn slices_tiles_and_shared_six_byte_budget_preserve_tab_mapping() {
    let source = format!("-  - {}中🙂\n\tordinary body &amp;\n", "a".repeat(240));
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
fn lazy_probe_never_visits_omitted_items_or_old_body() {
    for depth in [1, 100, 480] {
        let mut doc = Document::new();
        doc.append(&format!("-  {}first\n", "- ".repeat(depth)))
            .unwrap();
        let before = doc.work();
        for _ in 0..1000 {
            doc.append("\tlazy\n").unwrap();
        }
        assert_eq!(doc.container_count(), 2 * (depth + 1));
        assert!(doc.work().container_steps - before.container_steps <= 2000);
        assert!(doc.work().container_bytes - before.container_bytes <= 100_000);
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
    doc.append("-  - first\n").unwrap();
    for _ in 0..4096 {
        doc.append("\tlazy\n").unwrap();
    }
    let before = doc.work();
    doc.append("\tlast\n").unwrap();
    assert_eq!(doc.container_count(), 4);
    assert_eq!(doc.work().container_steps - before.container_steps, 2);
    assert!(doc.work().container_bytes - before.container_bytes <= 100);
}
