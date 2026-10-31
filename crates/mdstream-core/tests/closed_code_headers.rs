use mdstream::*;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn styles(doc: &Document, span: Span) -> Vec<(Style, Option<LinkId>)> {
    doc.runs_in(span)
        .unwrap()
        .flat_map(|r| std::iter::repeat_n((r.style, r.link), r.source.len()))
        .collect()
}

fn visible(doc: &Document, span: Span) -> String {
    doc.display_runs_in(span)
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect()
}

fn compare_header(prior: &str, prefix: &str, header: &str) {
    let tail = format!("{prefix}:--|--:\r\n{prefix}v|z\n");
    let late = parse(&format!("{prior}{prefix}{header}{tail}"));
    let early = parse(&format!("{prior}{prefix}|{header}{tail}"));
    assert_eq!(late.tables().len(), 1, "{prior:?} {header:?}");
    let old = Span::new(0, prior.len());
    assert_eq!(styles(&late, old), styles(&early, old));
    assert_eq!(visible(&late, old), visible(&early, old));
    let start = prior.len() + prefix.len();
    let span = Span::new(start, start + header.len());
    let shifted = Span::new(start + 1, span.end + 1);
    assert_eq!(styles(&late, span), styles(&early, shifted));
    assert_eq!(visible(&late, span), visible(&early, shifted));
    assert_eq!(late.links().len(), early.links().len());
    assert_eq!(late.maths().len(), early.maths().len());
    assert_eq!(
        late.footnote_references().len(),
        early.footnote_references().len()
    );
    assert!(late.work().inline_bytes <= late.source().len() as u64 + late.work().table_bytes);
}

#[test]
fn isolated_closed_code_matches_independent_cell_semantics() {
    for (open, prefix) in [("", ""), ("> ", "> "), ("> - [x] ", ">   "), ("1. ", "   ")] {
        for ticks in [1, 2, 3, 16] {
            let marker = "`".repeat(ticks);
            for padding in [0, 1010, 2048] {
                let prior = format!("{open}[p](u) &amp; *old {marker} prior\r\n");
                let header = format!(
                    "[h](v) $m$ [^n] &amp; {}end{marker} {}|[t](w)\r\n",
                    "x".repeat(padding),
                    "中🙂".repeat(150)
                );
                compare_header(&prior, prefix, &header);
            }
        }
    }
}

#[test]
fn only_cross_line_code_effects_wait_until_confirmation_or_failure() {
    for success in [false, true] {
        let mut doc = Document::new();
        doc.append("> [p](u) $p$ [^p] ` prior\r\n").unwrap();
        let link = *doc.link_info(LinkId(0)).unwrap();
        let math = doc.maths()[0];
        let footnote = doc.footnote_references()[0];
        doc.append(&format!(
            "> [h](v) $m$ [^n] &amp; end` {}",
            "x".repeat(2048)
        ))
        .unwrap();
        let old = Span::new(23, 27);
        assert!(!styles(&doc, old).iter().any(|s| s.0.contains(Style::CODE)));
        assert_eq!(doc.links().len(), 1);
        assert_eq!(doc.maths().len(), 1);
        assert_eq!(doc.footnote_references().len(), 1);
        let block = BlockId(doc.blocks().len() - 1);
        let before = doc.work();
        doc.append("|[t](w)\n").unwrap();
        assert_eq!(doc.work().inline_bytes, before.inline_bytes);
        let update = doc
            .append(if success { "> -|-\n" } else { "> wrong\n" })
            .unwrap();
        assert_eq!(update.new_tables.len(), usize::from(success));
        assert_eq!(update.new_links, 1..if success { 3 } else { 2 });
        assert_eq!(update.new_maths, 1..if success { 2 } else { 1 });
        assert_eq!(
            update.new_footnote_references,
            1..if success { 2 } else { 1 }
        );
        assert_eq!(*doc.link_info(LinkId(0)).unwrap(), link);
        assert_eq!(doc.maths()[0], math);
        assert_eq!(doc.footnote_references()[0], footnote);
        assert_eq!(styles(&doc, old)[0].0.contains(Style::CODE), !success);
        if success {
            assert_eq!(doc.tables()[0].block, block);
        }
        doc.finish();
    }
}

