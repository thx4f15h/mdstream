//! AVL interval index. The rightmost union component of a prefix is an
//! associative summary: an earlier maximum end bridges that component exactly
//! when it is strictly beyond its start. Touching intervals remain distinct.
use std::{
    cmp::Reverse,
    ops::Bound,
    sync::atomic::{AtomicU64, Ordering},
};

use crate::FoldId;

pub(crate) type Key = (usize, Reverse<usize>, u8, FoldId);

pub(crate) fn key(start: usize, end: usize, id: FoldId) -> Key {
    (
        start,
        Reverse(end),
        match id {
            FoldId::Heading(_) => 0,
            FoldId::Container(_) => 1,
            _ => 2,
        },
        id,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Summary {
    pub owner: Key,
    pub end: usize,
}

impl Summary {
    fn join(self, right: Self) -> Self {
        Self {
            owner: if self.end > right.owner.0 {
                self.owner
            } else {
                right.owner
            },
            end: self.end.max(right.end),
        }
    }
}

struct Node {
    key: Key,
    children: [Option<Box<Node>>; 2],
    height: u32,
    summary: Summary,
}

impl Node {
    fn new(key: Key) -> Box<Self> {
        Box::new(Self {
            key,
            children: [None, None],
            height: 1,
            summary: Summary {
                owner: key,
                end: key.1.0,
            },
        })
    }
    fn height(node: &Option<Box<Self>>) -> u32 {
        node.as_ref().map_or(0, |n| n.height)
    }
    fn update(&mut self) {
        self.height = 1 + Self::height(&self.children[0]).max(Self::height(&self.children[1]));
        let own = Summary {
            owner: self.key,
            end: self.key.1.0,
        };
        let left = self.children[0]
            .as_ref()
            .map_or(own, |n| n.summary.join(own));
        self.summary = self.children[1]
            .as_ref()
            .map_or(left, |n| left.join(n.summary));
    }
    fn rotate(mut root: Box<Self>, side: usize) -> Box<Self> {
        let mut next = root.children[side].take().unwrap();
        root.children[side] = next.children[1 - side].take();
        root.update();
        next.children[1 - side] = Some(root);
        next.update();
        next
    }
    fn balance(mut root: Box<Self>) -> Box<Self> {
        root.update();
        for side in 0..2 {
            if Self::height(&root.children[side]) > Self::height(&root.children[1 - side]) + 1 {
                let child = root.children[side].as_ref().unwrap();
                if Self::height(&child.children[1 - side]) > Self::height(&child.children[side]) {
                    root.children[side] =
                        Some(Self::rotate(root.children[side].take().unwrap(), 1 - side));
                }
                return Self::rotate(root, side);
            }
        }
        root
    }
    fn insert(
        node: Option<Box<Self>>,
        key: Key,
        visits: &AtomicU64,
        inserted: &mut bool,
    ) -> Box<Self> {
        visits.fetch_add(1, Ordering::Relaxed);
        let Some(mut node) = node else {
            *inserted = true;
            return Self::new(key);
        };
        if key != node.key {
            let side = usize::from(key > node.key);
            node.children[side] = Some(Self::insert(
                node.children[side].take(),
                key,
                visits,
                inserted,
            ));
        }
        Self::balance(node)
    }
    fn remove(
        node: Option<Box<Self>>,
        key: Key,
        visits: &AtomicU64,
        removed: &mut bool,
    ) -> Option<Box<Self>> {
        visits.fetch_add(1, Ordering::Relaxed);
        let mut node = node?;
        if key == node.key {
            *removed = true;
            if node.children[0].is_none() {
                return node.children[1].take();
            }
            if node.children[1].is_none() {
                return node.children[0].take();
            }
            let mut next = node.children[1].as_ref().unwrap();
            visits.fetch_add(1, Ordering::Relaxed);
            while let Some(left) = &next.children[0] {
                visits.fetch_add(1, Ordering::Relaxed);
                next = left
            }
            node.key = next.key;
            node.children[1] = Self::remove(node.children[1].take(), node.key, visits, removed);
        } else {
            let side = usize::from(key > node.key);
            node.children[side] = Self::remove(node.children[side].take(), key, visits, removed);
        }
        Some(Self::balance(node))
    }
    fn prefix(&self, offset: usize, visits: &AtomicU64) -> Option<Summary> {
        visits.fetch_add(1, Ordering::Relaxed);
        if self.key.0 > offset {
            return self.children[0]
                .as_ref()
                .and_then(|n| n.prefix(offset, visits));
        }
        let own = Summary {
            owner: self.key,
            end: self.key.1.0,
        };
        let left = self.children[0]
            .as_ref()
            .map_or(own, |n| n.summary.join(own));
        Some(
            self.children[1]
                .as_ref()
                .and_then(|n| n.prefix(offset, visits))
                .map_or(left, |right| left.join(right)),
        )
    }
    fn parent(&self, before: Key, visits: &AtomicU64) -> Option<Key> {
        visits.fetch_add(1, Ordering::Relaxed);
        if self.summary.end < before.1.0 {
            return None;
        }
        if self.key >= before {
            return self.children[0]
                .as_ref()
                .and_then(|n| n.parent(before, visits));
        }
        if let Some(found) = self.children[1]
            .as_ref()
            .and_then(|n| n.parent(before, visits))
        {
            return Some(found);
        }
        if self.key.1.0 >= before.1.0 {
            return Some(self.key);
        }
        self.children[0]
            .as_ref()
            .and_then(|n| n.parent(before, visits))
    }

    fn next(&self, after: Bound<Key>, start: usize, end: usize, visits: &AtomicU64) -> Option<Key> {
        visits.fetch_add(1, Ordering::Relaxed);
        if self.summary.end <= start {
            return None;
        }
        let skip = match after {
            Bound::Included(k) => self.key < k,
            Bound::Excluded(k) => self.key <= k,
            Bound::Unbounded => false,
        };
        if skip {
            return self.children[1]
                .as_ref()
                .and_then(|n| n.next(after, start, end, visits));
        }
        if let Some(found) = self.children[0]
            .as_ref()
            .and_then(|n| n.next(after, start, end, visits))
        {
            return Some(found);
        }
        if self.key.0 >= end {
            return None;
        }
        if self.key.1.0 > start {
            return Some(self.key);
        }
        self.children[1]
            .as_ref()
            .and_then(|n| n.next(after, start, end, visits))
    }
}

#[derive(Default)]
pub(crate) struct Intervals {
    root: Option<Box<Node>>,
    visits: AtomicU64,
    len: usize,
}

impl Intervals {
    pub fn visits(&self) -> u64 {
        self.visits.load(Ordering::Relaxed)
    }
    pub fn node_count(&self) -> usize {
        self.len
    }
    pub fn insert(&mut self, key: Key) {
        let mut inserted = false;
        self.root = Some(Node::insert(
            self.root.take(),
            key,
            &self.visits,
            &mut inserted,
        ));
        self.len += usize::from(inserted);
    }
    pub fn remove(&mut self, key: Key) {
        let mut removed = false;
        self.root = Node::remove(self.root.take(), key, &self.visits, &mut removed);
        self.len -= usize::from(removed);
    }
    /// Current union component, without chasing an overlap chain. Its right
    /// frontier includes starts <= offset; later extensions are paid next steps.
    pub fn component(&self, offset: usize) -> Option<Summary> {
        self.root
            .as_ref()?
            .prefix(offset, &self.visits)
            .filter(|s| s.end > offset)
    }

    pub fn preceding_end(&self, offset: usize) -> usize {
        self.root
            .as_ref()
            .and_then(|root| root.prefix(offset, &self.visits))
            .map_or(0, |summary| summary.end)
    }
    /// Closest preceding envelope containing the whole key, including ties.
    /// Rightmost start, then smallest containing end and newest earlier ID.
    pub fn parent(&self, key: Key) -> Option<Key> {
        self.root.as_ref()?.parent(key, &self.visits)
    }

    pub fn next(&self, after: Bound<Key>, start: usize, end: usize) -> Option<Key> {
        self.root.as_ref()?.next(after, start, end, &self.visits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HeadingId;

    #[test]
    fn avl_union_prefix_matches_independent_sorted_interval_model() {
        let mut index = Intervals::default();
        let mut keys = Vec::new();
        let mut random = 19u64;
        for turn in 0..6000 {
            random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
            if turn % 3 == 0 && !keys.is_empty() {
                let k = keys.remove(random as usize % keys.len());
                index.remove(k);
            } else {
                let start = random as usize % 1000;
                let k = key(
                    start,
                    start + 1 + (random >> 24) as usize % 150,
                    FoldId::Heading(HeadingId(turn)),
                );
                keys.push(k);
                index.insert(k);
            }
            keys.sort();
            let at = (random >> 32) as usize % 1150;
            let mut expected: Option<Summary> = None;
            for &k in &keys {
                if k.0 > at {
                    break;
                }
                expected = Some(match expected {
                    Some(s) if s.end > k.0 => Summary {
                        owner: s.owner,
                        end: s.end.max(k.1.0),
                    },
                    _ => Summary {
                        owner: k,
                        end: k.1.0,
                    },
                });
            }
            assert_eq!(index.component(at), expected.filter(|s| s.end > at));
            let probe = key(at, at + 7, FoldId::Heading(HeadingId(usize::MAX)));
            assert_eq!(
                index.parent(probe),
                keys.iter()
                    .copied()
                    .rev()
                    .find(|k| *k < probe && k.1.0 >= probe.1.0)
            );
            let lower = keys.get(keys.len() / 2).copied();
            let bound = lower.map_or(Bound::Unbounded, Bound::Excluded);
            assert_eq!(
                index.next(bound, at, at + 50),
                keys.iter()
                    .copied()
                    .find(|k| lower.is_none_or(|l| *k > l) && k.0 < at + 50 && k.1.0 > at)
            );
            fn check(n: &Option<Box<Node>>) -> (u32, usize) {
                let Some(n) = n else { return (0, 0) };
                let (a, ac) = check(&n.children[0]);
                let (b, bc) = check(&n.children[1]);
                assert!(a.abs_diff(b) <= 1);
                assert_eq!(n.height, 1 + a.max(b));
                (n.height, ac + bc + 1)
            }
            assert_eq!(check(&index.root).1, keys.len());
            assert_eq!(index.node_count(), keys.len());
        }
    }
}
