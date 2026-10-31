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

#[test]
fn explicit_suffix_waits_without_symbols_or_ids_then_commits_once_on_failure() {
    for suffix in ["(/url 'title')", "[KEY]", "[]"] {
        for marker in ["[", "!["] {
            let mut doc = Document::new();
            doc.append(&format!(
                "> [p](u) {marker}old\r\n> end]{suffix} {}",
                "x".repeat(2048)
            ))
            .unwrap();
            let prior = *doc.link_info(LinkId(0)).unwrap();
            assert_eq!(doc.links().len(), 1, "{suffix}");
            assert_eq!(doc.references().len(), 1); // Only the existing [p].
            doc.append("|[tail](v)\n").unwrap();
            let update = doc.append("> wrong\n").unwrap();
            assert_eq!(update.new_links, 1..3);
            assert_eq!(doc.link_info(LinkId(0)), Some(&prior));
            assert_eq!(doc.link_info(LinkId(1)).unwrap().state, SyntaxState::Sealed);
            assert_eq!(
                doc.link_destination(LinkId(1)),
                (suffix.starts_with('(')).then_some("/url")
            );
            let label = if suffix == "[KEY]" { "key" } else { "old end" };
            let update = doc.append(&format!("\n[{label}]: /late\n")).unwrap();
            assert!(update.new_links.is_empty());
            if !suffix.starts_with('(') {
                assert_eq!(doc.link_destination(LinkId(1)), Some("/late"));
            }
        }
    }
}

#[test]
fn explicit_suffix_headers_match_independent_early_cells() {
    for (open, prefix) in [("", ""), ("> ", "> "), ("> - [x] ", ">   "), ("1. ", "   ")] {
        for marker in ["[", "!["] {
            for suffix in ["(/url 'title')", "(a(b)c)", "()", "[KEY 中]", "[]", "[ ]"] {
                let prior = format!("{open}[p](u) $p$ *old {marker}prior\r\n");
                let header = format!(
                    "**中🙂** `code` $m$ &amp; end]{suffix} {}|[tail](v)\r\n",
                    "y".repeat(1100)
                );
                let tail = format!("{prefix}:--|--:\r\n{prefix}v|z\n");
                let late = parse(&format!("{prior}{prefix}{header}{tail}"));
                let early = parse(&format!("{prior}{prefix}|{header}{tail}"));
                assert_eq!(late.tables().len(), 1, "{open:?} {marker:?} {suffix}");
                let old = Span::new(0, prior.len());
                assert_eq!(styles(&late, old), styles(&early, old));
                assert_eq!(visible(&late, old), visible(&early, old));
                let start = prior.len() + prefix.len();
                let span = Span::new(start, start + header.len());
                let shifted = Span::new(start + 1, span.end + 1);
                assert_eq!(styles(&late, span), styles(&early, shifted), "{suffix}");
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
                assert!(late.work().inline_bytes <= late.source().len() as u64);
            }
        }
    }
}

fn compare_fallback(source: &str) {
    let doc = parse(source);
    let plain = parse(&source.replace('|', "/"));
    assert!(doc.tables().is_empty(), "unexpected table: {source:?}");
    let span = Span::new(0, source.len());
    assert_eq!(styles(&doc, span), styles(&plain, span));
    assert_eq!(visible(&doc, span).replace('|', "/"), visible(&plain, span));
    assert_eq!(doc.blocks(), plain.blocks());
    assert_eq!(doc.references(), plain.references());
    assert_eq!(doc.links(), plain.links());
    for i in 0..doc.links().len() {
        assert_eq!(doc.link_info(LinkId(i)), plain.link_info(LinkId(i)));
    }
    assert!(doc.work().inline_bytes <= source.len() as u64);
}

#[test]
fn failed_headers_eof_setext_and_scope_exit_retain_original_suffix_semantics() {
    for prefix in ["", "> "] {
        for marker in ["[", "!["] {
            for suffix in ["(u 'title')", "[KEY]", "[]", "(url 'pending", "[pending"] {
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
                    compare_fallback(&format!(
                        "{prefix}a {marker}中&amp;  \r\n{prefix}**🙂** end]{suffix} {}{end}",
                        "x".repeat(1030)
                    ));
                }
            }
        }
    }
    for suffix in ["(u)", "[key]", "[]"] {
        compare_fallback(&format!(
            "> ![old\n> end]{suffix} {}|b\n-|-\n",
            "x".repeat(1030)
        ));
    }
}

