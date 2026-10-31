use std::ffi::OsStr;
use std::io::{self, Read, Write};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

use super::env::fixed_pty_command;

pub const PTY_120X40: PtySize = PtySize {
    rows: 40,
    cols: 120,
    pixel_width: 0,
    pixel_height: 0,
};

pub const PTY_80X24: PtySize = PtySize {
    rows: 24,
    cols: 80,
    pixel_width: 0,
    pixel_height: 0,
};

const POLL_INTERVAL: Duration = Duration::from_millis(5);

pub fn command(program: impl AsRef<OsStr>) -> CommandBuilder {
    fixed_pty_command(program)
}

#[must_use]
pub fn current_binary_command() -> CommandBuilder {
    command(env!("CARGO_BIN_EXE_mdtui-tui"))
}

pub struct PtySession {
    master: Option<Box<dyn MasterPty + Send>>,
    child: Option<Box<dyn Child + Send + Sync>>,
    writer: Option<Box<dyn Write + Send>>,
    reader: Option<JoinHandle<()>>,
    receiver: Receiver<Vec<u8>>,
    output: Vec<u8>,
    eof: bool,
}

pub fn spawn(command: CommandBuilder, size: PtySize) -> io::Result<PtySession> {
    PtySession::spawn(command, size)
}

impl PtySession {
    pub fn spawn(command: CommandBuilder, size: PtySize) -> io::Result<Self> {
        let pair = native_pty_system()
            .openpty(size)
            .map_err(|error| io::Error::other(error.to_string()))?;
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| io::Error::other(error.to_string()))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|error| io::Error::other(error.to_string()))?;
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| io::Error::other(error.to_string()))?;
        drop(pair.slave);

        let (sender, receiver) = mpsc::channel();
        let reader_thread = thread::spawn(move || {
            let mut buffer = [0_u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => {
                        if sender.send(buffer[..count].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
        });

        Ok(Self {
            master: Some(pair.master),
            child: Some(child),
            writer: Some(writer),
            reader: Some(reader_thread),
            receiver,
            output: Vec::new(),
            eof: false,
        })
    }

    pub fn resize(&mut self, size: PtySize) -> io::Result<()> {
        self.master
            .as_ref()
            .ok_or_else(closed_error)?
            .resize(size)
            .map_err(|error| io::Error::other(error.to_string()))
    }

    pub fn send(&mut self, bytes: &[u8]) -> io::Result<()> {
        let writer = self.writer.as_mut().ok_or_else(closed_error)?;
        writer.write_all(bytes)?;
        writer.flush()
    }

    pub fn wait(&mut self, timeout: Duration) -> io::Result<portable_pty::ExitStatus> {
        let deadline = Instant::now() + timeout;
        loop {
            self.pump();
            let status = self.child.as_mut().ok_or_else(closed_error)?.try_wait()?;
            if let Some(status) = status {
                self.pump();
                return Ok(status);
            }
            if Instant::now() >= deadline {
                return Err(timeout_error("child"));
            }
            thread::sleep(POLL_INTERVAL);
        }
    }

    pub fn read_until(&mut self, needle: &[u8], timeout: Duration) -> io::Result<()> {
        if needle.is_empty() {
            return Ok(());
        }
        let deadline = Instant::now() + timeout;
        loop {
            self.pump();
            if self
                .output
                .windows(needle.len())
                .any(|window| window == needle)
            {
                return Ok(());
            }
            if self.eof {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "PTY reached EOF before expected output",
                ));
            }
            if Instant::now() >= deadline {
                return Err(timeout_error("output"));
            }
            thread::sleep(POLL_INTERVAL);
        }
    }

    pub fn read_until_eof(&mut self, timeout: Duration) -> io::Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            self.pump();
            if self.eof {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(timeout_error("EOF"));
            }
            thread::sleep(POLL_INTERVAL);
        }
    }

    #[must_use]
    pub fn output(&self) -> &[u8] {
        &self.output
    }

    #[must_use]
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.output).into_owned()
    }

    pub fn terminate(&mut self) -> io::Result<()> {
        if let Some(mut child) = self.child.take()
            && child.try_wait()?.is_none()
        {
            child.kill()?;
            child.wait()?;
        }
        self.writer.take();
        self.master.take();
        if let Some(reader) = self.reader.take() {
            reader
                .join()
                .map_err(|_| io::Error::other("PTY reader thread panicked"))?;
        }
        Ok(())
    }

    fn pump(&mut self) {
        loop {
            match self.receiver.try_recv() {
                Ok(chunk) => self.output.extend_from_slice(&chunk),
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    self.eof = true;
                    return;
                }
            }
        }
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        let _ = self.terminate();
    }
}

fn closed_error() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "PTY session is closed")
}

fn timeout_error(operation: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        format!("timed out waiting for PTY {operation}"),
    )
}
