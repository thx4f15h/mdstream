use mdstream::*;

#[path = "support/snapshot.rs"]
mod snapshot;

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

fn style_at(doc: &Document, at: usize) -> Style {
    doc.runs_in(Span::new(at, at + 1))
        .unwrap()
        .next()
        .unwrap()
        .style
}

#[test]
fn container_closure_model_trims_only_raw_paragraph_tails() {
    for len in 1..=6 {
        for mask in 0..1 << len {
            let tail: String = (0..len)
                .map(|bit| if mask & (1 << bit) == 0 { ' ' } else { '\t' })
                .collect();
            for prefix in ["> ", "- ", "> - ", "- > "] {
                for end in ["", "\n", "\r\n"] {
                    let source = format!("{prefix}中🙂{tail}{end}");
                    let doc = parse(&source);
                    assert_eq!(doc.source(), source);
                    assert_eq!(
                        visible(&doc),
                        if end.is_empty() {
                            "中🙂"
                        } else {
                            "中🙂\n"
                        },
                        "{source:?}"
                    );
                    if !end.is_empty() {
                        let style = style_at(&doc, source.len() - 1);
                        assert!(!style.contains(Style::HARD_BREAK));
                        assert!(!style.contains(Style::SOFT_BREAK));
                    }
                }
            }
        }
    }
    for (source, expected) in [
        ("> a  \n> b \t\n", "a\nb\n"),
        ("> a  \nb \t\n", "a\nb\n"),
        ("- a  \n- b \t\n", "a\nb\n"),
        ("- [x] a \t\n", "a\n"),
        ("- a\n\tb \t\n", "a\n  b\n"),
        ("> a  \n>\n> b \t\n", "a\n\nb\n"),
        ("> a  \n# h\n", "a\nh\n"),
        ("> a\\ \t\n", "a\\\n"),
        ("> a&#32; \t\n", "a \n"),
        ("> `a ` \t\n", "a \n"),
        ("> $a $ \t\n", "a \n"),
        ("> [a ](u) \t\n", "a \n"),
        ("> `a \t\n", "`a\n"),
        ("> $a \t\n", "$a\n"),
        ("> a\u{a0}\u{c} \t\n", "a\u{a0}\u{c}\n"),
        ("> a \r", "a \r"),
        ("> ```\n> a \t\n> ```\n", "a \t\n"),
        (">     a \t\n", "a \t\n"),
        ("> a \t\n> ===\n", "a\n"),
        ("> a\n> b \t\n> ===\n", "a\nb\n"),
    ] {
        assert_eq!(visible(&parse(source)), expected, "{source:?}");
    }
}

#[test]
fn open_container_tail_waits_for_logical_closure() {
    for (prefix, continuation) in [("> ", "> "), ("- ", "  "), ("> ", "")] {
        let mut doc = Document::new();
        doc.append(&format!("{prefix}a  \n")).unwrap();
        assert_eq!(visible(&doc), "a  \n");
        doc.append(&format!("{continuation}b \t\n")).unwrap();
        assert_eq!(visible(&doc), "a\nb \t\n");
        assert!(style_at(&doc, prefix.len() + 3).contains(Style::HARD_BREAK));
        let blocks = doc.blocks().to_vec();
        let key = doc.tile_key(TileId(0)).unwrap();
        let mut cursor = doc
            .display_cursor(Span::new(0, doc.source().len()))
            .unwrap();
        let tail = Span::new(doc.source().len() - 3, doc.source().len() - 1);
        let update = doc.finish();
        assert_eq!(visible(&doc), "a\nb\n");
        assert_ne!(key, doc.tile_key(TileId(0)).unwrap());
        assert!(
            update
                .dirty
                .iter()
                .any(|s| s.start <= tail.start && s.end >= tail.end)
        );
        assert_eq!(
            cursor.next(&doc, &mut Budget::new(WorkLimit::unlimited())),
            Err(LayoutError::StaleContinuation)
        );
        for (old, new) in blocks.iter().zip(doc.blocks()) {
            assert_eq!(
                (old.source, &old.kind, old.container),
                (new.source, &new.kind, new.container)
            );
        }
        assert!(doc.finish().dirty.is_empty());
    }
    let mut doc = Document::new();
    doc.append("> [a ](u) $b $ \t\n").unwrap();
    let links = doc.links().to_vec();
    let maths = doc.maths().to_vec();
    doc.finish();
    assert_eq!(doc.links(), links);
    assert_eq!(doc.maths(), maths);
    assert_eq!(visible(&doc), "a  b \n");
}