#[test]
fn suffix_syntax_invalid_input_and_later_objects_cancel_before_publication() {
    for suffix in [
        "(<url>)",
        "(u_r)",
        "(u '&amp;')",
        "(u `title`)",
        "(u \\\"title\\\")",
        "[a_b]",
        "[a&b]",
        "[^n]",
        "[a\\]b]",
        "[new[key]",
        "[key](u)",
        "[](u)",
        "(bad target)",
        "[key] $m$",
        "(u) [new](v)",
        "(u) &amp;",
        "(u) `code`",
    ] {
        for padding in [0, 1010, 2048] {
            compare_fallback(&format!(
                "[p](v) ![old\n{}end]{suffix} {}|b\n-|-\n",
                "x".repeat(padding),
                "y".repeat(1100)
            ));
        }
    }
    for (source, end, destination) in [
        (
            format!("![old\nend](url '{}", "t".repeat(1100)),
            "title')",
            Some("url"),
        ),
        (
            format!("![old\n{}end][{}", "x".repeat(200), "r".repeat(900)),
            "key]",
            None,
        ),
    ] {
        let mut doc = Document::new();
        doc.append(&source).unwrap();
        assert!(doc.links().is_empty());
        let update = doc.append("_").unwrap();
        assert_eq!(update.new_links, 0..1);
        assert_eq!(
            doc.link_info(LinkId(0)).unwrap().state,
            SyntaxState::Provisional
        );
        let update = doc.append(end).unwrap();
        assert!(update.new_links.is_empty());
        assert_eq!(update.changed_links, 0..1);
        assert_eq!(doc.link_info(LinkId(0)).unwrap().state, SyntaxState::Sealed);
        assert_eq!(doc.link_destination(LinkId(0)), destination);
    }
}

#[test]
fn destination_title_and_reference_bounds_preserve_success_and_fallback() {
    for (suffix, accepted) in [
        (format!("({})", "u".repeat(MAX_DESTINATION_BYTES)), true),
        (
            format!("({})", "u".repeat(MAX_DESTINATION_BYTES + 1)),
            false,
        ),
        (format!("(u '{}')", "t".repeat(MAX_LINK_TITLE_BYTES)), true),
        (
            format!("(u '{}')", "t".repeat(MAX_LINK_TITLE_BYTES + 1)),
            false,
        ),
        (format!("[{}]", "r".repeat(MAX_LABEL_BYTES)), true),
        (format!("[{}]", "r".repeat(MAX_LABEL_BYTES + 1)), false),
        (format!("({})", " ".repeat(MAX_LINK_SUFFIX_BYTES)), false),
    ] {
        let source = format!("![old\nend]{suffix} {}|b\n-|-\n", "x".repeat(1030));
        let doc = parse(&source);
        assert_eq!(doc.tables().len(), usize::from(accepted));
        if !accepted {
            compare_fallback(&source);
        }
        assert!(doc.work().inline_bytes <= source.len() as u64);
    }
    // Empty, whitespace and overlong old labels still accept a direct target;
    // collapsed tails only resolve if the original shortcut label is valid.
    for old in [
        String::new(),
        " ".to_owned(),
        "a".repeat(MAX_LABEL_BYTES - 1),
        "a".repeat(MAX_LABEL_BYTES),
        "a".repeat(1_000_000),
    ] {
        for suffix in ["(u)", "[]", "[key]"] {
            for success in [false, true] {
                let source = format!(
                    "a ![{old}\n]{suffix} {}|b\n{}\n",
                    "x".repeat(1030),
                    if success { "-|-" } else { "wrong" }
                );
                if success {
                    let doc = parse(&source);
                    assert_eq!(doc.tables().len(), 1);
                    assert_eq!(doc.links().len(), usize::from(suffix == "[key]"));
                } else {
                    compare_fallback(&source);
                }
            }
        }
    }
}

