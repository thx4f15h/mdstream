//! Append-only indexed chunks. Growth allocates a new uninitialized chunk and
//! never reallocates or copies old elements, including during budgeted adoption.
//! A fixed directory and bit arithmetic preserve O(1) random element access.
use std::ops::{Index, IndexMut};

pub(crate) struct AppendStore<T> {
    chunks: [Option<Vec<T>>; usize::BITS as usize],
    len: usize,
    capacity: usize,
    allocations: usize,
}
impl<T> Default for AppendStore<T> {
    fn default() -> Self {
        Self {
            chunks: std::array::from_fn(|_| None),
            len: 0,
            capacity: 0,
            allocations: 0,
        }
    }
}
impl<T> AppendStore<T> {
    fn location(index: usize) -> (usize, usize) {
        let one = index.checked_add(1).expect("append index fits usize");
        let chunk = one.ilog2() as usize;
        (chunk, one - (1usize << chunk))
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn capacity(&self) -> usize {
        self.capacity
    }
    pub fn allocations(&self) -> usize {
        self.allocations
    }
    pub fn push(&mut self, item: T) {
        let (chunk, offset) = Self::location(self.len);
        if self.chunks[chunk].is_none() {
            let data = Vec::with_capacity(1usize << chunk);
            self.capacity += data.capacity();
            self.allocations += 1;
            self.chunks[chunk] = Some(data);
        }
        let data = self.chunks[chunk].as_mut().unwrap();
        debug_assert_eq!(data.len(), offset);
        debug_assert!(data.len() < data.capacity());
        data.push(item);
        self.len += 1;
    }
    pub fn get(&self, index: usize) -> Option<&T> {
        if index >= self.len {
            return None;
        }
        let (chunk, offset) = Self::location(index);
        self.chunks[chunk].as_ref().unwrap().get(offset)
    }
    fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        if index >= self.len {
            return None;
        }
        let (chunk, offset) = Self::location(index);
        self.chunks[chunk].as_mut().unwrap().get_mut(offset)
    }
    pub fn last(&self) -> Option<&T> {
        self.get(self.len.checked_sub(1)?)
    }
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.chunks
            .iter()
            .filter_map(Option::as_ref)
            .flat_map(|chunk| chunk.iter())
    }
    pub fn partition_point(&self, mut predicate: impl FnMut(&T) -> bool) -> usize {
        let (mut lo, mut hi) = (0, self.len);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if predicate(&self[mid]) {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo
    }
    pub fn binary_search(&self, value: &T) -> Result<usize, usize>
    where
        T: Ord,
    {
        let at = self.partition_point(|item| item < value);
        if self.get(at).is_some_and(|item| item == value) {
            Ok(at)
        } else {
            Err(at)
        }
    }
}
impl<T> Index<usize> for AppendStore<T> {
    type Output = T;
    fn index(&self, index: usize) -> &T {
        self.get(index).expect("valid append index")
    }
}
impl<T> IndexMut<usize> for AppendStore<T> {
    fn index_mut(&mut self, index: usize) -> &mut T {
        self.get_mut(index).expect("valid append index")
    }
}
impl<T> IntoIterator for AppendStore<T> {
    type Item = T;
    type IntoIter = std::iter::Flatten<
        std::iter::Flatten<std::array::IntoIter<Option<Vec<T>>, { usize::BITS as usize }>>,
    >;
    fn into_iter(self) -> Self::IntoIter {
        self.chunks.into_iter().flatten().flatten()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn growth_never_moves_old_nonclone_elements_and_keeps_constant_time_addresses() {
        struct Item(usize);
        let mut items = AppendStore::default();
        let mut samples = Vec::new();
        for i in 0..100_000 {
            items.push(Item(i));
            if i.is_power_of_two() || i == 0 {
                samples.push((i, &items[i] as *const Item));
                for &(at, address) in &samples {
                    assert_eq!(&items[at] as *const Item, address);
                    assert_eq!(items[at].0, at);
                }
            }
        }
        assert_eq!(items.allocations(), 17);
        assert_eq!(items.capacity(), (1 << 17) - 1);
        assert_eq!(items.iter().map(|v| v.0).sum::<usize>(), (0..100_000).sum());
        assert_eq!(items.partition_point(|v| v.0 < 65536), 65536);
    }
}
