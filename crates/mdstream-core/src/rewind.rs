//! A single physical-line checkpoint. Mutations retain only overwritten cells;
//! no growing stack or ancestor chain is copied when the checkpoint begins.
use std::collections::{BTreeMap, VecDeque};
use std::ops::{Deref, Index, IndexMut, RangeFrom};

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct UndoStats {
    pub entries: usize,
    pub bytes: usize,
}
impl UndoStats {
    pub fn max(self, other: Self) -> Self {
        Self {
            entries: self.entries.max(other.entries),
            bytes: self.bytes.max(other.bytes),
        }
    }
    pub fn plus(self, other: Self) -> Self {
        Self {
            entries: self.entries + other.entries,
            bytes: self.bytes + other.bytes,
        }
    }
}
/// Records actual allocation peaks, including moved Clear buffers. No scan is
/// needed to observe an active log or to keep its peak after commit/rollback.
#[derive(Debug)]
pub(crate) struct UndoLog<T> {
    items: Vec<(T, usize)>,
    retained: usize,
    peak: UndoStats,
}
impl<T> Default for UndoLog<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            retained: 0,
            peak: UndoStats::default(),
        }
    }
}
impl<T> UndoLog<T> {
    pub fn push(&mut self, item: T) {
        self.push_retained(item, 0)
    }
    pub fn push_retained(&mut self, item: T, bytes: usize) {
        self.items.push((item, bytes));
        self.retained += bytes;
        self.peak = self.peak.max(UndoStats {
            entries: self.items.len(),
            bytes: self.items.capacity() * std::mem::size_of::<(T, usize)>() + self.retained,
        });
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub(crate) fn get(&self, index: usize) -> Option<&T> {
        self.items.get(index).map(|(item, _)| item)
    }
    pub fn last(&self) -> Option<&T> {
        self.items.last().map(|(item, _)| item)
    }
    pub fn pop(&mut self) -> Option<T> {
        let (item, bytes) = self.items.pop()?;
        self.retained -= bytes;
        Some(item)
    }
    pub fn stats(&self) -> UndoStats {
        self.peak
    }
}

enum VecUndo<T> {
    Push,
    Pop(T),
    Set(usize, T),
    Clear(Vec<T>),
}
pub(crate) struct Vector<T: Copy> {
    data: Vec<T>,
    undo: Option<UndoLog<VecUndo<T>>>,
    peak: UndoStats,
}
impl<T: Copy> Default for Vector<T> {
    fn default() -> Self {
        Self {
            data: Vec::new(),
            undo: None,
            peak: UndoStats::default(),
        }
    }
}
impl<T: Copy + std::fmt::Debug> std::fmt::Debug for Vector<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.data.fmt(f)
    }
}
impl<T: Copy> Deref for Vector<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        &self.data
    }
}
impl<T: Copy, I: std::slice::SliceIndex<[T]>> Index<I> for Vector<T> {
    type Output = I::Output;
    fn index(&self, i: I) -> &Self::Output {
        &self.data[i]
    }
}
impl<T: Copy> IndexMut<usize> for Vector<T> {
    fn index_mut(&mut self, i: usize) -> &mut T {
        if let Some(u) = &mut self.undo {
            // Consecutive writes to one collector endpoint need only the
            // first prior value. Pops/clears/other indexes remain separate.
            if !matches!(u.last(),Some(VecUndo::Set(old,_)) if *old==i) {
                u.push(VecUndo::Set(i, self.data[i]));
            }
        }
        &mut self.data[i]
    }
}
impl<T: Copy> IndexMut<RangeFrom<usize>> for Vector<T> {
    fn index_mut(&mut self, i: RangeFrom<usize>) -> &mut [T] {
        if let Some(u) = &mut self.undo {
            for at in i.start..self.data.len() {
                u.push(VecUndo::Set(at, self.data[at]));
            }
        }
        &mut self.data[i]
    }
}
impl<T: Copy> Vector<T> {
    pub fn undo_stats(&self) -> UndoStats {
        self.peak.max(
            self.undo
                .as_ref()
                .map_or(UndoStats::default(), UndoLog::stats),
        )
    }
    pub fn capacity(&self) -> usize {
        self.data.capacity()
    }
    pub fn push(&mut self, v: T) {
        if let Some(u) = &mut self.undo {
            u.push(VecUndo::Push);
        }
        self.data.push(v);
    }
    pub fn pop(&mut self) -> Option<T> {
        let v = self.data.pop()?;
        if let Some(u) = &mut self.undo {
            u.push(VecUndo::Pop(v));
        }
        Some(v)
    }
    pub fn clear(&mut self) {
        if let Some(u) = &mut self.undo {
            if !self.data.is_empty() {
                let bytes = self.data.capacity() * std::mem::size_of::<T>();
                u.push_retained(VecUndo::Clear(std::mem::take(&mut self.data)), bytes);
            }
        } else {
            self.data.clear();
        }
    }
    pub fn truncate(&mut self, len: usize) {
        if self.undo.is_some() {
            while self.data.len() > len {
                self.pop();
            }
        } else {
            self.data.truncate(len);
        }
    }
    pub fn last_mut(&mut self) -> Option<&mut T> {
        let i = self.data.len().checked_sub(1)?;
        Some(&mut self[i])
    }
    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, T> {
        if let Some(u) = &mut self.undo {
            for (i, &v) in self.data.iter().enumerate() {
                u.push(VecUndo::Set(i, v));
            }
        }
        self.data.iter_mut()
    }
    pub fn remove(&mut self, i: usize) -> T {
        assert!(self.undo.is_none());
        self.data.remove(i)
    }
    pub fn drain(&mut self, r: std::ops::RangeTo<usize>) -> std::vec::Drain<'_, T> {
        assert!(self.undo.is_none());
        self.data.drain(r)
    }
    pub fn begin(&mut self) {
        assert!(self.undo.is_none());
        self.undo = Some(UndoLog::default());
    }
    pub fn commit(&mut self) -> u64 {
        self.undo.take().map_or(0, |u| {
            self.peak = self.peak.max(u.stats());
            u.len() as u64
        })
    }
    pub fn rollback(&mut self) -> u64 {
        let mut u = self.undo.take().unwrap();
        self.peak = self.peak.max(u.stats());
        let count = u.len() as u64;
        while let Some(op) = u.pop() {
            match op {
                VecUndo::Push => {
                    self.data.pop();
                }
                VecUndo::Pop(v) => self.data.push(v),
                VecUndo::Set(i, v) => self.data[i] = v,
                VecUndo::Clear(v) => self.data = v,
            }
        }
        count
    }
}
impl<T: Copy> IntoIterator for Vector<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;
    fn into_iter(self) -> Self::IntoIter {
        assert!(self.undo.is_none());
        self.data.into_iter()
    }
}

