use mdstream::*;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn style(doc: &Document, at: usize) -> Style {
    doc.runs_in(Span::new(at, at + 1))
        .unwrap()
        .next()
        .unwrap()
        .style
}

#[test]
fn long_headers_preserve_cells_alignment_ranges_and_block_identity() {
    for (open, prefix) in [("", ""), ("> ", "> "), ("> - [x] ", ">   "), ("1. ", "   ")] {
        for outer in ["", "|"] {
            let header = format!(
                "{open}{outer}**name** | {} | `x\\|y` {outer}\r\n",
                "中🙂".repeat(300)
            );
            let mut doc = Document::new();
            doc.append(&header).unwrap();
            let id = doc.blocks()[0].clone();
            assert!(doc.tables().is_empty());
            assert_eq!(doc.work().inline_bytes, 0);
            assert!(!style(&doc, header.find("name").unwrap()).contains(Style::STRONG));
            let delimiter = format!("{prefix}:--|:--:|--:\r\n");
            let update = doc.append(&delimiter).unwrap();
            assert_eq!(update.new_tables, 0..1);
            doc.append(&format!("{prefix}x|y|z\n")).unwrap();
            doc.finish();
            let table = &doc.tables()[0];
            assert_eq!(table.block, BlockId(0));
            assert_eq!(doc.block(table.block).unwrap().container, id.container);
            assert_eq!(table.row(0).unwrap().source, Span::new(0, header.len()));
            assert_eq!(
                table.delimiter,
                Span::new(header.len(), header.len() + delimiter.len())
            );
            assert_eq!(table.source.end, doc.source().len());
            assert_eq!(
                table
                    .columns
                    .iter()
                    .map(|c| c.alignment)
                    .collect::<Vec<_>>(),
                [Alignment::Left, Alignment::Center, Alignment::Right]
            );
            assert_eq!(doc.text(table.cell(0, 0).unwrap().source), Some("**name**"));
            assert_eq!(doc.text(table.cell(0, 2).unwrap().source), Some("`x\\|y`"));
            assert!(style(&doc, header.find("name").unwrap()).contains(Style::STRONG));
            assert_eq!(
                doc.work().table_header_bytes,
                (header.len() - open.len()) as u64,
                "open={open:?} outer={outer:?}"
            );
        }
    }
}

#[test]
fn confirmation_discards_previous_inline_state_and_publishes_objects_once() {
    for prefix in ["", "> "] {
        let intro = format!("{prefix}*prior\n");
        let header = format!(
            "{prefix}end*|{} **strong** [x](u) $m$ [^n]\n",
            "a".repeat(2048)
        );
        let mut doc = Document::new();
        doc.append(&intro).unwrap();
        doc.append(&header).unwrap();
        assert!(doc.links().is_empty());
        assert!(doc.maths().is_empty());
        assert!(doc.footnote_references().is_empty());
        let before = doc.work();
        let update = doc.append(&format!("{prefix}-|-\n")).unwrap();
        assert_eq!(update.new_links, 0..1);
        assert_eq!(update.new_maths, 0..1);
        assert_eq!(update.new_footnote_references, 0..1);
        assert_eq!(
            doc.work().table_bytes - before.table_bytes,
            (header.len() - prefix.len()) as u64
        );
        assert_eq!(doc.blocks()[0].source, Span::new(0, intro.len()));
        assert_eq!(doc.tables()[0].source.start, intro.len());
        assert!(!style(&doc, prefix.len() + 1).contains(Style::EMPHASIS));
        assert!(!style(&doc, intro.len() + prefix.len()).contains(Style::EMPHASIS));
        assert!(style(&doc, doc.source().find("strong").unwrap()).contains(Style::STRONG));
    }
}

#[test]
fn failure_and_eof_resume_preceding_paragraph_without_replaying_semantics() {
    for prefix in ["", "> "] {
        for tail in ["", "\n", "\nwrong\n", "\n---\n"] {
            let intro = format!("{prefix}*prior\n");
            let header = format!("{prefix}end*|{} [x](u) $m$ [^n]", "a".repeat(2048));
            let tail = tail.replace('\n', &format!("\n{prefix}"));
            let source = format!("{intro}{header}{tail}");
            let doc = parse(&source);
            assert!(doc.tables().is_empty());
            assert!(style(&doc, prefix.len() + 1).contains(Style::EMPHASIS));
            assert!(style(&doc, intro.len() + prefix.len()).contains(Style::EMPHASIS));
            assert_eq!(doc.links().len(), 1);
            assert_eq!(doc.maths().len(), 1);
            assert_eq!(doc.footnote_references().len(), 1);
            assert!(doc.work().inline_bytes <= source.len() as u64);
            assert!(doc.work().classified_bytes < 3 * BLOCK_LOOKAHEAD as u64);
        }
    }
}

