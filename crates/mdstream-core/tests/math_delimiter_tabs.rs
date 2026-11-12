use mdstream::*;

#[path = "support/snapshot.rs"]
mod support;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn read(mut cursor: MathContent<'_>) -> String {
    let mut text = String::new();
    while let Some(chunk) = cursor
        .advance(&mut Budget::new(WorkLimit::unlimited()))
        .unwrap()
    {
        text.push_str(chunk.text);
    }
    text
}

fn visible(doc: &Document) -> String {
    doc.display_runs_in(Span::new(0, doc.source().len()))
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect()
}

#[test]
fn delimiters_match_independent_physical_column_model() {
    let mut cases = 0;
    for (before, prefix, body_prefix, residual) in [
        ("seed\n", "", "", 0),
        ("> seed\n", ">", "> ", 0),
        ("> seed\n", "> ", "> ", 0),
        (">> seed\n", ">> ", ">> ", 0),
        ("> > seed\n", "> > ", "> > ", 0),
        (">>> seed\n", ">>> ", ">>> ", 0),
        ("- seed\n", "  ", "  ", 0),
        ("1. seed\n", "   ", "   ", 0),
        ("> - seed\n", ">   ", ">   ", 0),
        ("- seed\n", "\t", "  ", 2),
        ("> seed\n", ">\t", "> ", 2),
        ("- - seed\n", "\t", "    ", 0),
    ] {
        for whitespace in ["", " ", "   ", "\t", " \t", "  \t", "\t ", "\t\t"] {
            let mut column = 0;
            for b in prefix.bytes() {
                column += if b == b'\t' { 4 - column % 4 } else { 1 };
            }
            let start = column;
            for b in whitespace.bytes() {
                column += if b == b'\t' { 4 - column % 4 } else { 1 };
            }
            let separator = usize::from(prefix == ">" && !whitespace.is_empty());
            let indent = residual + column - start - separator;
            for ending in ["\n", "\r\n", ""] {
                let delimiter = format!("{prefix}{whitespace}$$\t {ending}");
                let source = format!("{before}{delimiter}");
                let doc = parse(&source);
                let maths: Vec<_> = doc
                    .blocks()
                    .iter()
                    .filter(|b| matches!(b.kind, BlockKind::Math { .. }))
                    .collect();
                assert_eq!(maths.len(), usize::from(indent <= 3), "{source:?}");
                if let Some(block) = maths.first() {
                    assert_eq!(block.container, doc.blocks()[0].container);
                    assert_eq!(block.state, SyntaxState::Sealed);
                    assert!(matches!(block.kind, BlockKind::Math { closed: false, .. }));
                }
                let source =
                    format!("{before}{body_prefix}$$\n{body_prefix}中🙂 &amp;\n{delimiter}");
                let doc = parse(&source);
                let block = &doc.blocks()[1];
                assert!(
                    matches!(block.kind, BlockKind::Math { closed, .. } if closed == (indent <= 3)),
                    "{source:?}"
                );
                assert_eq!(block.container, doc.blocks()[0].container);
                assert_eq!(doc.source(), source);
                assert!(doc.maths().is_empty());
                if indent <= 3 {
                    assert_eq!(
                        read(doc.math_block_payload(BlockId(1)).unwrap()),
                        "中🙂 &amp;\n"
                    );
                }
                cases += 2;
            }
        }
    }
    assert_eq!(cases, 576);
}

#[test]
fn all_splits_prefixes_and_eof_agree() {
    for source in [
        "> \t$$\r\n> \t中🙂\r\n> \t$$\r\n> after\n",
        "- seed\n\t$$\n\t中🙂\n\t$$\n- next\n",
        "> seed\n>\t$$\n> x\n>\t$$",
        "- - seed\n\t$$\n\tx\n\t$$\n",
        "> \t$$",
        "> \t$$x\n> after\n",
        "- > $$\n\t$$\n",
        "> a | b\n> \t$$\n> x\n> \t$$\n",
    ] {
        let whole = support::snapshot(&parse(source));
        for split in (0..=source.len()).filter(|&at| source.is_char_boundary(at)) {
            let mut doc = Document::new();
            doc.append(&source[..split]).unwrap();
            doc.append(&source[split..]).unwrap();
            doc.finish();
            assert_eq!(support::snapshot(&doc), whole, "split {split}: {source:?}");
        }
        let mut doc = Document::new();
        for (at, ch) in source.char_indices() {
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            let text = &source[..at + ch.len_utf8()];
            let mut prefix = Document::new();
            prefix.append(text).unwrap();
            assert_eq!(support::snapshot(&doc), support::snapshot(&prefix));
            assert_eq!(doc.pending(), prefix.pending());
            let mut eof = Document::new();
            for ch in text.chars() {
                eof.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            }
            eof.finish();
            assert_eq!(support::snapshot(&eof), support::snapshot(&parse(text)));
        }
    }
}

