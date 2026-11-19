//! Direct RGBA Kitty packets. A single owner retains the wire from the first
//! byte through ST, including short writes. Preparation and reads yield too.
use super::{
    MAX_CANVAS,
    bytes::Bytes,
    worker::{CellPixels, Geometry},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use mdtui3::{Budget, WorkLimit};
use std::{
    io::{self, Write},
    sync::Arc,
};
const RAW_CHUNK: usize = 3072;
const PACKET_CAPACITY: usize = 4352;
pub const QUERY_ID: u32 = 31;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub image: u32,
    pub placement: u32,
    pub row: u16,
    pub column: u16,
    pub cells: Geometry,
    pub crop_x: u32,
    pub crop_y: u32,
    pub crop: Geometry,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Query,
    Place(Placement),
    Delete {
        image: u32,
        placement: Option<u32>,
        free: bool,
    },
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WireWork {
    pub raw_reads: usize,
    pub base64_bytes: usize,
    pub prepared_bytes: usize,
    pub wire_bytes: usize,
    pub packets: usize,
    pub would_block: usize,
}
#[derive(Clone, Copy)]
enum Piece {
    Literal(&'static [u8]),
    Number(u32),
}
struct Header {
    pieces: [Piece; 32],
    len: usize,
    at: usize,
    offset: usize,
    divisor: u32,
}
impl Default for Header {
    fn default() -> Self {
        Self {
            pieces: [Piece::Literal(b""); 32],
            len: 0,
            at: 0,
            offset: 0,
            divisor: 0,
        }
    }
}
impl Header {
    fn text(&mut self, s: &'static [u8]) {
        self.pieces[self.len] = Piece::Literal(s);
        self.len += 1
    }
    fn number(&mut self, n: u32) {
        self.pieces[self.len] = Piece::Number(n);
        self.len += 1
    }
    fn field(&mut self, s: &'static [u8], n: u32) {
        self.text(s);
        self.number(n)
    }
    // No formatted header buffer or whole-image base64 allocation. Every
    // generated digit/literal byte is admitted before being inspected/staged.
    fn byte(&mut self, b: &mut Budget) -> Result<Option<u8>, ()> {
        while self.at < self.len {
            if let Piece::Literal(s) = self.pieces[self.at]
                && self.offset == s.len()
            {
                self.at += 1;
                self.offset = 0;
                continue;
            }
            if !b.charge(WorkLimit {
                bytes: 1,
                queries: 1,
                objects: 1,
                ..Default::default()
            }) {
                return Err(());
            }
            return Ok(Some(match self.pieces[self.at] {
                Piece::Literal(s) => {
                    let v = s[self.offset];
                    self.offset += 1;
                    v
                }
                Piece::Number(n) => {
                    if self.divisor == 0 {
                        self.divisor = 1;
                        while n / self.divisor >= 10 {
                            self.divisor *= 10
                        }
                    }
                    let v = b'0' + ((n / self.divisor) % 10) as u8;
                    self.divisor /= 10;
                    if self.divisor == 0 {
                        self.at += 1;
                        self.offset = 0
                    }
                    v
                }
            }));
        }
        Ok(None)
    }
}
fn command(c: Command) -> Header {
    let mut h = Header::default();
    match c {
        Command::Query => h.text(b"\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\\x1b[c\x1b[16t"),
        Command::Place(p) => {
            h.field(b"\x1b[", u32::from(p.row) + 1);
            h.field(b";", u32::from(p.column) + 1);
            h.text(b"H");
            h.field(b"\x1b_Ga=p,i=", p.image);
            h.field(b",p=", p.placement);
            h.field(b",c=", p.cells.columns);
            h.field(b",r=", p.cells.rows);
            h.field(b",x=", p.crop_x);
            h.field(b",y=", p.crop_y);
            h.field(b",w=", p.crop.columns);
            h.field(b",h=", p.crop.rows);
            h.text(b",C=1;\x1b\\");
        }
        Command::Delete {
            image,
            placement,
            free,
        } => {
            h.text(if free {
                b"\x1b_Ga=d,d=I,i="
            } else {
                b"\x1b_Ga=d,d=i,i="
            });
            h.number(image);
            if let Some(p) = placement {
                h.field(b",p=", p)
            }
            h.text(b";\x1b\\")
        }
    }
    h
}
struct Upload {
    rgba: Arc<Bytes>,
    width: u32,
    height: u32,
    image: u32,
    raw: usize,
    end: usize,
    first: bool,
}
enum Request {
    Control(Command),
    Upload(Upload),
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Start,
    Header,
    Raw,
    End,
    Write,
    Done,
}
pub struct Wire {
    request: Option<Request>,
    header: Header,
    packet: Vec<u8>,
    at: usize,
    phase: Phase,
    group: [u8; 3],
    group_len: usize,
    cancelled: bool,
    pub work: WireWork,
}
pub struct Finished {
    pub pin: Option<Arc<Bytes>>,
    pub cancelled: bool,
}
impl Default for Wire {
    fn default() -> Self {
        Self {
            request: None,
            header: Header::default(),
            packet: Vec::with_capacity(PACKET_CAPACITY),
            at: 0,
            phase: Phase::Start,
            group: [0; 3],
            group_len: 0,
            cancelled: false,
            work: WireWork::default(),
        }
    }
}
impl Wire {
    pub fn busy(&self) -> bool {
        self.request.is_some()
    }
    pub fn packet_started(&self) -> bool {
        self.at > 0 && self.phase == Phase::Write
    }
    pub fn finished(&self) -> bool {
        self.busy() && self.phase == Phase::Done
    }
    pub fn capacity_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.packet.capacity()
    }
    pub fn control(&mut self, c: Command) -> bool {
        if self.busy() {
            return false;
        }
        self.request = Some(Request::Control(c));
        self.phase = Phase::Start;
        self.cancelled = false;
        true
    }
    pub fn upload(&mut self, rgba: Arc<Bytes>, canvas: Geometry, image: u32) -> bool {
        if self.busy()
            || image <= QUERY_ID
            || rgba.is_empty()
            || rgba.len() > MAX_CANVAS
            || u64::from(canvas.columns) * u64::from(canvas.rows) * 4 != rgba.len() as u64
        {
            return false;
        }
        self.request = Some(Request::Upload(Upload {
            rgba,
            width: canvas.columns,
            height: canvas.rows,
            image,
            raw: 0,
            end: 0,
            first: true,
        }));
        self.phase = Phase::Start;
        self.cancelled = false;
        true
    }
    /// Call only while this graphics request owns the arbiter; if text/OSC52
    /// currently owns it, finish/cancel that transaction before advancing here.
    pub fn cancel(&mut self) {
        if self.busy() && !self.cancelled {
            self.cancelled = true;
            self.packet.clear();
            self.at = 0;
            self.group_len = 0;
            self.phase = Phase::Start
        }
    }
    pub fn take_finished(&mut self) -> Option<Finished> {
        if !self.finished() {
            return None;
        }
        let request = self.request.take().unwrap();
        Some(Finished {
            pin: match request {
                Request::Upload(u) => Some(u.rgba),
                _ => None,
            },
            cancelled: self.cancelled,
        })
    }
    /// At most one complete packet per call. Text may run between upload
    /// packets; no other graphics request can enter until take_finished.
    pub fn advance<W: Write + ?Sized>(&mut self, w: &mut W, b: &mut Budget) -> io::Result<bool> {
        if !self.busy() {
            return Ok(true);
        }
        loop {
            match self.phase {
                Phase::Done => return Ok(true),
                Phase::Start => {
                    if !b.charge(WorkLimit {
                        queries: 1,
                        objects: 1,
                        ..Default::default()
                    }) {
                        return Ok(false);
                    }
                    self.header = if self.cancelled {
                        let mut h = Header::default();
                        h.text(b"\x18\x1b\\");
                        if let Some(Request::Upload(u)) = &self.request {
                            h.text(b"\x1b_Ga=d,d=I,i=");
                            h.number(u.image);
                            h.text(b";\x1b\\")
                        }
                        h
                    } else {
                        match self.request.as_mut().unwrap() {
                            Request::Control(c) => command(*c),
                            Request::Upload(u) => {
                                u.end = (u.raw + RAW_CHUNK).min(u.rgba.len());
                                let mut h = Header::default();
                                if u.first {
                                    h.text(b"\x1b_Ga=t,t=d,f=32");
                                    h.field(b",s=", u.width);
                                    h.field(b",v=", u.height);
                                    h.field(b",i=", u.image);
                                    h.text(b",m=");
                                    u.first = false
                                } else {
                                    h.text(b"\x1b_Gm=")
                                }
                                h.number(u32::from(u.end < u.rgba.len()));
                                h.text(b";");
                                h
                            }
                        }
                    };
                    self.phase = Phase::Header;
                }
                Phase::Header => match self.header.byte(b) {
                    Err(()) => return Ok(false),
                    Ok(Some(v)) => {
                        self.packet.push(v);
                        self.work.prepared_bytes += 1
                    }
                    Ok(None) => {
                        self.phase =
                            if !self.cancelled && matches!(self.request, Some(Request::Upload(_))) {
                                Phase::Raw
                            } else {
                                Phase::Write
                            }
                    }
                },
                Phase::Raw => {
                    let Some(Request::Upload(u)) = self.request.as_mut() else {
                        unreachable!()
                    };
                    if u.raw == u.end && self.group_len == 0 {
                        self.phase = Phase::End;
                        continue;
                    }
                    if self.group_len < 3 && u.raw < u.end {
                        if !b.charge(WorkLimit {
                            bytes: 1,
                            queries: 1,
                            objects: 1,
                            ..Default::default()
                        }) {
                            return Ok(false);
                        }
                        self.group[self.group_len] = u.rgba.byte(u.raw);
                        self.group_len += 1;
                        u.raw += 1;
                        self.work.raw_reads += 1;
                        continue;
                    }
                    if !b.charge(WorkLimit {
                        bytes: 4,
                        queries: 1,
                        objects: 1,
                        ..Default::default()
                    }) {
                        return Ok(false);
                    }
                    let mut out = [0; 4];
                    STANDARD
                        .encode_slice(&self.group[..self.group_len], &mut out)
                        .expect("one base64 group");
                    self.packet.extend_from_slice(&out);
                    self.group_len = 0;
                    self.work.base64_bytes += 4;
                    self.work.prepared_bytes += 4;
                }
                Phase::End => {
                    if !b.charge(WorkLimit {
                        bytes: 2,
                        queries: 1,
                        objects: 1,
                        ..Default::default()
                    }) {
                        return Ok(false);
                    }
                    self.packet.extend_from_slice(b"\x1b\\");
                    self.work.prepared_bytes += 2;
                    self.phase = Phase::Write
                }
                Phase::Write => {
                    if self.at == self.packet.len() {
                        self.work.packets += 1;
                        self.packet.clear();
                        self.at = 0;
                        let more = !self.cancelled
                            && matches!(&self.request,Some(Request::Upload(u)) if u.raw<u.rgba.len());
                        self.phase = if more { Phase::Start } else { Phase::Done };
                        return Ok(!more);
                    }
                    if b.remaining().bytes == 0
                        || !b.charge(WorkLimit {
                            queries: 1,
                            objects: 1,
                            ..Default::default()
                        })
                    {
                        return Ok(false);
                    }
                    let n = (self.packet.len() - self.at).min(b.remaining().bytes);
                    match w.write(&self.packet[self.at..self.at + n]) {
                        Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                        Ok(n) => {
                            assert!(b.charge(WorkLimit {
                                bytes: n,
                                ..Default::default()
                            }));
                            self.at += n;
                            self.work.wire_bytes += n
                        }
                        Err(e)
                            if matches!(
                                e.kind(),
                                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                            ) =>
                        {
                            self.work.would_block += 1;
                            return Ok(false);
                        }
                        Err(e) => return Err(e),
                    }
                }
            }
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Mode {
    Auto,
    #[default]
    Off,
    Kitty,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Support {
    Disabled,
    Pending,
    Ready(CellPixels),
    Unsupported,
    MissingGeometry,
}
pub struct Capability {
    pub support: Support,
    ack: bool,
    cell: Option<CellPixels>,
    sent: Option<std::time::Instant>,
    begun: bool,
}
impl Capability {
    pub fn new(mode: Mode) -> Self {
        Self {
            support: if mode == Mode::Off {
                Support::Disabled
            } else {
                Support::Pending
            },
            ack: false,
            cell: None,
            sent: None,
            begun: false,
        }
    }
    pub fn begin(&mut self) {
        self.begun = true
    }
    pub fn sent(&mut self, now: std::time::Instant) {
        self.begun = true;
        self.sent = Some(now)
    }
    pub fn input(&mut self, e: super::input::Input) {
        if self.support != Support::Pending || !self.begun {
            return;
        }
        match e {
            super::input::Input::Graphics(r) if r.image == QUERY_ID => {
                self.ack = r.ok;
                if !r.ok {
                    self.support = Support::Unsupported
                }
            }
            super::input::Input::Cell(c)
                if c.width > 0 && c.height > 0 && c.width <= 4096 && c.height <= 4096 =>
            {
                self.cell = Some(c)
            }
            super::input::Input::Device if !self.ack => self.support = Support::Unsupported,
            _ => {}
        }
        if self.ack
            && let Some(c) = self.cell
        {
            self.support = Support::Ready(c)
        }
    }
    pub fn tick(&mut self, now: std::time::Instant) {
        if self.support == Support::Pending
            && self.sent.is_some_and(|t| {
                now.saturating_duration_since(t) >= std::time::Duration::from_millis(200)
            })
        {
            self.support = if self.ack {
                Support::MissingGeometry
            } else {
                Support::Unsupported
            }
        }
    }
    pub fn resize(&mut self) {
        if self.support != Support::Disabled {
            self.support = Support::Pending;
            self.ack = false;
            self.cell = None;
            self.sent = None;
            self.begun = false
        }
    }
    pub fn query_needed(&self) -> bool {
        self.support == Support::Pending && !self.begun
    }
}
