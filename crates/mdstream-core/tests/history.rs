use mdstream::*;

fn document(text: &str) -> Document {
    let mut doc = Document::new();
    doc.append(text).unwrap();
    doc.finish();
    doc
}

fn tiny() -> Budget {
    Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        ..Default::default()
    })
}

fn heading(id: usize) -> FoldId {
    FoldId::Heading(HeadingId(id))
}

fn bookmark(doc: &Document, id: usize) -> Bookmark {
    Bookmark::new(
        doc,
        doc.headings().get(HeadingId(id)).unwrap().source.start,
        0,
    )
    .unwrap()
}

fn finish(
    task: &mut HistoryNavigation,
    doc: &Document,
    view: &mut FoldView,
    history: &mut NavigationHistory,
) -> (ScrollTarget, usize, Vec<FoldChange>) {
    let before = history.position();
    let mut changes = Vec::new();
    for step in 1..20 {
        let mut budget = tiny();
        let progress = task.advance(doc, view, history, &mut budget).unwrap();
        assert_eq!(budget.used().bytes, 0);
        assert_eq!(budget.used().tiles, 0);
        if let Some(change) = progress.change {
            changes.push(change);
        }
        if progress.completion == Completion::Covered {
            assert_eq!(history.current(), task.bookmark());
            let again = task
                .advance(doc, view, history, &mut Budget::new(WorkLimit::default()))
                .unwrap();
            assert_eq!(again.target, progress.target);
            assert_eq!(again.change, None);
            assert_eq!(again.completion, Completion::Covered);
            return (progress.target.unwrap(), step, changes);
        }
        assert_eq!(progress.completion, Completion::BudgetExhausted);
        assert_eq!(progress.target, None);
        assert_eq!(history.position(), before);
    }
    panic!("history navigation made no progress");
}

fn go(
    doc: &Document,
    view: &mut FoldView,
    history: &mut NavigationHistory,
    direction: HistoryDirection,
) -> Option<ScrollTarget> {
    let mut task = history.navigation(doc, view, direction, 1).unwrap()?;
    Some(finish(&mut task, doc, view, history).0)
}

#[test]
fn same_tile_duplicate_headings_and_scrolling_preserve_return_positions() {
    let doc = document("# Same\nbody\n### Same\n中文\n# Same\ntail\n");
    assert_eq!(doc.tile_count(), 1);
    let mut view = FoldView::new(&doc, 30).unwrap();
    let mut nav = Navigator::new();
    let mut history = NavigationHistory::new(bookmark(&doc, 0));
    for id in [1, 2] {
        history.push(bookmark(&doc, id)).unwrap();
    }
    let at_body = doc.source().find("tail").unwrap();
    let actual = nav.reconcile_source(view.heights(), 8, 4, at_body);
    let scrolled = Bookmark::new(&doc, actual.source.unwrap(), 2).unwrap();
    assert_eq!(scrolled.target(), Some(heading(2)));
    history.replace_current(scrolled).unwrap();
    let target = go(&doc, &mut view, &mut history, HistoryDirection::Back).unwrap();
    nav.apply_target(target);
    assert_eq!(target.source, Some(bookmark(&doc, 1).anchor().source()));
    assert_eq!(history.position(), 1);
    let target = go(&doc, &mut view, &mut history, HistoryDirection::Back).unwrap();
    assert_eq!(target.source, Some(0));
    assert!(go(&doc, &mut view, &mut history, HistoryDirection::Back).is_none());
    for expected in [bookmark(&doc, 1), scrolled] {
        let target = go(&doc, &mut view, &mut history, HistoryDirection::Forward).unwrap();
        assert_eq!(target.source, Some(expected.anchor().source()));
    }
    assert!(go(&doc, &mut view, &mut history, HistoryDirection::Forward).is_none());

    // A renderer maps the retained source to a new row after reflow. The saved
    // offset is relative to the viewport, not to the old tile/global height.
    view.reset(40).unwrap();
    let returned = nav.reconcile_source(
        view.heights(),
        history.current().viewport_top(17, 4),
        4,
        history.current().anchor().source(),
    );
    assert_eq!(returned.top, 15);
    assert_eq!(returned.source, Some(at_body));
    assert_eq!(history.current().viewport_top(17, 2), 16);
    assert_eq!(history.current().viewport_top(17, 0), 17);
    assert_eq!(history.current().viewport_top(1, 4), 0);
}