#[test]
fn failed_candidates_paths_and_window_keep_literal_body() {
    for candidate in ["\t$", "\t$$$", "\t$$ x", "\t$$\r", "\t\t$$", "    $$"] {
        // A bare CR is payload, not trailing delimiter whitespace.
        let suffix = if candidate.ends_with('\r') { "" } else { "\n" };
        let source = format!("> {candidate}{suffix}");
        assert!(
            parse(&source)
                .blocks()
                .iter()
                .all(|b| !matches!(b.kind, BlockKind::Math { .. })),
            "{source:?}"
        );
        let source = format!("> $$\n> {candidate}{suffix}");
        let doc = parse(&source);
        assert!(matches!(
            doc.blocks()[0].kind,
            BlockKind::Math { closed: false, .. }
        ));
        assert_eq!(
            read(doc.math_block_payload(BlockId(0)).unwrap()),
            format!("{candidate}{suffix}")
        );
    }
    for source in ["\t$$\n", "> ```\n> \t$$\n> ```\n", "- > seed\n\t$$\n"] {
        assert_eq!(
            parse(source)
                .blocks()
                .iter()
                .filter(|b| matches!(b.kind, BlockKind::Math { .. }))
                .count(),
            usize::from(source == "- > seed\n\t$$\n"),
            "{source:?}"
        );
    }
    let doc = parse("- > $$\n\t$$\n");
    assert!(matches!(
        doc.blocks()[0].kind,
        BlockKind::Math { closed: false, .. }
    ));
    assert_eq!(read(doc.math_block_payload(BlockId(0)).unwrap()), "");
    for tail in [1017, 1018, 1019, 2048] {
        for ending in ["\n", ""] {
            let line = format!("> \t$${}{ending}", " ".repeat(tail));
            let complete = true;
            let doc = parse(&line);
            assert_eq!(
                doc.blocks()
                    .iter()
                    .any(|b| matches!(b.kind, BlockKind::Math { .. })),
                complete,
                "tail {tail}, ending {ending:?}"
            );
            let doc = parse(&format!("> $$\n{line}"));
            assert!(
                matches!(doc.blocks()[0].kind, BlockKind::Math { closed, .. } if closed == complete)
            );
        }
    }
    for source in [
        ">\t$$\n> x\n>\t$$\n",
        "- \t$$\n    x\n    \t$$\n",
        "> a | b\n> \t$$\n> x\n> \t$$\n",
    ] {
        assert_eq!(
            parse(source)
                .blocks()
                .iter()
                .filter(|b| matches!(b.kind, BlockKind::Math { .. }))
                .count(),
            1,
            "{source:?}"
        );
    }
}

