mod support;
#[cfg(unix)]
mod unix {
    use super::support::{pty, temp::TempWorkspace};
    use portable_pty::PtySize;
    use std::{
        fs::OpenOptions,
        io::Write,
        time::{Duration, Instant},
    };
    fn screen(session: &mut pty::PtySession, size: PtySize) -> String {
        session.pump_output();
        let mut parser = vt100::Parser::new(
            size.rows.try_into().unwrap(),
            size.cols.try_into().unwrap(),
            0,
        );
        parser.process(session.output());
        parser.screen().contents()
    }
    fn wait(
        session: &mut pty::PtySession,
        size: PtySize,
        predicate: impl Fn(&str) -> bool,
    ) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let text = screen(session, size);
            if predicate(&text) {
                return text;
            }
            assert!(Instant::now() < deadline, "PTY timeout: {text}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn packets(session: &mut pty::PtySession, expected: usize) -> Vec<Vec<u8>> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            session.pump_output();
            let mut packets = Vec::new();
            let bytes = session.output();
            let mut start = 0;
            while let Some(at) = bytes[start..]
                .windows(7)
                .position(|part| part == b"\x1b]52;c;")
            {
                let at = start + at + 7;
                let Some(end) = bytes[at..].iter().position(|b| *b == 7) else {
                    break;
                };
                let mut bits = 0_u32;
                let mut count = 0;
                let mut decoded = Vec::new();
                for byte in bytes[at..at + end].iter().take_while(|b| **b != b'=') {
                    let alphabet =
                        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
                    bits = (bits << 6) | alphabet.iter().position(|b| b == byte).unwrap() as u32;
                    count += 6;
                    if count >= 8 {
                        count -= 8;
                        decoded.push((bits >> count) as u8);
                    }
                }
                packets.push(decoded);
                start = at + end + 1;
            }
            if packets.len() >= expected {
                return packets;
            }
            assert!(Instant::now() < deadline, "missing clipboard packets");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    #[test]
    fn actual_cli_selection_clipboard_search_q_navigation_follow_resize_and_held_writer_exit() {
        let workspace = TempWorkspace::new().unwrap();
        let fifo = workspace.path().join("reading.fifo");
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap()
                .success()
        );
        let mut command = pty::command(env!("CARGO_BIN_EXE_mdtui-tui"));
        command.arg(&fifo);
        let mut size = pty::PTY_80X24;
        let mut session = pty::spawn(command, size).unwrap();
        let mut writer = OpenOptions::new().write(true).open(&fifo).unwrap();
        writer
            .write_all(
                format!(
                    "a **b&amp;c** d\n\nq FIRST\n\n{}q SECOND\n\n{}TAIL\n",
                    "filler\n\n".repeat(30),
                    "ending\n\n".repeat(30)
                )
                .as_bytes(),
            )
            .unwrap();
        writer.flush().unwrap();
        wait(&mut session, size, |s| s.starts_with("a b&c d"));
        session.send(b"v ").unwrap();
        wait(&mut session, size, |s| s.contains("SELECT EXTEND 0..1"));
        for _ in 0..6 {
            session.send(b"\x1b[C").unwrap();
            // Each transition is observed before sending the next one.
            let deadline = Instant::now() + Duration::from_secs(5);
            let before = session.output().len();
            while session.output().len() == before {
                session.pump_output();
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        wait(&mut session, size, |s| s.contains("0..15"));
        session.send(b"Y").unwrap();
        assert_eq!(packets(&mut session, 1)[0], b"a **b&amp;c** d");
        wait(&mut session, size, |s| s.contains("clipboard request sent"));
        session.send(b"y").unwrap();
        assert_eq!(packets(&mut session, 2)[1], b"a b&c d");
        session.send(b"\x1b").unwrap();
        wait(&mut session, size, |s| s.contains("cancelled"));
        session.send(b"/q").unwrap();
        wait(&mut session, size, |s| s.contains("/q | Enter: search"));
        session.send(b"\r").unwrap();
        wait(&mut session, size, |s| s.starts_with("q FIRST"));
        session.send(b"n").unwrap();
        wait(&mut session, size, |s| s.starts_with("q SECOND"));
        session.send(b"N").unwrap();
        wait(&mut session, size, |s| s.starts_with("q FIRST"));
        size.cols = 40;
        size.rows = 12;
        session.resize(size).unwrap();
        wait(&mut session, size, |s| s.starts_with("q FIRST"));
        session.send(b"\x1b[F").unwrap();
        wait(&mut session, size, |s| {
            s.contains("FOLLOW") && s.contains("TAIL")
        });
        session.send(b"\x1b[A").unwrap();
        wait(&mut session, size, |s| !s.contains("FOLLOW"));
        writer.write_all(b"\nLATE\n").unwrap();
        writer.flush().unwrap();
        wait(&mut session, size, |s| s.contains("stale"));
        session.send(b"\x1b").unwrap();
        wait(&mut session, size, |s| s.contains("cancelled"));
        session.send(b"q").unwrap();
        assert!(session.wait(Duration::from_secs(3)).unwrap().success());
        session.read_until_eof(Duration::from_secs(3)).unwrap();
        assert!(session.output().windows(8).any(|s| s == b"\x1b[?1049l"));
        assert!(session.output().windows(6).any(|s| s == b"\x1b[?25h"));
        drop(writer);
    }
    #[test]
    fn ctrl_c_in_query_input_restores_terminal() {
        let workspace = TempWorkspace::new().unwrap();
        let file = workspace.path().join("query.md");
        std::fs::write(&file, "query ready\n").unwrap();
        let mut command = pty::command(env!("CARGO_BIN_EXE_mdtui-tui"));
        command.arg(file);
        let size = pty::PTY_80X24;
        let mut session = pty::spawn(command, size).unwrap();
        wait(&mut session, size, |s| s.contains("query ready"));
        session.send(b"/qq").unwrap();
        wait(&mut session, size, |s| s.contains("/qq | Enter"));
        session.send(b"\x03").unwrap();
        assert!(session.wait(Duration::from_secs(3)).unwrap().success());
        session.read_until_eof(Duration::from_secs(3)).unwrap();
        assert!(session.output().windows(8).any(|s| s == b"\x1b[?1049l"));
    }
    #[test]
    fn actual_cli_panned_table_selection_copies_clipped_columns_and_exact_source() {
        let workspace = TempWorkspace::new().unwrap();
        let file = workspace.path().join("pan.md");
        std::fs::write(
            &file,
            "|aaaa|bbbb|cccc|dddd|\n|---|---|---|---|\n|AAAA|BBBB|CCCC|DDDD|\n",
        )
        .unwrap();
        let mut command = pty::command(env!("CARGO_BIN_EXE_mdtui-tui"));
        command.args(["--table-min-width", "3", "--table-max-width", "3"]);
        command.arg(file);
        let size = portable_pty::PtySize {
            cols: 20,
            rows: 10,
            pixel_width: 0,
            pixel_height: 0,
        };
        let mut session = pty::spawn(command, size).unwrap();
        wait(&mut session, size, |s| {
            s.contains("aaa") && s.contains("bbb") && s.contains("ccc")
        });
        session.send(b"\x1b[C").unwrap();
        wait(&mut session, size, |s| {
            s.contains("bbb") && s.contains("ccc") && s.contains("ddd") && !s.contains("aaa")
        });
        session.send(b"v ").unwrap();
        wait(&mut session, size, |s| s.contains("SELECT EXTEND 6..7"));
        for _ in 0..8 {
            let before = session.output().len();
            session.send(b"\x1b[C").unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                session.pump_output();
                if session.output().len() > before {
                    break;
                }
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        wait(&mut session, size, |s| s.contains("6..19"));
        session.send(b"y").unwrap();
        assert_eq!(packets(&mut session, 1)[0], b"bbb\tccc\tddd");
        session.send(b"Y").unwrap();
        assert_eq!(packets(&mut session, 2)[1], b"bbbb|cccc|ddd");
        session.send(b"\x1b").unwrap();
        wait(&mut session, size, |s| s.contains("cancelled"));
        session.send(b"/DD\r").unwrap();
        wait(&mut session, size, |s| {
            s.contains("DDD") && !s.contains("BBB")
        });
        session.send(b"q").unwrap();
        assert!(session.wait(Duration::from_secs(3)).unwrap().success());
        session.read_until_eof(Duration::from_secs(3)).unwrap();
        assert!(session.output().windows(8).any(|s| s == b"\x1b[?1049l"));
    }
    #[test]
    fn actual_cli_conditional_style_selection_copy_and_late_definition_share_source_identity() {
        let workspace = TempWorkspace::new().unwrap();
        let fifo = workspace.path().join("conditional-reading.fifo");
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap()
                .success()
        );
        let mut command = pty::command(env!("CARGO_BIN_EXE_mdtui-tui"));
        command.arg(&fifo);
        let size = pty::PTY_80X24;
        let mut session = pty::spawn(command, size).unwrap();
        let mut writer = OpenOptions::new().write(true).open(&fifo).unwrap();
        writer.write_all(b"*foo [bar* baz]\n\n").unwrap();
        writer.flush().unwrap();
        wait(&mut session, size, |s| s.starts_with("foo [bar baz]"));
        session.send(b"v ").unwrap();
        wait(&mut session, size, |s| s.contains("SELECT EXTEND 1..2"));
        for end in [3, 4, 5, 6, 7, 8, 9, 11, 12, 13, 14, 15] {
            session.send(b"\x1b[C").unwrap();
            let status = format!("SELECT EXTEND 1..{end}");
            wait(&mut session, size, |s| s.contains(&status));
        }
        session.send(b"Y").unwrap();
        assert_eq!(packets(&mut session, 1)[0], b"foo [bar* baz]");
        session.send(b"y").unwrap();
        assert_eq!(packets(&mut session, 2)[1], b"foo [bar baz]");
        writer.write_all(b"[bar* baz]: /u\n").unwrap();
        writer.flush().unwrap();
        wait(&mut session, size, |s| {
            s.starts_with("*foo bar* baz") && s.contains("SELECT HIDDEN") && s.contains("1..15")
        });
        session.send(b"y").unwrap();
        assert_eq!(packets(&mut session, 3)[2], b"foo bar* baz");
        session.send(b"Y").unwrap();
        assert_eq!(packets(&mut session, 4)[3], b"foo [bar* baz]");
        session.send(b"\x1b").unwrap();
        wait(&mut session, size, |s| s.contains("cancelled"));
        session.send(b"/bar*\r").unwrap();
        wait(&mut session, size, |s| s.contains("/bar* | match"));
        session.send(b"q").unwrap();
        assert!(session.wait(Duration::from_secs(3)).unwrap().success());
        session.read_until_eof(Duration::from_secs(3)).unwrap();
        assert!(session.output().windows(8).any(|s| s == b"\x1b[?1049l"));
    }

    #[test]
    fn actual_cli_directory_conditional_title_refresh_and_q_editor_ctrl_c() {
        let workspace = TempWorkspace::new().unwrap();
        let fifo = workspace.path().join("conditional-directory.fifo");
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap()
                .success()
        );
        let mut command = pty::command(env!("CARGO_BIN_EXE_mdtui-tui"));
        command.arg(&fifo);
        let size = pty::PTY_80X24;
        let mut session = pty::spawn(command, size).unwrap();
        let mut writer = OpenOptions::new().write(true).open(&fifo).unwrap();
        writer.write_all(b"intro\n\n# *foo [bar* baz]\n\n").unwrap();
        writer.flush().unwrap();
        wait(&mut session, size, |s| s.starts_with("intro"));
        session.send(b"s\t/bar*\r").unwrap();
        wait(&mut session, size, |s| s.contains("names 0"));
        writer.write_all(b"[bar* baz]: /u\n").unwrap();
        writer.flush().unwrap();
        wait(&mut session, size, |s| s.contains("stale"));
        session.send(b"/\r").unwrap();
        wait(&mut session, size, |s| {
            s.contains("names 1") && s.contains("*foo bar* baz")
        });
        session.send(b"/q").unwrap();
        wait(&mut session, size, |s| s.contains("edit 5/256B /bar*q"));
        session.send(b"\x03").unwrap();
        assert!(session.wait(Duration::from_secs(3)).unwrap().success());
        session.read_until_eof(Duration::from_secs(3)).unwrap();
        assert!(session.output().windows(8).any(|s| s == b"\x1b[?1049l"));
        assert!(session.output().windows(6).any(|s| s == b"\x1b[?25h"));
    }
}
