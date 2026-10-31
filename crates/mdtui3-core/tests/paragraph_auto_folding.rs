use mdtui3::*;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}
fn frame(n: usize) -> Budget {
    Budget::new(WorkLimit {
        queries: n,
        objects: n,
        ..Default::default()
    })
}
fn policy(view: &mut FoldView, min_bytes: usize, preview_bytes: usize) {
    view.set_defaults(FoldDefaults {
        paragraphs: Some(ParagraphFoldPolicy {
            min_bytes,
            preview_bytes,
        }),
        ..view.defaults()
    });
}
fn drain(
    doc: &Document,
    view: &mut FoldView,
    task: &mut ParagraphAutoFold,
) -> (ParagraphAutoFoldProgress, usize) {
    let mut steps = 0;
    loop {
        let mut budget = frame(1);
        let progress = task.advance(doc, view, &mut budget).unwrap();
        assert_eq!(
            budget.used(),
            WorkLimit {
                queries: budget.used().queries,
                objects: budget.used().queries,
                ..Default::default()
            }
        );
        steps += budget.used().queries;
        if progress.completion == Completion::Covered {
            assert!(task.remaining_range().is_empty());
            let done = task.advance(doc, view, &mut frame(0)).unwrap();
            assert_eq!(done.registered, progress.registered);
            assert!(done.change.is_none());
            return (progress, steps);
        }
        assert!(steps < 100_000);
    }
}
fn run(doc: &Document, view: &mut FoldView) -> (ParagraphAutoFoldProgress, usize) {
    let mut task = ParagraphAutoFold::new(doc, view, Span::new(0, doc.source().len())).unwrap();
    drain(doc, view, &mut task)
}
fn apply(
    doc: &Document,
    view: &mut FoldView,
    update: &Update,
) -> (ParagraphAutoFoldProgress, usize) {
    view.apply(doc, update).unwrap();
    let mut task = ParagraphAutoFold::for_update(doc, view, update).unwrap();
    drain(doc, view, &mut task)
}
fn at(doc: &Document, view: &FoldView, start: usize) -> FoldCandidate {
    view.source_fold_at(doc, start).unwrap().unwrap()
}
fn source_id(id: FoldId) -> SourceFoldId {
    let FoldId::Source(id) = id else { panic!() };
    id
}
fn registrations(doc: &Document, view: &FoldView) -> Vec<(BlockId, Span, Span, bool)> {
    let mut task = SourceFoldCandidates::new(doc, view, Span::new(0, doc.source().len())).unwrap();
    let mut out = Vec::new();
    task.advance(doc, view, &mut frame(100_000), &mut out)
        .unwrap();
    out.into_iter()
        .map(|c| {
            (
                view.paragraph_fold_block(doc, source_id(c.id))
                    .unwrap()
                    .unwrap(),
                c.source,
                c.hidden,
                c.open,
            )
        })
        .collect()
}

#[test]
fn inclusive_raw_threshold_prefix_rounding_and_extreme_limits() {
    let doc = parse("a中🙂bc\r\nnext\r\n\n> x\n> y\n>\n- z\n  w\n");
    for (prefix, cut) in [(0, 0), (1, 1), (3, 1), (4, 4), (7, 4), (8, 8), (11, 10)] {
        let mut view = FoldView::new(&doc, 2).unwrap();
        let len = doc.block(BlockId(0)).unwrap().source.len();
        policy(&mut view, len, prefix);
        assert_eq!(run(&doc, &mut view).0.registered, 1);
        let c = at(&doc, &view, 0);
        assert_eq!(c.hidden, Span::new(cut, len));
        assert!(view.is_collapsed(c.id));
        assert_eq!(run(&doc, &mut view).0.registered, 0);
    }
    for (min, prefix) in [(usize::MAX, 0), (0, usize::MAX)] {
        let mut view = FoldView::new(&doc, 1).unwrap();
        policy(&mut view, min, prefix);
        assert_eq!(run(&doc, &mut view).0.registered, 0);
    }
    let mut view = FoldView::new(&doc, 1).unwrap();
    policy(&mut view, 0, 0);
    assert_eq!(run(&doc, &mut view).0.registered, 3);
    assert_eq!(view.source_fold_count(), 3);
}

