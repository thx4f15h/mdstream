use super::{
    pages::{Arena, Bytes, Id},
    runtime::{Cx, Job, Op, boxed},
};
use std::collections::{BTreeMap, LinkedList};

pub(super) const RAW: usize = 0;
pub(super) const CODE: usize = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Small {
    pub bytes: [u8; 32],
    pub start: u8,
    pub len: u8,
}
impl Small {
    pub const EMPTY: Self = Self {
        bytes: [0; 32],
        start: 0,
        len: 0,
    };
    pub fn from_bytes(bytes: &[u8]) -> Self {
        assert!(bytes.len() <= 32);
        let mut out = Self::EMPTY;
        out.bytes[..bytes.len()].copy_from_slice(bytes);
        out.len = bytes.len() as u8;
        out
    }
    pub fn get(self, at: usize) -> u8 {
        self.bytes[usize::from(self.start) + at]
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Rope {
    Empty,
    Span {
        buffer: usize,
        start: usize,
        len: usize,
    },
    Small(Small),
    Node(Id),
}
#[derive(Clone, Copy)]
struct Node {
    left: Rope,
    right: Rope,
    len: usize,
    height: u8,
    refs: usize,
}
struct Buffer {
    bytes: Bytes,
    refs: usize,
}

pub(super) struct Memory {
    pub cx: Cx,
    buffers: BTreeMap<usize, Buffer>,
    next_buffer: usize,
    byte_capacity: usize,
    nodes: Arena<Node>,
    retired_nodes: LinkedList<Id>,
    retired_buffers: LinkedList<usize>,
}
impl Memory {
    pub fn new(cx: Cx) -> Self {
        let mut buffers = BTreeMap::new();
        for id in [RAW, CODE] {
            buffers.insert(
                id,
                Buffer {
                    bytes: Bytes::default(),
                    refs: 1,
                },
            );
        }
        Self {
            nodes: Arena::tracked(cx.clone(), 0),
            cx,
            buffers,
            next_buffer: 2,
            byte_capacity: 0,
            retired_nodes: LinkedList::new(),
            retired_buffers: LinkedList::new(),
        }
    }
    fn sync(&self) {
        self.cx.storage(
            4,
            self.byte_capacity
                + self.buffers.len() * std::mem::size_of::<(usize, Buffer)>()
                + self.byte_capacity / super::pages::BYTE_PAGE
                    * std::mem::size_of::<(usize, Box<[u8; super::pages::BYTE_PAGE]>)>()
                + self.retired_nodes.len() * std::mem::size_of::<(Id, usize, usize)>()
                + self.retired_buffers.len() * std::mem::size_of::<(usize, usize, usize)>(),
            self.byte_capacity / super::pages::BYTE_PAGE + self.buffers.len(),
            self.retired_nodes.len() + self.retired_buffers.len(),
        );
    }
    pub fn new_buffer(&mut self) -> usize {
        let id = self.next_buffer;
        self.next_buffer = id.checked_add(1).expect("slug buffer identity exhausted");
        self.buffers.insert(
            id,
            Buffer {
                bytes: Bytes::default(),
                refs: 1,
            },
        );
        self.sync();
        id
    }
    pub fn push(&mut self, buffer: usize, byte: u8) {
        let b = self.buffers.get_mut(&buffer).unwrap();
        let old = b.bytes.capacity();
        b.bytes.push(byte);
        self.byte_capacity += b.bytes.capacity() - old;
        self.sync();
    }
    pub fn buffer_len(&self, buffer: usize) -> usize {
        self.buffers[&buffer].bytes.len
    }
    pub fn buffer_byte(&self, buffer: usize, at: usize) -> u8 {
        self.buffers[&buffer].bytes.get(at)
    }
    pub fn span(&mut self, buffer: usize, start: usize, len: usize) -> Rope {
        if len == 0 {
            return Rope::Empty;
        }
        assert!(start + len <= self.buffer_len(buffer));
        if buffer >= 2 {
            self.buffers.get_mut(&buffer).unwrap().refs += 1;
        }
        Rope::Span { buffer, start, len }
    }
    pub fn drop_buffer_owner(&mut self, buffer: usize) {
        assert!(buffer >= 2);
        let b = self.buffers.get_mut(&buffer).unwrap();
        b.refs -= 1;
        if b.refs == 0 {
            self.retired_buffers.push_back(buffer);
        }
        self.sync();
    }
    pub fn len(&self, rope: Rope) -> usize {
        match rope {
            Rope::Empty => 0,
            Rope::Span { len, .. } => len,
            Rope::Small(s) => usize::from(s.len),
            Rope::Node(id) => self.nodes.get(id).len,
        }
    }
    fn height(&self, rope: Rope) -> u8 {
        match rope {
            Rope::Node(id) => self.nodes.get(id).height,
            _ => 1,
        }
    }
    pub async fn retain(&mut self, rope: Rope) -> Rope {
        self.cx.charge(Op::Rope).await;
        match rope {
            Rope::Node(id) => self.nodes.get_mut(id).refs += 1,
            Rope::Span { buffer, .. } if buffer >= 2 => {
                self.buffers.get_mut(&buffer).unwrap().refs += 1
            }
            _ => (),
        }
        rope
    }
    pub async fn release(&mut self, rope: Rope) {
        self.cx.charge(Op::Rope).await;
        match rope {
            Rope::Node(id) => {
                let n = self.nodes.get_mut(id);
                n.refs -= 1;
                if n.refs == 0 {
                    self.retired_nodes.push_back(id);
                }
            }
            Rope::Span { buffer, .. } if buffer >= 2 => self.drop_buffer_owner(buffer),
            _ => (),
        }
        self.sync();
    }
    async fn branch(&mut self, left: Rope, right: Rope) -> Rope {
        self.cx.charge(Op::Rope).await;
        let node = Node {
            left,
            right,
            len: self.len(left).checked_add(self.len(right)).unwrap(),
            height: 1 + self.height(left).max(self.height(right)),
            refs: 1,
        };
        Rope::Node(self.nodes.insert(node))
    }
    async fn explode(&mut self, rope: Rope) -> (Rope, Rope) {
        self.cx.charge(Op::Rope).await;
        let Rope::Node(id) = rope else { unreachable!() };
        let n = *self.nodes.get(id);
        let left = self.retain(n.left).await;
        let right = self.retain(n.right).await;
        self.release(rope).await;
        (left, right)
    }
    async fn balance(&mut self, a: Rope, b: Rope) -> Rope {
        self.cx.charge(Op::Rope).await;
        if self.height(a) > self.height(b) + 1 {
            let (al, ar) = self.explode(a).await;
            if self.height(al) >= self.height(ar) {
                let right = self.branch(ar, b).await;
                self.branch(al, right).await
            } else {
                let (arl, arr) = self.explode(ar).await;
                let left = self.branch(al, arl).await;
                let right = self.branch(arr, b).await;
                self.branch(left, right).await
            }
        } else if self.height(b) > self.height(a) + 1 {
            let (bl, br) = self.explode(b).await;
            if self.height(br) >= self.height(bl) {
                let left = self.branch(a, bl).await;
                self.branch(left, br).await
            } else {
                let (bll, blr) = self.explode(bl).await;
                let left = self.branch(a, bll).await;
                let right = self.branch(blr, br).await;
                self.branch(left, right).await
            }
        } else {
            self.branch(a, b).await
        }
    }
    /// Consumes two owned handles. Plain source appends coalesce without copying.
    pub fn join(&mut self, a: Rope, b: Rope) -> Job<'_, Rope> {
        boxed(&self.cx.clone(), async move {
            self.cx.charge(Op::Rope).await;
            if a == Rope::Empty {
                return b;
            }
            if b == Rope::Empty {
                return a;
            }
            if let (
                Rope::Span {
                    buffer: ab,
                    start,
                    len,
                },
                Rope::Span {
                    buffer: bb,
                    start: bs,
                    len: bn,
                },
            ) = (a, b)
                && ab == bb
                && start + len == bs
            {
                self.release(b).await;
                return Rope::Span {
                    buffer: ab,
                    start,
                    len: len + bn,
                };
            }
            if let (Rope::Small(sa), Rope::Small(sb)) = (a, b)
                && usize::from(sa.len) + usize::from(sb.len) <= 32
            {
                let mut out = Small::EMPTY;
                for i in 0..usize::from(sa.len) + usize::from(sb.len) {
                    self.cx.charge(Op::Copy).await;
                    out.bytes[i] = if i < usize::from(sa.len) {
                        sa.get(i)
                    } else {
                        sb.get(i - usize::from(sa.len))
                    };
                    out.len += 1;
                }
                return Rope::Small(out);
            }
            if self.height(a) > self.height(b) + 1 {
                let (al, ar) = self.explode(a).await;
                let right = self.join(ar, b).await;
                self.balance(al, right).await
            } else if self.height(b) > self.height(a) + 1 {
                let (bl, br) = self.explode(b).await;
                let left = self.join(a, bl).await;
                self.balance(left, br).await
            } else {
                self.branch(a, b).await
            }
        })
    }
    /// Borrows the input, returning an owned slice. No byte decoding or copying.
    pub fn cut(&mut self, rope: Rope, lo: usize, hi: usize) -> Job<'_, Rope> {
        boxed(&self.cx.clone(), async move {
            self.cx.charge(Op::Rope).await;
            assert!(lo <= hi && hi <= self.len(rope));
            if lo == hi {
                return Rope::Empty;
            }
            if lo == 0 && hi == self.len(rope) {
                return self.retain(rope).await;
            }
            match rope {
                Rope::Span { buffer, start, .. } => self.span(buffer, start + lo, hi - lo),
                Rope::Small(mut s) => {
                    s.start += lo as u8;
                    s.len = (hi - lo) as u8;
                    Rope::Small(s)
                }
                Rope::Node(id) => {
                    let n = *self.nodes.get(id);
                    let left_len = self.len(n.left);
                    if hi <= left_len {
                        self.cut(n.left, lo, hi).await
                    } else if lo >= left_len {
                        self.cut(n.right, lo - left_len, hi - left_len).await
                    } else {
                        let left = self.cut(n.left, lo, left_len).await;
                        let right = self.cut(n.right, 0, hi - left_len).await;
                        self.join(left, right).await
                    }
                }
                Rope::Empty => unreachable!(),
            }
        })
    }
    pub async fn byte(&self, mut rope: Rope, mut at: usize) -> u8 {
        loop {
            self.cx.charge(Op::Rope).await;
            match rope {
                Rope::Span { buffer, start, len } => {
                    assert!(at < len);
                    return self.buffer_byte(buffer, start + at);
                }
                Rope::Small(s) => return s.get(at),
                Rope::Node(id) => {
                    let n = self.nodes.get(id);
                    let left = self.len(n.left);
                    if at < left {
                        rope = n.left;
                    } else {
                        at -= left;
                        rope = n.right;
                    }
                }
                Rope::Empty => unreachable!(),
            }
        }
    }
    pub async fn lcp(&self, a: Rope, b: Rope) -> usize {
        self.compare(a, b, false).await.0
    }
    pub async fn shared_prefix(&self, a: Rope, b: Rope) -> bool {
        if self.len(a) > self.len(b) {
            return false;
        }
        self.compare(a, b, true).await.1
    }
    async fn compare(&self, a: Rope, b: Rope, proof_only: bool) -> (usize, bool) {
        let mut aa = Stack::new(View::new(a, self.len(a)));
        let mut bb = Stack::new(View::new(b, self.len(b)));
        let mut total = 0;
        while let (Some(x), Some(y)) = (aa.last(), bb.last()) {
            self.cx.charge(Op::Rope).await;
            let n = x.len.min(y.len);
            let same = x.rope == y.rope && x.off == y.off
                || matches!((x.rope,y.rope),
                (Rope::Span{buffer:ab,start:as_,..},Rope::Span{buffer:bb,start:bs,..}) if ab==bb && as_+x.off==bs+y.off);
            if same {
                aa.consume(n);
                bb.consume(n);
                total += n;
                continue;
            }
            match (x.rope, y.rope) {
                (Rope::Node(_), Rope::Node(_)) if x.len >= y.len => self.expand(&mut aa),
                (_, Rope::Node(_)) => self.expand(&mut bb),
                (Rope::Node(_), _) => self.expand(&mut aa),
                _ => {
                    if proof_only {
                        return (total, false);
                    }
                    for i in 0..n {
                        self.cx.charge(Op::Compare).await;
                        let av = self.leaf_byte(x.rope, x.off + i);
                        let bv = self.leaf_byte(y.rope, y.off + i);
                        if av != bv {
                            return (total, false);
                        }
                        total += 1;
                    }
                    aa.consume(n);
                    bb.consume(n);
                }
            }
        }
        (total, aa.last().is_none())
    }
    fn leaf_byte(&self, rope: Rope, at: usize) -> u8 {
        match rope {
            Rope::Span { buffer, start, .. } => self.buffer_byte(buffer, start + at),
            Rope::Small(s) => s.get(at),
            _ => unreachable!(),
        }
    }
    fn expand(&self, stack: &mut Stack) {
        let v = stack.pop();
        let Rope::Node(id) = v.rope else {
            unreachable!()
        };
        let n = *self.nodes.get(id);
        let left = self.len(n.left);
        let end = v.off + v.len;
        if end > left {
            stack.push(View {
                rope: n.right,
                off: v.off.saturating_sub(left),
                len: end - left.max(v.off),
            });
        }
        if v.off < left {
            stack.push(View {
                rope: n.left,
                off: v.off,
                len: end.min(left) - v.off,
            });
        }
    }
    pub async fn retire(&mut self) {
        while !self.retired_nodes.is_empty() {
            self.cx.charge(Op::Retire).await;
            let id = self.retired_nodes.pop_front().unwrap();
            self.sync();
            let n = self.nodes.remove(id);
            assert_eq!(n.refs, 0);
            self.release(n.left).await;
            self.release(n.right).await;
        }
        while let Some(&id) = self.retired_buffers.front() {
            self.cx.charge(Op::Retire).await;
            if self.buffers.get_mut(&id).unwrap().bytes.retire_page() {
                self.byte_capacity -= super::pages::BYTE_PAGE;
            } else {
                self.buffers.remove(&id);
                self.retired_buffers.pop_front();
            }
            self.sync();
        }
        self.sync();
    }
    pub fn hop(&self, rope: Rope, at: usize) -> Result<u8, (Rope, usize)> {
        match rope {
            Rope::Node(id) => {
                let n = self.nodes.get(id);
                let len = self.len(n.left);
                if at < len {
                    Err((n.left, at))
                } else {
                    Err((n.right, at - len))
                }
            }
            Rope::Span { buffer, start, .. } => Ok(self.buffer_byte(buffer, start + at)),
            Rope::Small(text) => Ok(text.get(at)),
            Rope::Empty => unreachable!(),
        }
    }
    pub async fn close_buffers(&mut self) {
        while let Some((&id, _)) = self.buffers.first_key_value() {
            self.cx.charge(Op::Retire).await;
            if self.buffers.get_mut(&id).unwrap().bytes.retire_page() {
                self.byte_capacity -= super::pages::BYTE_PAGE;
            } else {
                self.buffers.remove(&id);
            }
            self.sync();
        }
    }
    #[cfg(test)]
    pub fn owned(&self) -> (usize, usize, usize, usize, usize) {
        (
            self.nodes.capacity(),
            self.buffers.values().map(|b| b.bytes.capacity()).sum(),
            self.nodes.directory_entries()
                + self
                    .buffers
                    .values()
                    .map(|b| b.bytes.page_count())
                    .sum::<usize>()
                + self.buffers.len(),
            self.nodes.live,
            self.retired_nodes.len() + self.retired_buffers.len(),
        )
    }
}

#[derive(Clone, Copy)]
struct View {
    rope: Rope,
    off: usize,
    len: usize,
}
impl View {
    fn new(rope: Rope, len: usize) -> Self {
        Self { rope, off: 0, len }
    }
}
const STACK: usize = usize::BITS as usize * 2 + 4;
struct Stack {
    slots: [View; STACK],
    len: usize,
}
impl Stack {
    fn new(v: View) -> Self {
        let mut s = Self {
            slots: [View::new(Rope::Empty, 0); STACK],
            len: 0,
        };
        s.push(v);
        s
    }
    fn push(&mut self, v: View) {
        if v.len != 0 {
            self.slots[self.len] = v;
            self.len += 1;
        }
    }
    fn last(&self) -> Option<View> {
        self.len.checked_sub(1).map(|i| self.slots[i])
    }
    fn pop(&mut self) -> View {
        self.len -= 1;
        self.slots[self.len]
    }
    fn consume(&mut self, n: usize) {
        let mut v = self.pop();
        v.off += n;
        v.len -= n;
        self.push(v);
    }
}