#[test]
fn pipe_eligibility_escape_parity_and_late_first_pipe_are_explicit() {
    for (start, accepted) in [("|", true), ("a|", true), ("\\|", false), ("\\\\|", true)] {
        let header = format!("{start}{}\n", "x".repeat(2048));
        let columns = usize::from(start != "|") + 1;
        let delimiter = format!("|{}\n", "-|".repeat(columns));
        assert_eq!(
            parse(&format!("{header}{delimiter}")).tables().len(),
            usize::from(accepted)
        );
    }
    for prefix in ["", "> "] {
        let doc = parse(&format!("{prefix}{}|b\n{prefix}-|-\n", "x".repeat(2048)));
        assert_eq!(doc.tables().len(), 1);
        // A leading pipe with only whitespace has zero cells and falls back.
        let doc = parse(&format!("{prefix}|{}\n{prefix}|-\n", " ".repeat(2048)));
        assert!(doc.tables().is_empty());
        assert!(doc.work().inline_bytes <= doc.source().len() as u64);
    }
}

#[test]
fn exact_container_boundaries_and_existing_blocks_keep_priority() {
    for (open, next) in [("> ", ""), ("> ", "> > "), ("- ", "- "), ("> - ", "> ")] {
        let source = format!("{open}|{}|b\n{next}|-|-\n", "x".repeat(2048));
        assert!(parse(&source).tables().is_empty(), "{open:?} {next:?}");
    }
    for source in [
        format!("```\n|{}\n|-\n```\n", "x".repeat(2048)),
        format!("> ```\n> |{}\n> |-\n> ```\n", "x".repeat(2048)),
        format!("    |{}\n|-\n", "x".repeat(2048)),
        format!("> prior\n|{}\n> |-\n", "x".repeat(2048)),
    ] {
        assert!(parse(&source).tables().is_empty());
    }
    let doc = parse(&format!("h|b\n-|-\n|{}|z\n", "x".repeat(2048)));
    assert_eq!(doc.tables().len(), 1);
    assert_eq!(doc.tables()[0].rows().len(), 2);
    assert_eq!(doc.work().table_header_bytes, 4);
}

#[test]
fn failed_long_delimiter_cannot_start_a_second_probe_on_the_same_line() {
    let mut doc = Document::new();
    doc.append(&format!(
        "> |{}|b\n> |{}",
        "x".repeat(4096),
        "-".repeat(4096)
    ))
    .unwrap();
    doc.append("x").unwrap();
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("|").unwrap();
    }
    let after = doc.work();
    assert_eq!(after.table_header_bytes, before.table_header_bytes);
    assert_eq!(after.table_candidate_bytes, before.table_candidate_bytes);
    assert_eq!(after.classified_bytes, before.classified_bytes);
    assert_eq!(after.inline_bytes - before.inline_bytes, 1000);
    doc.append("\n> h|b\n> -|-\n").unwrap();
    doc.finish();
    assert_eq!(doc.tables().len(), 1);
    assert_eq!(
        doc.text(doc.tables()[0].cell(0, 0).unwrap().source),
        Some("h")
    );
}

#[test]
fn million_byte_headers_only_summarize_new_bytes_with_constant_candidate_storage() {
    let prefix = ">".repeat(1000);
    for success in [false, true] {
        let mut doc = Document::new();
        doc.append(&format!("{prefix}|{}", "x".repeat(40))).unwrap();
        let before = doc.work();
        let annotations = doc.annotation_arena_stats();
        let containers = doc.container_arena_stats();
        for _ in 0..1000 {
            doc.append(&"a".repeat(1000)).unwrap();
            assert!(doc.pending().is_empty());
        }
        let after = doc.work();
        assert_eq!(
            after.table_header_bytes - before.table_header_bytes,
            1_000_000
        );
        assert_eq!(after.inline_bytes, before.inline_bytes);
        assert_eq!(after.classified_bytes, before.classified_bytes);
        assert_eq!(after.table_bytes, before.table_bytes);
        assert_eq!(after.container_steps, before.container_steps);
        assert_eq!(after.container_bytes, before.container_bytes);
        assert_eq!(after.heading_steps, before.heading_steps);
        assert_eq!(doc.annotation_arena_stats(), annotations);
        assert_eq!(doc.container_arena_stats(), containers);
        assert_eq!(doc.blocks().len(), 1);
        doc.append("\n").unwrap();
        assert_eq!(doc.work().inline_bytes, before.inline_bytes);
        doc.append(&format!("{prefix}{}\n", if success { "|-" } else { "no" }))
            .unwrap();
        assert_eq!(doc.tables().len(), usize::from(success));
        assert!(doc.work().inline_bytes <= doc.source().len() as u64);
        if success {
            assert_eq!(doc.work().table_bytes, 1_000_042);
        }
        let committed = doc.work();
        doc.finish();
        assert_eq!(doc.work().inline_bytes, committed.inline_bytes);
    }
}

