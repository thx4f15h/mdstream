use mdtui3::*;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

#[test]
fn long_delimiters_preserve_alignment_optional_pipes_and_source_ranges() {
    for (header, prefix) in [
        ("a|b|c|d\n", ""),
        ("> a|b|c|d\n", "> "),
        ("> - [x] a|b|c|d\n", ">   "),
        ("1. a|b|c|d\n", "   "),
        ("- > - a|b|c|d\n", "  >   "),
    ] {
        for outer in ["", "|"] {
            let delimiter = format!(
                "{prefix}{outer} {} | :--- | ---: | :---: {outer}\r\n",
                "-".repeat(2048)
            );
            let source = format!("{header}{delimiter}{prefix}中|b|c|d\n");
            let doc = parse(&source);
            let table = &doc.tables()[0];
            assert_eq!(doc.tables().len(), 1);
            assert_eq!(table.source, Span::new(0, source.len()));
            assert_eq!(
                table.delimiter,
                Span::new(header.len(), header.len() + delimiter.len())
            );
            assert_eq!(table.rows().len(), 2);
            assert_eq!(
                table
                    .columns
                    .iter()
                    .map(|c| c.alignment)
                    .collect::<Vec<_>>(),
                [
                    Alignment::Default,
                    Alignment::Left,
                    Alignment::Right,
                    Alignment::Center
                ]
            );
            assert_eq!(doc.text(table.cell(1, 0).unwrap().source), Some("中"));
            assert!(
                doc.runs_in(table.delimiter)
                    .unwrap()
                    .all(|r| r.style.contains(Style::HIDDEN))
            );
            assert_eq!(doc.table_at(header.len() + 1500), Some(TableId(0)));
            assert_eq!(table.row_at(header.len() + 1500), None);
        }
    }
}

#[test]
fn separator_can_cross_limit_in_whitespace_or_before_its_first_pipe() {
    for delimiter in [
        format!("{}-|-", " ".repeat(4096)),
        format!("{}|-", "-".repeat(4096)),
        format!("|-|-|{}", "\t".repeat(4096)),
        format!("|-{}|-|", " ".repeat(4096)),
    ] {
        for prefix in ["", "> "] {
            for eol in ["", "\n", "\r\n"] {
                let source = format!("{prefix}a|b\n{prefix}{delimiter}{eol}");
                let doc = parse(&source);
                assert_eq!(doc.tables().len(), 1);
                assert_eq!(doc.tables()[0].source.end, source.len());
                assert_eq!(
                    doc.work().table_candidate_bytes,
                    (delimiter.len() + eol.len()) as u64
                );
            }
        }
    }
}

#[test]
fn invalid_late_candidates_recover_inline_text_once_and_allow_the_next_table() {
    for suffix in ["x", "🙂", ":-", "::", " -", "\\|", "|--|--", "||"] {
        for prefix in ["", "> "] {
            let source = format!(
                "{prefix}**a**|b\n{prefix}|{}|-{suffix}\n{prefix}c|d\n{prefix}-|-\n",
                "-".repeat(4096)
            );
            let doc = parse(&source);
            assert_eq!(doc.tables().len(), 1, "{suffix:?}");
            assert_eq!(
                doc.text(doc.tables()[0].cell(0, 0).unwrap().source),
                Some("c")
            );
            assert!(
                doc.runs_in(Span::new(0, source.len()))
                    .unwrap()
                    .any(|r| r.text == "a" && r.style.contains(Style::STRONG))
            );
            assert!(doc.work().inline_bytes <= source.len() as u64);
            assert!(doc.work().classified_bytes < 3 * BLOCK_LOOKAHEAD as u64);
        }
    }
}

#[test]
fn missing_pipes_columns_and_eof_use_original_incomplete_prefix_fallback() {
    for tail in ["", "|", "|: ", "|--|--", " | "] {
        let source = format!("**a**|b\n{}{tail}", "-".repeat(4096));
        let doc = parse(&source);
        assert!(doc.tables().is_empty());
        assert!(doc.headings().entries().is_empty()); // No late Setext retry.
        assert_eq!(doc.blocks().len(), 1);
        assert_eq!(doc.blocks()[0].kind, BlockKind::Paragraph);
        assert!(doc.work().inline_bytes <= source.len() as u64);
    }
    let doc = parse(&format!("a|b\n{}", "-".repeat(BLOCK_LOOKAHEAD)));
    assert!(doc.tables().is_empty());
    assert_eq!(doc.blocks()[0].source.end, doc.source().len());
}

