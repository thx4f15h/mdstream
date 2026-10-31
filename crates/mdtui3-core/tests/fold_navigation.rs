use mdtui3::*;

fn document(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn heading(id: usize) -> FoldId {
    FoldId::Heading(HeadingId(id))
}

fn source(doc: &Document, id: usize) -> usize {
    doc.headings().get(HeadingId(id)).unwrap().source.start
}

fn tiny() -> Budget {
    Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        ..Default::default()
    })
}

fn finish(
    task: &mut FoldNavigation,
    doc: &Document,
    view: &mut FoldView,
) -> (Option<ScrollTarget>, usize, Vec<FoldChange>) {
    let mut changes = Vec::new();
    for step in 1..1000 {
        let mut budget = tiny();
        let progress = task.advance(doc, view, &mut budget).unwrap();
        assert_eq!(budget.used().bytes, 0);
        assert_eq!(budget.used().tiles, 0);
        if let Some(change) = progress.change {
            changes.push(change);
        }
        if progress.completion == Completion::Covered {
            let again = task
                .advance(doc, view, &mut Budget::new(WorkLimit::default()))
                .unwrap();
            assert_eq!(again.target, progress.target);
            assert_eq!(again.completion, Completion::Covered);
            assert_eq!(again.change, None);
            return (progress.target, step, changes);
        }
        assert_eq!(progress.completion, Completion::BudgetExhausted);
        assert_eq!(progress.target, None);
    }
    panic!("navigation made no progress");
}

fn run(
    doc: &Document,
    view: &mut FoldView,
    nav: &Navigator,
    action: NavAction,
    policy: FoldNavigationPolicy,
) -> (Option<ScrollTarget>, usize, Vec<FoldChange>) {
    let mut task = FoldNavigation::new(doc, view, nav, action, policy, 1).unwrap();
    finish(&mut task, doc, view)
}

#[test]
fn reveal_keeps_target_body_and_unrelated_choices_with_exact_same_tile_jumps() {
    let doc = document("# A\na\n## B\nb\n### C\nc\n## D\nd\n# E\ne\n");
    let mut view = FoldView::new(&doc, 1).unwrap();
    for id in 0..5 {
        view.set_collapsed(&doc, heading(id), true).unwrap();
    }
    let mut nav = Navigator::new();
    let (target, steps, changes) = run(
        &doc,
        &mut view,
        &nav,
        NavAction::Heading(HeadingId(2)),
        FoldNavigationPolicy::Reveal,
    );
    let target = target.unwrap();
    assert_eq!(target.source, Some(source(&doc, 2)));
    assert_eq!(steps, 5); // resolve, two opens, visible lookup, row lookup
    assert_eq!(changes.len(), 2);
    assert!(changes[0].source.range().contains(&source(&doc, 2)));
    assert_eq!(changes[1].version, view.version());
    for (id, collapsed) in [(0, false), (1, false), (2, true), (3, true), (4, true)] {
        assert_eq!(view.is_collapsed(heading(id)), collapsed);
    }
    nav.apply_target(target);
    let (next, _, _) = run(
        &doc,
        &mut view,
        &nav,
        NavAction::NextHeading { max_level: 6 },
        FoldNavigationPolicy::Reveal,
    );
    assert_eq!(next.unwrap().source, Some(source(&doc, 3)));
    assert!(view.is_collapsed(heading(3)));
    assert_eq!(doc.tile_count(), 1);
}