#[test]
fn fallback_no_pipe_eof_setext_and_scope_exit_restore_exact_mapping_once() {
    for prefix in ["", "> "] {
        for payload in ["", " ", "  ", "中\r\n", " \r\n\r\n", " &amp; "] {
            // Keep physical newlines inside the explicit scope.
            let payload = payload.replace('\n', &format!("\n{prefix}"));
            for end in [
                "",
                "\r\n",
                "|b",
                "|b\r\n",
                "|b\nwrong\n",
                "|b\n---\n",
                "\n# end\n",
            ] {
                let end = end.replace('\n', &format!("\n{prefix}"));
                let source = format!(
                    "{prefix}a `{payload}\r\n{prefix} ` {}{end}",
                    "x".repeat(1030)
                );
                let doc = parse(&source);
                let ordinary = parse(&source.replace('|', "/"));
                let span = Span::new(0, source.len());
                assert!(doc.tables().is_empty());
                assert_eq!(styles(&doc, span), styles(&ordinary, span));
                assert_eq!(
                    visible(&doc, span).replace('|', "/"),
                    visible(&ordinary, span)
                );
                assert_eq!(doc.blocks(), ordinary.blocks());
                assert!(doc.work().inline_bytes <= source.len() as u64);
            }
        }
    }
    let source = format!("> a ` prior\n> end` {}|b\n-|-\n", "x".repeat(1030));
    let doc = parse(&source);
    assert!(doc.tables().is_empty());
    assert!(styles(&doc, Span::new(6, 7))[0].0.contains(Style::CODE));
}

#[test]
fn unsafe_suffix_flushes_before_new_mappings_and_never_restarts_the_probe() {
    for suffix in [
        "&amp;",
        "<https://x>",
        "\\*",
        "*local*",
        "_local_",
        "~~local~~",
        "`local`",
        "$m$",
        "![i](u)",
        "[h](u)",
        "]",
        "[^n]",
    ] {
        for padding in [0, 1010, 2048] {
            let mut doc = Document::new();
            doc.append("a [old *old ` prior\r\n").unwrap();
            // One feed must flush code before a subsequent entity/code entry.
            doc.append(&format!(
                "{}end` {suffix} &amp; [live](v) `new {}",
                "x".repeat(padding),
                "y".repeat(1100)
            ))
            .unwrap();
            let before = doc.work();
            assert!(styles(&doc, Span::new(16, 17))[0].0.contains(Style::CODE));
            assert!(!doc.links().is_empty());
            let links = doc.links().to_vec();
            doc.append("|b`\n-|-\n").unwrap();
            doc.finish();
            assert!(doc.tables().is_empty(), "{suffix:?}");
            // The new short line may be summarized, but the long line never restarts.
            assert!(doc.work().table_header_bytes - before.table_header_bytes <= 4);
            assert_eq!(doc.links(), links);
            let ordinary = parse(&doc.source().replace('|', "/"));
            let span = Span::new(0, doc.source().len());
            assert_eq!(styles(&doc, span), styles(&ordinary, span));
            assert_eq!(
                visible(&doc, span).replace('|', "/"),
                visible(&ordinary, span)
            );
            assert_eq!(doc.maths(), ordinary.maths());
            assert!(doc.work().inline_bytes <= doc.source().len() as u64);
        }
    }
}

