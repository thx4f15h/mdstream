mod support;

use std::io::{self, Cursor, Read};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use mdtui_tui::cli::LinkMode;
use mdtui_tui::render::frame::{FIRST_FRAME_CLEAR, FrameCommit, FrameGeometry, FrameRuntime};
use mdtui_tui::stream::{
    FIRST_FRAME_FALLBACK_DELAY, FRAME_WORK, IngestResult, LoadEvent, MAX_BATCH_DELAY,
    MAX_PRE_READ_BATCHES, MAX_PRE_READ_BYTES, MAX_UPDATES_PER_FRAME, PRE_READ_BATCH_BYTES,
    READ_CHUNK_BYTES, Reader, StreamEngine, probe_has_drawable_lines,
};
use mdtui_tui::theme::{MONOKAI, Rgb, Underline};
use mdtui3::{Budget, Document, WorkLimit};
use support::terminal::TerminalScreen;
use vt100::Color;

const WIDTH: u16 = 120;
const HEIGHT: u16 = 40;
const CLEAR: &[u8] = b"\x1b[2J";
const HOME: &[u8] = b"\x1b[H";

fn geometry() -> FrameGeometry {
    FrameGeometry {
        rows: HEIGHT,
        cols: WIDTH,
        content_width: WIDTH,
    }
}

fn unlimited() -> Budget {
    Budget::new(WorkLimit::unlimited())
}

fn engine() -> StreamEngine {
    StreamEngine::new(FrameRuntime::new(geometry(), LinkMode::Default).unwrap())
}

fn render_due(
    engine: &mut StreamEngine,
    output: &mut Vec<u8>,
    now: Duration,
) -> Option<FrameCommit> {
    engine
        .render_due(output, &mut unlimited(), now)
        .expect("frame render must succeed")
}

fn finish_and_render(engine: &mut StreamEngine, output: &mut Vec<u8>, now: Duration) {
    engine.finish_input(now).expect("stream input must finish");
    for _ in 0..100_000 {
        if engine
            .advance_calibration(&mut unlimited())
            .expect("calibration must succeed")
            .is_some()
        {
            break;
        }
    }
    let commit = render_due(engine, output, now).expect("final frame must commit");
    assert!(commit.committed);
}

fn count(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|window| *window == needle)
        .count()
}

fn color(value: Option<Rgb>) -> Color {
    value.map_or(Color::Default, |value| {
        Color::Rgb(value.red, value.green, value.blue)
    })
}

fn assert_terminal_matches_one_shot(terminal: &TerminalScreen, source: &str) {
    let mut runtime =
        FrameRuntime::from_finished_source(source, geometry(), LinkMode::Default).unwrap();
    let mut output = Vec::new();
    let commit = runtime
        .frame_to(&mut output, &mut unlimited())
        .expect("one-shot frame");
    assert!(commit.committed);
    let expected = runtime.screen();
    assert_eq!(terminal.size(), (HEIGHT, WIDTH));
    for row in 0..HEIGHT {
        for column in 0..WIDTH {
            let expected = expected.front_cell(row, column).unwrap();
            let actual = terminal.cell(row, column).unwrap();
            if expected.is_continuation() {
                assert!(
                    actual.text.is_empty() || actual.text == " ",
                    "row {row} column {column}: continuation text mismatch"
                );
            } else if expected.text().trim().is_empty() {
                assert!(
                    actual.text.trim().is_empty(),
                    "row {row} column {column}: expected blank, got {:?}",
                    actual.text
                );
            } else {
                assert_eq!(
                    actual.text,
                    expected.text(),
                    "row {row} column {column}: text mismatch"
                );
            }
            let style = MONOKAI.resolve(expected.style());
            assert_eq!(actual.foreground, color(style.foreground));
            assert_eq!(actual.background, color(style.background));
            assert_eq!(actual.bold, style.bold);
            assert_eq!(actual.italic, style.italic);
            assert_eq!(actual.underline, style.underline != Underline::None);
            assert_eq!(actual.wide, expected.width() == 2);
            assert_eq!(actual.wide_continuation, expected.is_continuation());
        }
    }
}

fn large_source() -> String {
    let mut source = String::from("# Stream start\n\n");
    let mut index = 0;
    while source.len() < READ_CHUNK_BYTES * 2 + 4096 {
        source.push_str(&format!(
            "paragraph {index:04} abcdefghijklmnopqrstuvwxyz 0123456789\n\n"
        ));
        index += 1;
    }
    source.push_str("# Stream tail\n");
    source
}