#[test]
fn new_jump_truncates_forward_branch_but_refresh_and_duplicates_do_not() {
    let doc = document("# A\na\n# B\nb\n# C\nc\n# D\nd\n");
    let mut view = FoldView::new(&doc, 1).unwrap();
    let mut history = NavigationHistory::new(bookmark(&doc, 0));
    history.push(bookmark(&doc, 1)).unwrap();
    history.push(bookmark(&doc, 2)).unwrap();
    go(&doc, &mut view, &mut history, HistoryDirection::Back).unwrap();
    let body = Bookmark::new(&doc, doc.source().find("b\n").unwrap(), 1).unwrap();
    history.replace_current(body).unwrap();
    let mut forward = history
        .navigation(&doc, &view, HistoryDirection::Forward, 1)
        .unwrap()
        .unwrap();
    assert!(!history.push(body).unwrap());
    history.replace_current(body).unwrap();
    assert_eq!(history.entries().len(), 3);
    finish(&mut forward, &doc, &mut view, &mut history);
    go(&doc, &mut view, &mut history, HistoryDirection::Back).unwrap();
    history.push(bookmark(&doc, 3)).unwrap();
    assert_eq!(
        history.entries(),
        &[bookmark(&doc, 0), body, bookmark(&doc, 3)]
    );
    assert!(go(&doc, &mut view, &mut history, HistoryDirection::Forward).is_none());
}

#[test]
fn history_matches_a_fixed_seed_linear_model() {
    let doc = document(&"# same\nbody\n".repeat(80));
    let mut view = FoldView::new(&doc, 2).unwrap();
    let mut history = NavigationHistory::new(bookmark(&doc, 0));
    let mut entries = vec![bookmark(&doc, 0)];
    let mut position = 0usize;
    let mut seed = 0x87c5_29d1_u64;
    for _ in 0..2000 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let value = (seed >> 32) as usize;
        match value % 5 {
            0 | 1 => {
                let direction = if value.is_multiple_of(5) {
                    HistoryDirection::Back
                } else {
                    HistoryDirection::Forward
                };
                let destination = if direction == HistoryDirection::Back {
                    position.checked_sub(1)
                } else {
                    (position + 1 < entries.len()).then_some(position + 1)
                };
                let target = go(&doc, &mut view, &mut history, direction);
                assert_eq!(target.is_some(), destination.is_some());
                if let Some(next) = destination {
                    position = next;
                    assert_eq!(
                        target.unwrap().source,
                        Some(entries[position].anchor().source())
                    );
                }
            }
            2 => {
                let next = bookmark(&doc, value % 80);
                history.replace_current(next).unwrap();
                entries[position] = next;
            }
            _ => {
                let next = bookmark(&doc, value % 80);
                let changed = next != entries[position];
                assert_eq!(history.push(next).unwrap(), changed);
                if changed {
                    entries = entries[..=position].iter().copied().chain([next]).collect();
                    position += 1;
                }
            }
        }
        assert_eq!(history.position(), position);
        assert_eq!(history.entries(), entries);
    }
}

#[test]
fn million_byte_restore_opens_only_required_path_in_ten_steps() {
    let mut text = String::new();
    for level in 1..=6 {
        text.push_str(&format!("{} same\n", "#".repeat(level)));
    }
    text.push_str("```rs\n");
    let destination = text.len() + 500_000;
    text.push_str(&"x".repeat(1_000_000));
    text.push_str("\n```\n# unrelated\nbody\n");
    let doc = document(&text);
    let mut view = FoldView::new(&doc, 1).unwrap();
    let saved = Bookmark::new(&doc, destination, 3).unwrap();
    let code = FoldId::Code(doc.block_at(destination).unwrap());
    assert_eq!(saved.target(), Some(code));
    let mut history = NavigationHistory::new(saved);
    history.push(bookmark(&doc, 6)).unwrap();
    for id in (0..7).map(heading).chain([code]) {
        view.set_collapsed(&doc, id, true).unwrap();
    }
    let before = doc.work();
    let visits = view.heights().mutation_visits();
    let mut task = history
        .navigation(&doc, &view, HistoryDirection::Back, 5)
        .unwrap()
        .unwrap();
    let (target, steps, changes) = finish(&mut task, &doc, &mut view, &mut history);
    assert_eq!(target.source, Some(destination));
    assert_eq!(steps, 10);
    assert_eq!(changes.len(), 7);
    assert_eq!(task.bookmark().viewport_offset(), 3);
    assert!(view.is_collapsed(heading(6)));
    assert!(!view.is_collapsed(code));
    assert_eq!(doc.work(), before);
    assert!(view.heights().mutation_visits() - visits < 15_000);
    assert_eq!(history.entries().len(), 2);
}