#[test]
fn reference_symbols_and_first_use_follow_the_selected_interpretation() {
    for suffix in ["(u 't')", "[KEY]", "[]"] {
        for success in [false, true] {
            let mut doc = Document::new();
            doc.append("[old end]: /old\n[key]: /key\n\n").unwrap();
            let symbols = doc.references().to_vec();
            doc.append(&format!("![old\nend]{suffix} {}|b\n", "x".repeat(1030)))
                .unwrap();
            assert_eq!(doc.references(), symbols);
            let suffix_at = doc.source().find(suffix).unwrap();
            let old_at = doc.source().find("![old").unwrap();
            doc.append(if success { "-|-\n" } else { "wrong\n" })
                .unwrap();
            assert_eq!(doc.references()[0].first_use, (!success).then_some(old_at));
            assert_eq!(
                doc.references()[1].first_use,
                (suffix == "[KEY]").then_some(if success { suffix_at } else { old_at })
            );
            if success && suffix != "[KEY]" {
                assert!(doc.links().is_empty());
            } else {
                assert_eq!(
                    doc.link_destination(LinkId(0)),
                    Some(match suffix {
                        "[KEY]" => "/key",
                        "[]" => "/old",
                        _ => "u",
                    })
                );
            }
        }
    }
    for success in [false, true] {
        let mut doc = Document::new();
        doc.append(&format!(
            "{}\n\n![old\nend][key] {}|b\n",
            "[key] ".repeat(20_000),
            "x".repeat(1030)
        ))
        .unwrap();
        let symbol = doc.references()[0].clone();
        let old = *doc.link_info(LinkId(0)).unwrap();
        doc.append(if success { "-|-\n\n" } else { "wrong\n\n" })
            .unwrap();
        assert_eq!(doc.references()[0], symbol);
        let before = doc.work();
        let update = doc
            .append("[key]: /late 'title'\n[key]: /ignored\n")
            .unwrap();
        assert_eq!(update.references_changed, [SymbolId(0)]);
        assert!(update.new_links.is_empty());
        assert_eq!(doc.work().label_bytes - before.label_bytes, 6);
        assert_eq!(doc.link_info(LinkId(0)), Some(&old));
        for id in [LinkId(0), LinkId(19_999), LinkId(20_000)] {
            assert_eq!(doc.link_destination(id), Some("/late"));
        }
    }
}

