mod support;

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::time::Duration;

use mdtui_tui::render::frame::{
    BEGIN_SYNCHRONIZED_UPDATE, END_SYNCHRONIZED_UPDATE, FIRST_FRAME_CLEAR,
};
use mdtui_tui::static_render::ONCE_CLEAR;
use support::env::fixed_std_command;
use support::pty::{self, PTY_19X2, PTY_20X3, PTY_80X24, PTY_120X40};
use support::temp::TempWorkspace;
use support::terminal::TerminalScreen;

const BINARY: &str = env!("CARGO_BIN_EXE_mdtui-tui");
const HIDE_CURSOR: &[u8] = b"\x1b[?25l";
const SHOW_CURSOR: &[u8] = b"\x1b[?25h";
const ENTER_ALTERNATE_SCREEN: &[u8] = b"\x1b[?1049h";
const LEAVE_ALTERNATE_SCREEN: &[u8] = b"\x1b[?1049l";

fn run_args<I, S>(args: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = fixed_std_command(BINARY);
    command
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("binary must run")
}

fn fixture() -> (TempWorkspace, PathBuf) {
    let workspace = TempWorkspace::new().expect("temporary workspace must be created");
    let file = workspace
        .write("doc.md", "# P02 fixture\n")
        .expect("fixture must be written");
    (workspace, file)
}

fn long_fixture() -> (TempWorkspace, PathBuf) {
    let workspace = TempWorkspace::new().expect("temporary workspace must be created");
    let mut source = String::from("# TOP_MARK\n\n");
    for index in 0..160 {
        source.push_str(&format!(
            "paragraph {index:03} abcdefghijklmnopqrstuvwxyz\n\n"
        ));
    }
    source.push_str("BOTTOM_MARK\n");
    let file = workspace
        .write("long.md", source)
        .expect("long fixture must be written");
    (workspace, file)
}

fn pty_command(path: &Path) -> portable_pty::CommandBuilder {
    let mut command = pty::command(BINARY);
    command.arg(path);
    command
}

fn position(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
        .unwrap_or_else(|| panic!("missing sequence {needle:?} in PTY output {haystack:?}"))
}

fn rposition(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .rposition(|window| window == needle)
        .unwrap_or_else(|| panic!("missing sequence {needle:?} in PTY output {haystack:?}"))
}

fn count(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|window| *window == needle)
        .count()
}

fn assert_lifecycle(output: &[u8]) {
    let enter = position(output, ENTER_ALTERNATE_SCREEN);
    let hide = position(output, HIDE_CURSOR);
    let show = position(output, SHOW_CURSOR);
    let leave = position(output, LEAVE_ALTERNATE_SCREEN);
    let restore_sync_end = rposition(output, END_SYNCHRONIZED_UPDATE);
    assert!(enter < hide, "alternate screen must precede cursor hide");
    assert!(hide < show, "cursor show must follow cursor hide");
    assert!(
        restore_sync_end < show,
        "terminal restore must close synchronized update before showing the cursor"
    );
    assert!(
        show < leave,
        "cursor show must precede alternate-screen leave"
    );
}

fn assert_single_first_frame_clear(output: &[u8]) {
    assert_eq!(
        count(output, b"\x1b_G"),
        0,
        "default CLI retains alt fallback without a graphics capability probe"
    );
    let enter = position(output, ENTER_ALTERNATE_SCREEN);
    let hide = position(output, HIDE_CURSOR);
    let clear = position(output, FIRST_FRAME_CLEAR);
    let sync_begin = position(output, BEGIN_SYNCHRONIZED_UPDATE);
    let sync_end = position(output, END_SYNCHRONIZED_UPDATE);
    assert!(enter < hide, "alternate screen must precede cursor hiding");
    assert!(hide < clear, "first-frame clear must happen after setup");
    assert!(
        clear < sync_begin,
        "first-frame clear must precede synchronized update"
    );
    assert!(
        sync_begin < sync_end,
        "first-frame synchronized update must close"
    );
    assert_eq!(
        count(output, b"\x1b[2J"),
        1,
        "clear screen must be emitted once"
    );
    assert_eq!(
        count(output, b"\x1b[H"),
        1,
        "bare home must be emitted once"
    );
}

fn assert_size_notice(output: &[u8]) {
    let text = String::from_utf8_lossy(output);
    assert!(
        text.contains("terminal too small"),
        "missing size notice: {text:?}"
    );
    assert!(
        text.contains("minimum 20x3"),
        "missing minimum size: {text:?}"
    );
}

