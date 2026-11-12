use mdstream::*;

#[path = "support/snapshot.rs"]
mod support;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn visible(doc: &Document, span: Span) -> String {
    doc.display_runs_in(span)
        .unwrap()
        .filter(|run| !run.style.contains(Style::HIDDEN))
        .map(|run| run.text)
        .collect()
}

fn code(doc: &Document) -> &Block {
    let mut blocks = doc
        .blocks()
        .iter()
        .filter(|b| matches!(b.kind, BlockKind::Code { .. }));
    let block = blocks.next().unwrap();
    assert!(blocks.next().is_none());
    block
}

#[test]
fn fence_indent_matches_independent_space_model_in_existing_scopes() {
    let mut cases = 0;
    for (before, prefix) in [
        ("", ""),
        ("", "> "),
        ("", "> > "),
        ("- item\n", "  "),
        ("> - item\n", ">   "),
    ] {
        for indent in 0..=3 {
            for body_indent in 0_usize..=6 {
                for marker in ["```", "~~~~"] {
                    for ending in ["\n", "\r\n", ""] {
                        let source = format!(
                            "{before}{prefix}{}{marker} lang other\n{prefix}{}中🙂 *x* &amp;\t  {ending}",
                            " ".repeat(indent),
                            " ".repeat(body_indent)
                        );
                        let doc = parse(&source);
                        let block = code(&doc);
                        let expected = format!(
                            "{}中🙂 *x* &amp;\t  {}",
                            " ".repeat(body_indent.saturating_sub(indent)),
                            if ending.is_empty() { "" } else { "\n" }
                        );
                        assert_eq!(visible(&doc, block.source), expected, "{source:?}");
                        assert_eq!(doc.source(), source);
                        assert_eq!(block.state, SyntaxState::Sealed);
                        let BlockKind::Code { info: Some(info) } = block.kind else {
                            panic!("missing info")
                        };
                        assert_eq!(doc.text(info), Some("lang other"));
                        assert!(doc.links().is_empty());
                        assert!(doc.maths().is_empty());
                        cases += 1;
                    }
                }
            }
        }
    }
    assert_eq!(cases, 840);
}

#[test]
fn fence_indent_preserves_blank_lines_closers_and_math_dialect() {
    for (source, expected) in [
        ("   ```\n\n \n  \r\n    \n```\n", "\n\n\n \n"),
        (
            "  ~~~~\n  ~~~\n  ```\n  ~~~~ bad\n    ~~~~\n ~~~~~\n",
            "~~~\n```\n~~~~ bad\n  ~~~~\n",
        ),
        (
            "   ```\n\tx\n \tx\n  \tx\n   \tx\n```\n",
            " x\n x\n x\n\tx\n",
        ),
        (">   ```\n>   x\n", "x\n"),
        ("- x\n  ```\n\ty\n  ```\n", "  y\n"),
        ("- x\n\t```\n\ty\n  ```\n", "y\n"),
        ("- x\n   ```\n\ty\n  ```\n", " y\n"),
        ("- x\n\t ```\n\t  y\n  ```\n", " y\n"),
        ("- x\n\t```\n  y\n  ```\n", "y\n"),
        ("   ```\n   x\n```\n```\n   y\n```\n", "x\n   y\n"),
    ] {
        let doc = parse(source);
        let actual: String = doc
            .blocks()
            .iter()
            .filter(|b| matches!(b.kind, BlockKind::Code { .. }))
            .map(|b| visible(&doc, b.source))
            .collect();
        assert_eq!(actual, expected, "{source:?}");
    }
    let doc = parse("  $$\n   x\n $$\n");
    assert_eq!(visible(&doc, Span::new(0, doc.source().len())), "   x\n");
    let doc = parse("    x\n    y\n");
    assert_eq!(visible(&doc, code(&doc).source), "x\ny\n");
    // Missing scope seals the first fence; indentation is local to each opener.
    let doc = parse(">   ```\n>    x\n  ~~~\n   y\n~~~\n");
    assert_eq!(doc.blocks().len(), 2);
    assert_eq!(visible(&doc, doc.blocks()[0].source), " x\n");
    assert_eq!(visible(&doc, doc.blocks()[1].source), " y\n");
}