#[test]
fn source_reveal_opens_six_ancestors_and_code_without_reading_million_byte_body() {
    let mut text = String::new();
    for level in 1..=6 {
        text.push_str(&format!("{} level\n", "#".repeat(level)));
    }
    text.push_str("```rust\n");
    let destination = text.len() + 500_000;
    text.push_str(&"x".repeat(1_000_000));
    text.push_str("\n```\n# next\n");
    let doc = document(&text);
    let code = FoldId::Code(doc.block_at(destination).unwrap());
    let mut view = FoldView::new(&doc, 1).unwrap();
    for id in 0..6 {
        view.set_collapsed(&doc, heading(id), true).unwrap();
    }
    view.set_collapsed(&doc, code, true).unwrap();
    let ticket = view.heights().ticket(TileId(0)).unwrap();
    let before = doc.work();
    let visits = view.heights().mutation_visits();
    let (target, steps, changes) = run(
        &doc,
        &mut view,
        &Navigator::new(),
        NavAction::Source(destination),
        FoldNavigationPolicy::Reveal,
    );
    assert_eq!(target.unwrap().source, Some(destination));
    assert_eq!(steps, 10);
    assert_eq!(changes.len(), 7);
    assert!(!view.is_collapsed(code));
    assert!(view.heights().mutation_visits() - visits < 15_000);
    assert_eq!(doc.work(), before);
    assert_eq!(view.measure(ticket, 9), Err(LayoutError::StaleMeasurement));
}

#[test]
fn visible_navigation_skips_fifty_thousand_headings_in_constant_steps() {
    let doc = document(&format!(
        "# Parent\n{}# Next\n",
        "## hidden\n".repeat(50_000)
    ));
    let mut view = FoldView::new(&doc, 1).unwrap();
    view.set_collapsed(&doc, heading(0), true).unwrap();
    let mut nav = Navigator::new();
    let (next, steps, changes) = run(
        &doc,
        &mut view,
        &nav,
        NavAction::NextHeading { max_level: 6 },
        FoldNavigationPolicy::VisibleOnly,
    );
    let next = next.unwrap();
    assert_eq!(next.source, Some(source(&doc, 50_001)));
    assert_eq!(steps, 4);
    assert!(changes.is_empty());
    nav.apply_target(next);
    let (previous, steps, _) = run(
        &doc,
        &mut view,
        &nav,
        NavAction::PreviousHeading { max_level: 6 },
        FoldNavigationPolicy::VisibleOnly,
    );
    assert_eq!(previous.unwrap().source, Some(0));
    assert_eq!(steps, 4);
    assert!(view.is_collapsed(heading(0)));
}

#[test]
fn filtered_traversal_resumes_across_many_disjoint_hidden_ranges() {
    let mut text = String::from("## start\n");
    for _ in 0..100 {
        text.push_str("# parent\n## hidden\n");
    }
    text.push_str("# end\n## destination\n");
    let doc = document(&text);
    let mut view = FoldView::new(&doc, 1).unwrap();
    for id in (1..201).step_by(2) {
        view.set_collapsed(&doc, heading(id), true).unwrap();
    }
    let mut nav = Navigator::new();
    let (next, steps, _) = run(
        &doc,
        &mut view,
        &nav,
        NavAction::NextAtLevel { level: 2 },
        FoldNavigationPolicy::VisibleOnly,
    );
    assert_eq!(next.unwrap().source, Some(source(&doc, 202)));
    assert_eq!(steps, 103);
    nav.reconcile_source(view.heights(), 0, 1, source(&doc, 202));
    let (previous, steps, _) = run(
        &doc,
        &mut view,
        &nav,
        NavAction::PreviousAtLevel { level: 2 },
        FoldNavigationPolicy::VisibleOnly,
    );
    assert_eq!(previous.unwrap().source, Some(0));
    assert_eq!(steps, 103);
}

