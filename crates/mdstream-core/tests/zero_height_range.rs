use mdstream::*;

#[test]
fn base_zero_range_keeps_fold_masks_and_charges_one_logarithmic_operation() {
    let mut doc = Document::new();
    doc.append(&format!("# A\n{}\n# B\ntail\n", "x".repeat(16384)))
        .unwrap();
    doc.finish();
    let mut view = FoldView::<()>::with_state(&doc, 2).unwrap();
    view.set_collapsed(&doc, FoldId::Heading(HeadingId(0)), true)
        .unwrap();
    let mask = fold_mask(&doc, &view, &[FoldId::Heading(HeadingId(0))], None);
    let range = 2..doc.tile_count() - 2;
    let (heights, checkpoints) = view.layout_parts_mut(&doc).unwrap();
    let ticket = heights.zero_height_ticket(range.clone()).unwrap();
    let before = heights.mutation_visits();
    assert_eq!(
        heights.measure_zero_height_from(
            ticket,
            true,
            checkpoints,
            &mut Budget::new(WorkLimit::default())
        ),
        Ok(Completion::BudgetExhausted)
    );
    assert_eq!(heights.mutation_visits(), before);
    let mut budget = Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        ..Default::default()
    });
    assert_eq!(
        heights.measure_zero_height_from(ticket, true, checkpoints, &mut budget),
        Ok(Completion::Covered)
    );
    assert!(heights.mutation_visits() - before < 200);
    assert_eq!(budget.used().queries, 1);
    assert_eq!(budget.used().objects, 1);
    for (i, hidden) in mask.into_iter().enumerate() {
        let base = if range.contains(&i) { 0 } else { 2 };
        assert_eq!(heights.base_height(TileId(i)), Some(base));
        assert_eq!(
            heights.height(TileId(i)),
            Some(if hidden { 0 } else { base })
        );
        if range.contains(&i) {
            assert_eq!(heights.height_state(TileId(i)), Some(HeightState::Local));
        }
    }
}

fn micro() -> Budget {
    Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 6,
        ..Default::default()
    })
}

#[test]
fn zero_budget_and_stale_errors_account_for_actual_index_queries() {
    let mut h = HeightIndex::new(1).unwrap();
    h.grow_to(100).unwrap();
    let ticket = h.zero_height_ticket(20..90).unwrap();
    let mut other = HeightIndex::new(1).unwrap();
    other.grow_to(100).unwrap();
    let mut b = micro();
    assert_eq!(
        other.measure_zero_height(ticket, false, &mut b),
        Err(LayoutError::WrongView)
    );
    assert_eq!(b.used().queries, 0);
    assert_eq!(other.zero_height_queries(), 0);
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
        let before = (h.zero_height_queries(), h.mutation_visits());
        assert_eq!(
            h.measure_zero_height(ticket, false, &mut Budget::new(limit)),
            Ok(Completion::BudgetExhausted)
        );
        assert_eq!((h.zero_height_queries(), h.mutation_visits()), before);
    }
    h.expire(89..90).unwrap();
    let before = (h.zero_height_queries(), h.mutation_visits());
    assert_eq!(
        h.measure_zero_height(ticket, false, &mut Budget::new(WorkLimit::default())),
        Ok(Completion::BudgetExhausted)
    );
    assert_eq!((h.zero_height_queries(), h.mutation_visits()), before);
    let mut b = micro();
    assert_eq!(
        h.measure_zero_height(ticket, false, &mut b),
        Err(LayoutError::StaleMeasurement)
    );
    assert_eq!(b.used().queries, 1);
    assert_eq!(h.zero_height_queries(), before.0 + 1);
    assert_eq!(h.mutation_visits(), before.1);
    let ticket = h.zero_height_ticket(20..90).unwrap();
    h.reset(2).unwrap();
    let mut b = micro();
    assert_eq!(
        h.measure_zero_height(ticket, false, &mut b),
        Err(LayoutError::StaleMeasurement)
    );
    assert_eq!(b.used().queries, 0);
    let mut c = CheckpointIndex::default();
    c.insert(TileId(20), ()).unwrap();
    let t = h.zero_height_ticket_from(20..90, &c, TileId(20)).unwrap();
    c.insert(TileId(20), ()).unwrap();
    let before = (h.zero_height_queries(), h.mutation_visits());
    assert_eq!(
        h.measure_zero_height_from(t, false, &c, &mut Budget::new(WorkLimit::default())),
        Ok(Completion::BudgetExhausted)
    );
    assert_eq!((h.zero_height_queries(), h.mutation_visits()), before);
    let mut b = micro();
    assert_eq!(
        h.measure_zero_height_from(t, false, &c, &mut b),
        Err(LayoutError::StaleMeasurement)
    );
    assert_eq!(b.used().queries, 1);
    assert_eq!(h.zero_height_queries(), before.0 + 1);
    let t = h.zero_height_ticket_from(20..90, &c, TileId(20)).unwrap();
    let mut b = micro();
    assert_eq!(
        h.measure_zero_height_from(t, false, &c, &mut b),
        Ok(Completion::Covered)
    );
    assert_eq!(h.zero_height_queries(), before.0 + 3);
}

