#[path = "support/storage.rs"]
mod storage;

use mdstream::*;
use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
};
use storage::*;

#[test]
fn storage_workloads_and_all_untimed_metrics_are_reproducible() {
    let a = measure(4096, 9);
    let b = measure(4096, 9);
    assert_eq!(a.len(), 54);
    let mut keys = BTreeSet::new();
    for (a, b) in a.iter().zip(&b) {
        assert_eq!((a.case, a.scale, a.phase), (b.case, b.scale, b.phase));
        assert_eq!(a.metrics, b.metrics);
        let _ = (a.elapsed, b.elapsed); // Timing observations are not test thresholds.
        for (metric, _) in &a.metrics {
            assert!(keys.insert((a.case, a.scale, a.phase, metric)));
        }
    }
    assert_eq!(
        fingerprint(source("plain", 4096).as_bytes()),
        13_182_226_609_708_950_309
    );
}

#[test]
fn compaction_measurements_match_capacity_budget_and_refill_models() {
    for count in [17, 128, 2048] {
        for stride in [1, 97] {
            for batch in [1, 31, 4096] {
                let samples =
                    checkpoint_compaction_samples::<Vec<u8>>("model", count, stride, batch);
                let [before, after, refill] = samples.as_slice() else {
                    panic!("phases")
                };
                let retained = count.div_ceil(16) as u64;
                assert_eq!(before.get("nodes"), trie_nodes(count, stride) as u64);
                assert_eq!(
                    after.get("nodes"),
                    trie_nodes(retained as usize, stride * 16) as u64
                );
                assert_eq!(after.get("retired_nodes"), before.get("nodes"));
                assert_eq!(
                    after.get("steps"),
                    after.get("retired_nodes") + after.get("visited_nodes")
                );
                assert_eq!(
                    after.get("frames"),
                    after.get("steps").div_ceil(batch as u64)
                );
                assert_eq!(after.get("moved_states"), retained);
                assert_eq!(after.get("valid_states"), retained);
                assert_eq!(after.get("returned_states"), count as u64 - retained);
                assert_eq!(
                    after.get("released_payload_bytes"),
                    (count as u64 - retained) * 256
                );
                assert_eq!(after.get("payload_heap_bytes"), retained * 256);
                assert_eq!(after.get("source_capacity_bytes"), 0);
                assert!(after.get("arena_capacity_bytes") < before.get("arena_capacity_bytes"));
                assert!(
                    after.get("mutation_visits")
                        <= after.get("steps") + retained * 4 * u64::from(usize::BITS)
                );
                assert_eq!(refill.get("nodes"), before.get("nodes"));
                assert_eq!(
                    refill.get("arena_capacity_bytes"),
                    before.get("arena_capacity_bytes")
                );
                assert_eq!(refill.get("valid_states"), count as u64);
                assert_eq!(
                    refill.get("checksum"),
                    (0..count).map(|i| u64::from(i as u8)).sum()
                );
            }
        }
    }
}

#[test]
fn reusable_semantic_display_and_cursor_queries_reduce_allocations_with_identical_work() {
    for size in [4096, 16_384, 65_536] {
        for case in ["plain", "dense", "containers"] {
            let samples = document_samples(case, size, 64);
            let sample = |phase| samples.iter().find(|s| s.phase == phase).unwrap();
            for (fresh, reused) in [
                ("window_queries", "semantic_reused"),
                ("display_fresh", "display_reused"),
                ("cursor_fresh", "cursor_reused"),
            ] {
                let (fresh, reused) = (sample(fresh), sample(reused));
                for metric in ["queries", "runs", "checksum", "scratch_peak_capacity_bytes"] {
                    assert_eq!(fresh.get(metric), reused.get(metric));
                }
                assert!(
                    reused.get("scratch_allocation_calls") < fresh.get("scratch_allocation_calls")
                );
                if reused.phase == "cursor_reused" {
                    assert_eq!(fresh.get("budget_bytes"), reused.get("budget_bytes"));
                    assert!(reused.get("scratch_allocation_calls") <= 64 * 6);
                } else {
                    assert_eq!(reused.get("scratch_allocation_calls"), 0);
                    assert!(reused.get("warm_allocation_calls") <= 6);
                    assert_eq!(
                        reused.get("warm_capacity_bytes"),
                        reused.get("scratch_peak_capacity_bytes")
                    );
                    assert_eq!(
                        fresh.get("tree_query_visits"),
                        reused.get("tree_query_visits")
                    );
                }
            }
        }
    }
}

