use mdstream::*;
use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
};

fn budget(objects: usize, queries: usize) -> Budget {
    Budget::new(WorkLimit {
        objects,
        queries,
        ..Default::default()
    })
}

fn finish<S>(task: CheckpointCompaction<S>) -> CheckpointIndex<S> {
    match task.finish() {
        Ok(index) => index,
        Err(_) => panic!("unfinished compaction"),
    }
}

fn compact<S>(index: CheckpointIndex<S>, batch: usize) -> (CheckpointIndex<S>, Vec<S>) {
    let mut task = index.into_compaction();
    let mut out = Vec::new();
    while task.advance(&mut budget(batch, batch), &mut out) != Completion::Covered {}
    (finish(task), out)
}

// Independent union of dyadic intervals. Growth retains the initial empty
// left spine even if no checkpoint was inserted at tile zero.
fn intervals(points: impl IntoIterator<Item = usize>) -> BTreeSet<(u128, u128)> {
    let points: Vec<_> = points.into_iter().collect();
    let end = (points.iter().max().copied().unwrap_or(0) as u128 + 1).next_power_of_two();
    let mut result = BTreeSet::new();
    for point in points.into_iter().chain([0]) {
        let (mut lo, mut hi) = (0, end);
        loop {
            result.insert((lo, hi));
            if hi - lo == 1 {
                break;
            }
            let mid = (lo + hi) / 2;
            if (point as u128) < mid {
                hi = mid;
            } else {
                lo = mid;
            }
        }
    }
    result
}

#[test]
fn compaction_budget_is_atomic_shared_resumable_and_complete_is_idempotent() {
    let mut index = CheckpointIndex::new();
    index.insert(TileId(0), 7).unwrap();
    index.insert(TileId(2), 9).unwrap();
    index.invalidate(2..3).unwrap();
    let mut task = index.into_compaction();
    let mut out = vec![99];
    for (objects, queries) in [(0, 0), (1, 0), (0, usize::MAX)] {
        let before = task.stats();
        let mut b = budget(objects, queries);
        assert_eq!(task.advance(&mut b, &mut out), Completion::BudgetExhausted);
        assert_eq!(b.used(), WorkLimit::default());
        assert_eq!(task.stats(), before);
        assert_eq!(out, [99]);
    }
    task = match task.finish() {
        Err(task) => task,
        Ok(_) => panic!("early finish"),
    };
    loop {
        let mut b = Budget::new(WorkLimit {
            objects: 4,
            queries: 2,
            dependencies: 1,
            ..Default::default()
        });
        assert!(b.charge(WorkLimit {
            queries: 1,
            dependencies: 1,
            ..Default::default()
        }));
        let before = task.stats();
        let status = task.advance(&mut b, &mut out);
        let after = task.stats();
        assert_eq!(
            after.visited + after.retired - before.visited - before.retired,
            1
        );
        assert_eq!(
            b.used(),
            WorkLimit {
                objects: 1,
                queries: 2,
                dependencies: 1,
                ..Default::default()
            }
        );
        assert_eq!(task.get(TileId(0)), Some(&7));
        assert_eq!(task.len(), 1);
        assert!(!task.is_empty());
        assert_eq!(task.get(TileId(2)), None);
        if status == Completion::Covered {
            break;
        }
    }
    let stats = task.stats();
    assert_eq!(stats.source.capacity_bytes, 0);
    assert_eq!(stats.source.nodes, 0);
    assert_eq!(
        task.advance(&mut budget(0, 0), &mut out),
        Completion::Covered
    );
    assert_eq!(task.stats(), stats);
    assert_eq!(out, [99, 9]);
    assert_eq!(finish(task).node_count(), 1);
}