#[test]
fn deep_million_byte_tails_do_not_rescan_suffixes_labels_or_ancestors() {
    for suffix in ["(u 'title')", "[key]", "[]"] {
        for success in [false, true] {
            let prefix = ">".repeat(1000);
            let mut doc = Document::new();
            doc.append(&format!(
                "{prefix}a ![old\n{prefix}**中** end]{suffix} {}",
                "x".repeat(50)
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
            assert_eq!(after.label_bytes, before.label_bytes);
            assert_eq!(after.delimiter_steps, before.delimiter_steps);
            assert_eq!(after.classified_bytes, before.classified_bytes);
            assert_eq!(after.container_steps, before.container_steps);
            assert_eq!(after.container_bytes, before.container_bytes);
            assert_eq!(after.heading_steps, before.heading_steps);
            assert_eq!(doc.annotation_arena_stats(), annotations);
            assert_eq!(doc.container_arena_stats(), containers);
            assert!(doc.links().is_empty() && doc.references().is_empty());
            doc.append(&format!(
                "|b\n{prefix}{}\n",
                if success { "-|-" } else { "wrong" }
            ))
            .unwrap();
            assert_eq!(doc.tables().len(), usize::from(success));
            assert_eq!(
                doc.links().len(),
                usize::from(!success || suffix == "[key]")
            );
            assert!(doc.work().inline_bytes - after.inline_bytes < 12);
            assert!(doc.work().label_bytes - after.label_bytes < 30);
            doc.finish();
            assert!(doc.work().inline_bytes <= doc.source().len() as u64);
        }
    }
}

#[test]
fn many_objects_openers_and_columns_keep_ids_and_charge_each_opener_once() {
    let open = "- ".repeat(250);
    let prefix = " ".repeat(500);
    let mut doc = Document::new();
    doc.append(&format!(
        "{}\n\n{open}{}![old\n{prefix}{}end][key] ",
        "[p](u) $p$ ".repeat(4096),
        "*old ".repeat(4096),
        "`code` $m$ &amp; ".repeat(4096)
    ))
    .unwrap();
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
    doc.append(&format!(
        "|{}\n{prefix}|{}\n",
        "h|".repeat(4095),
        "-:|".repeat(4096)
    ))
    .unwrap();
    assert_eq!(doc.links().len(), links.len() + 1);
    for (i, old) in links.iter().enumerate() {
        assert_eq!(doc.link_info(LinkId(i)), Some(old));
    }
    assert_eq!(doc.maths(), maths);
    assert_eq!(doc.tables()[0].columns.len(), 4096);
    assert_eq!(
        doc.block(doc.tables()[0].block).unwrap().container,
        Some(ContainerId(499))
    );
    assert_eq!(doc.work().delimiter_steps - before.delimiter_steps, 4097);
    assert_eq!(doc.work().label_bytes - before.label_bytes, 3);
    assert_eq!(doc.work().math_steps, before.math_steps);
    assert!(doc.work().inline_bytes <= doc.source().len() as u64);
}

#[test]
fn generated_suffixes_and_long_separators_match_source_range_model() {
    let mut source = String::new();
    let mut expected = Vec::new();
    for i in 0..400 {
        let prefix = ">".repeat(i % 9);
        let marker = if i % 2 == 0 { "[" } else { "![" };
        let suffix = ["(u)", "[key]", "[]", "(a(b)c 'title')"][i % 4];
        source.push_str(&format!("{prefix}a {marker}old\n"));
        let start = source.len();
        source.push_str(&format!(
            "{prefix}end]{suffix} {}{}|b\r\n",
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
fn partial_tab_labels_map_to_raw_source_while_lazy_headers_and_opaque_closures_cancel() {
    for success in [false, true] {
        let mut doc = Document::new();
        doc.append(&format!("- ![a\n\t中\n  b][key] {}|c\n", "x".repeat(1030)))
            .unwrap();
        assert!(doc.links().is_empty() && doc.references().is_empty());
        doc.append(if success { "  -|-\n" } else { "  wrong\n" })
            .unwrap();
        assert_eq!(doc.tables().len(), usize::from(success));
        let info = doc.link_info(LinkId(0)).unwrap();
        if success {
            assert_eq!(doc.text(info.label), Some("key"));
        } else {
            assert_eq!(doc.text(info.label), Some("a\n\t中\n  b"));
            assert!(doc.references().iter().any(|r| r.label == "a 中 b"));
            assert_eq!(visible(&doc, info.label), "a\n  中\nb");
        }
    }
    for (prior, header, delimiter) in [
        ("> a [old\n", "end](u) ", "> -|-\n"),
        ("- [old\n", "\tend][key] ", "  -|-\n"),
        ("> a [old\n", "> # end](u) ", "> -|-\n"),
        ("a [old `code\n", "end`](u) ", "-|-\n"),
        ("a ![old $math\n", "end$][key] ", "-|-\n"),
    ] {
        let doc = parse(&format!(
            "{prior}{header}{}|b\n{delimiter}",
            "x".repeat(1030)
        ));
        assert!(doc.tables().is_empty(), "{prior:?} {header:?}");
    }
}

#[test]
fn suffix_commits_invalidate_old_measurements_readers_and_merged_views() {
    for success in [false, true] {
        let mut doc = Document::new();
        let mut view = FoldView::new(&doc, 1).unwrap();
        let mut merged = FoldView::new(&doc, 1).unwrap();
        let mut batch = Update::default();
        for chunk in [
            "> # title\n> a ![prior \r\n",
            &format!("> end][key] {}", "x".repeat(2048)),
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
fn suffix_budgeted_preview_search_and_folded_layout_preserve_source_mapping() {
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let update = doc
        .append(&format!(
            "> a ![old\n> **live** end](u) {}|b\n> -|-\n",
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
    assert!(text.starts_with("live end](u) "));
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
