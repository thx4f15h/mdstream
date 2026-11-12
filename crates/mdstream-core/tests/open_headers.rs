#[path = "support/header_confirmation.rs"]
mod header_oracle;
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
fn retained_emphasis_matches_independent_early_header_model() {
    for (open, next) in [("", ""), ("> ", "> "), ("> - [x] ", ">   "), ("1. ", "   ")] {
        for marker in ["*", "**", "_", "__", "~", "~~"] {
            for ending in ["", "  ", "\\"] {
                let prior = format!("{open}[p](u) {marker}prior{ending}\r\n");
                for first in [
                    "**local** [h](v) $m$ [^n] &amp; ",
                    "*local ",
                    "~local ",
                    "`local ",
                    "$local ",
                    "[local ",
                ] {
                    let header = format!("{first}{}|end*` ](w) $\r\n", "中🙂".repeat(150));
                    let tail = format!("{next}:--|--:\n{next}v|z\n");
                    let late = parse(&format!("{prior}{next}{header}{tail}"));
                    let early = parse(&format!("{prior}{next}|{header}{tail}"));
                    assert_eq!(late.tables().len(), 1, "{marker:?} {first:?}");
                    let old = Span::new(0, prior.len());
                    assert_eq!(styles(&late, old), styles(&early, old));
                    assert_eq!(visible(&late, old), visible(&early, old));
                    let start = prior.len() + next.len();
                    let span = Span::new(start, start + header.len());
                    let shifted = Span::new(span.start + 1, span.end + 1);
                    assert_eq!(styles(&late, span), styles(&early, shifted));
                    assert_eq!(visible(&late, span), visible(&early, shifted));
                    assert_eq!(late.links().len(), early.links().len());
                    assert_eq!(late.maths().len(), early.maths().len());
                    assert_eq!(
                        late.footnote_references().len(),
                        early.footnote_references().len()
                    );
                    assert!(
                        late.work().inline_bytes
                            <= late.source().len() as u64 + late.work().header_replay_bytes
                    );
                }
            }
        }
    }
}

#[test]
fn generated_local_pairs_and_crossings_match_early_header_or_plain_text_models() {
    let tokens = ["*", "**", "_", "~", "~~", "a ", "\\*", "`", "[x](u)"];
    let mut confirmed = 0;
    let mut rejected = 0;
    for a in tokens {
        for b in tokens {
            for c in tokens {
                let prior = "*old **old _old ~~old\n";
                let header = format!("{} {a}{b}{c}|b\n", "x".repeat(1030));
                let source = format!("{prior}{header}-|-\n");
                let doc = parse(&source);
                if doc.tables().is_empty() {
                    rejected += 1;
                    let ordinary = parse(&source.replace('|', "/"));
                    assert_eq!(
                        styles(&doc, Span::new(0, source.len())),
                        styles(&ordinary, Span::new(0, source.len()))
                    );
                    assert_eq!(doc.blocks(), ordinary.blocks());
                } else {
                    confirmed += 1;
                    let early = parse(&format!("{prior}|{header}-|-\n"));
                    let start = prior.len();
                    assert_eq!(
                        styles(&doc, Span::new(0, start)),
                        styles(&early, Span::new(0, start))
                    );
                    assert_eq!(
                        styles(&doc, Span::new(start, start + header.len())),
                        styles(&early, Span::new(start + 1, start + 1 + header.len()))
                    );
                }
            }
        }
    }
    assert!(confirmed == 729 && rejected == 0);
}

