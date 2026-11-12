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
fn root_tabs_match_four_space_code_without_expanding_payload_tabs() {
    let mut cases = 0;
    for spaces in 0..4 {
        for tail in ["", " ", "   ", "\t", " \t"] {
            for body in ["中🙂\tend", "*raw* &amp;", "# h", "```rs", "a|b", "- item"] {
                for ending in ["\n", "\r\n", ""] {
                    let source = format!("{}\t{tail}{body}{ending}", " ".repeat(spaces));
                    let doc = parse(&source);
                    let model = parse(&format!("    {tail}{body}{ending}"));
                    assert_eq!(doc.blocks().len(), 1, "{source:?}");
                    assert_eq!(doc.blocks()[0].kind, BlockKind::Code { info: None });
                    assert_eq!(doc.blocks()[0].container, None);
                    assert_eq!(visible(&doc), visible(&model), "{source:?}");
                    assert_eq!(
                        visible(&doc),
                        format!("{tail}{body}{}", if ending.is_empty() { "" } else { "\n" })
                    );
                    assert_eq!(doc.source(), source);
                    assert!(doc.links().is_empty());
                    assert!(doc.tables().is_empty());
                    cases += 1;
                }
            }
        }
    }
    assert_eq!(cases, 360);
}

#[test]
fn merging_blank_interruption_and_container_boundaries_keep_explicit_dialects() {
    let doc = parse("    first\n\tsecond\n  \tthird\n");
    assert_eq!(doc.blocks().len(), 1);
    assert_eq!(doc.blocks()[0].kind, BlockKind::Code { info: None });
    assert_eq!(visible(&doc), "first\nsecond\nthird\n");
    let doc = parse("prose\n\t*raw*\n\t \n\tlast\n");
    assert_eq!(doc.blocks().len(), 4);
    assert_eq!(doc.blocks()[0].kind, BlockKind::Paragraph);
    assert_eq!(doc.blocks()[1].kind, BlockKind::Code { info: None });
    assert_eq!(doc.blocks()[2].kind, BlockKind::Blank);
    assert_eq!(doc.blocks()[3].kind, BlockKind::Code { info: None });
    for source in ["\t", " \t\r\n", "\t\t\n"] {
        assert_eq!(parse(source).blocks()[0].kind, BlockKind::Blank);
    }
    let nested = parse("- first\n\t- nested\n");
    assert_eq!(nested.container_count(), 4);
    assert_eq!(visible(&nested), "first\nnested\n");
    for source in ["- first\n\t\t# literal\n", "> \tfoo\n"] {
        let doc = parse(source);
        assert!(
            doc.blocks()
                .iter()
                .any(|b| matches!(b.kind, BlockKind::Code { .. }))
                == source.starts_with('-')
        );
        assert_eq!(
            visible(&doc),
            if source.starts_with('-') {
                "first\n  # literal\n"
            } else {
                "\tfoo\n"
            }
        );
    }
    // A missing quote closes that quote while its matched Item remains.
    // Four leaf columns consume the first tab's residue and part of the next.
    let doc = parse("- > foo\n  >\n\t\tbar\n");
    let last = doc.blocks().last().unwrap();
    assert_eq!(last.container, Some(ContainerId(1)));
    assert_eq!(last.kind, BlockKind::Code { info: None });
    assert!(visible(&doc).ends_with("  bar\n"));
    for fence in ["```", "~~~", "$$"] {
        let doc = parse(&format!("{fence}\n\t*raw*\n{fence}\n"));
        assert_eq!(visible(&doc), "\t*raw*\n");
    }
    // Existing delimiter recognition precedes general leaf classification.
    assert_eq!(parse("a|b\n\t-|-\n").tables().len(), 1);
    for header in ["not a table", "a|b\n-|-\nx|y"] {
        let doc = parse(&format!("{header}\n\t*raw*\n"));
        assert_eq!(doc.tables().len(), usize::from(header.contains('|')));
        assert_eq!(
            doc.blocks().last().unwrap().kind,
            BlockKind::Code { info: None }
        );
    }
}