#[test]
fn query_scratch_is_local_logarithmic_and_empty_queries_do_not_allocate() {
    for size in [4096, 65_536, 1_048_576] {
        for case in ["plain", "dense"] {
            let input = source(case, size);
            let mut doc = Document::new();
            doc.append(&input).unwrap();
            doc.finish();
            let before = doc.annotation_arena_stats();
            let mut empty = doc.runs_in(Span::new(0, 0)).unwrap();
            assert!(empty.next().is_none());
            assert_eq!(empty.scratch_stats(), ArenaStats::default());
            let depth = usize::BITS - input.len().leading_zeros();
            for span in [
                Span::new(size / 2, size / 2 + 256),
                Span::new(0, input.len()),
            ] {
                let mut runs = doc.runs_in(span).unwrap();
                let mut at = span.start;
                let mut last = runs.scratch_stats();
                while let Some(run) = runs.next() {
                    assert_eq!(run.source.start, at);
                    assert_eq!(run.text, &input[run.source.range()]);
                    at = run.source.end;
                    let stats = runs.scratch_stats();
                    assert!(stats.nodes <= depth as usize + 1);
                    assert!(stats.capacity_bytes >= last.capacity_bytes);
                    assert!(stats.allocation_calls >= last.allocation_calls);
                    last = stats;
                }
                assert_eq!(at, span.end);
                // A frame consists of Option<index>, Span and a fixed mark.
                // 128 is an architecture-independent upper bound for 32/64 bit.
                assert!(last.capacity_bytes <= 2 * (depth as usize + 4) * 128);
                assert!(last.allocation_calls <= u64::from(depth.ilog2()) + 2);
                assert!(runs.annotation_visits() <= 4 * span.len() as u64 + 4 * u64::from(depth));
            }
            assert_eq!(doc.annotation_arena_stats(), before);
        }
    }
}

#[test]
fn viewport_warm_reuse_removes_capacity_growth_with_identical_results() {
    for size in [1024, 65_536, 1_048_576] {
        let fresh = viewport_sample(size, 97, false);
        let reuse = viewport_sample(size, 97, true);
        assert_eq!(fresh.get("checksum"), reuse.get("checksum"));
        assert_eq!(fresh.get("buffer_growth_observations"), 97);
        assert_eq!(reuse.get("buffer_growth_observations"), 0);
        assert_eq!(
            fresh.get("buffer_peak_capacity_bytes"),
            reuse.get("buffer_peak_capacity_bytes")
        );
        assert_eq!(
            reuse.get("warm_buffer_bytes"),
            reuse.get("buffer_peak_capacity_bytes")
        );
    }
}

fn trie_nodes(count: usize, stride: usize) -> usize {
    let capacity = ((count - 1) * stride + 1).next_power_of_two();
    let mut intervals = BTreeSet::new();
    for at in (0..count).map(|i| i * stride) {
        let (mut lo, mut hi) = (0, capacity);
        loop {
            intervals.insert((lo, hi));
            if hi - lo == 1 {
                break;
            }
            let mid = lo + (hi - lo) / 2;
            if at < mid {
                hi = mid;
            } else {
                lo = mid;
            }
        }
    }
    intervals.len()
}

#[test]
fn cache_capacity_and_reclamation_match_independent_trie_and_payload_models() {
    for count in [1, 17, 128, 2048] {
        for stride in [1, 97] {
            for batch in [1, 31, 4096] {
                let samples = checkpoint_samples::<Vec<u8>>("model", count, stride, batch);
                let [filled, clear, drain, refill] = samples.as_slice() else {
                    panic!("phases")
                };
                let nodes = trie_nodes(count, stride) as u64;
                assert_eq!(filled.get("nodes"), nodes);
                let insert_visits: u64 = (0..count)
                    .map(|i| u64::from((i * stride + 1).next_power_of_two().ilog2()) + 1)
                    .sum();
                assert_eq!(filled.get("mutation_visits"), insert_visits);
                assert_eq!(filled.get("payload_heap_bytes"), count as u64 * 256);
                assert_eq!(clear.get("mutation_visits"), 1);
                assert_eq!(clear.get("valid_states"), 0);
                assert_eq!(
                    clear.get("payload_heap_bytes"),
                    filled.get("payload_heap_bytes")
                );
                assert_eq!(drain.get("scanned_nodes"), nodes);
                assert_eq!(drain.get("frames"), nodes.div_ceil(batch as u64));
                assert_eq!(drain.get("returned_states"), count as u64);
                assert_eq!(drain.get("mutation_visits"), count as u64); // root already invalid
                assert_eq!(drain.get("payload_heap_bytes"), 0);
                assert_eq!(drain.get("released_payload_bytes"), count as u64 * 256);
                assert_eq!(refill.get("valid_states"), count as u64);
                assert_eq!(
                    refill.get("mutation_visits"),
                    count as u64
                        * (u64::from(((count - 1) * stride + 1).next_power_of_two().ilog2()) + 1)
                );
                for phase in [clear, drain, refill] {
                    assert_eq!(
                        phase.get("arena_capacity_bytes"),
                        filled.get("arena_capacity_bytes")
                    );
                    assert_eq!(
                        phase.get("arena_allocation_calls"),
                        filled.get("arena_allocation_calls")
                    );
                }
            }
        }
    }
}

