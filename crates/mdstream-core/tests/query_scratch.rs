use mdstream::*;
use std::panic::{AssertUnwindSafe, catch_unwind};

fn parse(text: &str) -> Document {
    let mut doc = Document::new();
    doc.append(text).unwrap();
    doc.finish();
    doc
}

fn boundaries(text: &str) -> Vec<usize> {
    text.char_indices()
        .map(|(at, _)| at)
        .chain([text.len()])
        .collect()
}

fn compare(doc: &Document, span: Span, scratch: &mut QueryScratch) {
    let mut fresh = doc.runs_in(span).unwrap();
    let mut reused = doc.runs_in_with_scratch(span, scratch).unwrap();
    assert_eq!(
        fresh.by_ref().collect::<Vec<_>>(),
        reused.by_ref().collect::<Vec<_>>()
    );
    assert_eq!(fresh.annotation_visits(), reused.annotation_visits());
    drop(reused);
    let mut fresh = doc.display_runs_in(span).unwrap();
    let mut reused = doc.display_runs_in_with_scratch(span, scratch).unwrap();
    assert_eq!(
        fresh.by_ref().collect::<Vec<_>>(),
        reused.by_ref().collect::<Vec<_>>()
    );
    assert_eq!(fresh.annotation_visits(), reused.annotation_visits());
    drop(reused);
    assert_eq!(scratch.stats().nodes, 0);
}

#[test]
fn query_scratch_all_unicode_windows_preserve_runs_mappings_and_visits() {
    let mut scratch = QueryScratch::default();
    for text in [
        "中 **🙂** &NotEqualTilde; ` a\r\nb ` [x](dest)\n",
        "> - ` a\n> \tb `\n\n| a | b |\n| - | - |\n| 中 | &amp; |\n",
        "# h\n\n```rs\n<&>\n```\n\n$$\nx\n$$\n",
    ] {
        let doc = parse(text);
        let points = boundaries(text);
        let before = doc.work();
        for (i, &start) in points.iter().enumerate() {
            for &end in &points[i..] {
                compare(&doc, Span::new(start, end), &mut scratch);
            }
        }
        for id in 0..doc.tile_count() {
            assert_eq!(
                doc.runs(TileId(id)).unwrap().collect::<Vec<_>>(),
                doc.runs_with_scratch(TileId(id), &mut scratch)
                    .unwrap()
                    .collect::<Vec<_>>()
            );
            assert_eq!(
                doc.display_runs(TileId(id)).unwrap().collect::<Vec<_>>(),
                doc.display_runs_with_scratch(TileId(id), &mut scratch)
                    .unwrap()
                    .collect::<Vec<_>>()
            );
        }
        assert_eq!(doc.work(), before);
    }
}

#[test]
fn query_scratch_semantics_match_independent_source_style_model() {
    let mut input = String::new();
    let mut model = Vec::new();
    for _ in 0..400 {
        for (text, style) in [
            ("中 ", Style::default()),
            ("**", Style::HIDDEN),
            ("🙂", Style::STRONG),
            ("**", Style::HIDDEN),
            (" ", Style::default()),
            ("*", Style::HIDDEN),
            ("é", Style::EMPHASIS),
            ("*", Style::HIDDEN),
            (" ", Style::default()),
        ] {
            input.push_str(text);
            model.extend(std::iter::repeat_n(style, text.len()));
        }
    }
    let doc = parse(&input);
    let points = boundaries(&input);
    let mut scratch = QueryScratch::default();
    let mut seed = 73_u64;
    for _ in 0..1200 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let a = points[seed as usize % points.len()];
        let b = points[(seed >> 32) as usize % points.len()];
        let span = Span::new(a.min(b), a.max(b));
        let mut at = span.start;
        for run in doc.runs_in_with_scratch(span, &mut scratch).unwrap() {
            assert_eq!(run.source.start, at);
            assert_eq!(run.text, &input[run.source.range()]);
            assert!(
                model[run.source.range()]
                    .iter()
                    .all(|style| *style == run.style)
            );
            assert_eq!(run.link, None);
            at = run.source.end;
        }
        assert_eq!(at, span.end);
    }
}

