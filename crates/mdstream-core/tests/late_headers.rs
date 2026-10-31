use mdstream::*;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn styles(doc: &Document, source: Span) -> Vec<(Style, Option<LinkId>)> {
    doc.runs_in(source)
        .unwrap()
        .flat_map(|run| std::iter::repeat_n((run.style, run.link), run.source.len()))
        .collect()
}

fn visible(doc: &Document, source: Span) -> String {
    doc.display_runs_in(source)
        .unwrap()
        .filter(|run| !run.style.contains(Style::HIDDEN))
        .map(|run| run.text)
        .collect()
}

#[test]
fn committed_first_cell_objects_keep_their_ids_and_tail_is_published_once() {
    for (open, next) in [("", ""), ("> ", "> "), ("> - [x] ", ">   "), ("1. ", "   ")] {
        for success in [false, true] {
            let payload = format!("**first** [a](u) $m$ [^a] {}", "x".repeat(2048));
            let mut doc = Document::new();
            doc.append(&format!("{open}{payload}")).unwrap();
            let block = doc.blocks()[0].clone();
            assert_eq!(doc.work().inline_bytes, payload.len() as u64);
            assert_eq!(doc.links().len(), 1);
            assert_eq!(doc.maths().len(), 1);
            assert_eq!(doc.footnote_references().len(), 1);
            let link = *doc.link_info(LinkId(0)).unwrap();
            let math = doc.maths()[0];
            let footnote = doc.footnote_references()[0];
            let before = doc.work();
            let suffix = "|**second** [b](v) $n$ [^b]\r\n";
            doc.append(suffix).unwrap();
            assert_eq!(doc.work().inline_bytes, before.inline_bytes);
            assert_eq!(doc.links().len(), 1);
            assert_eq!(doc.blocks()[0].source.end, doc.source().len());
            let update = doc
                .append(&format!(
                    "{next}{}\n",
                    if success { ":--|--:" } else { "wrong" }
                ))
                .unwrap();
            assert_eq!(update.new_tables.len(), usize::from(success));
            assert_eq!(update.new_links, 1..2);
            assert_eq!(update.new_maths, 1..2);
            assert_eq!(update.new_footnote_references, 1..2);
            assert_eq!(*doc.link_info(LinkId(0)).unwrap(), link);
            assert_eq!(doc.maths()[0], math);
            assert_eq!(doc.footnote_references()[0], footnote);
            assert_eq!(doc.footnotes()[0].number, Some(1));
            assert_eq!(doc.footnotes()[1].number, Some(2));
            assert_eq!(doc.blocks()[0].container, block.container);
            if success {
                assert_eq!(doc.tables()[0].block, BlockId(0));
                assert_eq!(doc.tables()[0].columns[0].alignment, Alignment::Left);
                assert_eq!(doc.tables()[0].columns[1].alignment, Alignment::Right);
                assert_eq!(
                    doc.text(doc.tables()[0].cell(0, 0).unwrap().source),
                    Some(payload.as_str())
                );
            }
            doc.finish();
            assert_eq!(doc.links().len(), 2);
            assert!(doc.work().inline_bytes <= doc.source().len() as u64);
        }
    }
}

#[test]
fn resumed_first_cell_matches_fully_deferred_table_semantics() {
    // The leading-pipe version is an independent existing parser path: its
    // entire header is deferred and RowParser starts with an empty Inline.
    for (left, right) in [
        ("**strong** &amp; ` a\\|b ` [x](u) $m$ [^n] ", " _next_"),
        ("*open ", " end*"),
        ("`open ", " end`"),
        ("$open ", " end$"),
        ("[open ", " end](u)"),
        ("<https:open", "end>"),
        ("[ref]", "tail"),
        ("  ![alt](img) ", "**other**"),
    ] {
        for prefix in ["", "> "] {
            let payload = format!("{left}{}|{right}", "中🙂".repeat(170));
            let tail = format!("\r\n{prefix}-|-\n\n[ref]: /target\n[^n]: note\n");
            let late = parse(&format!("{prefix}{payload}{tail}"));
            let early = parse(&format!("{prefix}|{payload}{tail}"));
            let span = Span::new(prefix.len(), prefix.len() + payload.len());
            let shifted = Span::new(span.start + 1, span.end + 1);
            assert_eq!(late.tables().len(), 1, "{left:?}");
            assert_eq!(styles(&late, span), styles(&early, shifted), "{left:?}");
            assert_eq!(visible(&late, span), visible(&early, shifted), "{left:?}");
            assert_eq!(late.links().len(), early.links().len());
            for id in 0..late.links().len() {
                assert_eq!(
                    late.link_destination(LinkId(id)),
                    early.link_destination(LinkId(id))
                );
            }
            assert_eq!(late.maths().len(), early.maths().len());
            assert_eq!(
                late.footnote_references().len(),
                early.footnote_references().len()
            );
            assert!(late.work().inline_bytes <= late.source().len() as u64);
        }
    }
}

