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
fn opaque_prefix_confirmation_matches_independent_early_header_semantics() {
    for (open, prefix) in [("", ""), ("> ", "> "), ("> - [x] ", ">   "), ("1. ", "   ")] {
        for ticks in [1, 2, 3, 16] {
            let marker = "`".repeat(ticks);
            let local = if ticks == 1 { "``" } else { "`" };
            for ending in ["", "  ", "\\"] {
                let prior = format!("{open}[p](u) &amp; *old {marker} prior{ending}\r\n");
                let header = format!(
                    "**local** {local} a\\|b {local} [h](v) $m$ [^n] &amp; {}|[t](w)\r\n",
                    "中🙂".repeat(150)
                );
                compare_header(&prior, prefix, &header);
            }
        }
    }
}

#[test]
fn pending_tick_run_at_pipe_is_discarded_without_publishing_old_code() {
    for prefix in ["", "> "] {
        for ticks in [1, 2, 7, 2048] {
            let marker = "`".repeat(ticks);
            // Precede the opener with text to avoid a fenced block.
            let prior = format!("{prefix}old {marker} 中\r\n");
            for run in [marker.clone(), format!("{marker}`")] {
                let header = format!("{} {run}|b\r\n", "x".repeat(1030));
                compare_header(&prior, prefix, &header);
                let source = format!("{prior}{prefix}{header}{prefix}wrong\n");
                let failed = parse(&source);
                let ordinary = parse(&source.replace('|', "/"));
                assert_eq!(
                    styles(&failed, Span::new(0, source.len())),
                    styles(&ordinary, Span::new(0, source.len()))
                );
                assert_eq!(
                    visible(&failed, Span::new(0, source.len())).replace('|', "/"),
                    visible(&ordinary, Span::new(0, source.len()))
                );
                assert!(failed.work().inline_bytes <= source.len() as u64);
            }
        }
    }
}

#[test]
fn fallback_eof_setext_and_scope_exit_preserve_original_code_mapping() {
    for prefix in ["", "> "] {
        for end in ["", "\r\n", "\r\nwrong\n", "\r\n---\n", "\r\n# out\n"] {
            for close in ["", " ` [after](u) &amp;"] {
                let tail = end.replace('\n', &format!("\n{prefix}"));
                let source = format!(
                    "{prefix}a ` \r\n{prefix}{}&amp; [raw](u) | end{close}{tail}",
                    "中🙂".repeat(150)
                );
                let doc = parse(&source);
                let ordinary = parse(&source.replace('|', "/"));
                assert!(doc.tables().is_empty());
                let span = Span::new(0, source.len());
                assert_eq!(styles(&doc, span), styles(&ordinary, span));
                assert_eq!(
                    visible(&doc, span).replace('|', "/"),
                    visible(&ordinary, span)
                );
                assert_eq!(doc.blocks(), ordinary.blocks());
                assert_eq!(doc.links(), ordinary.links());
                assert!(doc.work().inline_bytes <= source.len() as u64);
            }
        }
    }
    let source = format!("> a `\n> {}|b\n-|-\n", "x".repeat(1030));
    assert!(parse(&source).tables().is_empty());
}

#[test]
fn closure_before_pipe_cancels_probe_even_when_a_new_code_span_opens() {
    for prefix in ["", "> "] {
        for padding in [0, 1010, 2048] {
            let mut doc = Document::new();
            doc.append(&format!("{prefix}a ` 中\r\n")).unwrap();
            doc.append(&format!(
                "{prefix}{}end ` [live](u) &amp; `new {}",
                "x".repeat(padding),
                "y".repeat(1100)
            ))
            .unwrap();
            assert_eq!(doc.links().len(), 1);
            assert!(
                styles(&doc, Span::new(prefix.len() + 3, prefix.len() + 4))[0]
                    .0
                    .contains(Style::CODE)
            );
            let old_link = *doc.link_info(LinkId(0)).unwrap();
            let before = doc.work();
            doc.append("|b`\r\n").unwrap();
            assert_eq!(doc.work().table_header_bytes, before.table_header_bytes);
            doc.append(&format!("{prefix}-|-\n")).unwrap();
            doc.finish();
            assert!(doc.tables().is_empty());
            assert_eq!(*doc.link_info(LinkId(0)).unwrap(), old_link);
            let ordinary = parse(&doc.source().replace('|', "/"));
            let span = Span::new(0, doc.source().len());
            assert_eq!(styles(&doc, span), styles(&ordinary, span));
            assert_eq!(
                visible(&doc, span).replace('|', "/"),
                visible(&ordinary, span)
            );
        }
    }
}

