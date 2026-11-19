//! Session graphics state. Only FrameRuntime's arbiter may advance its wire.
use super::{
    bytes::Bytes,
    cache::{self, ResourceId},
    input::Input,
    protocol::{Capability, Command, Mode, Placement, Support, Wire},
    resources::Lease,
};
use mdtui3::{Budget, WorkLimit};
use std::{
    collections::VecDeque,
    io::{self, Write},
    sync::Arc,
    time::{Duration, Instant},
};
pub struct Session {
    pub wire: Wire,
    pub capability: Capability,
    query: bool,
    retiring: Option<Arc<Bytes>>,
    terminal: Terminal,
    operation: Option<Operation>,
    closing: bool,
    scene_owner: Option<u64>,
}
impl Session {
    pub fn new(mode: Mode) -> Self {
        Self {
            wire: Wire::default(),
            capability: Capability::new(mode),
            query: false,
            retiring: None,
            terminal: Terminal::default(),
            operation: None,
            closing: false,
            scene_owner: None,
        }
    }
    pub fn input(&mut self, e: Input) {
        self.capability.input(e);
        if let Input::Graphics(reply) = e
            && reply.image != super::protocol::QUERY_ID
        {
            // One bounded tty owner hands off a fixed-size reply, never a
            // callback or source buffer. Processing is paid by advance().
            if self.terminal.replies.len() < 64 {
                self.terminal.replies.push_back(reply)
            } else {
                self.terminal.dropped_replies += 1
            }
        }
    }
    pub fn tick(&mut self, now: Instant) {
        self.capability.tick(now);
        self.terminal.now = now
    }
    pub fn resize(&mut self) {
        self.capability.resize();
        self.scene_owner = None;
        self.terminal.clear_at = Some(0);
    }
    fn retire(&mut self, b: &mut Budget) -> bool {
        if let Some(pin) = self.retiring.as_mut() {
            if let Some(bytes) = Arc::get_mut(pin)
                && !bytes.retire(b)
            {
                return false;
            }
            if !b.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            }) {
                return false;
            }
            self.retiring = None;
        }
        true
    }
    pub fn advance<W: Write + ?Sized>(&mut self, w: &mut W, b: &mut Budget) -> io::Result<()> {
        if !self.retire(b) {
            return Ok(());
        }
        if self.wire.finished() {
            if !pay(b) {
                return Ok(());
            }
            let done = self.wire.take_finished().unwrap();
            self.retiring = done.pin;
            if self.query {
                if !done.cancelled {
                    self.capability.sent(Instant::now())
                }
                self.query = false;
            }
            if let Some(op) = self.operation.take() {
                self.terminal.finished(op, done.cancelled)
            }
            return Ok(());
        }
        if !self.wire.busy() {
            if self.capability.query_needed() && !self.closing {
                if !pay(b) {
                    return Ok(());
                }
                self.wire.control(Command::Query);
                self.query = true;
            } else {
                self.operation = self.terminal.prepare(
                    &mut self.wire,
                    self.capability.support,
                    self.closing,
                    self.scene_owner.is_some(),
                    b,
                );
            }
        }
        let before = self.wire.work.wire_bytes;
        self.wire.advance(w, b)?;
        if self.query && self.wire.work.wire_bytes > before {
            self.capability.begin()
        }
        Ok(())
    }
    /// The caller retains a rejected lease, including under zero credit. A
    /// cache slot cannot change generation until its owned terminal ID has
    /// been deleted and both source/pixel pins have completed paid retirement.
    // Fixed metadata/pins, not the pixel payload. Zero-credit rejection must
    // return the original owner without allocating a new box.
    #[allow(clippy::result_large_err)]
    pub fn admit(&mut self, lease: Lease, b: &mut Budget) -> Result<(), Lease> {
        if self.closing || !matches!(self.capability.support, Support::Ready(_)) || !pay(b) {
            return Err(lease);
        }
        self.terminal.admit(lease)
    }
    pub(crate) fn pause_scene(&mut self) -> Option<u64> {
        if self.closing || self.terminal.clear_at.is_some() {
            return None;
        }
        // Unique across all sessions as well as replacements in one session.
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let id = NEXT
            .try_update(
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
                |n| n.checked_add(1),
            )
            .expect("terminal scene identity exhausted");
        self.scene_owner = Some(id);
        Some(id)
    }
    pub(crate) fn clearing_scene(&self) -> bool {
        self.terminal.clear_at.is_some()
    }
    pub(crate) fn owns_scene(&self, id: u64) -> bool {
        self.scene_owner == Some(id)
    }
    pub(crate) fn finish_scene(&mut self, id: u64) -> bool {
        if !self.owns_scene(id) {
            return false;
        }
        self.scene_owner = None;
        true
    }
    pub fn image_status(&self, id: ResourceId) -> Option<ImageStatus> {
        self.terminal.get(id).map(|g| g.status())
    }
    pub fn image_id(&self, id: ResourceId) -> Option<u32> {
        self.terminal.get(id).map(|g| g.image)
    }
    /// Stable native band slots are supplied by the visible-frame owner. One
    /// placement may crop a contiguous visible slice of the padded raster.
    pub fn place(&mut self, slot: usize, id: ResourceId, mut p: Placement, b: &mut Budget) -> bool {
        if self.closing || self.terminal.clear_at.is_some() || !pay(b) {
            return false;
        }
        let Some(g) = self.terminal.get(id) else {
            return false;
        };
        let Some(lease) = g.lease.as_ref() else {
            return false;
        };
        if slot >= PLACEMENTS
            || g.retiring
            || p.cells.columns == 0
            || p.cells.rows == 0
            || p.cells.columns > 1000
            || p.cells.rows > 1000
            || p.crop.columns == 0
            || p.crop.rows == 0
            || p.crop_x
                .checked_add(p.crop.columns)
                .is_none_or(|n| n > lease.canvas.columns)
            || p.crop_y
                .checked_add(p.crop.rows)
                .is_none_or(|n| n > lease.canvas.rows)
        {
            return false;
        }
        p.image = g.image;
        p.placement = slot as u32 + 1;
        self.terminal.visuals[slot].wanted = Some(Target {
            id,
            placement: p,
            epoch: g.epoch,
            paint: self.terminal.paint_epoch,
        });
        true
    }
    /// Actual text output can erase graphics placements. Reissue only finite
    /// placement controls after that wire turn; the raster is never re-read.
    pub fn repainted(&mut self) {
        self.terminal.paint_epoch = self.terminal.paint_epoch.wrapping_add(1)
    }
    pub fn hide(&mut self, slot: usize, b: &mut Budget) -> bool {
        if slot >= PLACEMENTS || !pay(b) {
            return false;
        }
        self.terminal.visuals[slot].wanted = None;
        true
    }
    pub fn release(&mut self, id: ResourceId, b: &mut Budget) -> bool {
        if !pay(b) {
            return false;
        }
        self.mark_retiring(id);
        true
    }
    fn mark_retiring(&mut self, id: ResourceId) {
        let Some(g) = self.terminal.get_mut(id) else {
            return;
        };
        g.retiring = true;
        if matches!(self.operation,Some(Operation::Upload(i)) if i==id) {
            self.wire.cancel()
        }
    }
    pub(crate) fn retire_unneeded(
        &mut self,
        slot: usize,
        required: Option<ResourceId>,
        b: &mut Budget,
    ) -> bool {
        if slot >= 64 || !pay(b) {
            return false;
        }
        if let Some(g) = self.terminal.images[slot].as_ref()
            && required != Some(g.id)
        {
            let id = g.id;
            self.mark_retiring(id);
        }
        true
    }
    pub fn capacity(&self) -> TerminalCapacity {
        self.terminal.capacity()
    }
    pub fn abort<W: Write + ?Sized>(&mut self, w: &mut W, b: &mut Budget) -> io::Result<bool> {
        if !self.closing {
            if !pay(b) {
                return Ok(false);
            }
            self.closing = true;
            self.scene_owner = None;
            self.capability.support = Support::Disabled;
            if self.wire.busy() && !matches!(self.operation, Some(Operation::Free(_))) {
                self.wire.cancel()
            }
            self.terminal.clear_at = Some(0);
        }
        self.advance(w, b)?;
        Ok(!self.wire.busy() && self.retiring.is_none() && self.terminal.empty())
    }
}
fn pay(b: &mut Budget) -> bool {
    b.charge(WorkLimit {
        queries: 1,
        objects: 1,
        ..Default::default()
    })
}
const PLACEMENTS: usize = 256;
pub const TERMINAL_PIXEL_BYTES: usize = 16 * 1024 * 1024;
const ACK_TIMEOUT: Duration = Duration::from_secs(2);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageStatus {
    Pending,
    Ready,
    Rejected,
    AckTimeout,
    Retiring,
}
#[derive(Clone, Copy)]
enum Stage {
    New,
    Uploading,
    Await(Instant),
    Ready,
    Delete,
    Failed,
    Retire,
}
struct Gpu {
    id: ResourceId,
    image: u32,
    lease: Option<Lease>,
    stage: Stage,
    ack: Option<bool>,
    error: Option<ImageStatus>,
    retiring: bool,
    resident: bool,
    bytes: usize,
    epoch: u32,
    retried: bool,
}
impl Gpu {
    fn status(&self) -> ImageStatus {
        if self.retiring {
            ImageStatus::Retiring
        } else if let Some(e) = self.error {
            e
        } else if matches!(self.stage, Stage::Ready) {
            ImageStatus::Ready
        } else {
            ImageStatus::Pending
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Target {
    id: ResourceId,
    placement: Placement,
    epoch: u32,
    paint: u64,
}
#[derive(Default)]
struct Visual {
    wanted: Option<Target>,
    sent: Option<Target>,
}
#[derive(Clone, Copy)]
enum Operation {
    Upload(ResourceId),
    Free(ResourceId),
    Place(usize, Target),
    Hide(usize, Target),
}
struct RetireLease {
    slot: usize,
    buffers: [Option<Arc<Bytes>>; 2],
    at: usize,
}
#[derive(Clone, Copy, Default, Debug)]
pub struct TerminalCapacity {
    pub pixel_reservations: usize,
    pub pixel_resident_bound: usize,
    pub current_cpu_pins: usize,
    pub retiring_cpu_pins: usize,
    pub descriptor_bound: usize,
    pub images: usize,
    pub placements: usize,
    pub dropped_replies: u64,
}
struct Terminal {
    images: [Option<Gpu>; 64],
    visuals: [Visual; PLACEMENTS],
    replies: VecDeque<super::input::GraphicsReply>,
    serial: u32,
    reserved: usize,
    paint_epoch: u64,
    scan_at: usize,
    image_count: usize,
    placed_count: usize,
    clear_at: Option<usize>,
    retiring: Option<RetireLease>,
    now: Instant,
    dropped_replies: u64,
}
impl Default for Terminal {
    fn default() -> Self {
        Self {
            images: std::array::from_fn(|_| None),
            visuals: std::array::from_fn(|_| Visual::default()),
            replies: VecDeque::with_capacity(64),
            serial: 1,
            reserved: 0,
            paint_epoch: 0,
            scan_at: 0,
            image_count: 0,
            placed_count: 0,
            clear_at: None,
            retiring: None,
            now: Instant::now(),
            dropped_replies: 0,
        }
    }
}
impl Terminal {
    fn get(&self, id: ResourceId) -> Option<&Gpu> {
        self.images
            .get(usize::from(id.slot))?
            .as_ref()
            .filter(|g| g.id == id)
    }
    fn get_mut(&mut self, id: ResourceId) -> Option<&mut Gpu> {
        self.images
            .get_mut(usize::from(id.slot))?
            .as_mut()
            .filter(|g| g.id == id)
    }
    #[allow(clippy::result_large_err)] // same fixed descriptor as Session::admit
    fn admit(&mut self, lease: Lease) -> Result<(), Lease> {
        let slot = usize::from(lease.id.slot);
        let n = lease.rgba.len();
        if slot >= 64
            || self.images[slot].is_some()
            || lease.failed
            || n == 0
            || n > super::MAX_CANVAS
            || u64::from(lease.canvas.columns) * u64::from(lease.canvas.rows) * 4 != n as u64
            || self.reserved + n > TERMINAL_PIXEL_BYTES
            || self.serial > u32::MAX / 64
        {
            return Err(lease);
        }
        let image = self.serial * 64 + slot as u32;
        self.serial += 1;
        self.reserved += n;
        self.image_count += 1;
        self.images[slot] = Some(Gpu {
            id: lease.id,
            image,
            lease: Some(lease),
            stage: Stage::New,
            ack: None,
            error: None,
            retiring: false,
            resident: false,
            bytes: n,
            epoch: 0,
            retried: false,
        });
        Ok(())
    }
    fn reply(&mut self, r: super::input::GraphicsReply) {
        if let Some(p) = r.placement
            && (p == 0
                || p > PLACEMENTS as u32
                || self.visuals[p as usize - 1]
                    .sent
                    .is_none_or(|t| t.placement.image != r.image))
        {
            return;
        }
        let Some(g) = self.images[(r.image & 63) as usize]
            .as_mut()
            .filter(|g| g.image == r.image && !g.retiring)
        else {
            return;
        };
        if r.placement.is_none() && matches!(g.stage, Stage::Uploading | Stage::Await(_)) {
            g.ack = Some(r.ok);
            if matches!(g.stage, Stage::Await(_)) {
                if r.ok {
                    g.stage = Stage::Ready
                } else {
                    g.error = Some(ImageStatus::Rejected);
                    g.stage = Stage::Delete
                }
            }
        } else if r.placement.is_some_and(|p| p > 0 && p <= PLACEMENTS as u32)
            && !r.ok
            && matches!(g.stage, Stage::Ready)
        {
            // ENOENT after terminal eviction permits one coalesced re-upload,
            // never a retry per frame. The epoch invalidates old placements.
            if r.missing && !g.retried {
                g.retried = true;
                g.epoch += 1;
                g.ack = None;
                g.stage = Stage::New
            } else {
                g.error = Some(ImageStatus::Rejected);
                g.stage = Stage::Delete
            }
        }
    }
    fn retire_lease(&mut self, b: &mut Budget) -> bool {
        let Some(r) = self.retiring.as_mut() else {
            return true;
        };
        while r.at < 2 {
            if let Some(pin) = r.buffers[r.at].as_mut() {
                if let Some(bytes) = Arc::get_mut(pin)
                    && !bytes.retire(b)
                {
                    return false;
                }
                if !pay(b) {
                    return false;
                }
                r.buffers[r.at] = None;
            }
            r.at += 1;
        }
        if !pay(b) {
            return false;
        }
        let slot = r.slot;
        let g = self.images[slot].take().unwrap();
        self.reserved -= g.bytes;
        self.image_count -= 1;
        self.retiring = None;
        true
    }
    fn prepare(
        &mut self,
        wire: &mut Wire,
        support: Support,
        closing: bool,
        scene_paused: bool,
        b: &mut Budget,
    ) -> Option<Operation> {
        if !self.retire_lease(b) {
            return None;
        }
        while let Some(at) = self.clear_at {
            if at == PLACEMENTS {
                self.clear_at = None;
                break;
            }
            if !pay(b) {
                return None;
            }
            self.visuals[at].wanted = None;
            self.clear_at = Some(at + 1);
        }
        if !self.replies.is_empty() {
            if !pay(b) {
                return None;
            }
            let r = self.replies.pop_front().unwrap();
            self.reply(r);
        }
        if self.image_count == 0 && self.placed_count == 0 {
            return None;
        }
        // A retained cursor gives micro budgets the same progress as a large
        // frame. Each lookup, timeout and descriptor transition is paid.
        for _ in 0..64 + PLACEMENTS {
            if !pay(b) {
                return None;
            }
            let at = self.scan_at;
            self.scan_at = (at + 1) % (64 + PLACEMENTS);
            if at < 64 {
                let slot = at;
                let Some(g) = self.images[slot].as_mut() else {
                    continue;
                };
                if closing {
                    g.retiring = true
                }
                if g.retiring && !matches!(g.stage, Stage::Delete | Stage::Retire) {
                    g.stage = if g.resident {
                        Stage::Delete
                    } else {
                        Stage::Retire
                    };
                }
                if let Stage::Await(t) = g.stage
                    && self.now.saturating_duration_since(t) >= ACK_TIMEOUT
                {
                    g.error = Some(ImageStatus::AckTimeout);
                    g.stage = Stage::Delete;
                }
                match g.stage {
                    Stage::New if matches!(support, Support::Ready(_)) => {
                        let lease = g.lease.as_ref().unwrap();
                        if wire.upload(lease.rgba.clone(), lease.canvas, g.image) {
                            g.stage = Stage::Uploading;
                            g.resident = true;
                            return Some(Operation::Upload(g.id));
                        }
                    }
                    Stage::Delete => {
                        wire.control(Command::Delete {
                            image: g.image,
                            placement: None,
                            free: true,
                        });
                        return Some(Operation::Free(g.id));
                    }
                    Stage::Retire => {
                        let lease = g.lease.take().unwrap();
                        self.retiring = Some(RetireLease {
                            slot,
                            buffers: [Some(lease.uri), Some(lease.rgba)],
                            at: 0,
                        });
                        return None;
                    }
                    _ => {}
                }
                continue;
            }
            if scene_paused && !closing {
                continue;
            }
            let slot = at - 64;
            let v = &self.visuals[slot];
            let wanted = v.wanted.and_then(|mut t| {
                let g = self.get(t.id)?;
                if g.retiring || !matches!(g.stage, Stage::Ready) {
                    return None;
                }
                t.epoch = g.epoch;
                t.paint = self.paint_epoch;
                Some(t)
            });
            if let Some(sent) = v.sent
                && wanted.is_none_or(|t| t.id != sent.id)
            {
                wire.control(Command::Delete {
                    image: sent.placement.image,
                    placement: Some(sent.placement.placement),
                    free: false,
                });
                return Some(Operation::Hide(slot, sent));
            }
            if let Some(t) = wanted
                && v.sent != Some(t)
            {
                wire.control(Command::Place(t.placement));
                return Some(Operation::Place(slot, t));
            }
        }
        None
    }
    fn finished(&mut self, op: Operation, cancelled: bool) {
        match op {
            Operation::Upload(id) => {
                let now = self.now;
                if let Some(g) = self.get_mut(id) {
                    if cancelled {
                        g.resident = false;
                        g.stage = if g.retiring {
                            Stage::Retire
                        } else {
                            g.error = Some(ImageStatus::Rejected);
                            Stage::Failed
                        }
                    } else {
                        g.stage = match g.ack {
                            Some(true) => Stage::Ready,
                            Some(false) => {
                                g.error = Some(ImageStatus::Rejected);
                                Stage::Delete
                            }
                            None => Stage::Await(now),
                        }
                    }
                }
            }
            Operation::Free(id) => {
                if let Some(g) = self.get_mut(id) {
                    g.resident = false;
                    g.stage = if g.retiring {
                        Stage::Retire
                    } else {
                        Stage::Failed
                    }
                }
            }
            Operation::Place(slot, t) => {
                if !cancelled {
                    if self.visuals[slot].sent.is_none() {
                        self.placed_count += 1
                    }
                    self.visuals[slot].sent = Some(t)
                }
            }
            Operation::Hide(slot, t) => {
                if !cancelled && self.visuals[slot].sent == Some(t) {
                    self.visuals[slot].sent = None;
                    self.placed_count -= 1
                }
            }
        }
    }
    fn empty(&self) -> bool {
        self.image_count == 0 && self.placed_count == 0 && self.retiring.is_none()
    }
    fn capacity(&self) -> TerminalCapacity {
        let mut c = TerminalCapacity {
            pixel_reservations: self.reserved,
            descriptor_bound: std::mem::size_of::<Self>()
                + self.replies.capacity() * std::mem::size_of::<super::input::GraphicsReply>(),
            dropped_replies: self.dropped_replies,
            ..Default::default()
        };
        for g in self.images.iter().flatten() {
            c.images += 1;
            if g.resident {
                c.pixel_resident_bound += g.bytes
            }
            if let Some(lease) = &g.lease {
                let n = cache::cost(&lease.uri) + cache::cost(&lease.rgba);
                if g.retiring {
                    c.retiring_cpu_pins += n
                } else {
                    c.current_cpu_pins += n
                }
            }
        }
        if let Some(r) = &self.retiring {
            for pin in r.buffers.iter().flatten() {
                c.retiring_cpu_pins += cache::cost(pin)
            }
        }
        c.placements = self.visuals.iter().filter(|v| v.sent.is_some()).count();
        c
    }
}
