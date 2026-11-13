mod support;
#[cfg(unix)]
mod unix {
    use super::support::{pty, temp::TempWorkspace};
    use std::{fs::OpenOptions, io::Write, time::Duration};
    fn command(file: &std::path::Path) -> portable_pty::CommandBuilder {
        fn executable(name: &str) -> std::path::PathBuf {
            std::env::split_paths(&std::env::var_os("PATH").unwrap())
                .map(|path| path.join(name))
                .find(|path| path.is_file())
                .unwrap()
        }
        let mut command = pty::command(executable("bash"));
        command.args(["-c",r#"before=$("$3" -g); "$1" "$2"; result=$?; after=$("$3" -g); if [ "$before" = "$after" ]; then printf '\r\nGIANT_TERMIOS_OK\r\n'; else printf '\r\nGIANT_TERMIOS_BAD\r\n'; fi; exit "$result""#,"giant-pty",env!("CARGO_BIN_EXE_mdtui-tui")]);
        command.arg(file);
        command.arg(executable("stty"));
        command
    }
    #[test]
    fn actual_pty_receives_complete_utf8_cluster_and_q_restores_termios() {
        let workspace = TempWorkspace::new().unwrap();
        let cluster = format!("👩{}\u{200d}💻", "\u{301}".repeat(10_000));
        let file = workspace
            .write("giant.md", format!("{cluster}\n\nAFTER_GIANT\n"))
            .unwrap();
        let mut session = pty::spawn(command(&file), pty::PTY_80X24).unwrap();
        session
            .read_until(cluster.as_bytes(), Duration::from_secs(10))
            .unwrap();
        session
            .read_until(b"AFTER_GIANT", Duration::from_secs(10))
            .unwrap();
        assert_eq!(
            session
                .output()
                .windows(cluster.len())
                .filter(|w| *w == cluster.as_bytes())
                .count(),
            1
        );
        std::str::from_utf8(session.output()).unwrap();
        session.send(b"q").unwrap();
        assert!(session.wait(Duration::from_secs(2)).unwrap().success());
        session
            .read_until(b"GIANT_TERMIOS_OK", Duration::from_secs(2))
            .unwrap();
    }
    #[test]
    fn held_fifo_appends_form_one_wire_egc_and_query_status_waits_for_its_boundary() {
        let workspace = TempWorkspace::new().unwrap();
        let fifo = workspace.path().join("giant.fifo");
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap()
                .success()
        );
        let mut session = pty::spawn(command(&fifo), pty::PTY_80X24).unwrap();
        let mut source = OpenOptions::new().write(true).open(&fifo).unwrap();
        let mut cluster = String::from("a");
        source.write_all(cluster.as_bytes()).unwrap();
        source.flush().unwrap();
        session
            .read_until(cluster.as_bytes(), Duration::from_secs(10))
            .unwrap();
        for _ in 0..8 {
            let marks = "\u{301}".repeat(128);
            cluster.push_str(&marks);
            source.write_all(marks.as_bytes()).unwrap();
            source.flush().unwrap();
            session
                .read_until(cluster.as_bytes(), Duration::from_secs(10))
                .unwrap();
        }
        session.send(b"/zzzzzz").unwrap();
        std::thread::sleep(Duration::from_millis(120));
        session.pump_output();
        assert!(
            session.output().ends_with(cluster.as_bytes()),
            "status/control interrupted the open EGC"
        );
        assert_eq!(
            session.output().iter().filter(|&&b| b == 0xcc).count(),
            1024
        );
        session.send(b"\x03").unwrap();
        assert!(session.wait(Duration::from_secs(2)).unwrap().success());
        session
            .read_until(b"GIANT_TERMIOS_OK", Duration::from_secs(2))
            .unwrap();
        std::str::from_utf8(session.output()).unwrap();
        drop(source);
    }
    #[test]
    fn actual_resize_cancels_a_held_giant_transaction_once_and_repaints_complete_utf8() {
        let workspace = TempWorkspace::new().unwrap();
        let fifo = workspace.path().join("resize.fifo");
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap()
                .success()
        );
        let mut session = pty::spawn(command(&fifo), pty::PTY_80X24).unwrap();
        let mut source = OpenOptions::new().write(true).open(&fifo).unwrap();
        let cluster = format!("👩{}\u{200d}💻", "\u{301}".repeat(5000));
        source.write_all(cluster.as_bytes()).unwrap();
        source.flush().unwrap();
        session
            .read_until(cluster.as_bytes(), Duration::from_secs(10))
            .unwrap();
        let cut = session.output().len();
        session.resize(pty::PTY_120X40).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            session.pump_output();
            if session.output()[cut..]
                .windows(cluster.len())
                .any(|w| w == cluster.as_bytes())
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "no complete repaint after resize"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            session
                .output()
                .windows(cluster.len())
                .filter(|w| *w == cluster.as_bytes())
                .count(),
            2
        );
        assert_eq!(
            session.output()[..]
                .windows(3)
                .filter(|w| *w == b"\x18\x1b\\")
                .count(),
            1
        );
        session.send(b"q").unwrap();
        assert!(session.wait(Duration::from_secs(2)).unwrap().success());
        session
            .read_until(b"GIANT_TERMIOS_OK", Duration::from_secs(2))
            .unwrap();
        std::str::from_utf8(session.output()).unwrap();
        drop(source);
    }
    #[test]
    fn simultaneous_resize_and_quit_never_lose_the_tty_key_while_fifo_is_held() {
        for key in [b"q".as_slice(), b"\x03".as_slice()] {
            for _ in 0..4 {
                let workspace = TempWorkspace::new().unwrap();
                let fifo = workspace.path().join("simultaneous.fifo");
                assert!(
                    std::process::Command::new("mkfifo")
                        .arg(&fifo)
                        .status()
                        .unwrap()
                        .success()
                );
                let mut session = pty::spawn(command(&fifo), pty::PTY_80X24).unwrap();
                let mut source = OpenOptions::new().write(true).open(&fifo).unwrap();
                let cluster = format!("a{}", "\u{301}".repeat(1000));
                source.write_all(cluster.as_bytes()).unwrap();
                source.flush().unwrap();
                session
                    .read_until(cluster.as_bytes(), Duration::from_secs(10))
                    .unwrap();
                session.resize(pty::PTY_120X40).unwrap();
                session.send(key).unwrap();
                assert!(session.wait(Duration::from_secs(2)).unwrap().success());
                session
                    .read_until(b"GIANT_TERMIOS_OK", Duration::from_secs(2))
                    .unwrap();
                std::str::from_utf8(session.output()).unwrap();
                drop(source);
            }
        }
    }
}
