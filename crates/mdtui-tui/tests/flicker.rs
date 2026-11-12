mod support;

use std::io::Write;
use std::time::Duration;

use mdtui_tui::cli::LinkMode;
use mdtui_tui::render::frame::{
    BEGIN_SYNCHRONIZED_UPDATE, END_SYNCHRONIZED_UPDATE, FIRST_FRAME_CLEAR, FrameCommit,
    FrameGeometry, FrameRuntime,
};
use mdtui_tui::static_render::render_static;
use mdtui_tui::stream::{IngestResult, MAX_BATCH_DELAY, StreamEngine};
use mdtui3::{Budget, WorkLimit};
use support::terminal::{
    FrameOutcome, RecordedWrite, TerminalScreen, Vt100Writer, assert_terminal_matches_screen,
};

const WIDTH: u16 = 120;
const HEIGHT: u16 = 40;
const CLEAR: &[u8] = b"\x1b[2J";
const HOME: &[u8] = b"\x1b[H";
const ERASE: &[u8] = b"\x1b[K";

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

fn unlimited() -> Budget {
    Budget::new(WorkLimit::unlimited())
}

fn commit_due(engine: &mut StreamEngine, writer: &mut Vt100Writer, now: Duration) -> FrameCommit {
    writer.begin_frame();
    match engine.render_due(writer, &mut unlimited(), now) {
        Ok(Some(commit)) if commit.committed => {
            writer.finish_frame(FrameOutcome::Committed);
            commit
        }
        Ok(_) => {
            writer.finish_frame(FrameOutcome::NoOutput);
            panic!("expected a committed frame");
        }
        Err(error) => {
            writer.finish_frame(FrameOutcome::Failed);
            panic!("unexpected frame error: {error}");
        }
    }
}

fn finish_and_commit(
    engine: &mut StreamEngine,
    writer: &mut Vt100Writer,
    now: Duration,
) -> FrameCommit {
    engine.finish_input(now).expect("stream must finish");
    for _ in 0..100_000 {
        if engine
            .advance_calibration(&mut unlimited())
            .expect("calibration must succeed")
            .is_some()
        {
            break;
        }
    }
    commit_due(engine, writer, now)
}

fn contains(bytes: &[u8], needle: &[u8]) -> bool {
    bytes.windows(needle.len()).any(|window| window == needle)
}

fn count(bytes: &[u8], needle: &[u8]) -> usize {
    bytes
        .windows(needle.len())
        .filter(|window| *window == needle)
        .count()
}

fn row_text(terminal: &TerminalScreen, row: u16) -> String {
    let (_, cols) = terminal.size();
    let mut text = String::new();
    for column in 0..cols {
        text.push_str(&terminal.cell(row, column).unwrap().text);
    }
    text.truncate(text.trim_end().len());
    text
}

fn assert_no_clear_home_or_erase(writes: &[support::terminal::WriteRecord]) {
    for write in writes {
        assert!(!contains(&write.bytes, CLEAR), "unexpected 2J");
        assert!(!contains(&write.bytes, HOME), "unexpected bare H");
        assert!(!contains(&write.bytes, ERASE), "unexpected line erase");
    }
}

fn assert_frame_synchronization(writer: &Vt100Writer, frame: usize, first: bool) {
    let writes = writer.frame_writes(frame);
    if first {
        assert_eq!(writes[0].bytes, FIRST_FRAME_CLEAR);
        if writes.len() == 1 {
            return;
        }
        assert_eq!(writes[1].bytes, BEGIN_SYNCHRONIZED_UPDATE);
    } else if writes.is_empty() {
        return;
    } else {
        assert_eq!(writes[0].bytes, BEGIN_SYNCHRONIZED_UPDATE);
    }
    assert_eq!(
        writes.last().unwrap().bytes,
        END_SYNCHRONIZED_UPDATE,
        "frame {frame} must close synchronized update"
    );
    let bytes: Vec<u8> = writes
        .iter()
        .flat_map(|write| write.bytes.iter().copied())
        .collect();
    assert_eq!(count(&bytes, BEGIN_SYNCHRONIZED_UPDATE), 1);
    assert_eq!(count(&bytes, END_SYNCHRONIZED_UPDATE), 1);
}

