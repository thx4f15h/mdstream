mod support;

use std::path::Path;
use std::time::Duration;

use mdtui_tui::app::sidebar_width;
use mdtui_tui::screen::StyleRole;
use mdtui_tui::theme::MONOKAI;
use portable_pty::{CommandBuilder, PtySize};
use support::pty::{self, PTY_80X24, PTY_120X40};
use support::temp::TempWorkspace;
use support::terminal::{CellView, TerminalScreen};
use vt100::Color;

const BINARY: &str = env!("CARGO_BIN_EXE_mdtui-tui");
const ENTER_ALTERNATE_SCREEN: &[u8] = b"\x1b[?1049h";
const LEAVE_ALTERNATE_SCREEN: &[u8] = b"\x1b[?1049l";
const HIDE_CURSOR: &[u8] = b"\x1b[?25l";
const SHOW_CURSOR: &[u8] = b"\x1b[?25h";
const CLEAR: &[u8] = b"\x1b[2J";
const HOME: &[u8] = b"\x1b[H";
const PAGE_DOWN: &[u8] = b"\x1b[6~";
const PAGE_UP: &[u8] = b"\x1b[5~";
const ALT_LEFT: &[u8] = b"\x1b[1;3D";
const ALT_RIGHT: &[u8] = b"\x1b[1;3C";
const COMMAND_PAUSE: Duration = Duration::from_millis(120);

fn fixture_source() -> String {
    let mut source = String::from(concat!(
        "# TOP_MARK\n\n",
        "[jump](#target_mark) [web](https://example.com)\n\n",
        "WRAP_MARK paragraph with words that will reflow when the body becomes narrower for the sidebar mode and remain visible above the marker.\n\n",
    ));
    for index in 0..40 {
        source.push_str(&format!(
            "filler paragraph {index:02} with deterministic text for pty scrolling.\n\n"
        ));
    }
    source.push_str("# SECTION_ONE\n\nsection one body\n\n");
    for index in 0..20 {
        source.push_str(&format!(
            "section one filler {index:02} with deterministic text.\n\n"
        ));
    }
    source.push_str("# SECTION_TWO\n\nsection two body\n\n");
    source.push_str("# TARGET_MARK\n\ntarget body visible after anchor jump\n");
    source
}

fn fixture() -> (TempWorkspace, std::path::PathBuf) {
    let workspace = TempWorkspace::new().expect("temp workspace");
    let file = workspace
        .write("e2e.md", fixture_source())
        .expect("write e2e fixture");
    (workspace, file)
}

fn command(file: &Path, extra: &[&str]) -> CommandBuilder {
    let mut command = pty::command(BINARY);
    command.args(extra);
    command.arg(file);
    command
}

fn sleep() {
    std::thread::sleep(COMMAND_PAUSE);
}

fn position(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn count(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|window| *window == needle)
        .count()
}

fn screen_before_leave(session: &mut pty::PtySession, size: PtySize) -> TerminalScreen {
    session.pump_output();
    let end = position(session.output(), LEAVE_ALTERNATE_SCREEN)
        .unwrap_or_else(|| session.output().len());
    let mut terminal = TerminalScreen::new(size.rows, size.cols);
    terminal.feed(&session.output()[..end]);
    terminal
}

fn region_text(screen: &TerminalScreen, x: u16, width: u16) -> String {
    let mut output = String::new();
    for row in 0..screen.size().0 {
        for column in x..x.saturating_add(width) {
            output.push_str(&screen.cell(row, column).expect("region cell").text);
        }
        output.push('\n');
    }
    output
}

fn region_contains(region: &str, needle: &str) -> bool {
    region.contains(needle)
}

fn marker_line_width(region: &str, marker: &str) -> usize {
    region
        .lines()
        .find(|line| line.contains(marker))
        .map_or(0, |line| line.trim_end().chars().count())
}

fn body(screen: &TerminalScreen, cols: u16, sidebar_visible: bool) -> String {
    let x = if sidebar_visible {
        sidebar_width(cols).expect("sidebar width")
    } else {
        0
    };
    region_text(screen, x, cols.saturating_sub(x))
}

fn sidebar(screen: &TerminalScreen, cols: u16) -> String {
    let Some(width) = sidebar_width(cols) else {
        return String::new();
    };
    region_text(screen, 0, width)
}

fn assert_lifecycle(output: &[u8]) {
    let enter = position(output, ENTER_ALTERNATE_SCREEN).expect("alternate enter");
    let hide = position(output, HIDE_CURSOR).expect("cursor hide");
    let show = position(output, SHOW_CURSOR).expect("cursor show");
    let leave = position(output, LEAVE_ALTERNATE_SCREEN).expect("alternate leave");
    assert!(enter < hide && hide < show && show < leave);
    assert_eq!(count(output, CLEAR), 1, "clear must happen once");
    assert_eq!(count(output, HOME), 1, "bare home must happen once");
}