#[test]
fn streaming_threshold_uses_classified_bytes_and_finish_has_its_own_window() {
    for (initial, pending) in [
        ("abc\n", "中🙂"),
        ("> abc\n", "> 中🙂"),
        ("- abc\n", "  中🙂"),
    ] {
        let mut doc = Document::new();
        let mut view = FoldView::new(&doc, 1).unwrap();
        policy(&mut view, initial.len() + 1, 2);
        let u = doc.append(initial).unwrap();
        assert_eq!(apply(&doc, &mut view, &u).0.registered, 0);
        let u = doc.append(pending).unwrap();
        assert_eq!(apply(&doc, &mut view, &u).0.registered, 0);
        let u = doc.finish();
        assert!(u.appended.is_empty());
        assert_eq!(apply(&doc, &mut view, &u).0.registered, 1);
        assert!(!at(&doc, &view, 0).open);
    }
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 1).unwrap();
    policy(&mut view, 1024, 3);
    let u = doc.append("abc\n").unwrap();
    apply(&doc, &mut view, &u);
    let u = doc.append(&"x".repeat(2048)).unwrap();
    assert_eq!(apply(&doc, &mut view, &u).0.registered, 1);
    let c = at(&doc, &view, 0);
    view.set_collapsed(&doc, c.id, false).unwrap();
    let u = doc.append("more").unwrap();
    assert_eq!(apply(&doc, &mut view, &u).0.registered, 0);
    assert_eq!(at(&doc, &view, 0).id, c.id);
    assert!(!view.is_collapsed(c.id));
}

#[test]
fn budgets_are_atomic_and_deep_registration_remains_private_until_commit() {
    let doc = parse(&format!("{}body\n", ">".repeat(1000)));
    let mut view = FoldView::new(&doc, 1).unwrap();
    policy(&mut view, 0, 1);
    let mut task = ParagraphAutoFold::new(&doc, &view, Span::new(500, 501)).unwrap();
    for limit in [
        WorkLimit::default(),
        WorkLimit {
            queries: 1,
            ..Default::default()
        },
        WorkLimit {
            objects: 1,
            ..Default::default()
        },
    ] {
        let mut budget = Budget::new(limit);
        assert_eq!(
            task.advance(&doc, &mut view, &mut budget)
                .unwrap()
                .completion,
            Completion::BudgetExhausted
        );
        assert_eq!(budget.used(), WorkLimit::default());
    }
    let mut steps = 0;
    loop {
        steps += 1;
        let p = task.advance(&doc, &mut view, &mut frame(1)).unwrap();
        if p.completion == Completion::Covered {
            assert_eq!(steps, 2006); // discovery + two lookups/ends + 2000 ancestors + commit
            assert_eq!(p.registered, 1);
            assert_eq!(p.inspected, 1);
            assert_eq!(p.change.unwrap().source.start, 1);
            assert!(view.is_collapsed(at(&doc, &view, 0).id));
            break;
        }
        assert_eq!(view.source_fold_count(), 0);
        assert_eq!(task.remaining_range(), Span::new(0, 501));
        assert!(steps < 2006);
    }
    let mut other = FoldView::new(&doc, 1).unwrap();
    policy(&mut other, 0, 0);
    {
        let mut task = ParagraphAutoFold::new(&doc, &other, Span::new(0, 1)).unwrap();
        task.advance(&doc, &mut other, &mut frame(1000)).unwrap();
    }
    assert_eq!(other.source_fold_count(), 0);
    assert_eq!(run(&doc, &mut other).1, 2006);
}

