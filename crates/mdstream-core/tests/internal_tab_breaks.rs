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
fn mixed_whitespace_model_preserves_hard_break_rule_and_scopes() {
    // All binary space/tab strings up to length six; only the immediate
    // ASCII-space suffix determines a space-induced hard break.
    for len in 1..=6 {
        for mask in 0..1 << len {
            let whitespace: String = (0..len)
                .map(|bit| if mask & (1 << bit) == 0 { ' ' } else { '\t' })
                .collect();
            for (prefix, continuation) in [("", ""), ("> ", "> "), ("- ", "  "), ("> ", "")] {
                for ending in ["\n", "\r\n"] {
                    let source = format!("{prefix}a{whitespace}{ending}{continuation}b\n");
                    let doc = parse(&source);
                    assert_eq!(visible(&doc), "a\nb\n", "{source:?}");
                    assert_eq!(doc.source(), source);
                    let style = style_at(&doc, source.find('\n').unwrap());
                    let hard = whitespace.ends_with("  ");
                    assert_eq!(style.contains(Style::HARD_BREAK), hard, "{source:?}");
                    assert_eq!(style.contains(Style::SOFT_BREAK), !hard, "{source:?}");
                }
            }
        }
    }
}

#[test]
fn only_confirmed_ordinary_breaks_trim_raw_whitespace() {
    for (source, expected) in [
        ("a\t b\n", "a\t b\n"),
        ("a\t\rb\n", "a\t\rb\n"),
        ("a\t\r \nb\n", "a\t\r\nb\n"),
        ("a\\\t\nb\n", "a\\\nb\n"),
        ("a\t\\\nb\n", "a\t\nb\n"),
        ("a&#9;\t\nb\n", "a\t\nb\n"),
        ("`a\t\nb`\n", "a\t b\n"),
        ("$a\t\nb$\n", "a\t\nb\n"),
        ("```\na\t\nb\n```\n", "a\t\nb\n"),
        ("a\t\nb\n===\n", "a\nb\n"),
        ("> a\t\n# h\n", "a\nh\n"),
    ] {
        assert_eq!(visible(&parse(source)), expected, "{source:?}");
    }
    let mut doc = Document::new();
    doc.append("a \t\r\n").unwrap();
    assert_eq!(visible(&doc), "a \t\n");
    assert!(!style_at(&doc, 4).contains(Style::SOFT_BREAK));
    let old_key = doc.tile_key(TileId(0)).unwrap();
    let mut cursor = doc.display_cursor(Span::new(0, 5)).unwrap();
    let update = doc.append("b\n").unwrap();
    assert_eq!(visible(&doc), "a\nb\n");
    assert!(
        update
            .dirty
            .iter()
            .any(|span| span.start <= 1 && span.end >= 3)
    );
    assert_ne!(old_key, doc.tile_key(TileId(0)).unwrap());
    assert_eq!(
        cursor.next(&doc, &mut Budget::new(WorkLimit::unlimited())),
        Err(LayoutError::StaleContinuation)
    );
}

#[test]
fn every_split_and_prefix_including_deferred_headers() {
    let long = "x".repeat(BLOCK_LOOKAHEAD + 17);
    for source in [
        "中🙂 \t\r\nb\n".to_owned(),
        "> *a\t \n> b*\n".to_owned(),
        "- a \t  \r\n  b\n".to_owned(),
        "a\t\rb\t\nend\n".to_owned(),
        "[a\t\nb](u)\n".to_owned(),
        "a \t\nb\n===\n".to_owned(),
        format!("a \t\n{long} | b\n- | -\n"),
        format!("a \t\n{long} | b\nno delimiter\n"),
        format!("a \t\n{long}"),
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
fn late_header_success_discards_break_and_failure_commits_it() {
    for row in [
        "b | c".to_owned(),
        format!("{} | c", "x".repeat(BLOCK_LOOKAHEAD + 17)),
    ] {
        for (suffix, table) in [("- | -\n", true), ("no delimiter\n", false)] {
            let doc = parse(&format!("a \t\n{row}\n{suffix}"));
            assert_eq!(doc.tables().len(), usize::from(table));
            assert!(style_at(&doc, 1).contains(Style::HIDDEN));
            assert!(style_at(&doc, 2).contains(Style::HIDDEN));
            assert_eq!(style_at(&doc, 3).contains(Style::SOFT_BREAK), !table);
            assert!(!style_at(&doc, 3).contains(Style::HARD_BREAK));
        }
    }
}

#[test]
fn hidden_tabs_preserve_source_mapping_and_shared_budget() {
    let source = format!("{}中🙂{}\r\nb\n", "x".repeat(250), " \t".repeat(150));
    let doc = parse(&source);
    let expected = format!("{}中🙂\nb\n", "x".repeat(250));
    let tiled: String = (0..doc.tile_count())
        .flat_map(|i| doc.display_runs(TileId(i)).unwrap())
        .filter(|run| !run.style.contains(Style::HIDDEN))
        .map(|run| run.text)
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
    assert_eq!(hidden, 301);
    assert_eq!(output, expected);
}

#[test]
fn million_byte_mixed_tail_and_many_breaks_do_not_rescan() {
    let mut doc = Document::new();
    doc.append(&format!("a{}", " \t".repeat(500_000))).unwrap();
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("\t").unwrap();
    }
    let grown = doc.work();
    assert_eq!(grown.inline_bytes - before.inline_bytes, 1000);
    assert_eq!(grown.classified_bytes, before.classified_bytes);
    assert_eq!(grown.container_bytes, before.container_bytes);
    assert_eq!(grown.tree_visits, before.tree_visits);
    doc.append("\r\nb\n").unwrap();
    doc.finish();
    let sealed = doc.work();
    assert_eq!(sealed.inline_bytes - grown.inline_bytes, 4);
    assert!(sealed.classified_bytes - grown.classified_bytes < 100);
    assert!(sealed.tree_visits - grown.tree_visits < 500);
    assert_eq!(visible(&doc), "a\nb\n");
    let mut doc = Document::new();
    doc.append(&"a \t\n".repeat(4096)).unwrap();
    doc.finish();
    assert_eq!(visible(&doc), "a\n".repeat(4096));
    assert!(doc.work().inline_bytes <= 4096 * 4);
    assert!(doc.work().tree_visits < 4096 * 500);
}
