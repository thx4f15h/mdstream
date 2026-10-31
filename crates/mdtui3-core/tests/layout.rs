use mdtui3::{Anchor, Document, HeightIndex, LayoutError, TileId};

#[test]
fn weighted_selection_zero_regions_and_anchor_correction() {
    let mut index = HeightIndex::new(2).unwrap();
    index.grow_to(8).unwrap();
    for (tile, rows) in [2, 0, 0, 5, 0, 1, 0, 3].into_iter().enumerate() {
        index
            .measure(index.ticket(TileId(tile)).unwrap(), rows)
            .unwrap();
    }
    assert_eq!(index.total_rows(), 11);
    assert_eq!(
        index.locate(2),
        Some(Anchor {
            tile: TileId(3),
            offset: 0
        })
    );
    assert_eq!(
        index.locate(6),
        Some(Anchor {
            tile: TileId(3),
            offset: 4
        })
    );
    assert_eq!(index.locate(11), None);
    let viewport = index.viewport(1..9, 0, 10);
    assert_eq!(
        viewport.iter().map(|v| v.tile.0).collect::<Vec<_>>(),
        [0, 3, 5, 7]
    );
    let anchor = index.locate(6).unwrap();
    index.measure(index.ticket(TileId(0)).unwrap(), 9).unwrap();
    assert_eq!(index.row_of(anchor), Some(13));
}

#[test]
fn randomized_height_index_matches_naive_prefix_sums() {
    let mut index = HeightIndex::default();
    let mut model = Vec::<u64>::new();
    let mut seed = 7_u64;
    let mut random = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed as usize
    };
    let mut estimate = 1;
    for iteration in 0..2000 {
        match random() % 4 {
            0 => {
                let added = random() % 8;
                model.resize(model.len() + added, estimate);
                index.grow_to(model.len()).unwrap();
            }
            1 if !model.is_empty() => {
                let i = random() % model.len();
                let height = (random() % 20) as u64;
                index
                    .measure(index.ticket(TileId(i)).unwrap(), height)
                    .unwrap();
                model[i] = height;
            }
            2 if !model.is_empty() => {
                let a = random() % (model.len() + 1);
                let b = random() % (model.len() + 1);
                let range = a.min(b)..a.max(b);
                index.invalidate(range.clone()).unwrap();
                model[range].fill(estimate);
            }
            _ => {
                estimate = (random() % 9 + 1) as u64;
                index.reset(estimate).unwrap();
                model.fill(estimate);
            }
        }
        let total: u128 = model.iter().map(|&h| u128::from(h)).sum();
        assert_eq!(index.total_rows(), total, "iteration {iteration}");
        for _ in 0..5 {
            let end = random() % (model.len() + 1);
            assert_eq!(
                index.prefix_rows(end),
                Some(model[..end].iter().map(|&h| u128::from(h)).sum())
            );
            if total > 0 {
                let row = random() as u128 % total;
                let mut prefix = 0;
                let expected = model.iter().enumerate().find_map(|(i, &height)| {
                    if prefix + u128::from(height) > row {
                        Some(Anchor {
                            tile: TileId(i),
                            offset: (row - prefix) as u64,
                        })
                    } else {
                        prefix += u128::from(height);
                        None
                    }
                });
                assert_eq!(index.locate(row), expected);
            }
        }
    }
}

#[test]
fn resize_reuses_nodes_and_rejects_stale_measurements() {
    let mut index = HeightIndex::default();
    index.grow_to(1024).unwrap();
    for i in 0..1024 {
        index.measure(index.ticket(TileId(i)).unwrap(), 2).unwrap();
    }
    let nodes = index.node_count();
    for _ in 0..20 {
        let stale = index.ticket(TileId(3)).unwrap();
        let before = index.mutation_visits();
        index.reset(3).unwrap();
        assert_eq!(index.mutation_visits() - before, 1);
        assert_eq!(index.measure(stale, 3), Err(LayoutError::StaleMeasurement));
        for i in 0..1024 {
            index.measure(index.ticket(TileId(i)).unwrap(), 2).unwrap();
        }
        assert_eq!(index.node_count(), nodes);
    }
}

#[test]
fn streaming_updates_reference_reset_and_skipped_update_recovery() {
    let mut doc = Document::new();
    let mut index = HeightIndex::default();
    let update = doc.append(&"[x] ".repeat(1000)).unwrap();
    index.apply(&doc, &update).unwrap();
    let update = doc.append("\n\n").unwrap();
    index.apply(&doc, &update).unwrap();
    index.measure(index.ticket(TileId(0)).unwrap(), 8).unwrap();
    let stale = index.ticket(TileId(0)).unwrap();
    let update = doc.append("[x]: /url\n").unwrap();
    index.apply(&doc, &update).unwrap();
    assert_eq!(index.height(TileId(0)), Some(8));
    assert_eq!(
        index.height_state(TileId(0)),
        Some(mdtui3::HeightState::Expired)
    );
    assert_eq!(index.measure(stale, 5), Err(LayoutError::StaleMeasurement));
    doc.append("missed\n").unwrap();
    let update = doc.append("again\n").unwrap();
    assert_eq!(
        index.apply(&doc, &update),
        Err(LayoutError::OutOfOrderUpdate)
    );
    index.rebuild(&doc).unwrap();
    let update = doc.finish();
    index.apply(&doc, &update).unwrap();
}

#[test]
fn huge_sparse_viewports_and_height_arithmetic() {
    let mut index = HeightIndex::new(u64::MAX).unwrap();
    index.grow_to(1_000_000_000).unwrap();
    assert_eq!(
        index.total_rows(),
        1_000_000_000_u128 * u128::from(u64::MAX)
    );
    assert!(index.node_count() < 100);
    index.reset(1).unwrap();
    // A single visible tile at the very end can be selected without enumerating
    // the billion estimated leaves.
    let visible = index.viewport(999_999_999..1_000_000_000, 0, 2);
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].tile, TileId(999_999_999));
    assert_eq!(index.viewport(0..u128::MAX, u64::MAX, 3).len(), 3);
    assert!(HeightIndex::new(0).is_err());
    assert!(
        index
            .invalidate(std::ops::Range { start: 2, end: 1 })
            .is_err()
    );
    assert!(index.grow_to(0).is_err());
}
