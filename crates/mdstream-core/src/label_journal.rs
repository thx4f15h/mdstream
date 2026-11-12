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
#[derive(Debug)]
struct Node<T> {
    value: Option<T>,
    next: Option<NonZeroUsize>,
}
#[derive(Debug)]
pub(super) struct Arena<T> {
    nodes: Vec<Node<T>>,
    free: Option<NonZeroUsize>,
    live: usize,
    pub(super) steps: u64,
    allocations: u64,
}
impl<T> Default for Arena<T> {
    fn default() -> Self {
        Self {
            nodes: Vec::new(),
            free: None,
            live: 0,
            steps: 0,
            allocations: 0,
        }
    }
}
impl<T: Copy> Arena<T> {
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
        let head = self.nodes[tail.get() - 1].next.take();
        let suffix = List {
            head,
            tail: head.and(list.tail),
        };
        list.tail = Some(tail);
        suffix
    }
}
