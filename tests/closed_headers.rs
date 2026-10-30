use mdtui3::*;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn styles(doc: &Document, span: Span) -> Vec<(Style, Option<LinkId>)> {
    doc.runs_in(span)
        .unwrap()
        .flat_map(|run| std::iter::repeat_n((run.style, run.link), run.source.len()))
        .collect()
}

fn visible(doc: &Document, span: Span) -> String {
    doc.display_runs_in(span)
        .unwrap()
        .filter(|run| !run.style.contains(Style::HIDDEN))
        .map(|run| run.text)
        .collect()
}

#[test]
fn isolated_closures_match_independent_early_header_semantics() {
    for (open, next) in [("", ""), ("> ", "> "), ("> - [x] ", ">   "), ("1. ", "   ")] {
        for marker in ["*", "**", "_", "__", "~~"] {
            for ending in ["", "  ", "\\"] {
                let prior = format!("{open}[p](u) {marker}prior{ending}\r\n");
                let header = format!(
                    "end{marker} **local** [h](v) $m$ [^n] &amp; {}|`a\\|b`\r\n",
                    "中🙂".repeat(150)
                );
                let tail = format!("{next}:--|--:\n{next}v|z\n");
                let late = parse(&format!("{prior}{next}{header}{tail}"));
                let early = parse(&format!("{prior}{next}|{header}{tail}"));
                assert_eq!(late.tables().len(), 1, "{open:?} {marker:?}");
                let old = Span::new(0, prior.len());
                assert_eq!(styles(&late, old), styles(&early, old));
                assert_eq!(visible(&late, old), visible(&early, old));
                let start = prior.len() + next.len();
                let body = Span::new(start, start + header.len());
                let shifted = Span::new(start + 1, body.end + 1);
                assert_eq!(styles(&late, body), styles(&early, shifted));
                assert_eq!(visible(&late, body), visible(&early, shifted));
                assert_eq!(late.links().len(), early.links().len());
                assert_eq!(late.maths().len(), early.maths().len());
                assert_eq!(
                    late.footnote_references().len(),
                    early.footnote_references().len()
                );
                assert!(late.work().inline_bytes <= late.source().len() as u64);
            }
        }
    }
}

#[test]
fn only_cross_line_effects_wait_while_local_semantics_and_objects_are_live() {
    for success in [false, true] {
        let mut doc = Document::new();
        doc.append("> *prior  \r\n").unwrap();
        doc.append(&format!(
            "> end* **local** [h](u) $m$ [^n] {}",
            "x".repeat(2048)
        ))
        .unwrap();
        let old = Span::new(3, 8);
        assert!(
            !styles(&doc, old)
                .iter()
                .any(|s| s.0.contains(Style::EMPHASIS))
        );
        assert!(visible(&doc, Span::new(0, doc.source().len())).contains("end* local h"));
        let link = *doc.link_info(LinkId(0)).unwrap();
        let math = doc.maths()[0];
        let footnote = doc.footnote_references()[0];
        let before = doc.work();
        doc.append("|[tail](v)\n").unwrap();
        assert_eq!(doc.work().inline_bytes, before.inline_bytes);
        let update = doc
            .append(if success { "> -|-\n" } else { "> wrong\n" })
            .unwrap();
        assert_eq!(update.new_tables.len(), usize::from(success));
        assert_eq!(update.new_links, 1..2);
        assert_eq!(update.new_maths, 1..1);
        assert_eq!(update.new_footnote_references, 1..1);
        assert_eq!(*doc.link_info(LinkId(0)).unwrap(), link);
        assert_eq!(doc.maths()[0], math);
        assert_eq!(doc.footnote_references()[0], footnote);
        assert_eq!(styles(&doc, old)[0].0.contains(Style::EMPHASIS), !success);
        doc.finish();
        assert_eq!(doc.links().len(), 2);
    }
}

