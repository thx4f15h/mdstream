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

#[test]
fn shortcut_closure_waits_without_allocating_objects_or_reference_symbols() {
    for marker in ["[", "!["] {
        for success in [false, true] {
            let mut doc = Document::new();
            doc.append(&format!("> [p](u) $p$ {marker}old\r\n"))
                .unwrap();
            let prior = *doc.link_info(LinkId(0)).unwrap();
            let refs = doc.references().to_vec();
            let math = doc.maths()[0];
            let update = doc.append(&format!("> end] {}", "x".repeat(2048))).unwrap();
            assert!(update.new_links.is_empty());
            assert_eq!(doc.references(), refs);
            assert_eq!(doc.links().len(), 1);
            doc.append("|[tail](v)\r\n").unwrap();
            let update = doc
                .append(if success { "> -|-\n" } else { "> wrong\n" })
                .unwrap();
            assert_eq!(doc.tables().len(), usize::from(success));
            assert_eq!(update.new_links, 1..if success { 2 } else { 3 });
            assert_eq!(*doc.link_info(LinkId(0)).unwrap(), prior);
            assert_eq!(doc.maths(), &[math]);
            if success {
                assert!(!doc.references().iter().any(|r| r.label == "old end"));
            } else {
                let link = doc.link_info(LinkId(1)).unwrap();
                assert_eq!(link.state, SyntaxState::Sealed);
                assert_eq!(
                    link.kind,
                    if marker == "[" {
                        LinkKind::Link
                    } else {
                        LinkKind::Image
                    }
                );
                assert_eq!(doc.text(link.label), Some("old\r\n> end"));
                let symbol = doc
                    .references()
                    .iter()
                    .find(|r| r.label == "old end")
                    .unwrap();
                assert_eq!(symbol.first_use, Some(link.source.start));
            }
            assert!(doc.finish().new_links.is_empty());
        }
    }
}

