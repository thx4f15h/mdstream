use std::collections::{BTreeMap, BTreeSet};

pub(super) const BYTE_PAGE: usize = 256;
const SLOTS: usize = 32;

/// No directory Vec grows by moving old pages. Payload allocations are fixed;
/// directory entries and allocator/RSS observations are reported separately.
#[derive(Default)]
pub(super) struct Bytes {
    pages: BTreeMap<usize, Box<[u8; BYTE_PAGE]>>,
    pub len: usize,
}
impl Bytes {
    pub fn push(&mut self, byte: u8) {
        let page = self
            .pages
            .entry(self.len / BYTE_PAGE)
            .or_insert_with(|| Box::new([0; BYTE_PAGE]));
        page[self.len % BYTE_PAGE] = byte;
        self.len += 1;
    }
    pub fn get(&self, at: usize) -> u8 {
        assert!(at < self.len);
        self.pages[&(at / BYTE_PAGE)][at % BYTE_PAGE]
    }
    pub fn retire_page(&mut self) -> bool {
        self.pages.pop_first().is_some()
    }
    pub fn capacity(&self) -> usize {
        self.pages.len() * BYTE_PAGE
    }
    #[cfg(test)]
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Id {
    pub slot: usize,
    pub generation: u64,
}
struct Slot<T> {
    generation: u64,
    value: Option<T>,
}
struct Page<T> {
    live: usize,
    slots: [Slot<T>; SLOTS],
}
pub(super) struct Arena<T> {
    pages: BTreeMap<usize, Box<Page<T>>>,
    holes: BTreeSet<usize>,
    next_page: usize,
    pub live: usize,
    tracked: Option<(super::runtime::Cx, usize)>,
}
impl<T> Default for Arena<T> {
    fn default() -> Self {
        Self {
            pages: BTreeMap::new(),
            holes: BTreeSet::new(),
            next_page: 0,
            live: 0,
            tracked: None,
        }
    }
}
impl<T> Arena<T> {
    pub fn tracked(cx: super::runtime::Cx, k: usize) -> Self {
        Self {
            tracked: Some((cx, k)),
            ..Default::default()
        }
    }
    fn sync(&self) {
        if let Some((cx, k)) = &self.tracked {
            cx.storage(
                *k,
                self.capacity()
                    + self.holes.len() * std::mem::size_of::<usize>()
                    + self.pages.len() * std::mem::size_of::<(usize, Box<Page<T>>)>(),
                self.directory_entries(),
                self.live,
            );
        }
    }

    pub fn insert(&mut self, value: T) -> Id {
        if self.holes.is_empty() {
            let page = self.next_page;
            self.next_page = page.checked_add(1).expect("slug page identity exhausted");
            self.pages.insert(
                page,
                Box::new(Page {
                    live: 0,
                    slots: std::array::from_fn(|_| Slot {
                        generation: 0,
                        value: None,
                    }),
                }),
            );
            for slot in 0..SLOTS {
                self.holes.insert(page.checked_mul(SLOTS).unwrap() + slot);
            }
        }
        let slot = self.holes.pop_first().unwrap();
        let page = self.pages.get_mut(&(slot / SLOTS)).unwrap();
        let cell = &mut page.slots[slot % SLOTS];
        cell.generation = cell
            .generation
            .checked_add(1)
            .expect("slug slot generation exhausted");
        cell.value = Some(value);
        page.live += 1;
        self.live += 1;
        let generation = cell.generation;
        self.sync();
        Id { slot, generation }
    }
    pub fn get(&self, id: Id) -> &T {
        let slot = &self.pages[&(id.slot / SLOTS)].slots[id.slot % SLOTS];
        assert_eq!(slot.generation, id.generation);
        slot.value.as_ref().unwrap()
    }
    pub fn get_mut(&mut self, id: Id) -> &mut T {
        let slot = &mut self.pages.get_mut(&(id.slot / SLOTS)).unwrap().slots[id.slot % SLOTS];
        assert_eq!(slot.generation, id.generation);
        slot.value.as_mut().unwrap()
    }
    pub fn contains(&self, id: Id) -> bool {
        self.pages.get(&(id.slot / SLOTS)).is_some_and(|page| {
            let cell = &page.slots[id.slot % SLOTS];
            cell.generation == id.generation && cell.value.is_some()
        })
    }
    pub fn remove(&mut self, id: Id) -> T {
        let page_key = id.slot / SLOTS;
        let page = self.pages.get_mut(&page_key).unwrap();
        let cell = &mut page.slots[id.slot % SLOTS];
        assert_eq!(cell.generation, id.generation);
        let value = cell.value.take().unwrap();
        page.live -= 1;
        self.live -= 1;
        self.holes.insert(id.slot);
        if page.live == 0 {
            // At most one fixed page of empty slots, never a recursive graph.
            self.pages.remove(&page_key);
            for slot in 0..SLOTS {
                self.holes.remove(&(page_key * SLOTS + slot));
            }
        }
        self.sync();
        value
    }
    pub fn capacity(&self) -> usize {
        self.pages.len() * std::mem::size_of::<Page<T>>()
    }
    pub fn directory_entries(&self) -> usize {
        self.pages.len() + self.holes.len()
    }
}
