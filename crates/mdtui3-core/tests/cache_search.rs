use mdtui3::*;

#[test]
fn symbol_keys_ignore_unrelated_definitions_and_count_checks() {
    let mut doc = Document::new();
    doc.append(&format!("[used] {}\n\n", "x".repeat(2000)))
        .unwrap();
    let mut budget = Budget::new(WorkLimit::unlimited());
    let key = DependencyKey::capture(&doc, TileId(0), &mut budget)
        .unwrap()
        .unwrap();
    doc.append("[unrelated]: /other\n").unwrap();
    assert_eq!(
        key.validate(&doc, &mut budget).unwrap(),
        CacheValidity::Valid
    );
    let mut empty = Budget::new(WorkLimit::default());
    assert_eq!(
        key.validate(&doc, &mut empty).unwrap(),
        CacheValidity::BudgetExhausted
    );
    doc.append("[used]: /url\n").unwrap();
    assert_eq!(
        key.validate(&doc, &mut budget).unwrap(),
        CacheValidity::Changed
    );
}

#[test]
fn replacing_input_checkpoint_rejects_dependent_measurement() {
    let mut doc = Document::new();
    doc.append(&"x".repeat(4000)).unwrap();
    let mut heights = HeightIndex::default();
    heights.rebuild(&doc).unwrap();
    let mut checkpoints = CheckpointIndex::new();
    checkpoints.rebuild(&doc);
    checkpoints.insert(TileId(4), 3).unwrap();
    let ticket = heights
        .ticket_from(TileId(7), &checkpoints, TileId(4))
        .unwrap();
    checkpoints.insert(TileId(4), 9).unwrap();
    assert_eq!(
        heights.measure_from(ticket, 2, &checkpoints),
        Err(LayoutError::StaleMeasurement)
    );
    let ticket = heights
        .ticket_from(TileId(7), &checkpoints, TileId(4))
        .unwrap();
    heights.measure_from(ticket, 2, &checkpoints).unwrap();
}

#[test]
fn eviction_moves_payloads_without_running_destructors() {
    use std::{cell::Cell, rc::Rc};
    struct State(Rc<Cell<usize>>);
    impl Drop for State {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }
    let drops = Rc::new(Cell::new(0));
    let mut checkpoints = CheckpointIndex::new();
    for i in 0..100 {
        checkpoints.insert(TileId(i), State(drops.clone())).unwrap();
    }
    checkpoints.clear();
    let mut cursor = 0;
    let mut out = Vec::new();
    loop {
        let mut budget = Budget::new(WorkLimit {
            objects: 7,
            queries: 7,
            ..Default::default()
        });
        let completion = checkpoints.drain_before(TileId(50), &mut cursor, &mut budget, &mut out);
        assert_eq!(drops.get(), 0);
        assert!(budget.used().objects <= 7);
        if completion == Completion::Covered {
            break;
        }
    }
    assert_eq!(out.len(), 50);
    drop(out);
    assert_eq!(drops.get(), 50);
    drop(checkpoints);
    assert_eq!(drops.get(), 100);
}

#[test]
fn search_is_resumable_overlapping_and_streaming() {
    let mut doc = Document::new();
    let mut search = SourceSearch::new(&doc, "aba").unwrap();
    let mut found = Vec::new();
    for text in ["a", "bab", "a中aba"] {
        doc.append(text).unwrap();
        loop {
            let mut budget = Budget::new(WorkLimit {
                bytes: 1,
                objects: 1,
                ..Default::default()
            });
            if search.advance(&doc, &mut budget, &mut found).unwrap() == Completion::EndOfDocument {
                break;
            }
        }
    }
    assert_eq!(found, [Span::new(0, 3), Span::new(2, 5), Span::new(8, 11)]);
    let selection = Selection::new(&doc, found[2]).unwrap();
    assert_eq!(selection.text(&doc).unwrap(), "aba");
}

#[test]
fn task_overrides_are_view_local() {
    let mut doc = Document::new();
    doc.append("- [ ] task\n").unwrap();
    let mut a = TaskState::new(&doc);
    let b = TaskState::new(&doc);
    assert!(a.toggle(&doc, BlockId(0)).unwrap());
    assert!(!b.checked(&doc, BlockId(0)).unwrap());
    assert_eq!(doc.source(), "- [ ] task\n");
}

#[test]
fn source_snapshot_respects_copy_limit_and_shared_budget() {
    let mut doc = Document::new();
    doc.append("中文🙂 text").unwrap();
    let mut budget = Budget::new(WorkLimit {
        queries: 2,
        bytes: 10,
        objects: 2,
        ..Default::default()
    });
    assert!(
        doc.snapshot(Span::new(0, 10), 9, &mut budget)
            .unwrap()
            .is_none()
    );
    assert_eq!(budget.used(), WorkLimit::default());
    let snapshot = doc
        .snapshot(Span::new(0, 10), 10, &mut budget)
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.text, "中文🙂");
    assert!(
        doc.snapshot(Span::new(0, 10), 10, &mut budget)
            .unwrap()
            .is_none()
    );
    assert_eq!(snapshot.document, doc.id());
}