fn assert_frame_write_states(writer: &Vt100Writer, frame: usize, top_text: &str, first: bool) {
    let writes = writer.frame_writes(frame);
    if writes.is_empty() {
        let terminal = writer.terminal().snapshot();
        assert!(
            terminal.has_visible_text(),
            "frame {frame}: empty diff left no visible content"
        );
        if !first {
            assert_eq!(
                terminal.row_text(0),
                Some(top_text),
                "frame {frame}: empty diff changed the top row"
            );
        }
        return;
    }
    let mut content_visible = !first;
    for (index, write) in writes.iter().enumerate() {
        assert!(
            matches!(write.result, RecordedWrite::Ok(_)),
            "frame {frame} write {index}: committed frames contain only successful writes"
        );
        if first && index == 0 {
            assert_eq!(write.bytes, FIRST_FRAME_CLEAR);
            assert!(
                !write.terminal.has_visible_text(),
                "first clear write must leave the fresh screen empty"
            );
            continue;
        }
        assert!(
            !contains(&write.bytes, CLEAR),
            "frame {frame} write {index}: unexpected 2J"
        );
        assert!(
            !contains(&write.bytes, HOME),
            "frame {frame} write {index}: unexpected bare H"
        );
        assert!(
            !contains(&write.bytes, ERASE),
            "frame {frame} write {index}: unexpected line erase"
        );
        content_visible |= write.terminal.has_visible_text();
        if content_visible {
            assert!(
                write.terminal.has_visible_text(),
                "frame {frame} write {index}: intermediate terminal became empty"
            );
            if !first {
                assert_eq!(
                    write.terminal.row_text(0),
                    Some(top_text),
                    "frame {frame} write {index}: top row changed during an append frame"
                );
            }
        }
    }
    assert!(
        writes
            .last()
            .is_some_and(|write| write.terminal.has_visible_text()),
        "frame {frame}: committed frame left no visible content"
    );
}

#[derive(Debug, PartialEq, Eq)]
struct StableRun {
    frames: Vec<Vec<u8>>,
    screen: String,
}

fn deterministic_run() -> StableRun {
    let mut engine = engine();
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    assert_eq!(
        engine
            .ingest_data(
                b"# Stable\n\n[site](https://example.com) and **bold**\n\n",
                Duration::ZERO,
            )
            .unwrap(),
        IngestResult::FirstFrame
    );
    commit_due(&mut engine, &mut writer, Duration::ZERO);
    for (index, update) in [b"tail one\n\n".as_slice(), b"tail two\n\n".as_slice()]
        .into_iter()
        .enumerate()
    {
        let ingest_at = Duration::from_millis(100 * (index as u64 + 1));
        assert_eq!(
            engine.ingest_data(update, ingest_at).unwrap(),
            IngestResult::Applied
        );
        let render_at = ingest_at + MAX_BATCH_DELAY;
        assert!(engine.due(render_at));
        commit_due(&mut engine, &mut writer, render_at);
    }
    finish_and_commit(&mut engine, &mut writer, Duration::from_secs(1));
    let frames = (0..writer.frames().len())
        .map(|frame| {
            writer
                .frame_writes(frame)
                .iter()
                .flat_map(|write| write.bytes.iter().copied())
                .collect()
        })
        .collect();
    StableRun {
        frames,
        screen: writer.terminal().dump(),
    }
}