#[test]
fn first_pipe_detection_preserves_escape_parity_at_and_after_the_window() {
    for prefix in ["", "> ", "> - [ ] "] {
        let next = if prefix == "> - [ ] " { ">   " } else { prefix };
        for at in BLOCK_LOOKAHEAD - 2..=BLOCK_LOOKAHEAD + 2 {
            for slashes in 0..=4 {
                let start = format!(
                    "{prefix}{}{}|tail",
                    "x".repeat(at - prefix.len()),
                    "\\".repeat(slashes)
                );
                let source = format!("{start}\n{next}-|-\n");
                let doc = parse(&source);
                assert_eq!(doc.tables().len(), usize::from(slashes % 2 == 0));
                assert!(doc.work().inline_bytes <= source.len() as u64);
            }
        }
    }
    let doc = parse(&format!("`a\\|b` {}|tail\n-|-\n", "x".repeat(2048)));
    let first = doc.tables()[0].cell(0, 0).unwrap().source;
    assert!(visible(&doc, first).starts_with("a|b "));
}

#[test]
fn failure_eof_and_setext_consume_only_unparsed_suffix() {
    for prefix in ["", "> "] {
        for suffix in ["", "\r\n", "\r\nwrong\n", "\r\n---\n"] {
            let first = format!("{prefix}*{}", "x".repeat(2048));
            let suffix = suffix.replace('\n', &format!("\n{prefix}"));
            let doc = parse(&format!("{first}|tail* [a](u) $m$ [^n]{suffix}"));
            assert!(doc.tables().is_empty());
            assert!(
                styles(&doc, Span::new(prefix.len() + 1, prefix.len() + 2))[0]
                    .0
                    .contains(Style::EMPHASIS)
            );
            assert_eq!(doc.links().len(), 1);
            assert_eq!(doc.maths().len(), 1);
            assert_eq!(doc.footnote_references().len(), 1);
            assert!(doc.work().inline_bytes <= doc.source().len() as u64);
        }
        let doc = parse(&format!("{prefix}**{}**\n{prefix}---\n", "x".repeat(2048)));
        assert_eq!(doc.headings().entries().len(), 1);
        assert_eq!(doc.headings().entries()[0].level, 2);
    }
}

#[test]
fn ordinary_long_lines_keep_immediate_semantics_and_container_continuity() {
    for prefix in ["", "> "] {
        let mut doc = Document::new();
        doc.append(&format!("{prefix}{}", "x".repeat(2048)))
            .unwrap();
        for chunk in ["**live** ", "[x](u) ", "&amp; ", "$m$ ", "[^n] ", "*open"] {
            let before = doc.work();
            doc.append(chunk).unwrap();
            assert_eq!(
                doc.work().inline_bytes - before.inline_bytes,
                chunk.len() as u64
            );
            assert!(doc.pending().is_empty());
        }
        assert_eq!(doc.links().len(), 1);
        assert_eq!(doc.maths().len(), 1);
        assert_eq!(doc.footnote_references().len(), 1);
        let shown = visible(&doc, Span::new(0, doc.source().len()));
        assert!(shown.contains("live x & m"), "{shown:?}");
        doc.append(&format!("\r\n{prefix}end*\n")).unwrap();
        doc.finish();
        let at = doc.source().find("open").unwrap();
        assert!(
            styles(&doc, Span::new(at, at + 1))[0]
                .0
                .contains(Style::EMPHASIS)
        );
        assert!(doc.tables().is_empty());
        assert!(doc.work().inline_bytes <= doc.source().len() as u64);
    }
}

#[test]
fn open_cross_line_state_remains_explicitly_out_of_scope() {
    for prefix in ["", "> "] {
        let source = format!(
            "{prefix}*prior\n{prefix}end*{}|b\n{prefix}-|-\n",
            "x".repeat(2048)
        );
        let doc = parse(&source);
        assert!(doc.tables().is_empty());
        assert!(
            styles(&doc, Span::new(prefix.len() + 1, prefix.len() + 2))[0]
                .0
                .contains(Style::EMPHASIS)
        );
        // Starting an independent sequence after a real block boundary works.
        for boundary in [format!("{prefix}\n"), format!("{prefix}# title\n")] {
            let doc = parse(&format!(
                "{prefix}prior\n{boundary}{prefix}{}|b\n{prefix}-|-\n",
                "x".repeat(2048)
            ));
            assert_eq!(doc.tables().len(), 1);
        }
    }
}

