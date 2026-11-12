use std::io;
use std::time::{Duration, Instant};

use mdtui_tui::cli::LinkMode;
use mdtui_tui::render::frame::{FrameCommit, FrameGeometry, FrameMetrics, FrameRuntime};
use mdtui_tui::stream::{
    FRAME_WORK, IngestResult, MAX_PRE_READ_BYTES, READ_CHUNK_BYTES, StreamEngine,
};
use mdtui3::{Budget, WorkLimit};

const WIDTH: u16 = 80;
const HEIGHT: u16 = 24;
const EIGHT_MIB: usize = 8 * 1024 * 1024;
const SIXTY_FOUR_MIB: usize = 64 * 1024 * 1024;
const PREFIX_UNIT: &str = "[hidden]: /url\n";
const PREFIX_BASE_BYTES: usize = 15 * 1024;
const FIRST_FRAME_SEGMENT_LIMIT: usize = 4_096;
const FIRST_FRAME_EVENT_LIMIT: usize = 4_096;
const FIRST_FRAME_LAYOUT_ROWS_LIMIT: u64 = 4_096;

fn geometry() -> FrameGeometry {
    FrameGeometry {
        rows: HEIGHT,
        cols: WIDTH,
        content_width: WIDTH,
    }
}

fn engine() -> StreamEngine {
    StreamEngine::new(FrameRuntime::new(geometry(), LinkMode::Default).unwrap())
}

fn render_due(engine: &mut StreamEngine, now: Duration) -> FrameCommit {
    for _ in 0..10_000 {
        let mut budget = Budget::new(FRAME_WORK);
        if let Some(commit) = engine
            .render_due(&mut io::sink(), &mut budget, now)
            .expect("scale frame")
        {
            return commit;
        }
    }
    panic!("scale frame did not complete");
}

fn drain_due(engine: &mut StreamEngine, now: Duration) {
    for _ in 0..10_000 {
        if !engine.due(now) {
            return;
        }
        render_due(engine, now);
    }
    panic!("scale frames did not settle");
}

fn finish_calibration(engine: &mut StreamEngine, now: Duration) {
    engine.finish_input(now).expect("finish scale stream");
    for _ in 0..1_000_000 {
        let mut budget = Budget::new(FRAME_WORK);
        if engine
            .advance_calibration(&mut budget)
            .expect("scale calibration")
            .is_some()
        {
            break;
        }
    }
    drain_due(engine, now);
}

fn work_within(used: WorkLimit, limit: WorkLimit) -> bool {
    used.queries <= limit.queries
        && used.tiles <= limit.tiles
        && used.bytes <= limit.bytes
        && used.objects <= limit.objects
        && used.recovery <= limit.recovery
        && used.dependencies <= limit.dependencies
}

fn assert_first_frame_bounds(metrics: FrameMetrics) {
    assert!(work_within(metrics.layout_budget, FRAME_WORK));
    assert!(metrics.planned_tiles <= FRAME_WORK.tiles);
    assert!(metrics.visible_tiles <= metrics.planned_tiles);
    assert!(metrics.recovery_tiles <= metrics.planned_tiles);
    assert!(metrics.diff_rows <= usize::from(HEIGHT));
    assert!(metrics.output_bytes <= usize::from(HEIGHT) * usize::from(WIDTH) * 8);
    assert!(metrics.layout.segments_consumed <= FIRST_FRAME_SEGMENT_LIMIT);
    assert!(metrics.layout.events_consumed <= FIRST_FRAME_EVENT_LIMIT);
    assert!(metrics.layout.rows <= FIRST_FRAME_LAYOUT_ROWS_LIMIT);
    assert_eq!(metrics.layout.document_queries_in_callbacks, 0);
}

fn large_source(bytes: usize) -> String {
    const BLOCK: &str = "paragraph abcdefghijklmnopqrstuvwxyz 0123456789\n\n";
    let mut source = String::with_capacity(bytes);
    source.push_str("# Scale top\n\n");
    while source.len() < bytes {
        source.push_str(BLOCK);
    }
    source.truncate(bytes);
    source
}

struct FirstFrameMeasurement {
    metrics: FrameMetrics,
    read_bytes: usize,
    source_len: usize,
}

fn measure_first_frame(source: &str, keep_running: bool) -> (StreamEngine, FirstFrameMeasurement) {
    let mut engine = engine();
    let chunks: Vec<_> = source.as_bytes().chunks(READ_CHUNK_BYTES).collect();
    assert_eq!(
        engine
            .ingest_data(chunks[0], Duration::ZERO)
            .expect("first data"),
        IngestResult::FirstFrame
    );
    assert!(!engine.document().is_finished());
    assert_eq!(engine.pre_read_batches(), 1);
    assert!(engine.pre_read_bytes() <= MAX_PRE_READ_BYTES);

    let commit = render_due(&mut engine, Duration::ZERO);
    assert!(commit.committed && commit.cleared);
    assert!(!engine.document().is_finished());
    assert!(engine.metrics().read_bytes <= MAX_PRE_READ_BYTES);
    assert!(engine.document().source().len() <= MAX_PRE_READ_BYTES);
    assert_first_frame_bounds(commit.metrics);

    let measurement = FirstFrameMeasurement {
        metrics: commit.metrics,
        read_bytes: engine.metrics().read_bytes,
        source_len: engine.document().source().len(),
    };
    if keep_running {
        for chunk in chunks.into_iter().skip(1) {
            engine
                .ingest_data(chunk, Duration::ZERO)
                .expect("scale stream data");
            drain_due(&mut engine, Duration::ZERO);
        }
        finish_calibration(&mut engine, Duration::from_secs(1));
    }
    (engine, measurement)
}

