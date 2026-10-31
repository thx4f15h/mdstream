use mdtui3::{CheckpointIndex, Document, TileId};
use std::collections::BTreeMap;

#[test]
fn invalidating_and_reinserting_never_resurrects_stale_states() {
    let mut index = CheckpointIndex::new();
    for i in 0..4096 {
        index.insert(TileId(i), i).unwrap();
    }
    let nodes = index.node_count();
    let before = index.mutation_visits();
    index.invalidate(1..4096).unwrap();
    assert!(index.mutation_visits() - before < 100);
    assert_eq!(index.previous(TileId(4095)), Some((TileId(0), &0)));
    index.insert(TileId(3000), 99).unwrap();
    assert_eq!(index.previous(TileId(4095)), Some((TileId(3000), &99)));
    assert_eq!(index.previous(TileId(2999)), Some((TileId(0), &0)));
    assert!(index.get(TileId(2999)).is_none());
    for _ in 0..20 {
        let before = index.mutation_visits();
        index.clear();
        assert_eq!(index.mutation_visits() - before, 1);
        for i in 0..4096 {
            index.insert(TileId(i), i).unwrap();
        }
        assert_eq!(index.node_count(), nodes);
    }
}

#[test]
fn randomized_checkpoint_operations_match_ordered_map() {
    let mut index = CheckpointIndex::new();
    let mut model = BTreeMap::new();
    let mut seed = 99_u64;
    let mut random = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed as usize
    };
    for i in 0..10_000 {
        let a = random() % 4096;
        match random() % 20 {
            0 => {
                index.clear();
                model.clear();
            }
            1..=6 => {
                let b = random() % 4096;
                let range = a.min(b)..a.max(b);
                index.invalidate(range.clone()).unwrap();
                model.retain(|key, _| !range.contains(key));
            }
            _ => {
                index.insert(TileId(a), i).unwrap();
                model.insert(a, i);
            }
        }
        let key = random() % 5000;
        assert_eq!(
            index.previous(TileId(key)),
            model
                .range(..=key)
                .next_back()
                .map(|(&k, v)| (TileId(k), v))
        );
        assert_eq!(index.len(), model.len());
    }
}

#[test]
fn document_updates_preserve_only_valid_layout_prefix() {
    let mut doc = Document::new();
    doc.append(&"a ".repeat(5000)).unwrap();
    let mut index = CheckpointIndex::new();
    index.rebuild(&doc);
    for i in 0..doc.tile_count() {
        index.insert(TileId(i), i).unwrap();
    }
    let last = doc.tile_count() - 1;
    let update = doc.append("more").unwrap();
    index.apply(&doc, &update).unwrap();
    assert!(index.get(TileId(last)).is_none());
    assert!(index.get(TileId(last - 1)).is_some());
    let update = doc.append("\n===\n").unwrap();
    index.apply(&doc, &update).unwrap();
    assert!(index.is_empty());
}

#[test]
fn invalidation_does_not_run_payload_destructors() {
    use std::{cell::Cell, rc::Rc};
    struct State(Rc<Cell<usize>>);
    impl Drop for State {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }
    let drops = Rc::new(Cell::new(0));
    let mut index = CheckpointIndex::new();
    for i in 0..1000 {
        index.insert(TileId(i), State(drops.clone())).unwrap();
    }
    index.clear();
    assert_eq!(drops.get(), 0);
    drop(index);
    assert_eq!(drops.get(), 1000);
}

#[test]
fn appending_beyond_sparse_cache_preserves_early_checkpoints() {
    let mut doc = Document::new();
    doc.append(&"a".repeat(5000)).unwrap();
    let mut index = CheckpointIndex::new();
    index.rebuild(&doc);
    index.insert(TileId(0), "start").unwrap();
    let update = doc.append("more").unwrap();
    index.apply(&doc, &update).unwrap();
    assert_eq!(index.get(TileId(0)), Some(&"start"));
}