#[test]
fn closed_shortcuts_match_independent_early_pipe_cells() {
    for (open, prefix) in [("", ""), ("> ", "> "), ("> - [x] ", ">   "), ("1. ", "   ")] {
        for marker in ["[", "!["] {
            for padding in [0, 1010, 2048] {
                let prior = format!("{open}[p](u) $p$ *old {marker}prior\r\n");
                let header = format!(
                    "**中🙂** `code` $m$ &amp; {}end] {}|[tail](v)\r\n",
                    "x".repeat(padding),
                    "y".repeat(1100)
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
                for i in 0..late.links().len() {
                    let id = LinkId(i);
                    let left = late.link_info(id).unwrap();
                    let mut right = *early.link_info(id).unwrap();
                    let shift = usize::from(left.source.start >= prior.len());
                    right.source.start -= shift;
                    right.source.end -= shift;
                    right.label.start -= shift;
                    right.label.end -= shift;
                    assert_eq!(*left, right);
                    assert_eq!(late.link_destination(id), early.link_destination(id));
                }
                assert_eq!(
                    late.references()
                        .iter()
                        .map(|r| &r.label)
                        .collect::<Vec<_>>(),
                    early
                        .references()
                        .iter()
                        .map(|r| &r.label)
                        .collect::<Vec<_>>()
                );
                assert_eq!(late.maths().len(), early.maths().len());
                assert!(late.work().inline_bytes <= late.source().len() as u64);
            }
        }
    }
}

fn compare_plain(source: &str) {
    let doc = parse(source);
    let ordinary = parse(&source.replace('|', "/"));
    let span = Span::new(0, source.len());
    assert!(doc.tables().is_empty(), "{source:?}");
    assert_eq!(styles(&doc, span), styles(&ordinary, span));
    assert_eq!(
        visible(&doc, span).replace('|', "/"),
        visible(&ordinary, span)
    );
    assert_eq!(doc.blocks(), ordinary.blocks());
    assert_eq!(doc.references(), ordinary.references());
    assert_eq!(doc.links(), ordinary.links());
    for i in 0..doc.links().len() {
        assert_eq!(doc.link_info(LinkId(i)), ordinary.link_info(LinkId(i)));
    }
    assert_eq!(doc.maths(), ordinary.maths());
    assert!(doc.work().inline_bytes <= source.len() as u64);
}

#[test]
fn failures_eof_setext_and_scope_exit_commit_shortcut_once() {
    for prefix in ["", "> "] {
        for marker in ["[", "!["] {
            for ending in ["", "  ", "\\"] {
                for end in [
                    "",
                    "\r",
                    "\r\n",
                    "|b",
                    "|b\r\n",
                    "|b\nwrong\n",
                    "|b\n---\n",
                    "\n# out\n",
                ] {
                    let end = end.replace('\n', &format!("\n{prefix}"));
                    compare_plain(&format!(
                        "{prefix}a {marker}中&amp;{ending}\r\n{prefix}**🙂** end] {}{end}",
                        "x".repeat(1030)
                    ));
                }
            }
        }
    }
    compare_plain(&format!("> ![old\n> end] {}|b\n-|-\n", "x".repeat(1030)));
    compare_plain(&format!(
        "[old\nend] {}|b\n{}x|b\n",
        "x".repeat(1030),
        "-".repeat(4096)
    ));
}

#[test]
fn explicit_tails_and_syntax_cancel_before_publishing_later_objects() {
    for suffix in [
        "(u 'title')",
        "[]",
        "[ref]",
        "(bad target)",
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
            doc.append("[p](u) ![old\n").unwrap();
            let prior = *doc.link_info(LinkId(0)).unwrap();
            doc.append(&format!(
                "{}end]{suffix} [live](v) [open {}",
                "x".repeat(padding),
                "y".repeat(1100)
            ))
            .unwrap();
            let live = (0..doc.links().len())
                .map(|i| *doc.link_info(LinkId(i)).unwrap())
                .collect::<Vec<_>>();
            let before = doc.work();
            doc.append("|b\n-|-\n").unwrap();
            doc.finish();
            assert!(doc.tables().is_empty(), "{suffix:?}");
            assert_eq!(*doc.link_info(LinkId(0)).unwrap(), prior);
            for (i, info) in live.iter().enumerate() {
                assert_eq!(doc.link_info(LinkId(i)).unwrap(), info);
            }
            assert!(doc.work().table_header_bytes - before.table_header_bytes <= 4);
            compare_plain(doc.source());
        }
    }
    // An unsupported byte in a pending suffix publishes one provisional ID;
    // subsequent completion updates that same ID without replaying the suffix.
    let mut doc = Document::new();
    doc.append(&format!("![old\nend](url '_{}", "t".repeat(1100)))
        .unwrap();
    assert_eq!(doc.links().len(), 1);
    assert_eq!(
        doc.link_info(LinkId(0)).unwrap().state,
        SyntaxState::Provisional
    );
    let update = doc.append("')\n").unwrap();
    assert!(update.new_links.is_empty());
    assert_eq!(update.changed_links, 0..1);
    assert_eq!(doc.link_destination(LinkId(0)), Some("url"));
    assert_eq!(doc.link_title(LinkId(0)).unwrap().len(), 1101);
}

#[test]
fn label_limits_and_prefix_mapping_remain_bounded() {
    for length in [
        0,
        1,
        MAX_LABEL_BYTES - 2,
        MAX_LABEL_BYTES - 1,
        2048,
        1_000_000,
    ] {
        for success in [false, true] {
            let prefix = ">".repeat(1000);
            let mut doc = Document::new();
            doc.append(&format!(
                "{prefix}a ![{}\n{prefix}] {}|b\n",
                "x".repeat(length),
                "y".repeat(1040)
            ))
            .unwrap();
            assert!(doc.links().is_empty() && doc.references().is_empty());
            doc.append(&format!(
                "{prefix}{}\n",
                if success { "-|-" } else { "wrong" }
            ))
            .unwrap();
            doc.finish();
            let valid = !success && length != 0 && length < MAX_LABEL_BYTES;
            assert_eq!(doc.links().len(), usize::from(valid));
            assert_eq!(doc.references().len(), usize::from(valid));
            assert_eq!(doc.tables().len(), usize::from(success));
            if valid {
                assert_eq!(doc.references()[0].label, "x".repeat(length));
                let label = doc.link_info(LinkId(0)).unwrap().label;
                assert_eq!(visible(&doc, label), format!("{}\n", "x".repeat(length)));
            }
        }
    }
    // Earlier partial-tab continuation contributes semantic spaces to the
    // bounded raw label; its original byte remains the display source map.
    let mut doc = Document::new();
    doc.append(&format!(
        "- ![a\n\t中\n  b] {}|c\n  wrong\n",
        "x".repeat(1030)
    ))
    .unwrap();
    doc.finish();
    assert_eq!(doc.references()[0].label, "a 中 b");
    let label = doc.link_info(LinkId(0)).unwrap().label;
    assert_eq!(visible(&doc, label), "a\n  中\nb");
    assert!(
        doc.display_runs_in(label)
            .unwrap()
            .any(|r| r.text == "  " && doc.text(r.source) == Some("\t"))
    );
    let mut cursor = doc.display_cursor(label).unwrap();
    let mut mapped = String::new();
    for _ in 0..100 {
        let position = cursor.source_position();
        let mut zero = Budget::new(WorkLimit::default());
        assert_eq!(
            cursor.next(&doc, &mut zero).unwrap(),
            DisplayStep::BudgetExhausted
        );
        assert_eq!(cursor.source_position(), position);
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            // DisplayCursor includes two lookaround bytes in each charge.
            bytes: 6,
            ..Default::default()
        });
        let DisplayStep::Run(run) = cursor.next(&doc, &mut budget).unwrap() else {
            panic!()
        };
        assert!(label.start <= run.source.start && run.source.end <= label.end);
        assert!(budget.used().bytes <= 6);
        if !run.style.contains(Style::HIDDEN) {
            assert_eq!(run.link, Some(LinkId(0)));
            mapped.push_str(run.text);
        }
        assert!(cursor.source_position() > position);
        if cursor.source_position() == label.end {
            break;
        }
    }
    assert_eq!(mapped, "a\n  中\nb");
    assert_eq!(
        cursor
            .next(&doc, &mut Budget::new(WorkLimit::default()))
            .unwrap(),
        DisplayStep::Complete
    );
}

