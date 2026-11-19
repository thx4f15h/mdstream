//! Deterministic, safe capacity measurements. Timings are observations only.
use mdstream::*;
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

#[path = "storage_input.rs"]
mod input;
pub use input::{fingerprint, source};

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
    result.push(reusable_query_sample(
        &doc, case, size, queries, false, true,
    ));
    for reuse in [false, true] {
        result.push(reusable_query_sample(
            &doc, case, size, queries, true, reuse,
        ));
        result.push(cursor_sample(&doc, case, size, queries, reuse));
    }
    result
}

fn query_span(doc: &Document, i: usize) -> Span {
    let at = doc.source().len() / 2 + i % 8;
    Span::new(at, (at + 256).min(doc.source().len()))
}

// Same source/text/hidden checksum as the original semantic workload. Full
// styles, IDs, replacement mappings and budget costs have separate model tests.
fn query_checksum(text: &str, source: Span, style: Style) -> u64 {
    fingerprint(text.as_bytes()) ^ source.start as u64 ^ u64::from(style.contains(Style::HIDDEN))
}

pub fn reusable_query_sample(
    doc: &Document,
    case: &'static str,
    size: usize,
    queries: usize,
    display: bool,
    reuse: bool,
) -> Sample {
    let phase = match (display, reuse) {
        (false, false) => "semantic_fresh",
        (false, true) => "semantic_reused",
        (true, false) => "display_fresh",
        (true, true) => "display_reused",
    };
    let mut scratch = QueryScratch::default();
    let query = |span, scratch: &mut QueryScratch| {
        let (mut runs, mut checksum) = (0_u64, 0_u64);
        if display {
            let mut iter = doc.display_runs_in_with_scratch(span, scratch).unwrap();
            for run in iter.by_ref() {
                runs += 1;
                checksum = checksum.wrapping_add(query_checksum(run.text, run.source, run.style));
                black_box(run);
            }
            (
                runs,
                checksum,
                iter.annotation_visits(),
                iter.scratch_stats(),
            )
        } else {
            let mut iter = doc.runs_in_with_scratch(span, scratch).unwrap();
            for run in iter.by_ref() {
                runs += 1;
                checksum = checksum.wrapping_add(query_checksum(run.text, run.source, run.style));
                black_box(run);
            }
            (
                runs,
                checksum,
                iter.annotation_visits(),
                iter.scratch_stats(),
            )
        }
    };
    // Prewarm only the eight actual windows; do not traverse the full document.
    for i in 0..8 {
        black_box(query(query_span(doc, i), &mut scratch));
    }
    let warm = scratch.stats();
    let (mut runs, mut checksum, mut visits, mut allocations, mut peak) = (0, 0_u64, 0, 0, 0);
    let start = Instant::now();
    for i in 0..queries {
        if !reuse {
            scratch = QueryScratch::default();
        }
        let (count, sum, steps, stats) = query(query_span(doc, i), &mut scratch);
        runs += count;
        checksum = checksum.wrapping_add(sum);
        visits += steps;
        allocations += stats.allocation_calls;
        peak = peak.max(stats.capacity_bytes);
    }
    let mut sample = Sample::new(case, size, phase, start);
    sample.add("queries", queries);
    sample.add("scratch_allocation_calls", allocations);
    sample.add("scratch_peak_capacity_bytes", peak);
    sample.add("warm_allocation_calls", warm.allocation_calls);
    sample.add("warm_capacity_bytes", warm.capacity_bytes);
    sample.add("tree_query_visits", visits);
    sample.add("runs", runs);
    sample.add("checksum", checksum);
    sample
}