fn wait_for_body_marker(
    session: &mut pty::PtySession,
    size: PtySize,
    marker: &str,
    sidebar_visible: bool,
) {
    for _ in 0..20 {
        sleep();
        let screen = screen_before_leave(session, size);
        if region_contains(&body(&screen, size.cols, sidebar_visible), marker) {
            return;
        }
    }
    panic!("body marker {marker:?} did not appear");
}

fn run_basic_scenario(size: PtySize) {
    let (_workspace, file) = fixture();
    let mut session = pty::spawn(command(&file, &[]), size).expect("spawn e2e TUI");
    session
        .read_until(b"TOP_MARK", Duration::from_secs(5))
        .expect("first frame marker");
    assert!(session.output().windows(CLEAR.len()).any(|w| w == CLEAR));

    let initial = screen_before_leave(&mut session, size);
    assert!(region_contains(
        &body(&initial, size.cols, false),
        "TOP_MARK"
    ));
    // The heading can arrive before the wrap row in another PTY read. Wait
    // for the compared paragraph's tail before taking its initial width.
    session
        .read_until(b"marker.", Duration::from_secs(5))
        .expect("first wrap paragraph complete");
    let initial = screen_before_leave(&mut session, size);
    let initial_wrap_width = marker_line_width(&body(&initial, size.cols, false), "WRAP_MARK");

    for _ in 0..3 {
        session.send(PAGE_DOWN).expect("PageDown");
        sleep();
    }
    let paged = screen_before_leave(&mut session, size);
    assert!(!region_contains(
        &body(&paged, size.cols, false),
        "TOP_MARK"
    ));
    for _ in 0..3 {
        session.send(PAGE_UP).expect("PageUp");
        sleep();
    }
    let restored = screen_before_leave(&mut session, size);
    assert!(region_contains(
        &body(&restored, size.cols, false),
        "TOP_MARK"
    ));

    session.send(b"s").expect("open sidebar");
    sleep();
    let opened = screen_before_leave(&mut session, size);
    assert!(region_contains(&body(&opened, size.cols, true), "TOP_MARK"));
    assert!(region_contains(&sidebar(&opened, size.cols), "TOP_MARK"));
    let opened_wrap_width = marker_line_width(&body(&opened, size.cols, true), "WRAP_MARK");
    assert!(
        opened_wrap_width < initial_wrap_width,
        "body width did not reflow: initial={initial_wrap_width} opened={opened_wrap_width}"
    );

    session
        .send(b"\t\x1b[B\r")
        .expect("sidebar select and activate");
    wait_for_body_marker(&mut session, size, "SECTION_ONE", true);
    session.send(b"\x1b[B\r").expect("select next heading");
    wait_for_body_marker(&mut session, size, "SECTION_TWO", true);

    session.send(b"s").expect("close sidebar");
    sleep();
    let closed = screen_before_leave(&mut session, size);
    assert!(region_contains(
        &body(&closed, size.cols, false),
        "SECTION_TWO"
    ));
    session.send(b"s").expect("reopen sidebar");
    sleep();
    let reopened = screen_before_leave(&mut session, size);
    assert!(region_contains(
        &body(&reopened, size.cols, true),
        "SECTION_TWO"
    ));
    assert!(region_contains(
        &sidebar(&reopened, size.cols),
        "SECTION_TWO"
    ));

    session.send(ALT_LEFT).expect("history back");
    wait_for_body_marker(&mut session, size, "SECTION_ONE", true);
    session.send(ALT_RIGHT).expect("history forward");
    wait_for_body_marker(&mut session, size, "SECTION_TWO", true);

    let other = if size.cols == PTY_120X40.cols {
        PTY_80X24
    } else {
        PTY_120X40
    };
    session.resize(other).expect("resize away");
    sleep();
    session.resize(size).expect("resize back");
    sleep();
    let resized = screen_before_leave(&mut session, size);
    assert!(region_contains(
        &body(&resized, size.cols, true),
        "SECTION_TWO"
    ));

    session.send(b"q").expect("q");
    let status = session.wait(Duration::from_secs(5)).expect("quit");
    assert!(status.success());
    session
        .read_until_eof(Duration::from_secs(2))
        .expect("pty eof");
    assert_lifecycle(session.output());
    let final_screen = screen_before_leave(&mut session, size);
    assert!(region_contains(
        &body(&final_screen, size.cols, true),
        "SECTION_TWO"
    ));
}

#[cfg(unix)]
#[test]
fn pty_120x40_start_scroll_sidebar_directory_history_and_resize() {
    run_basic_scenario(PTY_120X40);
}

#[cfg(unix)]
#[test]
fn pty_80x24_start_scroll_sidebar_directory_history_and_resize() {
    run_basic_scenario(PTY_80X24);
}