#[test]
fn help_lists_modes_links_and_exit_codes() {
    let output = run_args(["--help"]);
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    let help = String::from_utf8(output.stdout.clone()).expect("help must be UTF-8");
    for expected in [
        "--once",
        "--width <N>",
        "--height <N>",
        "--open-links",
        "--no-links",
        "Disable OSC 8 only",
        "0 success",
        "2 invalid arguments",
        "3 file or terminal I/O error",
        "4 TUI mode requires an interactive terminal",
        "5 static rendering error",
        "default 120",
        "default 40",
        "complete reference ANSI",
        "CRLF line endings",
        "Tab/Shift-Tab",
        "Alt-Left/M-Left",
        "Alt-Right/M-Right",
        "q                      Quit",
        "Ctrl-C                 Quit",
        "Up/Down",
        "PageUp/PageDown",
        "Home/End",
        "Enter                  Activate",
        "at least 20 columns by 3 rows",
        "q or Ctrl-C",
    ] {
        assert!(help.contains(expected), "help must contain {expected:?}");
    }

    let repeated = run_args(["--help"]);
    assert_eq!(repeated.status.code(), Some(0));
    assert_eq!(repeated.stdout, output.stdout);
    assert_eq!(repeated.stderr, output.stderr);
}

#[test]
fn once_without_tty_emits_reference_ansi_deterministically() {
    let (_workspace, file) = fixture();
    let args = [
        OsStr::new("--once"),
        OsStr::new("--width"),
        OsStr::new("80"),
        OsStr::new("--height"),
        OsStr::new("24"),
        file.as_os_str(),
    ];
    let output = run_args(args);
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    assert!(output.stdout.starts_with(ONCE_CLEAR));
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("P02 fixture"));
    assert!(!text.contains("P26 skeleton"));
    assert!(output.stdout.ends_with(b"\r\n"));

    let repeated = run_args(args);
    assert_eq!(repeated.status.code(), Some(0));
    assert_eq!(repeated.stdout, output.stdout);
    assert_eq!(repeated.stderr, output.stderr);
}

#[test]
fn once_default_geometry_is_120_by_40() {
    let workspace = TempWorkspace::new().expect("temporary workspace must be created");
    let wide = workspace
        .write("wide.md", format!("# {}\n", "x".repeat(130)))
        .expect("wide fixture must be written");
    let output = run_args([OsStr::new("--once"), wide.as_os_str()]);
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.starts_with(ONCE_CLEAR));
    assert!(
        output
            .stdout
            .windows(130)
            .any(|window| window.iter().all(|byte| *byte == b'x'))
    );

    let mut tall_source = String::new();
    for paragraph in 0..21 {
        tall_source.push_str(&format!("Paragraph {paragraph}.\n"));
        if paragraph != 20 {
            tall_source.push('\n');
        }
    }
    let tall = workspace
        .write("tall.md", tall_source)
        .expect("tall fixture must be written");
    let overflow = run_args([OsStr::new("--once"), tall.as_os_str()]);
    assert_eq!(overflow.status.code(), Some(5));
    assert!(overflow.stdout.is_empty());
    assert!(String::from_utf8_lossy(&overflow.stderr).contains("exceeds height 40"));
}

#[test]
fn once_custom_width_and_height_are_applied() {
    let workspace = TempWorkspace::new().expect("temporary workspace must be created");
    let file = workspace
        .write("wide.md", "# abcdefghij\n")
        .expect("fixture must be written");
    let output = run_args([
        OsStr::new("--once"),
        OsStr::new("--width"),
        OsStr::new("5"),
        OsStr::new("--height"),
        OsStr::new("1"),
        file.as_os_str(),
    ]);
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.starts_with(ONCE_CLEAR));
    assert!(String::from_utf8_lossy(&output.stdout).contains("abcdefghij"));
}

#[test]
fn once_height_overflow_exits_5_without_truncation() {
    let workspace = TempWorkspace::new().expect("temporary workspace must be created");
    let file = workspace
        .write("tall.md", "one\n\ntwo\n")
        .expect("fixture must be written");
    let output = run_args([
        OsStr::new("--once"),
        OsStr::new("--width"),
        OsStr::new("80"),
        OsStr::new("--height"),
        OsStr::new("1"),
        file.as_os_str(),
    ]);
    assert_eq!(output.status.code(), Some(5));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("exceeds height 1"));
}

