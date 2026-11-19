//! Independent, literal byte/identity oracles for the host-only reload example.
#[path = "../examples/support/bookmark_reload.rs"]
mod host;

use host::{Binding, Error, MapStep, PointMap, Restore, Saved, Version, host_step};
use mdstream::*;

const OLD: &str = include_str!("fixtures/bookmark_reload/old.md");
const INSERTED: &str = include_str!("fixtures/bookmark_reload/inserted.md");
const DELETED: &str = include_str!("fixtures/bookmark_reload/deleted.md");
const CHANGED: &str = include_str!("fixtures/bookmark_reload/changed.md");
const V1: Version = Version {
    resource: 7,
    content: 1,
};
const V2: Version = Version {
    resource: 7,
    content: 2,
};

fn document(text: &str) -> Document {
    let mut document = Document::new();
    document.append(text).unwrap();
    document.finish();
    document
}

fn saved(document: &Document) -> Saved {
    Saved::capture(
        document,
        Binding::new(document, V1),
        Bookmark::new(document, 20, 9).unwrap(),
    )
    .unwrap()
}

fn map(saved: Saved, to: Version, steps: &[MapStep]) -> PointMap<'_> {
    PointMap {
        from: saved.version,
        source: saved.source,
        to,
        steps,
    }
}

fn zero() -> Budget {
    Budget::new(WorkLimit::default())
}

fn tiny() -> Budget {
    Budget::new(host_step())
}

fn finish(
    task: &mut Restore<'_>,
    document: &Document,
    view: &mut FoldView,
) -> (Bookmark, ScrollTarget) {
    for _ in 0..16 {
        if let Some(result) = task.advance(document, view, &mut tiny()).unwrap() {
            return result;
        }
    }
    panic!("point restore exceeded its independently bounded number of steps");
}

#[test]
fn handwritten_oracle_insertion_deletion_identical_and_same_source_changed_content() {
    let old = document(OLD);
    assert_eq!(OLD.len(), 39);
    assert_eq!(&OLD[20..26], "中文");
    let old_bookmark = Bookmark::new(&old, 20, 9).unwrap();
    for line in include_str!("fixtures/bookmark_reload/oracle.tsv")
        .lines()
        .filter(|s| !s.starts_with('#'))
    {
        let columns: Vec<_> = line.split('\t').collect();
        let text = match columns[0] {
            "inserted" => INSERTED,
            "deleted" => DELETED,
            "changed" => CHANGED,
            "identical" => OLD,
            _ => unreachable!(),
        };
        let old_source: usize = columns[1].parse().unwrap();
        let new_source: usize = columns[2].parse().unwrap();
        let heading: usize = columns[3].parse().unwrap();
        let new = document(text);
        assert_ne!(new.id(), old.id());
        let mut view = FoldView::new(&new, 1).unwrap();
        assert!(matches!(
            old_bookmark.navigation(&new, &view, 3),
            Err(LayoutError::WrongDocument)
        ));
        let persisted = saved(&old);
        assert_eq!(persisted.source, old_source);
        let steps = [MapStep::Exact(new_source)];
        let mut task = Restore::new(
            persisted,
            &new,
            Binding::new(&new, V2),
            map(persisted, V2, &steps),
            3,
        )
        .unwrap();
        assert_eq!(task.map_position(), 0);
        let (fresh, target) = finish(&mut task, &new, &mut view);
        assert_eq!(fresh.anchor().document_id(), new.id());
        assert_eq!(fresh.anchor().source(), new_source);
        assert_eq!(fresh.target(), Some(FoldId::Heading(HeadingId(heading))));
        assert_eq!(target.source, Some(new_source));
        assert!(text[new_source..].starts_with(columns[4]));
        assert_eq!(fresh.viewport_offset(), 9);
        // Core histories remain document-local even after successful host mapping.
        let mut history = NavigationHistory::new(old_bookmark);
        assert_eq!(history.push(fresh), Err(LayoutError::WrongDocument));
    }
}

