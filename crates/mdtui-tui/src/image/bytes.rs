//! Image byte pages reuse the renderer's nonrelocating directory. No append
//! reallocates an existing pixel/encoded payload. Retirement frees one page.
use crate::render::append_store::AppendStore;
use mdtui3::{Budget, WorkLimit};
const PAGE: usize = 4096;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ByteCapacity {
    pub bytes: usize,
    pub directory: usize,
    pub pages: usize,
}
#[derive(Default)]
pub struct Bytes {
    pages: AppendStore<Vec<u8>>,
    len: usize,
    capacity: usize,
}
impl std::fmt::Debug for Bytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bytes")
            .field("len", &self.len)
            .field("capacity", &self.capacity())
            .finish()
    }
}
impl Bytes {
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn byte(&self, at: usize) -> u8 {
        assert!(at < self.len);
        self.pages[at / PAGE][at % PAGE]
    }
    pub fn slice(&self, at: usize, max: usize) -> &[u8] {
        if at == self.len {
            return &[];
        }
        let page = &self.pages[at / PAGE];
        let offset = at % PAGE;
        &page[offset..(offset + max).min(page.len())]
    }
    pub fn append(&mut self, mut bytes: &[u8]) {
        while !bytes.is_empty() {
            if self.len.is_multiple_of(PAGE) {
                let page = Vec::with_capacity(PAGE);
                self.capacity += page.capacity();
                self.pages.push(page)
            }
            let page = &mut self.pages[self.len / PAGE];
            let n = (PAGE - page.len()).min(bytes.len());
            page.extend_from_slice(&bytes[..n]);
            self.len += n;
            bytes = &bytes[n..];
        }
    }
    pub fn capacity(&self) -> ByteCapacity {
        ByteCapacity {
            bytes: self.capacity,
            directory: self.pages.capacity() * std::mem::size_of::<Vec<u8>>(),
            pages: self.pages.len(),
        }
    }
    pub fn retire(&mut self, b: &mut Budget) -> bool {
        while !self.pages.len().eq(&0) {
            if !b.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            }) {
                return false;
            }
            let page = self.pages.pop().unwrap();
            self.len -= page.len();
            self.capacity -= page.capacity();
        }
        true
    }
}
