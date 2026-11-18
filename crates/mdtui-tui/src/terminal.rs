use std::fs::File;
use std::io::{self, Read, Write};
use std::panic;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::TryRecvError;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, MouseButton, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    Clear, ClearType, EndSynchronizedUpdate, EnterAlternateScreen, LeaveAlternateScreen,
    disable_raw_mode, enable_raw_mode, size as terminal_size,
};
use mdtui3::Budget;

use crate::app::AppState;
use crate::cli::LinkMode;
use crate::render::frame::{FrameError, FrameGeometry, FrameRuntime};
use crate::stream::{
    ANCHOR_WORK, FRAME_WORK, IDLE_POLL_INTERVAL, LoadEvent, Reader, StreamEngine, StreamError,
};

#[cfg(debug_assertions)]
const PANIC_TEST_ENV: &str = "MDTUI_TEST_PANIC_AFTER_ENTER";
#[cfg(debug_assertions)]
const FRAME_ERROR_TEST_ENV: &str = "MDTUI_TEST_FRAME_ERROR_AFTER_FIRST";
#[cfg(debug_assertions)]
const READ_ERROR_TEST_ENV: &str = "MDTUI_TEST_READ_ERROR_AFTER_FIRST";
#[cfg(debug_assertions)]
const OUTPUT_ERROR_TEST_ENV: &str = "MDTUI_TEST_OUTPUT_ERROR_AFTER_FIRST";

pub const MIN_TERMINAL_COLS: u16 = 20;
pub const MIN_TERMINAL_ROWS: u16 = 3;
pub const SMALL_TERMINAL_MESSAGE: &str = "terminal too small\r\nminimum 20x3";

pub fn run(link_mode: LinkMode, input: File) -> io::Result<()> {
    run_with_table_policy(
        link_mode,
        input,
        crate::render::table_widths::TableWidthPolicy::default(),
    )
}

pub fn run_with_table_policy(
    link_mode: LinkMode,
    input: File,
    policy: crate::render::table_widths::TableWidthPolicy,
) -> io::Result<()> {
    let restore = RestoreState::new(link_mode != LinkMode::NoOsc8);
    install_panic_hook(restore.clone());
    enable_raw_mode()?;
    restore.activate();
    if let Err(error) = execute!(io::stdout(), EnterAlternateScreen, Hide, EnableMouseCapture) {
        restore.restore();
        return Err(error);
    }
    let session = TerminalSession { restore };
    session.restore.nonblocking_output()?;
    // An unbuffered duplicate makes each successful writer ACK an actual OS
    // write; std::Stdout's LineWriter would hide uncredited later flushes.
    let mut output = File::from(rustix::io::dup(io::stdout())?);

    maybe_panic_for_test();

    let (cols, rows) = terminal_size()?;
    if cols < MIN_TERMINAL_COLS || rows < MIN_TERMINAL_ROWS {
        run_too_small()?;
        drop(session);
        return Ok(());
    }

    let mut reader = Reader::spawn(input_reader(input))?;
    let runtime = FrameRuntime::new(
        FrameGeometry {
            rows,
            cols,
            content_width: cols,
        },
        link_mode,
    )
    .map_err(frame_error)?;
    let mut engine = StreamEngine::new(runtime);
    engine.set_table_policy(policy).map_err(stream_error)?;
    let result = run_engine_loop(&mut engine, &mut reader, &mut output);
    abort_output(&mut engine, &mut output);
    drop(session);
    let closed = reader.close().map_err(stream_error);
    // Restore the terminal before draining the owned slug graph. Each batch
    // still uses the cooperative Budget; total destruction is linear work.
    let retired = loop {
        let mut b = Budget::new(ANCHOR_WORK);
        match engine.runtime_mut().retire_slug_session(&mut b) {
            Ok(mdtui3::Completion::Covered) => break Ok(()),
            Ok(_) => (),
            Err(e) => break Err(frame_error(e)),
        }
    };
    result.and(closed).and(retired)
}