#[test]
fn vt100_writer_feeds_every_write_immediately_and_records_boundaries() {
    let mut writer = Vt100Writer::new(2, 10);
    let frame = writer.begin_frame();
    writer.write_all(b"abc").unwrap();
    assert!(writer.terminal().text().contains("abc"));
    writer.finish_frame(FrameOutcome::Committed);

    writer.fail_on_call(2);
    writer.begin_frame();
    let error = writer.write_all(b"x").unwrap_err();
    assert_eq!(error.to_string(), "injected vt100 writer failure");
    writer.finish_frame(FrameOutcome::Failed);

    assert_eq!(frame, 0);
    assert!(writer.terminal().text().contains("abc"));
    assert!(!writer.terminal().text().contains('x'));
    assert_eq!(writer.writes().len(), 2);
    assert!(matches!(
        writer.writes()[1].result,
        RecordedWrite::Error { .. }
    ));
    assert_eq!(writer.frames()[0].start_write, 0);
    assert_eq!(writer.frames()[0].end_write, 1);
    assert_eq!(writer.frames()[1].start_write, 1);
    assert_eq!(writer.frames()[1].end_write, 2);
    assert_eq!(writer.frames()[1].outcome, Some(FrameOutcome::Failed));
    assert_eq!(writer.write_calls(), 2);
}

#[test]
fn three_deterministic_stream_runs_have_identical_frames_bytes_and_screen() {
    let runs = [
        deterministic_run(),
        deterministic_run(),
        deterministic_run(),
    ];
    assert!(runs[0].frames.len() > 1);
    let all_bytes: Vec<u8> = runs[0].frames.iter().flatten().copied().collect();
    assert!(contains(&all_bytes, BEGIN_SYNCHRONIZED_UPDATE));
    assert!(contains(&all_bytes, b"\x1b]8;;https://example.com"));
    assert_eq!(runs[1], runs[0]);
    assert_eq!(runs[2], runs[0]);
}

#[test]
fn streamed_frames_are_incremental_per_write_nonblank_and_match_committed_screen() {
    let first_source = "# Stable Top\n\nfirst body\n\nsecond body\n";
    let mut engine = engine();
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);

    assert_eq!(
        engine
            .ingest_data(first_source.as_bytes(), Duration::ZERO)
            .unwrap(),
        IngestResult::FirstFrame
    );
    let first = commit_due(&mut engine, &mut writer, Duration::ZERO);
    assert!(first.cleared);
    assert!(!engine.document().is_finished());
    let top_source = engine.runtime().top_source();
    let top_text = row_text(writer.terminal(), 0);
    assert!(!top_text.trim().is_empty());
    assert_terminal_matches_screen(writer.terminal(), engine.runtime().screen());
    let first_frame = writer.frames().len() - 1;
    assert_frame_write_states(&writer, first_frame, &top_text, true);
    assert_frame_synchronization(&writer, first_frame, true);

    let updates = [
        "\nthird paragraph\n\n",
        "\nfourth paragraph\n\n",
        "\nfifth paragraph\n\n",
    ];
    for (index, update) in updates.iter().enumerate() {
        let ingest_at = Duration::from_millis(100 * (index as u64 + 1));
        assert_eq!(
            engine.ingest_data(update.as_bytes(), ingest_at).unwrap(),
            IngestResult::Applied
        );
        let render_at = ingest_at + MAX_BATCH_DELAY;
        assert!(engine.due(render_at));
        commit_due(&mut engine, &mut writer, render_at);
        let frame = writer.frames().len() - 1;
        assert!(
            !writer.frame_writes(frame).is_empty(),
            "an append frame must emit at least one write"
        );
        assert_frame_write_states(&writer, frame, &top_text, false);
        assert_frame_synchronization(&writer, frame, false);
        assert!(!writer.terminal().text().trim().is_empty());
        assert_eq!(engine.runtime().top_source(), top_source);
        assert_eq!(row_text(writer.terminal(), 0), top_text);
        assert!(writer.terminal().text().contains(update.trim()));
        assert_terminal_matches_screen(writer.terminal(), engine.runtime().screen());
    }

    let final_now = Duration::from_secs(1);
    finish_and_commit(&mut engine, &mut writer, final_now);
    assert!(engine.document().is_finished());
    assert_eq!(engine.runtime().top_source(), top_source);
    let final_frame = writer.frames().len() - 1;
    assert_frame_write_states(&writer, final_frame, &top_text, false);
    assert_frame_synchronization(&writer, final_frame, false);
    assert_terminal_matches_screen(writer.terminal(), engine.runtime().screen());
    let mut expected =
        render_static(engine.document(), WIDTH, HEIGHT).expect("static final render");
    expected.screen.swap_buffers();
    assert_terminal_matches_screen(writer.terminal(), &expected.screen);

    assert_eq!(writer.frames().len(), updates.len() + 2);
    for (index, frame) in writer.frames().iter().enumerate() {
        assert_eq!(frame.outcome, Some(FrameOutcome::Committed));
        if index > 0 {
            assert_eq!(frame.start_write, writer.frames()[index - 1].end_write);
        }
    }
    assert_no_clear_home_or_erase(&writer.frame_writes(0)[1..]);
    for frame in 1..writer.frames().len() {
        assert_no_clear_home_or_erase(writer.frame_writes(frame));
    }
    let all_bytes: Vec<u8> = writer
        .writes()
        .iter()
        .flat_map(|write| write.bytes.iter().copied())
        .collect();
    assert_eq!(count(&all_bytes, CLEAR), 1);
    assert_eq!(count(&all_bytes, HOME), 1);
    assert_eq!(count(&all_bytes, ERASE), 0);
}

