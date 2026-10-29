use mdtui3::*;

#[test]
fn mixed_expiration_assignment_resize_and_growth_match_naive_model() {
    let mut tree = HeightIndex::new(2).unwrap();
    let mut values = Vec::new();
    let mut states = Vec::new();
    let mut seed = 731_u64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed as usize
    };
    let mut estimate = 2;
    for _ in 0..3000 {
        let len = values.len();
        let a = next() % (len + 1);
        let b = next() % (len + 1);
        let range = a.min(b)..a.max(b);
        match next() % 6 {
            0 => {
                let new = len + next() % 5;
                tree.grow_to(new).unwrap();
                values.resize(new, estimate);
                states.resize(new, HeightState::Estimated);
            }
            1 if len > 0 => {
                let i = next() % len;
                let height = (next() % 10) as u64;
                tree.measure(tree.ticket(TileId(i)).unwrap(), height)
                    .unwrap();
                values[i] = height;
                states[i] = HeightState::Measured;
            }
            2 => {
                tree.expire(range.clone()).unwrap();
                for i in range {
                    if values[i] == 0 {
                        values[i] = estimate;
                    }
                    states[i] = HeightState::Expired;
                }
            }
            3 => {
                tree.suppress(range.clone()).unwrap();
                values[range.clone()].fill(0);
                states[range].fill(HeightState::Measured);
            }
            4 => {
                tree.invalidate(range.clone()).unwrap();
                values[range.clone()].fill(estimate);
                states[range].fill(HeightState::Estimated);
            }
            _ => {
                estimate = (next() % 5 + 1) as u64;
                tree.reset(estimate).unwrap();
                values.fill(estimate);
                states.fill(HeightState::Estimated);
            }
        }
        assert_eq!(
            tree.total_rows(),
            values.iter().copied().map(u128::from).sum::<u128>()
        );
        let mut prefix = 0;
        for (i, &height) in values.iter().enumerate() {
            assert_eq!(tree.height(TileId(i)), Some(height));
            assert_eq!(tree.height_state(TileId(i)), Some(states[i]));
            assert_eq!(tree.prefix_rows(i), Some(prefix));
            if height > 0 {
                assert_eq!(tree.locate(prefix).unwrap().tile, TileId(i));
            }
            prefix += u128::from(height);
        }
    }
}

#[test]
fn suffix_append_preserves_tickets_but_prior_reference_changes_reject_them() {
    let mut doc = Document::new();
    doc.append(&format!("![][late]\n\n{}\n\n", "text ".repeat(500)))
        .unwrap();
    let mut heights = HeightIndex::default();
    heights.rebuild(&doc).unwrap();
    let early = heights.ticket(TileId(1)).unwrap();
    let update = doc.append("new tail\n").unwrap();
    heights.apply(&doc, &update).unwrap();
    heights.measure(early, 4).unwrap();
    let stale = heights.ticket(TileId(2)).unwrap(); // this tile has no reference itself
    let update = doc.append("\n[late]: image.png\n").unwrap();
    heights.apply(&doc, &update).unwrap();
    assert_eq!(
        heights.measure(stale, 3),
        Err(LayoutError::StaleMeasurement)
    );
    assert_eq!(heights.height(TileId(1)), Some(4));
}

#[test]
fn render_policy_preserves_unaffected_heights_and_checkpoints() {
    let mut doc = Document::new();
    doc.append(&format!("[late]\n\n{}\n\n", "x".repeat(2000)))
        .unwrap();
    let mut heights = HeightIndex::default();
    heights.rebuild(&doc).unwrap();
    let mut checkpoints = CheckpointIndex::new();
    checkpoints.rebuild(&doc);
    heights
        .measure(heights.ticket(TileId(0)).unwrap(), 6)
        .unwrap();
    checkpoints.insert(TileId(0), 7).unwrap();
    let ticket = heights.ticket(TileId(0)).unwrap();
    let update = doc.append("[late]: /url\n").unwrap();
    let policy = LayoutPolicy {
        references_affect_size: false,
        ..Default::default()
    };
    heights.apply_with_policy(&doc, &update, policy).unwrap();
    checkpoints
        .apply_with_policy(&doc, &update, policy)
        .unwrap();
    assert_eq!(heights.height_state(TileId(0)), Some(HeightState::Measured));
    assert_eq!(checkpoints.get(TileId(0)), Some(&7));
    heights.measure(ticket, 6).unwrap();
}

#[test]
fn expiring_billion_zero_heights_revives_the_range_without_visiting_leaves() {
    let mut heights = HeightIndex::default();
    heights.grow_to(1_000_000_000).unwrap();
    heights.suppress(0..heights.len()).unwrap();
    let before = heights.mutation_visits();
    heights.expire(10..999_999_990).unwrap();
    assert!(heights.mutation_visits() - before < 200);
    assert_eq!(heights.locate(0).unwrap().tile, TileId(10));
    assert_eq!(heights.total_rows(), 999_999_980);
}

#[test]
fn fixed_column_width_policy_keeps_old_table_layout_valid() {
    let mut doc = Document::new();
    doc.append(&format!("a|b\n-|-\n{}", "x|y\n".repeat(1000)))
        .unwrap();
    let mut heights = HeightIndex::default();
    heights.rebuild(&doc).unwrap();
    let mut checkpoints = CheckpointIndex::new();
    checkpoints.rebuild(&doc);
    heights
        .measure(heights.ticket(TileId(0)).unwrap(), 5)
        .unwrap();
    checkpoints.insert(TileId(0), 7).unwrap();
    let ticket = heights.ticket(TileId(0)).unwrap();
    let update = doc.append("wider|cell\n").unwrap();
    assert!(!update.tables_changed.is_empty());
    apply_layout_update_with_policy(
        &doc,
        &update,
        &mut heights,
        &mut checkpoints,
        LayoutPolicy {
            table_width_affects_size: false,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(heights.height_state(TileId(0)), Some(HeightState::Measured));
    assert_eq!(checkpoints.get(TileId(0)), Some(&7));
    heights.measure(ticket, 5).unwrap();
}