fn run_engine_loop(
    engine: &mut StreamEngine,
    reader: &mut Reader,
    output: &mut File,
) -> io::Result<()> {
    let mut app = AppState::default();
    let start = Instant::now();
    let mut first_frame_checked = false;

    loop {
        let now = start.elapsed();
        loop {
            if engine.runtime().fold_busy() {
                break;
            }
            let wire_pending = engine.wire_pending();
            if engine.due(now) && !wire_pending {
                break;
            }
            match reader.try_recv() {
                Ok(LoadEvent::Data(bytes)) => {
                    engine.ingest_data(&bytes, now).map_err(stream_error)?;
                }
                Ok(LoadEvent::Eof) => {
                    engine.finish_input(now).map_err(stream_error)?;
                    break;
                }
                Ok(LoadEvent::Error(error)) => {
                    return Err(io::Error::new(
                        error.kind(),
                        format!("input read error: {error}"),
                    ));
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if engine.input_finished() {
                        break;
                    }
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "input reader stopped before EOF",
                    ));
                }
            }
            // Fairness: at most one new reader batch before servicing a
            // pending wire descriptor and then polling keyboard/resize.
            if wire_pending {
                break;
            }
        }

        if engine.calibration_pending() && !engine.due(now) {
            let mut budget = Budget::new(FRAME_WORK);
            engine
                .advance_calibration(&mut budget)
                .map_err(stream_error)?;
        }
        if engine.due(start.elapsed()) {
            draw_due(engine, output, start.elapsed())?;
            app.frame_committed(engine);
            if !first_frame_checked && engine.first_frame_done() {
                first_frame_checked = true;
                maybe_fail_for_test_after_frame()?;
                maybe_fail_output_for_test_after_frame()?;
            }
        }
        if engine.anchor_build_pending() {
            let mut budget = Budget::new(ANCHOR_WORK);
            engine
                .advance_anchor_index(&mut budget)
                .map_err(stream_error)?;
        }

        // The level-triggered tty source preserves a key that arrives with
        // SIGWINCH. Give it a nonzero deadline even while a frame is due;
        // its zero-timeout path deliberately performs no fd poll.
        let timeout = engine
            .poll_timeout(start.elapsed())
            .min(IDLE_POLL_INTERVAL)
            .max(std::time::Duration::from_millis(1));
        if event::poll(timeout)? {
            match event::read()? {
                Event::Resize(cols, rows) => {
                    app.resize(engine, rows, cols).map_err(stream_error)?;
                }
                Event::Mouse(mouse) if mouse.kind == MouseEventKind::Down(MouseButton::Left) => {
                    app.click_link(engine, mouse.row, mouse.column)
                        .map_err(stream_error)?;
                }
                Event::Key(key) if key.is_press() || key.is_repeat() => {
                    let outcome = app
                        .handle_key(engine, key.code, key.modifiers)
                        .map_err(stream_error)?;
                    if outcome.quit {
                        return Ok(());
                    }
                }
                _ => {}
            }
        }
    }
}

