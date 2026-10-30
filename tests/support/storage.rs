//! Deterministic, safe capacity measurements. Timings are observations only.
use mdtui3::*;
use std::{
    hint::black_box,
    mem::size_of,
    time::{Duration, Instant},
};

#[derive(Debug)]
pub struct Sample {
    pub case: &'static str,
    pub scale: usize,
    pub phase: &'static str,
    pub metrics: Vec<(&'static str, u64)>,
    pub elapsed: Duration,
}
impl Sample {
    fn new(case: &'static str, scale: usize, phase: &'static str, start: Instant) -> Self {
        Self {
            case,
            scale,
            phase,
            metrics: Vec::new(),
            elapsed: start.elapsed(),
        }
    }
    fn add(&mut self, key: &'static str, value: impl TryInto<u64>) {
        self.metrics
            .push((key, value.try_into().ok().expect("counter fits u64")));
    }
    fn arena(&mut self, stats: ArenaStats) {
        self.add("nodes", stats.nodes);
        self.add("arena_capacity_bytes", stats.capacity_bytes);
        self.add("arena_allocation_calls", stats.allocation_calls);
    }
    pub fn get(&self, key: &str) -> u64 {
        self.metrics
            .iter()
            .find(|(k, _)| *k == key)
            .expect("metric")
            .1
    }
}

pub fn fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
    })
}

pub fn source(case: &str, size: usize) -> String {
    match case {
        "plain" => "x".repeat(size),
        "dense" => "*a* **b** ~~c~~ [d](e) `f`\n".repeat(size.div_ceil(26)),
        "containers" => "> - **bold** text\n\n".repeat(size.div_ceil(19)),
        _ => panic!("unknown storage case"),
    }
}

pub fn document_samples(case: &'static str, size: usize, queries: usize) -> Vec<Sample> {
    let input = source(case, size);
    let mut doc = Document::new();
    let node_bytes = doc.annotation_arena_stats().capacity_bytes;
    let mut update = Update::default();
    let start = Instant::now();
    for chunk in input.as_bytes().chunks(4096) {
        doc.append_into(std::str::from_utf8(chunk).unwrap(), &mut update)
            .unwrap();
    }
    doc.finish_into(&mut update);
    let mut parse = Sample::new(case, size, "parse", start);
    parse.arena(doc.annotation_arena_stats());
    parse.add("source_bytes", input.len());
    parse.add("source_fnv1a", fingerprint(input.as_bytes()));
    parse.add("annotation_node_bytes", node_bytes);
    parse.add("container_nodes", doc.container_arena_stats().nodes);
    parse.add(
        "container_capacity_bytes",
        doc.container_arena_stats().capacity_bytes,
    );
    parse.add(
        "container_allocation_calls",
        doc.container_arena_stats().allocation_calls,
    );
    parse.add("tree_write_visits", doc.work().tree_visits);
    parse.add("inline_bytes", doc.work().inline_bytes);
    let arena = doc.annotation_arena_stats();
    let mut result = vec![parse];
    for phase in ["window_queries", "whole_query"] {
        let count = if phase == "whole_query" { 1 } else { queries };
        let (mut allocations, mut peak, mut visits, mut runs, mut checksum) = (0, 0, 0, 0, 0_u64);
        let start = Instant::now();
        for i in 0..count {
            let span = if phase == "whole_query" {
                Span::new(0, input.len())
            } else {
                let at = input.len() / 2 + i % 8;
                Span::new(at, (at + 256).min(input.len()))
            };
            let mut iter = doc.runs_in(span).unwrap();
            for run in iter.by_ref() {
                checksum = checksum.wrapping_add(
                    fingerprint(run.text.as_bytes())
                        ^ run.source.start as u64
                        ^ u64::from(run.style.contains(Style::HIDDEN)),
                );
                runs += 1;
                black_box(run);
            }
            allocations += iter.scratch_stats().allocation_calls;
            peak = peak.max(iter.scratch_stats().capacity_bytes);
            visits += iter.annotation_visits();
        }
        let mut sample = Sample::new(case, size, phase, start);
        sample.add("queries", count);
        sample.add("scratch_allocation_calls", allocations);
        sample.add("scratch_peak_capacity_bytes", peak);
        sample.add("tree_query_visits", visits);
        sample.add("runs", runs);
        sample.add("checksum", checksum);
        assert_eq!(doc.annotation_arena_stats(), arena);
        result.push(sample);
    }
    result
}