#[test]
fn view_choices_exclusions_removal_preview_and_defaults_are_independent() {
    let mut doc = Document::new();
    doc.append("short\n").unwrap();
    let mut a = FoldView::new(&doc, 1).unwrap();
    let mut b = FoldView::new(&doc, 1).unwrap();
    policy(&mut a, 10, 2);
    policy(&mut b, 10, 3);
    a.set_paragraph_auto_excluded(&doc, BlockId(0), true)
        .unwrap();
    let u = doc.append("longer tail\n").unwrap();
    assert_eq!(apply(&doc, &mut a, &u).0.registered, 0);
    assert_eq!(apply(&doc, &mut b, &u).0.registered, 1);
    let bc = at(&doc, &b, 0);
    a.set_paragraph_auto_excluded(&doc, BlockId(0), false)
        .unwrap();
    assert_eq!(run(&doc, &mut a).0.registered, 1);
    let ac = at(&doc, &a, 0);
    assert_ne!(ac.id, bc.id);
    a.set_collapsed(&doc, ac.id, false).unwrap();
    b.set_preview(&doc, Some(bc.id), &mut frame(1)).unwrap();
    policy(&mut a, 0, 0);
    policy(&mut b, 0, 0);
    a.reset(4).unwrap();
    b.reset(4).unwrap();
    assert_eq!(run(&doc, &mut a).0.registered, 0);
    assert_eq!(run(&doc, &mut b).0.registered, 0);
    assert_eq!(at(&doc, &a, 0), ac);
    assert!(!a.is_collapsed(ac.id));
    assert_eq!(b.preview(), Some(bc.id));
    a.remove_source_fold(&doc, source_id(ac.id)).unwrap();
    let u = doc.append("more\n").unwrap();
    assert_eq!(apply(&doc, &mut a, &u).0.registered, 0);
    apply(&doc, &mut b, &u);
    assert_eq!(b.preview(), Some(bc.id));
    assert_eq!(at(&doc, &b, 0).id, bc.id);
    a.set_paragraph_auto_excluded(&doc, BlockId(0), false)
        .unwrap();
    assert_eq!(run(&doc, &mut a).0.registered, 1);
    assert_ne!(at(&doc, &a, 0).id, ac.id);
    a.set_defaults(FoldDefaults::default());
    assert_eq!(run(&doc, &mut a).1, 0);
    assert!(a.is_collapsed(at(&doc, &a, 0).id));
}

#[test]
fn manual_registration_and_overlap_win_without_changing_choices() {
    let doc = parse("first paragraph\n\nsecond paragraph\n\nthird paragraph\n");
    let mut view = FoldView::new(&doc, 1).unwrap();
    policy(&mut view, 0, 2);
    let mut manual = SourceFoldRegistration::new(&doc, &view, Span::new(2, 5)).unwrap();
    let id = manual
        .advance(&doc, &mut view, &mut frame(100))
        .unwrap()
        .id
        .unwrap();
    let second = doc.block_at(doc.source().find("second").unwrap()).unwrap();
    let mut manual = SourceFoldRegistration::paragraph(&doc, &view, second, 1).unwrap();
    let second_id = manual
        .advance(&doc, &mut view, &mut frame(100))
        .unwrap()
        .id
        .unwrap();
    let (p, _) = run(&doc, &mut view);
    assert_eq!((p.registered, p.skipped), (1, 2));
    assert_eq!(view.source_fold_count(), 3);
    assert!(!view.is_collapsed(id));
    assert!(!view.is_collapsed(second_id));
    let mut batch = FoldBatch::new(&doc, &view, FoldBatchAction::All { collapsed: false }).unwrap();
    batch.advance(&doc, &mut view, &mut frame(100)).unwrap();
    assert_eq!(run(&doc, &mut view).0.registered, 0);
    assert!(!view.is_collapsed(at(&doc, &view, doc.source().find("third").unwrap()).id));
}

#[test]
fn late_promotions_remove_without_transfer_and_contraction_preserves_admitted_choice() {
    for (initial, tail) in [
        ("one\ntwo\n", "===\n"),
        ("> one\n> two\n", "> ===\n"),
        ("a|b\n", "-|-\n"),
        ("> a|b\n", "> -|-\n"),
    ] {
        let mut doc = Document::new();
        doc.append(initial).unwrap();
        let mut view = FoldView::new(&doc, 1).unwrap();
        policy(&mut view, 0, 1);
        assert_eq!(run(&doc, &mut view).0.registered, 1);
        let c = at(&doc, &view, 0);
        view.set_preview(&doc, Some(c.id), &mut frame(1)).unwrap();
        let u = doc.append(tail).unwrap();
        assert_eq!(apply(&doc, &mut view, &u).0.registered, 0);
        assert_eq!(view.source_fold_count(), 0);
        assert!(view.preview().is_none());
        assert!(view.fold_candidate(&doc, c.id).unwrap().is_none());
    }
    for (lead, prefix, tail, end) in [
        ("lead\n", "", "|b\n-|-\n", 5),
        ("> lead\n", "> ", "|b\n> -|-\n", 7),
    ] {
        let initial = format!("{lead}{prefix}{}", "x".repeat(2048));
        for preview in [1, end] {
            let mut doc = Document::new();
            doc.append(&initial).unwrap();
            let mut view = FoldView::new(&doc, 1).unwrap();
            policy(&mut view, 1000, preview);
            assert_eq!(run(&doc, &mut view).0.registered, 1);
            let c = at(&doc, &view, 0);
            view.set_collapsed(&doc, c.id, false).unwrap();
            let u = doc.append(tail).unwrap();
            assert_eq!(apply(&doc, &mut view, &u).0.registered, 0);
            if preview < end {
                let now = at(&doc, &view, 0);
                assert_eq!(now.id, c.id);
                assert_eq!(now.source.end, end);
                assert!(!view.is_collapsed(c.id));
            } else {
                assert_eq!(view.source_fold_count(), 0);
            }
        }
    }
}