fn abort_output(engine: &mut StreamEngine, output: &mut File) {
    let until = Instant::now() + std::time::Duration::from_millis(100);
    loop {
        let mut budget = Budget::new(mdtui3::WorkLimit {
            queries: 64,
            objects: 64,
            bytes: 64,
            ..Default::default()
        });
        match engine.abort_output(output, &mut budget) {
            Ok(true) | Err(_) => break,
            Ok(false) => {}
        }
        if Instant::now() >= until {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

fn run_too_small() -> io::Result<()> {
    write_small_terminal_message()?;
    loop {
        let timeout = event::poll(IDLE_POLL_INTERVAL)?;
        if !timeout {
            continue;
        }
        match event::read()? {
            Event::Key(key)
                if (key.is_press() || key.is_repeat())
                    && (key.code == crossterm::event::KeyCode::Char('q')
                        || (key.code == crossterm::event::KeyCode::Char('c')
                            && key
                                .modifiers
                                .contains(crossterm::event::KeyModifiers::CONTROL))) =>
            {
                return Ok(());
            }
            Event::Resize(_, _) => write_small_terminal_message()?,
            _ => {}
        }
    }
}

fn write_small_terminal_message() -> io::Result<()> {
    let mut stdout = io::stdout();
    execute!(stdout, Clear(ClearType::All), MoveTo(0, 0))?;
    stdout.write_all(SMALL_TERMINAL_MESSAGE.as_bytes())?;
    stdout.flush()
}

fn draw_due(
    engine: &mut StreamEngine,
    output: &mut File,
    now: std::time::Duration,
) -> io::Result<()> {
    let mut budget = Budget::new(FRAME_WORK);
    engine
        .render_due(output, &mut budget, now)
        .map_err(stream_error)?;
    output.flush()
}

fn frame_error(error: FrameError) -> io::Error {
    io::Error::other(error.to_string())
}

fn stream_error(error: StreamError) -> io::Error {
    io::Error::other(error.to_string())
}

#[cfg(debug_assertions)]
fn input_reader(input: File) -> Box<dyn Read + Send> {
    if std::env::var_os(READ_ERROR_TEST_ENV).is_some() {
        Box::new(ErrorAfterFirstRead {
            input,
            first_read: true,
        })
    } else {
        Box::new(input)
    }
}

#[cfg(not(debug_assertions))]
fn input_reader(input: File) -> Box<dyn Read + Send> {
    Box::new(input)
}

#[cfg(debug_assertions)]
struct ErrorAfterFirstRead {
    input: File,
    first_read: bool,
}

#[cfg(debug_assertions)]
impl Read for ErrorAfterFirstRead {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.first_read {
            self.first_read = false;
            return self.input.read(buffer);
        }
        Err(io::Error::other("P43 injected read error"))
    }
}

#[derive(Clone)]
struct RestoreState {
    active: Arc<AtomicBool>,
    links: bool,
    output_flags: Arc<Mutex<Option<rustix::fs::OFlags>>>,
}

impl RestoreState {
    fn new(links: bool) -> Self {
        Self {
            links,
            active: Arc::new(AtomicBool::new(false)),
            output_flags: Arc::new(Mutex::new(None)),
        }
    }

    fn nonblocking_output(&self) -> io::Result<()> {
        let stdout = io::stdout();
        let flags = rustix::fs::fcntl_getfl(&stdout)?;
        rustix::fs::fcntl_setfl(&stdout, flags | rustix::fs::OFlags::NONBLOCK)?;
        *self.output_flags.lock().unwrap() = Some(flags);
        Ok(())
    }
    fn activate(&self) {
        self.active.store(true, Ordering::SeqCst);
    }

    fn restore(&self) {
        if self.active.swap(false, Ordering::SeqCst) {
            restore_terminal(self.links);
            if let Some(flags) = self
                .output_flags
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take()
            {
                let _ = rustix::fs::fcntl_setfl(io::stdout(), flags);
            }
        }
    }
}

struct TerminalSession {
    restore: RestoreState,
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        self.restore.restore();
    }
}

fn install_panic_hook(restore: RestoreState) {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        restore.restore();
        previous(info);
    }));
}

fn restore_terminal(links: bool) {
    let mut stdout = io::stdout();
    // Fixed recovery packet is separate from normal byte-credit output.
    // CAN + ST terminate a cancelled partial terminal control before cleanup.
    let _ = stdout.write_all(crate::render::wire::cleanup(
        crate::render::output::OutputOptions {
            allow_osc8: links,
            ..Default::default()
        },
    ));
    let _ = execute!(stdout, EndSynchronizedUpdate);
    let _ = execute!(stdout, Show);
    let _ = execute!(stdout, DisableMouseCapture, LeaveAlternateScreen);
    let _ = disable_raw_mode();
}

#[cfg(debug_assertions)]
fn maybe_panic_for_test() {
    if std::env::var_os(PANIC_TEST_ENV).is_some() {
        panic!("P02 panic recovery self-check");
    }
}

#[cfg(not(debug_assertions))]
fn maybe_panic_for_test() {}

#[cfg(debug_assertions)]
fn maybe_fail_for_test_after_frame() -> io::Result<()> {
    if std::env::var_os(FRAME_ERROR_TEST_ENV).is_some() {
        Err(io::Error::other("P42 frame error recovery self-check"))
    } else {
        Ok(())
    }
}

#[cfg(debug_assertions)]
fn maybe_fail_output_for_test_after_frame() -> io::Result<()> {
    if std::env::var_os(OUTPUT_ERROR_TEST_ENV).is_some() {
        Err(io::Error::other("P60 output error recovery self-check"))
    } else {
        Ok(())
    }
}

#[cfg(not(debug_assertions))]
fn maybe_fail_output_for_test_after_frame() -> io::Result<()> {
    Ok(())
}

#[cfg(not(debug_assertions))]
fn maybe_fail_for_test_after_frame() -> io::Result<()> {
    Ok(())
}