#[test]
fn every_split_prefix_and_classification_window_preserve_public_state() {
    let long = "x".repeat(BLOCK_LOOKAHEAD + 9);
    for source in [
        " \t中🙂\t*raw*\r\n\tend".to_owned(),
        "    x\n\ty\n\n\tz\n".to_owned(),
        "- foo\n\n\t\tbar\n".to_owned(),
        ">\t\tfoo\n".to_owned(),
        "a|b\n\t-|-\n".to_owned(),
        format!("   \t{long}\t\r\n"),
        format!("\t{}", " ".repeat(BLOCK_LOOKAHEAD + 9)),
    ] {
        let whole = support::snapshot(&parse(&source));
        for split in (0..=source.len()).filter(|&i| source.is_char_boundary(i)) {
            let mut doc = Document::new();
            doc.append(&source[..split]).unwrap();
            doc.append(&source[split..]).unwrap();
            doc.finish();
            assert_eq!(support::snapshot(&doc), whole, "split {split}");
        }
        let mut doc = Document::new();
        for (at, ch) in source.char_indices() {
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            let mut prefix = Document::new();
            prefix.append(&source[..at + ch.len_utf8()]).unwrap();
            assert_eq!(doc.pending(), prefix.pending());
            assert_eq!(support::snapshot(&doc), support::snapshot(&prefix));
        }
        doc.finish();
        assert_eq!(support::snapshot(&doc), whole);
    }
    let mut doc = Document::new();
    doc.append(" \t中").unwrap();
    assert_eq!(visible(&doc), " \t中");
    let old_key = doc.tile_key(TileId(0)).unwrap();
    let mut old_cursor = doc
        .display_cursor(Span::new(0, doc.source().len()))
        .unwrap();
    doc.append("\n").unwrap();
    assert_eq!(visible(&doc), "中\n");
    assert_ne!(doc.tile_key(TileId(0)).unwrap(), old_key);
    assert_eq!(
        old_cursor.next(&doc, &mut Budget::new(WorkLimit::unlimited())),
        Err(LayoutError::StaleContinuation)
    );
}

#[test]
fn hidden_tabs_keep_borrowed_mapping_and_resume_with_six_byte_budgets() {
    let source = format!("  \t{}\r\n\t中🙂\tend\n", "x".repeat(255));
    let doc = parse(&source);
    let expected = format!("{}\n中🙂\tend\n", "x".repeat(255));
    let mut end = 0;
    for run in doc.display_runs_in(Span::new(0, source.len())).unwrap() {
        assert_eq!(run.source.start, end);
        end = run.source.end;
        assert_eq!(run.replacement, None);
        assert_eq!(run.text, &source[run.source.range()]);
    }
    assert_eq!(end, source.len());
    let tiled: String = (0..doc.tile_count())
        .flat_map(|i| doc.display_runs(TileId(i)).unwrap())
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    assert_eq!(tiled, expected);
    for (at, ch) in source.char_indices() {
        let runs: Vec<_> = doc
            .display_runs_in(Span::new(at, at + ch.len_utf8()))
            .unwrap()
            .collect();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, &source[at..at + ch.len_utf8()]);
        assert_eq!(runs[0].replacement, None);
    }
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
                assert_eq!(run.source.start, position);
                assert_eq!(run.text, &source[run.source.range()]);
                assert_eq!(run.replacement, None);
                assert!(budget.used().bytes <= 6);
                assert!(budget.used().bytes >= run.source.len() + 2);
                if run.style.contains(Style::HIDDEN) {
                    hidden += run.source.len();
                } else {
                    output.push_str(run.text);
                }
            }
            DisplayStep::Complete => break,
            other => panic!("small budget failed to advance: {other:?}"),
        }
    }
    assert_eq!(hidden, 5); // Three bytes + one tab of indent, and one CR.
    assert_eq!(output, expected);
}

#[test]
fn million_byte_append_and_many_code_lines_do_not_rescan_history() {
    let mut doc = Document::new();
    doc.append(&format!("  \t{}", "x".repeat(1_000_000)))
        .unwrap();
    assert_eq!(doc.blocks()[0].kind, BlockKind::Code { info: None });
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("x").unwrap();
    }
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert_eq!(doc.work().classified_bytes, before.classified_bytes);
    assert_eq!(doc.work().container_bytes, before.container_bytes);
    assert_eq!(doc.work().container_steps, before.container_steps);
    assert_eq!(doc.work().tree_visits, before.tree_visits);
    doc.append("\n").unwrap();
    let before = doc.work();
    for _ in 0..4096 {
        doc.append(" \ta\n").unwrap();
    }
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert!(doc.work().classified_bytes - before.classified_bytes <= 4096 * 4);
    assert!(doc.work().container_bytes - before.container_bytes < 4096 * 20);
    assert!(doc.work().tree_visits - before.tree_visits < 4096 * 500);
    let before = doc.work();
    doc.finish();
    assert_eq!(doc.work().classified_bytes, before.classified_bytes);
    assert_eq!(doc.blocks().len(), 1);
    assert_eq!(
        visible(&doc),
        format!("{}\n{}", "x".repeat(1_001_000), "a\n".repeat(4096))
    );
}