#[test]
fn query_scratch_display_matches_literal_output_across_every_split() {
    let text = "中 &amp; ` a\r\nb ` &#x1f642; **z**";
    let doc = parse(text);
    let mut scratch = QueryScratch::default();
    for split in boundaries(text) {
        let mut visible = String::new();
        for span in [Span::new(0, split), Span::new(split, text.len())] {
            let mut at = span.start;
            for run in doc
                .display_runs_in_with_scratch(span, &mut scratch)
                .unwrap()
            {
                assert_eq!(run.source.start, at);
                assert!(run.source.end > at);
                if let Some(replacement) = run.replacement {
                    assert!(replacement.start <= run.source.start);
                    assert!(replacement.end >= run.source.end);
                    assert!(doc.text(replacement).is_some());
                } else {
                    assert_eq!(run.text, doc.text(run.source).unwrap());
                }
                if !run.style.contains(Style::HIDDEN) {
                    visible.push_str(run.text);
                }
                at = run.source.end;
            }
            assert_eq!(at, span.end);
        }
        assert_eq!(visible, "中 & a b 🙂 z");
    }
}

#[test]
fn query_scratch_invalid_and_empty_queries_preserve_retained_capacity() {
    let doc = parse("中 **bold**");
    let mut scratch = QueryScratch::default();
    compare(&doc, Span::new(0, 0), &mut scratch);
    assert_eq!(scratch.stats(), ArenaStats::default());
    compare(&doc, Span::new(0, doc.source().len()), &mut scratch);
    let warm = scratch.stats();
    for span in [Span::new(1, 3), Span::new(3, 1), Span::new(0, usize::MAX)] {
        assert!(doc.runs_in_with_scratch(span, &mut scratch).is_none());
        assert!(
            doc.display_runs_in_with_scratch(span, &mut scratch)
                .is_none()
        );
        assert_eq!(scratch.stats(), warm);
    }
    assert!(
        doc.runs_with_scratch(TileId(usize::MAX), &mut scratch)
            .is_none()
    );
    assert!(
        doc.display_runs_with_scratch(TileId(usize::MAX), &mut scratch)
            .is_none()
    );
    for at in [0, doc.source().len()] {
        compare(&doc, Span::new(at, at), &mut scratch);
        assert_eq!(scratch.stats(), warm);
    }
    scratch = QueryScratch::default();
    assert_eq!(scratch.stats(), ArenaStats::default());
}

#[test]
fn query_scratch_early_drop_unwind_and_cross_document_reuse_keep_borrowed_text() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<QueryScratch>();
    let doc = parse("a **b** &amp; ` x\ny `");
    let span = Span::new(0, doc.source().len());
    let mut scratch = QueryScratch::default();
    compare(&doc, span, &mut scratch);
    let warm = scratch.stats();
    let kept = doc
        .runs_in_with_scratch(span, &mut scratch)
        .unwrap()
        .next()
        .unwrap();
    let display = doc
        .display_runs_in_with_scratch(span, &mut scratch)
        .unwrap()
        .next()
        .unwrap();
    for display_query in [false, true] {
        let result = catch_unwind(AssertUnwindSafe(|| {
            if display_query {
                let mut iter = doc
                    .display_runs_in_with_scratch(span, &mut scratch)
                    .unwrap();
                iter.next();
                panic!("consumer stopped");
            } else {
                let mut iter = doc.runs_in_with_scratch(span, &mut scratch).unwrap();
                iter.next();
                panic!("consumer stopped");
            }
        }));
        assert!(result.is_err());
        assert_eq!(scratch.stats(), warm);
    }
    {
        let other = parse("new");
        compare(&other, Span::new(0, 3), &mut scratch);
    }
    assert_eq!(kept.text, "a ");
    assert_eq!(display.text, "a ");
    let mut second = QueryScratch::default();
    let mut first_iter = doc.runs_in_with_scratch(span, &mut scratch).unwrap();
    let mut second_iter = doc.runs_in_with_scratch(span, &mut second).unwrap();
    assert_eq!(first_iter.next(), second_iter.next());
    assert_eq!(
        first_iter.collect::<Vec<_>>(),
        second_iter.collect::<Vec<_>>()
    );
    assert_eq!(scratch.stats(), warm);
}