#[test]
fn million_byte_deep_probe_visits_only_new_bytes_and_reparses_only_the_row() {
    let prefix = ">".repeat(1000);
    for success in [false, true] {
        let mut doc = Document::new();
        doc.append(&format!(
            "{prefix}a `{}\n{prefix}end` {}",
            "o".repeat(1_000_000),
            "x".repeat(40)
        ))
        .unwrap();
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
        assert_eq!(after.delimiter_steps, before.delimiter_steps);
        assert_eq!(after.classified_bytes, before.classified_bytes);
        assert_eq!(after.container_bytes, before.container_bytes);
        assert_eq!(after.container_steps, before.container_steps);
        assert_eq!(after.heading_steps, before.heading_steps);
        assert_eq!(doc.annotation_arena_stats(), annotations);
        assert_eq!(doc.container_arena_stats(), containers);
        doc.append("|b\n").unwrap();
        doc.append(&format!(
            "{prefix}{}\n",
            if success { "-|-" } else { "wrong" }
        ))
        .unwrap();
        assert_eq!(doc.tables().len(), usize::from(success));
        if success {
            assert_eq!(doc.work().inline_bytes - after.inline_bytes, 1_000_046);
            assert_eq!(doc.work().delimiter_steps - after.delimiter_steps, 2);
        } else {
            assert!(doc.work().inline_bytes - after.inline_bytes < 12);
            assert_eq!(doc.work().delimiter_steps - after.delimiter_steps, 1);
        }
        doc.finish();
        assert!(doc.work().inline_bytes <= doc.source().len() as u64 + doc.work().table_bytes);
    }
}

#[test]
fn long_separator_and_unsafe_suffix_follow_independent_range_model() {
    let mut source = String::new();
    let mut expected = Vec::new();
    for i in 0..400 {
        let prefix = ">".repeat(i % 9);
        source.push_str(&format!("{prefix}a ` prior\n"));
        let start = source.len();
        source.push_str(&format!(
            "{prefix}end` {}{}|b\r\n",
            "x".repeat(1030 + i),
            if i % 3 == 0 { "&amp;" } else { "" }
        ));
        let delimiter = source.len();
        source.push_str(&format!(
            "{prefix}{}|-{}\n",
            "-".repeat(1030 + i),
            if i % 5 == 0 { "x" } else { "" }
        ));
        let end = source.len();
        if i % 3 != 0 && i % 5 != 0 {
            expected.push((Span::new(start, end), Span::new(delimiter, end)));
        }
        source.push('\n');
    }
    let doc = parse(&source);
    assert_eq!(doc.tables().len(), expected.len());
    for (table, (span, delimiter)) in doc.tables().iter().zip(expected) {
        assert_eq!(table.source, span);
        assert_eq!(table.delimiter, delimiter);
        assert_eq!(doc.block(table.block).unwrap().source, span);
    }
    assert!(doc.work().inline_bytes <= source.len() as u64 + doc.work().table_bytes);
    assert!(doc.work().table_header_bytes <= source.len() as u64);
    assert!(doc.work().table_candidate_bytes <= source.len() as u64);
}

#[test]
fn dense_old_objects_and_opaque_row_publish_once_without_visiting_history() {
    let open = "- ".repeat(250);
    let prefix = " ".repeat(500);
    let mut doc = Document::new();
    doc.append(&format!(
        "{}\n\n{open}{} ` prior\n{prefix}{}end` ",
        "[p](u) ".repeat(4096),
        "*old ".repeat(4096),
        "[h](v) $m$ [^n] ".repeat(4096)
    ))
    .unwrap();
    assert_eq!(doc.links().len(), 4096);
    let old = *doc.link_info(LinkId(4095)).unwrap();
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("x").unwrap();
    }
    assert_eq!(doc.work().delimiter_steps, before.delimiter_steps);
    assert_eq!(doc.work().label_bytes, before.label_bytes);
    doc.append(&format!(
        "|{}\n{prefix}|{}\n",
        "h|".repeat(4095),
        "-:|".repeat(4096)
    ))
    .unwrap();
    assert_eq!(doc.links().len(), 8192);
    assert_eq!(doc.maths().len(), 4096);
    assert_eq!(doc.footnote_references().len(), 4096);
    assert_eq!(*doc.link_info(LinkId(4095)).unwrap(), old);
    assert_eq!(doc.tables()[0].columns.len(), 4096);
    assert_eq!(
        doc.block(doc.tables()[0].block).unwrap().container,
        Some(ContainerId(499))
    );
    assert_eq!(
        doc.work().delimiter_steps - before.delimiter_steps,
        4096 + 8192 + 2
    );
    doc.finish();
    assert!(doc.work().inline_bytes <= doc.source().len() as u64 + doc.work().table_bytes);
}

