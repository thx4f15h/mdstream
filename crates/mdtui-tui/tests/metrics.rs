mod support;

use std::io;
use std::time::{Duration, Instant};

use mdtui_tui::cli::LinkMode;
use mdtui_tui::render::frame::{FrameCommit, FrameGeometry, FrameRuntime};
use mdtui_tui::stream::{
    FRAME_WORK, IngestResult, MAX_BATCH_DELAY, MAX_PRE_READ_BYTES, PRE_READ_BATCH_BYTES,
    READ_CHUNK_BYTES, StreamEngine, StreamMetrics,
};
use mdtui3::{Budget, WorkLimit};
use support::terminal::{FrameOutcome, Vt100Writer};

const WIDTH: u16 = 80;
const HEIGHT: u16 = 24;

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

fn commit_due(engine: &mut StreamEngine, writer: &mut Vt100Writer, now: Duration) -> FrameCommit {
    writer.begin_frame();
    for _ in 0..10_000 {
        let mut budget = Budget::new(FRAME_WORK);
        match engine.render_due(writer, &mut budget, now) {
            Ok(Some(commit)) if commit.committed => {
                writer.finish_frame(FrameOutcome::Committed);
                return commit;
            }
            Ok(_) => {}
            Err(error) => {
                writer.finish_frame(FrameOutcome::Failed);
                panic!("unexpected frame error: {error}");
            }
        }
    }
    writer.finish_frame(FrameOutcome::Failed);
    panic!("frame did not commit");
}

fn finish_and_commit(
    engine: &mut StreamEngine,
    writer: &mut Vt100Writer,
    now: Duration,
) -> FrameCommit {
    assert!(engine.finish_input(now).expect("stream must finish"));
    for _ in 0..100_000 {
        let mut budget = Budget::new(FRAME_WORK);
        if engine
            .advance_calibration(&mut budget)
            .expect("calibration must succeed")
            .is_some()
        {
            return commit_due(engine, writer, now);
        }
    }
    panic!("calibration did not complete");
}

fn frame_write_bytes(writer: &Vt100Writer, frame: usize) -> usize {
    writer
        .frame_writes(frame)
        .iter()
        .map(|write| write.bytes.len())
        .sum()
}

fn work_within(used: WorkLimit, limit: WorkLimit) -> bool {
    used.queries <= limit.queries
        && used.tiles <= limit.tiles
        && used.bytes <= limit.bytes
        && used.objects <= limit.objects
        && used.recovery <= limit.recovery
        && used.dependencies <= limit.dependencies
}

fn work_is_zero(work: WorkLimit) -> bool {
    work == WorkLimit::default()
}

fn accumulate_work(total: &mut WorkLimit, used: WorkLimit) {
    total.queries = total.queries.saturating_add(used.queries);
    total.tiles = total.tiles.saturating_add(used.tiles);
    total.bytes = total.bytes.saturating_add(used.bytes);
    total.objects = total.objects.saturating_add(used.objects);
    total.recovery = total.recovery.saturating_add(used.recovery);
    total.dependencies = total.dependencies.saturating_add(used.dependencies);
}

fn assert_frame_metrics(engine: &StreamEngine, writer: &Vt100Writer, commit: &FrameCommit) {
    let frame = writer.frames().len() - 1;
    let metrics = commit.metrics;
    let stream = engine.metrics();
    assert_eq!(stream.last_frame, Some(metrics));
    assert_eq!(commit.bytes, metrics.output_bytes);
    assert_eq!(frame_write_bytes(writer, frame), metrics.output_bytes);
    assert!(metrics.body_bytes <= metrics.output_bytes);
    assert!(metrics.diff_rows <= usize::from(HEIGHT));
    assert!(metrics.recovery_tiles <= metrics.planned_tiles);
    assert!(metrics.recovery_tiles <= metrics.visible_tiles);
    assert!(work_within(metrics.layout_budget, FRAME_WORK));
    let lower = Duration::ZERO;
    assert!(metrics.frame_elapsed >= lower);
}

fn hidden_batch() -> Vec<u8> {
    let mut source = String::new();
    let mut index = 0_usize;
    loop {
        let definition = format!("[hidden{index:05}]: /url\n");
        if source.len() + definition.len() > PRE_READ_BATCH_BYTES {
            break;
        }
        source.push_str(&definition);
        index += 1;
    }
    source.push_str(&"\n".repeat(PRE_READ_BATCH_BYTES - source.len()));
    assert_eq!(source.len(), PRE_READ_BATCH_BYTES);
    source.into_bytes()
}