#[test]
fn budget_exhaustion_cancellation_and_competing_tasks_do_not_move_history_early() {
    let doc = document("# A\n## B\nbody\n# C\nend\n");
    let mut view = FoldView::new(&doc, 1).unwrap();
    view.set_collapsed(&doc, heading(0), true).unwrap();
    view.set_collapsed(&doc, heading(1), true).unwrap();
    let mut history = NavigationHistory::new(bookmark(&doc, 1));
    history.push(bookmark(&doc, 2)).unwrap();
    {
        let mut task = history
            .navigation(&doc, &view, HistoryDirection::Back, 1)
            .unwrap()
            .unwrap();
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
            let progress = task
                .advance(&doc, &mut view, &mut history, &mut budget)
                .unwrap();
            assert_eq!(progress.completion, Completion::BudgetExhausted);
            assert_eq!(progress.change, None);
            assert_eq!(budget.used(), WorkLimit::default());
            assert_eq!(history.position(), 1);
            assert!(view.is_collapsed(heading(0)));
        }
        for _ in 0..2 {
            task.advance(&doc, &mut view, &mut history, &mut tiny())
                .unwrap();
        }
        assert!(!view.is_collapsed(heading(0)));
        assert!(view.is_collapsed(heading(1))); // target's own body stays folded
        assert_eq!(history.position(), 1);
    } // Cancel after the path opened, before the final location/commit.
    assert_eq!(history.position(), 1);
    let mut first = history
        .navigation(&doc, &view, HistoryDirection::Back, 1)
        .unwrap()
        .unwrap();
    let mut second = history
        .navigation(&doc, &view, HistoryDirection::Back, 1)
        .unwrap()
        .unwrap();
    finish(&mut first, &doc, &mut view, &mut history);
    let mut budget = tiny();
    assert_eq!(
        second.advance(&doc, &mut view, &mut history, &mut budget),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(budget.used(), WorkLimit::default());
}

#[test]
fn identities_changes_and_stale_tasks_are_rejected_before_mutation() {
    let mut doc = Document::new();
    doc.append("# A\nbody\n# B\ntail\n").unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let mut history = NavigationHistory::new(bookmark(&doc, 0));
    history.push(bookmark(&doc, 1)).unwrap();
    let other = document(doc.source());
    let foreign = bookmark(&other, 0);
    assert_eq!(history.push(foreign), Err(LayoutError::WrongDocument));
    assert_eq!(
        history.replace_current(foreign),
        Err(LayoutError::WrongDocument)
    );
    assert!(matches!(
        foreign.navigation(&doc, &view, 1),
        Err(LayoutError::WrongDocument)
    ));
    assert!(matches!(
        history.navigation(&other, &view, HistoryDirection::Back, 1),
        Err(LayoutError::WrongDocument)
    ));
    for change in 0..6 {
        let mut task = history
            .navigation(&doc, &view, HistoryDirection::Back, 1)
            .unwrap()
            .unwrap();
        let mut budget = tiny();
        match change {
            0 => {
                let mut other_history = NavigationHistory::new(bookmark(&doc, 0));
                assert_eq!(
                    task.advance(&doc, &mut view, &mut other_history, &mut budget),
                    Err(LayoutError::WrongHistory)
                );
            }
            1 => {
                let mut other_view = FoldView::new(&doc, 1).unwrap();
                assert_eq!(
                    task.advance(&doc, &mut other_view, &mut history, &mut budget),
                    Err(LayoutError::WrongView)
                );
            }
            _ => {
                match change {
                    2 => history
                        .replace_current(Bookmark::new(&doc, doc.source().len(), 0).unwrap())
                        .unwrap(),
                    3 => {
                        history.push(bookmark(&doc, 1)).unwrap();
                    }
                    4 => view.reset(3).unwrap(),
                    5 => {
                        let update = doc.append("more\n").unwrap();
                        view.apply(&doc, &update).unwrap();
                    }
                    _ => unreachable!(),
                }
                assert_eq!(
                    task.advance(&doc, &mut view, &mut history, &mut budget),
                    Err(LayoutError::StaleContinuation)
                );
            }
        }
        assert_eq!(budget.used(), WorkLimit::default());
    }
    assert_eq!(history.position(), 2);
    let update = doc.append("pending").unwrap();
    assert!(matches!(
        history.navigation(&doc, &view, HistoryDirection::Back, 1),
        Err(LayoutError::OutOfOrderUpdate)
    ));
    view.apply(&doc, &update).unwrap();
}