#[test]
fn exact_scope_rejects_siblings_lazy_lines_and_new_nested_containers() {
    for (header, prefix) in [
        ("> a|b\n", ""),
        ("> a|b\n", "> > "),
        ("- a|b\n", "- "),
        ("> - a|b\n", "> - "),
        ("> - a|b\n", "> "),
        ("1. a|b\n", "2. "),
    ] {
        let source = format!("{header}{prefix}{}|-\n", "-".repeat(2048));
        assert!(parse(&source).tables().is_empty(), "{header:?} {prefix:?}");
    }
    let fence = "```";
    for source in [
        format!("{fence}\na|b\n{}|-\n{fence}\n", "-".repeat(2048)),
        format!("> {fence}\n> a|b\n> {}|-\n> {fence}\n", "-".repeat(2048)),
        format!("a|b\n-|-\n{}|-\n", "-".repeat(2048)),
    ] {
        let doc = parse(&source);
        assert_eq!(doc.tables().len(), usize::from(source.starts_with("a|b")));
        assert_eq!(doc.work().table_candidate_bytes, 0);
    }
}

#[test]
fn million_byte_probe_has_no_reclassification_or_ancestor_work_on_append() {
    let prefix = ">".repeat(1000);
    for success in [true, false] {
        let mut doc = Document::new();
        doc.append(&format!("{prefix}**h**|b\n{prefix}|{}", "-".repeat(50)))
            .unwrap();
        let before = doc.work();
        for _ in 0..1000 {
            doc.append(&"-".repeat(1000)).unwrap();
            assert!(doc.pending().len() <= BLOCK_LOOKAHEAD + 3);
        }
        let after = doc.work();
        assert_eq!(
            after.table_candidate_bytes - before.table_candidate_bytes,
            1_000_000
        );
        assert_eq!(after.classified_bytes, before.classified_bytes);
        assert_eq!(after.inline_bytes, before.inline_bytes);
        assert_eq!(after.container_steps, before.container_steps);
        assert_eq!(after.container_bytes, before.container_bytes);
        assert_eq!(after.heading_steps, before.heading_steps);
        let update = doc.append(if success { "|-\n" } else { "|-x\n" }).unwrap();
        assert_eq!(update.new_tables, 0..usize::from(success));
        assert!(doc.work().classified_bytes - after.classified_bytes <= 2 * BLOCK_LOOKAHEAD as u64);
        if success {
            assert!(doc.work().inline_bytes - after.inline_bytes < 16);
            assert_eq!(doc.work().container_steps, after.container_steps);
        } else {
            assert!(doc.work().inline_bytes <= doc.source().len() as u64);
            assert!(doc.work().container_steps - after.container_steps < 1100);
        }
        doc.finish();
        assert_eq!(doc.container_count(), 1000);
    }
}

#[test]
fn late_rejection_does_not_keep_probing_the_same_line() {
    let mut doc = Document::new();
    doc.append(&format!("> *h|b\n> |{}", "-".repeat(100_000)))
        .unwrap();
    doc.append("x").unwrap();
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("-").unwrap();
    }
    let after = doc.work();
    assert_eq!(after.inline_bytes - before.inline_bytes, 1000);
    assert_eq!(after.table_candidate_bytes, before.table_candidate_bytes);
    assert_eq!(after.classified_bytes, before.classified_bytes);
    assert_eq!(after.container_bytes, before.container_bytes);
    doc.append("*\n").unwrap();
    doc.finish();
    assert!(doc.tables().is_empty());
    assert!(
        doc.runs_in(Span::new(0, doc.source().len()))
            .unwrap()
            .any(|r| r.text.contains('h') && r.style.contains(Style::EMPHASIS))
    );
    assert!(doc.work().inline_bytes <= doc.source().len() as u64);
}

