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
fn non_one_markers_keep_the_innermost_lazy_paragraph() {
    let mut cases = 0;
    for (first, prefix, count) in [
        ("> text\n", "", 1),
        ("> > text\n", "> ", 2),
        ("> - > text\n", ">   ", 4),
        ("- > text\n", "   ", 3),
        ("- > text\n", "  ", 3),
        ("- - > text\n", "    ", 5),
        ("> 12. text\n", "", 3),
    ] {
        for number in ["0", "2", "14", "000", "002", "123456789"] {
            for delimiter in [".", ")"] {
                for gap in [" ", "\t", "     "] {
                    for end in ["\n", "\r\n", ""] {
                        let payload = format!("{number}{delimiter}{gap}body{end}");
                        let source = format!("{first}{prefix}{payload}");
                        let doc = parse(&source);
                        assert_eq!(doc.container_count(), count, "{source:?}");
                        assert_eq!(
                            doc.blocks().last().unwrap().container,
                            doc.blocks()[0].container
                        );
                        assert!(
                            visible(&doc).contains(&format!("{number}{delimiter}{gap}body")),
                            "{source:?}"
                        );
                        assert!(
                            !doc.blocks()
                                .iter()
                                .any(|b| matches!(b.kind, BlockKind::Code { .. }))
                        );
                        cases += 1;
                    }
                }
            }
        }
    }
    assert_eq!(cases, 756);
}

#[test]
fn interrupting_markers_siblings_and_nonparagraphs_still_close_paths() {
    for (source, count) in [
        ("> text\n1. body\n", 3),
        ("> text\n001) body\n", 3),
        ("> text\n- body\n", 3),
        ("> text\n+ body\n", 3),
        ("> > text\n> 1. body\n", 4),
        ("1. text\n14. body\n", 3),
        ("0) text\n0) body\n", 3),
        ("- text\n14. body\n", 4),
        ("- text\n+ body\n", 4),
        ("> text\n\n14. body\n", 3),
        ("> # title\n14. body\n", 3),
        ("> text\n> > 14. body\n", 4),
    ] {
        assert_eq!(parse(source).container_count(), count, "{source:?}");
    }
    for first in [
        ">\n",
        "> # h\n",
        "> ---\n",
        "> [r]: /u\n",
        "> a|b\n> -|-\n",
        "> ```\n",
        "> text\n>\n",
    ] {
        let source = format!("{first}14. body\n");
        let doc = parse(&source);
        assert_ne!(
            doc.blocks().last().unwrap().container,
            doc.blocks()[0].container,
            "{source:?}"
        );
        assert_eq!(
            doc.container(ContainerId(0)).unwrap().source.end,
            first.len()
        );
    }
    for marker in ["1.", "001)", "*", "+"] {
        let doc = parse(&format!("> text\n{marker}\n"));
        assert_eq!(doc.container_count(), 1);
    }
}

#[test]
fn interruption_is_chunk_prefix_and_eof_invariant() {
    let cases = [
        "> text\n14. 中🙂 &amp;\n",
        "> > *text\n> 2) body*\r\n",
        "- [text\n  > quote\n002. body](u)\n",
        "> text\n14. body\n",
        "> text\n001) body\n",
        "> text\n2.\t",
        "- text\n2.     body\n",
        "> text|col\n2. body\n",
        "> text\n2.body\n",
        "> text\n1234567890. body\n",
        "> - > text\n> 14. body\n",
        "- > text\n\t14. body\n",
    ];
    for source in cases {
        let expected = support::snapshot(&parse(source));
        for split in (0..=source.len()).filter(|&at| source.is_char_boundary(at)) {
            let mut doc = Document::new();
            doc.append(&source[..split]).unwrap();
            doc.append(&source[split..]).unwrap();
            doc.finish();
            assert_eq!(
                support::snapshot(&doc),
                expected,
                "split {split}: {source:?}"
            );
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
        }
    }
}