#[test]
fn discarded_labels_do_not_change_existing_first_use_or_late_definition_dependencies() {
    for success in [false, true] {
        let mut doc = Document::new();
        doc.append("[old end]: /known 'title'\n\n").unwrap();
        let symbol = doc.references()[0].clone();
        assert_eq!(symbol.first_use, None);
        doc.append(&format!("![old\nend] {}|b\n", "x".repeat(1030)))
            .unwrap();
        assert_eq!(doc.references(), std::slice::from_ref(&symbol));
        doc.append(if success { "-|-\n" } else { "wrong\n" })
            .unwrap();
        assert_eq!(doc.links().len(), usize::from(!success));
        if success {
            assert_eq!(doc.references(), &[symbol]);
        } else {
            assert_eq!(doc.link_destination(LinkId(0)), Some("/known"));
            assert_eq!(doc.link_title(LinkId(0)), Some("title"));
            assert_eq!(
                doc.references()[0].first_use,
                Some(doc.link_info(LinkId(0)).unwrap().source.start)
            );
        }
    }
    // A discarded label must not move an existing symbol's first use backward
    // or alter any old link. A later definition still uses symbol indirection.
    for success in [false, true] {
        let mut doc = Document::new();
        doc.append(&format!(
            "{}\n\n![old\nend] {}|b\n",
            "[old end] ".repeat(20_000),
            "x".repeat(1030)
        ))
        .unwrap();
        let symbol = doc.references()[0].clone();
        let first = *doc.link_info(LinkId(0)).unwrap();
        doc.append(if success { "-|-\n\n" } else { "wrong\n\n" })
            .unwrap();
        assert_eq!(doc.references(), &[symbol]);
        let before = doc.work();
        let update = doc.append("[old end]: /late 't'\n").unwrap();
        assert_eq!(update.references_changed, [SymbolId(0)]);
        assert!(update.new_links.is_empty());
        assert_eq!(doc.work().label_bytes - before.label_bytes, 7);
        assert_eq!(*doc.link_info(LinkId(0)).unwrap(), first);
        for id in [LinkId(0), LinkId(19_999)] {
            assert_eq!(doc.link_destination(id), Some("/late"));
        }
    }
}