#[test]
fn failure_no_pipe_eof_setext_and_scope_exit_commit_each_pair_once() {
    for prefix in ["", "> "] {
        for marker in ["*", "**", "_", "__", "~~"] {
            for suffix in [
                "",
                "\n",
                "|b",
                "|b\r\n",
                "|b\nwrong\n",
                "|b\n---\n",
                "\n# end\n",
                "|b\n\n",
            ] {
                let source = format!(
                    "{prefix}{marker}old  \r\n{prefix}end{marker} [h](u) {}{}",
                    "x".repeat(1030),
                    suffix.replace('\n', &format!("\n{prefix}"))
                );
                let doc = parse(&source);
                let ordinary = parse(&source.replace('|', "/"));
                assert!(doc.tables().is_empty());
                assert_eq!(
                    styles(&doc, Span::new(0, source.len())),
                    styles(&ordinary, Span::new(0, source.len()))
                );
                assert_eq!(doc.blocks(), ordinary.blocks());
                assert_eq!(doc.links(), ordinary.links());
                assert!(
                    styles(&doc, Span::new(prefix.len(), prefix.len() + 1))[0]
                        .0
                        .contains(Style::HIDDEN)
                );
                assert!(doc.work().inline_bytes <= source.len() as u64);
            }
        }
    }
    let source = format!("> *old\n> end* {}|b\n-|-\n", "x".repeat(1030));
    let doc = parse(&source);
    assert!(doc.tables().is_empty());
    assert!(styles(&doc, Span::new(3, 4))[0].0.contains(Style::EMPHASIS));
}

#[test]
fn nonseparable_crossings_flush_isolated_pairs_and_keep_original_semantics() {
    for (prior, current) in [
        ("*old\n", "end*new "),      // The closer would open in a standalone cell.
        ("*old\n", "**local end* "), // A current-line opener would be lost.
        ("**old *old\n", "end* end**new "), // Flush an earlier isolated pair.
        ("**old *old\n", "end* _local end** "),
        ("[old *old\n", "end* end](u) "), // Later object cancels the probe.
        ("`old\n", "end` &amp; "),        // Semantic suffix cancels code isolation.
        ("$old\n", "end$ &amp; "),        // Semantic suffix cancels math isolation.
    ] {
        for prefix in ["", "> "] {
            let prior = format!("{prefix}{prior}");
            let body = format!("{prefix}{current}{}", "x".repeat(2048));
            let mut doc = Document::new();
            doc.append(&prior).unwrap();
            doc.append(&body).unwrap();
            let before = doc.work();
            doc.append(" live |b\n").unwrap();
            assert_eq!(doc.work().table_header_bytes, before.table_header_bytes);
            doc.append(&format!("{prefix}-|-\n")).unwrap();
            doc.finish();
            assert!(doc.tables().is_empty(), "{prior:?} {current:?}");
            let ordinary = parse(&doc.source().replace('|', "/"));
            assert_eq!(
                styles(&doc, Span::new(0, doc.source().len())),
                styles(&ordinary, Span::new(0, doc.source().len()))
            );
            assert_eq!(doc.links(), ordinary.links());
            assert_eq!(doc.maths(), ordinary.maths());
            assert!(doc.work().inline_bytes <= doc.source().len() as u64);
        }
    }
}

#[test]
fn isolated_pairs_can_pop_old_other_kinds_and_preserve_new_live_openers() {
    for (prior, current) in [
        ("*old **old ~~old\n", "end* **live** _new "),
        ("*old **old\n", "end** end* ~~live~~ *new "),
        ("*old *old\n", "end* end* *live* **new "),
        ("_old __old ~~old\n", "end~~ end__ end_ __local__ `new "),
    ] {
        let body = format!("{current}{}|end* ** ~~ `\n", "x".repeat(1030));
        let late = parse(&format!("{prior}{body}-|-\n"));
        let early = parse(&format!("{prior}|{body}-|-\n"));
        assert_eq!(late.tables().len(), 1);
        assert_eq!(
            styles(&late, Span::new(0, prior.len())),
            styles(&early, Span::new(0, prior.len()))
        );
        assert_eq!(
            styles(&late, Span::new(prior.len(), prior.len() + body.len())),
            styles(
                &early,
                Span::new(prior.len() + 1, prior.len() + body.len() + 1)
            )
        );
        assert_eq!(
            visible(&late, late.tables()[0].rows()[0].source),
            visible(&early, early.tables()[0].rows()[0].source)
        );
    }
}