#[test]
fn once_replaces_invalid_utf8_without_failing() {
    let workspace = TempWorkspace::new().expect("temporary workspace must be created");
    let file = workspace
        .write("invalid.md", b"# bad \xff\n".as_slice())
        .expect("fixture must be written");
    let output = run_args([
        OsStr::new("--once"),
        OsStr::new("--width"),
        OsStr::new("80"),
        OsStr::new("--height"),
        OsStr::new("4"),
        file.as_os_str(),
    ]);
    assert_eq!(output.status.code(), Some(0));
    let text = String::from_utf8(output.stdout).expect("ANSI output must be UTF-8");
    assert!(text.contains("bad \u{fffd}"), "unexpected output: {text:?}");
}

#[test]
fn invalid_arguments_exit_2() {
    let (_workspace, file) = fixture();
    let output = run_args([OsStr::new("--width"), OsStr::new("0"), file.as_os_str()]);
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("expected integer 1..=1000"));
}

#[test]
fn missing_and_non_regular_files_exit_3() {
    let (workspace, _file) = fixture();
    let missing = workspace.path().join("missing.md");
    let output = run_args([OsStr::new("--once"), missing.as_os_str()]);
    assert_eq!(output.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot open"));

    let directory = workspace.path().join("directory");
    fs::create_dir(&directory).expect("directory fixture must be created");
    let output = run_args([OsStr::new("--once"), directory.as_os_str()]);
    assert_eq!(output.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&output.stderr).contains("not a regular file"));
}

#[cfg(unix)]
#[test]
fn unreadable_file_exits_3() {
    use std::os::unix::fs::PermissionsExt;

    let workspace = TempWorkspace::new().expect("temporary workspace must be created");
    let file = workspace
        .write("unreadable.md", "content\n")
        .expect("fixture must be written");
    let mut permissions = fs::metadata(&file)
        .expect("fixture metadata must exist")
        .permissions();
    permissions.set_mode(0o000);
    fs::set_permissions(&file, permissions).expect("permissions must be updated");
    if fs::File::open(&file).is_ok() {
        return;
    }

    let output = run_args([OsStr::new("--once"), file.as_os_str()]);
    assert_eq!(output.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot open"));
}

#[test]
fn tui_rejects_non_tty_with_exit_4() {
    let (_workspace, file) = fixture();
    let output = run_args([file.as_os_str()]);
    assert_eq!(output.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&output.stderr).contains("requires an interactive terminal"));
}

#[cfg(unix)]
#[test]
fn pty_first_frame_clears_once_and_restores_after_120_80_120_resize() {
    let (_workspace, file) = fixture();
    let mut session =
        pty::spawn(pty_command(&file), PTY_120X40).expect("TUI binary must start in a PTY");
    session
        .read_until(HIDE_CURSOR, Duration::from_secs(5))
        .expect("TUI must enter its alternate screen and hide the cursor");
    session
        .read_until(FIRST_FRAME_CLEAR, Duration::from_secs(5))
        .expect("TUI must clear and home exactly once for its first frame");
    session
        .read_until(b"P02 fixture", Duration::from_secs(5))
        .expect("TUI must stream and display the input file");
    session
        .resize(PTY_80X24)
        .expect("first PTY resize must succeed");
    session
        .resize(PTY_120X40)
        .expect("second PTY resize must succeed");
    std::thread::sleep(Duration::from_millis(50));
    session.send(b"q").expect("q must be sent to the TUI");

    let status = session
        .wait(Duration::from_secs(5))
        .expect("TUI must exit after q");
    assert!(status.success());
    session
        .read_until_eof(Duration::from_secs(2))
        .expect("PTY reader must reach EOF");
    assert_lifecycle(session.output());
    assert_single_first_frame_clear(session.output());
}