#[test]
fn eight_mib_stream_first_frame_is_bounded_before_finish() {
    let source = large_source(EIGHT_MIB);
    assert_eq!(source.len(), EIGHT_MIB);
    let started = Instant::now();
    let (engine, first) = measure_first_frame(&source, true);
    let elapsed = started.elapsed();

    assert!(first.read_bytes <= MAX_PRE_READ_BYTES);
    assert!(first.source_len <= MAX_PRE_READ_BYTES);
    assert!(engine.document().is_finished());
    assert_eq!(engine.document().source(), source);
    assert_eq!(engine.metrics().read_bytes, EIGHT_MIB);
    assert!(engine.metrics().updates >= 2);
    assert!(elapsed >= Duration::ZERO);
    println!(
        "scale8m first_read={} first_source={} first_metrics={:?} stream={:?} elapsed={elapsed:?}",
        first.read_bytes,
        first.source_len,
        first.metrics,
        engine.metrics()
    );
}

#[test]
fn unread_tail_does_not_change_the_first_frame_work() {
    let one_mib = large_source(1024 * 1024);
    let eight_mib = large_source(EIGHT_MIB);
    let (_, one) = measure_first_frame(&one_mib, false);
    let (_, eight) = measure_first_frame(&eight_mib, false);

    assert_eq!(one.read_bytes, eight.read_bytes);
    assert_eq!(one.source_len, eight.source_len);
    assert_eq!(one.metrics.layout_budget, eight.metrics.layout_budget);
    assert_eq!(one.metrics.planned_tiles, eight.metrics.planned_tiles);
    assert_eq!(one.metrics.visible_tiles, eight.metrics.visible_tiles);
    assert_eq!(one.metrics.recovery_tiles, eight.metrics.recovery_tiles);
    assert_eq!(one.metrics.diff_rows, eight.metrics.diff_rows);
    assert_eq!(one.metrics.output_bytes, eight.metrics.output_bytes);
    assert_eq!(one.metrics.layout, eight.metrics.layout);
}

fn hidden_prefix(multiplier: usize) -> String {
    let units = PREFIX_BASE_BYTES / PREFIX_UNIT.len() * multiplier;
    PREFIX_UNIT.repeat(units)
}

fn prefix_source(multiplier: usize) -> String {
    let mut source = hidden_prefix(multiplier);
    source.push_str("\n# Visible target\n\nvisible body\n");
    source
}

struct PrefixMeasurement {
    input_bytes: u64,
    metrics: FrameMetrics,
    output_bytes: usize,
}

fn measure_prefix(multiplier: usize) -> PrefixMeasurement {
    let source = prefix_source(multiplier);
    let mut runtime = FrameRuntime::from_finished_source(&source, geometry(), LinkMode::Default)
        .expect("prefix runtime");
    assert!(runtime.set_sidebar_visible(true).unwrap().visible);
    assert!(runtime.sidebar_selection_end());
    assert!(
        runtime
            .activate_sidebar_selection(&mut Budget::new(WorkLimit::unlimited()))
            .expect("activate visible heading")
    );

    let mut output = Vec::new();
    let commit = runtime
        .frame_to(&mut output, &mut Budget::new(FRAME_WORK))
        .expect("prefix frame");
    assert!(commit.committed);
    assert_first_frame_bounds(commit.metrics);
    assert_eq!(runtime.document().work().input_bytes, source.len() as u64);

    println!(
        "prefix x{multiplier} bytes={} input_bytes={} metrics={:?}",
        source.len(),
        runtime.document().work().input_bytes,
        commit.metrics
    );
    PrefixMeasurement {
        input_bytes: runtime.document().work().input_bytes,
        metrics: commit.metrics,
        output_bytes: output.len(),
    }
}

#[test]
fn prefix_scaling_keeps_parser_linear_and_tui_work_bounded() {
    let measurements = [128, 256, 512].map(measure_prefix);
    for pair in measurements.windows(2) {
        assert!(pair[1].input_bytes > pair[0].input_bytes);
    }

    let baseline = &measurements[0];
    for measurement in &measurements[1..] {
        assert_eq!(
            measurement.metrics.planned_tiles,
            baseline.metrics.planned_tiles
        );
        assert_eq!(
            measurement.metrics.visible_tiles,
            baseline.metrics.visible_tiles
        );
        assert_eq!(
            measurement.metrics.recovery_tiles,
            baseline.metrics.recovery_tiles
        );
        assert_eq!(measurement.metrics.diff_rows, baseline.metrics.diff_rows);
        assert_eq!(measurement.output_bytes, baseline.output_bytes);
        assert_eq!(measurement.metrics.layout, baseline.metrics.layout);
        assert_eq!(
            measurement.metrics.layout_budget,
            baseline.metrics.layout_budget
        );
    }
}

#[test]
#[ignore = "manual 64 MiB scale acceptance; not a CI wall-clock assertion"]
fn manual_sixty_four_mib_stream_first_frame_and_final_metrics() {
    let source = large_source(SIXTY_FOUR_MIB);
    assert_eq!(source.len(), SIXTY_FOUR_MIB);
    let started = Instant::now();
    let (engine, first) = measure_first_frame(&source, true);
    let elapsed = started.elapsed();
    assert_eq!(engine.document().source().len(), SIXTY_FOUR_MIB);
    assert_eq!(engine.metrics().read_bytes, SIXTY_FOUR_MIB);
    assert!(first.read_bytes <= MAX_PRE_READ_BYTES);
    println!(
        "scale64m first_read={} first_source={} first_metrics={:?} stream={:?} elapsed={elapsed:?}",
        first.read_bytes,
        first.source_len,
        first.metrics,
        engine.metrics()
    );
}
