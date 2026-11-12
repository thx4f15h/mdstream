mod support;

#[cfg(unix)]
mod unix {
    use super::support;
    use portable_pty::PtySize;
    use std::{
        fs::OpenOptions,
        io::Write,
        time::{Duration, Instant},
    };
    use support::{pty, temp::TempWorkspace, terminal::TerminalScreen};

    fn screen(session: &mut pty::PtySession, size: PtySize) -> String {
        session.pump_output();
        let mut screen = TerminalScreen::new(size.rows, size.cols);
        screen.feed(session.output());
        let mut text = String::new();
        for row in 0..size.rows {
            for col in 0..size.cols {
                let cell = screen.cell(row, col).unwrap();
                if cell.text.is_empty() {
                    text.push(' ');
                } else {
                    text.push_str(&cell.text);
                }
            }
            text.push('\n');
        }
        text
    }
    fn wait_screen(
        session: &mut pty::PtySession,
        size: PtySize,
        test: impl Fn(&str) -> bool,
    ) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let text = screen(session, size);
            if test(&text) && session.output().ends_with(b"\x1b[?2026l") {
                return text;
            }
            assert!(
                Instant::now() < deadline,
                "screen condition timeout: {text}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn idle_screen(session: &mut pty::PtySession, size: PtySize) -> String {
        let mut last = String::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut stable = Instant::now();
        loop {
            let current = screen(session, size);
            if current != last {
                last = current;
                stable = Instant::now();
            }
            if stable.elapsed() >= Duration::from_millis(150) {
                return last;
            }
            assert!(Instant::now() < deadline, "screen never became idle");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    #[test]
    fn held_fifo_tail_pause_once_reenter_resize_and_q_restore() {
        let workspace = TempWorkspace::new().unwrap();
        let fifo = workspace.path().join("follow.fifo");
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap()
                .success()
        );
        let mut cmd = pty::command(env!("CARGO_BIN_EXE_mdtui-tui"));
        cmd.arg(&fifo);
        let mut size = pty::PTY_80X24;
        let mut session = pty::spawn(cmd, size).unwrap();
        let mut writer = OpenOptions::new().write(true).open(&fifo).unwrap();
        writer
            .write_all(
                format!(
                    "# FIRST_READY\n\n{}FIRST_END\n",
                    "body before tail\n\n".repeat(100)
                )
                .as_bytes(),
            )
            .unwrap();
        writer.flush().unwrap();
        wait_screen(&mut session, size, |s| s.contains("FIRST_READY"));
        session.send(b"\x1b[F").unwrap();
        wait_screen(&mut session, size, |s| {
            s.contains("FIRST_END") && s.contains("FOLLOW")
        });
        writer.write_all(b"\nAFTER_END_UNIQUE\n").unwrap();
        writer.flush().unwrap();
        let followed = wait_screen(&mut session, size, |s| s.contains("AFTER_END_UNIQUE"));
        eprintln!("held writer followed:\n{followed}");
        session.send(b"\x1b[5~").unwrap();
        let paused = wait_screen(&mut session, size, |s| {
            !s.contains("FOLLOW") && !s.contains("AFTER_END_UNIQUE")
        });
        writer
            .write_all(format!("\n{}AFTER_UP_UNIQUE\n", "new offscreen\n\n".repeat(100)).as_bytes())
            .unwrap();
        writer.flush().unwrap();
        let after = idle_screen(&mut session, size);
        assert_eq!(after, paused, "append stole paused viewport");
        session.send(b"\x1b[1;5F").unwrap();
        let once = wait_screen(&mut session, size, |s| {
            s.contains("AFTER_UP_UNIQUE") && !s.contains("FOLLOW")
        });
        writer.write_all(b"\nAFTER_ONCE_UNIQUE\n").unwrap();
        writer.flush().unwrap();
        assert_eq!(
            idle_screen(&mut session, size),
            once,
            "Ctrl+End followed a new input"
        );
        session.send(b"\x1b[F").unwrap();
        wait_screen(&mut session, size, |s| {
            s.contains("AFTER_ONCE_UNIQUE") && s.contains("FOLLOW")
        });
        size = PtySize {
            rows: 30,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        };
        session.resize(size).unwrap();
        wait_screen(&mut session, size, |s| {
            s.contains("AFTER_ONCE_UNIQUE") && s.contains("FOLLOW")
        });
        writer.write_all(b"\nAFTER_RESIZE_UNIQUE\n").unwrap();
        writer.flush().unwrap();
        let resized = wait_screen(&mut session, size, |s| {
            s.contains("AFTER_RESIZE_UNIQUE") && s.contains("FOLLOW")
        });
        eprintln!("resized tail:\n{resized}");
        let start = Instant::now();
        session.send(b"q").unwrap();
        let status = session.wait(Duration::from_secs(2)).unwrap();
        assert!(status.success());
        session.read_until_eof(Duration::from_secs(2)).unwrap();
        assert!(
            session
                .output()
                .windows(b"\x1b[?25h".len())
                .any(|w| w == b"\x1b[?25h")
        );
        assert!(
            session
                .output()
                .windows(b"\x1b[?1049l".len())
                .any(|w| w == b"\x1b[?1049l")
        );
        eprintln!(
            "q with writer still open: {:?}; restored cursor/alternate screen",
            start.elapsed()
        );
        drop(writer);
    }
}