#[test]
fn shared_zero_tiny_budgets_reveal_only_target_and_commit_host_slot_after_location() {
    let old = document(OLD);
    let new = document(INSERTED);
    let persisted = saved(&old);
    let steps = [MapStep::Working, MapStep::Exact(36)];
    let mut task = Restore::new(
        persisted,
        &new,
        Binding::new(&new, V2),
        map(persisted, V2, &steps),
        3,
    )
    .unwrap();
    let mut view = FoldView::new(&new, 1).unwrap();
    for h in 0..4 {
        view.set_collapsed(&new, FoldId::Heading(HeadingId(h)), true)
            .unwrap();
    }
    let mut active_slot = 1;
    for frame in 1..=7 {
        let cursor = task.map_position();
        let view_version = view.version();
        let mut no_work = zero();
        assert_eq!(task.advance(&new, &mut view, &mut no_work).unwrap(), None);
        assert_eq!(no_work.used(), WorkLimit::default());
        assert_eq!(task.map_position(), cursor);
        assert_eq!(view.version(), view_version);
        assert_eq!(active_slot, 1);
        let mut budget = tiny();
        let result = task.advance(&new, &mut view, &mut budget).unwrap();
        assert_eq!(
            budget.used(),
            if frame <= 3 {
                host_step()
            } else {
                WorkLimit {
                    queries: 1,
                    objects: 1,
                    ..Default::default()
                }
            }
        );
        assert_eq!(task.map_position(), frame.min(2));
        if frame < 7 {
            assert_eq!(result, None);
        } else {
            let (fresh, target) = result.unwrap();
            assert_eq!(fresh.anchor().source(), 36);
            assert_eq!(target.source, Some(36));
            active_slot = 0; // One O(1) host commit AFTER the task returned a target.
            let mut budget = zero();
            assert_eq!(task.advance(&new, &mut view, &mut budget).unwrap(), result);
            assert_eq!(budget.used(), WorkLimit::default());
        }
        assert_eq!(view.is_collapsed(FoldId::Heading(HeadingId(2))), frame < 5);
        for h in [0, 1, 3] {
            assert!(view.is_collapsed(FoldId::Heading(HeadingId(h))));
        }
    }
    assert_eq!(active_slot, 0);
}

#[test]
fn every_host_budget_dimension_is_atomic_and_navigation_needs_no_recovery() {
    let old = document(OLD);
    let new = document(INSERTED);
    let persisted = saved(&old);
    let steps = [MapStep::Exact(36)];
    let mut task = Restore::new(
        persisted,
        &new,
        Binding::new(&new, V2),
        map(persisted, V2, &steps),
        3,
    )
    .unwrap();
    let mut view = FoldView::new(&new, 1).unwrap();
    for missing in 0..3 {
        let mut limit = host_step();
        match missing {
            0 => limit.queries = 0,
            1 => limit.objects = 0,
            _ => limit.recovery = 0,
        }
        let mut budget = Budget::new(limit);
        assert_eq!(task.advance(&new, &mut view, &mut budget).unwrap(), None);
        assert_eq!(task.map_position(), 0);
        assert_eq!(budget.used(), WorkLimit::default());
    }
    let mut budget = Budget::new(WorkLimit {
        queries: 2,
        objects: 2,
        recovery: 2,
        ..Default::default()
    });
    assert_eq!(task.advance(&new, &mut view, &mut budget).unwrap(), None);
    assert_eq!(
        budget.used(),
        WorkLimit {
            queries: 2,
            objects: 2,
            recovery: 2,
            ..Default::default()
        }
    );
    for limit in [
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
        assert_eq!(task.advance(&new, &mut view, &mut budget).unwrap(), None);
        assert_eq!(budget.used(), WorkLimit::default());
    }
    for frame in 1..=3 {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        let result = task.advance(&new, &mut view, &mut budget).unwrap();
        assert_eq!(result.is_some(), frame == 3);
        assert_eq!(
            budget.used(),
            WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            }
        );
    }
}