#[test]
fn objects_in_opaque_prefix_publish_only_on_success_and_old_ids_stay_stable() {
    for success in [false, true] {
        let mut doc = Document::new();
        doc.append("> - [p](u) $p$ [^p] &amp; ` prior\n").unwrap();
        let link = *doc.link_info(LinkId(0)).unwrap();
        let math = doc.maths()[0];
        let footnote = doc.footnote_references()[0];
        doc.append(&format!(">   [h](v) $m$ [^n] &amp; {}", "x".repeat(2048)))
            .unwrap();
        assert_eq!(doc.links().len(), 1);
        assert_eq!(doc.maths().len(), 1);
        assert_eq!(doc.footnote_references().len(), 1);
        let block = BlockId(doc.blocks().len() - 1);
        doc.append("|[t](w) $n$ [^t]\n").unwrap();
        let update = doc
            .append(if success { ">   -|-\n" } else { ">   wrong\n" })
            .unwrap();
        assert_eq!(update.new_links, 1..if success { 3 } else { 1 });
        assert_eq!(update.new_maths, 1..if success { 3 } else { 1 });
        assert_eq!(
            update.new_footnote_references,
            1..if success { 3 } else { 1 }
        );
        assert_eq!(*doc.link_info(LinkId(0)).unwrap(), link);
        assert_eq!(doc.maths()[0], math);
        assert_eq!(doc.footnote_references()[0], footnote);
        if success {
            assert_eq!(doc.tables()[0].block, block);
            assert_eq!(doc.link_destination(LinkId(1)), Some("v"));
            assert_eq!(doc.footnotes()[2].number, Some(3));
        }
        doc.finish();
    }
}