#[test]
fn early_and_late_container_tables_seal_previous_tail_only_on_success() {
    for first in ["a \t\n", "`a \t\n", "$a \t\n", "[a \t\n"] {
        for row in [
            "b | c".to_owned(),
            format!("{} | c", "x".repeat(BLOCK_LOOKAHEAD + 17)),
        ] {
            for success in [false, true] {
                let source = format!(
                    "> {first}> {row}\n> {}\n",
                    if success { "- | -" } else { "no delimiter" }
                );
                let doc = parse(&source);
                assert_eq!(doc.tables().len(), usize::from(success));
                let first_end = 2 + first.len();
                let text: String = doc
                    .display_runs_in(Span::new(0, first_end))
                    .unwrap()
                    .filter(|r| !r.style.contains(Style::HIDDEN))
                    .map(|r| r.text)
                    .collect();
                // Failed opaque candidates retain their existing internal-break
                // fallback. Successful tables always terminate the old paragraph.
                let expected = if success || matches!(first.as_bytes()[0], b'a' | b'[') {
                    format!("{}\n", first.trim_end_matches([' ', '\t', '\n']))
                } else {
                    first.to_owned()
                };
                assert_eq!(text, expected, "{source:?}");
                if success {
                    let first_cell = doc.tables()[0].cell(0, 0).unwrap();
                    let displayed: String = doc
                        .display_runs_in(first_cell.source)
                        .unwrap()
                        .filter(|r| !r.style.contains(Style::HIDDEN))
                        .map(|r| r.text)
                        .collect();
                    assert_eq!(
                        displayed,
                        if row.starts_with('b') {
                            "b".to_owned()
                        } else {
                            "x".repeat(BLOCK_LOOKAHEAD + 17)
                        }
                    );
                }
            }
        }
    }
}

#[test]
fn every_split_and_prefix_preserves_scopes_tables_and_objects() {
    let long = "x".repeat(BLOCK_LOOKAHEAD + 17);
    for source in [
        "> - 中🙂 \t\r\n>   next \t\r\n".to_owned(),
        "> a  \nlazy \t\n# h\n".to_owned(),
        "- a \t\n- b \t".to_owned(),
        "> a \t\n>\n> `b ` [c ](u) \t\n".to_owned(),
        "> a\n> b \t\n> ===\n".to_owned(),
        "> a \t\n> b | c\n> - | -\n".to_owned(),
        "> a \t\n> b | c\n> no delimiter \t\n".to_owned(),
        format!("> `a \t\n> {long} | c\n> - | -\n"),
        format!("> a \t\n> {long} | c\n> no delimiter \t\n"),
        format!("> a \t\n> {long} \t"),
    ] {
        let whole = snapshot::snapshot(&parse(&source));
        for split in (0..=source.len()).filter(|&at| source.is_char_boundary(at)) {
            let mut doc = Document::new();
            doc.append(&source[..split]).unwrap();
            doc.append(&source[split..]).unwrap();
            doc.finish();
            assert_eq!(snapshot::snapshot(&doc), whole, "split {split}: {source:?}");
        }
        let mut doc = Document::new();
        for (at, ch) in source.char_indices() {
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            let mut prefix = Document::new();
            prefix.append(&source[..at + ch.len_utf8()]).unwrap();
            assert_eq!(doc.pending(), prefix.pending());
            assert_eq!(snapshot::snapshot(&doc), snapshot::snapshot(&prefix));
        }
        doc.finish();
        assert_eq!(snapshot::snapshot(&doc), whole);
    }
}

#[test]
fn hidden_container_tail_preserves_mapping_and_shared_budget_progress() {
    let source = format!("> {}中🙂{}\r\n", "x".repeat(250), " \t".repeat(150));
    let doc = parse(&source);
    let expected = format!("{}中🙂\n", "x".repeat(250));
    let tiled: String = (0..doc.tile_count())
        .flat_map(|i| doc.display_runs(TileId(i)).unwrap())
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    assert_eq!(tiled, expected);
    let mut cursor = doc.display_cursor(Span::new(0, source.len())).unwrap();
    let mut output = String::new();
    let mut hidden = 0;
    loop {
        let position = cursor.source_position();
        if position < source.len() {
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
                assert_eq!(run.replacement, None);
                assert_eq!(run.text, &source[run.source.range()]);
                assert!(budget.used().bytes >= run.source.len() + 2);
                assert!(budget.used().bytes <= 6);
                if run.style.contains(Style::HIDDEN) {
                    hidden += run.source.len();
                } else {
                    output.push_str(run.text);
                }
            }
            DisplayStep::Complete => break,
            other => panic!("six bytes must advance: {other:?}"),
        }
    }
    assert_eq!(hidden, 303);
    assert_eq!(output, expected);
}

#[test]
fn deep_million_byte_tail_and_many_closures_have_bounded_work() {
    let mut doc = Document::new();
    doc.append(&format!("{}a{}", ">".repeat(1000), " \t".repeat(500_000)))
        .unwrap();
    let before = doc.work();
    for _ in 0..1000 {
        doc.append(" ").unwrap();
    }
    let grown = doc.work();
    assert_eq!(grown.inline_bytes - before.inline_bytes, 1000);
    assert_eq!(grown.classified_bytes, before.classified_bytes);
    assert_eq!(grown.container_bytes, before.container_bytes);
    assert_eq!(grown.tree_visits, before.tree_visits);
    doc.finish();
    let sealed = doc.work();
    assert_eq!(sealed.inline_bytes, grown.inline_bytes);
    assert_eq!(sealed.classified_bytes, grown.classified_bytes);
    assert_eq!(sealed.container_bytes, grown.container_bytes);
    assert!(sealed.tree_visits - grown.tree_visits < 300);
    assert!(visible(&doc) == "a", "million-byte tail must be hidden");
    let mut doc = Document::new();
    for _ in 0..4096 {
        doc.append("- a \t\n").unwrap();
    }
    doc.finish();
    assert_eq!(visible(&doc), "a\n".repeat(4096));
    assert_eq!(doc.work().paragraph_closures, 4096);
    assert!(doc.work().inline_bytes <= 4096 * 5);
    assert!(doc.work().tree_visits < 4096 * 500);
}
