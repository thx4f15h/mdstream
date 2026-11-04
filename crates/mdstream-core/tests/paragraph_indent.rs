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
fn root_indentation_preserves_source_inline_semantics_and_dialect_boundaries() {
    for (source, expected) in [
        ("  中🙂\r\n bbb", "中🙂\nbbb"),
        ("   a  b\t", "a  b"),
        ("  *a\n b* &amp;\n", "a\nb &\n"),
        ("  a  \n   b\n", "a\nb\n"),
        ("  `a\n   b`\n", "a b\n"),
        ("  ` a\n   `\n", "a\n"),
        ("  `a\n   b\n", "`a\nb\n"),
        ("  [a\n b](u)\n", "a\nb\n"),
        ("  $a\n b$\n", "a\nb\n"),
        ("  aaa\n bbb\n===\n", "aaa\nbbb\n"),
        ("    a\n    b\n", "a\nb\n"),
        ("a\n     b\n", "a\n b\n"),
        ("```\n   a\n```\n", "   a\n"),
        ("\ta\n", "a\n"),
        (" \ta\n", "a\n"),
        (">   a\n>  b\n", "  a\n b\n"),
        ("  a\n\n  b\n", "a\n\nb\n"),
    ] {
        let doc = parse(source);
        assert_eq!(doc.source(), source);
        assert_eq!(visible(&doc), expected, "{source:?}");
    }
    let doc = parse("  $a\n b$\n");
    let mut cursor = doc.math_payload(MathId(0)).unwrap();
    let mut budget = Budget::new(WorkLimit::unlimited());
    let mut payload = String::new();
    while let Some(chunk) = cursor.advance(&mut budget).unwrap() {
        payload.push_str(chunk.text);
    }
    assert_eq!(payload, "a\nb");
}

#[test]
fn every_split_and_prefix_preserves_metadata_including_deferred_headers() {
    let long = "x".repeat(BLOCK_LOOKAHEAD + 17);
    let cases = [
        "  中🙂\r\n bbb".to_owned(),
        "  *a\n b* &amp;\n".to_owned(),
        "  ` a\n   `\n".to_owned(),
        "  [a\n b](u)\n".to_owned(),
        "  $a\n b$\n".to_owned(),
        "  a | b\nnot a delimiter\n".to_owned(),
        "  a | b\n- | -\n".to_owned(),
        "  aaa\n bbb\n===\n".to_owned(),
        format!("  {long} | b\n- | -\n"),
        format!("  {long} | b\nno delimiter\n"),
        format!("`a\n  {long}` | b\n- | -\n"),
        format!("$a\n  {long}$ | b\nno delimiter\n"),
        format!("  {long}\n b"),
    ];
    for source in cases {
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
fn classification_boundary_and_failed_header_hide_indentation_once() {
    let mut doc = Document::new();
    doc.append("  中").unwrap();
    assert_eq!(doc.pending(), Span::new(0, 5));
    assert_eq!(visible(&doc), "  中");
    let old_key = doc.tile_key(TileId(0)).unwrap();
    let mut old_cursor = doc.display_cursor(Span::new(0, 5)).unwrap();
    doc.append("\n").unwrap();
    assert!(doc.pending().is_empty());
    assert_eq!(visible(&doc), "中\n");
    assert_ne!(doc.tile_key(TileId(0)).unwrap(), old_key);
    let mut budget = Budget::new(WorkLimit::unlimited());
    assert_eq!(
        old_cursor.next(&doc, &mut budget),
        Err(LayoutError::StaleContinuation)
    );
    let indent = doc
        .display_runs_in(Span::new(0, 2))
        .unwrap()
        .next()
        .unwrap();
    assert_eq!(indent.text, "  ");
    assert!(indent.style.contains(Style::HIDDEN));

    let long = "x".repeat(BLOCK_LOOKAHEAD + 17);
    for first in ["", "*a\n", "`a\n", "$a\n", "[a\n"] {
        let mut doc = Document::new();
        doc.append(&format!("{first}  {long}")).unwrap();
        assert!(doc.pending().is_empty());
        assert!(!visible(&doc).contains("  "));
        doc.append(" | b\nno delimiter\n").unwrap();
        doc.finish();
        assert!(doc.tables().is_empty());
        assert!(!visible(&doc).contains("  "));
    }
    for row in ["a | b".to_owned(), format!("{long} | b")] {
        let doc = parse(&format!("  {row}\n- | -\n"));
        let model = parse(&format!("{row}\n- | -\n"));
        assert_eq!(doc.tables().len(), 1);
        assert_eq!(visible(&doc), visible(&model));
    }
}

#[test]
fn hidden_indent_is_charged_and_maps_through_tiles_and_tiny_budgets() {
    let source = format!("  {}\r\n   中🙂\n", "x".repeat(250));
    let doc = parse(&source);
    let expected = format!("{}\n中🙂\n", "x".repeat(250));
    let tiled: String = (0..doc.tile_count())
        .flat_map(|i| doc.display_runs(TileId(i)).unwrap())
        .filter(|run| !run.style.contains(Style::HIDDEN))
        .map(|run| run.text)
        .collect();
    assert_eq!(tiled, expected);
    let mut cursor = doc.display_cursor(Span::new(0, source.len())).unwrap();
    let mut output = String::new();
    let mut hidden_bytes = 0;
    loop {
        let position = cursor.source_position();
        if position < source.len() {
            let mut empty = Budget::new(WorkLimit::default());
            assert_eq!(
                cursor.next(&doc, &mut empty).unwrap(),
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
                    hidden_bytes += run.source.len();
                } else {
                    output.push_str(run.text);
                }
            }
            DisplayStep::Complete => break,
            other => panic!("six bytes fit a scalar and lookaround: {other:?}"),
        }
    }
    assert_eq!(hidden_bytes, 6); // Five indent spaces and the CR byte.
    assert_eq!(output, expected);
}

#[test]
fn million_byte_growth_and_many_indented_lines_keep_bounded_work() {
    let mut doc = Document::new();
    doc.append(&format!("  {}", "x".repeat(1_000_000))).unwrap();
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("x").unwrap();
    }
    assert_eq!(doc.work().inline_bytes - before.inline_bytes, 1000);
    assert_eq!(doc.work().classified_bytes, before.classified_bytes);
    assert_eq!(doc.work().container_bytes, before.container_bytes);
    assert_eq!(doc.work().container_steps, before.container_steps);
    assert_eq!(doc.work().tree_visits, before.tree_visits);
    doc.append("\n").unwrap();
    let before = doc.work();
    for _ in 0..4096 {
        doc.append("   a\n").unwrap();
    }
    assert_eq!(doc.work().inline_bytes - before.inline_bytes, 4096 * 5);
    assert!(doc.work().classified_bytes - before.classified_bytes <= 4096 * 5);
    assert!(doc.work().tree_visits - before.tree_visits < 4096 * 500);
    doc.finish();
    assert_eq!(doc.blocks().len(), 1);
    assert_eq!(
        visible(&doc),
        format!("{}\n{}", "x".repeat(1_001_000), "a\n".repeat(4096))
    );
}