#[test]
fn stream_metrics_count_read_bytes_and_nonempty_updates_exactly() {
    let source = b"# Metrics\n\nbody\n";
    let tail = b"tail\n";
    let mut engine = engine();
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);

    assert_eq!(
        engine.ingest_data(source, Duration::ZERO).unwrap(),
        IngestResult::FirstFrame
    );
    assert_eq!(engine.metrics().read_bytes, source.len());
    assert_eq!(engine.metrics().updates, 1);
    assert_eq!(engine.metrics().frames, 0);
    let first = commit_due(&mut engine, &mut writer, Duration::ZERO);
    assert_frame_metrics(&engine, &writer, &first);
    assert!(!work_is_zero(first.metrics.layout_budget));

    let after_first = engine.metrics();
    assert_eq!(
        engine.ingest_data(b"", Duration::ZERO).unwrap(),
        IngestResult::Empty
    );
    assert_eq!(engine.metrics(), after_first);

    let update_at = Duration::from_millis(100);
    assert_eq!(
        engine.ingest_data(tail, update_at).unwrap(),
        IngestResult::Applied
    );
    assert_eq!(
        engine.metrics().read_bytes,
        source.len().saturating_add(tail.len())
    );
    assert_eq!(engine.metrics().updates, 2);
    assert_eq!(engine.metrics().frames, 1);
    let render_at = update_at + MAX_BATCH_DELAY;
    assert!(engine.due(render_at));
    let second = commit_due(&mut engine, &mut writer, render_at);
    assert_frame_metrics(&engine, &writer, &second);

    let before_finish = engine.metrics();
    assert!(engine.finish_input(render_at).unwrap());
    assert_eq!(engine.metrics().read_bytes, before_finish.read_bytes);
    assert_eq!(engine.metrics().updates, 3);
    assert!(!engine.finish_input(render_at).unwrap());
    assert_eq!(engine.metrics().updates, 3);
    assert_eq!(engine.metrics().read_bytes, before_finish.read_bytes);

    for _ in 0..100_000 {
        let mut budget = Budget::new(FRAME_WORK);
        if engine
            .advance_calibration(&mut budget)
            .expect("calibration must succeed")
            .is_some()
        {
            break;
        }
    }
    let final_commit = commit_due(&mut engine, &mut writer, render_at);
    assert_frame_metrics(&engine, &writer, &final_commit);
    assert_eq!(engine.metrics().read_bytes, source.len() + tail.len());
    assert_eq!(engine.metrics().updates, 3);
    assert_eq!(engine.metrics().frames, 3);
}

#[test]
fn sixty_four_k_pre_read_counts_each_byte_once_and_drains_the_tail() {
    let mut source = String::from("# Pre-read\n\n");
    while source.len() < READ_CHUNK_BYTES {
        source.push_str("line\n\n");
    }
    source.truncate(READ_CHUNK_BYTES);
    if !source.ends_with('\n') {
        source.pop();
        source.push('\n');
    }
    let source = source.into_bytes();
    assert_eq!(source.len(), READ_CHUNK_BYTES);

    let mut engine = engine();
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    assert_eq!(
        engine.ingest_data(&source, Duration::ZERO).unwrap(),
        IngestResult::FirstFrame
    );
    assert_eq!(engine.metrics().read_bytes, READ_CHUNK_BYTES);
    assert_eq!(engine.metrics().updates, 1);
    assert_eq!(engine.pre_read_bytes(), PRE_READ_BATCH_BYTES);
    assert_eq!(
        engine.queued_bytes(),
        READ_CHUNK_BYTES - PRE_READ_BATCH_BYTES
    );

    let first = commit_due(&mut engine, &mut writer, Duration::ZERO);
    assert_frame_metrics(&engine, &writer, &first);
    assert_eq!(engine.metrics().read_bytes, READ_CHUNK_BYTES);
    assert_eq!(engine.metrics().updates, 2);
    assert_eq!(engine.queued_bytes(), 0);
    assert_eq!(engine.document().source().len(), READ_CHUNK_BYTES);
}