#[test]
fn million_continuations_are_one_same_tree_logarithmic_assignment() {
    // 2^18 production 256-byte tiles model 64 MiB; 2^20 models a million.
    // The standalone height index does not allocate source bytes.
    for power in [10, 14, 18, 20] {
        let n = 1 << power;
        let mut h = HeightIndex::new(3).unwrap();
        h.grow_to(n).unwrap();
        let t = h.zero_height_ticket(1..n - 1).unwrap();
        let before = h.mutation_visits();
        let bytes = h.arena_stats().capacity_bytes;
        assert_eq!(
            h.measure_zero_height(t, false, &mut micro()),
            Ok(Completion::Covered)
        );
        assert!(h.mutation_visits() - before <= 8 * power as u64);
        assert_eq!(h.total_rows(), 6);
        assert_eq!(h.base_height(TileId(n / 2)), Some(0));
        assert_eq!(h.height_state(TileId(n / 2)), Some(HeightState::Measured));
        assert!(h.arena_stats().capacity_bytes <= bytes + power * 1024);
        assert!(h.node_count() < 8 * power);
        eprintln!(
            "tiles={n} visits={} nodes={} capacity={}",
            h.mutation_visits() - before,
            h.node_count(),
            h.arena_stats().capacity_bytes
        );
    }
}

#[test]
fn prefix_tickets_survive_unrelated_append_but_not_rebinding_or_fold_changes() {
    let mut d = Document::new();
    d.append(&format!("# A\n{}\n\n# B\nend\n\n", "x".repeat(4096)))
        .unwrap();
    let mut v = FoldView::<()>::with_state(&d, 1).unwrap();
    let t = v.heights().zero_height_ticket(2..8).unwrap();
    let u = d.append("more\n").unwrap();
    v.apply(&d, &u).unwrap();
    {
        let (h, c) = v.layout_parts_mut(&d).unwrap();
        assert_eq!(
            h.measure_zero_height_from(t, true, c, &mut micro()),
            Ok(Completion::Covered)
        );
    }
    let t = v.heights().zero_height_ticket(2..8).unwrap();
    v.set_collapsed(&d, FoldId::Heading(HeadingId(0)), true)
        .unwrap();
    {
        let (h, c) = v.layout_parts_mut(&d).unwrap();
        assert_eq!(
            h.measure_zero_height_from(t, true, c, &mut micro()),
            Err(LayoutError::StaleMeasurement)
        );
    }
    let t = v.heights().zero_height_ticket(2..8).unwrap();
    v.set_preview(
        &d,
        Some(FoldId::Heading(HeadingId(0))),
        &mut Budget::new(WorkLimit::unlimited()),
    )
    .unwrap();
    {
        let (h, c) = v.layout_parts_mut(&d).unwrap();
        assert_eq!(
            h.measure_zero_height_from(t, true, c, &mut micro()),
            Err(LayoutError::StaleMeasurement)
        );
    }
    let mut d = Document::new();
    d.append(&format!("[x][r]\n\n{}\n\n", "x".repeat(4096)))
        .unwrap();
    let mut v = FoldView::<()>::with_state(&d, 1).unwrap();
    let t = v.heights().zero_height_ticket(2..8).unwrap();
    let u = d.append("[r]: /u\n").unwrap();
    v.apply(&d, &u).unwrap();
    let (h, c) = v.layout_parts_mut(&d).unwrap();
    assert_eq!(
        h.measure_zero_height_from(t, false, c, &mut micro()),
        Err(LayoutError::StaleMeasurement)
    );
}