#[test]
fn wide_separator_storage_is_limited_by_bounded_header_columns() {
    let source = format!("|{}\n|{}\nx\n", "h|".repeat(400), "---|".repeat(400));
    let doc = parse(&source);
    assert_eq!(doc.tables()[0].columns.len(), 400);
    assert_eq!(doc.tables()[0].row(1).unwrap().cells.len(), 1);
    let mut failed = Document::new();
    failed
        .append(&format!("a|b\n|{}", "-".repeat(2048)))
        .unwrap();
    failed.append("|-|-|").unwrap();
    let before = failed.work();
    failed.append(&"-|".repeat(20_000)).unwrap();
    assert_eq!(
        failed.work().table_candidate_bytes,
        before.table_candidate_bytes
    );
    failed.finish();
    assert!(failed.tables().is_empty());
}

#[test]
fn confirmation_invalidates_layout_and_fold_updates_can_be_merged() {
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let mut merged = FoldView::new(&doc, 1).unwrap();
    let mut batch = Update::default();
    for chunk in ["> # title\n> **h**|b\n", "> |", &"-".repeat(4096)] {
        let update = doc.append(chunk).unwrap();
        view.apply(&doc, &update).unwrap();
        batch.merge(update).unwrap();
    }
    let pending_key = doc.tile_key(TileId(0)).unwrap();
    let mut heights = HeightIndex::default();
    heights.rebuild(&doc).unwrap();
    let ticket = heights.ticket(TileId(0)).unwrap();
    let update = doc.append("|-|\n> x|y\n").unwrap();
    assert_eq!(update.new_tables, 0..1);
    assert_eq!(update.rows_changed[0].table, TableId(0));
    heights.apply(&doc, &update).unwrap();
    assert_eq!(
        heights.measure(ticket, 2),
        Err(LayoutError::StaleMeasurement)
    );
    assert_ne!(doc.tile_key(TileId(0)).unwrap(), pending_key);
    view.apply(&doc, &update).unwrap();
    batch.merge(update).unwrap();
    merged.apply(&doc, &batch).unwrap();
    let table = FoldId::Table(TableId(0));
    assert_eq!(
        doc.fold_candidate(table).unwrap().parent,
        Some(FoldId::Heading(HeadingId(0)))
    );
    for view in [&mut view, &mut merged] {
        view.set_collapsed(&doc, table, true).unwrap();
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            ..Default::default()
        });
        assert!(matches!(
            view.hidden_at(&doc, doc.source().len() - 2, &mut budget)
                .unwrap(),
            FoldLookup::Hidden(_)
        ));
    }
    assert_eq!(view.heights().total_rows(), merged.heights().total_rows());
}

#[test]
fn bounded_header_and_other_long_block_candidates_remain_out_of_scope() {
    for prefix in ["", "> "] {
        let source = format!("{prefix}{}|b\n{prefix}-|-\n", "x".repeat(BLOCK_LOOKAHEAD));
        assert!(parse(&source).tables().is_empty());
        let source = format!("{prefix}title\n{prefix}{}\n", "=".repeat(2048));
        assert!(parse(&source).headings().entries().is_empty());
    }
}

#[test]
fn long_delimiter_ranges_match_an_independent_multi_table_model() {
    let mut source = String::new();
    let mut expected = Vec::new();
    for i in 0..400 {
        let prefix = ">".repeat(i % 7);
        let start = source.len();
        source.push_str(&format!("{prefix}h{i}|b\n"));
        let delimiter_start = source.len();
        let length = 1010 + i % 41;
        source.push_str(&format!(
            "{prefix}|{}|-{}\n",
            "-".repeat(length),
            if i % 3 == 0 { "x" } else { ":" }
        ));
        let delimiter_end = source.len();
        source.push_str(&format!("{prefix}v{i}|z\n"));
        if i % 3 != 0 {
            expected.push((
                Span::new(start, source.len()),
                Span::new(delimiter_start, delimiter_end),
                Span::new(delimiter_end, source.len()),
                i,
            ));
        }
        source.push('\n');
    }
    let doc = parse(&source);
    assert_eq!(doc.tables().len(), expected.len());
    for (table, (span, delimiter, row, i)) in doc.tables().iter().zip(&expected) {
        assert_eq!(table.source, *span);
        assert_eq!(table.delimiter, *delimiter);
        assert_eq!(table.row(1).unwrap().source, *row);
        assert_eq!(
            doc.text(table.cell(0, 0).unwrap().source),
            Some(format!("h{i}").as_str())
        );
        assert_eq!(
            doc.text(table.cell(1, 0).unwrap().source),
            Some(format!("v{i}").as_str())
        );
    }
    for at in (0..source.len()).step_by(137) {
        let table = expected
            .iter()
            .position(|(span, ..)| span.range().contains(&at));
        assert_eq!(doc.table_at(at), table.map(TableId));
    }
    assert!(doc.work().table_candidate_bytes <= source.len() as u64);
    assert!(doc.work().inline_bytes <= source.len() as u64);
    assert!(doc.work().classified_bytes <= 2 * source.len() as u64);
}