#[test]
fn fence_indent_every_split_prefix_and_eof_preserves_metadata() {
    for source in [
        "   ```rs\r\n  中🙂\r\n    \n \n```\n",
        "> - item\n>     ~~~\n>      中🙂\n>   ~~~~",
        "- x\n\t ```\n\t  y\n  ```\n",
        "   ```\n   *x* &amp; [x](u) $x$\n    ```\n  ```",
        "   ```\n \t中🙂",
        ">   ```\n>    x\n  ~~~\n   y\n~~~\n",
    ] {
        let whole = support::snapshot(&parse(source));
        for split in (0..=source.len()).filter(|&at| source.is_char_boundary(at)) {
            let mut doc = Document::new();
            doc.append(&source[..split]).unwrap();
            doc.append(&source[split..]).unwrap();
            doc.finish();
            assert_eq!(support::snapshot(&doc), whole, "{source:?} split {split}");
        }
        let mut doc = Document::new();
        for (at, ch) in source.char_indices() {
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            let mut prefix = Document::new();
            prefix.append(&source[..at + ch.len_utf8()]).unwrap();
            assert_eq!(support::snapshot(&doc), support::snapshot(&prefix));
            assert_eq!(doc.pending(), prefix.pending());
            let mut eof = Document::new();
            for scalar in source[..at + ch.len_utf8()].chars() {
                eof.append(scalar.encode_utf8(&mut [0; 4])).unwrap();
            }
            eof.finish();
            assert_eq!(
                support::snapshot(&eof),
                support::snapshot(&parse(&source[..at + ch.len_utf8()]))
            );
        }
        doc.finish();
        assert_eq!(support::snapshot(&doc), whole);
    }
}

#[test]
fn fence_indent_pending_confirmation_invalidates_cache_and_keeps_identity() {
    let mut doc = Document::new();
    doc.append("  ```rs\n  中🙂").unwrap();
    let block = doc.blocks()[0].clone();
    assert_eq!(visible(&doc, Span::new(0, doc.source().len())), "  中🙂");
    let key = doc.tile_key(TileId(0)).unwrap();
    let mut cursor = doc
        .display_cursor(Span::new(0, doc.source().len()))
        .unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let fold = FoldId::Code(BlockId(0));
    view.toggle(&doc, fold).unwrap();
    let ticket = view.heights().ticket(TileId(0)).unwrap();
    let update = doc.append("\n").unwrap();
    view.apply(&doc, &update).unwrap();
    assert!(update.new_blocks.is_empty());
    assert!(update.changed_blocks.contains(&0));
    assert_eq!(doc.blocks()[0].kind, block.kind);
    assert_eq!(doc.blocks()[0].container, block.container);
    assert!(view.is_collapsed(fold));
    assert!(view.measure(ticket, 5).is_err());
    assert_ne!(doc.tile_key(TileId(0)).unwrap(), key);
    assert_eq!(
        cursor.next(&doc, &mut Budget::new(WorkLimit::unlimited())),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(visible(&doc, doc.blocks()[0].source), "中🙂\n");
    doc.append("```").unwrap();
    doc.finish();
    assert_eq!(doc.blocks().len(), 1);
    assert_eq!(visible(&doc, doc.blocks()[0].source), "中🙂\n");
    assert!(doc.finish().dirty.is_empty());
}

#[test]
fn fence_indent_mapping_and_hidden_bytes_share_tiny_display_budget() {
    for source in [
        format!("   ```\n   {}\r\n  中🙂\n```\n", "x".repeat(250)),
        "- x\n   ```\n\t中🙂\n  ```\n".to_owned(),
    ] {
        let doc = parse(&source);
        let span = code(&doc).source;
        let expected = visible(&doc, span);
        let mut cursor = doc.display_cursor(span).unwrap();
        let mut output = String::new();
        let mut covered = span.start;
        loop {
            let position = cursor.source_position();
            if position < span.end {
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
                    assert_eq!(run.source.start, covered);
                    covered = run.source.end;
                    assert!(budget.used().bytes <= 6);
                    assert!(budget.used().bytes >= run.source.len() + 2);
                    if let Some(original) = run.replacement {
                        assert_eq!(&source[original.range()], "\t");
                        assert_eq!(run.text, " ");
                    } else {
                        assert_eq!(run.text, &source[run.source.range()]);
                    }
                    if !run.style.contains(Style::HIDDEN) {
                        output.push_str(run.text);
                    }
                }
                DisplayStep::Complete => break,
                other => panic!("six bytes must advance: {other:?}"),
            }
        }
        assert_eq!(covered, span.end);
        assert_eq!(output, expected);
        let tiled: String = (0..doc.tile_count())
            .flat_map(|i| doc.display_runs(TileId(i)).unwrap())
            .filter(|r| !r.style.contains(Style::HIDDEN) && r.source.start >= span.start)
            .map(|r| r.text)
            .collect();
        assert_eq!(tiled, expected);
    }
}

