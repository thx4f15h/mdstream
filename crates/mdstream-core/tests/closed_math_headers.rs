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

fn payload(doc: &Document, id: MathId, mapped: bool) -> String {
    let math = doc.math(id).unwrap();
    let mut cursor = if mapped {
        doc.math_payload(id)
    } else {
        doc.math_content(id)
    }
    .unwrap();
    let mut zero = Budget::new(WorkLimit::default());
    let position = cursor.source_position();
    assert_eq!(cursor.advance(&mut zero), Err(Completion::BudgetExhausted));
    assert_eq!(cursor.source_position(), position);
    assert_eq!(zero.used(), WorkLimit::default());
    let mut result = String::new();
    let mut charged = 0;
    loop {
        let mut budget = Budget::new(WorkLimit {
            queries: 2,
            objects: 2,
            bytes: 4,
            ..Default::default()
        });
        let mut objects = doc.maths_in(math.source).unwrap();
        assert_eq!(objects.advance(&mut budget).unwrap(), Some((id, *math)));
        let Some(chunk) = cursor.advance(&mut budget).unwrap() else {
            break;
        };
        assert!(math.content.start <= chunk.source.start && chunk.source.end <= math.content.end);
        assert_eq!(budget.used().bytes, chunk.source.len());
        assert!(budget.used().queries <= 2 && budget.used().objects <= 2);
        if !chunk.text.is_empty() && doc.text(chunk.source) != Some(chunk.text) {
            assert_eq!(doc.text(chunk.source), Some("\t"));
            assert_eq!(chunk.text, "  ");
        }
        charged += budget.used().bytes;
        result.push_str(chunk.text);
    }
    assert_eq!(charged, math.content.len());
    result
}

#[test]
fn delayed_math_payload_preserves_prefix_gaps_partial_tabs_and_shared_budget() {
    for success in [false, true] {
        let mut doc = Document::new();
        doc.append("- $$ first\r\n\t中\r\n").unwrap();
        doc.append(&format!("  $m$ end$$ {}|$tail$\r\n", "x".repeat(1030)))
            .unwrap();
        assert!(doc.maths().is_empty());
        doc.append(if success { "  -|-\n" } else { "  wrong\n" })
            .unwrap();
        doc.finish();
        assert_eq!(doc.maths().len(), 2);
        assert_eq!(
            payload(&doc, MathId(0), true),
            if success {
                "m"
            } else {
                " first\r\n  中\r\n$m$ end"
            }
        );
        assert_eq!(payload(&doc, MathId(1), true), "tail");
        for id in [MathId(0), MathId(1)] {
            assert_eq!(
                payload(&doc, id, false),
                doc.text(doc.math(id).unwrap().content).unwrap()
            );
        }
        let mut objects = doc.maths_in(Span::new(0, doc.source().len())).unwrap();
        let mut zero = Budget::new(WorkLimit::default());
        assert_eq!(objects.advance(&mut zero), Err(Completion::BudgetExhausted));
        for id in [MathId(0), MathId(1)] {
            let mut budget = Budget::new(WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            });
            assert_eq!(
                objects.advance(&mut budget).unwrap(),
                Some((id, *doc.math(id).unwrap()))
            );
        }
    }
}

#[test]
fn lazy_partial_tab_and_block_boundaries_keep_their_existing_rules() {
    for (prior, header, delimiter) in [
        ("> a $old\n", "end$ ", "> -|-\n"),
        ("- $old\n", "\tend$ ", "  -|-\n"),
        ("> a $old\n", "> # end$ ", "> -|-\n"),
        ("> a $old\n", ">     end$ ", "> -|-\n"),
        ("> a $old\n", "> end$ ", "> > -|-\n"),
    ] {
        let source = format!("{prior}{header}{}|b\n{delimiter}", "x".repeat(1030));
        assert!(parse(&source).tables().is_empty(), "{prior:?} {header:?}");
    }
}

#[test]
fn closed_math_matches_independent_early_pipe_cells() {
    for (open, prefix) in [("", ""), ("> ", "> "), ("> - [x] ", ">   "), ("1. ", "   ")] {
        for marker in ["$", "$$"] {
            let local = if marker == "$" { "$$" } else { "$" };
            for padding in [0, 1010, 2048] {
                let prior = format!("{open}[p](u) $p$ [^p] *old {marker} prior\r\n");
                let header = format!(
                    "[h](v) {local}中🙂{local} [^n] &amp; {}end{marker} {}|$tail$\r\n",
                    "x".repeat(padding),
                    "中🙂".repeat(150)
                );
                let tail = format!("{prefix}:--|--:\r\n{prefix}v|z\n");
                let late = parse(&format!("{prior}{prefix}{header}{tail}"));
                let early = parse(&format!("{prior}{prefix}|{header}{tail}"));
                assert_eq!(late.tables().len(), 1, "{open:?} {marker:?} {padding}");
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
                for (left, right) in late.maths().iter().zip(early.maths()) {
                    assert_eq!(left.kind, right.kind);
                    assert_eq!(late.text(left.content), early.text(right.content));
                    let shift = usize::from(left.source.start >= prior.len());
                    assert_eq!(
                        right.source,
                        Span::new(left.source.start + shift, left.source.end + shift)
                    );
                }
                assert_eq!(
                    late.footnote_references().len(),
                    early.footnote_references().len()
                );
                assert!(
                    late.work().inline_bytes
                        <= late.source().len() as u64 + late.work().table_bytes
                );
            }
        }
    }
}