#[test]
fn wide_headers_keep_only_real_cells_and_sparse_rows() {
    let columns = 4096;
    let mut doc = Document::new();
    doc.append(&format!("|{}\n", "h|".repeat(columns))).unwrap();
    assert_eq!(doc.work().inline_bytes, 0);
    assert!(doc.annotation_node_count() < 64);
    doc.append(&format!("|{}\n", "-:|".repeat(columns)))
        .unwrap();
    doc.append(&"x\n".repeat(4000)).unwrap();
    doc.finish();
    let table = &doc.tables()[0];
    assert_eq!(table.columns.len(), columns);
    assert_eq!(table.row(0).unwrap().cells.len(), columns);
    assert_eq!(
        table.rows().iter().map(|r| r.cells.len()).sum::<usize>(),
        columns + 4000
    );
    assert!(
        table
            .columns
            .iter()
            .all(|c| c.alignment == Alignment::Right)
    );
    assert_eq!(doc.work().table_header_bytes, (2 * columns + 2) as u64);
}

#[test]
fn deep_list_headers_and_rejected_header_chains_remain_linear() {
    let depth = 250;
    let source = format!(
        "{}|{}|b\n{}|-|-\n",
        "- ".repeat(depth),
        "x".repeat(4096),
        " ".repeat(depth * 2)
    );
    let doc = parse(&source);
    assert_eq!(doc.container_count(), depth * 2);
    assert_eq!(
        doc.block(doc.tables()[0].block).unwrap().container,
        Some(ContainerId(depth * 2 - 1))
    );
    assert!(doc.work().container_bytes < 4 * source.len() as u64);
    let line = format!("> |{}|*end*\n", "x".repeat(1100));
    let doc = parse(&line.repeat(1000));
    assert!(doc.tables().is_empty());
    assert_eq!(doc.work().table_header_bytes, doc.work().inline_bytes);
    assert!(doc.work().inline_bytes <= doc.source().len() as u64);
    assert_eq!(doc.container_count(), 1);
}

#[test]
fn ranges_cells_and_scopes_match_independent_generated_model() {
    let mut source = String::new();
    let mut expected = Vec::new();
    for i in 0..400 {
        let prefix = ">".repeat(i % 9);
        let start = source.len();
        let cell = Span::new(
            start + prefix.len() + 1,
            start + prefix.len() + 1 + 1024 + i,
        );
        source.push_str(&format!("{prefix}|{}|b\n", "x".repeat(1024 + i)));
        let header_end = source.len();
        source.push_str(&format!(
            "{prefix}|{}|-{}\n",
            "-".repeat(1000 + i),
            if i % 3 == 0 { "x" } else { "" }
        ));
        let delimiter_end = source.len();
        source.push_str(&format!("{prefix}v|z\n"));
        if i % 3 != 0 {
            expected.push((
                Span::new(start, source.len()),
                Span::new(header_end, delimiter_end),
                cell,
            ));
        }
        source.push('\n');
    }
    let doc = parse(&source);
    assert_eq!(doc.tables().len(), expected.len());
    for (table, (span, delimiter, cell)) in doc.tables().iter().zip(&expected) {
        assert_eq!(table.source, *span);
        assert_eq!(table.delimiter, *delimiter);
        assert_eq!(table.cell(0, 0).unwrap().source, *cell);
        assert_eq!(
            table.row(0).unwrap().source,
            Span::new(span.start, delimiter.start)
        );
        assert_eq!(
            table.row(1).unwrap().source,
            Span::new(delimiter.end, span.end)
        );
    }
    for at in (0..source.len()).step_by(173) {
        let id = expected
            .iter()
            .position(|(span, ..)| span.range().contains(&at));
        assert_eq!(doc.table_at(at), id.map(TableId));
    }
    assert!(doc.work().table_header_bytes <= source.len() as u64);
    assert!(doc.work().table_candidate_bytes <= source.len() as u64);
    assert!(doc.work().inline_bytes <= source.len() as u64);
}

