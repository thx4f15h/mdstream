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
fn isolated_continuations_match_deferred_headers_including_previous_line() {
    for (open, next) in [("", ""), ("> ", "> "), ("> - [x] ", ">   "), ("1. ", "   ")] {
        for ending in ["", " ", "  ", "\\"] {
            let prior = format!("{open}**prior** [p](u) `code` $m$ [^n]{ending}\r\n");
            for first in ["**first** &amp; ", "*open ", "`open ", "[open ", "$open "] {
                let header = format!("{first}{}|close*` ](v) $\r\n", "中🙂".repeat(160));
                let tail = format!("{next}:--|--:\n{next}v|z\n");
                let late = parse(&format!("{prior}{next}{header}{tail}"));
                let early = parse(&format!("{prior}{next}|{header}{tail}"));
                assert_eq!(late.tables().len(), 1, "{open:?} {first:?}");
                let old = Span::new(0, prior.len());
                assert_eq!(styles(&late, old), styles(&early, old));
                assert_eq!(visible(&late, old), visible(&early, old));
                let start = prior.len() + next.len();
                let span = Span::new(start, start + header.len());
                let shifted = Span::new(span.start + 1, span.end + 1);
                assert_eq!(styles(&late, span), styles(&early, shifted), "{first:?}");
                assert_eq!(visible(&late, span), visible(&early, shifted));
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
fn first_cell_and_prior_object_ids_are_published_once() {
    for prefix in ["", "> "] {
        let mut doc = Document::new();
        doc.append(&format!("{prefix}[p](u) $a$ [^p]\n")).unwrap();
        let old = *doc.link_info(LinkId(0)).unwrap();
        let header_start = doc.source().len();
        doc.append(&format!("{prefix}[h](v) $b$ [^h] {}", "x".repeat(2048)))
            .unwrap();
        assert_eq!(doc.links().len(), 2);
        assert_eq!(doc.maths().len(), 2);
        assert_eq!(doc.footnote_references().len(), 2);
        let first = *doc.link_info(LinkId(1)).unwrap();
        let previous = doc.blocks()[0].clone();
        assert_eq!(doc.blocks().last().unwrap().source.end, doc.source().len());
        doc.append("|[t](w) $c$ [^t]\n").unwrap();
        assert_eq!(doc.links().len(), 2);
        let work = doc.work();
        let update = doc.append(&format!("{prefix}-|-\n")).unwrap();
        assert_eq!(update.new_links, 2..3);
        assert_eq!(update.new_maths, 2..3);
        assert_eq!(update.new_footnote_references, 2..3);
        assert_eq!(*doc.link_info(LinkId(0)).unwrap(), old);
        assert_eq!(*doc.link_info(LinkId(1)).unwrap(), first);
        assert_eq!(doc.footnotes()[2].number, Some(3));
        assert_eq!(doc.blocks()[0].source, Span::new(0, header_start));
        assert_eq!(doc.blocks()[0].container, previous.container);
        assert_eq!(doc.tables()[0].source.start, header_start);
        assert_eq!(doc.tables()[0].block, BlockId(1));
        assert!(doc.work().inline_bytes - work.inline_bytes < 30);
        doc.finish();
    }
}

#[test]
fn failure_and_eof_restore_soft_and_hard_breaks_without_replaying_prefix() {
    for prefix in ["", "> "] {
        for ending in ["", " ", "  ", "\\"] {
            for suffix in ["", "\r\n", "\r\nwrong\n", "\r\n---\n"] {
                let prior = format!("{prefix}prior{ending}\r\n");
                let head = format!("{prefix}*{}", "x".repeat(2048));
                let suffix = suffix.replace('\n', &format!("\n{prefix}"));
                let source = format!("{prior}{head}|end* [x](u){suffix}");
                let doc = parse(&source);
                // Replacing a literal pipe prevents the table probe; all byte
                // styles, object IDs, and final block ranges should agree.
                let ordinary = parse(&source.replace('|', "/"));
                assert!(doc.tables().is_empty());
                let all = Span::new(0, source.len());
                assert_eq!(styles(&doc, all), styles(&ordinary, all));
                assert_eq!(doc.blocks(), ordinary.blocks());
                assert!(doc.work().inline_bytes <= source.len() as u64);
                let newline = Span::new(prior.len() - 1, prior.len());
                let style = styles(&doc, newline)[0].0;
                assert!(style.contains(if ending.len() == 2 || ending == "\\" {
                    Style::HARD_BREAK
                } else {
                    Style::SOFT_BREAK
                }));
            }
        }
    }
}

#[test]
fn ordinary_long_lines_publish_content_immediately_and_defer_only_leading_break() {
    for prefix in ["", "> "] {
        let mut doc = Document::new();
        let prior = format!("{prefix}prior  \n");
        doc.append(&prior).unwrap();
        doc.append(&format!("{prefix}{}", "x".repeat(2048)))
            .unwrap();
        let old_key = doc.tile_key(TileId(0)).unwrap();
        for chunk in [" **live**", " [x](u)", " $m$", " [^n]", " &amp;"] {
            let before = doc.work();
            doc.append(chunk).unwrap();
            assert_eq!(
                doc.work().inline_bytes - before.inline_bytes,
                chunk.len() as u64
            );
            assert!(doc.pending().is_empty());
            assert_eq!(doc.blocks().last().unwrap().source.end, doc.source().len());
        }
        assert_eq!(doc.links().len(), 1);
        assert_eq!(doc.maths().len(), 1);
        assert_eq!(doc.footnote_references().len(), 1);
        assert!(visible(&doc, Span::new(prior.len(), doc.source().len())).contains("live x m"));
        let newline = Span::new(prior.len() - 1, prior.len());
        assert!(!styles(&doc, newline)[0].0.contains(Style::HARD_BREAK));
        doc.append("\n").unwrap();
        assert!(styles(&doc, newline)[0].0.contains(Style::HARD_BREAK));
        assert_ne!(doc.tile_key(TileId(0)).unwrap(), old_key);
        doc.append(&format!("{prefix}next\n")).unwrap();
        doc.finish();
        assert!(doc.tables().is_empty());
        assert!(doc.work().inline_bytes <= doc.source().len() as u64);
    }
}

#[test]
fn open_cross_line_candidates_keep_their_existing_semantics() {
    // Close-only emphasis now has an isolated path; bidirectional closers
    // retain this test's immediate-publication and rejection contract.
    for prefix in ["", "> "] {
        for (open, close, style) in [
            ("*open", "end*next ", Style::EMPHASIS),
            ("**open", "end**next ", Style::STRONG),
            ("~~open", "end~~next ", Style::STRIKE),
            ("`open", "end` &amp; ", Style::CODE),
        ] {
            let prior = format!("{prefix}{open}\n");
            let mut doc = Document::new();
            doc.append(&prior).unwrap();
            doc.append(&format!("{prefix}{close}{}", "x".repeat(2048)))
                .unwrap();
            let at = prior.find("open").unwrap();
            assert!(styles(&doc, Span::new(at, at + 1))[0].0.contains(style));
            doc.append(&format!("|b\n{prefix}-|-\n")).unwrap();
            doc.finish();
            assert!(doc.tables().is_empty());
        }
    }
    for (open, close) in [("[open", "end](u) "), ("$open", "end$ ")] {
        let source = format!("{open}\n{close}{}|b\n-|-\n", "x".repeat(2048));
        let doc = parse(&source);
        let ordinary = parse(&source.replace('|', "/"));
        assert!(doc.tables().is_empty());
        assert_eq!(
            styles(&doc, Span::new(0, source.len())),
            styles(&ordinary, Span::new(0, source.len()))
        );
        assert_eq!(doc.links(), ordinary.links());
        assert_eq!(doc.maths(), ordinary.maths());
    }
}

#[test]
fn mismatch_sibling_lazy_and_long_delimiter_failure_restore_boundary_once() {
    for (prior, next, delimiter) in [
        ("> prior  \n", "> ", "-|-\n"),
        ("- prior  \n", "  ", "- -|-\n"),
        ("> prior  \n", "> ", "> > -|-\n"),
    ] {
        let source = format!("{prior}{next}{}|b\n{delimiter}", "x".repeat(2048));
        let doc = parse(&source);
        assert!(doc.tables().is_empty());
        assert!(
            styles(&doc, Span::new(prior.len() - 1, prior.len()))[0]
                .0
                .contains(Style::HARD_BREAK)
        );
    }
    let lazy = parse(&format!("> prior\n{}|b\n> -|-\n", "x".repeat(2048)));
    assert!(lazy.tables().is_empty());
    let mut doc = Document::new();
    doc.append(&format!(
        "> prior\\\n> [h](u) {}|b\n> {}",
        "x".repeat(2048),
        "-".repeat(4096)
    ))
    .unwrap();
    doc.append("x").unwrap();
    assert_eq!(doc.links().len(), 1);
    assert!(
        styles(&doc, Span::new(8, 9))[0]
            .0
            .contains(Style::HARD_BREAK)
    );
    let work = doc.work();
    for _ in 0..1000 {
        doc.append("|").unwrap();
    }
    assert_eq!(doc.work().classified_bytes, work.classified_bytes);
    assert_eq!(doc.work().table_header_bytes, work.table_header_bytes);
    assert_eq!(doc.work().inline_bytes - work.inline_bytes, 1000);
    doc.finish();
}

#[test]
fn deep_million_byte_continuation_uses_constant_candidate_storage_and_new_bytes_only() {
    let prefix = ">".repeat(1000);
    for success in [false, true] {
        let mut doc = Document::new();
        doc.append(&format!("{prefix}prior\n{prefix}{}", "x".repeat(40)))
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
        doc.append("\n").unwrap();
        let before = doc.work();
        doc.append(&format!(
            "{prefix}{}\n",
            if success { "-|-" } else { "wrong" }
        ))
        .unwrap();
        assert_eq!(doc.tables().len(), usize::from(success));
        assert!(doc.work().inline_bytes - before.inline_bytes <= 1_000_010);
        assert_eq!(doc.container_arena_stats(), containers);
        doc.finish();
        assert!(doc.work().inline_bytes <= doc.source().len() as u64);
        assert!(doc.work().table_header_bytes <= doc.source().len() as u64);
    }
}

#[test]
fn repeated_no_pipe_continuations_and_huge_open_stack_do_not_rescan_history() {
    let mut doc = Document::new();
    doc.append("prior\n").unwrap();
    let line = format!("{}\n", "x".repeat(2048));
    for _ in 0..1000 {
        doc.append(&line).unwrap();
    }
    let before = doc.work();
    doc.append(&line).unwrap();
    assert_eq!(doc.blocks().len(), 1);
    assert_eq!(doc.blocks()[0].source.end, doc.source().len());
    assert_eq!(
        doc.work().inline_bytes - before.inline_bytes,
        line.len() as u64
    );
    assert_eq!(
        doc.work().table_header_bytes - before.table_header_bytes,
        line.len() as u64
    );
    doc.finish();
    assert_eq!(doc.work().inline_bytes, doc.source().len() as u64);

    let mut doc = Document::new();
    doc.append(&format!("> {}\n", "*open ".repeat(4096)))
        .unwrap();
    let before = doc.work();
    for _ in 0..100 {
        doc.append(&format!("> {}|b\n", "x".repeat(2048))).unwrap();
    }
    assert_eq!(doc.work().delimiter_steps, before.delimiter_steps);
    assert!(doc.tables().is_empty());
    doc.finish();
    assert_eq!(doc.work().delimiter_steps - before.delimiter_steps, 4096);
}

#[test]
fn generated_inherited_tables_match_independent_ranges_and_scope_model() {
    let mut source = String::new();
    let mut expected = Vec::new();
    for i in 0..400 {
        let prefix = ">".repeat(i % 9);
        source.push_str(&format!("{prefix}prior\n"));
        let start = source.len();
        source.push_str(&format!("{prefix}{}|b\r\n", "x".repeat(1024 + i)));
        let delimiter_start = source.len();
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
                Span::new(delimiter_start, delimiter_end),
                start + prefix.len(),
            ));
        }
        source.push('\n');
    }
    let doc = parse(&source);
    assert_eq!(doc.tables().len(), expected.len());
    for (table, (span, delimiter, cell)) in doc.tables().iter().zip(expected) {
        assert_eq!(table.source, span);
        assert_eq!(table.delimiter, delimiter);
        assert_eq!(table.cell(0, 0).unwrap().source.start, cell);
        assert_eq!(doc.block(table.block).unwrap().source, span);
    }
    assert!(doc.work().inline_bytes <= source.len() as u64);
    assert!(doc.work().table_header_bytes <= source.len() as u64);
}

#[test]
fn failure_invalidates_prior_break_measurements_and_confirmation_restores_root_extent() {
    for success in [false, true] {
        let mut doc = Document::new();
        let mut view = FoldView::new(&doc, 1).unwrap();
        let mut merged = FoldView::new(&doc, 1).unwrap();
        let mut batch = Update::default();
        let prior = "# title\nprior  \n";
        for chunk in [prior, &"x".repeat(2048), "|b\n"] {
            let update = doc.append(chunk).unwrap();
            view.apply(&doc, &update).unwrap();
            batch.merge(update).unwrap();
        }
        let mut heights = HeightIndex::default();
        heights.rebuild(&doc).unwrap();
        let old = heights.ticket(TileId(0)).unwrap();
        let update = doc
            .append(if success { "-|-\n" } else { "wrong\n" })
            .unwrap();
        heights.apply(&doc, &update).unwrap();
        assert_eq!(heights.measure(old, 2), Err(LayoutError::StaleMeasurement));
        view.apply(&doc, &update).unwrap();
        batch.merge(update).unwrap();
        merged.apply(&doc, &batch).unwrap();
        assert_eq!(view.heights().total_rows(), merged.heights().total_rows());
        if success {
            assert_eq!(doc.blocks()[1].source.end, prior.len());
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
                    view.hidden_at(&doc, 1500, &mut budget).unwrap(),
                    FoldLookup::Hidden(_)
                ));
            }
        }
    }
}