pub fn cursor_sample(
    doc: &Document,
    case: &'static str,
    size: usize,
    queries: usize,
    reuse: bool,
) -> Sample {
    let (mut allocations, mut peak, mut runs, mut checksum, mut bytes) = (0, 0, 0, 0_u64, 0);
    let start = Instant::now();
    for i in 0..queries {
        let span = query_span(doc, i);
        let mut cursor = doc.display_cursor(span).unwrap();
        while cursor.source_position() < span.end {
            // Recreating at every emitted run models the previous fresh-stack
            // next() behavior; both paths use exactly the same public budget API.
            if !reuse {
                cursor = doc
                    .display_cursor(Span::new(cursor.source_position(), span.end))
                    .unwrap();
            }
            let before = cursor.scratch_stats();
            let mut budget = Budget::new(WorkLimit {
                bytes: 66,
                queries: 1,
                objects: 1,
                ..Default::default()
            });
            let DisplayStep::Run(run) = cursor.next(doc, &mut budget).unwrap() else {
                panic!("progress")
            };
            checksum = checksum.wrapping_add(query_checksum(run.text, run.source, run.style));
            black_box(run);
            runs += 1;
            bytes += budget.used().bytes;
            allocations += cursor.scratch_stats().allocation_calls - before.allocation_calls;
            peak = peak.max(cursor.scratch_stats().capacity_bytes);
        }
    }
    let mut sample = Sample::new(
        case,
        size,
        if reuse {
            "cursor_reused"
        } else {
            "cursor_fresh"
        },
        start,
    );
    sample.add("queries", queries);
    sample.add("scratch_allocation_calls", allocations);
    sample.add("scratch_peak_capacity_bytes", peak);
    sample.add("runs", runs);
    sample.add("checksum", checksum);
    sample.add("budget_bytes", bytes);
    sample
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

/// Retain every sixteenth checkpoint, compact the historical tree, then refill
/// the same positions. All payload disposal is explicit and measured here.
pub fn checkpoint_compaction_samples<S: Payload>(
    case: &'static str,
    count: usize,
    stride: usize,
    batch: usize,
) -> Vec<Sample> {
    assert!(count > 0 && stride > 0 && batch > 0);
    let mut cache = CheckpointIndex::new();
    let start = Instant::now();
    let mut heap_bytes = 0;
    for i in 0..count {
        let state = S::make(i as u8);
        heap_bytes += state.heap_bytes();
        cache.insert(TileId(i * stride), state).ok().unwrap();
    }
    for i in (0..count).step_by(16) {
        cache
            .invalidate((i + 1) * stride..(i + 16).min(count) * stride)
            .unwrap();
    }
    let retained = count.div_ceil(16);
    let tokens: Vec<_> = (0..count)
        .step_by(16)
        .map(|i| cache.token(TileId(i * stride)).unwrap())
        .collect();
    let mut before = Sample::new(case, count, "compact_before", start);
    before.arena(cache.arena_stats());
    before.add("valid_states", cache.len());
    before.add("payload_heap_bytes", heap_bytes);
    before.add("stride", stride);
    let original_visits = cache.mutation_visits();
    let start = Instant::now();
    let mut task = cache.into_compaction();
    let (mut out, mut frames, mut returned, mut released, mut steps, mut peak, mut output_peak) =
        (Vec::new(), 0, 0, 0, 0, 0, 0);
    loop {
        let stats = task.stats();
        peak = peak.max(stats.source.capacity_bytes + stats.destination.capacity_bytes);
        let mut budget = Budget::new(WorkLimit {
            objects: batch,
            queries: batch,
            ..Default::default()
        });
        let status = task.advance(&mut budget, &mut out);
        frames += 1;
        steps += budget.used().objects;
        assert_eq!(budget.used().queries, budget.used().objects);
        assert!(budget.used().objects <= batch);
        assert_eq!(task.len(), retained);
        output_peak = output_peak.max(out.capacity() * size_of::<S>());
        returned += out.len();
        released += out.iter().map(Payload::heap_bytes).sum::<usize>();
        out.clear();
        if status == Completion::Covered {
            break;
        }
    }
    assert!(tokens.iter().all(|&token| task.validates(token)));
    let stats = task.stats();
    peak = peak.max(stats.source.capacity_bytes + stats.destination.capacity_bytes);
    assert_eq!(stats.source.capacity_bytes, 0);
    assert_eq!(stats.moved, retained);
    assert_eq!(returned, count - retained);
    let mut cache = match task.finish() {
        Ok(cache) => cache,
        Err(_) => panic!("completed pass"),
    };
    let mut after = Sample::new(case, count, "compacted", start);
    after.arena(cache.arena_stats());
    after.add("valid_states", cache.len());
    after.add("payload_heap_bytes", heap_bytes - released);
    after.add("returned_states", returned);
    after.add("released_payload_bytes", released);
    after.add("visited_nodes", stats.visited);
    after.add("moved_states", stats.moved);
    after.add("retired_nodes", stats.retired);
    after.add("steps", steps);
    after.add("frames", frames);
    after.add("batch", batch);
    after.add("mutation_visits", stats.mutation_visits - original_visits);
    after.add("source_capacity_bytes", stats.source.capacity_bytes);
    after.add("scratch_capacity_bytes", stats.scratch.capacity_bytes);
    after.add("scratch_allocation_calls", stats.scratch.allocation_calls);
    after.add("observed_arena_peak_capacity_bytes", peak);
    after.add("output_capacity_bytes", output_peak);
    let start = Instant::now();
    let visits = cache.mutation_visits();
    for i in 0..count {
        if i % 16 != 0 {
            cache
                .insert(TileId(i * stride), S::make(i as u8))
                .ok()
                .unwrap();
        }
    }
    let mut checksum = 0_u64;
    for i in 0..count {
        let (tile, state) = cache.previous(TileId(i * stride + stride - 1)).unwrap();
        assert_eq!(tile, TileId(i * stride));
        assert!(state.bytes().iter().all(|&b| b == i as u8));
        checksum += u64::from(state.bytes()[0]);
    }
    assert!(tokens.iter().all(|&token| cache.validates(token)));
    let mut refill = Sample::new(case, count, "compact_refilled", start);
    refill.arena(cache.arena_stats());
    refill.add("valid_states", cache.len());
    refill.add("payload_heap_bytes", heap_bytes);
    refill.add("mutation_visits", cache.mutation_visits() - visits);
    refill.add("checksum", checksum);
    vec![before, after, refill]
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
    samples.extend(checkpoint_compaction_samples::<[u8; 256]>(
        "compact_inline",
        count,
        1,
        31,
    ));
    samples.extend(checkpoint_compaction_samples::<Box<[u8; 256]>>(
        "compact_boxed",
        count,
        1,
        31,
    ));
    samples.extend(checkpoint_compaction_samples::<Vec<u8>>(
        "compact_vec",
        count,
        1,
        31,
    ));
    samples.extend(checkpoint_compaction_samples::<Vec<u8>>(
        "compact_sparse",
        count,
        97,
        31,
    ));
    samples
}
