mod support;

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::time::Duration;

use support::env::fixed_std_command;
use support::pty::{self, PTY_80X24, PTY_120X40, PTY_160X50};
use support::temp::TempWorkspace;

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

fn assert_lifecycle(output: &[u8]) {
    let enter = position(output, ENTER_ALTERNATE_SCREEN);
    let hide = position(output, HIDE_CURSOR);
    let show = position(output, SHOW_CURSOR);
    let leave = position(output, LEAVE_ALTERNATE_SCREEN);
    assert!(enter < hide, "alternate screen must precede cursor hide");
    assert!(hide < show, "cursor show must follow cursor hide");
    assert!(
        show < leave,
        "cursor show must precede alternate-screen leave"
    );
}

#[test]
fn help_lists_modes_links_and_exit_codes() {
    let output = run_args(["--help"]);
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    let help = String::from_utf8(output.stdout).expect("help must be UTF-8");
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
        "--once does not emit reference ANSI yet",
    ] {
        assert!(help.contains(expected), "help must contain {expected:?}");
    }
}

#[test]
fn once_runs_without_tty_and_emits_no_output() {
    let (_workspace, file) = fixture();
    let output = run_args([
        OsStr::new("--once"),
        OsStr::new("--width"),
        OsStr::new("80"),
        OsStr::new("--height"),
        OsStr::new("24"),
        file.as_os_str(),
    ]);
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
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
fn pty_q_exits_cleanly_and_restores_terminal_after_resize() {
    let (_workspace, file) = fixture();
    let mut session =
        pty::spawn(pty_command(&file), PTY_120X40).expect("TUI binary must start in a PTY");
    session
        .read_until(HIDE_CURSOR, Duration::from_secs(5))
        .expect("TUI must enter its alternate screen and hide the cursor");
    session
        .resize(PTY_80X24)
        .expect("first PTY resize must succeed");
    session
        .resize(PTY_160X50)
        .expect("second PTY resize must succeed");
    std::thread::sleep(Duration::from_millis(20));
    session.send(b"q").expect("q must be sent to the TUI");

    let status = session
        .wait(Duration::from_secs(5))
        .expect("TUI must exit after q");
    assert!(status.success());
    session
        .read_until_eof(Duration::from_secs(2))
        .expect("PTY reader must reach EOF");
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