enum QueueUndo<T> {
    Push,
    Pop(T),
    Clear(VecDeque<T>),
}
pub(crate) struct Queue<T: Copy> {
    data: VecDeque<T>,
    undo: Option<UndoLog<QueueUndo<T>>>,
    peak: UndoStats,
}
impl<T: Copy> Default for Queue<T> {
    fn default() -> Self {
        Self {
            data: VecDeque::new(),
            undo: None,
            peak: UndoStats::default(),
        }
    }
}
impl<T: Copy + std::fmt::Debug> std::fmt::Debug for Queue<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.data.fmt(f)
    }
}
impl<T: Copy> Queue<T> {
    pub fn undo_stats(&self) -> UndoStats {
        self.peak.max(
            self.undo
                .as_ref()
                .map_or(UndoStats::default(), UndoLog::stats),
        )
    }
    pub fn len(&self) -> usize {
        self.data.len()
    }
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
    pub fn front(&self) -> Option<&T> {
        self.data.front()
    }
    pub fn push(&mut self, v: T) {
        self.push_back(v)
    }
    pub fn push_back(&mut self, v: T) {
        if let Some(u) = &mut self.undo {
            u.push(QueueUndo::Push);
        }
        self.data.push_back(v)
    }
    pub fn pop_front(&mut self) -> Option<T> {
        let v = self.data.pop_front()?;
        if let Some(u) = &mut self.undo {
            u.push(QueueUndo::Pop(v));
        }
        Some(v)
    }
    pub fn clear(&mut self) {
        if let Some(u) = &mut self.undo {
            if !self.data.is_empty() {
                let bytes = self.data.capacity() * std::mem::size_of::<T>();
                u.push_retained(QueueUndo::Clear(std::mem::take(&mut self.data)), bytes);
            }
        } else {
            self.data.clear();
        }
    }
    pub fn begin(&mut self) {
        assert!(self.undo.is_none());
        self.undo = Some(UndoLog::default());
    }
    pub fn commit(&mut self) -> u64 {
        self.undo.take().map_or(0, |u| {
            self.peak = self.peak.max(u.stats());
            u.len() as u64
        })
    }
    pub fn rollback(&mut self) -> u64 {
        let mut u = self.undo.take().unwrap();
        self.peak = self.peak.max(u.stats());
        let n = u.len() as u64;
        while let Some(op) = u.pop() {
            match op {
                QueueUndo::Push => {
                    self.data.pop_back();
                }
                QueueUndo::Pop(v) => self.data.push_front(v),
                QueueUndo::Clear(v) => self.data = v,
            }
        }
        n
    }
}