#[test]
fn late_confirmation_invalidates_old_measurements_and_preserves_fold_composition() {
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let mut merged = FoldView::new(&doc, 1).unwrap();
    let mut batch = Update::default();
    for chunk in ["> # title\n> |**header**|", &"x".repeat(4096), "\r\n"] {
        let update = doc.append(chunk).unwrap();
        view.apply(&doc, &update).unwrap();
        batch.merge(update).unwrap();
    }
    let key = doc.tile_key(TileId(5)).unwrap();
    let mut heights = HeightIndex::default();
    heights.rebuild(&doc).unwrap();
    let ticket = heights.ticket(TileId(5)).unwrap();
    let update = doc.append("> |-|-\n> x|y\n").unwrap();
    heights.apply(&doc, &update).unwrap();
    assert_eq!(
        heights.measure(ticket, 2),
        Err(LayoutError::StaleMeasurement)
    );
    assert_ne!(doc.tile_key(TileId(5)).unwrap(), key);
    view.apply(&doc, &update).unwrap();
    batch.merge(update).unwrap();
    merged.apply(&doc, &batch).unwrap();
    let table = FoldId::Table(TableId(0));
    let heading = FoldId::Heading(HeadingId(0));
    assert_eq!(doc.fold_candidate(table).unwrap().parent, Some(heading));
    for view in [&mut view, &mut merged] {
        view.set_collapsed(&doc, table, true).unwrap();
        view.set_collapsed(&doc, heading, true).unwrap();
        view.set_collapsed(&doc, heading, false).unwrap();
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            ..Default::default()
        });
        assert!(matches!(
            view.hidden_at(&doc, 2000, &mut budget).unwrap(),
            FoldLookup::Hidden(_)
        ));
    }
    assert_eq!(view.heights().total_rows(), merged.heights().total_rows());
}

#[test]
fn preview_reads_only_budgeted_prefix_of_million_byte_header() {
    let doc = parse(&format!("> |{}|b\n> |-|-\n", "x".repeat(1_000_000)));
    let mut preview = TablePreview::new(
        &doc,
        TableId(0),
        TablePreviewLimits {
            rows: 1,
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
                assert_eq!(end, TablePreviewEnd::SourceLimit);
                assert_eq!(text, "x".repeat(61));
                assert!(charged < 128);
                return;
            }
        }
    }
    panic!("preview did not advance");
}

#[test]
fn folded_million_byte_header_skips_all_text_and_retains_streaming_range() {
    #[derive(Default)]
    struct Sink {
        text_bytes: usize,
        placeholders: usize,
    }
    impl LayoutSink for Sink {
        fn begin_tile(&mut self, _: TileId) {}
        fn text(&mut self, run: RenderRun<'_>) {
            self.text_bytes += run.text.len();
        }
        fn object(&mut self, _: LinkId, _: &LinkInfo) {
            panic!("hidden object");
        }
        fn end_tile(&mut self, _: TileId) -> u64 {
            1
        }
    }
    impl FoldLayoutSink for Sink {
        fn placeholder(&mut self, _: FoldPlaceholder) {
            self.placeholders += 1;
        }
    }
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let update = doc
        .append(&format!("> |{}|b\n> |-|-\n", "x".repeat(1_000_000)))
        .unwrap();
    view.apply(&doc, &update).unwrap();
    view.set_collapsed(&doc, FoldId::Table(TableId(0)), true)
        .unwrap();
    let update = doc.append("> **new**|row\n").unwrap();
    view.apply(&doc, &update).unwrap();
    let mut pass = LayoutPass::folded(&doc, &view, TileId(0), 10).unwrap();
    let mut sink = Sink::default();
    for _ in 0..20 {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            tiles: 1,
            objects: 1,
            bytes: 8,
            ..Default::default()
        });
        let result = pass
            .advance_folded(&doc, &mut view, &mut budget, &mut sink)
            .unwrap();
        assert_eq!(budget.used().bytes, 0);
        if result == Completion::EndOfDocument {
            assert_eq!(sink.text_bytes, 0);
            assert_eq!(sink.placeholders, 1);
            return;
        }
    }
    panic!("folded pass did not advance");
}