#[test]
fn deep_lists_and_dense_first_cell_objects_keep_scope_and_symbol_work() {
    let open = "- ".repeat(250);
    let next = " ".repeat(500);
    let mut doc = Document::new();
    doc.append(&format!(
        "{open}prior\n{next}{}",
        "[a](u) $m$ [^n] ".repeat(4096)
    ))
    .unwrap();
    assert_eq!(doc.links().len(), 4096);
    assert_eq!(doc.maths().len(), 4096);
    assert_eq!(doc.footnote_references().len(), 4096);
    doc.append(&format!("|{}\n", "h|".repeat(4095))).unwrap();
    let before = doc.work();
    let update = doc
        .append(&format!("{next}|{}\n", "-:|".repeat(4096)))
        .unwrap();
    assert_eq!(update.new_links, 4096..4096);
    assert_eq!(update.new_maths, 4096..4096);
    assert_eq!(update.new_footnote_references, 4096..4096);
    assert_eq!(doc.work().label_bytes, before.label_bytes);
    assert_eq!(doc.work().footnote_steps, before.footnote_steps);
    assert_eq!(doc.work().math_steps, before.math_steps);
    doc.finish();
    assert_eq!(doc.container_count(), 500);
    let table = &doc.tables()[0];
    assert_eq!(table.columns.len(), 4096);
    assert_eq!(
        doc.block(table.block).unwrap().container,
        Some(ContainerId(499))
    );
    assert_eq!(table.rows()[0].cells.len(), 4096);
    assert!(doc.work().container_bytes < 4 * doc.source().len() as u64);
}