#[test]
fn code_lazy_sibling_and_nested_boundaries_keep_their_existing_rules() {
    for (open, next) in [("> ", ""), ("> ", "> > "), ("- ", "- "), ("> - ", "> ")] {
        let doc = parse(&format!("{open}{}|b\n{next}-|-\n", "x".repeat(2048)));
        assert!(doc.tables().is_empty());
    }
    for source in [
        format!("```\n{}|b\n-|-\n```\n", "x".repeat(2048)),
        format!("> ```\n> {}|b\n> -|-\n> ```\n", "x".repeat(2048)),
        format!("    {}|b\n-|-\n", "x".repeat(2048)),
        format!("> prior\n{}|b\n> -|-\n", "x".repeat(2048)),
    ] {
        assert!(parse(&source).tables().is_empty());
    }
    let source = format!(
        "{}{}|b\n{}-|-\n",
        "- ".repeat(250),
        "x".repeat(2048),
        " ".repeat(500)
    );
    let doc = parse(&source);
    assert_eq!(doc.tables().len(), 1);
    assert_eq!(doc.container_count(), 500);
    assert_eq!(
        doc.block(doc.tables()[0].block).unwrap().container,
        Some(ContainerId(499))
    );
    assert!(doc.work().container_bytes < 4 * source.len() as u64);
}

#[test]
fn million_byte_prefix_and_tail_only_consume_new_bytes_with_constant_candidate_state() {
    let prefix = ">".repeat(1000);
    for success in [false, true] {
        let mut doc = Document::new();
        doc.append(&format!("{prefix}{}", "x".repeat(40))).unwrap();
        let before = doc.work();
        let annotations = doc.annotation_arena_stats();
        let containers = doc.container_arena_stats();
        for _ in 0..1000 {
            doc.append(&"x".repeat(1000)).unwrap();
        }
        let after = doc.work();
        assert_eq!(after.inline_bytes - before.inline_bytes, 1_000_000);
        assert_eq!(
            after.table_header_bytes - before.table_header_bytes,
            1_000_000
        );
        assert_eq!(after.classified_bytes, before.classified_bytes);
        assert_eq!(after.container_steps, before.container_steps);
        assert_eq!(after.container_bytes, before.container_bytes);
        assert_eq!(after.heading_steps, before.heading_steps);
        assert_eq!(doc.annotation_arena_stats(), annotations);
        assert_eq!(doc.container_arena_stats(), containers);
        doc.append("|").unwrap();
        for _ in 0..1000 {
            doc.append(&"y".repeat(1000)).unwrap();
        }
        assert_eq!(doc.work().inline_bytes, after.inline_bytes);
        assert_eq!(doc.annotation_arena_stats(), annotations);
        assert_eq!(doc.container_arena_stats(), containers);
        assert_eq!(
            doc.work().table_header_bytes - after.table_header_bytes,
            1_000_001
        );
        assert!(doc.pending().is_empty());
        doc.append("\n").unwrap();
        let before = doc.work();
        doc.append(&format!("{prefix}{}\n", if success { "-|-" } else { "no" }))
            .unwrap();
        assert_eq!(doc.tables().len(), usize::from(success));
        assert!(doc.work().inline_bytes - before.inline_bytes <= 1_000_010);
        if success {
            assert_eq!(doc.work().table_bytes, 2_000_042);
        }
        doc.finish();
        assert!(doc.work().inline_bytes <= doc.source().len() as u64);
        assert!(doc.work().table_header_bytes <= doc.source().len() as u64);
    }
}

#[test]
fn failed_long_delimiter_does_not_replay_prefix_or_restart_candidate() {
    let mut doc = Document::new();
    doc.append(&format!(
        "> {}|b\n> {}",
        "[x](u) ".repeat(1024),
        "-".repeat(4096)
    ))
    .unwrap();
    assert_eq!(doc.links().len(), 1024);
    doc.append("x").unwrap();
    assert_eq!(doc.links().len(), 1024);
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("|").unwrap();
    }
    assert_eq!(doc.work().table_header_bytes, before.table_header_bytes);
    assert_eq!(
        doc.work().table_candidate_bytes,
        before.table_candidate_bytes
    );
    assert_eq!(doc.work().classified_bytes, before.classified_bytes);
    assert_eq!(doc.work().inline_bytes - before.inline_bytes, 1000);
    doc.append("\n> h|b\n> -|-\n").unwrap();
    doc.finish();
    assert_eq!(doc.tables().len(), 1);
    assert_eq!(
        doc.text(doc.tables()[0].cell(0, 0).unwrap().source),
        Some("h")
    );
    assert!(doc.work().inline_bytes <= doc.source().len() as u64);
}