#[test]
fn generated_closer_flags_and_stack_interference_match_independent_model() {
    let markers = ["*", "**", "_", "__", "~~"];
    for old in markers {
        for local in ["", "*local ", "**local ", "_local ", "__local ", "~~local "] {
            for following in [" ", "\t", "new ", "! "] {
                let prior = format!("{old}prior\n");
                let body = format!("{local}end{old}{following}{}|b\n", "x".repeat(1030));
                let source = format!("{prior}{body}-|-\n");
                let doc = parse(&source);
                // A matching local opener shields the old one; underscore
                // intraword runs are literal. Otherwise only close-only pairs
                // with no current-line stack can be isolated.
                let local_marker = local.strip_suffix("local ");
                let intraword = old.starts_with('_') && following == "new ";
                let expected = intraword
                    || local_marker == Some(old)
                    || (local.is_empty() && matches!(following, " " | "\t"));
                assert_eq!(
                    !doc.tables().is_empty(),
                    expected,
                    "{old:?} {local:?} {following:?}"
                );
                if expected {
                    let early = parse(&format!("{prior}|{body}-|-\n"));
                    assert_eq!(
                        styles(&doc, Span::new(0, prior.len())),
                        styles(&early, Span::new(0, prior.len()))
                    );
                    assert_eq!(
                        styles(&doc, Span::new(prior.len(), prior.len() + body.len())),
                        styles(
                            &early,
                            Span::new(prior.len() + 1, prior.len() + body.len() + 1)
                        )
                    );
                } else {
                    let ordinary = parse(&source.replace('|', "/"));
                    assert_eq!(
                        styles(&doc, Span::new(0, source.len())),
                        styles(&ordinary, Span::new(0, source.len()))
                    );
                }
            }
        }
    }
}

#[test]
fn million_byte_deep_probe_never_revisits_body_ancestors_or_pairs() {
    let prefix = ">".repeat(1000);
    for success in [false, true] {
        let mut doc = Document::new();
        doc.append(&format!("{prefix}*old\n{prefix}end* {}", "x".repeat(40)))
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
        assert_eq!(doc.work().delimiter_steps - after.delimiter_steps, 1);
        assert!(doc.work().inline_bytes - after.inline_bytes < 12);
        doc.finish();
        assert!(doc.work().inline_bytes <= doc.source().len() as u64);
    }
}

#[test]
fn thousands_of_pairs_and_historical_objects_are_consumed_once() {
    for success in [false, true] {
        let mut doc = Document::new();
        doc.append(&format!(
            "{}\n\n{}\n",
            "[p](u) ".repeat(4096),
            "*old ".repeat(4096)
        ))
        .unwrap();
        let old = *doc.link_info(LinkId(4095)).unwrap();
        doc.append(&format!(
            "{}[h](v) {}",
            "end* ".repeat(4096),
            "x".repeat(2048)
        ))
        .unwrap();
        let before = doc.work();
        for _ in 0..1000 {
            doc.append("x").unwrap();
        }
        assert_eq!(doc.work().delimiter_steps, before.delimiter_steps);
        doc.append("|b\n").unwrap();
        doc.append(if success { "-|-\n" } else { "wrong\n" })
            .unwrap();
        assert_eq!(doc.tables().len(), usize::from(success));
        assert_eq!(doc.work().delimiter_steps - before.delimiter_steps, 4096);
        assert_eq!(doc.work().label_bytes, before.label_bytes);
        assert_eq!(*doc.link_info(LinkId(4095)).unwrap(), old);
        assert_eq!(doc.links().len(), 4097);
        doc.finish();
        assert_eq!(doc.work().delimiter_steps - before.delimiter_steps, 4096);
        assert!(doc.work().inline_bytes <= doc.source().len() as u64);
    }
}

#[test]
fn long_delimiter_late_failure_and_range_models_preserve_pending_pairs() {
    let mut source = String::new();
    let mut expected = Vec::new();
    for i in 0..400 {
        let prefix = ">".repeat(i % 9);
        source.push_str(&format!("{prefix}**old *old\n"));
        let start = source.len();
        source.push_str(&format!(
            "{prefix}end* end** {}|b\r\n",
            "x".repeat(1030 + i)
        ));
        let delimiter = source.len();
        source.push_str(&format!(
            "{prefix}{}|-{}\n",
            "-".repeat(1030 + i),
            if i % 3 == 0 { "x" } else { "" }
        ));
        let end = source.len();
        if i % 3 != 0 {
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
    assert!(doc.work().inline_bytes <= source.len() as u64);
    assert!(doc.work().table_header_bytes <= source.len() as u64);
    assert!(doc.work().table_candidate_bytes <= source.len() as u64);
}

#[test]
fn deferred_commits_invalidate_old_measurements_readers_and_merged_views() {
    for success in [false, true] {
        let mut doc = Document::new();
        let mut view = FoldView::new(&doc, 1).unwrap();
        let mut merged = FoldView::new(&doc, 1).unwrap();
        let mut batch = Update::default();
        for chunk in [
            "> # title\n> *prior  \n",
            &format!("> end* {}", "x".repeat(2048)),
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
            "> *old\n> end* **live** {}|b\n> -|-\n",
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
    assert!(text.starts_with("end* live "));
    let mut search = DisplaySearch::new(&doc, "end*").unwrap();
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