#[test]
fn mappings_agree_for_whole_scalar_tile_and_budgeted_reads() {
    for suffix in ["-|-\n", "wrong\n", "---\n"] {
        let source = format!(
            "> a ` \r\n> {}\r\n> [h](u) &amp; end ` {}|` a\\|b `\r\n> {suffix}",
            "raw ".repeat(1030),
            "中🙂".repeat(150)
        );
        let doc = parse(&source);
        let expected = visible(&doc, Span::new(0, source.len()));
        let tiled: String = (0..doc.tile_count())
            .flat_map(|i| doc.display_runs(TileId(i)).unwrap())
            .filter(|r| !r.style.contains(Style::HIDDEN))
            .map(|r| r.text)
            .collect();
        assert_eq!(tiled, expected);
        let mut sliced = String::new();
        for (at, c) in source.char_indices() {
            for run in doc
                .display_runs_in(Span::new(at, at + c.len_utf8()))
                .unwrap()
            {
                if let Some(replacement) = run.replacement {
                    assert!(
                        replacement.start <= run.source.start && run.source.end <= replacement.end
                    );
                }
                if !run.style.contains(Style::HIDDEN) {
                    sliced.push_str(run.text);
                }
            }
        }
        assert_eq!(sliced, expected);
        let mut cursor = doc.display_cursor(Span::new(0, source.len())).unwrap();
        let mut text = String::new();
        let mut zero = Budget::new(WorkLimit::default());
        assert_eq!(
            cursor.next(&doc, &mut zero).unwrap(),
            DisplayStep::BudgetExhausted
        );
        assert_eq!(zero.used(), WorkLimit::default());
        for frame in 0..20000 {
            let mut budget = Budget::new(WorkLimit {
                queries: 1,
                objects: 1,
                bytes: 8,
                ..Default::default()
            });
            match cursor.next(&doc, &mut budget).unwrap() {
                DisplayStep::Run(run) if !run.style.contains(Style::HIDDEN) => {
                    text.push_str(run.text)
                }
                DisplayStep::Complete => break,
                _ => {}
            }
            assert!(budget.used().bytes <= 8);
            assert!(frame < 19999);
        }
        assert_eq!(text, expected);
    }
}