#[test]
fn unmappable_and_unfinished_maps_never_publish_or_change_cursor() {
    let old = document(OLD);
    let new = document(CHANGED);
    let persisted = saved(&old);
    let mut view = FoldView::new(&new, 1).unwrap();
    let version = view.version();
    for (answer, expected) in [
        (MapStep::Missing, Error::Missing),
        (MapStep::Ambiguous, Error::Ambiguous),
        (MapStep::Deleted, Error::Deleted),
        (MapStep::Changed, Error::Changed),
    ] {
        let steps = [answer];
        let mut task = Restore::new(
            persisted,
            &new,
            Binding::new(&new, V2),
            map(persisted, V2, &steps),
            3,
        )
        .unwrap();
        let mut budget = zero();
        assert_eq!(task.advance(&new, &mut view, &mut budget).unwrap(), None);
        assert_eq!(task.advance(&new, &mut view, &mut tiny()), Err(expected));
        let mut budget = tiny();
        assert_eq!(task.advance(&new, &mut view, &mut budget), Err(expected));
        assert_eq!(budget.used(), WorkLimit::default());
        assert_eq!(view.version(), version);
    }
    // Unresolved evidence is not Missing; even unlimited budget cannot invent it.
    for steps in [&[][..], &[MapStep::Working][..]] {
        let mut task = Restore::new(
            persisted,
            &new,
            Binding::new(&new, V2),
            map(persisted, V2, steps),
            3,
        )
        .unwrap();
        assert_eq!(
            task.advance(&new, &mut view, &mut Budget::new(WorkLimit::unlimited()))
                .unwrap(),
            None
        );
        assert_eq!(task.map_position(), steps.len());
        let mut budget = tiny();
        assert_eq!(task.advance(&new, &mut view, &mut budget).unwrap(), None);
        assert_eq!(budget.used(), WorkLimit::default());
        assert_eq!(view.version(), version);
    }
    // Host later completes THIS point's report; restart it, no history migration.
    let steps = [MapStep::Exact(20)];
    let mut task = Restore::new(
        persisted,
        &new,
        Binding::new(&new, V2),
        map(persisted, V2, &steps),
        3,
    )
    .unwrap();
    assert_eq!(finish(&mut task, &new, &mut view).1.source, Some(20));
}

#[test]
fn payload_capture_and_point_map_owners_are_checked_without_text_matching() {
    let old = document(OLD);
    let new = document(CHANGED);
    let bookmark = Bookmark::new(&old, 20, 9).unwrap();
    let binding = Binding::new(&old, V1);
    assert_eq!(
        Saved::capture(&new, binding, bookmark),
        Err(Error::Core(LayoutError::WrongDocument))
    );
    assert_eq!(
        Saved::capture(&new, Binding::new(&new, V2), bookmark),
        Err(Error::Core(LayoutError::WrongDocument))
    );
    let persisted = saved(&old);
    let steps = [MapStep::Exact(20)];
    let wrong_resource = Version {
        resource: 8,
        content: 2,
    };
    assert!(matches!(
        Restore::new(
            persisted,
            &new,
            Binding::new(&new, wrong_resource),
            map(persisted, wrong_resource, &steps),
            3
        ),
        Err(Error::WrongResource)
    ));
    for report in [
        PointMap {
            from: V2,
            source: 20,
            to: V2,
            steps: &steps,
        },
        PointMap {
            from: V1,
            source: 21,
            to: V2,
            steps: &steps,
        },
        PointMap {
            from: V1,
            source: 20,
            to: V1,
            steps: &steps,
        },
    ] {
        assert!(matches!(
            Restore::new(persisted, &new, Binding::new(&new, V2), report, 3),
            Err(Error::WrongMapping)
        ));
    }
    assert!(matches!(
        Restore::new(persisted, &new, binding, map(persisted, V2, &steps), 3),
        Err(Error::Core(LayoutError::WrongDocument))
    ));
    // Same resource and same byte number do not imply unchanged content.
    let alpha = Saved::capture(&old, binding, Bookmark::new(&old, 7, 0).unwrap()).unwrap();
    assert_eq!(&OLD[7..12], "alpha");
    assert_eq!(&CHANGED[7..12], "other");
    let steps = [MapStep::Changed];
    let mut task = Restore::new(
        alpha,
        &new,
        Binding::new(&new, V2),
        map(alpha, V2, &steps),
        3,
    )
    .unwrap();
    let mut view = FoldView::new(&new, 1).unwrap();
    assert_eq!(
        task.advance(&new, &mut view, &mut tiny()),
        Err(Error::Changed)
    );
}

#[test]
fn malformed_mapped_boundaries_are_core_errors_without_fold_or_history_publication() {
    let old = document(OLD);
    let new = document(INSERTED);
    let persisted = saved(&old);
    for invalid in [37, new.source().len() + 1, usize::MAX] {
        let steps = [MapStep::Exact(invalid)];
        let mut task = Restore::new(
            persisted,
            &new,
            Binding::new(&new, V2),
            map(persisted, V2, &steps),
            3,
        )
        .unwrap();
        let mut view = FoldView::new(&new, 1).unwrap();
        let version = view.version();
        assert_eq!(task.advance(&new, &mut view, &mut tiny()).unwrap(), None);
        assert_eq!(
            task.advance(&new, &mut view, &mut tiny()),
            Err(Error::Core(LayoutError::InvalidRange))
        );
        assert_eq!(view.version(), version);
    }
}