#[test]
fn lazy_paragraph_preserves_identity_inline_and_table_fallback() {
    let mut doc = Document::new();
    doc.append("> *[text](u)\n").unwrap();
    let old = doc.block_at(0).unwrap();
    let scope = doc.blocks()[0].container;
    let update = doc.append("14. body*\n").unwrap();
    assert!(update.new_containers.is_empty());
    assert!(update.closed_containers.is_empty());
    assert_eq!(doc.blocks().last().unwrap().container, scope);
    assert_eq!(doc.block_at(0), Some(old));
    assert_eq!(doc.link_destination(LinkId(0)), Some("u"));
    assert!(
        doc.runs_in(Span::new(0, doc.source().len()))
            .unwrap()
            .filter(|r| r.text.contains("body"))
            .all(|r| r.style.contains(Style::EMPHASIS))
    );
    doc.finish();
    assert_eq!(visible(&doc), "text\n14. body\n");

    for source in ["> a|b\n14. body\n", "> > a|b\n> 14. body\n"] {
        let doc = parse(source);
        assert!(doc.tables().is_empty());
        assert!(visible(&doc).contains("14. body"));
        assert_eq!(
            doc.blocks().last().unwrap().container,
            doc.blocks()[0].container
        );
    }
}

#[test]
fn noninterrupting_text_keeps_source_mapping_and_shared_budget() {
    let source = format!("> > {}中🙂\r\n>\t14. body &amp;\n", "a".repeat(240));
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
    let tab = source.find('\t').unwrap();
    let mapped: Vec<_> = doc
        .display_runs_in(Span::new(tab, tab + 1))
        .unwrap()
        .collect();
    assert_eq!(mapped.len(), 1);
    assert_eq!(mapped[0].source, Span::new(tab, tab + 1));
    // The matched outer quote consumes one tab column before the missing quote.
    assert_eq!(mapped[0].replacement, Some(mapped[0].source));
    assert_eq!(mapped[0].text, "  ");
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
fn bounded_marker_decision_never_revisits_long_body_or_history() {
    let prefix = ">".repeat(970);
    let mut doc = Document::new();
    doc.append(&format!("{prefix} text\n")).unwrap();
    doc.append(&format!("14. {}", "a".repeat(2048))).unwrap();
    assert_eq!(doc.container_count(), 970);
    let before = doc.work();
    for _ in 0..1000 {
        doc.append(&"x".repeat(1000)).unwrap();
    }
    let after = doc.work();
    assert_eq!(after.container_bytes, before.container_bytes);
    assert_eq!(after.container_steps, before.container_steps);
    assert_eq!(after.classified_bytes, before.classified_bytes);
    assert_eq!(after.inline_bytes - before.inline_bytes, 1_000_000);
    doc.finish();
    let mut doc = Document::new();
    doc.append("> text\n").unwrap();
    for _ in 0..4096 {
        doc.append("14. body\n").unwrap();
    }
    let before = doc.work();
    doc.append("14. body\n").unwrap();
    assert_eq!(doc.container_count(), 1);
    assert!(doc.work().container_bytes - before.container_bytes <= 100);
    assert!(doc.work().container_steps - before.container_steps <= 8);
}

#[test]
fn number_decision_is_final_at_the_bounded_window_and_lazy_depth_is_constant() {
    for number in ["1", "14"] {
        for tail in ["", "body"] {
            let source = format!(
                "> text\n{number}. {}{tail}\n",
                " ".repeat(BLOCK_LOOKAHEAD * 2)
            );
            let whole = parse(&source);
            assert_eq!(whole.container_count(), if number == "1" { 3 } else { 1 });
            for split in [
                7,
                8,
                9,
                BLOCK_LOOKAHEAD + 6,
                BLOCK_LOOKAHEAD + 7,
                source.len(),
            ] {
                let mut doc = Document::new();
                doc.append(&source[..split]).unwrap();
                doc.append(&source[split..]).unwrap();
                doc.finish();
                assert_eq!(support::snapshot(&doc), support::snapshot(&whole));
            }
        }
    }
    for depth in [1, 100, 970] {
        let mut doc = Document::new();
        doc.append(&format!("{} text\n", ">".repeat(depth)))
            .unwrap();
        let before = doc.work();
        for _ in 0..1000 {
            let update = doc.append("14. body\n").unwrap();
            assert!(update.new_containers.is_empty());
            assert!(update.closed_containers.is_empty());
        }
        assert_eq!(doc.container_count(), depth);
        assert!(doc.work().container_steps - before.container_steps <= 3000);
        assert!(doc.work().container_bytes - before.container_bytes <= 100_000);
        doc.finish();
    }
}
