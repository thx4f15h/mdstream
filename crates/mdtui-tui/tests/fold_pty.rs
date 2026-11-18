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

    fn wait(
        session: &mut pty::PtySession,
        size: PtySize,
        predicate: impl Fn(&str) -> bool,
    ) -> String {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            session.pump_output();
            let mut parser = vt100::Parser::new(
                size.rows.try_into().unwrap(),
                size.cols.try_into().unwrap(),
                0,
            );
            parser.process(session.output());
            let text = parser.screen().contents();
            if predicate(&text) && session.output().ends_with(b"\x1b[?2026l") {
                return text;
            }
            assert!(Instant::now() < deadline, "fold PTY timeout: {text}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn real_fifo_fold_menu_deep_batch_stream_backpressure_resize_hit_activation_and_q() {
        let workspace = TempWorkspace::new().unwrap();
        let fifo = workspace.path().join("fold.fifo");
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
            .write_all(format!("{}DEEP_READY\n\n", "> ".repeat(600)).as_bytes())
            .unwrap();
        writer.flush().unwrap();
        wait(&mut session, size, |s| s.contains("DEEP_READY"));
        session.send(b"z").unwrap();
        wait(&mut session, size, |s| {
            s.contains("Enter Esc") && s.contains("node:")
        });
        // This writer exceeds the bounded reader channel. The live menu must
        // retain its revision long enough to complete the deep paid operation.
        let (release, held) = std::sync::mpsc::channel();
        let producer = std::thread::spawn(move || {
            for n in 0..64 {
                if writer
                    .write_all(format!("\n{}STREAM_{n:03}\n", "new input ".repeat(600)).as_bytes())
                    .is_err()
                {
                    return;
                }
            }
            let _ = writer.write_all(b"\nAFTER_PAUSE_TAIL\n");
            let _ = writer.flush();
            // The writer remains alive until explicitly released after q.
            let _ = held.recv_timeout(Duration::from_secs(30));
        });
        session.send(b"\x1b[C\x1b[B\x1b[B").unwrap();
        wait(&mut session, size, |s| s.contains("collapse document"));
        session.send(b"\r").unwrap();
        let collapsed = wait(&mut session, size, |s| {
            s.contains("[+ container") && s.contains("changed") && !s.contains("Esc cancel")
        });
        assert!(!collapsed.contains("DEEP_READY"));
        eprintln!("deep folded menu:\n{collapsed}");
        session.send(b"\x1b").unwrap();
        wait(&mut session, size, |s| {
            s.contains("[+ container") && !s.contains("document:")
        });
        session.send(b"\t").unwrap();
        session.send(b"\r").unwrap();
        // Opening the outer object retains collapsed descendants, so another
        // descendant placeholder remains available as a real activation target.
        wait(&mut session, size, |s| s.contains("[+ container"));
        size = pty::PTY_20X3;
        session.resize(size).unwrap();
        session.send(b"z").unwrap();
        let narrow = wait(&mut session, size, |s| s.contains("Esc") && s.contains("↵"));
        eprintln!("narrow menu:\n{narrow}");
        session.send(b"\x1b").unwrap();
        wait(&mut session, size, |s| {
            !s.contains("Esc") && !s.contains("↵")
        });
        size = pty::PTY_80X24;
        session.resize(size).unwrap();
        wait(&mut session, size, |s| {
            s.contains("[+ container") && !s.contains("Enter Esc")
        });
        session.send(b"\x1b[F").unwrap();
        wait(&mut session, size, |s| {
            s.contains("AFTER_PAUSE_TAIL") && s.contains("FOLLOW")
        });
        session.send(b"z").unwrap();
        wait(&mut session, size, |s| s.contains("Enter Esc"));
        let start = Instant::now();
        session.send(b"q").unwrap();
        assert!(session.wait(Duration::from_secs(2)).unwrap().success());
        assert!(start.elapsed() < Duration::from_secs(2));
        session.pump_output();
        assert!(session.output().windows(6).any(|p| p == b"\x1b[?25h"));
        assert!(session.output().windows(8).any(|p| p == b"\x1b[?1049l"));
        let mut synchronized = false;
        for part in session.output().windows(8) {
            if part == b"\x1b[?2026h" {
                assert!(!synchronized, "nested synchronized frame");
                synchronized = true;
            }
            if part == b"\x1b[?2026l" {
                synchronized = false;
            }
        }
        // Abort/terminal restoration may emit an extra end marker.
        assert!(!synchronized);
        let _ = release.send(());
        producer.join().unwrap();
        eprintln!(
            "fold PTY terminal bytes {}, q {:?}",
            session.output().len(),
            start.elapsed()
        );
    }
}