#[test]
fn two_hundred_eighty_eight_k_pre_read_is_counted_once() {
    let chunk = hidden_batch();
    let mut engine = engine();
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    for batch in 0..9 {
        let result = engine.ingest_data(&chunk, Duration::ZERO).unwrap();
        if batch == 8 {
            assert_eq!(result, IngestResult::FirstFrame);
        } else {
            assert_eq!(result, IngestResult::PreReading);
        }
    }
    assert_eq!(engine.metrics().read_bytes, MAX_PRE_READ_BYTES);
    assert_eq!(engine.metrics().updates, 9);
    assert_eq!(engine.pre_read_bytes(), MAX_PRE_READ_BYTES);
    assert_eq!(engine.pre_read_batches(), 9);
    assert!(engine.due(Duration::ZERO));

    let commit = commit_due(&mut engine, &mut writer, Duration::ZERO);
    assert_frame_metrics(&engine, &writer, &commit);
    assert_eq!(commit.metrics.layout_budget, WorkLimit::default());
    assert_eq!(engine.metrics().read_bytes, MAX_PRE_READ_BYTES);
    assert_eq!(engine.document().source().len(), MAX_PRE_READ_BYTES);
}

#[test]
fn frame_metrics_track_budget_diff_output_and_cumulative_stream_totals() {
    let mut engine = engine();
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    assert_eq!(
        engine
            .ingest_data(b"# Bounds\n\nbody\n", Duration::ZERO)
            .unwrap(),
        IngestResult::FirstFrame
    );
    let first = commit_due(&mut engine, &mut writer, Duration::ZERO);
    assert_frame_metrics(&engine, &writer, &first);
    assert!(!work_is_zero(first.metrics.layout_budget));

    let mut expected = StreamMetrics {
        read_bytes: 15,
        updates: 1,
        ..StreamMetrics::default()
    };
    expected.frames = 1;
    expected.diff_rows = first.metrics.diff_rows;
    expected.recovery_tiles = first.metrics.recovery_tiles;
    expected.output_bytes = first.metrics.output_bytes;
    expected.layout_budget = first.metrics.layout_budget;
    expected.last_frame = Some(first.metrics);
    assert_eq!(engine.metrics(), expected);

    for (index, update) in [b"\nsecond\n\n".as_slice(), b"\nthird\n\n".as_slice()]
        .into_iter()
        .enumerate()
    {
        let at = Duration::from_millis(100 * (index as u64 + 1));
        assert_eq!(
            engine.ingest_data(update, at).unwrap(),
            IngestResult::Applied
        );
        let render_at = at + MAX_BATCH_DELAY;
        let commit = commit_due(&mut engine, &mut writer, render_at);
        assert_frame_metrics(&engine, &writer, &commit);
        expected.read_bytes += update.len();
        expected.updates += 1;
        expected.frames += 1;
        expected.diff_rows += commit.metrics.diff_rows;
        expected.recovery_tiles += commit.metrics.recovery_tiles;
        expected.output_bytes += commit.metrics.output_bytes;
        accumulate_work(&mut expected.layout_budget, commit.metrics.layout_budget);
        expected.last_frame = Some(commit.metrics);
    }

    let final_commit = finish_and_commit(&mut engine, &mut writer, Duration::from_secs(1));
    assert_frame_metrics(&engine, &writer, &final_commit);
    expected.updates += 1;
    expected.frames += 1;
    expected.diff_rows += final_commit.metrics.diff_rows;
    expected.recovery_tiles += final_commit.metrics.recovery_tiles;
    expected.output_bytes += final_commit.metrics.output_bytes;
    accumulate_work(
        &mut expected.layout_budget,
        final_commit.metrics.layout_budget,
    );
    expected.last_frame = Some(final_commit.metrics);
    assert_eq!(engine.metrics(), expected);
}

#[test]
fn failed_frame_does_not_update_stream_metrics_and_retry_succeeds() {
    let mut engine = engine();
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    assert_eq!(
        engine
            .ingest_data(b"# Stable\n\nbody\n", Duration::ZERO)
            .unwrap(),
        IngestResult::FirstFrame
    );
    commit_due(&mut engine, &mut writer, Duration::ZERO);

    let at = Duration::from_millis(100);
    assert_eq!(
        engine.ingest_data(b"\nafter\n", at).unwrap(),
        IngestResult::Applied
    );
    let before_render = engine.metrics();
    let render_at = at + MAX_BATCH_DELAY;
    writer.fail_on_call(writer.write_calls() + 3);
    writer.begin_frame();
    let failed_frame = writer.frames().len() - 1;
    let mut budget = Budget::new(FRAME_WORK);
    let error = engine
        .render_due(&mut writer, &mut budget, render_at)
        .expect_err("injected write failure must propagate");
    assert!(error.to_string().contains("injected vt100 writer failure"));
    writer.finish_frame(FrameOutcome::Failed);
    assert_eq!(engine.metrics(), before_render);
    assert_eq!(
        writer.frames()[failed_frame].outcome,
        Some(FrameOutcome::Failed)
    );

    writer.clear_failure();
    let commit = commit_due(&mut engine, &mut writer, render_at);
    assert_frame_metrics(&engine, &writer, &commit);
    assert_eq!(engine.metrics().frames, before_render.frames + 1);
    assert_eq!(engine.metrics().last_frame, Some(commit.metrics));
}