#[test]
fn bookmarks_survive_growth_setext_finish_and_new_views_but_not_reload() {
    let mut doc = Document::new();
    let initial = Bookmark::new(&doc, 0, 0).unwrap();
    assert_eq!(initial.target(), None);
    doc.append("# A\n中文\n").unwrap();
    let source = doc.source().find('中').unwrap();
    let saved = Bookmark::new(&doc, source, 4).unwrap();
    let eof = Bookmark::new(&doc, doc.source().len(), 0).unwrap();
    assert_eq!(saved.target(), Some(heading(0)));
    for invalid in [source + 1, doc.source().len() + 1, usize::MAX] {
        assert_eq!(
            Bookmark::new(&doc, invalid, 0),
            Err(LayoutError::InvalidRange)
        );
    }
    let mut view = FoldView::new(&doc, 1).unwrap();
    let mut stale = saved.navigation(&doc, &view, 1).unwrap();
    let update = doc.append("===\nmore\n").unwrap();
    view.apply(&doc, &update).unwrap();
    assert_eq!(
        stale.advance(&doc, &mut view, &mut tiny()),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(doc.fold_candidate_at(source).unwrap().id, heading(1));
    assert_eq!(saved.target(), Some(heading(0))); // metadata does not redirect source
    let update = doc.finish();
    view.apply(&doc, &update).unwrap();
    for mut view in [view, FoldView::new(&doc, 10).unwrap()] {
        view.set_collapsed(&doc, heading(0), true).unwrap();
        view.reset(7).unwrap();
        for saved in [initial, saved, eof] {
            let mut task = saved.navigation(&doc, &view, 3).unwrap();
            let progress = task
                .advance(&doc, &mut view, &mut Budget::new(WorkLimit::unlimited()))
                .unwrap();
            assert_eq!(
                progress.target.unwrap().source,
                Some(saved.anchor().source())
            );
            assert_eq!(progress.completion, Completion::Covered);
        }
    }
    let reloaded = document(doc.source());
    let view = FoldView::new(&reloaded, 1).unwrap();
    assert!(matches!(
        saved.navigation(&reloaded, &view, 1),
        Err(LayoutError::WrongDocument)
    ));
}

#[test]
fn history_uses_latest_heights_when_finishing_and_bookmarks_restore_empty_eof() {
    let doc = document(&format!("# A\n{}\n# B\ntail\n", "a".repeat(2000)));
    let mut view = FoldView::new(&doc, 1).unwrap();
    let destination = bookmark(&doc, 1);
    let mut history = NavigationHistory::new(destination);
    history.push(bookmark(&doc, 0)).unwrap();
    let mut task = history
        .navigation(&doc, &view, HistoryDirection::Back, 1)
        .unwrap()
        .unwrap();
    for _ in 0..2 {
        task.advance(&doc, &mut view, &mut history, &mut tiny())
            .unwrap();
    }
    let ticket = view.heights().ticket(TileId(0)).unwrap();
    view.measure(ticket, 100).unwrap();
    let (target, steps, _) = finish(&mut task, &doc, &mut view, &mut history);
    assert_eq!(steps, 1);
    let tile = doc.tile_at(destination.anchor().source()).unwrap();
    assert_eq!(target.top, view.heights().prefix_rows(tile.0).unwrap());
    assert!(target.top >= 100);

    let empty = Document::new();
    let saved = Bookmark::new(&empty, 0, u64::MAX).unwrap();
    let mut view = FoldView::new(&empty, 1).unwrap();
    let mut task = saved.navigation(&empty, &view, 0).unwrap();
    let result = task
        .advance(&empty, &mut view, &mut Budget::new(WorkLimit::unlimited()))
        .unwrap();
    assert_eq!(
        result.target,
        Some(ScrollTarget {
            top: 0,
            anchor: None,
            source: Some(0)
        })
    );
}