#[test]
fn compaction_preserves_predecessors_and_tokens_at_every_pause_under_lazy_invalidation() {
    let mut index = CheckpointIndex::new();
    let mut model = BTreeMap::new();
    let mut tokens = Vec::new();
    for i in 0..256 {
        index.insert(TileId(i * 17), i).unwrap();
        model.insert(i * 17, i);
        tokens.push((i * 17, index.token(TileId(i * 17)).unwrap()));
    }
    index.invalidate(17..240 * 17).unwrap();
    model.retain(|&k, _| !(17..240 * 17).contains(&k));
    index.insert(TileId(97 * 17), 999).unwrap();
    model.insert(97 * 17, 999);
    let survivor = index.token(TileId(97 * 17)).unwrap();
    let old = index.arena_stats();
    let mut task = index.into_compaction();
    let mut out = Vec::new();
    loop {
        let before = task.stats();
        let mut b = budget(7, 3);
        let status = task.advance(&mut b, &mut out);
        let after = task.stats();
        assert_eq!(b.used().objects, b.used().queries);
        assert!(b.used().objects <= 3);
        assert_eq!(
            after.visited + after.retired - before.visited - before.retired,
            b.used().objects
        );
        assert!(
            after.mutation_visits - before.mutation_visits <= 3 * (1 + 4 * u64::from(usize::BITS))
        );
        assert_eq!(task.len(), model.len());
        for at in (0..256 * 17 + 17).step_by(11).chain([usize::MAX]) {
            assert_eq!(
                task.previous(TileId(at)).map(|(id, s)| (id.0, *s)),
                model.range(..=at).next_back().map(|(&id, &s)| (id, s))
            );
            assert_eq!(task.get(TileId(at)), model.get(&at));
        }
        for &(tile, token) in &tokens {
            assert_eq!(
                task.validates(token),
                model.contains_key(&tile) && tile != 97 * 17
            );
        }
        assert!(task.validates(survivor));
        if status == Completion::Covered {
            break;
        }
    }
    let stats = task.stats();
    assert_eq!(stats.retired, old.nodes);
    assert_eq!(stats.moved, model.len());
    assert!(stats.visited <= 2 * intervals(model.keys().copied()).len());
    let index = finish(task);
    assert_eq!(index.node_count(), intervals(model.keys().copied()).len());
    assert!(index.arena_stats().capacity_bytes < old.capacity_bytes);
    assert!(index.arena_stats().allocation_calls >= old.allocation_calls);
    out.sort_unstable();
    assert_eq!(out, (1..240).filter(|&i| i != 97).collect::<Vec<_>>());
}

struct State {
    value: usize,
    drops: Rc<Cell<usize>>,
}
impl Drop for State {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
    }
}

#[test]
fn compaction_cleared_cache_retires_each_node_and_releases_capacity_without_user_drops() {
    let drops = Rc::new(Cell::new(0));
    let mut index = CheckpointIndex::new();
    for value in 0..4096 {
        index
            .insert(
                TileId(value * 97),
                State {
                    value,
                    drops: drops.clone(),
                },
            )
            .unwrap();
    }
    index.clear();
    let old = index.arena_stats();
    let mut task = index.into_compaction();
    let mut out = Vec::new();
    let mut steps = 0;
    loop {
        let mut b = budget(31, 31);
        let status = task.advance(&mut b, &mut out);
        steps += b.used().objects;
        assert_eq!(drops.get(), 0);
        assert!(task.is_empty());
        assert!(task.previous(TileId(usize::MAX)).is_none());
        let stats = task.stats();
        assert_eq!(stats.visited, 1); // invalid root skips every stale descendant
        if status == Completion::Covered {
            break;
        }
        assert_eq!(stats.source.capacity_bytes, old.capacity_bytes);
        assert_eq!(stats.source.nodes + stats.retired, old.nodes);
    }
    assert_eq!(steps, old.nodes + 1);
    assert_eq!(task.stats().source.capacity_bytes, 0);
    assert_eq!(out.len(), 4096);
    assert_eq!(out.iter().map(|s| s.value).sum::<usize>(), (0..4096).sum());
    let index = finish(task);
    assert_eq!(index.node_count(), 1);
    assert!(index.arena_stats().capacity_bytes * 1000 < old.capacity_bytes);
    drop(index);
    assert_eq!(drops.get(), 0);
    drop(out);
    assert_eq!(drops.get(), 4096);
}

#[test]
fn compaction_random_history_matches_ordered_map_and_dyadic_node_model() {
    let mut index = CheckpointIndex::new();
    let mut model = BTreeMap::new();
    let mut seed = 0x9701_u64;
    let mut random = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed as usize
    };
    for round in 0..100 {
        for operation in 0..96 {
            let a = random() % 8192;
            match random() % 12 {
                0 => {
                    index.clear();
                    model.clear();
                }
                1..=4 => {
                    let b = random() % 8192;
                    index.invalidate(a.min(b)..a.max(b)).unwrap();
                    model.retain(|&k, _| !(a.min(b)..a.max(b)).contains(&k));
                }
                _ => {
                    let value = round * 96 + operation;
                    index.insert(TileId(a), value).unwrap();
                    model.insert(a, value);
                }
            }
        }
        let tokens: Vec<_> = model
            .keys()
            .map(|&k| index.token(TileId(k)).unwrap())
            .collect();
        let old_nodes = index.node_count();
        let mut task = index.into_compaction();
        let mut out = Vec::new();
        loop {
            let status = task.advance(&mut budget(1 + round % 31, 17), &mut out);
            let at = random() % 9000;
            assert_eq!(
                task.previous(TileId(at)).map(|(id, s)| (id.0, *s)),
                model.range(..=at).next_back().map(|(&k, &v)| (k, v))
            );
            assert!(tokens.iter().all(|&t| task.validates(t)));
            assert_eq!(task.len(), model.len());
            if status == Completion::Covered {
                break;
            }
        }
        assert_eq!(task.stats().retired, old_nodes);
        index = finish(task);
        assert_eq!(index.node_count(), intervals(model.keys().copied()).len());
        for at in 0..9000 {
            assert_eq!(
                index.previous(TileId(at)).map(|(id, s)| (id.0, *s)),
                model.range(..=at).next_back().map(|(&k, &v)| (k, v))
            );
        }
    }
}