#[test]
fn visible_policy_matches_naive_filtered_heading_list() {
    let mut seed = 0xcafe_f00du64;
    let mut random = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        (seed >> 32) as usize
    };
    let mut text = String::new();
    for _ in 0..80 {
        text.push_str(&format!("{} 重复\nbody\n", "#".repeat(random() % 6 + 1)));
    }
    let doc = document(&text);
    let mut view = FoldView::new(&doc, 1).unwrap();
    let mut hidden = Vec::new();
    for id in 0..80 {
        if random() % 3 == 0 {
            view.set_collapsed(&doc, heading(id), true).unwrap();
            hidden.push(doc.fold_candidate(heading(id)).unwrap().hidden);
        }
    }
    for _ in 0..300 {
        let offset = source(&doc, random() % 80);
        let level = (random() % 8) as u8;
        let reverse = random() % 2 == 0;
        let exact = random() % 2 == 0;
        let action = match (reverse, exact) {
            (false, false) => NavAction::NextHeading { max_level: level },
            (true, false) => NavAction::PreviousHeading { max_level: level },
            (false, true) => NavAction::NextAtLevel { level },
            (true, true) => NavAction::PreviousAtLevel { level },
        };
        let eligible: Vec<_> = doc
            .headings()
            .entries()
            .iter()
            .filter(|h| {
                (if exact {
                    h.level == level
                } else {
                    h.level <= level
                }) && (if reverse {
                    h.source.start < offset
                } else {
                    h.source.start > offset
                }) && !hidden
                    .iter()
                    .any(|span| span.range().contains(&h.source.start))
            })
            .collect();
        let expected = if reverse {
            eligible.last()
        } else {
            eligible.first()
        }
        .map(|h| h.source.start);
        let mut nav = Navigator::new();
        nav.reconcile_source(view.heights(), 0, 1, offset);
        let (actual, _, changes) = run(
            &doc,
            &mut view,
            &nav,
            action,
            FoldNavigationPolicy::VisibleOnly,
        );
        assert_eq!(
            actual.and_then(|t| t.source),
            expected,
            "offset={offset}, action={action:?}"
        );
        assert!(changes.is_empty());
    }
}

#[test]
fn exact_and_structural_actions_have_explicit_hidden_and_missing_results() {
    let doc = document("# A\n## B\nb\n### C\nc\n## D\nd\n");
    let mut view = FoldView::new(&doc, 1).unwrap();
    view.set_collapsed(&doc, heading(0), true).unwrap();
    for action in [
        NavAction::Heading(HeadingId(1)),
        NavAction::Source(source(&doc, 1)),
        NavAction::FirstChildHeading,
        NavAction::Heading(HeadingId(999)),
        NavAction::ParentHeading,
        NavAction::PreviousSiblingHeading,
    ] {
        let (target, _, changes) = run(
            &doc,
            &mut view,
            &Navigator::new(),
            action,
            FoldNavigationPolicy::VisibleOnly,
        );
        assert_eq!(target, None);
        assert!(changes.is_empty());
    }
    let (target, _, _) = run(
        &doc,
        &mut view,
        &Navigator::new(),
        NavAction::FirstChildHeading,
        FoldNavigationPolicy::Reveal,
    );
    assert_eq!(target.unwrap().source, Some(source(&doc, 1)));
    let mut nav = Navigator::new();
    nav.reconcile_source(view.heights(), 0, 1, source(&doc, 2));
    let (target, _, _) = run(
        &doc,
        &mut view,
        &nav,
        NavAction::ParentHeading,
        FoldNavigationPolicy::Reveal,
    );
    assert_eq!(target.unwrap().source, Some(source(&doc, 1)));
}

#[test]
fn budgets_are_atomic_and_changes_are_aggregated_per_call() {
    let doc = document("# A\n## B\n### C\nbody\n");
    let mut view = FoldView::new(&doc, 1).unwrap();
    for id in 0..3 {
        view.set_collapsed(&doc, heading(id), true).unwrap();
    }
    let mut task = FoldNavigation::new(
        &doc,
        &view,
        &Navigator::new(),
        NavAction::Source(doc.source().len() - 1),
        FoldNavigationPolicy::Reveal,
        1,
    )
    .unwrap();
    let version = view.version();
    for limit in [
        WorkLimit::default(),
        WorkLimit {
            queries: 10,
            ..Default::default()
        },
        WorkLimit {
            objects: 10,
            ..Default::default()
        },
    ] {
        let mut budget = Budget::new(limit);
        let progress = task.advance(&doc, &mut view, &mut budget).unwrap();
        assert_eq!(progress.completion, Completion::BudgetExhausted);
        assert_eq!(budget.used(), WorkLimit::default());
        assert_eq!(view.version(), version);
    }
    let progress = task
        .advance(&doc, &mut view, &mut Budget::new(WorkLimit::unlimited()))
        .unwrap();
    assert_eq!(progress.completion, Completion::Covered);
    assert_eq!(
        progress.change,
        Some(FoldChange {
            version: version + 3,
            source: doc.fold_candidate(heading(0)).unwrap().hidden
        })
    );
}

