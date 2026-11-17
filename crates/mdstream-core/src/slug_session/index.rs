//! Exact source-order greedy claims. Query leases are unoccupied endpoints.
use super::{
    numbers::{Numbers, Root},
    pages::{Arena, Id},
    rope::{Memory, Rope, Small},
    runtime::{Cx, Op},
};
use std::collections::{BTreeMap, BTreeSet};

struct Node {
    edge: Rope,
    parent: Option<Id>,
    children: BTreeMap<u8, Id>,
    refs: usize,
    owner: Option<usize>,
    numeric: Option<(Id, usize)>,
    candidates: Root,
    members: Root,
}
impl Node {
    fn new(edge: Rope, parent: Option<Id>) -> Self {
        Self {
            edge,
            parent,
            children: BTreeMap::new(),
            refs: 0,
            owner: None,
            numeric: None,
            candidates: None,
            members: None,
        }
    }
}
#[derive(Clone, Copy)]
pub(super) struct Entry {
    pub base: Id,
    pub rope: Rope,
    pub claim: Option<Id>,
    pub ordinal: usize,
}
pub(super) struct Index {
    pub mem: Memory,
    nodes: Arena<Node>,
    numbers: Numbers,
    root: Id,
    pub entries: BTreeMap<usize, Entry>,
    pub dirty: BTreeSet<usize>,
    pub epoch: u64,
}
impl Index {
    pub fn new(cx: Cx) -> Self {
        let mut nodes = Arena::tracked(cx.clone(), 1);
        let mut root = Node::new(Rope::Empty, None);
        root.refs = 1;
        let root = nodes.insert(root);
        Self {
            mem: Memory::new(cx.clone()),
            nodes,
            numbers: Numbers::new(cx),
            root,
            entries: BTreeMap::new(),
            dirty: BTreeSet::new(),
            epoch: 0,
        }
    }
    fn sync(&self) {
        self.mem.cx.storage(
            6,
            self.entries.len() * std::mem::size_of::<(usize, Entry)>()
                + self.dirty.len() * std::mem::size_of::<usize>(),
            self.entries.len() + self.dirty.len(),
            0,
        );
    }
    async fn keep(&mut self, id: Id) {
        self.mem.cx.charge(Op::Trie).await;
        self.nodes.get_mut(id).refs += 1;
    }
    async fn release(&mut self, mut id: Id) {
        self.mem.cx.charge(Op::Trie).await;
        self.nodes.get_mut(id).refs -= 1;
        loop {
            self.mem.cx.charge(Op::Trie).await;
            let node = self.nodes.get(id);
            let Some(parent) = node.parent else {
                break;
            };
            if node.refs != 0 || node.owner.is_some() || node.children.len() > 1 {
                break;
            }
            assert!(node.candidates.is_none() && node.members.is_none() && node.numeric.is_none());
            let edge = node.edge;
            let child = node.children.first_key_value().map(|(_, id)| *id);
            let first = self.mem.byte(edge, 0).await;
            if let Some(child) = child {
                let child_edge = self.nodes.get(child).edge;
                let left = self.mem.retain(edge).await;
                let right = self.mem.retain(child_edge).await;
                let joined = self.mem.join(left, right).await;
                self.nodes.get_mut(child).edge = joined;
                self.nodes.get_mut(child).parent = Some(parent);
                self.nodes.get_mut(parent).children.insert(first, child);
                self.mem.release(child_edge).await;
            } else {
                self.nodes.get_mut(parent).children.remove(&first);
            }
            let removed = self.nodes.remove(id);
            self.mem.release(removed.edge).await;
            id = parent;
        }
    }
    async fn intern(&mut self, rope: Rope, start: Id, mut off: usize) -> Id {
        let mut node = start;
        let len = self.mem.len(rope);
        while off < len {
            self.mem.cx.charge(Op::Trie).await;
            let first = self.mem.byte(rope, off).await;
            let child = self.nodes.get(node).children.get(&first).copied();
            let Some(child) = child else {
                let edge = self.mem.cut(rope, off, len).await;
                let child = self.nodes.insert(Node::new(edge, Some(node)));
                self.nodes.get_mut(node).children.insert(first, child);
                return child;
            };
            let edge = self.nodes.get(child).edge;
            let tail = self.mem.cut(rope, off, len).await;
            let same = self.mem.lcp(edge, tail).await;
            self.mem.release(tail).await;
            if same < self.mem.len(edge) {
                assert!(same != 0);
                let prefix = self.mem.cut(edge, 0, same).await;
                let split = self.nodes.insert(Node::new(prefix, Some(node)));
                self.nodes.get_mut(node).children.insert(first, split);
                let suffix = self.mem.cut(edge, same, self.mem.len(edge)).await;
                self.nodes.get_mut(child).edge = suffix;
                self.nodes.get_mut(child).parent = Some(split);
                let byte = self.mem.byte(suffix, 0).await;
                self.nodes.get_mut(split).children.insert(byte, child);
                self.mem.release(edge).await;
                if off + same == len {
                    return split;
                }
                let edge = self.mem.cut(rope, off + same, len).await;
                let extra = self.nodes.insert(Node::new(edge, Some(split)));
                let byte = self.mem.byte(edge, 0).await;
                self.nodes.get_mut(split).children.insert(byte, extra);
                return extra;
            }
            off += same;
            node = child;
        }
        node
    }
    async fn numeric_parent(&mut self, node: Id, rope: Rope, hint: Option<(Id, usize)>) {
        if self.nodes.get(node).numeric.is_some() {
            return;
        }
        let parent = if let Some(hint) = hint {
            Some(hint)
        } else {
            let mut at = self.mem.len(rope);
            let mut digits = 0;
            while at != 0 && digits <= 20 {
                let byte = self.mem.byte(rope, at - 1).await;
                if !byte.is_ascii_digit() {
                    break;
                }
                at -= 1;
                digits += 1;
            }
            if digits == 0
                || digits > 20
                || at <= 1
                || self.mem.byte(rope, at - 1).await != b'-'
                || self.mem.byte(rope, at).await == b'0'
            {
                None
            } else {
                let mut number = Some(0usize);
                for pos in at..at + digits {
                    let digit = self.mem.byte(rope, pos).await - b'0';
                    number = number
                        .and_then(|n| n.checked_mul(10))
                        .and_then(|n| n.checked_add(usize::from(digit)));
                }
                if let Some(number) = number {
                    let prefix = self.mem.cut(rope, 0, at - 1).await;
                    let parent = self.intern(prefix, self.root, 0).await;
                    self.mem.release(prefix).await;
                    Some((parent, number))
                } else {
                    None
                }
            }
        };
        if let Some((parent, _)) = parent {
            self.keep(parent).await;
        }
        self.nodes.get_mut(node).numeric = parent;
    }
    async fn enqueue(&mut self, id: usize) {
        self.mem.cx.charge(Op::Trie).await;
        self.dirty.insert(id);
        self.sync();
    }
    async fn wake(&mut self, base: Id, ordinal: usize) {
        let root = self.nodes.get(base).members;
        if let Some(id) = self.numbers.min_after(root, ordinal).await {
            self.enqueue(id).await;
        }
    }
    async fn owner(
        &mut self,
        node: Id,
        value: Option<usize>,
        rope: Rope,
        hint: Option<(Id, usize)>,
    ) {
        let old = self.nodes.get(node).owner;
        if old == value {
            return;
        }
        if let Some(value) = value {
            if old.is_none() {
                self.keep(node).await;
                self.numeric_parent(node, rope, hint).await;
            }
            let root = self.nodes.get(node).candidates;
            let root = self.numbers.put(root, 0, value).await;
            self.nodes.get_mut(node).candidates = root;
            if let Some((parent, n)) = self.nodes.get(node).numeric {
                let root = self.nodes.get(parent).candidates;
                let root = self.numbers.put(root, n, value).await;
                self.nodes.get_mut(parent).candidates = root;
            }
            let old_edge = self.nodes.get(node).edge;
            let len = self.mem.len(rope);
            let edge = self.mem.cut(rope, len - self.mem.len(old_edge), len).await;
            self.nodes.get_mut(node).edge = edge;
            self.mem.release(old_edge).await;
        } else {
            let root = self.nodes.get(node).candidates;
            let root = self.numbers.remove(root, 0).await;
            self.nodes.get_mut(node).candidates = root;
            if let Some((parent, n)) = self.nodes.get(node).numeric {
                let root = self.nodes.get(parent).candidates;
                let root = self.numbers.remove(root, n).await;
                self.nodes.get_mut(parent).candidates = root;
            }
            self.wake(node, 0).await;
            if let Some((parent, n)) = self.nodes.get(node).numeric {
                self.wake(parent, n).await;
            }
        }
        self.nodes.get_mut(node).owner = value;
        self.epoch = self.epoch.checked_add(1).unwrap();
        if value.is_none() {
            let numeric = self.nodes.get_mut(node).numeric.take();
            self.release(node).await;
            if let Some((parent, _)) = numeric {
                self.release(parent).await;
            }
        }
    }
    async fn detach(&mut self, id: usize) {
        let entry = self.entries[&id];
        let Some(claim) = entry.claim else {
            return;
        };
        let root = self.nodes.get(entry.base).members;
        let root = self.numbers.remove(root, entry.ordinal).await;
        self.nodes.get_mut(entry.base).members = root;
        self.entries.get_mut(&id).unwrap().claim = None;
        self.owner(claim, None, Rope::Empty, None).await;
    }
    pub async fn set(&mut self, id: usize, mut rope: Rope) {
        if self.mem.len(rope) == 0 {
            rope = Rope::Small(Small::from_bytes(b"section"));
        }
        let old = self.entries.get(&id).copied();
        let base = if let Some(old) = old
            && self.mem.shared_prefix(old.rope, rope).await
        {
            self.intern(rope, old.base, self.mem.len(old.rope)).await
        } else {
            self.intern(rope, self.root, 0).await
        };
        if let Some(old) = old
            && old.base == base
        {
            self.entries.get_mut(&id).unwrap().rope = rope;
            self.mem.release(old.rope).await;
            return;
        }
        self.mem.cx.base_changed();
        self.keep(base).await;
        if let Some(old) = old {
            self.detach(id).await;
            self.release(old.base).await;
            self.mem.release(old.rope).await;
        }
        self.entries.insert(
            id,
            Entry {
                base,
                rope,
                claim: None,
                ordinal: 0,
            },
        );
        self.sync();
        self.enqueue(id).await;
    }
    async fn suffix(&self, mut n: usize) -> Small {
        let mut reverse = [0; 32];
        let mut len = 0;
        loop {
            reverse[len] = b'0' + (n % 10) as u8;
            len += 1;
            n /= 10;
            if n == 0 {
                break;
            }
        }
        let mut out = Small::EMPTY;
        for at in 0..=len {
            self.mem.cx.charge(Op::Suffix).await;
            out.bytes[at] = if at == 0 { b'-' } else { reverse[len - at] };
            out.len += 1;
        }
        out
    }
    pub async fn maintain(&mut self) {
        while let Some(&id) = self.dirty.first() {
            self.mem.cx.charge(Op::Trie).await;
            self.dirty.remove(&id);
            self.sync();
            let entry = self.entries[&id];
            let ordinal = self
                .numbers
                .first_available(self.nodes.get(entry.base).candidates, id)
                .await;
            let name = if ordinal == 0 {
                self.mem.retain(entry.rope).await
            } else {
                let suffix = self.suffix(ordinal).await;
                let base = self.mem.retain(entry.rope).await;
                self.mem.join(base, Rope::Small(suffix)).await
            };
            let node = if ordinal == 0 {
                entry.base
            } else {
                self.intern(name, entry.base, self.mem.len(entry.rope))
                    .await
            };
            if Some(node) == entry.claim {
                self.mem.release(name).await;
                continue;
            }
            self.keep(node).await;
            self.detach(id).await;
            if let Some(displaced) = self.nodes.get(node).owner {
                assert!(displaced > id);
                let d = self.entries[&displaced];
                let root = self.nodes.get(d.base).members;
                let root = self.numbers.remove(root, d.ordinal).await;
                self.nodes.get_mut(d.base).members = root;
                self.entries.get_mut(&displaced).unwrap().claim = None;
                self.enqueue(displaced).await;
            }
            self.owner(
                node,
                Some(id),
                name,
                (ordinal != 0).then_some((entry.base, ordinal)),
            )
            .await;
            let e = self.entries.get_mut(&id).unwrap();
            e.claim = Some(node);
            e.ordinal = ordinal;
            let root = self.nodes.get(entry.base).members;
            let root = self.numbers.put(root, ordinal, id).await;
            self.nodes.get_mut(entry.base).members = root;
            self.release(node).await;
            self.mem.release(name).await;
            self.mem.cx.changed();
        }
    }
    pub async fn register(&mut self, rope: Rope) -> Id {
        let node = self.intern(rope, self.root, 0).await;
        self.keep(node).await;
        self.mem.release(rope).await;
        node
    }
    pub async fn unregister(&mut self, id: Id) {
        if self.nodes.contains(id) {
            self.release(id).await;
        }
    }
    pub fn current(&self, id: Id) -> Option<usize> {
        self.nodes
            .contains(id)
            .then(|| self.nodes.get(id).owner)
            .flatten()
    }
    pub async fn close(&mut self) {
        while let Some((&id, _)) = self.entries.first_key_value() {
            self.mem.cx.charge(Op::Retire).await;
            let e = self.entries[&id];
            self.detach(id).await;
            self.entries.remove(&id);
            self.sync();
            self.dirty.remove(&id);
            self.sync();
            self.release(e.base).await;
            self.mem.release(e.rope).await;
        }
        while !self.dirty.is_empty() {
            self.mem.cx.charge(Op::Retire).await;
            self.dirty.pop_first();
            self.sync();
        }
        self.mem.cx.charge(Op::Retire).await;
        let root = self.nodes.remove(self.root);
        assert!(root.children.is_empty());
        self.sync();
    }
}
