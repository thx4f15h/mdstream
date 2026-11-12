use mdstream::*;

#[test]
fn one_owner_supports_local_measure_expiration_and_moving_checkpoint_compaction() {
    let mut document = Document::new();
    document
        .append(&format!(
            "# A\n{}\n## B\n{}\n# C\ntail\n",
            "a".repeat(1000),
            "b".repeat(1000)
        ))
        .unwrap();
    let mut view = FoldView::<usize>::with_state(&document, 3).unwrap();
    let parent = FoldId::Heading(HeadingId(0));
    let child = FoldId::Heading(HeadingId(1));
    let ticket = view.heights().ticket(TileId(0)).unwrap();
    view.set_collapsed(&document, child, true).unwrap();
    view.set_collapsed(&document, parent, true).unwrap();
    assert_eq!(view.measure(ticket, 99), Err(LayoutError::StaleMeasurement));
    let total = view.heights().total_rows();
    {
        let (heights, checkpoints) = view.layout_parts_mut(&document).unwrap();
        let ticket = heights.ticket(TileId(0)).unwrap();
        heights.measure_local(ticket, 4).unwrap();
        heights.expire(1..heights.len()).unwrap();
        checkpoints.insert(TileId(0), 10).unwrap();
        checkpoints.insert(TileId(1), 20).unwrap();
        checkpoints.invalidate(1..2).unwrap();
    }
    assert_eq!(
        view.heights().height_state(TileId(0)),
        Some(HeightState::Local)
    );
    assert!(view.heights().total_rows() >= total);
    let token = view.checkpoints().token(TileId(0)).unwrap();
    let mut task = view.into_checkpoint_compaction();
    let mut retired = Vec::new();
    assert_eq!(
        task.advance(
            &document,
            &mut Budget::new(WorkLimit::default()),
            &mut retired
        )
        .unwrap(),
        Completion::BudgetExhausted
    );
    task = match task.finish(&document) {
        Err(task) => task,
        Ok(_) => panic!("early finish"),
    };
    loop {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        if task.advance(&document, &mut budget, &mut retired).unwrap() == Completion::Covered {
            break;
        }
    }
    let mut view = match task.finish(&document) {
        Ok(view) => view,
        Err(_) => panic!("complete maintenance"),
    };
    assert_eq!(retired, [20]);
    assert!(view.checkpoints().validates(token));
    assert!(view.is_collapsed(parent) && view.is_collapsed(child));
    view.reset(2).unwrap();
    let update = document.append("more\n").unwrap();
    view.apply(&document, &update).unwrap();
    view.set_collapsed(&document, parent, false).unwrap();
    assert!(view.is_collapsed(child));
}

fn register<S>(document: &Document, view: &mut FoldView<S>, span: Span) -> FoldId {
    let mut task = SourceFoldRegistration::overlapping(document, view, span).unwrap();
    for _ in 0..1000 {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        if let Some(id) = task.advance(document, view, &mut budget).unwrap().id {
            return id;
        }
    }
    panic!("registration stalled")
}

#[test]
fn borrowed_layout_preserves_independent_base_heights_parent_child_masks_and_reset() {
    let mut document = Document::new();
    document.append(&"a".repeat(6000)).unwrap();
    document.finish();
    let mut view = FoldView::<usize>::with_state(&document, 2).unwrap();
    let parent_span = Span::new(100, 5300);
    let child_span = Span::new(800, 3000);
    let parent = register(&document, &mut view, parent_span);
    let child = register(&document, &mut view, child_span);
    let mut base: Vec<_> = (0..document.tile_count())
        .map(|i| 1 + i as u64 % 5)
        .collect();
    {
        let (heights, checkpoints) = view.layout_parts_mut(&document).unwrap();
        for (i, rows) in base.iter().enumerate() {
            heights
                .measure_local(heights.ticket(TileId(i)).unwrap(), *rows)
                .unwrap();
        }
        checkpoints
            .insert_for_layout(heights, TileId(2), 99)
            .unwrap();
        let stale = heights
            .ticket_from(TileId(3), checkpoints, TileId(2))
            .unwrap();
        checkpoints
            .insert_for_layout(heights, TileId(2), 100)
            .unwrap();
        assert_eq!(
            heights.measure_from(stale, 9, checkpoints),
            Err(LayoutError::StaleMeasurement)
        );
    }
    for (parent_on, child_on) in [
        (true, false),
        (true, true),
        (false, true),
        (false, false),
        (true, true),
    ] {
        view.set_collapsed(&document, parent, parent_on).unwrap();
        view.set_collapsed(&document, child, child_on).unwrap();
        {
            let (heights, checkpoints) = view.layout_parts_mut(&document).unwrap();
            heights.expire(0..heights.len()).unwrap();
            checkpoints.invalidate(0..document.tile_count()).unwrap();
        }
        let mut total = 0u128;
        for (i, rows) in base.iter().enumerate() {
            // Independent whole-tile interior model; owner/end boundary tiles
            // keep stored base estimates until real local shaping measures them.
            let masked = [(parent_span, parent_on), (child_span, child_on)]
                .iter()
                .any(|(span, on)| *on && span.start / TILE_BYTES < i && i < span.end / TILE_BYTES);
            let expected = if masked { 0 } else { *rows };
            assert_eq!(view.heights().base_height(TileId(i)), Some(*rows));
            assert_eq!(view.heights().height(TileId(i)), Some(expected));
            total += u128::from(expected);
        }
        assert_eq!(view.heights().total_rows(), total);
    }
    view.reset(3).unwrap();
    base.fill(3);
    for (i, rows) in base.iter().enumerate() {
        let masked = [parent_span, child_span]
            .iter()
            .any(|span| span.start / TILE_BYTES < i && i < span.end / TILE_BYTES);
        assert_eq!(view.heights().base_height(TileId(i)), Some(*rows));
        assert_eq!(
            view.heights().height(TileId(i)),
            Some(if masked { 0 } else { *rows })
        );
    }
    assert!(view.is_collapsed(parent) && view.is_collapsed(child));
}