#[test]
fn inline_boxed_and_vec_payloads_preserve_queries_but_have_distinct_costs() {
    let inline = checkpoint_samples::<[u8; 256]>("inline", 128, 97, 7);
    let boxed = checkpoint_samples::<Box<[u8; 256]>>("boxed", 128, 97, 7);
    let vector = checkpoint_samples::<Vec<u8>>("vec", 128, 97, 7);
    for other in [&boxed, &vector] {
        assert_eq!(other[0].get("nodes"), inline[0].get("nodes"));
        assert_eq!(other[3].get("checksum"), inline[3].get("checksum"));
        assert_eq!(other[0].get("payload_heap_bytes"), 128 * 256);
        assert!(
            other[0].get("arena_capacity_bytes") + other[0].get("payload_heap_bytes")
                < inline[0].get("arena_capacity_bytes")
        );
    }
    assert_eq!(inline[0].get("payload_heap_bytes"), 0);
    assert_eq!(inline[2].get("released_payload_bytes"), 0);
    assert!(boxed[0].get("node_bytes") < vector[0].get("node_bytes"));
}

#[test]
fn partial_drains_and_revivals_match_map_without_premature_destructors() {
    struct State {
        value: usize,
        drops: Rc<Cell<usize>>,
    }
    impl Drop for State {
        fn drop(&mut self) {
            self.drops.set(self.drops.get() + 1);
        }
    }
    let drops = Rc::new(Cell::new(0));
    let mut cache = CheckpointIndex::new();
    let mut model = BTreeMap::new();
    let mut tokens = Vec::new();
    for i in 0..128 {
        cache
            .insert(
                TileId(i * 97),
                State {
                    value: i,
                    drops: drops.clone(),
                },
            )
            .unwrap();
        model.insert(i * 97, i);
        tokens.push(cache.token(TileId(i * 97)).unwrap());
    }
    let arena = cache.arena_stats();
    cache.invalidate(20 * 97..40 * 97).unwrap();
    model.retain(|tile, _| !(20 * 97..40 * 97).contains(tile));
    assert_eq!(drops.get(), 0);
    let (mut cursor, mut out, mut drained) = (0, Vec::new(), 0);
    let mut zero = Budget::new(WorkLimit::default());
    assert_eq!(
        cache.drain_before(TileId(64 * 97), &mut cursor, &mut zero, &mut out),
        Completion::BudgetExhausted
    );
    assert_eq!(cursor, 0);
    loop {
        let mut budget = Budget::new(WorkLimit {
            objects: 7,
            queries: 3,
            ..Default::default()
        });
        let before = cursor;
        let status = cache.drain_before(TileId(64 * 97), &mut cursor, &mut budget, &mut out);
        assert_eq!(cursor - before, budget.used().objects);
        assert!(budget.used().objects <= 3);
        assert_eq!(drops.get(), drained);
        for state in &out {
            model.remove(&(state.value * 97));
        }
        drained += out.len();
        out.clear();
        assert_eq!(drops.get(), drained);
        if status == Completion::Covered {
            break;
        }
    }
    assert_eq!(drained, 64);
    for at in 0..128 * 97 {
        assert_eq!(
            cache.previous(TileId(at)).map(|(id, s)| (id.0, s.value)),
            model.range(..=at).next_back().map(|(id, s)| (*id, *s))
        );
    }
    for (i, token) in tokens.into_iter().enumerate() {
        assert_eq!(cache.validates(token), i >= 64);
    }
    assert_eq!(cache.arena_stats(), arena);
    drop(cache);
    assert_eq!(drops.get(), 128);
}

#[test]
fn repeated_cache_clear_and_refill_reuses_nodes_and_rejects_old_tokens() {
    let mut cache = CheckpointIndex::new();
    for i in 0..1024 {
        cache.insert(TileId(i * 17), i).unwrap();
    }
    let arena = cache.arena_stats();
    for round in 0..32 {
        let token = cache.token(TileId(0)).unwrap();
        cache.clear();
        assert!(cache.previous(TileId(1024 * 17)).is_none());
        for i in 0..1024 {
            assert_eq!(cache.insert(TileId(i * 17), i).unwrap(), Some(i));
        }
        assert!(!cache.validates(token), "{round}");
        assert_eq!(cache.arena_stats(), arena);
        assert_eq!(cache.len(), 1024);
    }
}