#[test]
fn compaction_repeat_refill_does_not_accumulate_historical_nodes_or_resurrect_tokens() {
    let mut index = CheckpointIndex::new();
    index.insert(TileId(0), 999).unwrap();
    let retained = index.token(TileId(0)).unwrap();
    let small = index.arena_stats().capacity_bytes;
    let mut stale = Vec::new();
    for round in 0..32 {
        for i in 0..128 {
            let tile = TileId(1 + round * 8192 + i * 17);
            assert!(index.insert(tile, i).unwrap().is_none());
            stale.push(index.token(tile).unwrap());
        }
        index.invalidate(1..usize::MAX).unwrap();
        let (next, out) = compact(index, 7);
        index = next;
        assert_eq!(out.len(), 128);
        assert_eq!(index.arena_stats().capacity_bytes, small);
        assert_eq!(index.node_count(), 1);
        assert!(index.validates(retained));
        assert!(stale.iter().all(|&t| !index.validates(t)));
    }
    index.insert(TileId(1), 77).unwrap();
    assert!(stale.iter().all(|&t| !index.validates(t)));
}

#[test]
fn compaction_extreme_indices_keep_logarithmic_scratch_and_old_measurement_semantics() {
    for end in [1023, (1_usize << 30) + 1, isize::MAX as usize] {
        let mut index = CheckpointIndex::new();
        let mut model = BTreeMap::new();
        let mut points = BTreeSet::from([0, 1, end / 2, end - 2, end - 1]);
        for bit in 1..usize::BITS - 1 {
            let n = 1_usize << bit;
            for at in [n - 1, n, n + 1] {
                if at < end {
                    points.insert(at);
                }
            }
        }
        for (value, &at) in points.iter().rev().enumerate() {
            index.insert(TileId(at), value).unwrap();
            model.insert(at, value);
        }
        index.invalidate(2..end / 2).unwrap();
        model.retain(|&k, _| !(2..end / 2).contains(&k));
        let mut heights = HeightIndex::default();
        heights.grow_to(end).unwrap();
        let ticket = heights
            .ticket_from(TileId(end - 1), &index, TileId(end - 2))
            .unwrap();
        let mut task = index.into_compaction();
        let mut out = Vec::new();
        loop {
            let before = task.stats();
            let status = task.advance(&mut budget(1, 1), &mut out);
            let after = task.stats();
            assert!(after.scratch.nodes <= usize::BITS as usize);
            assert!(after.scratch.capacity_bytes <= 2 * usize::BITS as usize * size_of::<usize>());
            assert!(
                after.mutation_visits - before.mutation_visits <= 1 + 4 * u64::from(usize::BITS)
            );
            for at in [0, end / 2, end - 2, end - 1, usize::MAX] {
                assert_eq!(
                    task.previous(TileId(at)).map(|(id, s)| (id.0, *s)),
                    model.range(..=at).next_back().map(|(&k, &v)| (k, v))
                );
            }
            if status == Completion::Covered {
                break;
            }
        }
        let mut index = finish(task);
        assert_eq!(index.node_count(), intervals(model.keys().copied()).len());
        heights.measure_from(ticket, 7, &index).unwrap();
        let old = index.token(TileId(end - 1)).unwrap();
        index.insert(TileId(end - 1), 999).unwrap();
        assert!(!index.validates(old));
        for bad in [isize::MAX as usize, usize::MAX] {
            assert_eq!(index.insert(TileId(bad), 0), Err(LayoutError::InvalidRange));
        }
    }
}

