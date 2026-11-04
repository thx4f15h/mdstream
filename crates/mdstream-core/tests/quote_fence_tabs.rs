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
fn new_quote_separator_uses_one_column_before_fence() {
    let mut cases = 0;
    for prefix in ["", " ", "  ", "   ", "> ", ">> ", "- ", "1. "] {
        for whitespace in ["", " ", "  ", "   ", "\t", " \t", "\t ", "\t\t"] {
            let column = prefix.len() + 1;
            let after_tab = column + 4 - column % 4;
            let mut end = after_tab;
            for byte in whitespace.bytes() {
                end += if byte == b'\t' { 4 - end % 4 } else { 1 };
            }
            let indent = end - (column + 1);
            for marker in ["```", "~~~~", "~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~"] {
                for ending in ["\n", "\r\n", ""] {
                    let source = format!("{prefix}>\t{whitespace}{marker} rs &amp;{ending}");
                    let doc = parse(&source);
                    let code = doc.blocks().iter().find_map(|block| match block.kind {
                        BlockKind::Code { info: Some(info) } => Some(info),
                        _ => None,
                    });
                    assert_eq!(code.is_some(), indent <= 3, "{source:?}, indent {indent}");
                    if let Some(info) = code {
                        assert_eq!(doc.text(info), Some("rs &amp;"));
                        assert_eq!(visible(&doc), "");
                    }
                    assert_eq!(doc.source(), source);
                    cases += 1;
                }
            }
        }
    }
    assert_eq!(cases, 576);
    // The same tab supplies a separator and two columns of opener indent.
    for (opener, expected) in [(">\t~~~", "  中🙂\t\n"), (">\t ~~~", " 中🙂\t\n")] {
        let source = format!("{opener}\n>     中🙂\t\n> ~~~\n");
        let doc = parse(&source);
        assert_eq!(visible(&doc), expected, "{source:?}");
        assert_eq!(doc.container_count(), 1);
        assert_eq!(doc.blocks().len(), 1);
        assert_eq!(doc.blocks()[0].container, Some(ContainerId(0)));
    }
}

#[test]
fn failure_other_leaves_and_window_keep_their_dialect() {
    for (source, expected) in [
        (">\t ~~ rs\n", "   ~~ rs\n"),
        (">\t ```bad`\n", "   ```bad`\n"),
        (">\t # title\n", "title\n"),
        (">\t $$\n", ""), // A confirmed math delimiter is hidden.
        (">\t ordinary\n", "   ordinary\n"),
    ] {
        assert_eq!(visible(&parse(source)), expected, "{source:?}");
    }
    for length in [
        BLOCK_LOOKAHEAD - 5,
        BLOCK_LOOKAHEAD - 4,
        BLOCK_LOOKAHEAD - 3,
        2048,
    ] {
        let source = format!(">\t {}\n> x\n", "~".repeat(length));
        let doc = parse(&source);
        assert_eq!(
            doc.blocks()
                .iter()
                .any(|b| matches!(b.kind, BlockKind::Code { .. })),
            length + 3 < BLOCK_LOOKAHEAD
        );
    }
    let doc = parse(">\t ~~~\n# outside\n");
    assert_eq!(doc.headings().entries().len(), 1);
    assert_eq!(doc.blocks()[0].container, Some(ContainerId(0)));
    assert_eq!(doc.blocks()[1].container, None);
}