#[test]
fn late_closure_keeps_identity_folds_and_invalidates_readers() {
    let mut doc = Document::new();
    let mut update = doc.append("> \t$$\n> \t中🙂\n> \t$$").unwrap();
    let mut heights = HeightIndex::default();
    heights.apply(&doc, &update).unwrap();
    assert!(matches!(
        doc.blocks()[0].kind,
        BlockKind::Math { closed: false, .. }
    ));
    let scope = doc.blocks()[0].container;
    let id = FoldId::Math(BlockId(0));
    let mut view = FoldView::new(&doc, 1).unwrap();
    view.set_collapsed(&doc, id, true).unwrap();
    let ticket = view.heights().ticket(TileId(0)).unwrap();
    let mut reader = doc
        .display_cursor(Span::new(0, doc.source().len()))
        .unwrap();
    let mut search = DisplaySearch::new(&doc, "中").unwrap();
    let next = doc.append("\n> after\n").unwrap();
    heights.apply(&doc, &next).unwrap();
    view.apply(&doc, &next).unwrap();
    update.merge(next).unwrap();
    let end = doc.finish();
    heights.apply(&doc, &end).unwrap();
    view.apply(&doc, &end).unwrap();
    update.merge(end).unwrap();
    assert_eq!(doc.blocks()[0].container, scope);
    assert_eq!(doc.blocks()[1].container, scope);
    assert!(matches!(
        doc.blocks()[0].kind,
        BlockKind::Math { closed: true, .. }
    ));
    assert!(view.is_collapsed(id));
    assert!(view.measure(ticket, 8).is_err());
    let mut budget = Budget::new(WorkLimit::unlimited());
    assert_eq!(
        reader.next(&doc, &mut budget),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(
        search.advance(&doc, &mut budget, &mut Vec::new()),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(budget.used(), WorkLimit::default());
    let mut rebuilt = HeightIndex::default();
    rebuilt.apply(&doc, &update).unwrap();
    assert_eq!(rebuilt.total_rows(), heights.total_rows());
    assert_eq!(visible(&doc), "\t中🙂\nafter\n");
}

#[test]
fn raw_and_mapped_payloads_preserve_tabs_and_six_byte_budget() {
    for (source, expected) in [
        (
            "> \t$$\r\n> \t中🙂 &amp; **x**\r\n> \t$$\r\n",
            "\t中🙂 &amp; **x**\r\n",
        ),
        ("- \t$$\n\t\t中🙂\n  \t$$\n", "\t中🙂\n"),
        ("- seed\n\t$$\n\t\t中🙂\n\t$$\n", "  \t中🙂\n"),
    ] {
        let doc = parse(source);
        let id = BlockId(
            doc.blocks()
                .iter()
                .position(|b| matches!(b.kind, BlockKind::Math { .. }))
                .unwrap(),
        );
        let BlockKind::Math {
            content,
            closed: true,
        } = doc.blocks()[id.0].kind
        else {
            panic!("closed math")
        };
        assert_eq!(
            read(doc.math_block_content(id).unwrap()),
            &source[content.range()]
        );
        let mut cursor = doc.math_block_payload(id).unwrap();
        let mut output = String::new();
        let mut charged = 0;
        while cursor.source_position() < content.end {
            let at = cursor.source_position();
            assert_eq!(
                cursor.advance(&mut Budget::new(WorkLimit::default())),
                Err(Completion::BudgetExhausted)
            );
            assert_eq!(cursor.source_position(), at);
            let mut budget = Budget::new(WorkLimit {
                queries: 1,
                objects: 1,
                bytes: 6,
                ..Default::default()
            });
            let chunk = cursor.advance(&mut budget).unwrap().unwrap();
            assert_eq!(chunk.source.start, at);
            assert_eq!(budget.used().bytes, chunk.source.len());
            assert!(budget.used().bytes <= 6);
            assert_eq!(budget.used().queries, 1);
            assert_eq!(budget.used().objects, 1);
            if !chunk.text.is_empty() && doc.text(chunk.source) != Some(chunk.text) {
                assert_eq!(doc.text(chunk.source), Some("\t"));
                assert_eq!(chunk.text, "  ");
            }
            output.push_str(chunk.text);
            charged += budget.used().bytes;
        }
        assert_eq!(output, expected, "{source:?}");
        assert_eq!(charged, content.len());
        let tiled: String = (0..doc.tile_count())
            .flat_map(|i| doc.display_runs(TileId(i)).unwrap())
            .filter(|r| !r.style.contains(Style::HIDDEN))
            .map(|r| r.text)
            .collect();
        assert_eq!(tiled, visible(&doc));
        assert_eq!(
            doc.runs_in(Span::new(0, source.len()))
                .unwrap()
                .map(|r| r.text)
                .collect::<String>(),
            source
        );
    }
}

#[test]
fn deep_long_body_and_failed_closer_do_not_rescan() {
    let prefix = format!("{} ", ">".repeat(998));
    let mut doc = Document::new();
    doc.append(&format!(
        "{prefix}\t$$\n{prefix}\t{}",
        "x".repeat(1_000_000)
    ))
    .unwrap();
    assert!(matches!(doc.blocks()[0].kind, BlockKind::Math { .. }));
    let arena = doc.container_arena_stats();
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("x").unwrap();
    }
    assert_eq!(doc.work().container_steps, before.container_steps);
    assert_eq!(doc.work().container_bytes, before.container_bytes);
    assert_eq!(doc.work().classified_bytes, before.classified_bytes);
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert_eq!(doc.container_arena_stats(), arena);
    doc.append(&format!("\n{prefix}\t$${}", " ".repeat(2048)))
        .unwrap();
    let before = doc.work();
    for _ in 0..1000 {
        doc.append(" ").unwrap();
    }
    assert_eq!(doc.work().container_steps, before.container_steps);
    assert_eq!(doc.work().container_bytes, before.container_bytes);
    assert_eq!(doc.work().classified_bytes, before.classified_bytes);
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    doc.append(&format!("\n{prefix}\t$$\n")).unwrap();
    doc.finish();
    assert!(matches!(
        doc.blocks()[0].kind,
        BlockKind::Math { closed: true, .. }
    ));
    assert_eq!(
        read(doc.math_block_payload(BlockId(0)).unwrap()),
        format!("\t{}\n", "x".repeat(1_001_000))
    );
    assert_eq!(doc.container_count(), 998);
    assert!(doc.work().classified_bytes < 30 * BLOCK_LOOKAHEAD as u64);
    let source = "> \t$$\n> x\n> \t$$\n".repeat(4096);
    let doc = parse(&source);
    assert_eq!(doc.blocks().len(), 4096);
    assert!(doc.work().classified_bytes < 8 * source.len() as u64);
    assert!(doc.work().container_steps < 8 * source.len() as u64);
}