#[test]
fn stale_tasks_wrong_view_updates_and_invalid_windows_are_side_effect_free() {
    let mut doc = Document::new();
    doc.append("one\n").unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    policy(&mut view, 0, 1);
    let mut task = ParagraphAutoFold::new(&doc, &view, Span::new(0, 4)).unwrap();
    let mut other = FoldView::new(&doc, 1).unwrap();
    let mut budget = frame(100);
    assert_eq!(
        task.advance(&doc, &mut other, &mut budget),
        Err(LayoutError::WrongView)
    );
    assert_eq!(
        task.advance(&parse("one\n"), &mut view, &mut budget),
        Err(LayoutError::WrongDocument)
    );
    assert_eq!(budget.used(), WorkLimit::default());
    task.advance(&doc, &mut view, &mut frame(2)).unwrap();
    view.set_paragraph_auto_excluded(&doc, BlockId(0), true)
        .unwrap();
    assert_eq!(
        task.advance(&doc, &mut view, &mut budget),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(budget.used(), WorkLimit::default());
    assert_eq!(view.source_fold_count(), 0);
    let mut task = ParagraphAutoFold::new(&doc, &view, Span::new(0, 4)).unwrap();
    let u = doc.append("more\n").unwrap();
    assert!(matches!(
        ParagraphAutoFold::for_update(&doc, &view, &u),
        Err(LayoutError::OutOfOrderUpdate)
    ));
    assert_eq!(
        task.advance(&doc, &mut view, &mut budget),
        Err(LayoutError::StaleContinuation)
    );
    apply(&doc, &mut view, &u);
    let old = u;
    let u = doc.finish();
    apply(&doc, &mut view, &u);
    assert!(matches!(
        ParagraphAutoFold::for_update(&doc, &view, &old),
        Err(LayoutError::OutOfOrderUpdate)
    ));
    for range in [Span::new(2, 1), Span::new(0, usize::MAX)] {
        assert!(matches!(
            ParagraphAutoFold::new(&doc, &view, range),
            Err(LayoutError::InvalidRange)
        ));
    }
    assert_eq!(
        view.set_paragraph_auto_excluded(&doc, BlockId(99), true),
        Err(LayoutError::InvalidRange)
    );
    assert_eq!(budget.used(), WorkLimit::default());
}

#[test]
fn unfinished_windows_can_be_carried_across_merged_updates_and_replay() {
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 1).unwrap();
    policy(&mut view, 0, 1);
    let u = doc
        .append("first paragraph\n\nsecond paragraph\n\n")
        .unwrap();
    view.apply(&doc, &u).unwrap();
    let mut task = ParagraphAutoFold::for_update(&doc, &view, &u).unwrap();
    let p = task.advance(&doc, &mut view, &mut frame(8)).unwrap();
    assert_eq!(p.registered, 1);
    assert_eq!(view.source_fold_count(), 1);
    let remaining = task.remaining_range();
    assert!(remaining.start > 0);
    let mut u = doc.append("third\n").unwrap();
    u.merge(doc.append("paragraph\n\n").unwrap()).unwrap();
    u.merge(doc.finish()).unwrap();
    view.apply(&doc, &u).unwrap();
    view.apply(&doc, &u).unwrap();
    assert_eq!(
        task.advance(&doc, &mut view, &mut frame(1)),
        Err(LayoutError::StaleContinuation)
    );
    let new_range = ParagraphAutoFold::for_update(&doc, &view, &u)
        .unwrap()
        .remaining_range();
    let mut task = ParagraphAutoFold::new(
        &doc,
        &view,
        Span::new(remaining.start.min(new_range.start), new_range.end),
    )
    .unwrap();
    assert_eq!(drain(&doc, &mut view, &mut task).0.registered, 2);
    assert_eq!(view.source_fold_count(), 3);
    assert_eq!(apply(&doc, &mut view, &u).0.registered, 0);
    let u = doc.finish();
    assert_eq!(apply(&doc, &mut view, &u).1, 0);
}