#[test]
fn million_byte_growth_is_single_pass_and_confirmation_only_consumes_current_row() {
    let prefix = ">".repeat(1000);
    for success in [false, true] {
        let mut doc = Document::new();
        doc.append(&format!(
            "{prefix}a `{}\n{prefix}{}",
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
        assert_eq!(after.classified_bytes, before.classified_bytes);
        assert_eq!(after.delimiter_steps, before.delimiter_steps);
        assert_eq!(after.container_bytes, before.container_bytes);
        assert_eq!(after.container_steps, before.container_steps);
        assert_eq!(after.heading_steps, before.heading_steps);
        assert_eq!(doc.annotation_arena_stats(), annotations);
        assert_eq!(doc.container_arena_stats(), containers);
        doc.append("|b`\n").unwrap();
        let update = doc
            .append(&format!(
                "{prefix}{}\n",
                if success { "-|-" } else { "wrong" }
            ))
            .unwrap();
        assert_eq!(update.new_tables.len(), usize::from(success));
        assert_eq!(
            doc.work().inline_bytes - after.inline_bytes,
            if success { 1_000_042 } else { 10 }
        );
        assert_eq!(
            doc.work().table_bytes - after.table_bytes,
            if success { 1_000_044 } else { 0 }
        );
        doc.finish();
        assert!(doc.work().inline_bytes <= doc.source().len() as u64 + doc.work().table_bytes);
    }
}

#[test]
fn repeated_failed_probes_keep_code_summary_and_never_walk_old_openers() {
    let mut doc = Document::new();
    doc.append(&format!("> {} ` \r\n", "*open ".repeat(4096)))
        .unwrap();
    let before = doc.work();
    for i in 0..200 {
        doc.append(&format!(
            "> {}{}\r\n> wrong\r\n",
            "x".repeat(1030),
            if i % 2 == 0 { "|b" } else { "" }
        ))
        .unwrap();
    }
    assert_eq!(doc.work().delimiter_steps, before.delimiter_steps);
    doc.append("> ` [live](u)\n").unwrap();
    assert_eq!(doc.links().len(), 1);
    assert!(doc.tables().is_empty());
    assert!(doc.work().inline_bytes <= doc.source().len() as u64);
    let expected = parse(&doc.source().replace('|', "/"));
    doc.finish();
    let span = Span::new(0, doc.source().len());
    assert_eq!(
        visible(&doc, span).replace('|', "/"),
        visible(&expected, span)
    );
    assert!(doc.work().delimiter_steps - before.delimiter_steps <= 4098);
}

#[test]
fn generated_code_lengths_and_closures_match_header_or_plain_models() {
    let tokens = [
        "a ", "`", "``", "```", "` ", "`` ", "``` ", "\\`", "&amp;", "[x](u)", "*x*",
    ];
    let mut confirmed = 0;
    let mut rejected = 0;
    for ticks in [1, 2, 3] {
        for a in tokens {
            for b in tokens {
                let prior = format!("a {} prior\r\n", "`".repeat(ticks));
                let header = format!("{}{a}{b}|end\n", "x".repeat(1030));
                let source = format!("{prior}{header}-|-\n");
                let doc = parse(&source);
                if doc.tables().is_empty() {
                    rejected += 1;
                    let ordinary = parse(&source.replace('|', "/"));
                    let span = Span::new(0, source.len());
                    assert_eq!(styles(&doc, span), styles(&ordinary, span));
                    assert_eq!(
                        visible(&doc, span).replace('|', "/"),
                        visible(&ordinary, span)
                    );
                } else {
                    confirmed += 1;
                    compare_header(&prior, "", &header);
                }
            }
        }
    }
    assert!(
        confirmed > 80 && rejected > 20,
        "{confirmed} confirmed, {rejected} rejected"
    );
}

#[test]
fn display_mapping_is_identical_across_scalar_tile_and_budgeted_reads() {
    for success in [false, true] {
        let source = format!(
            "> a `` \r\n> {} ` a\\|b ` &amp; [x](u)|end ``\r\n> {}\n",
            "中🙂".repeat(150),
            if success { "-|-" } else { "wrong" }
        );
        let doc = parse(&source);
        let span = Span::new(0, source.len());
        let expected = visible(&doc, span);
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
        let mut cursor = doc.display_cursor(span).unwrap();
        let mut text = String::new();
        for frame in 0..10000 {
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
            assert!(frame < 9999);
        }
        assert_eq!(text, expected);
    }
}

#[test]
fn generated_tables_and_rejections_match_independent_scope_and_range_model() {
    let mut source = String::new();
    let mut expected = Vec::new();
    for i in 0..400 {
        let prefix = ">".repeat(i % 9);
        source.push_str(&format!("{prefix}a ` prior\n"));
        let start = source.len();
        source.push_str(&format!(
            "{prefix}{}{}|b\r\n",
            "x".repeat(1030 + i),
            if i % 3 == 0 { "` closed " } else { "" }
        ));
        let delimiter_start = source.len();
        source.push_str(&format!(
            "{prefix}{}|-{}\n",
            "-".repeat(1030),
            if i % 5 == 0 { "x" } else { "" }
        ));
        let delimiter_end = source.len();
        source.push_str(&format!("{prefix}v|z\n"));
        if i % 3 != 0 && i % 5 != 0 {
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
    assert!(doc.work().inline_bytes <= source.len() as u64 + doc.work().table_bytes);
    assert!(doc.work().table_header_bytes <= source.len() as u64);
}

#[test]
fn dense_deferred_objects_and_columns_publish_once_with_budgeted_preview() {
    let open = "- ".repeat(250);
    let prefix = " ".repeat(500);
    let mut doc = Document::new();
    doc.append(&format!(
        "{open}{} ` prior\n{prefix}{}",
        "*old ".repeat(4096),
        "[a](u) $m$ [^n] ".repeat(4096)
    ))
    .unwrap();
    assert_eq!(doc.links().len(), 0);
    doc.append(&format!("|{}\n", "h|".repeat(4095))).unwrap();
    let before = doc.work();
    let update = doc
        .append(&format!("{prefix}|{}\n", "-:|".repeat(4096)))
        .unwrap();
    assert_eq!(update.new_links, 0..4096);
    assert_eq!(update.new_maths, 0..4096);
    assert_eq!(update.new_footnote_references, 0..4096);
    assert_eq!(doc.work().math_steps - before.math_steps, 8192);
    assert_eq!(
        doc.work().delimiter_steps - before.delimiter_steps,
        4096 + 8192
    );
    assert_eq!(doc.container_count(), 500);
    assert_eq!(doc.tables()[0].columns.len(), 4096);
    assert_eq!(
        doc.block(doc.tables()[0].block).unwrap().container,
        Some(ContainerId(499))
    );
    let mut preview = TablePreview::new(
        &doc,
        TableId(0),
        TablePreviewLimits {
            rows: 1,
            source_bytes: 550,
            display_bytes: 8,
        },
    )
    .unwrap();
    let mut zero = Budget::new(WorkLimit::default());
    assert_eq!(
        preview.next(&doc, &mut zero).unwrap(),
        TablePreviewStep::BudgetExhausted
    );
    assert_eq!(zero.used(), WorkLimit::default());
    for frame in 0..200 {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 8,
            ..Default::default()
        });
        let step = preview.next(&doc, &mut budget).unwrap();
        assert!(budget.used().bytes <= 8);
        if matches!(step, TablePreviewStep::Complete(_)) {
            return;
        }
        assert!(frame < 199);
    }
}

#[test]
fn late_code_mapping_invalidates_measurements_and_merged_fold_views_agree() {
    for success in [false, true] {
        let mut doc = Document::new();
        let mut view = FoldView::new(&doc, 1).unwrap();
        let mut merged = FoldView::new(&doc, 1).unwrap();
        let mut batch = Update::default();
        for chunk in [
            "> a ` prior\r\n",
            &format!("> {}", "x".repeat(2048)),
            "|end `\r\n",
        ] {
            let update = doc.append(chunk).unwrap();
            view.apply(&doc, &update).unwrap();
            batch.merge(update).unwrap();
        }
        let mut heights = HeightIndex::default();
        heights.rebuild(&doc).unwrap();
        let ticket = heights.ticket(TileId(0)).unwrap();
        let key = doc.tile_key(TileId(0)).unwrap();
        let update = doc
            .append(if success { "> -|-\n" } else { "> wrong\n" })
            .unwrap();
        heights.apply(&doc, &update).unwrap();
        if !success {
            assert_ne!(doc.tile_key(TileId(0)).unwrap(), key);
            assert_eq!(
                heights.measure(ticket, 2),
                Err(LayoutError::StaleMeasurement)
            );
        }
        view.apply(&doc, &update).unwrap();
        batch.merge(update).unwrap();
        merged.apply(&doc, &batch).unwrap();
        assert_eq!(view.heights().total_rows(), merged.heights().total_rows());
        if success {
            for view in [&mut view, &mut merged] {
                view.set_collapsed(&doc, FoldId::Table(TableId(0)), true)
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
