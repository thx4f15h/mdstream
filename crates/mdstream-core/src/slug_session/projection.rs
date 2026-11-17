//! A source-coordinate range tree with cached normalized ropes. Interior raw
//! gaps and closed code bodies are evaluated from the tape, never re-decoded.
use super::{
    pages::{Arena, Id},
    rope::{Memory, Rope, Small},
    runtime::{Job, Op, boxed},
    tape::Tape,
};
use crate::Span;
use std::collections::LinkedList;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Raw,
    Code(Span),
    Hidden,
    Atom { anchor: usize, text: Small },
}
struct Node {
    tag: Option<Kind>,
    children: [Option<Id>; 2],
    rope: Rope,
}
pub(super) struct Projection {
    nodes: Arena<Node>,
    root: Option<Id>,
    capacity: usize,
    available: usize,
    retired: LinkedList<Id>,
}
impl Default for Projection {
    fn default() -> Self {
        Self {
            nodes: Arena::default(),
            root: None,
            capacity: 1,
            available: 0,
            retired: LinkedList::new(),
        }
    }
}
impl Projection {
    pub fn new(cx: super::runtime::Cx) -> Self {
        Self {
            nodes: Arena::tracked(cx, 3),
            ..Default::default()
        }
    }
    async fn value(kind: Kind, span: Span, tape: &Tape, mem: &mut Memory) -> Rope {
        if span.is_empty() {
            return Rope::Empty;
        }
        match kind {
            Kind::Raw => tape.raw(mem, span).await,
            Kind::Code(context) => tape.code(mem, span, context).await,
            Kind::Hidden => Rope::Empty,
            Kind::Atom { anchor, text } => {
                if span.start <= anchor && anchor < span.end {
                    Rope::Small(text)
                } else {
                    Rope::Empty
                }
            }
        }
    }
    async fn aggregate(&self, id: Option<Id>, span: Span, tape: &Tape, mem: &mut Memory) -> Rope {
        match id {
            Some(id) => mem.retain(self.nodes.get(id).rope).await,
            None => Self::value(Kind::Raw, span, tape, mem).await,
        }
    }
    pub async fn extend(&mut self, tape: &Tape, mem: &mut Memory, changed: Span) {
        self.available = tape.len;
        while self.capacity < self.available {
            mem.cx.charge(Op::Projection).await;
            let left = self.root;
            let middle = self.capacity;
            self.capacity = self
                .capacity
                .checked_mul(2)
                .expect("source range fits usize");
            let a = self.aggregate(left, Span::new(0, middle), tape, mem).await;
            let b = Self::value(
                Kind::Raw,
                Span::new(middle, self.available.min(self.capacity)),
                tape,
                mem,
            )
            .await;
            let rope = mem.join(a, b).await;
            self.root = Some(self.nodes.insert(Node {
                tag: None,
                children: [left, None],
                rope,
            }));
        }
        self.root = self
            .refresh(self.root, Span::new(0, self.capacity), changed, tape, mem)
            .await;
    }
    fn refresh<'a>(
        &'a mut self,
        id: Option<Id>,
        whole: Span,
        changed: Span,
        tape: &'a Tape,
        mem: &'a mut Memory,
    ) -> Job<'a, Option<Id>> {
        boxed(&mem.cx.clone(), async move {
            mem.cx.charge(Op::Projection).await;
            if whole.start >= changed.end || changed.start >= whole.end {
                return id;
            }
            let id = id?;
            let (tag, children) = {
                let n = self.nodes.get(id);
                (n.tag, n.children)
            };
            let end = whole.end.min(self.available);
            let next = if let Some(kind) = tag {
                Self::value(kind, Span::new(whole.start.min(end), end), tape, mem).await
            } else {
                let mid = whole.start + (whole.end - whole.start) / 2;
                let a = self
                    .refresh(children[0], Span::new(whole.start, mid), changed, tape, mem)
                    .await;
                let b = self
                    .refresh(children[1], Span::new(mid, whole.end), changed, tape, mem)
                    .await;
                self.nodes.get_mut(id).children = [a, b];
                self.join_children([a, b], whole, tape, mem).await
            };
            let old = std::mem::replace(&mut self.nodes.get_mut(id).rope, next);
            mem.release(old).await;
            Some(id)
        })
    }
    async fn join_children(
        &self,
        children: [Option<Id>; 2],
        whole: Span,
        tape: &Tape,
        mem: &mut Memory,
    ) -> Rope {
        let mid = whole.start + (whole.end - whole.start) / 2;
        let end = whole.end.min(self.available);
        let left = self
            .aggregate(
                children[0],
                Span::new(whole.start.min(end), mid.min(end)),
                tape,
                mem,
            )
            .await;
        let right = self
            .aggregate(children[1], Span::new(mid.min(end), end), tape, mem)
            .await;
        mem.join(left, right).await
    }
    pub async fn assign(&mut self, source: Span, kind: Kind, tape: &Tape, mem: &mut Memory) {
        assert!(source.end <= self.available);
        if !source.is_empty() {
            self.root = self
                .set(
                    self.root,
                    Span::new(0, self.capacity),
                    source,
                    kind,
                    tape,
                    mem,
                )
                .await;
        }
    }
    fn set<'a>(
        &'a mut self,
        id: Option<Id>,
        whole: Span,
        source: Span,
        kind: Kind,
        tape: &'a Tape,
        mem: &'a mut Memory,
    ) -> Job<'a, Option<Id>> {
        boxed(&mem.cx.clone(), async move {
            mem.cx.charge(Op::Projection).await;
            if whole.start >= source.end || source.start >= whole.end {
                return id;
            }
            let tag = id.map_or(Some(Kind::Raw), |id| self.nodes.get(id).tag);
            if tag == Some(kind) {
                return id;
            }
            let id = match id {
                Some(id) => id,
                None => {
                    let end = whole.end.min(self.available);
                    let rope =
                        Self::value(Kind::Raw, Span::new(whole.start.min(end), end), tape, mem)
                            .await;
                    self.nodes.insert(Node {
                        tag: Some(Kind::Raw),
                        children: [None, None],
                        rope,
                    })
                }
            };
            if source.start <= whole.start && whole.end <= source.end {
                let old_children = self.nodes.get(id).children;
                for child in old_children.into_iter().flatten() {
                    self.retired.push_back(child);
                }
                self.sync_retired(mem);
                let next = Self::value(kind, whole, tape, mem).await;
                let n = self.nodes.get_mut(id);
                n.tag = Some(kind);
                n.children = [None, None];
                let old = std::mem::replace(&mut n.rope, next);
                mem.release(old).await;
                return Some(id);
            }
            let mid = whole.start + (whole.end - whole.start) / 2;
            if let Some(old) = tag {
                let mut children = [None, None];
                if old != Kind::Raw {
                    for (side, part) in [Span::new(whole.start, mid), Span::new(mid, whole.end)]
                        .into_iter()
                        .enumerate()
                    {
                        mem.cx.charge(Op::Projection).await;
                        let end = part.end.min(self.available);
                        let part = Span::new(part.start.min(end), end);
                        let rope = Self::value(old, part, tape, mem).await;
                        children[side] = Some(self.nodes.insert(Node {
                            tag: Some(old),
                            children: [None, None],
                            rope,
                        }));
                    }
                }
                let n = self.nodes.get_mut(id);
                n.tag = None;
                n.children = children;
            }
            let children = self.nodes.get(id).children;
            let a = self
                .set(
                    children[0],
                    Span::new(whole.start, mid),
                    source,
                    kind,
                    tape,
                    mem,
                )
                .await;
            let b = self
                .set(
                    children[1],
                    Span::new(mid, whole.end),
                    source,
                    kind,
                    tape,
                    mem,
                )
                .await;
            self.nodes.get_mut(id).children = [a, b];
            let next = self.join_children([a, b], whole, tape, mem).await;
            let old = std::mem::replace(&mut self.nodes.get_mut(id).rope, next);
            mem.release(old).await;
            Some(id)
        })
    }
    pub async fn range(&self, source: Span, tape: &Tape, mem: &mut Memory) -> Rope {
        self.read(self.root, Span::new(0, self.capacity), source, tape, mem)
            .await
    }
    fn read<'a>(
        &'a self,
        id: Option<Id>,
        whole: Span,
        source: Span,
        tape: &'a Tape,
        mem: &'a mut Memory,
    ) -> Job<'a, Rope> {
        boxed(&mem.cx.clone(), async move {
            mem.cx.charge(Op::Projection).await;
            let a = whole.start.max(source.start);
            let b = whole.end.min(source.end).min(self.available);
            if a >= b {
                return Rope::Empty;
            }
            let Some(id) = id else {
                return Self::value(Kind::Raw, Span::new(a, b), tape, mem).await;
            };
            let n = self.nodes.get(id);
            if source.start <= whole.start && whole.end <= source.end {
                return mem.retain(n.rope).await;
            }
            if let Some(tag) = n.tag {
                return Self::value(tag, Span::new(a, b), tape, mem).await;
            }
            let mid = whole.start + (whole.end - whole.start) / 2;
            let children = n.children;
            let left = self
                .read(children[0], Span::new(whole.start, mid), source, tape, mem)
                .await;
            let right = self
                .read(children[1], Span::new(mid, whole.end), source, tape, mem)
                .await;
            mem.join(left, right).await
        })
    }
    fn sync_retired(&self, mem: &Memory) {
        mem.cx.storage(
            8,
            self.retired.len() * std::mem::size_of::<(Id, usize, usize)>(),
            self.retired.len(),
            self.retired.len(),
        );
    }
    pub async fn retire(&mut self, mem: &mut Memory) {
        while !self.retired.is_empty() {
            mem.cx.charge(Op::Retire).await;
            let id = self.retired.pop_front().unwrap();
            let n = self.nodes.remove(id);
            for child in n.children.into_iter().flatten() {
                self.retired.push_back(child);
            }
            self.sync_retired(mem);
            mem.release(n.rope).await;
        }
    }
    pub fn close(&mut self, mem: &Memory) {
        if let Some(id) = self.root.take() {
            self.retired.push_back(id);
        }
        self.sync_retired(mem);
    }
}