struct PausedReader {
    data: Vec<u8>,
    offset: usize,
    reads: Arc<AtomicUsize>,
    gate: Arc<(Mutex<bool>, Condvar)>,
}

impl Read for PausedReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read_number = self.reads.fetch_add(1, Ordering::SeqCst) + 1;
        if read_number == 2 {
            let (lock, wake) = &*self.gate;
            let mut released = lock.lock().unwrap();
            while !*released {
                released = wake.wait(released).unwrap();
            }
        }
        if self.offset == self.data.len() {
            return Ok(0);
        }
        let count = buffer.len().min(self.data.len() - self.offset);
        buffer[..count].copy_from_slice(&self.data[self.offset..self.offset + count]);
        self.offset += count;
        Ok(count)
    }
}

struct Gate {
    state: Arc<(Mutex<bool>, Condvar)>,
}

impl Gate {
    fn release(&self) {
        let (lock, wake) = &*self.state;
        *lock.lock().unwrap() = true;
        wake.notify_all();
    }
}

impl Drop for Gate {
    fn drop(&mut self) {
        self.release();
    }
}

#[test]
fn paused_reader_commits_first_frame_before_eof_then_matches_one_shot() {
    let source = large_source();
    let reads = Arc::new(AtomicUsize::new(0));
    let gate = Gate {
        state: Arc::new((Mutex::new(false), Condvar::new())),
    };
    let mut reader = Reader::spawn(PausedReader {
        data: source.as_bytes().to_vec(),
        offset: 0,
        reads: reads.clone(),
        gate: gate.state.clone(),
    })
    .unwrap();
    let mut engine = engine();
    let mut output = Vec::new();
    let now = Duration::ZERO;

    let first = reader
        .recv_timeout(Duration::from_secs(2))
        .expect("first 64 KiB data event");
    let LoadEvent::Data(first) = first else {
        panic!("expected first data event");
    };
    assert_eq!(first.len(), READ_CHUNK_BYTES);
    assert_eq!(
        engine.ingest_data(&first, now).unwrap(),
        IngestResult::FirstFrame
    );
    assert_eq!(engine.pre_read_bytes(), PRE_READ_BATCH_BYTES);
    assert_eq!(engine.pre_read_batches(), 1);
    assert_eq!(
        engine.queued_bytes(),
        READ_CHUNK_BYTES - PRE_READ_BATCH_BYTES
    );
    assert_eq!(engine.document().source().len(), PRE_READ_BATCH_BYTES);
    let mut first_budget = Budget::new(FRAME_WORK);
    let first_commit = engine
        .render_due(&mut output, &mut first_budget, now)
        .expect("first frame")
        .expect("first frame must commit within the terminal frame budget");
    assert!(first_commit.committed);
    assert!(first_commit.cleared);
    assert!(engine.first_frame_done());
    assert_eq!(engine.document().source().len(), READ_CHUNK_BYTES);
    assert_eq!(engine.queued_bytes(), 0);
    assert!(!engine.document().is_finished());
    assert!(matches!(
        reader.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
    wait_until(Duration::from_secs(2), || reads.load(Ordering::SeqCst) >= 2);

    gate.release();
    let mut frames = 1_usize;
    loop {
        let event = reader
            .recv_timeout(Duration::from_secs(2))
            .expect("stream event after release");
        match event {
            LoadEvent::Data(bytes) => {
                engine.ingest_data(&bytes, now).unwrap();
                if let Some(commit) = render_due(&mut engine, &mut output, now) {
                    assert!(commit.committed);
                    frames += 1;
                }
            }
            LoadEvent::Eof => {
                engine.finish_input(now).unwrap();
                break;
            }
            LoadEvent::Error(error) => panic!("unexpected read error: {error}"),
        }
    }
    for _ in 0..100_000 {
        if engine
            .advance_calibration(&mut unlimited())
            .expect("calibration")
            .is_some()
        {
            break;
        }
    }
    let final_commit = render_due(&mut engine, &mut output, now).expect("final frame");
    assert!(final_commit.committed);
    frames += 1;
    reader.close().expect("reader joins");

    assert!(engine.document().is_finished());
    assert_eq!(engine.document().source(), source);
    assert!(frames >= 2);
    let mut terminal = TerminalScreen::new(HEIGHT, WIDTH);
    terminal.feed(&output);
    assert_terminal_matches_one_shot(&terminal, &source);
    assert!(output.starts_with(FIRST_FRAME_CLEAR));
    assert_eq!(count(&output, CLEAR), 1);
    assert_eq!(count(&output, HOME), 1);
}

#[test]
fn one_thousand_small_updates_obey_frame_bound_and_keep_the_last_update() {
    let mut engine = engine();
    let mut output = Vec::new();
    let now = Duration::ZERO;
    let updates = 1_000_usize;
    let mut frames = 0_usize;

    for index in 0..updates {
        let result = engine.ingest_data(b"x", now).unwrap();
        if index == 0 {
            assert_eq!(result, IngestResult::FirstFrame);
        }
        if engine.due(now) {
            let commit = render_due(&mut engine, &mut output, now).expect("due frame");
            assert!(commit.committed);
            frames += 1;
        }
    }
    let bound = 1 + (updates - 1).div_ceil(MAX_UPDATES_PER_FRAME);
    assert_eq!(frames, bound - 1);
    assert_eq!(engine.pending_updates(), 7);
    assert_eq!(engine.document().source().len(), updates);
    assert!(!engine.document().is_finished());
    assert_eq!(
        engine.runtime().viewport().revision(),
        engine.document().revision()
    );

    finish_and_render(&mut engine, &mut output, now);
    frames += 1;
    assert_eq!(frames, bound);
    assert!(engine.document().is_finished());
    assert_eq!(engine.document().source().len(), updates);

    let mut terminal = TerminalScreen::new(HEIGHT, WIDTH);
    terminal.feed(&output);
    assert_terminal_matches_one_shot(&terminal, &"x".repeat(updates));
    assert_eq!(terminal.text().matches('x').count(), updates);
    assert_eq!(count(&output, CLEAR), 1);
    assert_eq!(count(&output, HOME), 1);
}

#[test]
fn deadline_flushes_a_small_pending_batch_with_injected_time() {
    let mut engine = engine();
    let mut output = Vec::new();
    assert_eq!(
        engine.ingest_data(b"a", Duration::ZERO).unwrap(),
        IngestResult::FirstFrame
    );
    render_due(&mut engine, &mut output, Duration::ZERO).expect("first frame");

    engine
        .ingest_data(b"b", Duration::ZERO)
        .expect("second update");
    assert!(!engine.due(Duration::ZERO));
    engine
        .ingest_data(b"c", MAX_BATCH_DELAY)
        .expect("third update");
    assert!(engine.due(MAX_BATCH_DELAY));
    let commit = render_due(&mut engine, &mut output, MAX_BATCH_DELAY).expect("deadline frame");
    assert!(commit.committed);
    assert!(commit.bytes > 0);
}

struct ChunkReader {
    chunks_left: usize,
    reads: Arc<AtomicUsize>,
}

impl Read for ChunkReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if self.chunks_left == 0 {
            return Ok(0);
        }
        self.chunks_left -= 1;
        let count = buffer.len().min(READ_CHUNK_BYTES);
        buffer[..count].fill(b'a');
        Ok(count)
    }
}

fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while !condition() {
        assert!(Instant::now() < deadline, "condition timed out");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn sync_channel_capacity_applies_backpressure_to_the_reader() {
    let reads = Arc::new(AtomicUsize::new(0));
    let mut reader = Reader::spawn(ChunkReader {
        chunks_left: 8,
        reads: reads.clone(),
    })
    .unwrap();
    wait_until(Duration::from_secs(2), || reads.load(Ordering::SeqCst) >= 2);
    std::thread::sleep(Duration::from_millis(25));
    assert_eq!(
        reads.load(Ordering::SeqCst),
        2,
        "reader must block with one queued chunk and one blocked send"
    );

    let first = reader
        .recv_timeout(Duration::from_secs(2))
        .expect("queued chunk");
    let LoadEvent::Data(first) = first else {
        panic!("expected data");
    };
    assert_eq!(first.len(), READ_CHUNK_BYTES);
    wait_until(Duration::from_secs(2), || reads.load(Ordering::SeqCst) >= 3);
    reader.close().expect("reader joins after receiver close");
}

struct ErrorAfterData {
    sent: bool,
}

impl Read for ErrorAfterData {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if !self.sent {
            self.sent = true;
            buffer[..4].copy_from_slice(b"data");
            return Ok(4);
        }
        Err(io::Error::other("injected read failure"))
    }
}

