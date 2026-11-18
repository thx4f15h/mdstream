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
    fn empty_fifo_menu_escape_resumes_six_byte_first_input_and_later_batches() {
        for keys in [b"".as_slice(), b"\x1b[H", b"z\x1b"] {
            let workspace = TempWorkspace::new().unwrap();
            let fifo = workspace.path().join("empty.fifo");
            assert!(
                std::process::Command::new("mkfifo")
                    .arg(&fifo)
                    .status()
                    .unwrap()
                    .success()
            );
            // Holding both ends avoids EOF before the first keyboard event.
            let mut writer = OpenOptions::new()
                .read(true)
                .write(true)
                .open(&fifo)
                .unwrap();
            let mut command = pty::current_binary_command();
            command.arg(&fifo);
            let size = pty::PTY_80X24;
            let mut session = pty::spawn(command, size).unwrap();
            session
                .read_until(b"\x1b[?1006h", Duration::from_secs(15))
                .unwrap();
            if keys == b"z\x1b" {
                session.send(b"z").unwrap();
                wait(&mut session, size, |s| s.contains("Enter Esc"));
                session.send(b"\x1b").unwrap();
                wait(&mut session, size, |s| !s.contains("Enter Esc"));
            } else {
                session.send(keys).unwrap();
            }
            writer.write_all(b"READY\n").unwrap();
            let text = wait(&mut session, size, |s| s.contains("READY"));
            assert!(!text.contains("Enter Esc"));
            writer.write_all(b"NEXT\n").unwrap();
            wait(&mut session, size, |s| {
                s.contains("READY") && s.contains("NEXT")
            });
            drop(writer);
            // q must remain independent of the Reader, including after EOF.
            session.send(b"q").unwrap();
            assert!(session.wait(Duration::from_secs(15)).unwrap().success());
        }
    }

    #[test]
    fn empty_fifo_menu_pauses_then_resumes_and_q_does_not_wait_for_first_bytes() {
        for quit_in_menu in [false, true] {
            let workspace = TempWorkspace::new().unwrap();
            let fifo = workspace.path().join("held.fifo");
            assert!(
                std::process::Command::new("mkfifo")
                    .arg(&fifo)
                    .status()
                    .unwrap()
                    .success()
            );
            let mut writer = OpenOptions::new()
                .read(true)
                .write(true)
                .open(&fifo)
                .unwrap();
            let mut command = pty::current_binary_command();
            command.arg(&fifo);
            let size = pty::PTY_80X24;
            let mut session = pty::spawn(command, size).unwrap();
            session
                .read_until(b"\x1b[?1006h", Duration::from_secs(15))
                .unwrap();
            session.send(b"z").unwrap();
            wait(&mut session, size, |s| s.contains("Enter Esc"));
            if !quit_in_menu {
                writer.write_all(b"READY\n").unwrap();
                for page in [
                    "document:",
                    "range:",
                    "level:",
                    "container:",
                    "new defaults:",
                ] {
                    session.send(b"\x1b[C").unwrap();
                    let text = wait(&mut session, size, |s| s.contains(page));
                    assert!(
                        !text.contains("READY"),
                        "reader consumed while menu open: {text}"
                    );
                }
                session.send(b"\x1b").unwrap();
                wait(&mut session, size, |s| {
                    s.contains("READY") && !s.contains("Enter Esc")
                });
            }
            session.send(b"q").unwrap();
            assert!(session.wait(Duration::from_secs(15)).unwrap().success());
            // Keep the writer alive until q has completed, with no EOF to help it.
            drop(writer);
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