#[test]
fn fence_indent_crosses_classification_limit_without_rescanning_ancestors() {
    // The opener fits the window, but spelling the same path with seven
    // optional spaces leaves only one payload space in the body's window.
    let opener = format!("{}    ```\n", ">".repeat(1015));
    let prefix = format!("{}{}", "> ".repeat(7), ">".repeat(1008));
    let source = format!("{opener}{prefix}    中🙂\n");
    let whole = parse(&source);
    assert_eq!(whole.container_count(), 1015);
    assert_eq!(visible(&whole, code(&whole).source), "中🙂\n");
    for split in opener.len() + BLOCK_LOOKAHEAD - 2..=opener.len() + BLOCK_LOOKAHEAD + 2 {
        let mut doc = Document::new();
        doc.append(&source[..split]).unwrap();
        for ch in source[split..].chars() {
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
        }
        doc.finish();
        assert_eq!(support::snapshot(&doc), support::snapshot(&whole));
    }
    let mut doc = Document::new();
    doc.append(&format!("{opener}{prefix}  ")).unwrap();
    assert!(!doc.pending().is_empty());
    let before = doc.work();
    for _ in 0..2 {
        doc.append(" ").unwrap();
    }
    assert_eq!(doc.work().container_steps, before.container_steps);
    assert_eq!(doc.work().container_bytes, before.container_bytes);
    assert!(doc.work().classified_bytes - before.classified_bytes <= 2);
    doc.append("x\n").unwrap();
    doc.finish();
    assert_eq!(visible(&doc, code(&doc).source), "x\n");
}

#[test]
fn fence_indent_million_byte_append_and_many_lines_keep_bounded_work() {
    let prefix = ">".repeat(1000);
    let mut doc = Document::new();
    doc.append(&format!(
        "{prefix}    ```\n{prefix}    {}",
        "x".repeat(1_000_000)
    ))
    .unwrap();
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
    doc.append("\n").unwrap();
    let before = doc.work();
    doc.finish();
    assert_eq!(doc.work().classified_bytes, before.classified_bytes);
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert_eq!(
        visible(&doc, code(&doc).source),
        format!("{}\n", "x".repeat(1_001_000))
    );

    let mut doc = Document::new();
    doc.append(&"  ```\n  x\n```\n".repeat(4096)).unwrap();
    let before = doc.work();
    doc.append("   ~~~\n    y\n~~~\n").unwrap();
    assert!(doc.work().classified_bytes - before.classified_bytes < 30);
    assert!(doc.work().tree_visits - before.tree_visits < 2000);
    doc.finish();
    assert_eq!(doc.blocks().len(), 4097);
    assert_eq!(visible(&doc, doc.blocks().last().unwrap().source), " y\n");
}