#[test]
fn stale_tasks_reject_document_view_fold_and_resize_changes_before_work() {
    let mut doc = Document::new();
    doc.append("# A\n## B\nbody\n").unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    for mutation in 0..4 {
        let mut task = FoldNavigation::new(
            &doc,
            &view,
            &Navigator::new(),
            NavAction::Heading(HeadingId(1)),
            FoldNavigationPolicy::Reveal,
            1,
        )
        .unwrap();
        task.advance(&doc, &mut view, &mut tiny()).unwrap();
        match mutation {
            0 => {
                view.toggle(&doc, heading(0)).unwrap();
            }
            1 => {
                view.reset(2).unwrap();
            }
            2 => {
                let update = doc.append("more\n").unwrap();
                view.apply(&doc, &update).unwrap();
            }
            _ => {
                let update = doc.finish();
                view.apply(&doc, &update).unwrap();
            }
        }
        let mut budget = tiny();
        assert_eq!(
            task.advance(&doc, &mut view, &mut budget),
            Err(LayoutError::StaleContinuation)
        );
        assert_eq!(budget.used(), WorkLimit::default());
    }
    let mut task = FoldNavigation::new(
        &doc,
        &view,
        &Navigator::new(),
        NavAction::Home,
        FoldNavigationPolicy::Reveal,
        1,
    )
    .unwrap();
    let mut other_view = FoldView::new(&doc, 1).unwrap();
    assert_eq!(
        task.advance(&doc, &mut other_view, &mut tiny()),
        Err(LayoutError::WrongView)
    );
    let other = document(doc.source());
    assert_eq!(
        task.advance(&other, &mut view, &mut tiny()),
        Err(LayoutError::WrongDocument)
    );
}

#[test]
fn measurement_between_steps_updates_estimate_and_regular_scrolls_match_navigator() {
    let doc = document(&format!("# A\n{}\n# B\n", "x".repeat(2000)));
    let mut view = FoldView::new(&doc, 1).unwrap();
    let mut task = FoldNavigation::new(
        &doc,
        &view,
        &Navigator::new(),
        NavAction::Heading(HeadingId(1)),
        FoldNavigationPolicy::Reveal,
        1,
    )
    .unwrap();
    task.advance(&doc, &mut view, &mut tiny()).unwrap();
    view.measure(view.heights().ticket(TileId(0)).unwrap(), 100)
        .unwrap();
    let target = finish(&mut task, &doc, &mut view).0.unwrap();
    assert_eq!(
        target.top,
        view.heights()
            .prefix_rows(doc.tile_at(source(&doc, 1)).unwrap().0)
            .unwrap()
    );
    assert!(target.top >= 100);
    let mut nav = Navigator::new();
    for action in [
        NavAction::LineDown,
        NavAction::PageDown,
        NavAction::LineUp,
        NavAction::PageUp,
        NavAction::End,
        NavAction::Home,
    ] {
        let (target, steps, changes) =
            run(&doc, &mut view, &nav, action, FoldNavigationPolicy::Reveal);
        assert_eq!(target, nav.navigate(&doc, view.heights(), action, 1));
        assert_eq!(steps, 1);
        assert!(changes.is_empty());
    }
}