#[test]
fn fallback_and_eof_preserve_openers_breaks_objects_and_block_ranges() {
    for prefix in ["", "> "] {
        for marker in ["*", "**", "_", "__", "~", "~~"] {
            for ending in ["", "  ", "\\"] {
                for suffix in ["", "\r\n", "\r\nwrong\n", "\r\n---\n"] {
                    let suffix = suffix.replace('\n', &format!("\n{prefix}"));
                    let source = format!(
                        "{prefix}{marker}prior{ending}\r\n{prefix}[h](u) {}|end{marker} $m$ [^n]{suffix}",
                        "x".repeat(1030)
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
                    assert_eq!(doc.maths(), ordinary.maths());
                    assert_eq!(doc.footnote_references(), ordinary.footnote_references());
                    assert!(
                        doc.work().inline_bytes
                            <= source.len() as u64 + doc.work().header_replay_bytes
                    );
                }
            }
        }
    }
}

#[test]
fn unresolved_run_at_first_pipe_uses_cell_boundary_on_success_and_original_stack_on_failure() {
    for prefix in ["", "> "] {
        for marker in ["*", "**", "_", "__", "~", "~~"] {
            let prior = format!("{prefix}{marker}prior\n");
            let header = format!("{}end{marker}|b\n", "x".repeat(1030));
            let source = format!("{prior}{prefix}{header}{prefix}-|-\n");
            let doc = parse(&source);
            let early = parse(&format!("{prior}{prefix}|{header}{prefix}-|-\n"));
            assert_eq!(doc.tables().len(), 1);
            assert_eq!(
                styles(&doc, Span::new(0, prior.len())),
                styles(&early, Span::new(0, prior.len()))
            );
            let start = prior.len() + prefix.len();
            assert_eq!(
                styles(&doc, Span::new(start, start + header.len())),
                styles(&early, Span::new(start + 1, start + 1 + header.len()))
            );
            let failed = parse(&source.replace("-|-", "bad"));
            assert!(failed.tables().is_empty());
            assert!(
                styles(
                    &failed,
                    Span::new(prefix.len(), prefix.len() + marker.len())
                )[0]
                .0
                .contains(Style::HIDDEN)
            );
        }
    }
}

#[test]
fn published_cross_line_closure_abandons_probe_even_if_stack_regrows() {
    for prefix in ["", "> "] {
        for padding in [0, 1020, 2048] {
            let prior = format!("{prefix}*prior  \r\n");
            let body = format!(
                "{prefix}{}end*next *new **new __new {}",
                "x".repeat(padding),
                "y".repeat(1100)
            );
            let mut doc = Document::new();
            doc.append(&prior).unwrap();
            doc.append(&body).unwrap();
            assert!(
                styles(&doc, Span::new(prefix.len() + 1, prefix.len() + 2))[0]
                    .0
                    .contains(Style::EMPHASIS)
            );
            doc.append(" [h](u) |end**\r\n").unwrap();
            doc.append(&format!("{prefix}-|-\n")).unwrap();
            doc.finish();
            header_oracle::assert_scope_composition(&doc);
        }
    }
}

#[test]
fn current_line_objects_publish_immediately_and_keep_ids_after_success_or_fallback() {
    for success in [false, true] {
        let mut doc = Document::new();
        doc.append("> - [p](u) *prior\n").unwrap();
        doc.append(&format!(">   {}", "x".repeat(2048))).unwrap();
        let before = doc.work();
        let objects = " **live** [h](v) $m$ [^n] &amp; ";
        doc.append(objects).unwrap();
        assert_eq!(
            doc.work().inline_bytes - before.inline_bytes,
            objects.len() as u64
        );
        assert_eq!(doc.links().len(), 2);
        assert_eq!(doc.maths().len(), 1);
        assert_eq!(doc.footnote_references().len(), 1);
        let link = *doc.link_info(LinkId(1)).unwrap();
        let math = doc.maths()[0];
        let footnote = doc.footnote_references()[0];
        doc.append("|end* [t](w) $n$ [^t]\n").unwrap();
        assert_eq!(doc.links().len(), 2);
        let update = doc
            .append(if success { ">   -|-\n" } else { ">   wrong\n" })
            .unwrap();
        assert_eq!(update.new_links, 2..3);
        assert_eq!(update.new_maths, 1..2);
        assert_eq!(update.new_footnote_references, 1..2);
        assert_eq!(
            *doc.link_info(LinkId(1)).unwrap(),
            LinkInfo {
                state: SyntaxState::Sealed,
                ..link
            }
        );
        assert_eq!(doc.maths()[0], math);
        assert_eq!(doc.footnote_references()[0], footnote);
        assert_eq!(doc.tables().len(), usize::from(success));
        doc.finish();
        assert_eq!(doc.footnotes()[1].number, Some(2));
    }
}

#[test]
fn deep_million_byte_probe_has_no_old_body_or_ancestor_work() {
    let prefix = ">".repeat(1000);
    for success in [false, true] {
        let mut doc = Document::new();
        doc.append(&format!("{prefix}*prior\n{prefix}{}", "x".repeat(40)))
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
        doc.append("|end*\n").unwrap();
        let update = doc
            .append(&format!(
                "{prefix}{}\n",
                if success { "-|-" } else { "wrong" }
            ))
            .unwrap();
        assert_eq!(update.new_tables.len(), usize::from(success));
        assert!(
            doc.work().inline_bytes
                - after.inline_bytes
                - (doc.work().header_replay_bytes - after.header_replay_bytes)
                < 16
        );
        doc.finish();
        assert!(
            doc.work().inline_bytes <= doc.source().len() as u64 + doc.work().header_replay_bytes
        );
    }
}

#[test]
fn many_old_openers_and_repeated_failed_candidates_do_not_clone_or_walk_stack() {
    let mut doc = Document::new();
    doc.append(&format!("> {}\n", "*open ".repeat(4096)))
        .unwrap();
    let before = doc.work();
    for i in 0..200 {
        doc.append(&format!(
            "> {}{}\n",
            "x".repeat(1030),
            if i % 2 == 0 { "|b" } else { "" }
        ))
        .unwrap();
        doc.append("> wrong\n").unwrap();
    }
    assert_eq!(doc.work().delimiter_steps, before.delimiter_steps);
    doc.append(&format!("> {}|b\n", "x".repeat(1030))).unwrap();
    let before = doc.work();
    doc.append("> -|-\n").unwrap();
    assert_eq!(doc.tables().len(), 1);
    assert_eq!(doc.work().delimiter_steps - before.delimiter_steps, 4096);
    doc.finish();
    assert_eq!(doc.work().delimiter_steps - before.delimiter_steps, 4096);
    assert!(doc.work().inline_bytes <= doc.source().len() as u64 + doc.work().header_replay_bytes);
}

#[test]
fn generated_openers_tables_and_rejections_match_independent_range_model() {
    let mut source = String::new();
    let mut expected = Vec::new();
    for i in 0..400 {
        let prefix = ">".repeat(i % 9);
        source.push_str(&format!("{prefix}*prior\n"));
        let start = source.len();
        source.push_str(&format!(
            "{prefix}{}{}|b\r\n",
            "x".repeat(1030 + i),
            if i % 3 == 0 { "end* " } else { "" }
        ));
        let delimiter_start = source.len();
        source.push_str(&format!(
            "{prefix}{}|-{}\n",
            "-".repeat(1030),
            if i % 5 == 0 { "x" } else { "" }
        ));
        let delimiter_end = source.len();
        source.push_str(&format!("{prefix}v|z\n"));
        // The close-only old pair is now isolatable; only bad separators fail.
        if i % 5 != 0 {
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
    assert!(doc.work().inline_bytes <= source.len() as u64 + doc.work().header_replay_bytes);
    assert!(doc.work().table_header_bytes <= source.len() as u64);
}

#[test]
fn fallback_invalidates_old_cross_line_measurement_and_merged_views_agree() {
    for success in [false, true] {
        let mut doc = Document::new();
        let mut view = FoldView::new(&doc, 1).unwrap();
        let mut merged = FoldView::new(&doc, 1).unwrap();
        let mut batch = Update::default();
        let prior = "> *prior  \n";
        for chunk in [prior, &format!("> {}", "x".repeat(2048)), "|end*\n"] {
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

#[test]
fn inherited_openers_with_dense_objects_and_sparse_columns_keep_budgeted_preview() {
    let open = "- ".repeat(250);
    let next = " ".repeat(500);
    let mut doc = Document::new();
    doc.append(&format!(
        "{open}*prior\n{next}{}",
        "[a](u) $m$ [^n] ".repeat(4096)
    ))
    .unwrap();
    assert_eq!(doc.links().len(), 4096);
    doc.append(&format!("|{}\n", "h|".repeat(4095))).unwrap();
    let before = doc.work();
    let update = doc
        .append(&format!("{next}|{}\n", "-:|".repeat(4096)))
        .unwrap();
    assert_eq!(update.new_links, 4096..4096);
    assert_eq!(update.new_maths, 4096..4096);
    assert_eq!(update.new_footnote_references, 4096..4096);
    assert_eq!(doc.work().label_bytes, before.label_bytes + 4096);
    assert_eq!(doc.work().math_steps, before.math_steps + 8192);
    assert_eq!(doc.work().footnote_steps, before.footnote_steps + 8192);
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