#[test]
fn exhausted_layout_step_reports_actual_budget_without_output_metrics() {
    let mut source = String::new();
    for index in 0..100 {
        source.push_str(&format!(
            "paragraph {index:03} abcdefghijklmnopqrstuvwxyz\n\n"
        ));
    }
    let mut runtime =
        FrameRuntime::from_finished_source(&source, geometry(), LinkMode::Default).unwrap();
    let limit = WorkLimit {
        queries: 1,
        tiles: 1,
        bytes: 32,
        objects: 1,
        recovery: 1,
        dependencies: 1,
    };
    let mut budget = Budget::new(limit);
    let mut output = Vec::new();
    let commit = runtime.frame_to(&mut output, &mut budget).unwrap();
    assert!(!commit.committed);
    assert!(output.is_empty());
    assert_eq!(commit.metrics.layout_budget, budget.used());
    assert!(work_within(commit.metrics.layout_budget, limit));
    assert_eq!(commit.metrics.diff_rows, 0);
    assert_eq!(commit.metrics.output_bytes, 0);
    assert_eq!(commit.metrics.body_bytes, 0);
}

fn drain_due(engine: &mut StreamEngine, writer: &mut io::Sink, now: Duration) {
    for _ in 0..10_000 {
        if !engine.due(now) {
            return;
        }
        let mut budget = Budget::new(FRAME_WORK);
        match engine.render_due(writer, &mut budget, now).unwrap() {
            Some(commit) if commit.committed => {}
            _ => {}
        }
    }
    panic!("due frames did not settle");
}

fn finish_calibration(engine: &mut StreamEngine, now: Duration) {
    engine.finish_input(now).unwrap();
    for _ in 0..1_000_000 {
        let mut budget = Budget::new(FRAME_WORK);
        if engine.advance_calibration(&mut budget).unwrap().is_some() {
            break;
        }
    }
    drain_due(engine, &mut io::sink(), now);
}

#[test]
#[ignore = "manual wall-clock benchmark; not a CI assertion"]
fn benchmark_1000_updates_and_large_stream() {
    let mut sink = io::sink();
    let update_started = Instant::now();
    let mut update_engine = engine();
    let prefix = b"# Updates\n";
    update_engine.ingest_data(prefix, Duration::ZERO).unwrap();
    drain_due(&mut update_engine, &mut sink, Duration::ZERO);
    for index in 0..1_000 {
        update_engine.ingest_data(b"x", Duration::ZERO).unwrap();
        if index % 32 == 31 {
            drain_due(&mut update_engine, &mut sink, Duration::ZERO);
        }
    }
    finish_calibration(&mut update_engine, Duration::from_secs(1));
    let update_elapsed = update_started.elapsed();
    let update_metrics = update_engine.metrics();
    assert_eq!(update_metrics.read_bytes, prefix.len() + 1_000);

    let mut source = String::from("# Large\n\n");
    while source.len() < 1024 * 1024 {
        source.push_str("paragraph abcdefghijklmnopqrstuvwxyz 0123456789\n\n");
    }
    let stream_started = Instant::now();
    let mut stream_engine = engine();
    for chunk in source.as_bytes().chunks(READ_CHUNK_BYTES) {
        stream_engine.ingest_data(chunk, Duration::ZERO).unwrap();
        drain_due(&mut stream_engine, &mut sink, Duration::ZERO);
    }
    finish_calibration(&mut stream_engine, Duration::from_secs(1));
    let stream_elapsed = stream_started.elapsed();
    let stream_metrics = stream_engine.metrics();
    assert_eq!(stream_metrics.read_bytes, source.len());
    println!(
        "metrics benchmark: updates={:?} update_elapsed={update_elapsed:?}; large_bytes={} large_elapsed={stream_elapsed:?}; frames={} output_bytes={}",
        update_metrics,
        stream_metrics.read_bytes,
        stream_metrics.frames,
        stream_metrics.output_bytes
    );
}