#[test]
fn reader_emits_error_and_eof_as_control_events() {
    let mut error_reader = Reader::spawn(ErrorAfterData { sent: false }).unwrap();
    let first = error_reader
        .recv_timeout(Duration::from_secs(2))
        .expect("data before error");
    assert!(matches!(first, LoadEvent::Data(bytes) if bytes == b"data"));
    let error = error_reader
        .recv_timeout(Duration::from_secs(2))
        .expect("read error event");
    let LoadEvent::Error(error) = error else {
        panic!("expected error event");
    };
    assert_eq!(error.to_string(), "injected read failure");
    error_reader.close().expect("error reader joins");

    let mut eof_reader = Reader::spawn(Cursor::new(b"tail".to_vec())).unwrap();
    let data = eof_reader
        .recv_timeout(Duration::from_secs(2))
        .expect("data before EOF");
    assert!(matches!(data, LoadEvent::Data(bytes) if bytes == b"tail"));
    assert!(matches!(
        eof_reader
            .recv_timeout(Duration::from_secs(2))
            .expect("EOF event"),
        LoadEvent::Eof
    ));
    eof_reader.close().expect("EOF reader joins");

    let mut empty = Reader::spawn(Cursor::new(Vec::<u8>::new())).unwrap();
    assert!(matches!(
        empty
            .recv_timeout(Duration::from_secs(2))
            .expect("empty EOF event"),
        LoadEvent::Eof
    ));
    empty.close().expect("empty reader joins");
}

#[test]
fn empty_stream_commits_only_the_final_calibrated_first_frame() {
    let mut engine = engine();
    let mut output = Vec::new();
    let now = Duration::ZERO;
    assert!(engine.finish_input(now).unwrap());
    for _ in 0..100_000 {
        if engine
            .advance_calibration(&mut unlimited())
            .expect("empty calibration")
            .is_some()
        {
            break;
        }
    }
    let commit = render_due(&mut engine, &mut output, now).expect("empty final frame");
    assert!(commit.committed);
    assert!(commit.first_frame);
    assert_eq!(output, FIRST_FRAME_CLEAR);
    assert!(engine.document().is_finished());
    assert!(engine.document().source().is_empty());
    assert!(!engine.calibration_pending());
}

fn document(source: &str) -> Document {
    let mut document = Document::new();
    document.append(source).unwrap();
    document.finish();
    document
}

#[test]
fn drawable_probe_ignores_blank_and_hidden_and_finds_visible_rows() {
    assert!(!probe_has_drawable_lines(&document("\n\n   \n\n"), WIDTH, &mut unlimited()).unwrap());
    assert!(
        !probe_has_drawable_lines(
            &document("[hidden]: https://example.com\n\n"),
            WIDTH,
            &mut unlimited()
        )
        .unwrap()
    );
    assert!(
        probe_has_drawable_lines(&document("visible text\n"), WIDTH, &mut unlimited()).unwrap()
    );
    assert!(
        probe_has_drawable_lines(&document("```rust\n```\n"), WIDTH, &mut unlimited()).unwrap()
    );
    let padded = String::from_utf8(padded_batch("# HEAD\n")).unwrap();
    assert!(
        probe_has_drawable_lines(&document(&padded), WIDTH, &mut unlimited()).unwrap(),
        "padded heading should be drawable"
    );
    let mut unfinished = Document::new();
    unfinished.append(&padded).unwrap();
    assert!(
        probe_has_drawable_lines(&unfinished, WIDTH, &mut unlimited()).unwrap(),
        "unfinished padded heading should be drawable"
    );
    let blank = String::from_utf8(padded_batch("")).unwrap();
    assert!(!probe_has_drawable_lines(&document(&blank), WIDTH, &mut unlimited()).unwrap());
    let hidden = String::from_utf8(hidden_batch("")).unwrap();
    assert!(!probe_has_drawable_lines(&document(&hidden), WIDTH, &mut unlimited()).unwrap());
    let mut combined = blank.clone();
    combined.push_str(&String::from_utf8(padded_batch("# FOUND\n")).unwrap());
    assert!(
        probe_has_drawable_lines(&document(&combined), WIDTH, &mut unlimited()).unwrap(),
        "heading after a blank pre-read batch should be drawable"
    );
}

