mod support;

#[cfg(unix)]
#[test]
fn directory_jump_after_real_resize_then_fold_keeps_tail_without_end_recovery() {
    use portable_pty::PtySize;
    use std::time::{Duration, Instant};
    use support::{pty, temp::TempWorkspace};

    fn wait(
        session: &mut pty::PtySession,
        parser: &mut vt100::Parser,
        parsed: &mut usize,
        predicate: impl Fn(&str) -> bool,
    ) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            session.pump_output();
            parser.process(&session.output()[*parsed..]);
            *parsed = session.output().len();
            let frame = parser.screen().contents();
            if predicate(&frame) && session.output().ends_with(b"\x1b[?2026l") {
                return frame;
            }
            assert!(
                Instant::now() < deadline,
                "navigation anchor PTY timeout:\n{frame}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn body_first(frame: &str) -> &str {
        frame
            .lines()
            .next()
            .unwrap()
            .rsplit('│')
            .next()
            .unwrap()
            .trim()
    }

    let workspace = TempWorkspace::new().unwrap();
    let input = workspace.path().join("stream-final.md");
    std::fs::write(
        &input,
        "# Stream\n- a\n- b\n\n- c\n\n  d\n\n> q\n>\n> r\n\nFINAL\n\n# Tail\nREADY\n",
    )
    .unwrap();
    let mut command = pty::current_binary_command();
    command.arg(&input);
    let mut size = PtySize {
        rows: 24,
        cols: 60,
        pixel_width: 0,
        pixel_height: 0,
    };
    let mut parser = vt100::Parser::new(
        size.rows.try_into().unwrap(),
        size.cols.try_into().unwrap(),
        0,
    );
    let mut parsed = 0;
    let started = Instant::now();
    let mut session = pty::spawn(command, size).unwrap();
    wait(&mut session, &mut parser, &mut parsed, |s| {
        s.contains("READY")
    });
    session.send(b"\x1b[F").unwrap();
    wait(&mut session, &mut parser, &mut parsed, |s| {
        s.contains("FOLLOW")
    });
    size.rows = 10;
    size.cols = 40;
    session.resize(size).unwrap();
    parser.set_size(size.rows.try_into().unwrap(), size.cols.try_into().unwrap());
    wait(&mut session, &mut parser, &mut parsed, |s| {
        s.contains("READY") && s.contains("FOLLOW")
    });
    session.send(b"\x1b[H").unwrap();
    wait(&mut session, &mut parser, &mut parsed, |s| {
        body_first(s) == "Stream" && !s.contains("FOLLOW")
    });
    session.send(b"s").unwrap();
    wait(&mut session, &mut parser, &mut parsed, |s| {
        s.contains("│Stream")
    });
    session.send(b"\t\x1b[B\r").unwrap();
    let before = wait(&mut session, &mut parser, &mut parsed, |s| {
        body_first(s) == "Tail" && s.contains("READY")
    });
    session.send(b"z").unwrap();
    wait(&mut session, &mut parser, &mut parsed, |s| {
        s.contains("toggle") && s.contains("Esc")
    });
    session.send(b"\r").unwrap();
    let folded = wait(&mut session, &mut parser, &mut parsed, |s| {
        body_first(s) == "Tail" && s.contains("[+ heading 1")
    });
    session.send(b"\x1b").unwrap();
    let closed = wait(&mut session, &mut parser, &mut parsed, |s| {
        body_first(s) == "Tail" && s.contains("[+ heading 1") && !s.contains("toggle")
    });
    assert!(!closed.contains("READY"));
    session.send(b"q").unwrap();
    assert!(session.wait(Duration::from_secs(2)).unwrap().success());
    session.pump_output();
    assert!(session.output().windows(6).any(|s| s == b"\x1b[?25h"));
    assert!(session.output().windows(8).any(|s| s == b"\x1b[?1049l"));
    eprintln!(
        "037 real PTY before:\n{before}\nfolded:\n{folded}\nclosed:\n{closed}\nterminal_bytes={} elapsed={:?} exit=0",
        session.output().len(),
        started.elapsed()
    );
}