#[test]
fn wrong_document_and_view_during_navigation_cost_nothing_and_cancellation_keeps_host_cursor() {
    let old = document(OLD);
    let new = document(INSERTED);
    let other = document(INSERTED);
    let persisted = saved(&old);
    let steps = [MapStep::Exact(36)];
    let mut view = FoldView::new(&new, 1).unwrap();
    let mut other_view = FoldView::new(&new, 1).unwrap();
    let mut foreign_view = FoldView::new(&other, 1).unwrap();
    view.set_collapsed(&new, FoldId::Heading(HeadingId(2)), true)
        .unwrap();
    let active_slot = 1;
    {
        let mut task = Restore::new(
            persisted,
            &new,
            Binding::new(&new, V2),
            map(persisted, V2, &steps),
            3,
        )
        .unwrap();
        let mut budget = tiny();
        assert_eq!(
            task.advance(&other, &mut foreign_view, &mut budget),
            Err(Error::Core(LayoutError::WrongDocument))
        );
        assert_eq!(budget.used(), WorkLimit::default());
        assert_eq!(task.map_position(), 0);
        // map, publish, start, reveal; final location is still pending.
        for _ in 0..4 {
            assert_eq!(task.advance(&new, &mut view, &mut tiny()).unwrap(), None);
        }
        let mut budget = tiny();
        assert_eq!(
            task.advance(&new, &mut other_view, &mut budget),
            Err(Error::Core(LayoutError::WrongView))
        );
        assert_eq!(budget.used(), WorkLimit::default());
        assert_eq!(active_slot, 1);
    }
    assert!(!view.is_collapsed(FoldId::Heading(HeadingId(2))));
    assert_eq!(active_slot, 1); // Drop cancels selection; opened folds stay open.
    let mut task = Restore::new(
        persisted,
        &new,
        Binding::new(&new, V2),
        map(persisted, V2, &steps),
        3,
    )
    .unwrap();
    assert_eq!(finish(&mut task, &new, &mut view).1.source, Some(36));
    let mut task = Restore::new(
        persisted,
        &new,
        Binding::new(&new, V2),
        map(persisted, V2, &steps),
        3,
    )
    .unwrap();
    let mut budget = tiny();
    assert_eq!(
        task.advance(&new, &mut foreign_view, &mut budget),
        Err(Error::Core(LayoutError::WrongDocument))
    );
    assert_eq!(budget.used(), WorkLimit::default());
    assert_eq!(task.map_position(), 0);
    assert_eq!(task.advance(&new, &mut view, &mut tiny()).unwrap(), None);
    assert_eq!(task.map_position(), 1);
    assert_eq!(finish(&mut task, &new, &mut view).1.source, Some(36));

    // A newly bound document with an unapplied update also fails preflight.
    let mut live = Document::new();
    live.append(INSERTED).unwrap();
    let mut delayed = FoldView::new(&live, 1).unwrap();
    let update = live.append("tail\n").unwrap();
    let v3 = Version {
        resource: 7,
        content: 3,
    };
    let mut task = Restore::new(
        persisted,
        &live,
        Binding::new(&live, v3),
        map(persisted, v3, &steps),
        3,
    )
    .unwrap();
    let mut budget = tiny();
    assert_eq!(
        task.advance(&live, &mut delayed, &mut budget),
        Err(Error::Core(LayoutError::OutOfOrderUpdate))
    );
    assert_eq!(budget.used(), WorkLimit::default());
    assert_eq!(task.map_position(), 0);
    delayed.apply(&live, &update).unwrap();
    assert_eq!(finish(&mut task, &live, &mut delayed).1.source, Some(36));
}