pub fn viewport_sample(size: usize, queries: usize, reuse: bool) -> Sample {
    let mut heights = HeightIndex::new(1).unwrap();
    heights.grow_to(size).unwrap();
    let arena = heights.arena_stats();
    let mut scratch = Vec::new();
    heights.viewport_into(0..4, 0, 4, &mut scratch);
    let warm_bytes = scratch.capacity() * size_of::<VisibleTile>();
    let (mut growths, mut peak, mut checksum) = (0, 0, 0_u64);
    let start = Instant::now();
    for i in 0..queries {
        if !reuse {
            scratch = Vec::new();
        }
        let before = scratch.capacity();
        let at = (size / 2 + i % 8) as u128;
        heights.viewport_into(at..at + 4, 0, 4, &mut scratch);
        growths += usize::from(scratch.capacity() > before);
        peak = peak.max(scratch.capacity() * size_of::<VisibleTile>());
        for tile in &scratch {
            checksum += tile.tile.0 as u64;
        }
        black_box(&scratch);
    }
    let mut sample = Sample::new(
        if reuse {
            "viewport_reused"
        } else {
            "viewport_fresh"
        },
        size,
        "queries",
        start,
    );
    sample.arena(heights.arena_stats());
    sample.add("queries", queries);
    sample.add("warm_buffer_bytes", warm_bytes);
    sample.add("buffer_peak_capacity_bytes", peak);
    // Capacity observed at API boundaries, not an allocator-wide count.
    sample.add("buffer_growth_observations", growths);
    sample.add("checksum", checksum);
    assert_eq!(heights.arena_stats(), arena);
    sample
}

pub trait Payload {
    fn make(value: u8) -> Self;
    fn bytes(&self) -> &[u8];
    fn heap_bytes(&self) -> usize;
}
impl Payload for [u8; 256] {
    fn make(value: u8) -> Self {
        [value; 256]
    }
    fn bytes(&self) -> &[u8] {
        self
    }
    fn heap_bytes(&self) -> usize {
        0
    }
}
impl Payload for Box<[u8; 256]> {
    fn make(value: u8) -> Self {
        Box::new([value; 256])
    }
    fn bytes(&self) -> &[u8] {
        self.as_ref()
    }
    fn heap_bytes(&self) -> usize {
        256
    }
}
impl Payload for Vec<u8> {
    fn make(value: u8) -> Self {
        vec![value; 256]
    }
    fn bytes(&self) -> &[u8] {
        self
    }
    fn heap_bytes(&self) -> usize {
        self.capacity()
    }
}

