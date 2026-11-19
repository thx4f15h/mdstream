//! One bounded tty input owner. Graphics replies never become application keys.
//! The maintained termion parser handles complete ordinary/SS3 events; CSI
//! framing/modifiers/mouse are checked here before calling any parser.
use super::worker::CellPixels;
use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use std::{
    collections::VecDeque,
    fs::{File, OpenOptions},
    io::{self, Read},
    time::{Duration, Instant},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GraphicsReply {
    pub image: u32,
    pub placement: Option<u32>,
    pub ok: bool,
    pub missing: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Graphics(GraphicsReply),
    Cell(CellPixels),
    Device,
    Ignored,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputWork {
    pub read_bytes: usize,
    pub parsed_bytes: usize,
    pub keys: usize,
    pub protocol_bytes: usize,
    pub dropped: usize,
    pub polls: usize,
}
pub struct Parser {
    pending: Vec<u8>,
    started: Option<Instant>,
    dropping: bool,
    escaped: bool,
    paste: bool,
    paste_end: usize,
    pub work: InputWork,
}
impl Default for Parser {
    fn default() -> Self {
        Self {
            pending: Vec::with_capacity(512),
            started: None,
            dropping: false,
            escaped: false,
            paste: false,
            paste_end: 0,
            work: InputWork::default(),
        }
    }
}
fn key(code: KeyCode, modifiers: KeyModifiers) -> Input {
    Input::Key(KeyEvent::new(code, modifiers))
}
fn modifiers(value: u32) -> KeyModifiers {
    let n = value.saturating_sub(1);
    let mut m = KeyModifiers::NONE;
    if n & 1 != 0 {
        m |= KeyModifiers::SHIFT
    }
    if n & 2 != 0 {
        m |= KeyModifiers::ALT
    }
    if n & 4 != 0 {
        m |= KeyModifiers::CONTROL
    }
    if n & 8 != 0 {
        m |= KeyModifiers::SUPER
    }
    if n & 16 != 0 {
        m |= KeyModifiers::HYPER
    }
    if n & 32 != 0 {
        m |= KeyModifiers::META
    }
    m
}
impl Parser {
    pub fn pending_capacity(&self) -> usize {
        self.pending.capacity()
    }
    fn finish(&mut self) -> Input {
        let event = parse(&self.pending);
        self.work.protocol_bytes +=
            if matches!(event, Input::Graphics(_) | Input::Device | Input::Cell(_)) {
                self.pending.len()
            } else {
                0
            };
        if matches!(event, Input::Key(_)) {
            self.work.keys += 1
        }
        if event == Input::Ignored {
            self.work.dropped += 1
        }
        self.pending.clear();
        self.started = None;
        event
    }
    pub fn feed(&mut self, byte: u8, now: Instant) -> Option<Input> {
        self.work.parsed_bytes += 1;
        if self.paste {
            const END: &[u8] = b"\x1b[201~";
            if byte == END[self.paste_end] {
                self.paste_end += 1;
                if self.paste_end == END.len() {
                    self.paste = false;
                    self.paste_end = 0
                }
            } else {
                self.paste_end = usize::from(byte == 0x1b)
            }
            return None;
        }
        if self.dropping {
            if self.escaped && byte == b'\\' {
                self.dropping = false;
                self.escaped = false;
                self.started = None;
                return Some(Input::Ignored);
            }
            self.escaped = byte == 0x1b;
            return None;
        }
        if self.pending.is_empty() {
            self.started = Some(now)
        }
        if self.pending == b"\x1b" && byte == 0x1b {
            self.started = Some(now);
            return Some(key(KeyCode::Esc, KeyModifiers::NONE));
        }
        if self.pending.len() == 512 {
            self.pending.clear();
            self.dropping = true;
            self.escaped = byte == 0x1b;
            self.work.dropped += 1;
            return None;
        }
        self.pending.push(byte);
        if matches!(
            self.pending.as_slice(),
            b"\x1b" | b"\x1b_" | b"\x1b[" | b"\x1b[[" | b"\x1bO"
        ) {
            return None;
        }
        if self.pending.starts_with(b"\x1b_") {
            return self.pending.ends_with(b"\x1b\\").then(|| self.finish());
        }
        if self.pending.starts_with(b"\x1b[") {
            if self.pending.starts_with(b"\x1b[M") && self.pending.len() < 6 {
                return None;
            }
            if self.pending.len() > 3 && self.pending[2] == b'M' {
                return Some(self.finish());
            }
            if !matches!(byte, 0x40..=0x7e) {
                return None;
            }
            if self.pending == b"\x1b[200~" {
                self.pending.clear();
                self.paste = true;
                self.paste_end = 0;
                self.started = None;
                return Some(Input::Ignored);
            }
            return Some(self.finish());
        }
        if self.pending.starts_with(b"\x1bO") {
            return (self.pending.len() == 3).then(|| self.finish());
        }
        let start = usize::from(self.pending[0] == 0x1b);
        if self.pending.get(start).is_some_and(|b| *b >= 0x80) {
            match std::str::from_utf8(&self.pending[start..]) {
                Err(e) if e.error_len().is_none() => return None,
                Err(_) => {
                    self.pending.clear();
                    self.started = None;
                    self.work.dropped += 1;
                    return Some(Input::Ignored);
                }
                Ok(_) => {}
            }
        }
        Some(self.finish())
    }
    pub fn deadline(&self) -> Option<Instant> {
        self.started.map(|t| {
            t + if self.pending == b"\x1b" {
                Duration::from_millis(20)
            } else {
                Duration::from_millis(200)
            }
        })
    }
    pub fn expire(&mut self, now: Instant) -> Option<Input> {
        if self.deadline().is_some_and(|t| now >= t) {
            let esc = self.pending == b"\x1b";
            self.pending.clear();
            self.started = None;
            self.dropping = false;
            self.escaped = false;
            return Some(if esc {
                key(KeyCode::Esc, KeyModifiers::NONE)
            } else {
                Input::Ignored
            });
        }
        None
    }
}
fn parse(b: &[u8]) -> Input {
    if b.starts_with(b"\x1b_G") && b.ends_with(b"\x1b\\") {
        let Ok(s) = std::str::from_utf8(&b[3..b.len() - 2]) else {
            return Input::Ignored;
        };
        let Some((params, status)) = s.split_once(';') else {
            return Input::Ignored;
        };
        let mut id = None;
        let mut placement = None;
        for part in params.split(',') {
            if let Some(v) = part.strip_prefix("i=") {
                id = v.parse().ok()
            }
            if let Some(v) = part.strip_prefix("p=") {
                placement = v.parse().ok()
            }
        }
        return id.map_or(Input::Ignored, |image| {
            Input::Graphics(GraphicsReply {
                image,
                placement,
                ok: status == "OK",
                missing: status.starts_with("ENOENT"),
            })
        });
    }
    if b.starts_with(b"\x1b[") {
        if b.len() == 6 && b[2] == b'M' {
            return mouse(
                u32::from(b[3].saturating_sub(32)),
                u32::from(b[4].saturating_sub(32)),
                u32::from(b[5].saturating_sub(32)),
                false,
            );
        }
        if b.len() == 4 && b[2] == b'[' && matches!(b[3], b'A'..=b'E') {
            return key(KeyCode::F(b[3] - b'A' + 1), KeyModifiers::NONE);
        }
        let Some((&last, body)) = b[2..].split_last() else {
            return Input::Ignored;
        };
        let Ok(body) = std::str::from_utf8(body) else {
            return Input::Ignored;
        };
        if last == b'c' && body.starts_with('?') {
            return Input::Device;
        }
        if last == b'u' {
            return unicode_key(body);
        }
        let nums: Option<Vec<u32>> = body
            .trim_start_matches('<')
            .split(';')
            .filter(|s| !s.is_empty())
            .map(|s| s.parse().ok())
            .collect();
        let Some(n) = nums else { return Input::Ignored };
        if last == b't' {
            return if n.len() == 3 && n[0] == 6 && n[1] > 0 && n[2] > 0 {
                Input::Cell(CellPixels {
                    width: n[2],
                    height: n[1],
                })
            } else {
                Input::Ignored
            };
        }
        if matches!(last, b'm' | b'M') && n.len() == 3 {
            return mouse(
                if body.starts_with('<') {
                    n[0]
                } else {
                    n[0].saturating_sub(32)
                },
                n[1],
                n[2],
                last == b'm',
            );
        }
        let m = modifiers(n.get(1).copied().unwrap_or(1));
        let code = match last {
            b'P' => KeyCode::F(1),
            b'Q' => KeyCode::F(2),
            b'R' => KeyCode::F(3),
            b'S' => KeyCode::F(4),
            b'A' => KeyCode::Up,
            b'B' => KeyCode::Down,
            b'C' => KeyCode::Right,
            b'D' => KeyCode::Left,
            b'H' => KeyCode::Home,
            b'F' => KeyCode::End,
            b'Z' => return key(KeyCode::BackTab, m | KeyModifiers::SHIFT),
            b'~' => match n.first().copied() {
                Some(1 | 7) => KeyCode::Home,
                Some(2) => KeyCode::Insert,
                Some(3) => KeyCode::Delete,
                Some(4 | 8) => KeyCode::End,
                Some(5) => KeyCode::PageUp,
                Some(6) => KeyCode::PageDown,
                Some(v @ 11..=15) => KeyCode::F((v - 10) as u8),
                Some(v @ 17..=21) => KeyCode::F((v - 11) as u8),
                Some(v @ 23..=24) => KeyCode::F((v - 12) as u8),
                Some(27) if n.len() == 3 => {
                    return char::from_u32(n[2])
                        .map_or(Input::Ignored, |c| key(KeyCode::Char(c), m));
                }
                _ => return Input::Ignored,
            },
            _ => return Input::Ignored,
        };
        return key(code, m);
    }
    let alt = b.first() == Some(&0x1b) && b.len() > 1 && !b.starts_with(b"\x1bO");
    let bytes = if alt { &b[1..] } else { b };
    let modifier = if alt {
        KeyModifiers::ALT
    } else {
        KeyModifiers::NONE
    };
    if bytes.len() == 1 && matches!(bytes[0], 1..=26) && !matches!(bytes[0], 9 | 13) {
        return key(
            KeyCode::Char(char::from(bytes[0] - 1 + b'a')),
            modifier | KeyModifiers::CONTROL,
        );
    }
    if bytes.len() == 1 && matches!(bytes[0], 28..=31) {
        return key(
            KeyCode::Char(char::from(bytes[0] - 28 + b'4')),
            modifier | KeyModifiers::CONTROL,
        );
    }
    if bytes == b"\t" {
        return key(KeyCode::Tab, modifier);
    }
    if bytes == b"\r" {
        return key(KeyCode::Enter, modifier);
    }
    let mut rest = bytes[1..].iter().copied().map(Ok);
    match termion::event::parse_event(bytes[0], &mut rest) {
        Ok(termion::event::Event::Key(k)) => {
            use termion::event::Key as K;
            let (code, extra) = match k {
                K::Backspace => (KeyCode::Backspace, KeyModifiers::NONE),
                K::Left => (KeyCode::Left, KeyModifiers::NONE),
                K::Right => (KeyCode::Right, KeyModifiers::NONE),
                K::Up => (KeyCode::Up, KeyModifiers::NONE),
                K::Down => (KeyCode::Down, KeyModifiers::NONE),
                K::Home => (KeyCode::Home, KeyModifiers::NONE),
                K::End => (KeyCode::End, KeyModifiers::NONE),
                K::F(v) => (KeyCode::F(v), KeyModifiers::NONE),
                K::Char(c) => (KeyCode::Char(c), KeyModifiers::NONE),
                K::Ctrl(c) => (KeyCode::Char(c), KeyModifiers::CONTROL),
                K::Alt(c) => (KeyCode::Char(c), KeyModifiers::ALT),
                K::Esc => (KeyCode::Esc, KeyModifiers::NONE),
                K::Null => (KeyCode::Char(' '), KeyModifiers::CONTROL),
                _ => return Input::Ignored,
            };
            let shift = if matches!(code,KeyCode::Char(c) if c.is_uppercase()) {
                KeyModifiers::SHIFT
            } else {
                KeyModifiers::NONE
            };
            key(code, modifier | extra | shift)
        }
        _ => Input::Ignored,
    }
}
// Compatibility with the existing crossterm CSI-u character/modifier events.
// We never enable keyboard enhancement modes; unsolicited non-character
// extension codes are consumed instead of leaking protocol bytes as keys.
fn unicode_key(body: &str) -> Input {
    let mut fields = body.split(';');
    let mut codes = fields.next().unwrap_or("").split(':');
    let Some(c) = codes
        .next()
        .and_then(|s| s.parse::<u32>().ok())
        .and_then(char::from_u32)
    else {
        return Input::Ignored;
    };
    let mut attrs = fields.next().unwrap_or("1").split(':');
    let Some(mask) = attrs.next().and_then(|s| s.parse::<u32>().ok()) else {
        return Input::Ignored;
    };
    let mut m = modifiers(mask);
    let kind = match attrs.next().unwrap_or("1") {
        "1" => KeyEventKind::Press,
        "2" => KeyEventKind::Repeat,
        "3" => KeyEventKind::Release,
        _ => return Input::Ignored,
    };
    let mut code = match c {
        '\x1b' => KeyCode::Esc,
        '\r' => KeyCode::Enter,
        '\t' => {
            if m.contains(KeyModifiers::SHIFT) {
                KeyCode::BackTab
            } else {
                KeyCode::Tab
            }
        }
        '\x7f' => KeyCode::Backspace,
        '\n' => {
            m |= KeyModifiers::CONTROL;
            KeyCode::Char('j')
        }
        '\u{e000}'..='\u{f8ff}' => return Input::Ignored,
        _ => KeyCode::Char(c),
    };
    if m.contains(KeyModifiers::SHIFT)
        && let Some(c) = codes
            .next()
            .and_then(|s| s.parse::<u32>().ok())
            .and_then(char::from_u32)
    {
        code = KeyCode::Char(c);
        m.remove(KeyModifiers::SHIFT)
    }
    Input::Key(KeyEvent::new_with_kind(code, m, kind))
}
fn mouse(code: u32, x: u32, y: u32, released: bool) -> Input {
    let mut modifiers = KeyModifiers::NONE;
    if code & 4 != 0 {
        modifiers |= KeyModifiers::SHIFT
    }
    if code & 8 != 0 {
        modifiers |= KeyModifiers::ALT
    }
    if code & 16 != 0 {
        modifiers |= KeyModifiers::CONTROL
    }
    let button = match code & 3 {
        0 => MouseButton::Left,
        1 => MouseButton::Middle,
        2 => MouseButton::Right,
        _ => MouseButton::Left,
    };
    let kind = if code & 64 != 0 {
        match code & 3 {
            0 => MouseEventKind::ScrollUp,
            1 => MouseEventKind::ScrollDown,
            2 => MouseEventKind::ScrollLeft,
            _ => MouseEventKind::ScrollRight,
        }
    } else if released {
        MouseEventKind::Up(button)
    } else if code & 3 == 3 {
        if code & 32 != 0 {
            MouseEventKind::Moved
        } else {
            MouseEventKind::Up(button)
        }
    } else if code & 32 != 0 {
        MouseEventKind::Drag(button)
    } else {
        MouseEventKind::Down(button)
    };
    Input::Mouse(MouseEvent {
        kind,
        column: x.saturating_sub(1).min(u32::from(u16::MAX)) as u16,
        row: y.saturating_sub(1).min(u32::from(u16::MAX)) as u16,
        modifiers,
    })
}
pub struct Source {
    file: File,
    parser: Parser,
    queue: VecDeque<Input>,
    size: (u16, u16),
    pub work: InputWork,
}
impl Source {
    pub fn open() -> io::Result<Self> {
        let file = OpenOptions::new().read(true).open("/dev/tty")?;
        let flags = rustix::fs::fcntl_getfl(&file)?;
        rustix::fs::fcntl_setfl(&file, flags | rustix::fs::OFlags::NONBLOCK)?;
        Ok(Self {
            file,
            parser: Parser::default(),
            queue: VecDeque::with_capacity(64),
            size: crossterm::terminal::size()?,
            work: InputWork::default(),
        })
    }
    pub fn resize(&mut self) -> io::Result<Option<(u16, u16)>> {
        let next = crossterm::terminal::size()?;
        if next != self.size {
            self.size = next;
            Ok(Some(next))
        } else {
            Ok(None)
        }
    }
    pub fn poll(&mut self, mut timeout: Duration) -> io::Result<Option<Input>> {
        if let Some(e) = self.queue.pop_front() {
            return Ok(Some(e));
        }
        let now = Instant::now();
        if let Some(e) = self.parser.expire(now) {
            return Ok(Some(e));
        }
        if let Some(deadline) = self.parser.deadline() {
            timeout = timeout.min(deadline.saturating_duration_since(now))
        }
        timeout = timeout.max(Duration::from_millis(1));
        self.work.polls += 1;
        let mut fds = [rustix::event::PollFd::new(
            &self.file,
            rustix::event::PollFlags::IN,
        )];
        let t = rustix::event::Timespec {
            tv_sec: timeout.as_secs() as i64,
            tv_nsec: i64::from(timeout.subsec_nanos()),
        };
        match rustix::event::poll(&mut fds, Some(&t)) {
            Err(rustix::io::Errno::INTR) => return Ok(None),
            Err(e) => return Err(e.into()),
            Ok(_) => {}
        }
        let mut bytes = [0; 64];
        match self.file.read(&mut bytes) {
            Ok(n) => {
                self.work.read_bytes += n;
                for byte in &bytes[..n] {
                    if let Some(e) = self.parser.feed(*byte, Instant::now()) {
                        self.queue.push_back(e)
                    }
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) => {}
            Err(e) => return Err(e),
        }
        self.work.parsed_bytes = self.parser.work.parsed_bytes;
        self.work.keys = self.parser.work.keys;
        self.work.protocol_bytes = self.parser.work.protocol_bytes;
        self.work.dropped = self.parser.work.dropped;
        Ok(self
            .queue
            .pop_front()
            .or_else(|| self.parser.expire(Instant::now())))
    }
}