#[test]
fn all_splits_and_prefixes_preserve_scopes_and_identity() {
    for source in [
        ">\t ~~~ rs\r\n>     中🙂\r\n> ~~~\r\n> # after\n",
        "- >\t~~~\n  >     x\n  > ~~~\n- sibling\n",
        "> seed\n> >\t ~~~\n> >     x\n> > ~~~\n",
        "- seed\n\t>\t ~~~\n  >    x\n  > ~~~",
        ">\t ```bad`info\n> ordinary\n",
        ">\t  ~~~\n",
        ">\t ~~~",
    ] {
        let whole = parse(source);
        for split in (0..=source.len()).filter(|&n| source.is_char_boundary(n)) {
            let mut doc = Document::new();
            doc.append(&source[..split]).unwrap();
            doc.append(&source[split..]).unwrap();
            doc.finish();
            assert_eq!(
                support::snapshot(&doc),
                support::snapshot(&whole),
                "{source:?} at {split}"
            );
        }
        let mut doc = Document::new();
        for (start, ch) in source.char_indices() {
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            let mut prefix = Document::new();
            prefix.append(&source[..start + ch.len_utf8()]).unwrap();
            assert_eq!(support::snapshot(&doc), support::snapshot(&prefix));
            assert_eq!(doc.pending(), prefix.pending());
        }
        doc.finish();
        assert_eq!(support::snapshot(&doc), support::snapshot(&whole));
    }
}

#[test]
fn source_slices_tiles_and_shared_six_byte_budget() {
    let source = ">\t ~~~ rs\r\n>     中🙂\t\r\n> ~~~\n";
    let doc = parse(source);
    assert_eq!(visible(&doc), " 中🙂\t\n");
    let mut end = 0;
    for run in doc.display_runs_in(Span::new(0, source.len())).unwrap() {
        assert_eq!(run.source.start, end);
        assert_eq!(run.text, &source[run.source.range()]);
        assert_eq!(run.replacement, None);
        end = run.source.end;
    }
    assert_eq!(end, source.len());
    let sliced: String = source
        .char_indices()
        .flat_map(|(at, ch)| {
            doc.display_runs_in(Span::new(at, at + ch.len_utf8()))
                .unwrap()
        })
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    assert_eq!(sliced, visible(&doc));
    let tiled: String = (0..doc.tile_count())
        .flat_map(|i| doc.display_runs(TileId(i)).unwrap())
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    assert_eq!(tiled, sliced);
    let mut cursor = doc.display_cursor(Span::new(0, source.len())).unwrap();
    let mut output = String::new();
    while cursor.source_position() < source.len() {
        let old = cursor.source_position();
        assert_eq!(
            cursor
                .next(&doc, &mut Budget::new(WorkLimit::default()))
                .unwrap(),
            DisplayStep::BudgetExhausted
        );
        assert_eq!(cursor.source_position(), old);
        let mut budget = Budget::new(WorkLimit {
            bytes: 6,
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        let DisplayStep::Run(run) = cursor.next(&doc, &mut budget).unwrap() else {
            panic!("must advance")
        };
        assert!(budget.used().bytes <= 6);
        assert!(budget.used().queries <= 1 && budget.used().objects <= 1);
        assert!(cursor.source_position() > old);
        if !run.style.contains(Style::HIDDEN) {
            output.push_str(run.text);
        }
    }
    assert_eq!(output, sliced);
}

#[test]
fn deep_million_byte_body_does_not_rescan_prefix_or_history() {
    let ancestors = ">".repeat(988);
    let prefix = format!("{ancestors}> ");
    let mut doc = Document::new();
    doc.append(&format!(
        "{ancestors}>\t ~~~\n{prefix}{}",
        "x".repeat(1_000_000)
    ))
    .unwrap();
    assert!(matches!(doc.blocks()[0].kind, BlockKind::Code { .. }));
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("x").unwrap();
    }
    let after = doc.work();
    assert_eq!(after.input_bytes - before.input_bytes, 1000);
    assert_eq!(after.classified_bytes, before.classified_bytes);
    assert_eq!(after.container_bytes, before.container_bytes);
    assert_eq!(after.container_steps, before.container_steps);
    assert_eq!(after.inline_bytes, before.inline_bytes);
    assert_eq!(after.tree_visits, before.tree_visits);
    doc.append(&format!("\n{prefix}~~~\n")).unwrap();
    doc.finish();
    assert_eq!(doc.container_count(), 989);
    assert_eq!(visible(&doc), format!("{}\n", "x".repeat(1_001_000)));
    assert!(doc.work().container_bytes < 80 * BLOCK_LOOKAHEAD as u64);
}