#[test]
fn applying_scroll_clears_old_source_before_the_next_heading_action() {
    let doc = document("# A\n# B\n# C\n# D\n");
    let mut view = FoldView::new(&doc, 1).unwrap();
    view.measure(view.heights().ticket(TileId(0)).unwrap(), 4)
        .unwrap();
    let mut nav = Navigator::new();
    nav.reconcile_source(view.heights(), 2, 1, source(&doc, 2));
    let (target, _, _) = run(
        &doc,
        &mut view,
        &nav,
        NavAction::Home,
        FoldNavigationPolicy::Reveal,
    );
    let target = target.unwrap();
    assert_eq!(target.source, None);
    nav.apply_target(target);
    let (next, _, _) = run(
        &doc,
        &mut view,
        &nav,
        NavAction::NextHeading { max_level: 6 },
        FoldNavigationPolicy::Reveal,
    );
    assert_eq!(next.unwrap().source, Some(source(&doc, 1)));

    // The renderer supplies the actual source after a row-only scroll.
    nav.reconcile_source(view.heights(), 1, 1, source(&doc, 1));
    let (next, _, _) = run(
        &doc,
        &mut view,
        &nav,
        NavAction::NextHeading { max_level: 6 },
        FoldNavigationPolicy::Reveal,
    );
    assert_eq!(next.unwrap().source, Some(source(&doc, 2)));
}

#[test]
fn source_anchor_projects_to_outer_placeholder_and_recovers_after_append_and_resize() {
    let mut doc = Document::new();
    doc.append("# A\n## B\n```\n中文\n").unwrap();
    let source = doc.source().find("中文").unwrap();
    let anchor = FoldAnchor::new(&doc, source).unwrap();
    let code = FoldId::Code(doc.block_at(source).unwrap());
    let mut view = FoldView::new(&doc, 1).unwrap();
    for id in [code, heading(1), heading(0)] {
        view.set_collapsed(&doc, id, true).unwrap();
    }
    let update = doc.append("more\n```\n# C\n").unwrap();
    view.apply(&doc, &update).unwrap();
    view.reset(4).unwrap();
    for hidden in [Some(heading(0)), Some(heading(1)), Some(code), None] {
        let projected = anchor
            .resolve(&doc, &view, 1, &mut tiny())
            .unwrap()
            .unwrap();
        assert_eq!(projected.hidden_by, hidden);
        assert_eq!(
            projected.target.source,
            Some(hidden.map_or(source, |id| doc.fold_candidate(id).unwrap().hidden.start))
        );
        assert_eq!(anchor.source(), source);
        if let Some(id) = hidden {
            view.set_collapsed(&doc, id, false).unwrap();
        }
    }
    let other_view = FoldView::new(&doc, 1).unwrap();
    assert_eq!(
        anchor
            .resolve(&doc, &other_view, 1, &mut tiny())
            .unwrap()
            .unwrap()
            .target
            .source,
        Some(source)
    );
    assert_eq!(
        anchor
            .resolve(&doc, &view, 1, &mut Budget::new(WorkLimit::default()))
            .unwrap(),
        None
    );
}