#[test]
fn failed_frame_write_is_not_committed_as_a_successful_frame() {
    let mut engine = engine();
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    assert_eq!(
        engine
            .ingest_data(b"# Stable\n\nbody\n", Duration::ZERO)
            .unwrap(),
        IngestResult::FirstFrame
    );
    commit_due(&mut engine, &mut writer, Duration::ZERO);
    let before: Vec<Vec<mdtui_tui::screen::Cell>> = (0..HEIGHT)
        .map(|row| engine.runtime().screen().front_row(row).unwrap().to_vec())
        .collect();

    let ingest_at = Duration::from_millis(100);
    assert_eq!(
        engine.ingest_data(b"\nafter failure\n", ingest_at).unwrap(),
        IngestResult::Applied
    );
    let render_at = ingest_at + MAX_BATCH_DELAY;
    assert!(engine.due(render_at));
    writer.fail_on_call(writer.write_calls() + 3);
    writer.begin_frame();
    let failed_frame = writer.frames().len() - 1;
    let error = engine
        .render_due(&mut writer, &mut unlimited(), render_at)
        .expect_err("injected writer failure must propagate");
    assert!(error.to_string().contains("injected vt100 writer failure"));
    writer.finish_frame(FrameOutcome::Failed);
    for row in 0..HEIGHT {
        assert_eq!(
            engine.runtime().screen().front_row(row).unwrap(),
            before[usize::from(row)].as_slice(),
            "failed frame must not swap front buffer at row {row}"
        );
    }
    let writes = writer.frame_writes(failed_frame);
    assert_eq!(writes[0].bytes, BEGIN_SYNCHRONIZED_UPDATE);
    assert!(
        writes
            .iter()
            .any(|write| matches!(write.result, RecordedWrite::Error { .. }))
    );
    assert_eq!(writes.last().unwrap().bytes, END_SYNCHRONIZED_UPDATE);
    assert_terminal_matches_screen(writer.terminal(), engine.runtime().screen());
    assert_eq!(
        writer.frames().last().unwrap().outcome,
        Some(FrameOutcome::Failed)
    );

    writer.clear_failure();
    commit_due(&mut engine, &mut writer, render_at);
    assert_terminal_matches_screen(writer.terminal(), engine.runtime().screen());
}