#[test]
fn query_scratch_streaming_restarts_after_late_structure_and_display_changes() {
    let source = format!(
        "> - **{}\n>   中** &amp; ` a\r\n>   b `\n\nhead\n===\n\na | b\n- | -\n1 | 2\n\n[x][r]\n\n[r]: /late\n",
        "x".repeat(1100)
    );
    let mut scratch = QueryScratch::default();
    let mut doc = Document::new();
    let mut update = Update::default();
    for part in source.split_inclusive('\n') {
        doc.append_into(part, &mut update).unwrap();
        compare(&doc, Span::new(0, doc.source().len()), &mut scratch);
        // Abandon a live traversal before the next append/late rewrite.
        doc.display_runs_in_with_scratch(Span::new(0, doc.source().len()), &mut scratch)
            .unwrap()
            .next();
    }
    doc.finish();
    compare(&doc, Span::new(0, source.len()), &mut scratch);
    let warm = scratch.stats();
    for _ in 0..64 {
        compare(&doc, Span::new(0, source.len()), &mut scratch);
    }
    assert_eq!(scratch.stats(), warm);
}

#[test]
fn query_scratch_million_byte_and_deep_container_windows_stay_logarithmic() {
    let mut scratch = QueryScratch::default();
    for text in [
        format!("x **{}** &amp;", "a".repeat(1_000_000)),
        format!("{} **{}** &amp;", "> ".repeat(1000), "a".repeat(1_000_000)),
        "**a** &amp; `b` ".repeat(65_536),
    ] {
        let doc = parse(&text);
        let before = doc.work();
        let arena = doc.annotation_arena_stats();
        let depth = usize::BITS - text.len().leading_zeros();
        compare(&doc, Span::new(0, text.len()), &mut scratch);
        let warm = scratch.stats();
        for i in 0..128 {
            let at = (text.len() / 2 + i * 17) % (text.len() - 256);
            let span = Span::new(at, at + 256);
            compare(&doc, span, &mut scratch);
            let mut iter = doc
                .display_runs_in_with_scratch(span, &mut scratch)
                .unwrap();
            while iter.next().is_some() {
                assert!(iter.scratch_stats().nodes <= depth as usize + 1);
            }
            assert!(iter.annotation_visits() <= 4 * 256 + 4 * u64::from(depth));
        }
        assert_eq!(scratch.stats(), warm);
        assert!(warm.capacity_bytes <= 2 * (depth as usize + 4) * 128);
        assert_eq!(doc.work(), before);
        assert_eq!(doc.annotation_arena_stats(), arena);
    }
}

