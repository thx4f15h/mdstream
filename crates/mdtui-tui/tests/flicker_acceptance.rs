mod support;

use std::time::Duration;

use mdtui_tui::cli::LinkMode;
use mdtui_tui::render::frame::{
    BEGIN_SYNCHRONIZED_UPDATE, END_SYNCHRONIZED_UPDATE, FIRST_FRAME_CLEAR, FrameCommit,
    FrameGeometry, FrameRuntime,
};
use mdtui_tui::static_render::render_static;
use mdtui_tui::stream::{IngestResult, MAX_UPDATES_PER_FRAME, StreamEngine};
use mdtui3::{Budget, WorkLimit};
use support::terminal::{FrameOutcome, Vt100Writer, assert_terminal_matches_screen};

const WIDTH: u16 = 120;
const HEIGHT: u16 = 40;
const APPENDS: usize = 1_000;
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

fn engine(link_mode: LinkMode) -> StreamEngine {
    StreamEngine::new(FrameRuntime::new(geometry(), link_mode).unwrap())
}

fn unlimited() -> Budget {
    Budget::new(WorkLimit::unlimited())
}

fn commit(engine: &mut StreamEngine, writer: &mut Vt100Writer, now: Duration) -> FrameCommit {
    writer.begin_frame();
    let commit = engine
        .render_due(writer, &mut unlimited(), now)
        .expect("frame render")
        .expect("due frame");
    assert!(commit.committed);
    writer.finish_frame(FrameOutcome::Committed);
    commit
}

fn finish_and_commit(engine: &mut StreamEngine, writer: &mut Vt100Writer) -> FrameCommit {
    engine.finish_input(Duration::ZERO).expect("finish stream");
    for _ in 0..100_000 {
        if engine
            .advance_calibration(&mut unlimited())
            .expect("calibration")
            .is_some()
        {
            break;
        }
    }
    commit(engine, writer, Duration::ZERO)
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn count(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|window| *window == needle)
        .count()
}

fn assert_incremental_protocol(writer: &Vt100Writer, frame: usize, first: bool) {
    let writes = writer.frame_writes(frame);
    if writes.is_empty() {
        assert!(
            writer.terminal().snapshot().has_visible_text(),
            "empty frame {frame} left a blank screen"
        );
        return;
    }

    if first {
        assert_eq!(writes[0].bytes, FIRST_FRAME_CLEAR);
    }
    let body = if first { &writes[1..] } else { writes };
    if body.is_empty() {
        assert!(writer.terminal().snapshot().has_visible_text());
        return;
    }

    let bytes: Vec<u8> = body
        .iter()
        .flat_map(|write| write.bytes.iter().copied())
        .collect();
    // A published frame can leave its final EGC transaction open. Sync closes
    // before that payload; a later append closes its style/OSC tail first.
    assert_eq!(
        count(&bytes, BEGIN_SYNCHRONIZED_UPDATE)
            + count(&bytes, b"\x18\x1b\\\x1b]8;;\x1b\\\x1b[0m\x1b[?2026l")
            + count(&bytes, b"\x18\x1b\\\x1b[0m\x1b[?2026l"),
        count(&bytes, END_SYNCHRONIZED_UPDATE)
    );
    assert!(!contains(&bytes, CLEAR), "normal frame contains 2J");
    assert!(!contains(&bytes, HOME), "normal frame contains bare H");
    assert!(!contains(&bytes, ERASE), "normal frame contains K");
    assert!(
        writer.terminal().snapshot().has_visible_text(),
        "frame {frame} left the terminal empty"
    );
}

fn tail_source() -> String {
    let mut tail = String::from(concat!(
        "```rust\n```\n\n",
        "```text\na  b\t c\n```\n\n",
        "| A | B |\n| --- | --- |\n| one | two |\n\n",
        "ENTITY &amp; and [link](https://example.com)\n\n",
    ));
    for index in 0..4 {
        tail.push_str(&format!(
            "stream paragraph {index:03} with fixed text for flicker acceptance\n\n"
        ));
    }
    tail.push_str(&"x".repeat(APPENDS));
    tail.push('\n');
    tail.push_str("LAST_STREAM_MARK\n");
    assert!(tail.len() >= APPENDS);
    tail
}

fn split_tail(tail: &str) -> Vec<&str> {
    let mut chunks = Vec::with_capacity(APPENDS);
    for index in 0..APPENDS {
        let start = tail.len() * index / APPENDS;
        let end = tail.len() * (index + 1) / APPENDS;
        let chunk = &tail[start..end];
        assert!(!chunk.is_empty(), "tail chunk {index} is empty");
        chunks.push(chunk);
    }
    chunks
}

#[derive(Debug, PartialEq, Eq)]
struct StableRun {
    frames: Vec<Vec<u8>>,
    final_screen: String,
    frame_count: usize,
}

