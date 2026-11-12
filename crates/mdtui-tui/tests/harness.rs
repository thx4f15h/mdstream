mod support;

use std::path::Path;
use std::time::Duration;

use support::clock::FakeClock;
use support::env::{FIXED_ENV, FixedEnv, fixed_std_command};
use support::snapshot::snapshot_settings;
use support::temp::TempWorkspace;
use support::terminal::TerminalScreen;

#[test]
fn terminal_helper_reads_cells_styles_cursor_and_size() {
    let mut terminal = TerminalScreen::new(3, 12);
    terminal.feed(b"ab\x1b[1;3;4;38;2;255;121;198;48;2;40;40;40mC\x1b[0m\xe4\xbd\xa0");

    assert_eq!(terminal.size(), (3, 12));
    assert_eq!(terminal.cursor(), (0, 5));
    assert_eq!(terminal.text().lines().next(), Some("abC你"));

    let styled = terminal.cell(0, 2).expect("styled cell must exist");
    styled.expect_text("C").expect("cell text must match");
    assert_eq!(styled.foreground, vt100::Color::Rgb(255, 121, 198));
    assert_eq!(styled.background, vt100::Color::Rgb(40, 40, 40));
    assert!(styled.bold);
    assert!(styled.italic);
    assert!(styled.underline);

    let wide = terminal.cell(0, 3).expect("wide cell must exist");
    wide.expect_text("你").expect("wide cell text must match");
    assert!(wide.wide);
    let continuation = terminal.cell(0, 4).expect("wide continuation must exist");
    assert!(continuation.wide_continuation);
}

#[test]
fn harness_failure_self_check_returns_mismatch() {
    let mut terminal = TerminalScreen::new(2, 4);
    terminal.feed(b"x");
    let cell = terminal.cell(0, 0).expect("cell must exist");
    let error = cell
        .expect_text("y")
        .expect_err("deliberate mismatch must return an error");

    assert!(error.contains("expected \"y\""));
    assert!(error.contains("actual \"x\""));
}

#[test]
fn snapshot_settings_are_deterministic() {
    let settings = snapshot_settings();
    assert_eq!(settings.snapshot_path(), Path::new("snapshots"));
    assert!(!settings.prepend_module_to_snapshot());
    assert!(settings.omit_expression());

    let mut terminal = TerminalScreen::new(3, 12);
    terminal.feed(b"ab\x1b[1;3;4;38;2;255;121;198;48;2;40;40;40mC\x1b[0m\xe4\xbd\xa0");
    let dump = terminal.dump();
    settings.bind(|| insta::assert_snapshot!("vt100_screen", dump));
}

#[test]
fn fake_clock_advances_monotonically() {
    let mut clock = FakeClock::new(Duration::from_millis(100));
    assert_eq!(clock.now(), Duration::from_millis(100));
    assert_eq!(
        clock.advance(Duration::from_millis(25)),
        Duration::from_millis(125)
    );
    assert_eq!(
        clock.advance(Duration::from_millis(75)),
        Duration::from_millis(200)
    );
    assert_eq!(clock.now(), Duration::from_millis(200));
}

#[test]
fn temporary_workspace_writes_relative_fixtures() {
    let workspace = TempWorkspace::new().expect("temporary workspace must be created");
    let fixture = workspace
        .write("spec/example.md", "# fixture\n")
        .expect("relative fixture must be written");

    assert!(fixture.starts_with(workspace.path()));
    assert_eq!(
        std::fs::read_to_string(fixture).expect("fixture must be readable"),
        "# fixture\n"
    );
    assert!(workspace.write("/absolute.md", "invalid").is_err());
}

#[test]
fn fixed_environment_is_applied_to_spawned_commands() {
    assert_eq!(
        FIXED_ENV,
        [
            ("TERM", "xterm-256color"),
            ("COLORTERM", "truecolor"),
            ("LANG", "C.UTF-8"),
            ("COLUMNS", "120"),
            ("LINES", "40"),
        ]
    );

    let mut command = portable_pty::CommandBuilder::new("/bin/sh");
    FixedEnv::configure_pty(&mut command);
    assert_eq!(
        command.get_env("TERM"),
        Some(std::ffi::OsStr::new("xterm-256color"))
    );
    assert_eq!(command.get_env("NO_COLOR"), None);
}

#[cfg(unix)]
#[test]
fn fixed_environment_is_isolated_from_the_test_process() {
    let mut command = fixed_std_command("/bin/sh");
    command.args([
        "-c",
        r#"printf '%s|%s|%s|%s|%s|%s' "$TERM" "$COLORTERM" "$LANG" "$COLUMNS" "$LINES" "${NO_COLOR-unset}""#,
    ]);
    let output = command
        .output()
        .expect("fixed environment command must run");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("environment output must be UTF-8"),
        "xterm-256color|truecolor|C.UTF-8|120|40|unset"
    );
}

#[cfg(unix)]
#[test]
fn pty_self_check_launches_current_binary_and_resizes() {
    let workspace = TempWorkspace::new().expect("temporary workspace must be created");
    let file = workspace
        .write("p03-self-check.md", "# P03 self-check\n")
        .expect("P03 fixture must be written");
    let mut command = support::pty::current_binary_command();
    command.arg("--once");
    command.arg(file);
    let mut session = support::pty::spawn(command, support::pty::PTY_120X40)
        .expect("current crate binary must start in a PTY");

    session
        .resize(support::pty::PTY_80X24)
        .expect("PTY resize must succeed");
    let status = session
        .wait(Duration::from_secs(5))
        .expect("current crate binary must exit");
    assert!(status.success());
    session
        .read_until_eof(Duration::from_secs(2))
        .expect("PTY reader must reach EOF");
    session.terminate().expect("PTY cleanup must succeed");
}

#[cfg(unix)]
#[test]
fn pty_self_check_writes_reads_and_resizes() {
    let mut command = support::pty::command("/bin/sh");
    command.args([
        "-c",
        r#"printf 'ready\n'; IFS= read -r line; printf 'got:%s\n' "$line""#,
    ]);
    let mut session =
        support::pty::spawn(command, support::pty::PTY_120X40).expect("shell must start in a PTY");

    session
        .read_until(b"ready", Duration::from_secs(5))
        .expect("PTY output must become readable");
    session
        .resize(support::pty::PTY_80X24)
        .expect("PTY resize must succeed");
    session.send(b"hello\n").expect("PTY write must succeed");
    let status = session
        .wait(Duration::from_secs(5))
        .expect("shell must exit after input");
    assert!(status.success());
    session
        .read_until_eof(Duration::from_secs(2))
        .expect("PTY reader must reach EOF");
    assert!(session.text().contains("got:hello"));
}

#[cfg(unix)]
#[test]
fn pty_self_check_terminates_running_child() {
    let mut command = support::pty::command("/bin/sh");
    command.args(["-c", "IFS= read -r line"]);
    let mut session = support::pty::spawn(command, support::pty::PTY_120X40)
        .expect("blocking shell must start in a PTY");

    session
        .terminate()
        .expect("running PTY child must terminate");
    session
        .terminate()
        .expect("PTY termination must be idempotent");
}