#[test]
fn budgeted_preview_skips_the_entire_long_delimiter() {
    let doc = parse(&format!("> h|b\n> |{}|-|\n> 中|z\n", "-".repeat(1_000_000)));
    let mut preview = TablePreview::new(
        &doc,
        TableId(0),
        TablePreviewLimits {
            rows: 2,
            source_bytes: 64,
            display_bytes: 64,
        },
    )
    .unwrap();
    let mut zero = Budget::new(WorkLimit::default());
    assert_eq!(
        preview.next(&doc, &mut zero).unwrap(),
        TablePreviewStep::BudgetExhausted
    );
    assert_eq!(zero.used(), WorkLimit::default());
    let mut text = String::new();
    let mut charged = 0;
    for _ in 0..100 {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 7,
            ..Default::default()
        });
        let step = preview.next(&doc, &mut budget).unwrap();
        assert!(budget.used().bytes <= 7);
        charged += budget.used().bytes;
        match step {
            TablePreviewStep::Run { run, .. } => text.push_str(run.text),
            TablePreviewStep::BudgetExhausted => {}
            TablePreviewStep::Complete(end) => {
                assert_eq!(end, TablePreviewEnd::RowsEnd);
                assert_eq!(text, "hb\n中z\n");
                assert!(charged <= 64);
                return;
            }
        }
    }
    panic!("preview failed to advance");
}

#[test]
fn long_probes_keep_deep_list_identity_and_original_failure_classification() {
    let depth = 250;
    let mut doc = Document::new();
    doc.append(&format!(
        "{}h|b\n{}{}",
        "- ".repeat(depth),
        " ".repeat(depth * 2),
        "-".repeat(2048)
    ))
    .unwrap();
    let before = doc.work();
    doc.append(&"-".repeat(4096)).unwrap();
    assert_eq!(doc.work().container_steps, before.container_steps);
    assert_eq!(doc.work().container_bytes, before.container_bytes);
    doc.append("|-\n").unwrap();
    assert_eq!(doc.container_count(), depth * 2);
    let table = &doc.tables()[0];
    assert_eq!(
        doc.block(table.block).unwrap().container,
        Some(ContainerId(depth * 2 - 1))
    );
    assert!(doc.work().container_bytes < 5 * doc.source().len() as u64);

    for prefix in ["", "> "] {
        let source = format!("{prefix}**h**|b\n{prefix}{}x\n", " ".repeat(4096));
        let doc = parse(&source);
        assert!(doc.tables().is_empty());
        assert!(matches!(doc.blocks()[1].kind, BlockKind::Code { .. }));
        assert!(
            doc.runs_in(Span::new(0, source.len()))
                .unwrap()
                .any(|r| r.text == "h" && r.style.contains(Style::STRONG))
        );
    }
    // A root header following an existing paragraph has no block of its own
    // until confirmation; the old paragraph must keep its exact endpoint.
    let source = format!("intro *open\nh|b\n{}|-\n", "-".repeat(4096));
    let doc = parse(&source);
    assert_eq!(doc.tables()[0].source.start, "intro *open\n".len());
    assert_eq!(doc.blocks()[0].source, Span::new(0, "intro *open\n".len()));
}
