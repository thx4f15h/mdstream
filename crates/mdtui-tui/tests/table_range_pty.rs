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

    fn wait(s: &mut pty::PtySession, size: PtySize, predicate: impl Fn(&str) -> bool) -> String {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            s.pump_output();
            let mut parser = vt100::Parser::new(
                size.rows.try_into().unwrap(),
                size.cols.try_into().unwrap(),
                0,
            );
            parser.process(s.output());
            let text = parser.screen().contents();
            if predicate(&text) && s.output().ends_with(b"\x1b[?2026l") {
                return text;
            }
            assert!(
                Instant::now() < deadline,
                "table PTY did not settle: {text}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn menu(s: &mut pty::PtySession, size: PtySize) {
        s.send(b"z").unwrap();
        wait(s, size, |t| t.contains("toggle"));
        s.send(b"t").unwrap();
        wait(s, size, |t| t.contains("first 0") || t.contains("first0"));
    }

    #[test]
    fn real_static_table_prefix_numeric_id_minimum_resize_and_quit() {
        let temp = TempWorkspace::new().unwrap();
        let file=temp.write("table.md","| Head | Value |\n| :--- | ---: |\n| FIRST | one |\n| SECRET | two |\n| LAST | three |\n\nAFTER\n").unwrap();
        let mut cmd = pty::current_binary_command();
        cmd.arg(&file);
        let mut size = PtySize {
            rows: 12,
            cols: 40,
            pixel_width: 0,
            pixel_height: 0,
        };
        let mut s = pty::spawn(cmd, size).unwrap();
        wait(&mut s, size, |t| {
            t.contains("FIRST") && t.contains("SECRET") && t.contains("AFTER")
        });
        menu(&mut s, size);
        s.send(b"\x1b[B\r").unwrap();
        let text = wait(&mut s, size, |t| t.contains("R0C") && t.contains("[+ rows"));
        assert!(text.contains("Head") && text.contains("FIRST"));
        assert!(!text.contains("SECRET") && !text.contains("LAST"));
        assert_eq!(text.matches('┌').count(), 1);
        assert_eq!(text.matches('└').count(), 1);
        size = pty::PTY_20X3;
        s.resize(size).unwrap();
        let text = wait(&mut s, size, |t| {
            t.contains("Head")
                && t.lines()
                    .last()
                    .is_some_and(|l| l.contains("R0C") && l.contains("Esc"))
        });
        assert!(text.contains("Head"));
        size = PtySize {
            rows: 12,
            cols: 40,
            pixel_width: 0,
            pixel_height: 0,
        };
        s.resize(size).unwrap();
        wait(&mut s, size, |t| {
            t.lines().last().is_some_and(|l| l.contains("R0C"))
        });
        s.send(b"\x1b[H\x1b[B\x1b[B\x1b[B\r").unwrap();
        wait(&mut s, size, |t| t.contains("Table N="));
        s.send(b"2\r").unwrap();
        let text = wait(&mut s, size, |t| {
            t.contains("R0C T1 3..*") && t.contains("SECRET")
        });
        assert!(!text.contains("LAST"));
        s.send(b"q").unwrap();
        assert!(s.wait(Duration::from_secs(15)).unwrap().success());
    }

    #[test]
    fn real_fifo_dormant_live_append_same_id_seal_future_row_and_q_without_eof() {
        let temp = TempWorkspace::new().unwrap();
        let fifo = temp.path().join("table.fifo");
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
        let mut cmd = pty::current_binary_command();
        cmd.arg(&fifo);
        let size = PtySize {
            rows: 12,
            cols: 40,
            pixel_width: 0,
            pixel_height: 0,
        };
        let mut s = pty::spawn(cmd, size).unwrap();
        writer.write_all(b"| HEAD |\n| - |\n").unwrap();
        writer.flush().unwrap();
        wait(&mut s, size, |t| t.contains("HEAD"));
        menu(&mut s, size);
        s.send(b"\x1b[B\r").unwrap();
        wait(&mut s, size, |t| t.contains("R0D T1 2..*"));
        writer.write_all(b"| FIRST |\n| SECRET |\n").unwrap();
        writer.flush().unwrap();
        s.send(b"\x1b").unwrap();
        let text = wait(&mut s, size, |t| {
            t.contains("FIRST") && t.contains("[+ rows") && !t.contains("R0D")
        });
        assert!(!text.contains("SECRET"));
        assert_eq!(text.matches("[+ rows").count(), 1);
        writer.write_all(b"| NEW_HIDDEN |\n").unwrap();
        writer.flush().unwrap();
        let received = "| HEAD |\n| - |\n| FIRST |\n| SECRET |\n| NEW_HIDDEN |\n".len();
        wait(&mut s, size, |t| {
            t.contains(&format!("..{received}]")) && !t.contains("NEW_HIDDEN")
        });
        // Selecting the actual registered ID after append proves its live mode
        // and identity without relying on a printed source offset or EOF count.
        menu(&mut s, size);
        s.send(b"\x1b[F\r").unwrap();
        let text = wait(&mut s, size, |t| t.contains("R0L T1 2..*"));
        assert!(!text.contains("NEW_HIDDEN"));
        s.send(b"\x1b[H").unwrap();
        for _ in 0..11 {
            s.send(b"\x1b[B").unwrap();
        }
        s.send(b"\r").unwrap();
        wait(&mut s, size, |t| t.contains("R0C"));
        s.send(b"\x1b").unwrap();
        wait(&mut s, size, |t| t.contains("FIRST") && !t.contains("R0C"));
        writer.write_all(b"| FUTURE |\n").unwrap();
        writer.flush().unwrap();
        let text = wait(&mut s, size, |t| t.contains("FUTURE"));
        assert!(!text.contains("SECRET") && !text.contains("NEW_HIDDEN"));
        assert_eq!(text.matches("[+ rows").count(), 1);
        s.send(b"q").unwrap();
        assert!(s.wait(Duration::from_secs(15)).unwrap().success());
        drop(writer);
    }
}