#[test]
fn confirmed_inherited_header_preview_and_folding_obey_shared_budget() {
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let update = doc
        .append(&format!("> prior\n> {}|b\n> -|-\n", "x".repeat(1_000_000)))
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
    for frame in 0..100 {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 8,
            ..Default::default()
        });
        match preview.next(&doc, &mut budget).unwrap() {
            TablePreviewStep::Run { run, .. } => text.push_str(run.text),
            TablePreviewStep::BudgetExhausted => {}
            TablePreviewStep::Complete(end) => {
                assert_eq!(end, TablePreviewEnd::SourceLimit);
                break;
            }
        }
        assert!(budget.used().bytes <= 8);
        assert!(frame < 99);
    }
    assert_eq!(text, "x".repeat(62));
    view.set_collapsed(&doc, FoldId::Table(TableId(0)), true)
        .unwrap();
    let update = doc.append("> y|z\n").unwrap();
    view.apply(&doc, &update).unwrap();
    view.set_collapsed(&doc, FoldId::Container(ContainerId(0)), true)
        .unwrap();
    let mut sink = PlaceholderOnly::default();
    let mut pass = LayoutPass::folded(&doc, &view, TileId(0), 10).unwrap();
    for _ in 0..20 {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            tiles: 1,
            objects: 1,
            bytes: 0,
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

#[derive(Default)]
struct PlaceholderOnly(usize);

impl LayoutSink for PlaceholderOnly {
    fn begin_tile(&mut self, _: TileId) {}
    fn object(&mut self, _: LinkId, _: &LinkInfo) {
        panic!("hidden object was read");
    }
    fn end_tile(&mut self, _: TileId) -> u64 {
        1
    }
    fn text(&mut self, _run: RenderRun<'_>) {
        panic!("hidden header was read");
    }
}

impl FoldLayoutSink for PlaceholderOnly {
    fn placeholder(&mut self, _placeholder: FoldPlaceholder) {
        self.0 += 1;
    }
}

#[test]
fn million_byte_leading_break_commits_one_range_and_invalidates_old_tiles() {
    let mut doc = Document::new();
    doc.append(&format!(
        "prior{}\n{}|b\n",
        " ".repeat(1_000_000),
        "x".repeat(2048)
    ))
    .unwrap();
    let mut heights = HeightIndex::default();
    heights.rebuild(&doc).unwrap();
    let ticket = heights.ticket(TileId(1000)).unwrap();
    let key = doc.tile_key(TileId(1000)).unwrap();
    let before = doc.work();
    let nodes = doc.annotation_node_count();
    let update = doc.append("wrong\n").unwrap();
    heights.apply(&doc, &update).unwrap();
    assert_eq!(
        heights.measure(ticket, 2),
        Err(LayoutError::StaleMeasurement)
    );
    assert_ne!(doc.tile_key(TileId(1000)).unwrap(), key);
    assert!(
        styles(&doc, Span::new(500_000, 500_001))[0]
            .0
            .contains(Style::HIDDEN)
    );
    assert!(doc.work().inline_bytes - before.inline_bytes < 16);
    assert!(doc.work().tree_visits - before.tree_visits < 1000);
    assert!(doc.annotation_node_count() - nodes < 200);
    doc.finish();
}