#[test]
fn deferred_commits_invalidate_old_measurements_readers_and_merged_views() {
    for success in [false, true] {
        let mut doc = Document::new();
        let mut view = FoldView::new(&doc, 1).unwrap();
        let mut merged = FoldView::new(&doc, 1).unwrap();
        let mut batch = Update::default();
        for chunk in [
            "> # title\n> a ` prior \r\n",
            &format!("> end` {}", "x".repeat(2048)),
            "|b\n",
        ] {
            let update = doc.append(chunk).unwrap();
            view.apply(&doc, &update).unwrap();
            batch.merge(update).unwrap();
        }
        let block = doc.blocks().last().unwrap().clone();
        let old_key = doc.tile_key(TileId(0)).unwrap();
        let ticket = view.heights().ticket(TileId(0)).unwrap();
        let mut reader = doc
            .display_cursor(Span::new(0, doc.source().len()))
            .unwrap();
        let mut search = DisplaySearch::new(&doc, "prior").unwrap();
        let update = doc
            .append(if success { "> -|-\n" } else { "> wrong\n" })
            .unwrap();
        view.apply(&doc, &update).unwrap();
        batch.merge(update).unwrap();
        merged.apply(&doc, &batch).unwrap();
        assert_eq!(view.heights().total_rows(), merged.heights().total_rows());
        assert_ne!(doc.tile_key(TileId(0)).unwrap(), old_key);
        assert_eq!(view.measure(ticket, 9), Err(LayoutError::StaleMeasurement));
        let mut budget = Budget::new(WorkLimit::unlimited());
        assert_eq!(
            reader.next(&doc, &mut budget).unwrap_err(),
            LayoutError::StaleContinuation
        );
        assert_eq!(
            search
                .advance(&doc, &mut budget, &mut Vec::new())
                .unwrap_err(),
            LayoutError::StaleContinuation
        );
        assert_eq!(budget.used(), WorkLimit::default());
        if success {
            let table = doc.tables()[0].block;
            assert_eq!(doc.block(table).unwrap().source.start, block.source.start);
            assert_eq!(doc.block(table).unwrap().container, block.container);
            for view in [&mut view, &mut merged] {
                view.set_collapsed(&doc, FoldId::Table(TableId(0)), true)
                    .unwrap();
                view.set_collapsed(&doc, FoldId::Heading(HeadingId(0)), true)
                    .unwrap();
                view.set_collapsed(&doc, FoldId::Heading(HeadingId(0)), false)
                    .unwrap();
                let mut budget = Budget::new(WorkLimit {
                    queries: 1,
                    ..Default::default()
                });
                assert!(matches!(
                    view.hidden_at(&doc, 1500, &mut budget).unwrap(),
                    FoldLookup::Hidden(_)
                ));
            }
        }
    }
}

#[test]
fn budgeted_preview_search_and_folded_layout_preserve_source_mapping() {
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let update = doc
        .append(&format!(
            "> a ` old\n> **live** end` {}|b\n> -|-\n",
            "x".repeat(1_000_000)
        ))
        .unwrap();
    view.apply(&doc, &update).unwrap();
    let mut preview = TablePreview::new(
        &doc,
        TableId(0),
        TablePreviewLimits {
            rows: 1,
            source_bytes: 48,
            display_bytes: 48,
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
        match preview.next(&doc, &mut budget).unwrap() {
            TablePreviewStep::Run { run, .. } => {
                assert!(doc.text(run.source).is_some());
                text.push_str(run.text);
            }
            TablePreviewStep::Complete(end) => {
                assert_eq!(end, TablePreviewEnd::SourceLimit);
                complete = true;
            }
            TablePreviewStep::BudgetExhausted => {}
        }
        assert!(budget.used().bytes <= 7);
        charged += budget.used().bytes;
        if complete {
            break;
        }
    }
    assert!(complete && charged < 100);
    assert!(text.starts_with("live end` "));
    let mut search = DisplaySearch::new(&doc, "end`").unwrap();
    let mut hits = Vec::new();
    for _ in 0..100 {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 7,
            ..Default::default()
        });
        search.advance(&doc, &mut budget, &mut hits).unwrap();
        assert!(budget.used().bytes <= 7);
        if !hits.is_empty() {
            break;
        }
    }
    assert_eq!(hits.len(), 1);

    struct Sink(usize);
    impl LayoutSink for Sink {
        fn begin_tile(&mut self, _: TileId) {}
        fn text(&mut self, _: RenderRun<'_>) {
            panic!("hidden body read");
        }
        fn object(&mut self, _: LinkId, _: &LinkInfo) {
            panic!("hidden object read");
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
    let container = FoldId::Container(doc.blocks()[0].container.unwrap());
    view.set_collapsed(&doc, container, true).unwrap();
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
    panic!("folded pass did not finish");
}