fn padded_batch(prefix: &str) -> Vec<u8> {
    assert!(prefix.len() <= PRE_READ_BATCH_BYTES);
    let mut bytes = prefix.as_bytes().to_vec();
    bytes.resize(PRE_READ_BATCH_BYTES, b'\n');
    bytes
}

fn hidden_batch(prefix: &str) -> Vec<u8> {
    let mut source = String::from(prefix);
    if !source.is_empty() && !source.ends_with('\n') {
        source.push('\n');
    }
    if !source.is_empty() {
        source.push('\n');
    }
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
fn pre_read_splits_64k_chunk_and_preserves_tail_until_after_first_frame() {
    let first_batch = hidden_batch("# HEAD\n");
    let tail_batch = padded_batch("# TAIL\n");
    let mut chunk = first_batch.clone();
    chunk.extend_from_slice(&tail_batch);
    assert_eq!(chunk.len(), READ_CHUNK_BYTES);

    let mut engine = engine();
    let mut output = Vec::new();
    let now = Duration::ZERO;
    assert_eq!(
        engine.ingest_data(&chunk, now).unwrap(),
        IngestResult::FirstFrame
    );
    assert_eq!(engine.pre_read_bytes(), PRE_READ_BATCH_BYTES);
    assert_eq!(engine.pre_read_batches(), 1);
    assert_eq!(
        engine.queued_bytes(),
        READ_CHUNK_BYTES - PRE_READ_BATCH_BYTES
    );
    assert_eq!(engine.document().source().len(), PRE_READ_BATCH_BYTES);
    assert!(!engine.document().is_finished());
    assert!(!engine.document().source().contains("TAIL"));

    let first = render_due(&mut engine, &mut output, now).expect("first frame");
    assert!(first.committed);
    assert!(String::from_utf8_lossy(&output).contains("HEAD"));
    assert!(engine.queued_bytes() == 0);
    assert_eq!(engine.document().source().len(), READ_CHUNK_BYTES);
    assert!(engine.document().source().contains("TAIL"));
    assert!(!String::from_utf8_lossy(&output).contains("TAIL"));

    assert!(engine.runtime_mut().scroll_down(u64::MAX).unwrap());
    finish_and_render(&mut engine, &mut output, now);
    let mut terminal = TerminalScreen::new(HEIGHT, WIDTH);
    terminal.feed(&output);
    assert!(terminal.text().contains("TAIL"));
    assert_eq!(count(&output, CLEAR), 1);
    assert_eq!(count(&output, HOME), 1);
}

#[test]
fn pre_read_continues_until_a_later_batch_has_drawable_content() {
    let mut engine = engine();
    let mut output = Vec::new();
    let now = Duration::ZERO;
    let empty = hidden_batch("");
    assert_eq!(
        engine.ingest_data(&empty, now).unwrap(),
        IngestResult::PreReading
    );
    assert_eq!(engine.pre_read_bytes(), PRE_READ_BATCH_BYTES);
    assert_eq!(engine.pre_read_batches(), 1);
    assert!(!engine.first_frame_done());
    assert!(!engine.due(now));
    assert!(!engine.document().is_finished());

    let found = padded_batch("# FOUND\n");
    assert_eq!(
        engine.ingest_data(&found, now).unwrap(),
        IngestResult::FirstFrame
    );
    assert_eq!(engine.pre_read_bytes(), PRE_READ_BATCH_BYTES * 2);
    assert_eq!(engine.pre_read_batches(), 2);
    let first = render_due(&mut engine, &mut output, now).expect("first frame after content");
    assert!(first.committed);
    assert!(String::from_utf8_lossy(&output).contains("FOUND"));
    assert!(!engine.document().is_finished());
    finish_and_render(&mut engine, &mut output, now);
    let mut terminal = TerminalScreen::new(HEIGHT, WIDTH);
    terminal.feed(&output);
    assert!(
        terminal.text().contains("FOUND"),
        "final screen must still show later-batch content"
    );
}

#[test]
fn pre_read_stops_at_288k_without_content_and_commits_first_frame() {
    let mut engine = engine();
    let mut output = Vec::new();
    let now = Duration::ZERO;
    let empty = hidden_batch("");

    for batch in 0..MAX_PRE_READ_BATCHES {
        let result = engine.ingest_data(&empty, now).unwrap();
        assert_eq!(engine.pre_read_batches(), batch + 1);
        if batch + 1 == MAX_PRE_READ_BATCHES {
            assert_eq!(result, IngestResult::FirstFrame);
        } else {
            assert_eq!(result, IngestResult::PreReading);
            assert!(!engine.due(now));
        }
    }
    assert_eq!(engine.pre_read_bytes(), MAX_PRE_READ_BYTES);
    assert_eq!(engine.document().source().len(), MAX_PRE_READ_BYTES);
    assert!(!engine.document().is_finished());
    assert!(engine.due(now));
    let first = render_due(&mut engine, &mut output, now).expect("288 KiB fallback frame");
    assert!(first.committed);
    assert_eq!(output, FIRST_FRAME_CLEAR);
    assert!(engine.first_frame_done());
    assert!(!engine.document().is_finished());
}

#[test]
fn paused_no_content_pre_read_falls_back_after_deadline() {
    let data = hidden_batch("");
    let reads = Arc::new(AtomicUsize::new(0));
    let gate = Gate {
        state: Arc::new((Mutex::new(false), Condvar::new())),
    };
    let mut reader = Reader::spawn(PausedReader {
        data,
        offset: 0,
        reads: reads.clone(),
        gate: gate.state.clone(),
    })
    .unwrap();
    let mut engine = engine();
    let mut output = Vec::new();
    let event = reader
        .recv_timeout(Duration::from_secs(2))
        .expect("first pre-read batch");
    let LoadEvent::Data(first) = event else {
        panic!("expected data");
    };
    assert_eq!(first.len(), PRE_READ_BATCH_BYTES);
    assert_eq!(
        engine.ingest_data(&first, Duration::ZERO).unwrap(),
        IngestResult::PreReading
    );
    assert!(!engine.due(Duration::ZERO));
    assert!(matches!(
        reader.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
    assert!(engine.due(FIRST_FRAME_FALLBACK_DELAY));
    let first_frame = render_due(&mut engine, &mut output, FIRST_FRAME_FALLBACK_DELAY)
        .expect("deadline fallback frame");
    assert!(first_frame.committed);
    assert!(engine.first_frame_done());
    assert!(!engine.document().is_finished());
    assert_eq!(output, FIRST_FRAME_CLEAR);
    assert!(reads.load(Ordering::SeqCst) >= 2);

    gate.release();
    assert!(matches!(
        reader
            .recv_timeout(Duration::from_secs(2))
            .expect("EOF after release"),
        LoadEvent::Eof
    ));
    engine.finish_input(FIRST_FRAME_FALLBACK_DELAY).unwrap();
    for _ in 0..100_000 {
        if engine
            .advance_calibration(&mut unlimited())
            .expect("calibration")
            .is_some()
        {
            break;
        }
    }
    let final_commit =
        render_due(&mut engine, &mut output, FIRST_FRAME_FALLBACK_DELAY).expect("final frame");
    assert!(final_commit.committed);
    assert_eq!(count(&output, CLEAR), 1);
    assert_eq!(count(&output, HOME), 1);
    reader.close().expect("reader joins");
}

#[test]
fn eof_before_pre_read_limit_finishes_to_one_shot() {
    let source = "# Early EOF\n\nshort body\n";
    let mut engine = engine();
    let mut output = Vec::new();
    let now = Duration::ZERO;
    assert_eq!(
        engine.ingest_data(source.as_bytes(), now).unwrap(),
        IngestResult::FirstFrame
    );
    let first = render_due(&mut engine, &mut output, now).expect("first frame");
    assert!(first.committed);
    assert!(!engine.document().is_finished());
    assert!(engine.pre_read_bytes() < MAX_PRE_READ_BYTES);
    finish_and_render(&mut engine, &mut output, now);
    assert!(engine.document().is_finished());

    let mut terminal = TerminalScreen::new(HEIGHT, WIDTH);
    terminal.feed(&output);
    assert_terminal_matches_one_shot(&terminal, source);
    assert_eq!(count(&output, CLEAR), 1);
    assert_eq!(count(&output, HOME), 1);
}
