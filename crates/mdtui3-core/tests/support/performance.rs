//! Shared release workloads and deterministic complexity measurements.
//! No allocator hooks, dependencies or production instrumentation.
use mdtui3::*;
use std::{
    hint::black_box,
    mem::size_of,
    time::{Duration, Instant},
};

pub const CASES: [&str; 17] = [
    "one_byte_appends",
    "one_byte_reused",
    "append_64",
    "append_4096",
    "append_whole",
    "sparse_markup",
    "dense_markup",
    "unicode_1",
    "unicode_257",
    "wide_sparse",
    "late_emphasis",
    "late_code",
    "late_math",
    "late_references",
    "deep_openers",
    "resize",
    "height_corrections",
];
pub const FRAME_LIMIT: WorkLimit = WorkLimit {
    queries: 256,
    tiles: 16,
    bytes: 2048,
    objects: 256,
    recovery: 256,
    dependencies: 8,
};

pub struct Workload {
    pub source: String,
    pub suffix: &'static str,
    pub chunk: usize,
    pub columns: usize,
    pub rows: usize,
}
pub fn workload(case: &str, size: usize) -> Workload {
    assert!(size >= 1024);
    let mut w = Workload {
        source: String::new(),
        suffix: "",
        chunk: 4096,
        columns: 0,
        rows: 0,
    };
    w.source = match case {
        "one_byte_appends" | "one_byte_reused" | "append_64" | "append_4096" | "append_whole" => {
            w.chunk = match case {
                "one_byte_appends" | "one_byte_reused" => 1,
                "append_64" => 64,
                "append_whole" => usize::MAX,
                _ => 4096,
            };
            "x".repeat(size)
        }
        "sparse_markup" => format!("**a** {}\n", "x".repeat(size)),
        "dense_markup" => "*a* **b** ~~c~~ [d](e) `f`\n".repeat(size.div_ceil(26)),
        "unicode_1" | "unicode_257" => {
            w.chunk = if case == "unicode_1" { 1 } else { 257 };
            "中文🙂e\u{301} 👩\u{200d}💻\r\n".repeat(size.div_ceil(27))
        }
        "wide_sparse" => {
            // Grow both dimensions: rectangular storage would be quadratic.
            w.columns = (size / 64).max(16);
            w.rows = size / 8;
            format!(
                "|{}\n|{}\n{}",
                "h|".repeat(w.columns),
                "-|".repeat(w.columns),
                "x\n".repeat(w.rows)
            )
        }
        "late_emphasis" | "late_code" | "late_math" => {
            let delimiter = match case {
                "late_emphasis" => "*",
                "late_code" => "`",
                _ => "$",
            };
            w.suffix = match case {
                "late_emphasis" => "*\n",
                "late_code" => "`\n",
                _ => "$\n",
            };
            format!("{delimiter}{}", "x".repeat(size))
        }
        "late_references" => {
            w.suffix = "\n\n[key]: /destination\n";
            "[key] ".repeat(size.div_ceil(6))
        }
        "deep_openers" => format!("*a {}z*\n", "_b ".repeat(size.div_ceil(3))),
        "resize" | "height_corrections" => "plain 中文🙂 **bold**\n\n".repeat(size.div_ceil(28)),
        _ => panic!("unknown case: {case}"),
    };
    w
}

pub struct Parsed {
    pub doc: Document,
    pub calls: usize,
    pub elapsed: Duration,
    pub closure_elapsed: Duration,
    // Late suffix + finish deltas, not whole-document counters.
    pub closure: [u64; 6],
}
fn closure_values(w: Work) -> [u64; 6] {
    [
        w.input_bytes,
        w.inline_bytes,
        w.delimiter_steps,
        w.tree_visits,
        w.label_bytes,
        w.math_steps,
    ]
}
pub fn parse(case: &str, w: &Workload) -> Parsed {
    // All cases include the same adapter; Unicode one-byte splits are real.
    let mut stream = Utf8Document::new();
    let mut update = Update::default();
    let mut calls = 0;
    let start = Instant::now();
    for chunk in w.source.as_bytes().chunks(w.chunk) {
        if case == "one_byte_appends" {
            black_box(stream.append(chunk).unwrap());
        } else {
            stream.append_into(chunk, &mut update).unwrap();
            black_box(&update);
        }
        calls += 1;
    }
    let before = closure_values(stream.document().work());
    let close = Instant::now();
    if !w.suffix.is_empty() {
        stream
            .append_into(w.suffix.as_bytes(), &mut update)
            .unwrap();
        calls += 1;
    }
    stream.finish_into(&mut update);
    black_box(&update);
    let closure_elapsed = close.elapsed();
    let elapsed = start.elapsed();
    let after = closure_values(stream.document().work());
    Parsed {
        doc: stream.into_document(),
        calls,
        elapsed,
        closure_elapsed,
        closure: std::array::from_fn(|i| after[i] - before[i]),
    }
}