#[test]
fn million_byte_deep_probe_does_not_revisit_labels_ancestors_or_local_semantics() {
    let prefix = ">".repeat(1000);
    for success in [false, true] {
        let mut doc = Document::new();
        doc.append(&format!(
            "{prefix}a ![old\n{prefix}**中** end] {}",
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
        assert_eq!(after.label_bytes, before.label_bytes);
        assert_eq!(after.classified_bytes, before.classified_bytes);
        assert_eq!(after.container_bytes, before.container_bytes);
        assert_eq!(after.container_steps, before.container_steps);
        assert_eq!(after.heading_steps, before.heading_steps);
        assert_eq!(doc.annotation_arena_stats(), annotations);
        assert_eq!(doc.container_arena_stats(), containers);
        assert!(doc.links().is_empty() && doc.references().is_empty());
        doc.append("|b\n").unwrap();
        doc.append(&format!(
            "{prefix}{}\n",
            if success { "-|-" } else { "wrong" }
        ))
        .unwrap();
        assert_eq!(doc.tables().len(), usize::from(success));
        assert_eq!(doc.links().len(), usize::from(!success));
        assert!(doc.work().inline_bytes - after.inline_bytes < 12);
        doc.finish();
        assert!(doc.work().inline_bytes <= doc.source().len() as u64);
    }
}

#[test]
fn long_separators_and_rejections_match_independent_range_model() {
    let mut source = String::new();
    let mut expected = Vec::new();
    for i in 0..400 {
        let prefix = ">".repeat(i % 9);
        let marker = if i % 2 == 0 { "[" } else { "![" };
        source.push_str(&format!("{prefix}a {marker}prior\n"));
        let start = source.len();
        source.push_str(&format!(
            "{prefix}end] {}{}|b\r\n",
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
    assert!(doc.work().inline_bytes <= source.len() as u64);
    assert!(doc.work().table_header_bytes <= source.len() as u64);
    assert!(doc.work().table_candidate_bytes <= source.len() as u64);
}

#[test]
fn dense_history_openers_and_columns_reuse_live_objects_without_reparsing() {
    let open = "- ".repeat(250);
    let prefix = " ".repeat(500);
    let mut doc = Document::new();
    doc.append(&format!(
        "{}\n\n{open}{}![prior\n{prefix}{}end] ",
        "[p](u) $p$ ".repeat(4096),
        "*old ".repeat(4096),
        "`code` $m$ &amp; ".repeat(4096)
    ))
    .unwrap();
    assert_eq!(doc.links().len(), 4096);
    assert_eq!(doc.maths().len(), 8192);
    let links = (0..doc.links().len())
        .map(|i| *doc.link_info(LinkId(i)).unwrap())
        .collect::<Vec<_>>();
    let maths = doc.maths().to_vec();
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("x").unwrap();
    }
    assert_eq!(doc.work().delimiter_steps, before.delimiter_steps);
    assert_eq!(doc.work().label_bytes, before.label_bytes);
    assert_eq!(doc.work().math_steps, before.math_steps);
    doc.append(&format!(
        "|{}\n{prefix}|{}\n",
        "h|".repeat(4095),
        "-:|".repeat(4096)
    ))
    .unwrap();
    assert_eq!(doc.links().len(), links.len());
    for (i, link) in links.iter().enumerate() {
        assert_eq!(doc.link_info(LinkId(i)).unwrap(), link);
    }
    assert_eq!(doc.maths(), maths);
    assert_eq!(doc.tables()[0].columns.len(), 4096);
    assert_eq!(
        doc.block(doc.tables()[0].block).unwrap().container,
        Some(ContainerId(499))
    );
    assert_eq!(doc.work().delimiter_steps - before.delimiter_steps, 4097);
    assert_eq!(doc.work().label_bytes, before.label_bytes);
    assert_eq!(doc.work().math_steps, before.math_steps);
    doc.finish();
    assert!(doc.work().inline_bytes <= doc.source().len() as u64);
}

#[test]
fn lazy_partial_tab_block_and_opaque_boundaries_retain_existing_rules() {
    for (prior, header, delimiter) in [
        ("> a [old\n", "end] ", "> -|-\n"),
        ("- [old\n", "\tend] ", "  -|-\n"),
        ("> a [old\n", "> # end] ", "> -|-\n"),
        ("> a [old\n", ">     end] ", "> -|-\n"),
        ("> a [old\n", "> end] ", "> > -|-\n"),
        ("a [old `code\n", "end`] ", "-|-\n"),
        ("a ![old $math\n", "end$] ", "-|-\n"),
    ] {
        let source = format!("{prior}{header}{}|b\n{delimiter}", "x".repeat(1030));
        assert!(parse(&source).tables().is_empty(), "{prior:?} {header:?}");
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
            "> # title\n> a ![prior \r\n",
            &format!("> end] {}", "x".repeat(2048)),
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
            "> a ![old\n> **live** end] {}|b\n> -|-\n",
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
    assert!(text.starts_with("live end] "));
    let mut search = DisplaySearch::new(&doc, "end]").unwrap();
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
