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

#[test]
fn terminal_raw_whitespace_preserves_inline_content_and_dialect_boundaries() {
    for (source, expected) in [
        ("aaa     \nbbb     \n", "aaa\nbbb\n"),
        ("中🙂 \t \r\n", "中🙂\n"),
        ("a \t ", "a"),
        ("a\\\n", "a\\\n"),
        ("a\\  \n", "a\\\n"),
        ("a &nbsp;  \n", "a \u{a0}\n"),
        ("a &#32;  \n", "a  \n"),
        ("a `b `  \n", "a b \n"),
        ("a `b   \n", "a `b\n"),
        ("a $b   \n", "a $b\n"),
        ("a $b $  \n", "a b \n"),
        ("[a ](u)  \n", "a \n"),
        ("a\u{a0} \n", "a\u{a0}\n"),
        ("a\u{c} \n", "a\u{c}\n"),
        ("a \r", "a \r"),
        ("a \r \t", "a \r"),
        ("a  \n\n", "a\n\n"),
        ("a  \n# h\n", "a\nh\n"),
        ("a  \n> b  \n", "a\nb\n"),
        ("a  \n    b  \n", "a\nb  \n"),
        ("```\na  \n```\n", "a  \n"),
        ("a  \nb  \n===\n", "a\nb\n"),
    ] {
        let doc = parse(source);
        assert_eq!(doc.source(), source);
        assert_eq!(visible(&doc), expected, "{source:?}");
    }
}

#[test]
fn open_tail_waits_for_continuation_or_closure_and_invalidates_caches() {
    let mut doc = Document::new();
    doc.append("a  \n").unwrap();
    assert_eq!(visible(&doc), "a  \n");
    let old_key = doc.tile_key(TileId(0)).unwrap();
    let mut cursor = doc.display_cursor(Span::new(0, 4)).unwrap();
    let update = doc.finish();
    assert_eq!(visible(&doc), "a\n");
    assert_ne!(old_key, doc.tile_key(TileId(0)).unwrap());
    assert!(update.dirty.iter().any(|s| s.start <= 1 && s.end >= 3));
    let mut budget = Budget::new(WorkLimit::unlimited());
    assert_eq!(
        cursor.next(&doc, &mut budget),
        Err(LayoutError::StaleContinuation)
    );
    assert!(doc.finish().dirty.is_empty());
    for (prefix, hard) in [("a  \n", true), ("a\\\r\n", true), ("a \n", false)] {
        let mut doc = Document::new();
        doc.append(prefix).unwrap();
        doc.append("b  \n").unwrap();
        let newline = prefix.len() - 1;
        let style = doc
            .runs_in(Span::new(newline, newline + 1))
            .unwrap()
            .next()
            .unwrap()
            .style;
        assert_eq!(style.contains(Style::HARD_BREAK), hard);
        assert_eq!(style.contains(Style::SOFT_BREAK), !hard);
        doc.finish();
        assert_eq!(visible(&doc), "a\nb\n");
        let last = doc
            .runs_in(Span::new(doc.source().len() - 1, doc.source().len()))
            .unwrap()
            .next()
            .unwrap()
            .style;
        assert!(!last.contains(Style::HARD_BREAK));
        assert!(!last.contains(Style::SOFT_BREAK));
    }
}

#[test]
fn confirmed_early_and_late_headers_seal_only_the_previous_paragraph_tail() {
    for first in ["a  \n", "a\\\n", "`a  \n", "$a  \n", "[a  \n"] {
        for row in [
            "b | c".to_owned(),
            format!("{} | c", "x".repeat(BLOCK_LOOKAHEAD + 17)),
        ] {
            let doc = parse(&format!("{first}{row}\n- | -\n"));
            assert_eq!(doc.tables().len(), 1);
            let paragraph = doc
                .blocks()
                .iter()
                .find(|b| b.kind == BlockKind::Paragraph)
                .unwrap();
            let text: String = doc
                .display_runs_in(paragraph.source)
                .unwrap()
                .filter(|run| !run.style.contains(Style::HIDDEN))
                .map(|run| run.text)
                .collect();
            assert_eq!(
                text,
                format!("{}\n", first.trim_end_matches([' ', '\n'])),
                "{first:?}"
            );
        }
    }
}

#[test]
fn every_split_and_prefix_preserves_tail_and_deferred_header_metadata() {
    let long = "x".repeat(BLOCK_LOOKAHEAD + 17);
    for source in [
        "中🙂 \t \r\n".to_owned(),
        "a\\  \n\n".to_owned(),
        "a &#32; ` b ` $c $  \n".to_owned(),
        "a \r \t".to_owned(),
        "a  \nb  \n===\n".to_owned(),
        format!("a  \n{long} | b\n- | -\n"),
        format!("`a  \n{long} | b\n- | -\n"),
        format!("a  \n{long} | b  \nno delimiter  \n"),
        format!("a  \n{long}  "),
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
fn hidden_tail_keeps_mapping_and_charges_tiny_shared_budgets() {
    let source = format!("{}中🙂{}\r\n", "x".repeat(250), " \t".repeat(150));
    let doc = parse(&source);
    let expected = format!("{}中🙂\n", "x".repeat(250));
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
fn million_byte_tail_growth_and_closure_do_not_rescan_source() {
    let mut doc = Document::new();
    doc.append(&format!("a{}", " ".repeat(1_000_000))).unwrap();
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
    assert!(sealed.tree_visits - grown.tree_visits < 300);
    assert_eq!(visible(&doc), "a");
    let mut doc = Document::new();
    for _ in 0..4096 {
        doc.append("a  \n\n").unwrap();
    }
    doc.finish();
    assert_eq!(visible(&doc), "a\n\n".repeat(4096));
    assert!(doc.work().inline_bytes <= 4096 * 5);
    assert!(doc.work().tree_visits < 4096 * 500);
}