#[derive(Default)]
struct ScalarLines {
    width: usize,
    column: usize,
    rows: u64,
    bytes: usize,
    scalars: usize,
    objects: usize,
    measurements: usize,
}
impl LayoutSink for ScalarLines {
    fn begin_tile(&mut self, _: TileId) {
        self.rows = 0;
    }
    fn text(&mut self, run: RenderRun<'_>) {
        self.bytes += run.text.len();
        if run.style.contains(Style::HIDDEN) {
            return;
        }
        for c in run.text.chars() {
            self.scalars += 1;
            if c == '\n' {
                self.column = 0;
                self.rows += 1;
            } else {
                self.column += 1;
                if self.column == self.width {
                    self.column = 0;
                    self.rows += 1;
                }
            }
        }
    }
    fn object(&mut self, _: LinkId, _: &LinkInfo) {
        self.objects += 1;
    }
    fn end_tile(&mut self, _: TileId) -> u64 {
        self.measurements += 1;
        self.rows
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FrameCounts {
    pub used: WorkLimit,
    pub bytes: usize,
    pub scalars: usize,
    pub objects: usize,
    pub measurements: usize,
    pub height_visits: u64,
    pub dependency_visits: u64,
    pub reset_visits: u64,
    pub correction_visits: u64,
    pub resets: usize,
    pub corrections: usize,
    pub exhausted: usize,
}
pub fn budget_values(w: WorkLimit) -> [usize; 6] {
    [
        w.queries,
        w.tiles,
        w.bytes,
        w.objects,
        w.recovery,
        w.dependencies,
    ]
}
fn frame(
    doc: &Document,
    heights: &mut HeightIndex,
    scratch: &mut Vec<VisibleTile>,
    case: &str,
    iteration: usize,
) -> (FrameCounts, Duration) {
    let start = Instant::now();
    let mut counts = FrameCounts::default();
    let mut budget = Budget::new(FRAME_LIMIT);
    let before = heights.mutation_visits();
    let deps = heights.dependency_mutation_visits();
    let width = if case == "resize" && iteration.is_multiple_of(2) {
        19
    } else {
        41
    };
    if case == "resize" {
        assert!(budget.charge(WorkLimit {
            queries: 1,
            ..Default::default()
        }));
        let capacity = heights.arena_stats();
        heights.reset(if width == 19 { 7 } else { 3 }).unwrap();
        counts.resets = 1;
        counts.reset_visits = heights.mutation_visits() - before;
        assert_eq!(
            heights.arena_stats(),
            capacity,
            "resize must reuse the arena"
        );
    }
    let middle = (doc.tile_count() / 2 + iteration % 8).min(doc.tile_count() - 1);
    if case == "height_corrections" {
        assert!(budget.charge(WorkLimit {
            queries: 1,
            tiles: 1,
            ..Default::default()
        }));
        let before = heights.mutation_visits();
        let ticket = heights.ticket(TileId(middle)).unwrap();
        heights
            .measure(ticket, [0, 3, 11, 1][iteration % 4])
            .unwrap();
        counts.corrections = 1;
        counts.correction_visits = heights.mutation_visits() - before;
    }
    // Timed frame includes reset/correction, both queries, bounded recovery,
    // semantic consumption and all resulting height writes.
    assert!(budget.charge(WorkLimit {
        queries: 1,
        ..Default::default()
    }));
    let top = heights.prefix_rows(middle).unwrap();
    black_box(heights.viewport_budgeted(top..top + 4, 0, &mut budget, scratch));
    let mut sink = ScalarLines {
        width,
        ..Default::default()
    };
    if let Some(first) = scratch.first()
        && budget.charge(WorkLimit {
            queries: 1,
            ..Default::default()
        })
    {
        let target = first.tile;
        let input = TileId(target.0.saturating_sub(1));
        let mut pass = LayoutPass::from_checkpoint(doc, heights, input, target, 4, 512)
            .unwrap()
            .unwrap();
        // Synthetic restored column zero, not terminal shaping or a cache hit.
        let status = pass.advance(doc, heights, &mut budget, &mut sink).unwrap();
        counts.exhausted = usize::from(status == Completion::BudgetExhausted);
    }
    if budget.charge(WorkLimit {
        queries: 1,
        ..Default::default()
    }) {
        let top = heights.prefix_rows(middle).unwrap();
        black_box(heights.viewport_budgeted(top..top + 4, 0, &mut budget, scratch));
    }
    let elapsed = start.elapsed();
    counts.used = budget.used();
    counts.bytes = sink.bytes;
    counts.scalars = sink.scalars;
    counts.objects = sink.objects;
    counts.measurements = sink.measurements;
    counts.height_visits = heights.mutation_visits() - before;
    counts.dependency_visits = heights.dependency_mutation_visits() - deps;
    (counts, elapsed)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Capacity {
    pub annotation: ArenaStats,
    pub container: ArenaStats,
    pub height: ArenaStats,
    pub dependency: ArenaStats,
    pub table_column_bytes: usize,
    pub table_cell_bytes: usize,
    pub physical_cells: usize,
    pub scratch_bytes: usize,
}
fn capacity(doc: &Document, heights: &HeightIndex, scratch: &Vec<VisibleTile>) -> Capacity {
    Capacity {
        annotation: doc.annotation_arena_stats(),
        container: doc.container_arena_stats(),
        height: heights.arena_stats(),
        dependency: heights.dependency_arena_stats(),
        table_column_bytes: doc
            .tables()
            .iter()
            .map(|t| t.columns.capacity() * size_of::<Column>())
            .sum(),
        table_cell_bytes: doc
            .tables()
            .iter()
            .flat_map(|t| t.rows())
            .map(|r| r.cells.capacity() * size_of::<TableCell>())
            .sum(),
        physical_cells: doc
            .tables()
            .iter()
            .flat_map(|t| t.rows())
            .map(|r| r.cells.len())
            .sum(),
        scratch_bytes: scratch.capacity() * size_of::<VisibleTile>(),
    }
}
pub struct Report {
    pub case: &'static str,
    pub requested: usize,
    pub bytes: usize,
    pub fingerprint: u64,
    pub chunk: usize,
    pub append_calls: usize,
    pub parse_time: Duration,
    pub closure_time: Duration,
    pub closure: [u64; 6],
    pub work: Work,
    pub tiles: usize,
    pub columns: usize,
    pub rows: usize,
    pub capacity: Capacity,
    pub frames: Vec<FrameCounts>,
    pub query_times: Vec<Duration>,
}
pub fn measure(case: &'static str, size: usize, samples: usize) -> Report {
    assert!(samples > 0);
    let w = workload(case, size);
    let parsed = parse(case, &w);
    let doc = &parsed.doc;
    let mut heights = HeightIndex::default();
    heights.rebuild(doc).unwrap();
    let mut scratch = Vec::with_capacity(FRAME_LIMIT.tiles);
    black_box(frame(doc, &mut heights, &mut scratch, case, 0)); // Excluded warmup.
    let mut frames = Vec::with_capacity(samples);
    let mut query_times = Vec::with_capacity(samples);
    for i in 1..=samples {
        let (counts, elapsed) = frame(doc, &mut heights, &mut scratch, case, i);
        frames.push(counts);
        query_times.push(elapsed);
    }
    Report {
        case,
        requested: size,
        bytes: doc.source().len(),
        fingerprint: doc.source().bytes().fold(0xcbf29ce484222325, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
        }),
        chunk: w.chunk,
        append_calls: parsed.calls,
        parse_time: parsed.elapsed,
        closure_time: parsed.closure_elapsed,
        closure: parsed.closure,
        work: doc.work(),
        tiles: doc.tile_count(),
        columns: w.columns,
        rows: w.rows,
        capacity: capacity(doc, &heights, &scratch),
        frames,
        query_times,
    }
}
/// Nearest-rank quantiles; no interpolation or time gates.
pub fn percentile(samples: &[Duration], percent: usize) -> Duration {
    assert!(!samples.is_empty() && (1..=100).contains(&percent));
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    sorted[(sorted.len() * percent).div_ceil(100) - 1]
}
impl Report {
    pub fn columns(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        macro_rules! value {
            ($name:expr, $v:expr) => {
                out.push(($name.to_string(), $v.to_string()));
            };
        }
        value!("schema", 2);
        value!("case", self.case);
        value!("requested_bytes", self.requested);
        value!("bytes", self.bytes);
        value!("source_fnv1a", self.fingerprint);
        value!("pointer_bits", usize::BITS);
        value!("debug_assertions", cfg!(debug_assertions));
        value!(
            "chunk_bytes_0_whole",
            if self.chunk == usize::MAX {
                0
            } else {
                self.chunk
            }
        );
        value!("append_calls", self.append_calls);
        value!("parse_ms", self.parse_time.as_secs_f64() * 1000.0);
        value!(
            "parse_mib_s",
            self.bytes as f64 / 1_048_576.0 / self.parse_time.as_secs_f64().max(f64::MIN_POSITIVE)
        );
        value!("closure_us", self.closure_time.as_secs_f64() * 1_000_000.0);
        macro_rules! work { ($($f:ident),+) => { $(value!(stringify!($f), self.work.$f);)+ }; }
        work!(
            input_bytes,
            classified_bytes,
            inline_bytes,
            delimiter_steps,
            label_bytes,
            tree_visits,
            table_bytes,
            table_header_bytes,
            table_candidate_bytes,
            atx_bytes,
            autolink_bytes,
            entity_bytes,
            container_bytes,
            container_steps,
            list_steps,
            heading_steps,
            math_steps,
            footnote_steps,
            footnote_label_bytes,
            paragraph_closures
        );
        for (name, n) in [
            "input_bytes",
            "inline_bytes",
            "delimiter_steps",
            "tree_visits",
            "label_bytes",
            "math_steps",
        ]
        .into_iter()
        .zip(self.closure)
        {
            value!(format!("closure_{name}"), n);
        }
        value!("tiles", self.tiles);
        value!("table_columns", self.columns);
        value!("table_body_rows", self.rows);
        for (name, stats) in [
            ("annotation", self.capacity.annotation),
            ("container", self.capacity.container),
            ("height", self.capacity.height),
            ("dependency", self.capacity.dependency),
        ] {
            value!(format!("{name}_nodes"), stats.nodes);
            value!(format!("{name}_capacity_bytes"), stats.capacity_bytes);
            value!(format!("{name}_allocation_calls"), stats.allocation_calls);
        }
        value!(
            "table_column_capacity_bytes",
            self.capacity.table_column_bytes
        );
        value!("table_cell_capacity_bytes", self.capacity.table_cell_bytes);
        value!("physical_cells", self.capacity.physical_cells);
        value!("scratch_capacity_bytes", self.capacity.scratch_bytes);
        value!("samples", self.frames.len());
        for (i, name) in [
            "queries",
            "tiles",
            "bytes",
            "objects",
            "recovery",
            "dependencies",
        ]
        .into_iter()
        .enumerate()
        {
            value!(
                format!("frame_total_{name}"),
                self.frames
                    .iter()
                    .map(|f| budget_values(f.used)[i])
                    .sum::<usize>()
            );
            value!(format!("frame_limit_{name}"), budget_values(FRAME_LIMIT)[i]);
        }
        macro_rules! sum { ($($f:ident),+) => { $(value!(concat!("frame_total_", stringify!($f)), self.frames.iter().map(|f| f.$f).sum::<u64>());)+ }; }
        sum!(
            height_visits,
            dependency_visits,
            reset_visits,
            correction_visits
        );
        macro_rules! sum_size { ($($f:ident),+) => { $(value!(concat!("sink_total_", stringify!($f)), self.frames.iter().map(|f| f.$f).sum::<usize>());)+ }; }
        sum_size!(
            bytes,
            scalars,
            objects,
            measurements,
            resets,
            corrections,
            exhausted
        );
        for p in [50, 95, 99] {
            value!(
                format!("query_p{p}_us"),
                percentile(&self.query_times, p).as_secs_f64() * 1_000_000.0
            );
        }
        value!(
            "query_max_us",
            self.query_times.iter().max().unwrap().as_secs_f64() * 1_000_000.0
        );
        out
    }
}
