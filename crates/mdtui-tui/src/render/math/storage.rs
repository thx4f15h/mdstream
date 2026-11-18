//! Math query storage. A full page links a NEW bounded allocation; it never
//! resizes an old payload or an unbounded `Vec<Page>` directory.
use std::collections::BTreeMap;
use std::ops::{Index, IndexMut};

pub(super) const PAGE: usize = 64;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StorageWork {
    pub records: usize,
    pub pages: usize,
    pub page_allocations: usize,
    pub directory_operations: usize,
    pub capacity_bytes: usize,
    /// Directory key/value payload, excluding std BTree headers and slack.
    pub directory_payload_bytes: usize,
    /// Payload records copied by growth. Always zero: pages never grow.
    pub old_records_relocated: usize,
    pub max_page_bytes: usize,
}

#[derive(Debug)]
pub(super) struct Paged<T> {
    pages: BTreeMap<usize, Vec<T>>,
    len: usize,
    allocations: usize,
    directory: usize,
}
impl<T> Default for Paged<T> {
    fn default() -> Self {
        Self {
            pages: BTreeMap::new(),
            len: 0,
            allocations: 0,
            directory: 0,
        }
    }
}
impl<T> Paged<T> {
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn push(&mut self, value: T) -> usize {
        let id = self.len;
        let page = id / PAGE;
        if id.is_multiple_of(PAGE) {
            self.pages.insert(page, Vec::with_capacity(PAGE));
            self.allocations += 1;
            self.directory += 1;
        }
        let values = self.pages.get_mut(&page).expect("allocated tail page");
        debug_assert!(values.len() < values.capacity());
        debug_assert_eq!(values.capacity(), PAGE);
        values.push(value);
        self.len += 1;
        id
    }
    pub fn pop(&mut self) -> Option<T> {
        let id = self.len.checked_sub(1)?;
        let page = id / PAGE;
        let values = self.pages.get_mut(&page).unwrap();
        let value = values.pop();
        self.len -= 1;
        if values.is_empty() {
            self.pages.remove(&page);
            if self.pages.is_empty() {
                // std BTreeMap can retain an empty root allocation. Its last
                // fixed node also retires inside this already paid page step.
                self.pages = BTreeMap::new();
            }
            self.directory += 1;
        }
        value
    }
    pub fn last(&self) -> Option<&T> {
        self.len.checked_sub(1).map(|id| &self[id])
    }
    pub fn last_mut(&mut self) -> Option<&mut T> {
        let id = self.len.checked_sub(1)?;
        Some(&mut self[id])
    }
    pub fn stats(&self) -> StorageWork {
        StorageWork {
            records: self.len,
            pages: self.pages.len(),
            page_allocations: self.allocations,
            directory_operations: self.directory,
            capacity_bytes: self.pages.len() * PAGE * std::mem::size_of::<T>(),
            directory_payload_bytes: self.pages.len()
                * (std::mem::size_of::<usize>() + std::mem::size_of::<Vec<T>>()),
            old_records_relocated: 0,
            max_page_bytes: PAGE * std::mem::size_of::<T>(),
        }
    }
}

/// One Copy key/value removal, including the final empty fixed root node.
pub(super) fn retire_map<K: Ord + Copy, V: Copy>(map: &mut BTreeMap<K, V>) -> bool {
    let removed = map.pop_last().is_some();
    if map.is_empty() {
        *map = BTreeMap::new();
    }
    removed
}
impl<T> Index<usize> for Paged<T> {
    type Output = T;
    fn index(&self, id: usize) -> &T {
        assert!(id < self.len);
        &self.pages[&(id / PAGE)][id % PAGE]
    }
}
impl<T> IndexMut<usize> for Paged<T> {
    fn index_mut(&mut self, id: usize) -> &mut T {
        assert!(id < self.len);
        &mut self.pages.get_mut(&(id / PAGE)).unwrap()[id % PAGE]
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct List {
    pub first: Option<usize>,
    pub last: Option<usize>,
    pub len: usize,
}
#[derive(Clone, Copy, Debug)]
pub(super) struct Edge {
    pub child: usize,
    pub next: Option<usize>,
    pub previous: Option<usize>,
    pub attached: bool,
}
impl List {
    pub fn push(&mut self, edges: &mut Paged<Edge>, child: usize) {
        let id = edges.push(Edge {
            child,
            next: None,
            previous: self.last,
            attached: true,
        });
        if let Some(last) = self.last {
            edges[last].next = Some(id)
        } else {
            self.first = Some(id)
        }
        self.last = Some(id);
        self.len += 1;
    }
    pub fn pop(&mut self, edges: &mut Paged<Edge>) -> Option<usize> {
        let id = self.last?;
        let edge = edges[id];
        edges[id].attached = false;
        self.last = edge.previous;
        if let Some(last) = self.last {
            edges[last].next = None
        } else {
            self.first = None
        }
        self.len -= 1;
        Some(edge.child)
    }
    pub fn nth_small(&self, edges: &Paged<Edge>, at: usize) -> Option<usize> {
        assert!(at < 3);
        let mut cursor = self.first;
        for _ in 0..at {
            cursor = edges[cursor?].next
        }
        cursor.map(|id| edges[id].child)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_page_boundary_preserves_old_payload_addresses_and_paid_retirement() {
        let mut pages = Paged::default();
        assert_eq!(pages.stats().capacity_bytes, 0);
        let mut pointers = Vec::new();
        for i in 0..100000 {
            pages.push(i);
            if i % PAGE == 0 {
                pointers.push((&pages[i]) as *const usize)
            }
            if i % PAGE == 0 {
                for (page, pointer) in pointers.iter().enumerate() {
                    assert!(std::ptr::eq(&pages[page * PAGE], *pointer))
                }
            }
        }
        assert_eq!(pages.stats().old_records_relocated, 0);
        assert_eq!(pages.stats().pages, 100000usize.div_ceil(PAGE));
        for i in (0..100000).rev() {
            assert_eq!(pages.pop(), Some(i));
        }
        assert_eq!(pages.stats().pages, 0);
    }
}