#[test]
fn compaction_preserves_document_revision_tickets_and_rejects_missed_updates() {
    let mut doc = Document::new();
    doc.append(&"text ".repeat(3000)).unwrap();
    let mut heights = HeightIndex::default();
    heights.rebuild(&doc).unwrap();
    let mut index = CheckpointIndex::new();
    index.rebuild(&doc);
    for i in 0..doc.tile_count() {
        index.insert(TileId(i), i).unwrap();
    }
    let retained = heights.ticket_from(TileId(1), &index, TileId(0)).unwrap();
    let stale = heights.ticket_from(TileId(2), &index, TileId(2)).unwrap();
    index.invalidate(2..doc.tile_count()).unwrap();
    let (mut index, _) = compact(index, 3);
    heights.measure_from(retained, 5, &index).unwrap();
    assert_eq!(
        heights.measure_from(stale, 5, &index),
        Err(LayoutError::StaleMeasurement)
    );
    assert!(
        index
            .recovery(&doc, TileId(1), usize::MAX, &mut budget(0, 1))
            .unwrap()
            .is_some()
    );
    let update = doc.append("more").unwrap();
    apply_layout_update(&doc, &update, &mut heights, &mut index).unwrap();
    assert_eq!(index.get(TileId(0)), Some(&0));
    index
        .insert_for_layout(&mut heights, TileId(0), 44)
        .unwrap();
    assert_eq!(
        heights.measure_from(retained, 7, &index),
        Err(LayoutError::StaleMeasurement)
    );
    let mut task = index.into_compaction();
    doc.append(" skipped").unwrap();
    let update = doc.append(" tail").unwrap();
    while task.advance(&mut budget(2, 2), &mut Vec::new()) != Completion::Covered {}
    let mut index = finish(task);
    assert_eq!(
        index.apply(&doc, &update),
        Err(LayoutError::OutOfOrderUpdate)
    );
    assert_eq!(
        index.recovery(&doc, TileId(0), 0, &mut budget(0, 1)),
        Err(LayoutError::OutOfOrderUpdate)
    );
    let other = Document::new();
    assert_eq!(
        index.recovery(&other, TileId(0), 0, &mut budget(0, 1)),
        Err(LayoutError::WrongDocument)
    );
    index.rebuild(&doc);
    assert!(index.is_empty());
}

#[test]
fn compaction_partial_drain_and_post_compaction_drain_return_each_payload_once() {
    let mut index = CheckpointIndex::new();
    for i in 0..128 {
        index.insert(TileId(i * 17), i).unwrap();
    }
    let mut cursor = 0;
    let mut out = Vec::new();
    index.drain_before(
        TileId(64 * 17),
        &mut cursor,
        &mut budget(100, 100),
        &mut out,
    );
    let old_drained = out.clone();
    index.invalidate(0..64 * 17).unwrap();
    let (mut index, stale) = compact(index, 31);
    out.extend(stale);
    out.sort_unstable();
    assert_eq!(out, (0..64).collect::<Vec<_>>());
    assert!(!old_drained.is_empty());
    cursor = 0; // arena positions changed
    out.clear();
    while index.drain_before(TileId(usize::MAX), &mut cursor, &mut budget(5, 5), &mut out)
        != Completion::Covered
    {}
    out.sort_unstable();
    assert_eq!(out, (64..128).collect::<Vec<_>>());
    let (index, stale) = compact(index, 3);
    assert!(index.is_empty());
    assert_eq!(index.node_count(), 1);
    assert!(stale.is_empty());
}

#[test]
fn compaction_drop_at_either_phase_owns_all_remaining_nonclone_payloads() {
    for steps in [0, 1, 15, 63, 64, 100, 200] {
        let drops = Rc::new(Cell::new(0));
        let mut index = CheckpointIndex::new();
        for value in 0..32 {
            index
                .insert(
                    TileId(value),
                    State {
                        value,
                        drops: drops.clone(),
                    },
                )
                .unwrap();
        }
        index.invalidate(16..32).unwrap();
        let mut task = index.into_compaction();
        let mut out = Vec::new();
        task.advance(&mut budget(steps, steps), &mut out);
        assert_eq!(drops.get(), 0);
        let returned = out.len();
        drop(task);
        assert_eq!(drops.get(), 32 - returned);
        drop(out);
        assert_eq!(drops.get(), 32);
    }
}

#[test]
fn compaction_all_live_keeps_identity_versions_and_serial_across_repeated_passes() {
    let mut index = CheckpointIndex::new();
    for i in (0..128).rev() {
        index.insert(TileId(i * 97), i).unwrap();
    }
    let mut foreign = CheckpointIndex::new();
    foreign.insert(TileId(0), 0).unwrap();
    let foreign_token = foreign.token(TileId(0)).unwrap();
    let old: Vec<_> = (0..128)
        .map(|i| index.token(TileId(i * 97)).unwrap())
        .collect();
    for _ in 0..3 {
        let (next, out) = compact(index, 1);
        index = next;
        assert!(out.is_empty());
        assert!(old.iter().all(|&token| index.validates(token)));
        assert!(!index.validates(foreign_token));
        for (i, &token) in old.iter().enumerate() {
            assert_eq!(index.token(TileId(i * 97)), Some(token));
            assert_eq!(index.get(TileId(i * 97)), Some(&i));
        }
    }
    // Migration visits keys in source order, not original insertion/version order.
    for i in 0..128 {
        index.insert(TileId(i * 97), i).unwrap();
    }
    assert!(old.iter().all(|&token| !index.validates(token)));
    let (empty, out) = compact(CheckpointIndex::<usize>::new(), 1);
    assert_eq!(empty.node_count(), 1);
    assert!(empty.is_empty() && out.is_empty());
}
