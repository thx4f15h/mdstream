//! One bounded current cache. Pins borrow its actual byte pages; retired pages
//! remain accounted until paid removal. No entry is allocated per revision.
use super::{
    CACHE_BYTES,
    bytes::Bytes,
    worker::{Geometry, WorkerMetrics},
};
use mdtui3::{Budget, WorkLimit};
use std::{
    collections::{HashSet, VecDeque},
    sync::Arc,
};
const ENTRIES: usize = 64;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ResourceId {
    pub slot: u8,
    pub generation: u64,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capacity {
    pub current: usize,
    pub pinned: usize,
    pub retiring: usize,
    pub descriptor: usize,
    pub entries: usize,
    pub retiring_buffers: usize,
}
impl Capacity {
    pub fn total(&self) -> usize {
        self.current + self.retiring + self.descriptor
    }
}
pub struct Resource {
    pub id: ResourceId,
    pub uri: Arc<Bytes>,
    pub hash: u64,
    pub encoded: Arc<Bytes>,
    pub rgba: Arc<Bytes>,
    pub original: Geometry,
    pub canvas: Geometry,
    pub cells: Geometry,
    pub metrics: WorkerMetrics,
    pub failed: bool,
    pub touched: u64,
}
impl Resource {
    pub fn cost(&self) -> usize {
        cost(&self.uri) + cost(&self.encoded) + cost(&self.rgba)
    }
}
pub(crate) fn cost(bytes: &Bytes) -> usize {
    let c = bytes.capacity();
    c.bytes + c.directory + std::mem::size_of::<Bytes>() + 2 * std::mem::size_of::<usize>()
}
pub struct Cache {
    entries: [Option<Resource>; ENTRIES],
    free: u64,
    buffers: HashSet<usize>,
    retiring: VecDeque<Arc<Bytes>>,
    clock: u64,
    next: u64,
    pub evictions: usize,
    current_bytes: usize,
    retiring_bytes: usize,
    evict_at: usize,
    oldest: Option<(ResourceId, u64)>,
}
impl Default for Cache {
    fn default() -> Self {
        Self {
            entries: std::array::from_fn(|_| None),
            free: u64::MAX,
            buffers: HashSet::with_capacity(ENTRIES * 6),
            retiring: VecDeque::with_capacity(ENTRIES * 3),
            clock: 0,
            next: 0,
            evictions: 0,
            current_bytes: 0,
            retiring_bytes: 0,
            evict_at: 0,
            oldest: None,
        }
    }
}
pub struct Match {
    uri: Arc<Bytes>,
    hash: u64,
    slot: usize,
    at: usize,
    selected: Option<ResourceId>,
}
impl Match {
    pub fn new(uri: Arc<Bytes>, hash: u64) -> Self {
        Self {
            uri,
            hash,
            slot: 0,
            at: 0,
            selected: None,
        }
    }
    /// Continue a URI search after rejecting a raster variant. The cursor is
    /// still generation-bound, including across any subsequent mutation.
    pub fn skip(&mut self, id: ResourceId) -> bool {
        if self.selected != Some(id) {
            return false;
        }
        self.slot += 1;
        self.at = 0;
        self.selected = None;
        true
    }
}
impl Cache {
    pub fn get(&self, id: ResourceId) -> Option<&Resource> {
        self.entries
            .get(usize::from(id.slot))?
            .as_ref()
            .filter(|r| r.id == id)
    }
    pub fn touch(&mut self, id: ResourceId) {
        self.clock += 1;
        if let Some(r) = self
            .entries
            .get_mut(usize::from(id.slot))
            .and_then(Option::as_mut)
            .filter(|r| r.id == id)
        {
            r.touched = self.clock
        }
    }
    pub fn find(
        &self,
        task: &mut Match,
        b: &mut Budget,
    ) -> Result<Option<ResourceId>, mdtui3::Completion> {
        while task.slot < ENTRIES {
            if task.selected.is_none() {
                if !b.charge(WorkLimit {
                    queries: 1,
                    objects: 1,
                    ..Default::default()
                }) {
                    return Err(mdtui3::Completion::BudgetExhausted);
                }
                let Some(r) = &self.entries[task.slot] else {
                    task.slot += 1;
                    continue;
                };
                if r.hash != task.hash || r.uri.len() != task.uri.len() {
                    task.slot += 1;
                    continue;
                }
                task.selected = Some(r.id);
                task.at = 0;
            }
            // Each resumed comparison/commit pays for and validates the exact
            // resource owner before inspecting bytes. Slot reuse cannot inherit
            // a comparison prefix belonging to an older generation.
            let count = if task.at < task.uri.len() { 2 } else { 0 };
            if !b.charge(WorkLimit {
                bytes: count,
                queries: 1,
                objects: 1,
                ..Default::default()
            }) {
                return Err(mdtui3::Completion::BudgetExhausted);
            }
            let Some(r) = self.get(task.selected.unwrap()) else {
                task.selected = None;
                task.at = 0;
                continue;
            };
            if r.hash != task.hash || r.uri.len() != task.uri.len() {
                task.selected = None;
                task.at = 0;
                continue;
            }
            if task.at == task.uri.len() {
                return Ok(Some(r.id));
            }
            if task.uri.byte(task.at) != r.uri.byte(task.at) {
                task.slot += 1;
                task.selected = None;
                task.at = 0
            } else {
                task.at += 1
            }
        }
        Ok(None)
    }
    pub fn capacity(&self) -> Capacity {
        let mut c = Capacity {
            descriptor: self.descriptor_bound(),
            ..Default::default()
        };
        for r in self.entries.iter().flatten() {
            c.entries += 1;
            for buf in [&r.uri, &r.encoded, &r.rgba] {
                let n = cost(buf);
                c.current += n;
                if Arc::strong_count(buf) > 1 {
                    c.pinned += n
                }
            }
        }
        for buf in &self.retiring {
            c.retiring += cost(buf);
            c.retiring_buffers += 1
        }
        c
    }
    fn retire_buffer(&mut self, bytes: Arc<Bytes>) {
        self.retiring_bytes += cost(&bytes);
        self.retiring.push_back(bytes)
    }
    pub fn retire(&mut self, b: &mut Budget) -> bool {
        while let Some(front) = self.retiring.front_mut() {
            let Some(bytes) = Arc::get_mut(front) else {
                return false;
            };
            let before = cost(bytes);
            let done = bytes.retire(b);
            self.retiring_bytes -= before - cost(bytes);
            if !done {
                return false;
            }
            if !b.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            }) {
                return false;
            }
            self.retiring_bytes -= cost(self.retiring.front().unwrap());
            self.buffers
                .remove(&(Arc::as_ptr(self.retiring.front().unwrap()) as usize));
            self.retiring.pop_front();
        }
        true
    }
    /// Returns the evicted resource identity; the protocol owner must enqueue
    /// its targeted delete before another upload can reuse terminal capacity.
    pub fn evict(&mut self, b: &mut Budget) -> Option<ResourceId> {
        while self.evict_at < ENTRIES {
            if !b.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            }) {
                return None;
            }
            let i = self.evict_at;
            self.evict_at += 1;
            if let Some(r) = &self.entries[i]
                && Arc::strong_count(&r.uri) == 1
                && Arc::strong_count(&r.encoded) == 1
                && Arc::strong_count(&r.rgba) == 1
                && self.oldest.is_none_or(|(_, t)| r.touched < t)
            {
                self.oldest = Some((r.id, r.touched))
            }
        }
        if !b.charge(WorkLimit {
            queries: 1,
            objects: 1,
            ..Default::default()
        }) {
            return None;
        }
        self.evict_at = 0;
        let (selected, touched) = self.oldest.take()?;
        if self.retiring.len() + 3 > ENTRIES * 3 {
            return None;
        }
        let slot = usize::from(selected.slot);
        let r = self.get(selected)?;
        if r.touched != touched {
            return None;
        }
        if [&r.uri, &r.encoded, &r.rgba]
            .iter()
            .any(|v| Arc::strong_count(v) > 1)
        {
            return None;
        }
        let r = self.entries[slot].take().unwrap();
        self.free |= 1u64 << slot;
        let id = r.id;
        self.current_bytes -= r.cost();
        self.retire_buffer(r.uri);
        self.retire_buffer(r.encoded);
        self.retire_buffer(r.rgba);
        self.evictions += 1;
        Some(id)
    }
    /// Container allocation is conservatively bounded; page allocations and
    /// their capacities are exact. The pointer set never exceeds 384 records.
    fn descriptor_bound(&self) -> usize {
        let buckets = (self.buffers.capacity().saturating_mul(8) / 7 + 1).next_power_of_two();
        std::mem::size_of::<Self>()
            + self.retiring.capacity() * std::mem::size_of::<Arc<Bytes>>()
            + buckets * (std::mem::size_of::<usize>() + 1)
            + 32
    }
    pub fn known_total(&self) -> usize {
        self.current_bytes + self.retiring_bytes + self.descriptor_bound()
    }
    pub fn available(&self, receiving: usize) -> bool {
        self.known_total().saturating_add(receiving) <= CACHE_BYTES && self.free != 0
    }
    pub fn eviction_pending(&self) -> bool {
        self.evict_at != 0 || self.oldest.is_some()
    }
    // Fixed 168-byte metadata/Arc descriptor. Rejection returns the original
    // owner without introducing a new heap allocation on the rejection path.
    #[allow(clippy::result_large_err)]
    pub fn insert(&mut self, mut r: Resource) -> Result<ResourceId, Resource> {
        if self.known_total().saturating_add(r.cost()) > CACHE_BYTES {
            return Err(r);
        }
        if self.free == 0 {
            return Err(r);
        }
        // Each resource owns its three distinct buffers. Variants keep an
        // independent URI/raster; only the source-bearing resource retains
        // encoded bytes. External leases may pin buffers, while internal
        // aliases are rejected rather than becoming impossible self-pins.
        let pointers = [
            Arc::as_ptr(&r.uri) as usize,
            Arc::as_ptr(&r.encoded) as usize,
            Arc::as_ptr(&r.rgba) as usize,
        ];
        if pointers[0] == pointers[1]
            || pointers[0] == pointers[2]
            || pointers[1] == pointers[2]
            || pointers.iter().any(|p| self.buffers.contains(p))
        {
            return Err(r);
        }
        let slot = self.free.trailing_zeros() as usize;
        self.next = self
            .next
            .checked_add(1)
            .expect("image cache generation exhausted");
        self.clock += 1;
        let id = ResourceId {
            slot: slot as u8,
            generation: self.next,
        };
        r.id = id;
        r.touched = self.clock;
        self.current_bytes += r.cost();
        self.free &= !(1u64 << slot);
        self.buffers.extend(pointers);
        self.entries[slot] = Some(r);
        Ok(id)
    }
    pub fn replace_canvas(
        &mut self,
        id: ResourceId,
        rgba: Arc<Bytes>,
        canvas: Geometry,
        cells: Geometry,
        metrics: WorkerMetrics,
    ) -> Result<(), Arc<Bytes>> {
        if self.known_total().saturating_add(cost(&rgba)) > CACHE_BYTES
            || self.retiring.len() == ENTRIES * 3
            || self.buffers.contains(&(Arc::as_ptr(&rgba) as usize))
        {
            return Err(rgba);
        }
        let Some(r) = self
            .entries
            .get_mut(usize::from(id.slot))
            .and_then(Option::as_mut)
            .filter(|r| r.id == id)
        else {
            return Err(rgba);
        };
        self.current_bytes = self.current_bytes - cost(&r.rgba) + cost(&rgba);
        self.buffers.insert(Arc::as_ptr(&rgba) as usize);
        let old = std::mem::replace(&mut r.rgba, rgba);
        r.canvas = canvas;
        r.cells = cells;
        r.metrics = metrics;
        self.retire_buffer(old);
        Ok(())
    }
}
pub fn hash(bytes: &Bytes) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for i in 0..bytes.len() {
        h = (h ^ u64::from(bytes.byte(i))).wrapping_mul(0x100000001b3)
    }
    h
}