#[test]
fn nested_fold_masks_match_independent_arrays_after_zero_measurement_and_reset() {
    let mut d = Document::new();
    d.append(&format!(
        "# A\n{}\n## child\n{}\n# B\nend\n",
        "x".repeat(8192),
        "y".repeat(8192)
    ))
    .unwrap();
    d.finish();
    let mut v = FoldView::<()>::with_state(&d, 2).unwrap();
    for id in [HeadingId(0), HeadingId(1)] {
        v.set_collapsed(&d, FoldId::Heading(id), true).unwrap();
    }
    let mut base = vec![2; d.tile_count()];
    for (parent, preview) in [
        (false, None),
        (true, None),
        (true, Some(FoldId::Heading(HeadingId(0)))),
        (false, None),
    ] {
        v.set_collapsed(&d, FoldId::Heading(HeadingId(0)), parent)
            .unwrap();
        v.set_preview(&d, preview, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap();
        v.reset(2).unwrap();
        base.fill(2);
        let mask = fold_mask(
            &d,
            &v,
            &[FoldId::Heading(HeadingId(0)), FoldId::Heading(HeadingId(1))],
            preview,
        );
        let range = 2..d.tile_count() - 2;
        {
            let (h, c) = v.layout_parts_mut(&d).unwrap();
            let ticket = h.zero_height_ticket(range.clone()).unwrap();
            h.measure_zero_height_from(ticket, true, c, &mut micro())
                .unwrap();
        }
        base[range].fill(0);
        for i in 0..d.tile_count() {
            assert_eq!(v.heights().base_height(TileId(i)), Some(base[i]));
            assert_eq!(
                v.heights().height(TileId(i)),
                Some(if mask[i] { 0 } else { base[i] })
            );
        }
        v.reset(3).unwrap();
        for (i, hidden) in mask.iter().enumerate() {
            assert_eq!(v.heights().base_height(TileId(i)), Some(3));
            assert_eq!(
                v.heights().height(TileId(i)),
                Some(if *hidden { 0 } else { 3 })
            );
        }
    }
}

#[test]
fn exact_checkpoint_excludes_older_prefix_but_covers_range_end() {
    let mut h = HeightIndex::new(1).unwrap();
    h.grow_to(100).unwrap();
    let mut c = CheckpointIndex::default();
    c.insert(TileId(20), ()).unwrap();
    let ticket = h.zero_height_ticket_from(20..90, &c, TileId(20)).unwrap();
    h.expire(0..19).unwrap();
    assert_eq!(
        h.measure_zero_height_from(ticket, false, &c, &mut micro()),
        Ok(Completion::Covered)
    );
    let ticket = h.zero_height_ticket_from(20..90, &c, TileId(20)).unwrap();
    h.expire(89..90).unwrap();
    assert_eq!(
        h.measure_zero_height_from(ticket, false, &c, &mut micro()),
        Err(LayoutError::StaleMeasurement)
    );
}

fn fold_mask(d: &Document, v: &FoldView<()>, ids: &[FoldId], preview: Option<FoldId>) -> Vec<bool> {
    let mut mask = vec![false; d.tile_count()];
    for &id in ids {
        if !v.is_collapsed(id) || Some(id) == preview {
            continue;
        }
        let hidden = v.fold_candidate(d, id).unwrap().unwrap().hidden;
        if hidden.is_empty() {
            continue;
        }
        for (i, covered) in mask.iter_mut().enumerate() {
            let tile = d.tile(TileId(i)).unwrap();
            *covered |= tile.start > hidden.start
                && tile.end <= hidden.end
                && i > d.tile_at(hidden.start).unwrap().0
                && i < d.tile_at(hidden.end).map_or(d.tile_count(), |t| t.0);
        }
    }
    mask
}
