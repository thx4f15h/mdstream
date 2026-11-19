//! One owned worker slot and a bounded current resource queue. URI resolution
//! emits an event and waits for caller revalidation before any image fetch.
//! Expensive URL/HTTP/codec calls remain inside the private worker process.
use super::{
    MAX_URI,
    bytes::Bytes,
    cache::{self, Cache, Match, Resource, ResourceId},
    worker::{self, CellPixels, Geometry, Job, Process, Reply, Task, WorkerMetrics},
};
use mdtui3::{Budget, WorkLimit};
use std::{collections::VecDeque, path::PathBuf, sync::Arc};
pub const QUEUE: usize = 8;
// A turn drains old source retirement before advancing a worker. Its largest
// remaining atomic handoff is reply URI + failed record's three buffers +
// canonical URI + encoded pin + two Fetch inputs = eight descriptors.
// Queued requests separately reserve two descriptors, so cancellation needs
// neither spare allocation nor an intervening advance call.
const RETIRING: usize = QUEUE * 2 + 8;
pub const INPUT_BYTES: usize = 1024 * 1024;
pub const READY_URI_BYTES: usize = 2 * 1024 * 1024;
// Includes maximum encoded/raster/URI pipe buffers and their page directories.
// Current cache + receiving reserve stays inside the same 32 MiB admission.
const RECEIVING: usize = 14 * 1024 * 1024;
#[derive(Clone)]
pub struct Fetch {
    pub token: u64,
    pub base: Arc<Bytes>,
    pub reference: Arc<Bytes>,
    pub base_is_path: bool,
    pub cell: CellPixels,
    pub area: Geometry,
}
impl Fetch {
    fn cost(&self) -> usize {
        cache::cost(&self.base) + cache::cost(&self.reference)
    }
}
/// One fixed reference supplied by the caller's current semantic owner.
/// Resolution never opens a URL, fetches an image or publishes OSC8 itself.
pub struct UriRequest {
    pub token: u64,
    pub base: Arc<Bytes>,
    pub reference: Arc<Bytes>,
    pub base_is_path: bool,
}
impl UriRequest {
    fn cost(&self) -> usize {
        cache::cost(&self.base) + cache::cost(&self.reference)
    }
    fn into_input(self) -> Fetch {
        // The common IPC header has geometry fields; Task::Uri never uses
        // them, decodes pixels or enters the image/cache state machine.
        Fetch {
            token: self.token,
            base: self.base,
            reference: self.reference,
            base_is_path: self.base_is_path,
            cell: CellPixels {
                width: 1,
                height: 1,
            },
            area: Geometry {
                columns: 1,
                rows: 1,
            },
        }
    }
}
pub struct Lease {
    pub id: ResourceId,
    pub uri: Arc<Bytes>,
    pub rgba: Arc<Bytes>,
    pub original: Geometry,
    pub canvas: Geometry,
    pub cells: Geometry,
    pub failed: bool,
    pub metrics: WorkerMetrics,
}
pub enum Failure {
    Status(&'static str),
    Diagnostic(Arc<Bytes>),
}
pub enum Event {
    Resolved { token: u64 },
    Ready { token: u64, resource: Lease },
    Failed { token: u64, error: Failure },
    Evicted(ResourceId),
    UriReady { token: u64, uri: Arc<Bytes> },
    Interrupted { token: u64 },
}
#[derive(Clone, Copy, Debug, Default)]
pub struct ResourceWork {
    pub resolves: u64,
    pub uri_resolves: u64,
    pub uri_preemptions: u64,
    pub loads: u64,
    pub cache_hits: u64,
    pub cancellations: u64,
    /// Killed/abandoned jobs without a final metrics report. CPU/body/pixel
    /// totals below cover reports received, not fictional zero-cost cancels.
    pub unreported_workers: u64,
    pub worker_request_bytes: u64,
    pub worker_response_bytes: u64,
    pub worker_would_block: u64,
    pub downloaded_body_bytes: u64,
    pub encoded_bytes: u64,
    pub decoded_pixels: u64,
    pub scaled_pixels: u64,
    pub cpu_us: u64,
    pub resolve_cpu_us: u64,
    pub load_cpu_us: u64,
    pub declared_pixels: u64,
    pub peak_worker_rss_kib: u64,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct ResourceCapacity {
    pub input_current_charged: usize,
    pub retiring_source_descriptors: usize,
    pub queued_retirement_reservations: usize,
    pub source_descriptor_limit: usize,
    pub input_retiring_charged: usize,
    pub cache_current: usize,
    pub cache_pinned_subset: usize,
    pub cache_retiring: usize,
    pub worker_response: usize,
    pub worker_request_pins: usize,
    pub transient: usize,
    pub ready_uri_charged: usize,
    pub descriptor_bound: usize,
}
impl ResourceCapacity {
    pub fn total_bound(&self) -> usize {
        self.input_current_charged
            + self.input_retiring_charged
            + self.cache_current
            + self.cache_retiring
            + self.worker_response
            + self.transient
            + self.ready_uri_charged
            + self.descriptor_bound
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Raster {
    scaled: Geometry,
    canvas: Geometry,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    StartResolve,
    Resolve,
    ReadResolve,
    ReapResolve,
    Announce,
    UriPublish,
    Await,
    Lookup,
    Inspect(ResourceId),
    Admit,
    Evict,
    StartLoad,
    Load,
    ReadLoad,
    ReapLoad,
    Insert,
    Publish,
    Failed,
    Cancel,
    Done,
}
struct Active {
    fetch: Option<Fetch>,
    step: Step,
    process: Option<Process>,
    uri: Option<Arc<Bytes>>,
    hash: u64,
    search: Option<Match>,
    encoded: Option<Arc<Bytes>>,
    reply: Option<Reply>,
    record: Option<Resource>,
    result: Option<ResourceId>,
    cells: Geometry,
    error: Option<Failure>,
    cancelled: bool,
    uri_only: bool,
    interrupted: bool,
    metrics_reported: bool,
}
impl Active {
    fn new(fetch: Fetch) -> Self {
        Self {
            fetch: Some(fetch),
            step: Step::StartResolve,
            process: None,
            uri: None,
            hash: 0,
            search: None,
            encoded: None,
            reply: None,
            record: None,
            result: None,
            cells: Geometry::default(),
            error: None,
            cancelled: false,
            uri_only: false,
            interrupted: false,
            metrics_reported: false,
        }
    }
    fn token(&self) -> u64 {
        self.fetch.as_ref().unwrap().token
    }
}
struct Retiring {
    bytes: Arc<Bytes>,
    input: bool,
}
pub struct Resources {
    executable: PathBuf,
    cache: Cache,
    rasters: [Option<(ResourceId, Raster)>; 64],
    negative: [Option<(ResourceId, CellPixels, Geometry)>; 64],
    queue: VecDeque<Fetch>,
    uri_queue: Option<UriRequest>,
    ready_uri: usize,
    events: VecDeque<Event>,
    active: Option<Box<Active>>,
    retiring: VecDeque<Retiring>,
    input_current: usize,
    input_retiring: usize,
    closing: bool,
    pub work: ResourceWork,
}
fn pay(b: &mut Budget) -> bool {
    b.charge(WorkLimit {
        queries: 1,
        objects: 1,
        ..Default::default()
    })
}
impl Resources {
    pub fn new(executable: PathBuf) -> Self {
        Self {
            executable,
            cache: Cache::default(),
            rasters: [None; 64],
            negative: [None; 64],
            queue: VecDeque::with_capacity(QUEUE),
            uri_queue: None,
            ready_uri: 0,
            events: VecDeque::with_capacity(QUEUE),
            active: None,
            retiring: VecDeque::with_capacity(RETIRING),
            input_current: 0,
            input_retiring: 0,
            closing: false,
            work: ResourceWork::default(),
        }
    }
    pub fn cache(&self) -> &Cache {
        &self.cache
    }
    pub fn active_token(&self) -> Option<u64> {
        self.active.as_ref().map(|a| a.token())
    }
    /// Takes no input on rejection, including a zero Budget. The caller keeps
    /// its owned URI activity until it can be submitted or retired explicitly.
    pub fn submit(&mut self, fetch: Fetch, b: &mut Budget) -> Result<(), Fetch> {
        if !pay(b) {
            return Err(fetch);
        }
        if self.closing
            || self.retiring.len() + 2 * (self.queued() + 1) > RETIRING
            || [
                fetch.cell.width,
                fetch.cell.height,
                fetch.area.columns,
                fetch.area.rows,
            ]
            .contains(&0)
            || fetch.area.columns > 1000
            || fetch.area.rows > 1000
            || fetch.cell.width > 4096
            || fetch.cell.height > 4096
            || self.queued() == QUEUE
            || fetch.base.len() > MAX_URI
            || fetch.reference.len() > MAX_URI
            || self.input_current + self.input_retiring + fetch.cost() > INPUT_BYTES
            || self.has_token(fetch.token)
        {
            return Err(fetch);
        }
        self.input_current += fetch.cost();
        self.queue.push_back(fetch);
        Ok(())
    }
    fn queued(&self) -> usize {
        self.queue.len() + usize::from(self.uri_queue.is_some())
    }
    fn has_token(&self, token: u64) -> bool {
        self.active_token() == Some(token)
            || self.queue.iter().any(|q| q.token == token)
            || self.uri_queue.as_ref().is_some_and(|q| q.token == token)
            || self.events.iter().any(|e| match e {
                Event::Resolved { token: t }
                | Event::Ready { token: t, .. }
                | Event::Failed { token: t, .. }
                | Event::UriReady { token: t, .. }
                | Event::Interrupted { token: t } => *t == token,
                Event::Evicted(_) => false,
            })
    }
    /// One priority pending URI plus the same eight total queued requests.
    /// A full image queue can sacrifice its last queued image; all input
    /// retirement is still reserved before taking the new caller owner.
    pub fn submit_uri(&mut self, request: UriRequest, b: &mut Budget) -> Result<(), UriRequest> {
        if !pay(b) {
            return Err(request);
        }
        let full = self.queued() == QUEUE;
        let retiring = self.retiring.len() + if full { 2 } else { 0 };
        let queued = if full {
            self.queued()
        } else {
            self.queued() + 1
        };
        if self.closing
            || self.uri_queue.is_some()
            || retiring + queued * 2 > RETIRING
            || request.base.len() > MAX_URI
            || request.reference.len() > MAX_URI
            || self.input_current + self.input_retiring + request.cost() > INPUT_BYTES
            || self.has_token(request.token)
            || (full && self.events.len() == QUEUE)
        {
            return Err(request);
        }
        if full {
            let old = self.queue.pop_back().unwrap();
            let token = old.token;
            self.release_fetch(old);
            self.events.push_back(Event::Interrupted { token });
            self.work.cancellations += 1;
            self.work.uri_preemptions += 1;
        }
        if let Some(active) = self.active.as_mut()
            && !active.uri_only
            && !active.cancelled
        {
            active.cancelled = true;
            active.interrupted = true;
            if let Some(p) = active.process.as_mut() {
                p.cancel()
            }
            self.work.cancellations += 1;
            self.work.uri_preemptions += 1;
        }
        self.input_current += request.cost();
        self.uri_queue = Some(request);
        Ok(())
    }
    /// Resolution is complete but no network/decode has started. This borrowed
    /// fixed backing may be copied/read only with the consumer's byte credit.
    pub fn resolved_uri(&self, token: u64) -> Option<&Bytes> {
        let a = self
            .active
            .as_ref()
            .filter(|a| a.token() == token && matches!(a.step, Step::Await | Step::Announce))?;
        a.uri.as_deref()
    }
    /// The current selected semantic producer must revalidate its native owner
    /// before calling this method. Approval is never inferred from elapsed time.
    pub fn approve(&mut self, token: u64, b: &mut Budget) -> bool {
        if !pay(b) {
            return false;
        }
        let Some(a) = self
            .active
            .as_mut()
            .filter(|a| a.token() == token && a.step == Step::Await && !a.cancelled)
        else {
            return false;
        };
        a.search = Some(Match::new(a.uri.as_ref().unwrap().clone(), a.hash));
        a.step = Step::Lookup;
        true
    }
    pub fn event(&mut self, b: &mut Budget) -> Option<Event> {
        if self.events.is_empty() {
            return None;
        }
        if !pay(b) {
            return None;
        }
        let event = self.events.pop_front()?;
        if let Event::UriReady { uri, .. } = &event {
            self.ready_uri -= cache::cost(uri)
        }
        Some(event)
    }
    fn retire_arc(&mut self, bytes: Arc<Bytes>, input: bool) {
        assert!(
            self.retiring.len() < RETIRING,
            "bounded image retirement descriptors"
        );
        if input {
            self.input_retiring += cache::cost(&bytes)
        }
        self.retiring.push_back(Retiring { bytes, input })
    }
    fn retire_bytes(&mut self, bytes: Bytes) {
        self.retire_arc(Arc::new(bytes), false)
    }
    fn retire_sources(&mut self, b: &mut Budget) -> bool {
        while let Some(front) = self.retiring.front_mut() {
            if let Some(bytes) = Arc::get_mut(&mut front.bytes) {
                let before = cache::cost(bytes);
                let done = bytes.retire(b);
                if front.input {
                    self.input_retiring -= before - cache::cost(bytes)
                }
                if !done {
                    return false;
                }
            }
            if !pay(b) {
                return false;
            }
            let r = self.retiring.pop_front().unwrap();
            if r.input {
                self.input_retiring -= cache::cost(&r.bytes)
            }
        }
        true
    }
    fn release_fetch(&mut self, fetch: Fetch) {
        self.input_current -= fetch.cost();
        self.retire_arc(fetch.base, true);
        self.retire_arc(fetch.reference, true)
    }
    pub fn cancel(&mut self, token: u64, b: &mut Budget) -> bool {
        if !pay(b) {
            return false;
        }
        if let Some(a) = self.active.as_mut().filter(|a| a.token() == token) {
            if !a.cancelled {
                a.cancelled = true;
                if let Some(p) = a.process.as_mut() {
                    p.cancel()
                }
                self.work.cancellations += 1;
            }
            return true;
        }
        if self.uri_queue.as_ref().is_some_and(|q| q.token == token) {
            let q = self.uri_queue.take().unwrap();
            self.release_fetch(q.into_input());
            self.work.cancellations += 1;
            return true;
        }
        if let Some(at) = self.queue.iter().position(|r| r.token == token) {
            let fetch = self.queue.remove(at).unwrap();
            self.release_fetch(fetch);
            self.work.cancellations += 1;
        }
        true
    }
    pub fn cancel_all(&mut self, b: &mut Budget) -> bool {
        if !self.closing {
            if !pay(b) {
                return false;
            }
            self.closing = true;
        }
        if let Some(a) = self.active.as_ref()
            && !a.cancelled
        {
            let token = a.token();
            if !self.cancel(token, b) {
                return false;
            }
        }
        if self.uri_queue.is_some() {
            if !pay(b) {
                return false;
            }
            let q = self.uri_queue.take().unwrap();
            self.release_fetch(q.into_input());
            self.work.cancellations += 1;
        }
        while !self.queue.is_empty() {
            if !pay(b) {
                return false;
            }
            let fetch = self.queue.pop_front().unwrap();
            self.release_fetch(fetch);
            self.work.cancellations += 1;
        }
        true
    }
    fn reaped(&mut self, a: &mut Active, b: &mut Budget) -> bool {
        let Some(p) = a.process.as_mut() else {
            return true;
        };
        match p.retire(b) {
            Ok(true) => {
                let p = a.process.take().unwrap();
                if !a.metrics_reported {
                    self.work.unreported_workers += 1;
                }
                self.work.worker_request_bytes += p.work.request_bytes as u64;
                self.work.worker_response_bytes += p.work.response_bytes as u64;
                self.work.worker_would_block += p.work.would_block as u64;
                true
            }
            _ => false,
        }
    }
    fn release_active(&mut self, a: &mut Active) {
        if let Some(reply) = a.reply.take() {
            self.retire_bytes(reply.uri);
            self.retire_bytes(reply.encoded);
            self.retire_bytes(reply.rgba)
        }
        if let Some(r) = a.record.take() {
            self.retire_arc(r.uri, false);
            self.retire_arc(r.encoded, false);
            self.retire_arc(r.rgba, false)
        }
        if let Some(uri) = a.uri.take() {
            self.retire_arc(uri, false)
        }
        if let Some(encoded) = a.encoded.take() {
            self.retire_arc(encoded, false)
        }
        a.search = None;
        if let Some(fetch) = a.fetch.take() {
            self.release_fetch(fetch)
        }
    }
    /// One advancement only; the host schedules waiting worker/capacity turns
    /// at its ordinary frame/input deadline, never an EAGAIN retry spin.
    pub fn advance(&mut self, b: &mut Budget) {
        if !self.retire_sources(b) {
            return;
        }
        self.cache.retire(b);
        if self.active.is_none() {
            if self.closing || self.queued() == 0 || !pay(b) {
                return;
            }
            self.active = Some(Box::new(if let Some(uri) = self.uri_queue.take() {
                let mut a = Active::new(uri.into_input());
                a.uri_only = true;
                a
            } else {
                Active::new(self.queue.pop_front().unwrap())
            }))
        }
        let mut a = self.active.take().unwrap();
        let done = self.advance_active(&mut a, b);
        if done {
            self.release_active(&mut a)
        } else {
            self.active = Some(a)
        }
    }
    fn advance_active(&mut self, a: &mut Active, b: &mut Budget) -> bool {
        loop {
            if a.cancelled {
                a.step = Step::Cancel
            }
            match a.step {
                Step::Done => return true,
                Step::StartResolve | Step::StartLoad => {
                    if !pay(b) {
                        return false;
                    }
                    let f = a.fetch.as_ref().unwrap();
                    let resolve = a.step == Step::StartResolve;
                    let job = Job {
                        task: if resolve {
                            if a.uri_only {
                                Task::Uri
                            } else {
                                Task::ImageUri
                            }
                        } else {
                            Task::Load
                        },
                        base_is_path: resolve && f.base_is_path,
                        base: if resolve {
                            f.base.clone()
                        } else {
                            Arc::new(Bytes::default())
                        },
                        target: if resolve {
                            f.reference.clone()
                        } else {
                            a.uri.as_ref().unwrap().clone()
                        },
                        encoded: if resolve { None } else { a.encoded.clone() },
                        cell: f.cell,
                        area: f.area,
                    };
                    match Process::spawn(&self.executable, job) {
                        Ok(p) => {
                            a.process = Some(p);
                            a.metrics_reported = false;
                            if resolve {
                                if a.uri_only {
                                    self.work.uri_resolves += 1
                                } else {
                                    self.work.resolves += 1;
                                }
                                a.step = Step::Resolve
                            } else {
                                self.work.loads += 1;
                                a.step = Step::Load
                            }
                        }
                        Err(_) => {
                            a.error = Some(Failure::Status("image worker could not start"));
                            a.step = Step::Failed
                        }
                    }
                }
                Step::Resolve | Step::Load => match a.process.as_mut().unwrap().advance(b) {
                    Ok(true) => {
                        a.step = if a.step == Step::Resolve {
                            Step::ReadResolve
                        } else {
                            Step::ReadLoad
                        }
                    }
                    Ok(false) => return false,
                    Err(_) => {
                        a.error = Some(Failure::Status("image worker stopped or timed out"));
                        a.step = if a.step == Step::Resolve {
                            Step::ReapResolve
                        } else {
                            Step::ReapLoad
                        }
                    }
                },
                Step::ReadResolve => {
                    if !pay(b) {
                        return false;
                    }
                    let reply = a.process.as_mut().unwrap().take_reply().unwrap();
                    a.metrics_reported = true;
                    self.work.cpu_us += reply.metrics.cpu_us;
                    self.work.resolve_cpu_us += reply.metrics.cpu_us;
                    self.work.peak_worker_rss_kib = self
                        .work
                        .peak_worker_rss_kib
                        .max(reply.metrics.peak_rss_kib);
                    if reply.error {
                        a.error = Some(Failure::Diagnostic(Arc::new(reply.rgba)));
                        self.retire_bytes(reply.uri)
                    } else {
                        a.hash = reply.uri_hash;
                        a.uri = Some(Arc::new(reply.uri));
                        self.retire_bytes(reply.rgba)
                    }
                    self.retire_bytes(reply.encoded);
                    a.step = Step::ReapResolve;
                }
                Step::ReapResolve => {
                    if !self.reaped(a, b) {
                        return false;
                    }
                    a.step = if a.error.is_some() {
                        Step::Failed
                    } else if a.uri_only {
                        Step::UriPublish
                    } else {
                        Step::Announce
                    }
                }
                Step::UriPublish => {
                    if self.events.len() == QUEUE || !pay(b) {
                        return false;
                    }
                    let uri = a.uri.as_ref().unwrap();
                    let cost = cache::cost(uri);
                    if self.ready_uri + cost > READY_URI_BYTES {
                        return false;
                    }
                    self.ready_uri += cost;
                    self.events.push_back(Event::UriReady {
                        token: a.token(),
                        uri: uri.clone(),
                    });
                    a.step = Step::Done;
                }
                Step::Announce => {
                    if self.events.len() == QUEUE || !pay(b) {
                        return false;
                    }
                    self.events.push_back(Event::Resolved { token: a.token() });
                    a.step = Step::Await;
                    return false;
                }
                Step::Await => return false,
                Step::Lookup => match self.cache.find(a.search.as_mut().unwrap(), b) {
                    Ok(Some(id)) => a.step = Step::Inspect(id),
                    Ok(None) => a.step = Step::Admit,
                    Err(_) => return false,
                },
                Step::Inspect(id) => {
                    if !pay(b) {
                        return false;
                    }
                    let Some(r) = self.cache.get(id) else {
                        a.step = Step::Lookup;
                        continue;
                    };
                    if r.failed {
                        let f = a.fetch.as_ref().unwrap();
                        if self.negative[usize::from(id.slot)] == Some((id, f.cell, f.area)) {
                            self.work.cache_hits += 1;
                            a.result = Some(id);
                            a.step = Step::Publish;
                        } else {
                            assert!(a.search.as_mut().unwrap().skip(id));
                            a.step = Step::Lookup;
                        }
                        continue;
                    }
                    let f = a.fetch.as_ref().unwrap();
                    let Ok((scaled, canvas, cells)) =
                        worker::fit(r.original.columns, r.original.rows, f.cell, f.area)
                    else {
                        a.error = Some(Failure::Status("image display-pixel admission exceeded"));
                        a.step = Step::Failed;
                        continue;
                    };
                    if self.rasters[usize::from(id.slot)] == Some((id, Raster { scaled, canvas })) {
                        a.cells = cells;
                        a.result = Some(id);
                        self.work.cache_hits += 1;
                        a.step = Step::Publish;
                        continue;
                    }
                    if a.encoded.is_none() && !r.encoded.is_empty() {
                        a.encoded = Some(r.encoded.clone())
                    }
                    assert!(a.search.as_mut().unwrap().skip(id));
                    a.step = Step::Lookup;
                }
                Step::Admit => {
                    if !pay(b) {
                        return false;
                    }
                    if self.cache.available(RECEIVING) {
                        a.step = Step::StartLoad;
                        continue;
                    }
                    a.step = Step::Evict;
                }
                Step::Evict => {
                    if self.events.len() == QUEUE
                        || b.remaining().queries == 0
                        || b.remaining().objects == 0
                    {
                        return false;
                    }
                    if let Some(id) = self.cache.evict(b) {
                        self.rasters[usize::from(id.slot)] = None;
                        self.negative[usize::from(id.slot)] = None;
                        self.events.push_back(Event::Evicted(id));
                        a.step = Step::Admit;
                        return false;
                    }
                    if !self.cache.eviction_pending() {
                        a.error = Some(Failure::Status(
                            "image cache admission unavailable (32 MiB)",
                        ));
                        a.step = Step::Failed;
                        continue;
                    }
                    return false;
                }
                Step::ReadLoad => {
                    if !pay(b) {
                        return false;
                    }
                    let reply = a.process.as_mut().unwrap().take_reply().unwrap();
                    a.metrics_reported = true;
                    let m = reply.metrics;
                    self.work.downloaded_body_bytes += m.downloaded_bytes;
                    self.work.encoded_bytes += m.encoded_bytes;
                    self.work.decoded_pixels += m.decoded_pixels;
                    self.work.scaled_pixels += m.scaled_pixels;
                    self.work.cpu_us += m.cpu_us;
                    self.work.load_cpu_us += m.cpu_us;
                    self.work.declared_pixels += m.declared_pixels;
                    self.work.peak_worker_rss_kib =
                        self.work.peak_worker_rss_kib.max(m.peak_rss_kib);
                    a.reply = Some(reply);
                    a.step = Step::ReapLoad;
                }
                Step::ReapLoad => {
                    if !self.reaped(a, b) {
                        return false;
                    }
                    a.step = if a.error.is_some() {
                        Step::Failed
                    } else {
                        Step::Insert
                    }
                }
                Step::Insert => {
                    if !pay(b) {
                        return false;
                    }
                    if a.record.is_none() {
                        let reply = a.reply.take().unwrap();
                        a.cells = reply.cells;
                        self.retire_bytes(reply.uri);
                        a.record = Some(Resource {
                            id: ResourceId {
                                slot: 0,
                                generation: 0,
                            },
                            uri: a.uri.as_ref().unwrap().clone(),
                            hash: a.hash,
                            encoded: Arc::new(reply.encoded),
                            rgba: Arc::new(reply.rgba),
                            original: reply.original,
                            canvas: reply.canvas,
                            cells: reply.cells,
                            metrics: reply.metrics,
                            failed: reply.error,
                            touched: 0,
                        });
                    }
                    let record = a.record.take().unwrap();
                    let original = record.original;
                    let failed = record.failed;
                    match self.cache.insert(record) {
                        Ok(id) => {
                            if !failed {
                                let f = a.fetch.as_ref().unwrap();
                                let (scaled, canvas, _) =
                                    worker::fit(original.columns, original.rows, f.cell, f.area)
                                        .expect("admitted worker geometry");
                                self.rasters[usize::from(id.slot)] =
                                    Some((id, Raster { scaled, canvas }));
                            } else {
                                let f = a.fetch.as_ref().unwrap();
                                self.negative[usize::from(id.slot)] = Some((id, f.cell, f.area));
                            }
                            a.result = Some(id);
                            a.step = Step::Publish
                        }
                        Err(record) => {
                            a.record = Some(record);
                            a.error = Some(Failure::Status("image cache admission changed"));
                            a.step = Step::Failed
                        }
                    }
                }
                Step::Publish => {
                    if self.events.len() == QUEUE || !pay(b) {
                        return false;
                    }
                    let id = a.result.unwrap();
                    let Some(r) = self.cache.get(id) else {
                        a.error =
                            Some(Failure::Status("image resource retired before publication"));
                        a.step = Step::Failed;
                        continue;
                    };
                    let lease = Lease {
                        id,
                        uri: r.uri.clone(),
                        rgba: r.rgba.clone(),
                        original: r.original,
                        canvas: r.canvas,
                        cells: if r.failed {
                            Geometry::default()
                        } else {
                            a.cells
                        },
                        failed: r.failed,
                        metrics: r.metrics,
                    };
                    self.cache.touch(id);
                    self.events.push_back(Event::Ready {
                        token: a.token(),
                        resource: lease,
                    });
                    a.step = Step::Done;
                }
                Step::Failed => {
                    if a.process.is_some() && !self.reaped(a, b) {
                        return false;
                    }
                    if self.events.len() == QUEUE || !pay(b) {
                        return false;
                    }
                    self.events.push_back(Event::Failed {
                        token: a.token(),
                        error: a.error.take().unwrap_or(Failure::Status("image cancelled")),
                    });
                    a.step = Step::Done
                }
                Step::Cancel => {
                    if a.process.is_some() && !self.reaped(a, b) {
                        return false;
                    }
                    if a.interrupted && !self.closing && self.events.len() == QUEUE {
                        return false;
                    }
                    if !pay(b) {
                        return false;
                    }
                    if a.interrupted && !self.closing {
                        self.events
                            .push_back(Event::Interrupted { token: a.token() });
                    }
                    a.step = Step::Done;
                    return true;
                }
            }
        }
    }
    pub fn capacity(&self) -> ResourceCapacity {
        let c = self.cache.capacity();
        let mut out = ResourceCapacity {
            retiring_source_descriptors: self.retiring.len(),
            queued_retirement_reservations: self.queued() * 2,
            source_descriptor_limit: RETIRING,
            input_current_charged: self.input_current,
            ready_uri_charged: self.ready_uri,
            input_retiring_charged: self.input_retiring,
            cache_current: c.current,
            cache_pinned_subset: c.pinned,
            cache_retiring: c.retiring,
            descriptor_bound: c.descriptor + std::mem::size_of::<Self>()
                - std::mem::size_of::<Cache>()
                + self.queue.capacity() * std::mem::size_of::<Fetch>()
                + self.events.capacity() * std::mem::size_of::<Event>()
                + self.retiring.capacity() * std::mem::size_of::<Retiring>(),
            ..Default::default()
        };
        if let Some(a) = &self.active {
            out.descriptor_bound += std::mem::size_of::<Active>();
            if let Some(p) = &a.process {
                out.worker_response = p.capacity_bytes();
                out.worker_request_pins = p.request_capacity_bytes()
            }
            if let Some(uri) = &a.uri {
                out.transient += cache::cost(uri)
            }
            if let Some(reply) = &a.reply {
                out.transient +=
                    cache::cost(&reply.uri) + cache::cost(&reply.encoded) + cache::cost(&reply.rgba)
            }
            if let Some(r) = &a.record {
                out.transient += r.cost()
            }
        }
        for r in &self.retiring {
            if !r.input {
                out.transient += cache::cost(&r.bytes)
            }
        }
        out
    }
}