fn run_link_scenario(extra: &[&str], check_no_links: bool) {
    let (_workspace, file) = fixture();
    let mut session = pty::spawn(command(&file, extra), PTY_120X40).expect("spawn link TUI");
    session
        .read_until(b"TOP_MARK", Duration::from_secs(5))
        .expect("link first frame");
    assert!(!session.output().is_empty());

    session.send(b"\t").expect("focus first link");
    sleep();
    sleep();
    let mut reached = false;
    for _ in 0..20 {
        session.send(b"\r").expect("activate link");
        for _ in 0..4 {
            sleep();
            let screen = screen_before_leave(&mut session, PTY_120X40);
            if region_contains(&body(&screen, PTY_120X40.cols, false), "TARGET_MARK") {
                reached = true;
                break;
            }
        }
        if reached {
            break;
        }
    }
    assert!(reached, "fragment target did not become visible");
    if check_no_links {
        assert!(
            !contains(session.output(), b"\x1b]8;"),
            "NoOsc8 output contained OSC8"
        );
    }
    session.send(b"q").expect("quit link TUI");
    let status = session.wait(Duration::from_secs(5)).expect("link quit");
    assert!(status.success());
    session
        .read_until_eof(Duration::from_secs(2))
        .expect("link eof");
    assert_lifecycle(session.output());
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[cfg(unix)]
#[test]
fn pty_internal_fragment_focus_jumps_to_target() {
    run_link_scenario(&[], false);
}

#[cfg(unix)]
#[test]
fn pty_no_links_keeps_internal_focus_without_osc8() {
    run_link_scenario(&["--no-links"], true);
}

fn find_body_cell(screen: &TerminalScreen, x: u16, width: u16, needle: &str) -> Option<CellView> {
    for row in 0..screen.size().0 {
        let mut text = String::new();
        for column in x..x.saturating_add(width) {
            let cell = screen.cell(row, column).expect("body cell");
            if cell.text.is_empty() {
                text.push(' ');
            } else {
                text.push_str(&cell.text);
            }
        }
        if let Some(byte) = text.find(needle) {
            let column = x + u16::try_from(text[..byte].chars().count()).ok()?;
            return screen.cell(row, column);
        }
    }
    None
}

#[cfg(unix)]
#[test]
fn pty_external_link_focus_does_not_launch_or_move_body() {
    let (_workspace, file) = fixture();
    let mut session = pty::spawn(command(&file, &[]), PTY_120X40).expect("spawn external TUI");
    session
        .read_until(b"TOP_MARK", Duration::from_secs(5))
        .expect("external first frame");
    session.send(b"\t").expect("focus first link");
    sleep();
    session.send(b"\t").expect("focus external link");
    sleep();
    let screen = screen_before_leave(&mut session, PTY_120X40);
    let cell = find_body_cell(&screen, 0, PTY_120X40.cols, "web").expect("external link cell");
    let resolved = MONOKAI.resolve(StyleRole::LINK_FOCUS);
    let expected_background = resolved.background.map_or(Color::Default, |rgb| {
        Color::Rgb(rgb.red, rgb.green, rgb.blue)
    });
    assert_eq!(cell.background, expected_background);
    let top = body(&screen, PTY_120X40.cols, false);

    session.send(b"\r").expect("activate default external");
    sleep();
    let after = screen_before_leave(&mut session, PTY_120X40);
    assert_eq!(body(&after, PTY_120X40.cols, false), top);
    session.send(b"q").expect("quit external TUI");
    let status = session.wait(Duration::from_secs(5)).expect("external quit");
    assert!(status.success());
    session
        .read_until_eof(Duration::from_secs(2))
        .expect("external eof");
    assert_lifecycle(session.output());
}

#[cfg(unix)]
#[test]
fn pty_q_exits_and_restores_while_fifo_writer_stays_open_without_eof() {
    use std::io::Write;
    let workspace = TempWorkspace::new().unwrap();
    let fifo = workspace.path().join("paused.fifo");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    let mut session = pty::spawn(command(&fifo, &[]), PTY_80X24).unwrap();
    let mut writer = std::fs::OpenOptions::new().write(true).open(&fifo).unwrap();
    writer
        .write_all(b"# FIFO_READY\n\nbody before a paused writer\n")
        .unwrap();
    writer.flush().unwrap();
    session
        .read_until(b"FIFO_READY", Duration::from_secs(5))
        .expect("first frame before EOF");
    session.send(b"q").unwrap();
    let status = session
        .wait(Duration::from_secs(5))
        .expect("q must not wait for a blocked FIFO read");
    assert!(status.success());
    session.read_until_eof(Duration::from_secs(2)).unwrap();
    assert_lifecycle(session.output());
    // Keep the writer alive until after child exit; EOF cannot explain success.
    drop(writer);
}