#[test]
fn query_scratch_budget_cursor_matches_fresh_window_and_atomic_cost_model() {
    let doc = parse(&"中 &NotEqualTilde; ` a\r\n🙂 b ` **z** ".repeat(40));
    for bytes in [6, 7, 19, TILE_BYTES + 2, 10_000] {
        let mut cursor = doc
            .display_cursor(Span::new(0, doc.source().len()))
            .unwrap();
        let mut output = String::new();
        while cursor.source_position() < doc.source().len() {
            let mut budget = Budget::new(WorkLimit {
                bytes,
                queries: 3,
                objects: 2,
                ..WorkLimit::unlimited()
            });
            assert!(budget.charge(WorkLimit {
                dependencies: 1,
                ..Default::default()
            }));
            loop {
                let at = cursor.source_position();
                let prior = budget.used();
                let stats = cursor.scratch_stats();
                let remaining = budget.remaining();
                let end = boundaries(doc.source())
                    .into_iter()
                    .filter(|&end| {
                        end >= at && end - at <= remaining.bytes.saturating_sub(2).min(TILE_BYTES)
                    })
                    .max()
                    .unwrap();
                let step = cursor.next(&doc, &mut budget).unwrap();
                if at == doc.source().len() {
                    assert_eq!(step, DisplayStep::Complete);
                    assert_eq!(budget.used(), prior);
                    assert_eq!(cursor.scratch_stats(), stats);
                    break;
                }
                if end == at || remaining.queries == 0 || remaining.objects == 0 {
                    assert_eq!(step, DisplayStep::BudgetExhausted);
                    assert_eq!(cursor.source_position(), at);
                    assert_eq!(budget.used(), prior);
                    assert_eq!(cursor.scratch_stats(), stats);
                    break;
                }
                let expected = doc
                    .display_runs_in(Span::new(at, end))
                    .unwrap()
                    .next()
                    .unwrap();
                assert_eq!(step, DisplayStep::Run(expected.clone()));
                assert_eq!(
                    budget.used(),
                    WorkLimit {
                        bytes: prior.bytes + end - at + 2,
                        queries: prior.queries + 1,
                        objects: prior.objects + 1,
                        ..prior
                    }
                );
                assert_eq!(cursor.source_position(), expected.source.end);
                assert_eq!(cursor.scratch_stats().nodes, 0);
                if !expected.style.contains(Style::HIDDEN) {
                    output.push_str(expected.text);
                }
            }
        }
        assert_eq!(output, "中 ≂̸ a 🙂 b z ".repeat(40));
        assert!(cursor.scratch_stats().allocation_calls <= 6);
    }
}

#[test]
fn query_scratch_cursor_rejections_and_completion_do_not_allocate_or_charge() {
    let mut doc = Document::new();
    doc.append("中 **b**").unwrap();
    let mut cursor = doc
        .display_cursor(Span::new(0, doc.source().len()))
        .unwrap();
    for limit in [
        WorkLimit::default(),
        WorkLimit {
            bytes: 4,
            ..WorkLimit::unlimited()
        },
        WorkLimit {
            queries: 0,
            ..WorkLimit::unlimited()
        },
        WorkLimit {
            objects: 0,
            ..WorkLimit::unlimited()
        },
    ] {
        let mut budget = Budget::new(limit);
        assert_eq!(
            cursor.next(&doc, &mut budget).unwrap(),
            DisplayStep::BudgetExhausted
        );
        assert_eq!(cursor.source_position(), 0);
        assert_eq!(cursor.scratch_stats(), ArenaStats::default());
        assert_eq!(budget.used(), WorkLimit::default());
    }
    cursor
        .next(&doc, &mut Budget::new(WorkLimit::unlimited()))
        .unwrap();
    let stats = cursor.scratch_stats();
    let at = cursor.source_position();
    let mut budget = Budget::new(WorkLimit::unlimited());
    assert_eq!(
        cursor.next(&Document::new(), &mut budget),
        Err(LayoutError::WrongDocument)
    );
    doc.append("more").unwrap();
    assert_eq!(
        cursor.next(&doc, &mut budget),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(cursor.scratch_stats(), stats);
    assert_eq!(cursor.source_position(), at);
    assert_eq!(budget.used(), WorkLimit::default());
    let mut empty = doc.display_cursor(Span::new(0, 0)).unwrap();
    assert_eq!(
        empty.next(&doc, &mut budget).unwrap(),
        DisplayStep::Complete
    );
    assert_eq!(empty.scratch_stats(), ArenaStats::default());
    assert_eq!(budget.used(), WorkLimit::default());
}