#[test]
fn cross_line_object_waits_and_existing_ids_are_never_retracted() {
    for success in [false, true] {
        let mut doc = Document::new();
        doc.append("> [p](u) $p$ [^p] $$ prior\r\n").unwrap();
        let link = *doc.link_info(LinkId(0)).unwrap();
        let math = doc.maths()[0];
        let footnote = doc.footnote_references()[0];
        let old_start = doc.source().find("$$").unwrap();
        let update = doc
            .append(&format!(
                "> [h](v) $m$ [^n] &amp; end$$ {}",
                "x".repeat(2048)
            ))
            .unwrap();
        assert!(update.new_maths.is_empty());
        assert_eq!(doc.maths(), &[math]);
        assert_eq!(doc.links().len(), 1);
        assert!(
            !styles(&doc, Span::new(old_start, old_start + 2))
                .iter()
                .any(|s| s.0.contains(Style::HIDDEN))
        );
        let block = BlockId(doc.blocks().len() - 1);
        doc.append("|$tail$ [t](w) [^t]\n").unwrap();
        let update = doc
            .append(if success { "> -|-\n" } else { "> wrong\n" })
            .unwrap();
        assert_eq!(update.new_maths, 1..3);
        assert_eq!(doc.tables().len(), usize::from(success));
        assert_eq!(*doc.link_info(LinkId(0)).unwrap(), link);
        assert_eq!(*doc.math(MathId(0)).unwrap(), math);
        assert_eq!(doc.footnote_references()[0], footnote);
        if success {
            assert_eq!(doc.tables()[0].block, block);
            assert_eq!(doc.text(doc.math(MathId(1)).unwrap().content), Some("m"));
        } else {
            assert_eq!(doc.math(MathId(1)).unwrap().source.start, old_start);
        }
        assert_eq!(doc.text(doc.math(MathId(2)).unwrap().content), Some("tail"));
        assert!(
            doc.maths()
                .windows(2)
                .all(|m| m[0].source.end <= m[1].source.start)
        );
        assert!(doc.finish().new_maths.is_empty());
    }
}

fn compare_plain(source: &str) {
    let doc = parse(source);
    let ordinary = parse(&source.replace('|', "/"));
    let span = Span::new(0, source.len());
    assert!(doc.tables().is_empty());
    assert_eq!(styles(&doc, span), styles(&ordinary, span));
    assert_eq!(
        visible(&doc, span).replace('|', "/"),
        visible(&ordinary, span)
    );
    assert_eq!(doc.blocks(), ordinary.blocks());
    assert_eq!(doc.maths(), ordinary.maths());
    assert_eq!(doc.links(), ordinary.links());
    assert!(doc.work().inline_bytes <= source.len() as u64);
}

#[test]
fn no_pipe_eof_setext_failed_separator_and_scope_exit_commit_once() {
    for prefix in ["", "> "] {
        for marker in ["$", "$$"] {
            for end in [
                "",
                "\r",
                "\r\n",
                "|b",
                "|b\r\n",
                "|b\nwrong\n",
                "|b\n---\n",
                "\n# end\n",
            ] {
                let end = end.replace('\n', &format!("\n{prefix}"));
                compare_plain(&format!(
                    "{prefix}a {marker}中\\{marker} raw\r\n{prefix}end{marker} {}{end}",
                    "x".repeat(1030)
                ));
            }
        }
    }
    compare_plain(&format!(
        "> a $ prior\n> end$ {}|b\n-|-\n",
        "x".repeat(1030)
    ));
}

#[test]
fn syntax_suffix_flushes_old_math_before_any_new_object_and_never_restarts() {
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
            doc.append("$prior$ a [old *old $$ prior\r\n").unwrap();
            let original = doc.maths()[0];
            let update = doc
                .append(&format!(
                    "{}end$$ {suffix} &amp; [live](v) $$new {}",
                    "x".repeat(padding),
                    "y".repeat(1100)
                ))
                .unwrap();
            assert_eq!(update.new_maths.start, 1);
            assert!(update.new_maths.end >= 2);
            let closed = doc.maths()[1];
            let before = doc.work();
            doc.append("|b$$\n-|-\n").unwrap();
            doc.finish();
            assert!(doc.tables().is_empty(), "{suffix:?}");
            assert!(doc.work().table_header_bytes - before.table_header_bytes <= 4);
            assert_eq!(doc.maths()[0], original);
            assert_eq!(doc.maths()[1], closed);
            assert!(
                doc.maths()
                    .windows(2)
                    .all(|m| m[0].source.end <= m[1].source.start)
            );
            compare_plain(doc.source());
        }
    }
}