#[test]
fn append_finish_resize_restart_the_point_job_and_offsets_remain_separate_from_layout() {
    let old = document(OLD);
    let persisted = saved(&old);
    let steps = [MapStep::Exact(36)];
    let mut new = Document::new();
    new.append(INSERTED).unwrap();
    let binding = Binding::new(&new, V2);
    let mut view = FoldView::new(&new, 1).unwrap();
    let mut task = Restore::new(persisted, &new, binding, map(persisted, V2, &steps), 3).unwrap();
    assert_eq!(task.advance(&new, &mut view, &mut tiny()).unwrap(), None);
    let update = new.append("added\n").unwrap();
    view.apply(&new, &update).unwrap();
    let mut budget = tiny();
    assert_eq!(
        task.advance(&new, &mut view, &mut budget),
        Err(Error::Core(LayoutError::StaleContinuation))
    );
    assert_eq!(budget.used(), WorkLimit::default());
    assert_eq!(
        Saved::capture(&new, binding, Bookmark::new(&new, 36, 9).unwrap()),
        Err(Error::Core(LayoutError::StaleContinuation))
    );
    let v3 = Version {
        resource: 7,
        content: 3,
    };
    let binding = Binding::new(&new, v3);
    let mut task = Restore::new(persisted, &new, binding, map(persisted, v3, &steps), 3).unwrap();
    let update = new.finish();
    view.apply(&new, &update).unwrap();
    let mut budget = tiny();
    assert_eq!(
        task.advance(&new, &mut view, &mut budget),
        Err(Error::Core(LayoutError::StaleContinuation))
    );
    assert_eq!(budget.used(), WorkLimit::default());
    // Same bytes/version after finish, but a new transient revision binding.
    let binding = Binding::new(&new, v3);
    let mut task = Restore::new(persisted, &new, binding, map(persisted, v3, &steps), 3).unwrap();
    for _ in 0..2 {
        assert_eq!(task.advance(&new, &mut view, &mut tiny()).unwrap(), None);
    }
    view.reset(7).unwrap();
    let mut budget = tiny();
    assert_eq!(
        task.advance(&new, &mut view, &mut budget),
        Err(Error::Core(LayoutError::StaleContinuation))
    );
    assert_eq!(budget.used(), WorkLimit::default());
    let mut task = Restore::new(persisted, &new, binding, map(persisted, v3, &steps), 2).unwrap();
    let (bookmark, target) = finish(&mut task, &new, &mut view);
    assert_eq!(target.source, Some(36));
    assert_eq!(bookmark.viewport_offset(), 9);
    // Hand-supplied local row, not the estimated row from FoldNavigation.
    assert_eq!(bookmark.viewport_top(12, 2), 11);
    assert_eq!(bookmark.viewport_top(12, 3), 10);
    assert_eq!(bookmark.viewport_top(1, 30), 0);
    assert_eq!(bookmark.viewport_top(12, 0), 12);
    let mut navigator = Navigator::new();
    let top = bookmark.viewport_top(12, 2);
    let refined = navigator.reconcile_source(view.heights(), top, 2, 36);
    assert_eq!(refined.source, Some(36));
    assert_eq!(navigator.reconcile(view.heights(), top, 2).source, Some(36));
}

#[test]
fn empty_and_eof_are_explicit_points_and_append_preserves_left_affinity() {
    for (old_text, old_source, new_text, new_source) in [("", 0, "", 0), (OLD, 39, INSERTED, 55)] {
        let old = document(old_text);
        let mut new = document(new_text);
        let persisted = Saved::capture(
            &old,
            Binding::new(&old, V1),
            Bookmark::new(&old, old_source, u64::MAX).unwrap(),
        )
        .unwrap();
        let steps = [MapStep::Exact(new_source)];
        let mut view = FoldView::new(&new, 1).unwrap();
        let mut task = Restore::new(
            persisted,
            &new,
            Binding::new(&new, V2),
            map(persisted, V2, &steps),
            0,
        )
        .unwrap();
        let (bookmark, target) = finish(&mut task, &new, &mut view);
        assert_eq!(target.source, Some(new_source));
        assert_eq!(bookmark.viewport_top(12, 3), 10);
        // Independent live document shows mapped old EOF stays a byte point.
        new = Document::new();
        new.append(new_text).unwrap();
        let bookmark = Bookmark::new(&new, new_source, 0).unwrap();
        let mut view = FoldView::new(&new, 1).unwrap();
        let update = new.append("tail\n").unwrap();
        view.apply(&new, &update).unwrap();
        let mut navigation = bookmark.navigation(&new, &view, 3).unwrap();
        let target = navigation
            .advance(&new, &mut view, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap()
            .target
            .unwrap();
        assert_eq!(target.source, Some(new_source));
    }
}