#[test]
fn dense_published_objects_and_wide_sparse_tables_stay_linear() {
    let mut doc = Document::new();
    let first = "[a](u) $m$ [^n] ".repeat(4096);
    doc.append(&first).unwrap();
    assert_eq!(doc.links().len(), 4096);
    assert_eq!(doc.maths().len(), 4096);
    assert_eq!(doc.footnote_references().len(), 4096);
    doc.append(&format!("|{}\n", "h|".repeat(4095))).unwrap();
    let before = doc.work();
    let update = doc.append(&format!("|{}\n", "-:|".repeat(4096))).unwrap();
    assert_eq!(update.new_links, 4096..4096);
    assert_eq!(update.new_maths, 4096..4096);
    assert_eq!(update.new_footnote_references, 4096..4096);
    assert_eq!(doc.work().label_bytes, before.label_bytes);
    assert_eq!(doc.work().footnote_steps, before.footnote_steps);
    assert_eq!(doc.work().math_steps, before.math_steps);
    doc.append(&"x\n".repeat(4096)).unwrap();
    doc.finish();
    let table = &doc.tables()[0];
    assert_eq!(table.columns.len(), 4096);
    assert_eq!(
        table.rows().iter().map(|r| r.cells.len()).sum::<usize>(),
        8192
    );
    assert!(doc.work().inline_bytes <= doc.source().len() as u64);
    assert!(doc.annotation_node_count() <= 4 * doc.source().len());
}

#[test]
fn generated_success_failure_ranges_match_independent_model() {
    let mut source = String::new();
    let mut expected = Vec::new();
    for i in 0..400 {
        let prefix = ">".repeat(i % 9);
        let start = source.len();
        source.push_str(&format!("{prefix}{}|b\r\n", "x".repeat(1024 + i)));
        let header_end = source.len();
        source.push_str(&format!(
            "{prefix}{}|-{}\n",
            "-".repeat(1024 + i),
            if i % 3 == 0 { "x" } else { "" }
        ));
        let delimiter_end = source.len();
        source.push_str(&format!("{prefix}v|z\n"));
        if i % 3 != 0 {
            expected.push((
                Span::new(start, source.len()),
                Span::new(header_end, delimiter_end),
                Span::new(start + prefix.len(), start + prefix.len() + 1024 + i),
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
    }
    for at in (0..source.len()).step_by(179) {
        let id = expected
            .iter()
            .position(|(span, ..)| span.range().contains(&at));
        assert_eq!(doc.table_at(at), id.map(TableId));
    }
    assert!(doc.work().inline_bytes <= source.len() as u64);
    assert!(doc.work().table_header_bytes <= source.len() as u64);
    assert!(doc.work().table_candidate_bytes <= source.len() as u64);
}

#[test]
fn confirmation_invalidates_measurements_and_merges_with_container_heading_folds() {
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let mut merged = FoldView::new(&doc, 1).unwrap();
    let mut batch = Update::default();
    for chunk in ["> # title\n> **header** ", &"x".repeat(4096), "|b\r\n"] {
        let update = doc.append(chunk).unwrap();
        view.apply(&doc, &update).unwrap();
        batch.merge(update).unwrap();
    }
    let key = doc.tile_key(TileId(5)).unwrap();
    let mut heights = HeightIndex::default();
    heights.rebuild(&doc).unwrap();
    let ticket = heights.ticket(TileId(5)).unwrap();
    let update = doc.append("> -|-\n> x|y\n").unwrap();
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
fn million_byte_late_header_preview_and_folded_layout_obey_shared_budgets() {
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let update = doc
        .append(&format!("> {}|b\n> -|-\n", "x".repeat(1_000_000)))
        .unwrap();
    view.apply(&doc, &update).unwrap();
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
    let mut complete = false;
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
                complete = true;
                break;
            }
        }
    }
    assert!(complete);
    assert_eq!(text, "x".repeat(62));
    assert!(charged < 128);

    struct Sink(usize);
    impl LayoutSink for Sink {
        fn begin_tile(&mut self, _: TileId) {}
        fn text(&mut self, _: RenderRun<'_>) {
            panic!("folded text");
        }
        fn object(&mut self, _: LinkId, _: &LinkInfo) {
            panic!("folded object");
        }
        fn end_tile(&mut self, _: TileId) -> u64 {
            1
        }
    }
    impl FoldLayoutSink for Sink {
        fn placeholder(&mut self, _: FoldPlaceholder) {
            self.0 += 1;
        }
    }
    view.set_collapsed(&doc, FoldId::Table(TableId(0)), true)
        .unwrap();
    let update = doc.append("> **new**|row\n").unwrap();
    view.apply(&doc, &update).unwrap();
    let mut pass = LayoutPass::folded(&doc, &view, TileId(0), 10).unwrap();
    let mut sink = Sink(0);
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
            assert_eq!(sink.0, 1);
            return;
        }
    }
    panic!("folded pass did not advance");
}