pub fn checkpoint_samples<S: Payload>(
    case: &'static str,
    count: usize,
    stride: usize,
    batch: usize,
) -> Vec<Sample> {
    assert!(count > 0 && stride > 0 && batch > 0);
    let mut cache = CheckpointIndex::new();
    let node_bytes = cache.arena_stats().capacity_bytes;
    let mut heap_bytes = 0;
    let start = Instant::now();
    for i in 0..count {
        let payload = S::make(i as u8);
        heap_bytes += payload.heap_bytes();
        assert!(cache.insert(TileId(i * stride), payload).unwrap().is_none());
    }
    let mut filled = Sample::new(case, count, "filled", start);
    filled.arena(cache.arena_stats());
    filled.add("node_bytes", node_bytes);
    filled.add("payload_heap_bytes", heap_bytes);
    filled.add(
        "payload_allocation_calls",
        if heap_bytes > 0 { count } else { 0 },
    );
    filled.add("valid_states", cache.len());
    filled.add("stride", stride);
    filled.add("mutation_visits", cache.mutation_visits());
    let arena = cache.arena_stats();
    let old_token = cache.token(TileId(0)).unwrap();
    let start = Instant::now();
    let before = cache.mutation_visits();
    cache.clear();
    let mut cleared = Sample::new(case, count, "cleared", start);
    cleared.arena(cache.arena_stats());
    cleared.add("payload_heap_bytes", heap_bytes);
    cleared.add("valid_states", cache.len());
    cleared.add("mutation_visits", cache.mutation_visits() - before);
    assert!(!cache.validates(old_token));
    assert!(cache.previous(TileId(count * stride)).is_none());
    let (mut cursor, mut out, mut frames, mut scanned, mut released, mut returned, mut peak) =
        (0, Vec::new(), 0, 0, 0, 0, 0);
    let before = cache.mutation_visits();
    let start = Instant::now();
    loop {
        let mut budget = Budget::new(WorkLimit {
            objects: batch,
            queries: batch,
            ..Default::default()
        });
        let status = cache.drain_before(TileId(count * stride), &mut cursor, &mut budget, &mut out);
        frames += 1;
        scanned += budget.used().objects;
        assert_eq!(budget.used().objects, budget.used().queries);
        assert!(budget.used().objects <= batch);
        peak = peak.max(out.capacity() * size_of::<S>());
        for payload in &out {
            released += payload.heap_bytes();
        }
        returned += out.len();
        // Explicit caller-chosen disposal, included in this phase's timing.
        out.clear();
        if status == Completion::Covered {
            break;
        }
    }
    let mut drained = Sample::new(case, count, "drained", start);
    drained.arena(cache.arena_stats());
    drained.add("payload_heap_bytes", heap_bytes - released);
    drained.add("released_payload_bytes", released);
    drained.add("returned_states", returned);
    drained.add("scanned_nodes", scanned);
    drained.add("frames", frames);
    drained.add("batch", batch);
    drained.add("output_capacity_bytes", peak);
    drained.add("mutation_visits", cache.mutation_visits() - before);
    assert_eq!(cache.arena_stats(), arena);
    assert_eq!(returned, count);
    assert_eq!(released, heap_bytes);
    let before = cache.mutation_visits();
    let start = Instant::now();
    for i in 0..count {
        assert!(
            cache
                .insert(TileId(i * stride), S::make(i as u8))
                .unwrap()
                .is_none()
        );
    }
    let mut checksum = 0_u64;
    for i in 0..count {
        let (tile, payload) = cache.previous(TileId(i * stride + stride - 1)).unwrap();
        assert_eq!(tile, TileId(i * stride));
        assert!(payload.bytes().iter().all(|b| *b == i as u8));
        checksum += u64::from(payload.bytes()[0]);
    }
    let mut refill = Sample::new(case, count, "refilled", start);
    refill.arena(cache.arena_stats());
    refill.add("payload_heap_bytes", heap_bytes);
    refill.add("valid_states", cache.len());
    refill.add("mutation_visits", cache.mutation_visits() - before);
    refill.add("checksum", checksum);
    assert_eq!(cache.arena_stats(), arena);
    assert!(!cache.validates(old_token));
    vec![filled, cleared, drained, refill]
}

pub fn measure(size: usize, queries: usize) -> Vec<Sample> {
    let mut samples = Vec::new();
    for case in ["plain", "dense", "containers"] {
        samples.extend(document_samples(case, size, queries));
    }
    for reuse in [false, true] {
        samples.push(viewport_sample(size, queries, reuse));
    }
    let count = size / 32;
    samples.extend(checkpoint_samples::<[u8; 256]>(
        "checkpoint_inline",
        count,
        1,
        31,
    ));
    samples.extend(checkpoint_samples::<Box<[u8; 256]>>(
        "checkpoint_boxed",
        count,
        1,
        31,
    ));
    samples.extend(checkpoint_samples::<Vec<u8>>(
        "checkpoint_vec",
        count,
        1,
        31,
    ));
    samples.extend(checkpoint_samples::<Vec<u8>>(
        "checkpoint_sparse",
        count,
        97,
        31,
    ));
    samples
}
