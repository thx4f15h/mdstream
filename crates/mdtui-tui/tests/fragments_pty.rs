mod support;
#[cfg(unix)]
mod unix {
    use super::support::{pty, temp::TempWorkspace};
    use std::{fs::OpenOptions, io::Write, time::Duration};
    fn held_fifo(mouse: bool) {
        let workspace = TempWorkspace::new().unwrap();
        let fifo = workspace.path().join("slug.fifo");
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap()
                .success()
        );
        let mut command = pty::current_binary_command();
        command.arg(&fifo);
        let mut session = pty::spawn(command, pty::PTY_80X24).unwrap();
        let mut input = OpenOptions::new().write(true).open(&fifo).unwrap();
        let source = format!(
            "[jump](#later)\n\n# Early\n\n{}",
            "body line\n\n".repeat(60)
        );
        input.write_all(source.as_bytes()).unwrap();
        input.flush().unwrap();
        session
            .read_until(b"jump", Duration::from_secs(10))
            .unwrap();
        if mouse {
            session.send(b"\x1b[<0;1;1M").unwrap();
        } else {
            session.send(b"\t\r").unwrap();
        }
        session
            .read_until("等待标题出现".as_bytes(), Duration::from_secs(10))
            .unwrap();
        // Keep the FIFO open throughout activation and the final assertion.
        input
            .write_all(b"\n# Later\n\nSLUG_PRE_EOF_TARGET\n")
            .unwrap();
        input.flush().unwrap();
        session
            .read_until(b"SLUG_PRE_EOF_TARGET", Duration::from_secs(10))
            .unwrap();
        session.send(b"q").unwrap();
        assert!(session.wait(Duration::from_secs(3)).unwrap().success());
        session
            .read_until(b"\x1b[?1006l", Duration::from_secs(3))
            .unwrap();
    }
    #[test]
    fn enter_request_finishes_on_a_held_fifo_before_eof() {
        held_fifo(false);
    }
    #[test]
    fn actual_sgr_mouse_click_uses_current_fragment_before_eof() {
        held_fifo(true);
    }
    fn grid_until(
        session: &mut pty::PtySession,
        parser: &mut vt100::Parser,
        at: &mut usize,
        ready: impl Fn(&str) -> bool,
    ) {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            session.pump_output();
            parser.process(&session.output()[*at..]);
            *at = session.output().len();
            let text = parser.screen().contents();
            if ready(&text) {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "PTY grid timeout: {text:?}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    #[test]
    fn actual_late_binding_current_owner_heading_label_and_resize_without_eof() {
        let workspace = TempWorkspace::new().unwrap();
        let fifo = workspace.path().join("late.fifo");
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap()
                .success()
        );
        let mut command = pty::current_binary_command();
        command.arg(&fifo);
        let mut session = pty::spawn(command, pty::PTY_80X24).unwrap();
        let mut input = OpenOptions::new().write(true).open(&fifo).unwrap();
        let source = format!(
            "[jump](#foo)\n\n# [foo][r]\n\nFIRST TARGET\n\n{}# foo\n\nSECOND TARGET\n",
            "filler paragraph\n\n".repeat(80)
        );
        input.write_all(source.as_bytes()).unwrap();
        input.flush().unwrap();
        let mut parser = vt100::Parser::new(
            std::num::NonZeroU16::new(24).unwrap(),
            std::num::NonZeroU16::new(80).unwrap(),
            0,
        );
        let mut at = 0;
        grid_until(&mut session, &mut parser, &mut at, |s| s.contains("jump"));
        session.send(b"\t\r").unwrap();
        grid_until(&mut session, &mut parser, &mut at, |s| {
            s.contains("SECOND TARGET") && !s.contains("FIRST TARGET")
        });
        input.write_all(b"\n[r]: /yes\n").unwrap();
        input.flush().unwrap();
        session.send(b"\x1b[H").unwrap();
        grid_until(&mut session, &mut parser, &mut at, |s| {
            s.contains("jump") && s.lines().any(|l| l.trim() == "foo")
        });
        session.send(b"\x1b[<0;1;1M").unwrap();
        grid_until(&mut session, &mut parser, &mut at, |s| {
            s.contains("FIRST TARGET")
                && !s.contains("jump")
                && s.lines().any(|l| l.trim() == "foo")
        });
        parser.set_size(
            std::num::NonZeroU16::new(24).unwrap(),
            std::num::NonZeroU16::new(100).unwrap(),
        );
        session
            .resize(portable_pty::PtySize {
                rows: 24,
                cols: 100,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        grid_until(&mut session, &mut parser, &mut at, |s| {
            s.contains("FIRST TARGET") && s.lines().any(|l| l.trim() == "foo")
        });
        session.send(b"q").unwrap();
        assert!(session.wait(Duration::from_secs(3)).unwrap().success());
    }
}
