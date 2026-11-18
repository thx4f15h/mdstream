mod support;
#[cfg(unix)]
mod unix {
    use super::support::{pty, temp::TempWorkspace};
    use std::time::Duration;
    fn command(file: &std::path::Path) -> portable_pty::CommandBuilder {
        let executable = |name: &str| {
            std::env::split_paths(&std::env::var_os("PATH").unwrap())
                .map(|p| p.join(name))
                .find(|p| p.is_file())
                .unwrap()
        };
        let mut command = pty::command(executable("bash"));
        command.args(["-c",r#"before=$("$3" -g); "$1" "$2"; result=$?; after=$("$3" -g); if [ "$before" = "$after" ]; then printf '\r\nMATH_TERMIOS_OK\r\n'; else printf '\r\nMATH_TERMIOS_BAD\r\n'; fi; exit "$result""#,"math-pty",env!("CARGO_BIN_EXE_mdtui-tui")]);
        command.arg(file);
        command.arg(executable("stty"));
        command.env("COLUMNS", "80");
        command.env("LINES", "24");
        command
    }
    #[test]
    fn real_pty_fraction_grid_search_and_q_restore_terminal() {
        let workspace = TempWorkspace::new().unwrap();
        let file=workspace.write("math.md","A \\(\\frac{a}{b}\\) Z\n\n\\[\n\\left(\\frac{\\alpha{}́}{中}\\right)\n\\]\n\nAFTER_MATH\n").unwrap();
        let mut session = pty::spawn(command(&file), pty::PTY_80X24).unwrap();
        session
            .read_until(b"AFTER_MATH", Duration::from_secs(30))
            .unwrap();
        let mut terminal = vt100::Parser::new(
            std::num::NonZeroU16::new(24).unwrap(),
            std::num::NonZeroU16::new(80).unwrap(),
            0,
        );
        terminal.process(session.output());
        let content = terminal.screen().contents();
        let lines: Vec<_> = content.lines().collect();
        let row = lines
            .iter()
            .position(|l| l.contains("A ─── Z"))
            .expect("actual baseline");
        let column = lines[row].find('A').unwrap();
        assert_eq!(
            terminal
                .screen()
                .cell(row as u16 - 1, column as u16 + 3)
                .unwrap()
                .contents(),
            "a"
        );
        assert_eq!(
            terminal
                .screen()
                .cell(row as u16 + 1, column as u16 + 3)
                .unwrap()
                .contents(),
            "b"
        );
        assert!(content.contains("ά"));
        assert!(content.contains('中'));
        assert!(!content.contains("frac"));
        session.send("/ά\r".as_bytes()).unwrap();
        session
            .read_until(b"match", Duration::from_secs(30))
            .unwrap_or_else(|e| panic!("{e}: {:?}", String::from_utf8_lossy(session.output())));
        session.send(b"q").unwrap();
        assert!(session.wait(Duration::from_secs(3)).unwrap().success());
        session
            .read_until(b"MATH_TERMIOS_OK", Duration::from_secs(3))
            .unwrap();
        std::str::from_utf8(session.output()).unwrap();
    }
    #[test]
    fn generated_greek_and_a_long_post_tex_egc_are_one_wire_glyph() {
        let workspace = TempWorkspace::new().unwrap();
        let marks = "\u{301}".repeat(10_000);
        let glyph = format!("α{marks}");
        let file = workspace
            .write(
                "long-math.md",
                format!("\\[\n\\alpha{{}}{marks}\n\\]\n\nAFTER_MATH_CLUSTER\n"),
            )
            .unwrap();
        let mut session = pty::spawn(command(&file), pty::PTY_80X24).unwrap();
        session
            .read_until(b"AFTER_MATH_CLUSTER", Duration::from_secs(30))
            .unwrap();
        assert_eq!(
            session
                .output()
                .windows(glyph.len())
                .filter(|s| *s == glyph.as_bytes())
                .count(),
            1
        );
        std::str::from_utf8(session.output()).unwrap();
        session.send(b"q").unwrap();
        assert!(session.wait(Duration::from_secs(3)).unwrap().success());
        session
            .read_until(b"MATH_TERMIOS_OK", Duration::from_secs(3))
            .unwrap();
    }
    struct PtyScreen {
        parser: vt100::Parser,
        read: usize,
    }
    impl PtyScreen {
        fn new(size: portable_pty::PtySize) -> Self {
            Self {
                parser: vt100::Parser::new(
                    size.rows.try_into().unwrap(),
                    size.cols.try_into().unwrap(),
                    0,
                ),
                read: 0,
            }
        }
        fn wait(
            &mut self,
            session: &mut pty::PtySession,
            size: portable_pty::PtySize,
            predicate: impl Fn(&str) -> bool,
        ) -> String {
            self.parser
                .set_size(size.rows.try_into().unwrap(), size.cols.try_into().unwrap());
            let previous_read = self.read;
            let deadline = std::time::Instant::now() + Duration::from_secs(30);
            loop {
                session.pump_output();
                self.parser.process(&session.output()[self.read..]);
                self.read = session.output().len();
                let text = self.parser.screen().contents();
                if self.read > previous_read
                    && predicate(&text)
                    && session.output().ends_with(b"\x1b[?2026l")
                {
                    return text;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "math PTY timeout: {text}; tail={:?}",
                    String::from_utf8_lossy(
                        &session.output()[session.output().len().saturating_sub(512)..]
                    )
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
    #[test]
    fn real_pty_tall_matrix_wide_fraction_table_resize_end_and_post_tex_search() {
        let workspace = TempWorkspace::new().unwrap();
        let matrix = (0..40)
            .map(|i| format!("r{i}"))
            .collect::<Vec<_>>()
            .join(r"\\");
        let file = workspace.write("math-navigation.md", format!(
            "\\[\n\\frac{{\\alpha}}{{{}}}\n\\]\n\n\\[\n\\begin{{matrix}}{matrix}\\end{{matrix}}\n\\]\n\n| a | b |\n| - | - |\n| \\({}\\xi\\) | z |\n\nMATH_END\n", "x".repeat(100), "x".repeat(20_000)
        )).unwrap();
        let mut session = pty::spawn(command(&file), pty::PTY_80X24).unwrap();
        let mut screen = PtyScreen::new(pty::PTY_80X24);
        let initial = screen.wait(&mut session, pty::PTY_80X24, |s| s.contains("r0"));
        assert!(
            initial.contains('α') && !initial.contains("frac"),
            "{initial}"
        );
        session.send(b"/r35\r").unwrap();
        let matrix = screen.wait(&mut session, pty::PTY_80X24, |s| {
            s.lines().next().is_some_and(|l| l.contains("r35"))
        });
        assert!(!matrix.contains("begin"));
        session.send(b"\x1b[F").unwrap();
        screen.wait(&mut session, pty::PTY_80X24, |s| {
            s.contains("MATH_END") && s.contains("FOLLOW")
        });
        session.resize(pty::PTY_20X3).unwrap();
        screen.wait(&mut session, pty::PTY_20X3, |s| s.contains("MATH_END"));
        // Slash edits the previous query; replace its three ASCII scalars.
        session.send(b"/\x7f\x7f\x7f").unwrap();
        session.send("ξ\r".as_bytes()).unwrap();
        let narrow = screen.wait(&mut session, pty::PTY_20X3, |s| {
            s.lines().take(2).any(|l| l.contains('ξ'))
        });
        assert!(narrow.lines().take(2).any(|l| l.contains('ξ')), "{narrow}");
        session.resize(pty::PTY_80X24).unwrap();
        screen.wait(&mut session, pty::PTY_80X24, |s| {
            s.lines().take(23).any(|l| l.contains('ξ'))
        });
        session.send(b"q").unwrap();
        assert!(session.wait(Duration::from_secs(3)).unwrap().success());
        session
            .read_until(b"MATH_TERMIOS_OK", Duration::from_secs(3))
            .unwrap();
    }
    #[test]
    fn real_pty_operator_search_visible_and_raw_clipboard_keep_command_atom() {
        let workspace = TempWorkspace::new().unwrap();
        let file = workspace.write("math-copy.md", "\\(\\sin\\)\n").unwrap();
        let size = portable_pty::PtySize {
            rows: 4,
            cols: 20,
            pixel_width: 0,
            pixel_height: 0,
        };
        let mut session = pty::spawn(command(&file), size).unwrap();
        let mut screen = PtyScreen::new(size);
        screen.wait(&mut session, size, |s| {
            s.lines().next().is_some_and(|l| l == "sin")
        });
        session.send(b"/n\r").unwrap();
        let state = screen.wait(&mut session, size, |s| {
            s.contains("/n | match") || s.contains("/n | stale")
        });
        if state.contains("/n | stale") {
            // The first body may commit before the file's host EOF. Resubmit
            // against the current native revision, as the stale UI requests.
            session.send(b"/\x7fn\r").unwrap();
            screen.wait(&mut session, size, |s| s.contains("/n | match"));
        }
        session.send(b"vy").unwrap();
        session
            .read_until(b"\x1b]52;c;c2lu\x07", Duration::from_secs(30))
            .unwrap();
        session.send(b"Y").unwrap();
        session
            .read_until(b"\x1b]52;c;XHNpbg==\x07", Duration::from_secs(30))
            .unwrap();
        session.send(b"q").unwrap();
        assert!(session.wait(Duration::from_secs(3)).unwrap().success());
        session
            .read_until(b"MATH_TERMIOS_OK", Duration::from_secs(3))
            .unwrap();
    }
}