fn run_acceptance(link_mode: LinkMode) -> StableRun {
    let base = "# Flicker acceptance\n\nbase [link](https://example.com) &amp;\n\n";
    let tail = tail_source();
    let chunks = split_tail(&tail);
    let mut engine = engine(link_mode);
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);

    assert_eq!(
        engine.ingest_data(base.as_bytes(), Duration::ZERO).unwrap(),
        IngestResult::FirstFrame
    );
    let first = commit(&mut engine, &mut writer, Duration::ZERO);
    assert!(first.committed && first.cleared);
    assert!(!engine.document().is_finished());
    assert_incremental_protocol(&writer, 0, true);
    assert_terminal_matches_screen(writer.terminal(), engine.runtime().screen());

    for chunk in &chunks {
        assert_eq!(
            engine
                .ingest_data(chunk.as_bytes(), Duration::ZERO)
                .unwrap(),
            IngestResult::Applied
        );
        if engine.due(Duration::ZERO) {
            commit(&mut engine, &mut writer, Duration::ZERO);
            let frame = writer.frames().len() - 1;
            assert_incremental_protocol(&writer, frame, false);
            assert_terminal_matches_screen(writer.terminal(), engine.runtime().screen());
        }
    }
    assert!(engine.pending_updates() > 0);
    assert!(!engine.document().is_finished());

    let final_commit = finish_and_commit(&mut engine, &mut writer);
    assert!(final_commit.committed && engine.document().is_finished());
    let final_frame = writer.frames().len() - 1;
    assert_incremental_protocol(&writer, final_frame, false);
    assert_terminal_matches_screen(writer.terminal(), engine.runtime().screen());
    assert_eq!(engine.document().source(), format!("{base}{tail}"));
    assert!(engine.document().source().ends_with("LAST_STREAM_MARK\n"));

    let mut expected = render_static(engine.document(), WIDTH, HEIGHT).expect("static final");
    expected.screen.swap_buffers();
    assert_terminal_matches_screen(writer.terminal(), &expected.screen);

    let frame_count = writer.frames().len();
    let bound = 1 + APPENDS.div_ceil(MAX_UPDATES_PER_FRAME);
    assert!(
        frame_count <= bound,
        "frame count {frame_count} exceeds bound {bound}"
    );
    for frame in 0..frame_count {
        assert_eq!(
            writer.frames()[frame].outcome,
            Some(FrameOutcome::Committed)
        );
    }
    let all_bytes: Vec<u8> = writer
        .writes()
        .iter()
        .flat_map(|write| write.bytes.iter().copied())
        .collect();
    assert!(contains(&all_bytes, BEGIN_SYNCHRONIZED_UPDATE));
    assert!(contains(&all_bytes, END_SYNCHRONIZED_UPDATE));
    assert_eq!(count(&all_bytes, CLEAR), 1);
    assert_eq!(count(&all_bytes, HOME), 1);

    StableRun {
        frames: (0..frame_count)
            .map(|frame| {
                writer
                    .frame_writes(frame)
                    .iter()
                    .flat_map(|write| write.bytes.iter().copied())
                    .collect()
            })
            .collect(),
        final_screen: writer.terminal().dump(),
        frame_count,
    }
}

#[test]
fn thousand_appends_are_incremental_and_finish_to_static_screen() {
    let run = run_acceptance(LinkMode::Default);
    assert!(run.frame_count >= 2);
    assert!(run.frames.iter().any(|frame| !frame.is_empty()));
    run.frames.iter().skip(1).for_each(|frame| {
        assert!(!contains(frame, CLEAR));
        assert!(!contains(frame, HOME));
        assert!(!contains(frame, ERASE));
    });
    assert!(
        run.final_screen.contains("[rust]"),
        "empty fence label missing"
    );
    assert!(run.final_screen.contains("┌"), "table border missing");
    println!(
        "flicker acceptance frames={} bound={}",
        run.frame_count,
        1 + APPENDS.div_ceil(MAX_UPDATES_PER_FRAME)
    );
}

#[test]
fn three_deterministic_acceptance_runs_are_byte_and_screen_identical() {
    let runs = [
        run_acceptance(LinkMode::Default),
        run_acceptance(LinkMode::Default),
        run_acceptance(LinkMode::Default),
    ];
    assert_eq!(runs[1], runs[0]);
    assert_eq!(runs[2], runs[0]);
}

#[test]
fn no_links_acceptance_has_no_osc8_and_same_final_style_screen() {
    let run = run_acceptance(LinkMode::NoOsc8);
    let bytes: Vec<u8> = run.frames.iter().flatten().copied().collect();
    assert!(!contains(&bytes, b"\x1b]8;"));
    assert!(run.frame_count >= 2);
}