#[cfg(unix)]
#[test]
fn pty_page_down_moves_viewport_and_sidebar_key_keeps_frame_protocol() {
    let (_workspace, file) = long_fixture();
    let mut session =
        pty::spawn(pty_command(&file), PTY_120X40).expect("TUI binary must start in a PTY");
    session
        .read_until(b"TOP_MARK", Duration::from_secs(5))
        .expect("first frame must show the top marker");

    for _ in 0..10 {
        session
            .send(b"\x1b[6~")
            .expect("PageDown must be sent to the TUI");
        std::thread::sleep(Duration::from_millis(10));
    }
    session
        .read_until(b"BOTTOM_MARK", Duration::from_secs(5))
        .expect("PageDown must move the viewport to the bottom marker");
    std::thread::sleep(Duration::from_millis(100));
    session
        .read_until(b"BOTTOM_MARK", Duration::from_secs(1))
        .expect("bottom frame output must be drained");
    let sync_before_sidebar = count(session.output(), BEGIN_SYNCHRONIZED_UPDATE);

    session
        .send(b"s")
        .expect("sidebar toggle must be sent to the TUI");
    std::thread::sleep(Duration::from_millis(100));
    session.send(b"q").expect("q must terminate the TUI");
    let status = session
        .wait(Duration::from_secs(5))
        .expect("TUI must exit after q");
    assert!(status.success());
    session
        .read_until_eof(Duration::from_secs(2))
        .expect("PTY reader must reach EOF");

    assert!(
        count(session.output(), BEGIN_SYNCHRONIZED_UPDATE) > sync_before_sidebar,
        "sidebar toggle must request another rendered frame"
    );
    let leave = position(session.output(), LEAVE_ALTERNATE_SCREEN);
    let mut terminal = TerminalScreen::new(PTY_120X40.rows, PTY_120X40.cols);
    terminal.feed(&session.output()[..leave]);
    let sidebar_text: String = (0..PTY_120X40.rows)
        .map(|row| {
            let mut text = String::new();
            for column in 0..36 {
                text.push_str(&terminal.cell(row, column).unwrap().text);
            }
            text
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        sidebar_text.contains("TOP_MARK"),
        "sidebar columns must show the current heading: {sidebar_text:?}"
    );
    assert_lifecycle(session.output());
    assert_single_first_frame_clear(session.output());
}

#[cfg(unix)]
// Fault injection is deliberately absent from the production release binary.
#[cfg(debug_assertions)]
#[test]
fn pty_frame_error_restores_terminal_after_first_frame() {
    let (_workspace, file) = fixture();
    let mut command = pty_command(&file);
    command.env("MDTUI_TEST_FRAME_ERROR_AFTER_FIRST", "1");
    let mut session = pty::spawn(command, PTY_120X40).expect("TUI binary must start in a PTY");

    session
        .read_until(FIRST_FRAME_CLEAR, Duration::from_secs(5))
        .expect("first frame must be written before the injected error");
    let status = session
        .wait(Duration::from_secs(5))
        .expect("TUI must exit after the injected frame error");
    assert_eq!(status.exit_code(), 3);
    session
        .read_until_eof(Duration::from_secs(2))
        .expect("PTY reader must reach EOF");
    assert_lifecycle(session.output());
    assert_single_first_frame_clear(session.output());
    assert!(
        String::from_utf8_lossy(session.output()).contains("P42 frame error recovery self-check")
    );
}

#[cfg(unix)]
// Fault injection is deliberately absent from the production release binary.
#[cfg(debug_assertions)]
#[test]
fn pty_read_error_restores_terminal_after_first_frame() {
    let (_workspace, file) = fixture();
    let mut command = pty_command(&file);
    command.env("MDTUI_TEST_READ_ERROR_AFTER_FIRST", "1");
    let mut session = pty::spawn(command, PTY_120X40).expect("TUI binary must start in a PTY");

    session
        .read_until(b"P02 fixture", Duration::from_secs(5))
        .expect("first file chunk must render before the injected read error");
    let status = session
        .wait(Duration::from_secs(5))
        .expect("TUI must exit after the injected read error");
    assert_eq!(status.exit_code(), 3);
    session
        .read_until_eof(Duration::from_secs(2))
        .expect("PTY reader must reach EOF");
    assert_lifecycle(session.output());
    assert_single_first_frame_clear(session.output());
    assert!(String::from_utf8_lossy(session.output()).contains("P43 injected read error"));
}

#[cfg(unix)]
// Fault injection is deliberately absent from the production release binary.
#[cfg(debug_assertions)]
#[test]
fn pty_output_error_restores_terminal_after_first_frame() {
    let (_workspace, file) = fixture();
    let mut command = pty_command(&file);
    command.env("MDTUI_TEST_OUTPUT_ERROR_AFTER_FIRST", "1");
    let mut session = pty::spawn(command, PTY_120X40).expect("TUI binary must start in a PTY");

    session
        .read_until(FIRST_FRAME_CLEAR, Duration::from_secs(5))
        .expect("first frame must be written before the injected output error");
    let status = session
        .wait(Duration::from_secs(5))
        .expect("TUI must exit after the injected output error");
    assert_eq!(status.exit_code(), 3);
    session
        .read_until_eof(Duration::from_secs(2))
        .expect("PTY reader must reach EOF");
    assert_lifecycle(session.output());
    assert_single_first_frame_clear(session.output());
    assert!(
        String::from_utf8_lossy(session.output()).contains("P60 output error recovery self-check")
    );
}

#[cfg(unix)]
#[test]
fn pty_narrow_terminal_shows_size_notice_and_q_restores() {
    let (_workspace, file) = fixture();
    let mut session =
        pty::spawn(pty_command(&file), PTY_19X2).expect("TUI binary must start in a PTY");
    session
        .read_until(b"minimum 20x3", Duration::from_secs(5))
        .expect("narrow terminal must show the size notice");
    assert_size_notice(session.output());
    assert!(!String::from_utf8_lossy(session.output()).contains("P02 fixture"));

    session
        .send(b"q")
        .expect("q must be sent to the narrow TUI");
    let status = session
        .wait(Duration::from_secs(5))
        .expect("narrow TUI must exit after q");
    assert!(status.success());
    session
        .read_until_eof(Duration::from_secs(2))
        .expect("PTY reader must reach EOF");
    assert_lifecycle(session.output());
    assert_size_notice(session.output());
}

#[cfg(unix)]
#[test]
fn pty_narrow_terminal_supports_ctrl_c_and_restores() {
    let (_workspace, file) = fixture();
    let mut session =
        pty::spawn(pty_command(&file), PTY_19X2).expect("TUI binary must start in a PTY");
    session
        .read_until(b"minimum 20x3", Duration::from_secs(5))
        .expect("narrow terminal must show the size notice");
    session
        .send(b"\x03")
        .expect("Ctrl-C must be sent to the narrow TUI");
    let status = session
        .wait(Duration::from_secs(5))
        .expect("narrow TUI must exit after Ctrl-C");
    assert!(status.success());
    session
        .read_until_eof(Duration::from_secs(2))
        .expect("PTY reader must reach EOF");
    assert_lifecycle(session.output());
    assert_size_notice(session.output());
}

#[cfg(unix)]
#[test]
fn pty_20x3_is_the_supported_minimum_terminal_size() {
    let (_workspace, file) = fixture();
    let mut session =
        pty::spawn(pty_command(&file), PTY_20X3).expect("TUI binary must start in a PTY");
    session
        .read_until(b"P02 fixture", Duration::from_secs(5))
        .expect("20x3 must use the normal TUI path");
    session.send(b"q").expect("q must be sent to the TUI");
    let status = session
        .wait(Duration::from_secs(5))
        .expect("20x3 TUI must exit after q");
    assert!(status.success());
    session
        .read_until_eof(Duration::from_secs(2))
        .expect("PTY reader must reach EOF");
    assert!(!String::from_utf8_lossy(session.output()).contains("terminal too small"));
    assert_lifecycle(session.output());
}

#[cfg(unix)]
#[test]
fn pty_ctrl_c_exits_cleanly_and_restores_terminal() {
    let (_workspace, file) = fixture();
    let mut session =
        pty::spawn(pty_command(&file), PTY_120X40).expect("TUI binary must start in a PTY");
    session
        .read_until(HIDE_CURSOR, Duration::from_secs(5))
        .expect("TUI must enter its alternate screen and hide the cursor");
    session
        .send(b"\x03")
        .expect("Ctrl-C must be sent to the TUI");

    let status = session
        .wait(Duration::from_secs(5))
        .expect("TUI must exit after Ctrl-C");
    assert!(status.success());
    session
        .read_until_eof(Duration::from_secs(2))
        .expect("PTY reader must reach EOF");
    assert_lifecycle(session.output());
}

#[cfg(unix)]
// Fault injection is deliberately absent from the production release binary.
#[cfg(debug_assertions)]
#[test]
fn pty_panic_restores_terminal_before_exit() {
    let (_workspace, file) = fixture();
    let mut command = pty_command(&file);
    command.env("MDTUI_TEST_PANIC_AFTER_ENTER", "1");
    let mut session = pty::spawn(command, PTY_120X40).expect("TUI binary must start in a PTY");

    let status = session
        .wait(Duration::from_secs(5))
        .expect("panicking TUI must exit");
    assert!(!status.success());
    session
        .read_until_eof(Duration::from_secs(2))
        .expect("PTY reader must reach EOF");
    let output = session.text();
    assert!(output.contains("P02 panic recovery self-check"));
    assert_lifecycle(session.output());
}