pub(crate) struct Map<K: Ord + Copy, V: Copy> {
    data: BTreeMap<K, V>,
    undo: Option<UndoLog<(K, Option<V>)>>,
    peak: UndoStats,
    peak_len: usize,
}
impl<K: Ord + Copy, V: Copy> Default for Map<K, V> {
    fn default() -> Self {
        Self {
            data: BTreeMap::new(),
            undo: None,
            peak: UndoStats::default(),
            peak_len: 0,
        }
    }
}
impl<K: Ord + Copy + std::fmt::Debug, V: Copy + std::fmt::Debug> std::fmt::Debug for Map<K, V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.data.fmt(f)
    }
}
impl<K: Ord + Copy, V: Copy> Deref for Map<K, V> {
    type Target = BTreeMap<K, V>;
    fn deref(&self) -> &Self::Target {
        &self.data
    }
}
impl<K: Ord + Copy, V: Copy> Index<&K> for Map<K, V> {
    type Output = V;
    fn index(&self, k: &K) -> &V {
        &self.data[k]
    }
}
impl<K: Ord + Copy, V: Copy> Map<K, V> {
    pub fn peak_entries(&self) -> usize {
        self.peak_len
    }
    pub fn undo_stats(&self) -> UndoStats {
        self.peak.max(
            self.undo
                .as_ref()
                .map_or(UndoStats::default(), UndoLog::stats),
        )
    }
    pub fn insert(&mut self, k: K, v: V) -> Option<V> {
        let old = self.data.insert(k, v);
        self.peak_len = self.peak_len.max(self.data.len());
        if let Some(u) = &mut self.undo {
            u.push((k, old));
        }
        old
    }
    pub fn get_mut(&mut self, k: &K) -> Option<&mut V> {
        if let Some(&v) = self.data.get(k)
            && let Some(u) = &mut self.undo
        {
            u.push((*k, Some(v)));
        }
        self.data.get_mut(k)
    }
    pub fn remove(&mut self, k: &K) -> Option<V> {
        let old = self.data.remove(k);
        if let (Some(u), Some(v)) = (&mut self.undo, old) {
            u.push((*k, Some(v)));
        }
        old
    }
    pub fn pop_first(&mut self) -> Option<(K, V)> {
        let (k, v) = self.data.pop_first()?;
        if let Some(u) = &mut self.undo {
            u.push((k, Some(v)));
        }
        Some((k, v))
    }
    pub fn clear(&mut self) {
        if self.undo.is_some() {
            while self.pop_first().is_some() {}
        } else {
            self.data.clear();
        }
    }
    pub fn begin(&mut self) {
        assert!(self.undo.is_none());
        self.undo = Some(UndoLog::default());
    }
    pub fn commit(&mut self) -> u64 {
        self.undo.take().map_or(0, |u| {
            self.peak = self.peak.max(u.stats());
            u.len() as u64
        })
    }
    pub fn rollback(&mut self) -> u64 {
        let mut u = self.undo.take().unwrap();
        self.peak = self.peak.max(u.stats());
        let n = u.len() as u64;
        while let Some((k, v)) = u.pop() {
            if let Some(v) = v {
                self.data.insert(k, v);
            } else {
                self.data.remove(&k);
            }
        }
        n
    }
}

impl<'a, T: Copy> IntoIterator for &'a Vector<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.data.iter()
    }
}
impl<T: Copy + PartialEq> PartialEq<Vec<T>> for Vector<T> {
    fn eq(&self, other: &Vec<T>) -> bool {
        self.data == *other
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_touched_cells_are_journaled_and_reverse_replay_restores_reuse() {
        let mut v = Vector::default();
        for n in 0..20_000 {
            v.push(n);
        }
        v.begin();
        assert_eq!(v.undo.as_ref().unwrap().len(), 0);
        v[19_999] = 7;
        v.pop();
        v.clear();
        v.push(42);
        assert_eq!(v.undo.as_ref().unwrap().len(), 4);
        assert_eq!(v.rollback(), 4);
        assert_eq!(v.len(), 20_000);
        assert_eq!(v[19_999], 19_999);
        v.begin();
        v.pop();
        assert_eq!(v.commit(), 1);
        assert_eq!(v.len(), 19_999);
        let mut q = Queue::default();
        for n in 0..20_000 {
            q.push_back(n);
        }
        q.begin();
        assert_eq!(q.pop_front(), Some(0));
        q.push_back(42);
        q.clear();
        q.push_back(9);
        assert_eq!(q.rollback(), 4);
        assert_eq!(q.len(), 20_000);
        assert_eq!(q.pop_front(), Some(0));
        let mut m = Map::default();
        for n in 0..20_000 {
            m.insert(n, n);
        }
        m.begin();
        m.insert(10, 8);
        m.remove(&11);
        *m.get_mut(&12).unwrap() = 7;
        m.insert(20_001, 0);
        assert_eq!(m.rollback(), 4);
        assert_eq!(m.len(), 20_000);
        assert_eq!(m[&10], 10);
        assert_eq!(m[&11], 11);
        assert_eq!(m[&12], 12);
    }
}