#[test]
fn ordinary_unrelated_append_ticket_survives_and_layout_borrow_rejects_wrong_binding() {
    let mut document = Document::new();
    document
        .append(&format!("first\n\n{}\n", "a".repeat(2000)))
        .unwrap();
    let mut view = FoldView::<usize>::with_state(&document, 1).unwrap();
    let ticket = view.heights().ticket(TileId(0)).unwrap();
    let update = document.append("more\n").unwrap();
    assert!(matches!(
        view.layout_parts_mut(&document),
        Err(LayoutError::OutOfOrderUpdate)
    ));
    view.apply(&document, &update).unwrap();
    view.measure(ticket, 7).unwrap();
    assert_eq!(view.heights().height(TileId(0)), Some(7));
    let mut foreign = Document::new();
    foreign.append("foreign\n").unwrap();
    assert!(matches!(
        view.layout_parts_mut(&foreign),
        Err(LayoutError::WrongDocument)
    ));
    let mut task = view.into_checkpoint_compaction();
    let before = task.stats();
    let mut budget = Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        ..Default::default()
    });
    assert_eq!(
        task.advance(&foreign, &mut budget, &mut Vec::new()),
        Err(LayoutError::WrongDocument)
    );
    assert_eq!(budget.used(), WorkLimit::default());
    assert_eq!(task.stats(), before);
    task = match task.finish(&foreign) {
        Err(task) => task,
        Ok(_) => panic!("foreign restore"),
    };
    let update = document
        .append("invalidates maintenance binding\n")
        .unwrap();
    assert!(!update.appended.is_empty());
    assert_eq!(
        task.advance(&document, &mut budget, &mut Vec::new()),
        Err(LayoutError::OutOfOrderUpdate)
    );
    assert_eq!(budget.used(), WorkLimit::default());
    assert!(task.finish(&document).is_err());
}

#[test]
fn compaction_preserves_live_payload_identity_tokens_tickets_and_reclaims_capacity() {
    use std::{cell::Cell, rc::Rc};
    struct Payload {
        value: usize,
        drops: Rc<Cell<usize>>,
    }
    impl Drop for Payload {
        fn drop(&mut self) {
            self.drops.set(self.drops.get() + 1);
        }
    }
    let drops = Rc::new(Cell::new(0));
    let mut document = Document::new();
    document.append(&"body\n".repeat(2000)).unwrap();
    let mut view = FoldView::<Payload>::with_state(&document, 2).unwrap();
    for i in 0..document.tile_count() {
        view.insert_checkpoint(
            TileId(i),
            Payload {
                value: i,
                drops: drops.clone(),
            },
        )
        .unwrap();
    }
    let last = document.tile_count() - 1;
    view.layout_parts_mut(&document)
        .unwrap()
        .1
        .invalidate(0..last)
        .unwrap();
    let pointer = view.checkpoints().get(TileId(last)).unwrap().drops.as_ptr();
    let token = view.checkpoints().token(TileId(last)).unwrap();
    let ticket = view
        .heights()
        .ticket_from(TileId(last), view.checkpoints(), TileId(last))
        .unwrap();
    let old_capacity = view.checkpoints().arena_stats().capacity_bytes;
    let height_arena = view.heights().arena_stats();
    let mut task = view.into_checkpoint_compaction();
    let mut retired = Vec::new();
    assert_eq!(drops.get(), 0);
    for round in 0..10000 {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        if task.advance(&document, &mut budget, &mut retired).unwrap() == Completion::Covered {
            break;
        }
        assert!(round < 9999);
    }
    assert_eq!(drops.get(), 0); // No unbudgeted user destructor in maintenance.
    let mut view = match task.finish(&document) {
        Ok(view) => view,
        Err(_) => panic!("unfinished"),
    };
    assert_eq!(view.heights().arena_stats(), height_arena);
    assert_eq!(view.checkpoints().get(TileId(last)).unwrap().value, last);
    assert_eq!(
        view.checkpoints().get(TileId(last)).unwrap().drops.as_ptr(),
        pointer
    );
    assert!(view.checkpoints().validates(token));
    view.measure(ticket, 4).unwrap();
    assert!(view.checkpoints().arena_stats().capacity_bytes < old_capacity);
    assert_eq!(retired.len(), last);
    drop(retired);
    assert_eq!(drops.get(), last);
    drop(view);
    assert_eq!(drops.get(), last + 1);
}