#[test]
fn utf8_all_splits_and_each_prefix_match_full_window_policy() {
    let sources = [
        "中🙂 *text*\r\nmore\r\n\r\na|b\n-|-\n\nplain\n===\n\nend",
        "> 中🙂\n> lazy\n>\n> lead\n> a|b\n> -|-\n\n- one\n  two\n- last\n",
        "> title\n> more\n> ===\n>\n> x\ncontinuation\n\nend\n",
        "-\t中🙂\n\tmore\r\n\r\nlast\r",
    ];
    for source in sources {
        for split in (0..=source.len()).filter(|&at| source.is_char_boundary(at)) {
            let mut doc = Document::new();
            let mut a = FoldView::new(&doc, 1).unwrap();
            let mut b = FoldView::new(&doc, 1).unwrap();
            policy(&mut a, 6, 2);
            policy(&mut b, 6, 2);
            for part in [&source[..split], &source[split..]] {
                let u = doc.append(part).unwrap();
                apply(&doc, &mut a, &u);
                b.apply(&doc, &u).unwrap();
                run(&doc, &mut b);
                assert_eq!(
                    registrations(&doc, &a),
                    registrations(&doc, &b),
                    "{source:?} split {split}"
                );
            }
            let u = doc.finish();
            apply(&doc, &mut a, &u);
            b.apply(&doc, &u).unwrap();
            run(&doc, &mut b);
            assert_eq!(registrations(&doc, &a), registrations(&doc, &b));
        }
        let mut doc = Document::new();
        let mut a = FoldView::new(&doc, 1).unwrap();
        let mut b = FoldView::new(&doc, 1).unwrap();
        policy(&mut a, 6, 2);
        policy(&mut b, 6, 2);
        for ch in source.chars() {
            let u = doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            apply(&doc, &mut a, &u);
            b.apply(&doc, &u).unwrap();
            run(&doc, &mut b);
            assert_eq!(registrations(&doc, &a), registrations(&doc, &b));
        }
    }
}

#[test]
fn sparse_nonparagraphs_share_budget_and_disabled_policy_is_free() {
    let doc = parse(&format!("{}tail\n", "# h\n".repeat(4096)));
    let mut view = FoldView::new(&doc, 1).unwrap();
    assert_eq!(run(&doc, &mut view).1, 0);
    policy(&mut view, 0, 0);
    let mut task = ParagraphAutoFold::new(&doc, &view, Span::new(0, doc.source().len())).unwrap();
    for i in 0..4096 {
        let p = task.advance(&doc, &mut view, &mut frame(1)).unwrap();
        assert_eq!(p.inspected, i + 1);
        assert_eq!(p.registered, 0);
        assert_eq!(p.completion, Completion::BudgetExhausted);
    }
    assert_eq!(drain(&doc, &mut view, &mut task).0.registered, 1);
}

#[test]
fn million_byte_append_does_not_revisit_history_or_deep_ancestors() {
    for depth in [0, 1000] {
        let mut doc = Document::new();
        doc.append(&"old paragraph\n\n".repeat(4096)).unwrap();
        let mut view = FoldView::new(&doc, 1).unwrap();
        policy(&mut view, 0, 3);
        assert_eq!(run(&doc, &mut view).0.registered, 4096);
        let start = doc.source().len();
        let u = doc
            .append(&format!("{}{}", ">".repeat(depth), "x".repeat(1_000_000)))
            .unwrap();
        let (p, steps) = apply(&doc, &mut view, &u);
        assert_eq!(p.registered, 1);
        assert!(steps <= 2 * depth + 8);
        let c = at(&doc, &view, start);
        let work = doc.work();
        let visits = view.range_visits();
        for _ in 0..1000 {
            let u = doc.append("x").unwrap();
            let (p, steps) = apply(&doc, &mut view, &u);
            assert_eq!(steps, 1);
            assert_eq!((p.inspected, p.registered, p.skipped), (1, 0, 1));
            assert_eq!(at(&doc, &view, start).id, c.id);
        }
        assert_eq!(view.range_visits() - visits, 1000);
        assert_eq!(doc.work().inline_bytes - work.inline_bytes, 1000);
        assert_eq!(doc.work().container_steps, work.container_steps);
        assert_eq!(doc.work().paragraph_closures, work.paragraph_closures);
        assert_eq!(view.source_fold_count(), 4097);
    }
}