#[test]
fn million_byte_deep_probe_visits_only_new_bytes_and_reparses_only_the_row() {
    let prefix = ">".repeat(1000);
    for success in [false, true] {
        let mut doc = Document::new();
        doc.append(&format!(
            "{prefix}a ${}\n{prefix}end$ {}",
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
        assert_eq!(after.math_steps, before.math_steps);
        assert_eq!(after.delimiter_steps, before.delimiter_steps);
        assert_eq!(after.classified_bytes, before.classified_bytes);
        assert_eq!(after.container_bytes, before.container_bytes);
        assert_eq!(after.container_steps, before.container_steps);
        assert_eq!(after.heading_steps, before.heading_steps);
        assert_eq!(doc.annotation_arena_stats(), annotations);
        assert_eq!(doc.container_arena_stats(), containers);
        assert!(doc.maths().is_empty());
        doc.append("|b\n").unwrap();
        doc.append(&format!(
            "{prefix}{}\n",
            if success { "-|-" } else { "wrong" }
        ))
        .unwrap();
        assert_eq!(doc.tables().len(), usize::from(success));
        assert_eq!(doc.maths().len(), usize::from(!success));
        if success {
            assert_eq!(doc.work().inline_bytes - after.inline_bytes, 1_000_046);
            assert_eq!(doc.work().math_steps - after.math_steps, 2);
        } else {
            assert!(doc.work().inline_bytes - after.inline_bytes < 12);
            assert_eq!(doc.work().math_steps - after.math_steps, 1);
        }
        doc.finish();
        assert!(doc.work().inline_bytes <= doc.source().len() as u64 + doc.work().table_bytes);
    }
}

#[test]
fn long_separator_and_syntax_suffix_follow_independent_range_model() {
    let mut source = String::new();
    let mut expected = Vec::new();
    for i in 0..400 {
        let prefix = ">".repeat(i % 9);
        let marker = if i % 2 == 0 { "$" } else { "$$" };
        source.push_str(&format!("{prefix}a {marker} prior\n"));
        let start = source.len();
        source.push_str(&format!(
            "{prefix}end{marker} {}{}|b\r\n",
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
fn dense_old_objects_and_openers_are_not_revisited_or_renumbered() {
    let open = "- ".repeat(250);
    let prefix = " ".repeat(500);
    let mut doc = Document::new();
    doc.append(&format!(
        "{}\n\n{open}{} $$ prior\n{prefix}{}end$$ ",
        "$p$ [p](u) ".repeat(4096),
        "*old ".repeat(4096),
        "[h](v) $m$ [^n] ".repeat(4096)
    ))
    .unwrap();
    assert_eq!(doc.links().len(), 4096);
    assert_eq!(doc.maths().len(), 4096);
    let old = doc.maths().to_vec();
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("x").unwrap();
    }
    assert_eq!(doc.work().delimiter_steps, before.delimiter_steps);
    assert_eq!(doc.work().math_steps, before.math_steps);
    assert_eq!(doc.work().label_bytes, before.label_bytes);
    doc.append(&format!(
        "|{}\n{prefix}|{}\n",
        "h|".repeat(4095),
        "-:|".repeat(4096)
    ))
    .unwrap();
    assert_eq!(doc.links().len(), 8192);
    assert_eq!(doc.maths().len(), 8192);
    assert_eq!(&doc.maths()[..4096], old);
    assert_eq!(doc.footnote_references().len(), 4096);
    assert_eq!(doc.tables()[0].columns.len(), 4096);
    assert_eq!(
        doc.block(doc.tables()[0].block).unwrap().container,
        Some(ContainerId(499))
    );
    assert_eq!(doc.work().math_steps - before.math_steps, 8194);
    doc.finish();
    assert!(doc.work().inline_bytes <= doc.source().len() as u64 + doc.work().table_bytes);
}

#[test]
fn deferred_commits_invalidate_old_measurements_readers_and_merged_views() {
    for success in [false, true] {
        let mut doc = Document::new();
        let mut view = FoldView::new(&doc, 1).unwrap();
        let mut merged = FoldView::new(&doc, 1).unwrap();
        let mut batch = Update::default();
        for chunk in [
            "> # title\n> a $ prior \r\n",
            &format!("> end$ {}", "x".repeat(2048)),
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
            "> a $ old\n> **live** end$ {}|b\n> -|-\n",
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
    assert!(text.starts_with("live end$ "));
    let mut search = DisplaySearch::new(&doc, "end$").unwrap();
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
