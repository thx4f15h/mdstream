//! Linked pending delimiter summaries. Failure splices in O(1); a summary
//! is consumed once and its slot is reused without copying growing labels.
use std::num::NonZeroUsize;
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct List {
    pub(super) head: Option<NonZeroUsize>,
    pub(super) tail: Option<NonZeroUsize>,
}
impl std::fmt::Debug for List {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("List")
            .field("pending", &self.head.is_some())
            .finish()
    }
}
#[derive(Clone, Copy, Debug)]
struct Node<T> {
    value: Option<T>,
    next: Option<NonZeroUsize>,
}
type LineCheckpoint<T> = (
    usize,
    Option<NonZeroUsize>,
    usize,
    crate::rewind::UndoLog<(usize, Node<T>)>,
);
#[derive(Debug)]
pub(super) struct Arena<T> {
    nodes: Vec<Node<T>>,
    free: Option<NonZeroUsize>,
    live: usize,
    pub(super) steps: u64,
    allocations: u64,
    checkpoint: Option<LineCheckpoint<T>>,
    undo_peak: crate::rewind::UndoStats,
}
impl<T> Default for Arena<T> {
    fn default() -> Self {
        Self {
            nodes: Vec::new(),
            free: None,
            live: 0,
            steps: 0,
            allocations: 0,
            checkpoint: None,
            undo_peak: crate::rewind::UndoStats::default(),
        }
    }
}
impl<T: Copy> Arena<T> {
    pub(super) fn undo_stats(&self) -> crate::rewind::UndoStats {
        self.undo_peak.max(
            self.checkpoint
                .as_ref()
                .map_or(crate::rewind::UndoStats::default(), |c| c.3.stats()),
        )
    }

    fn save(&mut self, id: usize) {
        if let Some((len, _, _, undo)) = &mut self.checkpoint
            && id < *len
        {
            undo.push((id, self.nodes[id]));
        }
    }
    pub(super) fn begin(&mut self) {
        assert!(self.checkpoint.is_none());
        self.checkpoint = Some((
            self.nodes.len(),
            self.free,
            self.live,
            crate::rewind::UndoLog::default(),
        ));
    }
    pub(super) fn commit(&mut self) -> u64 {
        self.checkpoint.take().map_or(0, |(_, _, _, undo)| {
            self.undo_peak = self.undo_peak.max(undo.stats());
            undo.len() as u64
        })
    }
    pub(super) fn rollback(&mut self) -> u64 {
        let (len, free, live, mut undo) = self.checkpoint.take().unwrap();
        self.undo_peak = self.undo_peak.max(undo.stats());
        let count = undo.len() as u64;
        while let Some((id, node)) = undo.pop() {
            self.nodes[id] = node;
        }
        self.nodes.truncate(len);
        self.free = free;
        self.live = live;
        count
    }

    #[cfg(test)]
    pub(super) fn snapshot(&self, mut list: List) -> Vec<T> {
        let mut values = Vec::new();
        while let Some(id) = list.head {
            let n = &self.nodes[id.get() - 1];
            values.push(n.value.unwrap());
            list.head = n.next;
        }
        values
    }
    #[cfg(test)]
    pub(super) fn value(&self, id: Option<NonZeroUsize>) -> Option<T> {
        id.and_then(|id| self.nodes[id.get() - 1].value)
    }
    pub(super) fn len(&self) -> usize {
        self.live
    }
    #[cfg(test)]
    pub(super) fn capacity(&self) -> usize {
        self.nodes.capacity()
    }
    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.live == 0
    }
    pub(super) fn stats(&self) -> crate::ArenaStats {
        crate::ArenaStats {
            nodes: self.live,
            capacity_bytes: self.nodes.capacity() * std::mem::size_of::<Node<T>>(),
            allocation_calls: self.allocations,
        }
    }
    pub(super) fn push(&mut self, list: &mut List, value: T) {
        self.steps += 1;
        let id = if let Some(id) = self.free {
            self.save(id.get() - 1);
            self.free = self.nodes[id.get() - 1].next;
            self.nodes[id.get() - 1] = Node {
                value: Some(value),
                next: None,
            };
            id
        } else {
            if self.nodes.len() == self.nodes.capacity() {
                self.allocations += 1;
            }
            self.nodes.push(Node {
                value: Some(value),
                next: None,
            });
            NonZeroUsize::new(self.nodes.len()).unwrap()
        };
        self.live += 1;
        let mut singleton = List {
            head: Some(id),
            tail: Some(id),
        };
        self.splice(list, &mut singleton);
    }
    pub(super) fn splice(&mut self, destination: &mut List, source: &mut List) {
        self.steps += 1;
        if let Some(head) = source.head {
            if let Some(tail) = destination.tail {
                self.save(tail.get() - 1);
                self.nodes[tail.get() - 1].next = Some(head);
            } else {
                destination.head = Some(head);
            }
            destination.tail = source.tail;
        }
        *source = List::default();
    }
    pub(super) fn pop(&mut self, list: &mut List) -> Option<T> {
        let id = list.head?;
        self.steps += 1;
        self.save(id.get() - 1);
        let n = &mut self.nodes[id.get() - 1];
        list.head = n.next;
        if list.head.is_none() {
            list.tail = None;
        }
        let value = n.value.take().unwrap();
        n.next = self.free;
        self.free = Some(id);
        self.live -= 1;
        Some(value)
    }
    pub(super) fn split_after(&mut self, list: &mut List, tail: Option<NonZeroUsize>) -> List {
        self.steps += 1;
        let Some(tail) = tail else {
            return std::mem::take(list);
        };
        self.save(tail.get() - 1);
        let head = self.nodes[tail.get() - 1].next.take();
        let suffix = List {
            head,
            tail: head.and(list.tail),
        };
        list.tail = Some(tail);
        suffix
    }
}

#[cfg(test)]
mod checkpoint_tests {
    use super::*;
    #[test]
    fn rollback_restores_freed_and_reused_nodes_before_old_roots() {
        let mut arena = Arena::default();
        let mut old = List::default();
        for i in 0..4096 {
            arena.push(&mut old, i);
        }
        let root = old;
        let capacity = arena.capacity();
        let before = arena.steps;
        arena.begin();
        assert_eq!(arena.checkpoint.as_ref().unwrap().3.len(), 0);
        for i in 0..2048 {
            assert_eq!(arena.pop(&mut old), Some(i));
        }
        let mut fresh = List::default();
        for i in 0..2048 {
            arena.push(&mut fresh, 10_000 + i);
        }
        arena.splice(&mut old, &mut fresh);
        let head = old.head;
        let _ = arena.split_after(&mut old, head);
        assert_eq!(arena.capacity(), capacity);
        let paid = arena.rollback();
        old = root;
        assert_eq!(arena.snapshot(old), (0..4096).collect::<Vec<_>>());
        assert_eq!(arena.len(), 4096);
        assert!(paid <= arena.steps - before);
        for i in 0..4096 {
            assert_eq!(arena.pop(&mut old), Some(i));
        }
        assert!(arena.is_empty());
    }
}