#[test]
fn multi_leaf_paragraph_and_nested_lists_charge_only_discovery_and_endpoints() {
    let doc = parse(&"> line\n".repeat(4096));
    let mut view = FoldView::new(&doc, 1).unwrap();
    policy(&mut view, 0, 2);
    let (p, steps) = run(&doc, &mut view);
    assert_eq!(p.inspected, 1);
    assert_eq!(p.registered, 1);
    assert_eq!(steps, 8);
    assert_eq!(doc.work().paragraph_closures, 1);
    let mut doc = Document::new();
    for depth in 0..250 {
        doc.append(&format!("{}- item\n", "  ".repeat(depth)))
            .unwrap();
    }
    doc.finish();
    let mut view = FoldView::new(&doc, 1).unwrap();
    policy(&mut view, 0, 0);
    let end = doc.source().len();
    let mut task = ParagraphAutoFold::new(&doc, &view, Span::new(end - 1, end)).unwrap();
    assert_eq!(drain(&doc, &mut view, &mut task).1, 1006);
    assert_eq!(
        view.paragraph_fold_block(&doc, source_id(at(&doc, &view, end - 1).id))
            .unwrap(),
        Some(BlockId(249))
    );
}

#[test]
fn source_windows_and_length_policy_match_independent_paragraph_spans() {
    let mut source = String::new();
    let mut paragraphs = Vec::new();
    for i in 0..400 {
        let start = source.len();
        let body = "x".repeat(i % 73 + 1);
        match i % 3 {
            0 => source.push_str(&format!("{body}\ncontinued\n")),
            1 => source.push_str(&format!("> {body}\n> continued\n")),
            _ => source.push_str(&format!("- {body}\n  continued\n")),
        }
        paragraphs.push(Span::new(start, source.len()));
        source.push_str("\n# boundary\n\n");
    }
    let doc = parse(&source);
    let mut seed = 832u64;
    for _ in 0..400 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let start = seed as usize % source.len();
        let end = (start + (seed >> 32) as usize % 2000).min(source.len());
        let min = (seed >> 20) as usize % 90;
        let prefix = (seed >> 16) as usize % 100;
        let want: Vec<_> = paragraphs
            .iter()
            .copied()
            .filter(|s| {
                start < end && s.start < end && start < s.end && s.len() >= min && s.len() > prefix
            })
            .collect();
        let mut view = FoldView::new(&doc, 1).unwrap();
        policy(&mut view, min, prefix);
        let mut task = ParagraphAutoFold::new(&doc, &view, Span::new(start, end)).unwrap();
        let (p, _) = drain(&doc, &mut view, &mut task);
        assert_eq!(p.registered, want.len());
        let got = registrations(&doc, &view);
        assert_eq!(got.iter().map(|(_, s, _, _)| *s).collect::<Vec<_>>(), want);
        for (_, span, hidden, open) in got {
            assert_eq!(hidden, Span::new(span.start + prefix, span.end));
            assert!(!open);
        }
    }
}

#[test]
fn admission_timing_is_explicit_and_policy_changes_invalidate_pending_work() {
    let mut doc = Document::new();
    doc.append(&format!("lead\n{}", "x".repeat(2048))).unwrap();
    let mut early = FoldView::new(&doc, 1).unwrap();
    let mut deferred = FoldView::new(&doc, 1).unwrap();
    policy(&mut early, 1000, 1);
    policy(&mut deferred, 1000, 1);
    run(&doc, &mut early);
    let mut task = ParagraphAutoFold::new(&doc, &deferred, Span::new(0, 1)).unwrap();
    task.advance(&doc, &mut deferred, &mut frame(3)).unwrap();
    policy(&mut deferred, 2000, 2);
    let mut budget = frame(100);
    assert_eq!(
        task.advance(&doc, &mut deferred, &mut budget),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(budget.used(), WorkLimit::default());
    assert_eq!(deferred.source_fold_count(), 0);
    let u = doc.append("|b\n-|-\n").unwrap();
    apply(&doc, &mut early, &u);
    apply(&doc, &mut deferred, &u);
    // Initialization is observed once, not re-enforced on late contraction.
    assert_eq!(at(&doc, &early, 0).source, Span::new(0, 5));
    assert_eq!(deferred.source_fold_count(), 0);
    assert_eq!(run(&doc, &mut early).0.registered, 0);
    assert_eq!(run(&doc, &mut deferred).0.registered, 0);
}