#[test]
fn source_boundaries_empty_eof_wrong_document_and_late_setext_are_defined() {
    let empty = Document::new();
    let mut view = FoldView::new(&empty, 1).unwrap();
    let anchor = FoldAnchor::new(&empty, 0).unwrap();
    let resolved = anchor
        .resolve(&empty, &view, 0, &mut tiny())
        .unwrap()
        .unwrap();
    assert_eq!(
        resolved.target,
        ScrollTarget {
            top: 0,
            anchor: None,
            source: Some(0)
        }
    );
    let (target, _, _) = run(
        &empty,
        &mut view,
        &Navigator::new(),
        NavAction::Source(0),
        FoldNavigationPolicy::Reveal,
    );
    assert_eq!(target, Some(resolved.target));
    let mut doc = Document::new();
    doc.append("# A\n中文\n").unwrap();
    let position = doc.source().find('中').unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let mut nav = Navigator::new();
    for invalid in [position + 1, doc.source().len() + 1, usize::MAX] {
        assert_eq!(
            nav.navigate(&doc, view.heights(), NavAction::Source(invalid), 1),
            None
        );
    }
    assert_eq!(
        FoldAnchor::new(&doc, position + 1),
        Err(LayoutError::InvalidRange)
    );
    assert!(matches!(
        FoldNavigation::new(
            &doc,
            &view,
            &Navigator::new(),
            NavAction::Source(usize::MAX),
            FoldNavigationPolicy::Reveal,
            1
        ),
        Err(LayoutError::InvalidRange)
    ));
    assert_eq!(
        anchor.resolve(&doc, &view, 1, &mut tiny()),
        Err(LayoutError::WrongDocument)
    );
    let anchor = FoldAnchor::new(&doc, position).unwrap();
    view.set_collapsed(&doc, heading(0), true).unwrap();
    assert_eq!(
        anchor
            .resolve(&doc, &view, 1, &mut tiny())
            .unwrap()
            .unwrap()
            .hidden_by,
        Some(heading(0))
    );
    let update = doc.append("===\n").unwrap();
    assert_eq!(
        anchor.resolve(&doc, &view, 1, &mut tiny()),
        Err(LayoutError::OutOfOrderUpdate)
    );
    view.apply(&doc, &update).unwrap();
    let resolved = anchor
        .resolve(&doc, &view, 1, &mut tiny())
        .unwrap()
        .unwrap();
    assert_eq!(resolved.hidden_by, None);
    assert_eq!(resolved.target.source, Some(position));
    let eof = FoldAnchor::new(&doc, doc.source().len()).unwrap();
    assert_eq!(
        eof.resolve(&doc, &view, 1, &mut tiny())
            .unwrap()
            .unwrap()
            .target
            .source,
        Some(doc.source().len())
    );
}

#[test]
fn table_bookmarks_project_to_summary_and_reveal_only_the_required_path() {
    let doc =
        document("# A\n## B\na | b\n- | -\nexact | value\n\n## Other\na | b\n- | -\nother | row\n");
    let destination = doc.source().find("exact").unwrap() + 2;
    let table = FoldId::Table(TableId(0));
    let other = FoldId::Table(TableId(1));
    let bookmark = Bookmark::new(&doc, destination, 2).unwrap();
    assert_eq!(bookmark.target(), Some(table));
    let mut view = FoldView::new(&doc, 1).unwrap();
    for id in [table, other, heading(2)] {
        view.set_collapsed(&doc, id, true).unwrap();
    }
    let anchor = bookmark
        .anchor()
        .resolve(&doc, &view, 24, &mut tiny())
        .unwrap()
        .unwrap();
    assert_eq!(anchor.hidden_by, Some(table));
    assert_eq!(anchor.target.source, Some(doc.tables()[0].source.start));
    for id in [heading(0), heading(1)] {
        view.set_collapsed(&doc, id, true).unwrap();
    }
    let anchor = bookmark
        .anchor()
        .resolve(&doc, &view, 24, &mut tiny())
        .unwrap()
        .unwrap();
    assert_eq!(anchor.hidden_by, Some(heading(0)));
    let mut task = bookmark.navigation(&doc, &view, 24).unwrap();
    let (target, steps, changes) = finish(&mut task, &doc, &mut view);
    assert!(steps > 1);
    assert_eq!(target.unwrap().source, Some(destination));
    assert_eq!(changes.len(), 3);
    for id in [heading(0), heading(1), table] {
        assert!(!view.is_collapsed(id));
    }
    assert!(view.is_collapsed(other));
    assert!(view.is_collapsed(heading(2)));
    let anchor = bookmark
        .anchor()
        .resolve(&doc, &view, 24, &mut tiny())
        .unwrap()
        .unwrap();
    assert_eq!(anchor.hidden_by, None);
    assert_eq!(anchor.target.source, Some(destination));
}
