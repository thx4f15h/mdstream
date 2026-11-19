use mdtui_tui::image::{
    bytes::Bytes,
    cache::{Cache, Match, Resource, ResourceId},
    worker::{Geometry, WorkerMetrics},
};
use mdtui3::{Budget, WorkLimit};
use std::sync::Arc;
fn bytes(input: &[u8]) -> Arc<Bytes> {
    let mut b = Bytes::default();
    b.append(input);
    Arc::new(b)
}
fn resource(uri: &[u8], hash: u64) -> Resource {
    Resource {
        id: ResourceId {
            slot: 0,
            generation: 0,
        },
        uri: bytes(uri),
        hash,
        encoded: bytes(b""),
        rgba: bytes(b""),
        original: Geometry::default(),
        canvas: Geometry::default(),
        cells: Geometry::default(),
        metrics: WorkerMetrics::default(),
        failed: false,
        touched: 0,
    }
}
fn tiny() -> Budget {
    Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 2,
        ..WorkLimit::default()
    })
}
fn full() -> Budget {
    Budget::new(WorkLimit {
        queries: 1024,
        objects: 1024,
        bytes: 4096,
        ..WorkLimit::default()
    })
}
fn find(cache: &Cache, query: &mut Match) -> Option<ResourceId> {
    for _ in 0..1000 {
        let mut b = tiny();
        match cache.find(query, &mut b) {
            Ok(v) => return v,
            Err(_) => assert_ne!(b.used(), WorkLimit::default()),
        }
    }
    panic!("lookup did not progress")
}
#[test]
fn root_native_generation_red_rejects_reused_slot_after_partial_prefix() {
    for replacement_hash in [u64::from(b'b'), u64::from(b'a')] {
        let mut cache = Cache::default();
        let first = cache.insert(resource(b"a0", u64::from(b'a'))).ok().unwrap();
        let mut query = Match::new(bytes(b"a0"), u64::from(b'a'));
        assert!(cache.find(&mut query, &mut tiny()).is_err());
        assert!(cache.find(&mut query, &mut tiny()).is_err());
        assert_eq!(cache.evict(&mut full()), Some(first));
        let replacement = cache
            .insert(resource(b"b0", replacement_hash))
            .ok()
            .unwrap();
        assert_eq!(first.slot, replacement.slot);
        assert_ne!(first.generation, replacement.generation);
        assert_eq!(
            find(&cache, &mut query),
            None,
            "a0 cannot hit b0, including a hash collision"
        );
    }
}
#[test]
fn final_commit_revalidates_generation_after_all_old_bytes_compared() {
    let mut cache = Cache::default();
    let first = cache.insert(resource(b"a0", 97)).ok().unwrap();
    let mut query = Match::new(bytes(b"a0"), 97);
    for _ in 0..3 {
        assert!(cache.find(&mut query, &mut tiny()).is_err())
    }
    assert_eq!(cache.evict(&mut full()), Some(first));
    cache.insert(resource(b"b0", 97)).ok().unwrap();
    assert_eq!(find(&cache, &mut query), None);
}
#[test]
fn unchanged_pinned_owner_survives_zero_micro_and_paid_eviction() {
    let mut cache = Cache::default();
    let first = cache.insert(resource(b"a0", 97)).ok().unwrap();
    let pin = cache.get(first).unwrap().uri.clone();
    let mut query = Match::new(bytes(b"a0"), 97);
    for _ in 0..3 {
        let mut zero = Budget::new(WorkLimit::default());
        assert!(cache.find(&mut query, &mut zero).is_err());
        assert_eq!(zero.used(), WorkLimit::default())
    }
    assert_eq!(cache.evict(&mut full()), None);
    assert_eq!(find(&cache, &mut query), Some(first));
    drop(pin);
    let mut evicted = None;
    for _ in 0..1000 {
        let mut b = tiny();
        evicted = cache.evict(&mut b);
        assert!(b.used().queries <= 1 && b.used().objects <= 1);
        if evicted.is_some() {
            break;
        }
    }
    assert_eq!(evicted, Some(first));
    assert!(cache.capacity().retiring > 0);
    for _ in 0..1000 {
        if cache.retire(&mut tiny()) {
            break;
        }
    }
    assert_eq!(cache.capacity().retiring, 0);
}
#[test]
fn touched_lru_candidate_is_revalidated_at_paid_commit() {
    let mut c = Cache::default();
    let a = c.insert(resource(b"a0", 97)).ok().unwrap();
    let b = c.insert(resource(b"b0", 98)).ok().unwrap();
    for _ in 0..64 {
        assert!(c.evict(&mut tiny()).is_none())
    }
    c.touch(a);
    assert!(
        c.evict(&mut tiny()).is_none(),
        "a newly touched candidate cannot inherit an old LRU choice"
    );
    assert_eq!(c.evict(&mut full()), Some(b));
    assert!(c.get(a).is_some());
}
#[test]
fn buffer_ownership_rejects_internal_aliases_and_retirement_directory_is_bounded() {
    let mut c = Cache::default();
    let mut bad = resource(b"a0", 97);
    bad.encoded = bad.uri.clone();
    assert!(c.insert(bad).is_err());
    assert_eq!(c.capacity().entries, 0);
    for _ in 0..64 {
        let id = c.insert(resource(b"a0", 97)).ok().unwrap();
        assert_eq!(c.evict(&mut full()), Some(id));
        assert_eq!(c.capacity().total(), c.known_total())
    }
    assert_eq!(c.capacity().retiring_buffers, 192);
    let current = c.insert(resource(b"b0", 98)).ok().unwrap();
    assert!(c.evict(&mut full()).is_none());
    assert!(c.get(current).is_some());
    let mut wrong = resource(b"c0", 99);
    wrong.uri = c.get(current).unwrap().uri.clone();
    assert!(c.insert(wrong).is_err());
    for _ in 0..2000 {
        if c.retire(&mut tiny()) {
            break;
        }
    }
    assert_eq!(c.capacity().retiring_buffers, 0);
    assert_eq!(c.capacity().total(), c.known_total());
    assert_eq!(c.evict(&mut full()), Some(current));
}
#[test]
fn stale_or_out_of_range_ids_do_not_mutate_a_reused_owner() {
    let mut c = Cache::default();
    let old = c.insert(resource(b"a0", 97)).ok().unwrap();
    c.evict(&mut full()).unwrap();
    let new = c.insert(resource(b"b0", 98)).ok().unwrap();
    let canvas = c.get(new).unwrap().canvas;
    for bad in [
        old,
        ResourceId {
            slot: 255,
            generation: 0,
        },
    ] {
        c.touch(bad);
        assert!(
            c.replace_canvas(
                bad,
                bytes(&[0; 4]),
                Geometry {
                    columns: 1,
                    rows: 1
                },
                Geometry {
                    columns: 1,
                    rows: 1
                },
                WorkerMetrics::default()
            )
            .is_err()
        );
        assert_eq!(c.get(new).unwrap().canvas, canvas)
    }
}
#[test]
fn variant_search_skip_still_revalidates_generation_and_full_uri() {
    let mut c = Cache::default();
    let a = c.insert(resource(b"same", 1)).ok().unwrap();
    let b = c.insert(resource(b"same", 1)).ok().unwrap();
    let mut q = Match::new(bytes(b"same"), 1);
    assert_eq!(find(&c, &mut q), Some(a));
    assert!(q.skip(a));
    assert_eq!(find(&c, &mut q), Some(b));
    assert!(q.skip(b));
    assert_eq!(find(&c, &mut q), None);
}
