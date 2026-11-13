//! A balanced priority search tree: source keys partition the AVL leaves,
//! while the largest source end is promoted through separate point slots.
//! Insertion has at most two AVL rotations; each rotation repairs only O(log L)
//! heap positions. Source ends can change without changing the key skeleton.
//! A stabbing report visits O(log L + results) nodes, including crossing spans.
use crate::{
    ArenaStats, Budget, Document, DocumentId, LayoutError, LinkId, LinkInfo, Span, WorkLimit,
};
use std::mem::size_of;

#[derive(Clone, Copy)]
enum Kind {
    Leaf(LinkId),
    Branch { split: LinkId, children: [usize; 2] },
}
struct Node {
    kind: Kind,
    point: Option<LinkId>,
    first: LinkId,
    last: LinkId,
    height: u8,
}
struct Slot {
    source: Span,
    next: Option<LinkId>,
    location: Option<usize>,
    parent: Option<LinkId>,
    pending: Option<LinkId>,
    gate: crate::link_semantics::Gate,
    nested: bool,
    active: bool,
}
#[derive(Clone, Copy, Default)]
pub(crate) struct Children {
    pub(crate) head: Option<LinkId>,
    pub(crate) tail: Option<LinkId>,
}
impl std::fmt::Debug for Children {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Children")
    }
}
type LineSlotUndo = (LinkId, Option<LinkId>, Option<LinkId>);
#[derive(Default)]
pub(crate) struct LinkIndex {
    pub(crate) shape: crate::DisplayShapeEffects,
    nodes: Vec<Node>,
    slots: Vec<Slot>,
    root: Option<usize>,
    first: Option<LinkId>,
    pub(crate) steps: u64,
    allocations: u64,
    line_undo: Option<crate::rewind::UndoLog<LineSlotUndo>>,
    undo_peak: crate::rewind::UndoStats,
}
impl LinkIndex {
    pub(crate) fn undo_stats(&self) -> crate::rewind::UndoStats {
        self.undo_peak.max(self.line_undo.as_ref().map_or(
            crate::rewind::UndoStats::default(),
            crate::rewind::UndoLog::stats,
        ))
    }

    pub(crate) fn begin_line(&mut self) {
        self.shape.emit(crate::DisplayShapeEffectKind::Begin(
            crate::DisplayShapeDomain::LinkParents,
        ));
        assert!(self.line_undo.is_none());
        self.line_undo = Some(crate::rewind::UndoLog::default());
    }
    fn save_line_slot(&mut self, id: LinkId) {
        if let Some(undo) = &mut self.line_undo {
            undo.push((id, self.slots[id.0].parent, self.slots[id.0].pending));
        }
    }
    pub(crate) fn commit_line(&mut self) -> u64 {
        if self.line_undo.is_some() {
            self.shape.emit(crate::DisplayShapeEffectKind::Commit(
                crate::DisplayShapeDomain::LinkParents,
            ));
        }
        self.line_undo.take().map_or(0, |u| {
            self.undo_peak = self.undo_peak.max(u.stats());
            u.len() as u64
        })
    }
    pub(crate) fn rollback_line(&mut self) -> u64 {
        self.shape.emit(crate::DisplayShapeEffectKind::Rollback(
            crate::DisplayShapeDomain::LinkParents,
        ));
        let mut u = self.line_undo.take().unwrap();
        self.undo_peak = self.undo_peak.max(u.stats());
        let n = u.len() as u64;
        while let Some((id, parent, pending)) = u.pop() {
            self.slots[id.0].parent = parent;
            self.slots[id.0].pending = pending;
        }
        n
    }
    pub(crate) fn active(&self, id: LinkId) -> bool {
        self.slots[id.0].active
    }
    pub(crate) fn deactivate(&mut self, id: LinkId) {
        self.shape
            .emit(crate::DisplayShapeEffectKind::LinkActive { id, active: false });
        self.slots[id.0].active = false;
        self.slots[id.0].gate =
            crate::link_semantics::Gate::only(crate::link_semantics::Guard::Never);
    }

    fn push_node(&mut self, node: Node) {
        if self.nodes.len() == self.nodes.capacity() {
            self.allocations += 1;
        }
        self.nodes.push(node);
    }
    pub(crate) fn insert(&mut self, source: Span, id: LinkId) {
        self.shape
            .emit(crate::DisplayShapeEffectKind::LinkSource { id, source });
        self.shape
            .emit(crate::DisplayShapeEffectKind::LinkActive { id, active: true });
        if id.0 < self.slots.len() {
            if !self.slots[id.0].active {
                self.shape
                    .emit(crate::DisplayShapeEffectKind::LinkParent { id, parent: None });
                self.slots[id.0].parent = None;
                self.slots[id.0].pending = None;
                self.slots[id.0].gate = crate::link_semantics::Gate::default();
                self.slots[id.0].active = true;
            }
            assert_eq!(self.slots[id.0].source.start, source.start);
            self.update(id, source.end);
        } else {
            assert_eq!(id.0, self.slots.len());
            assert_eq!(self.insert_new(source), id);
        }
    }
    pub(crate) fn parent(&self, id: LinkId) -> Option<LinkId> {
        self.slots.get(id.0)?.parent
    }
    pub(crate) fn gate(&self, id: LinkId) -> crate::link_semantics::Gate {
        self.slots[id.0].gate
    }
    pub(crate) fn set_gate(&mut self, id: LinkId, gate: crate::link_semantics::Gate, nested: bool) {
        self.slots[id.0].gate = gate;
        self.slots[id.0].nested = nested;
        self.shape.emit(crate::DisplayShapeEffectKind::LinkGate {
            id,
            conditional: !gate.unconditional(),
            nested,
        });
    }
    pub(crate) fn nested(&self, id: LinkId) -> bool {
        self.slots[id.0].nested
    }
    pub(crate) fn append(&mut self, destination: &mut Children, source: Children) {
        self.steps += 1;
        if let Some(head) = source.head {
            if let Some(tail) = destination.tail {
                self.save_line_slot(tail);
                self.slots[tail.0].pending = Some(head);
            } else {
                destination.head = Some(head);
            }
            destination.tail = source.tail;
        }
    }
    pub(crate) fn adopt(&mut self, children: Children, parent: LinkId, update: &mut crate::Update) {
        let mut child = children.head;
        while let Some(id) = child {
            self.steps += 1;
            assert!(self.slots[id.0].parent.is_none());
            self.save_line_slot(id);
            child = self.slots[id.0].pending.take();
            self.slots[id.0].parent = Some(parent);
            self.shape.emit(crate::DisplayShapeEffectKind::LinkParent {
                id,
                parent: Some(parent),
            });
            update.changed_links.start = update.changed_links.start.min(id.0);
        }
    }
    pub(crate) fn len(&self) -> usize {
        self.slots.len()
    }
    pub(crate) fn next(&self, id: LinkId) -> Option<LinkId> {
        self.slots.get(id.0)?.next
    }
    pub(crate) fn stats(&self) -> ArenaStats {
        ArenaStats {
            nodes: self.nodes.len() + self.slots.len(),
            capacity_bytes: self.nodes.capacity() * size_of::<Node>()
                + self.slots.capacity() * size_of::<Slot>(),
            allocation_calls: self.allocations,
        }
    }
    fn key(&self, id: LinkId) -> (usize, usize) {
        (self.slots[id.0].source.start, id.0)
    }
    fn end(&self, id: LinkId) -> usize {
        self.slots[id.0].source.end
    }
    fn height(&self, i: usize) -> u8 {
        self.nodes[i].height
    }
    fn children(&self, i: usize) -> Option<[usize; 2]> {
        match self.nodes[i].kind {
            Kind::Leaf(_) => None,
            Kind::Branch { children, .. } => Some(children),
        }
    }
    fn take(&mut self, i: usize) -> Option<LinkId> {
        let point = self.nodes[i].point.take();
        if let Some(id) = point {
            self.slots[id.0].location = None;
        }
        point
    }
    fn set(&mut self, i: usize, id: LinkId) {
        assert!(self.nodes[i].point.is_none());
        assert!(self.slots[id.0].location.is_none());
        self.nodes[i].point = Some(id);
        self.slots[id.0].location = Some(i);
    }
    fn refresh(&mut self, i: usize) {
        self.steps += 1;
        if let Some([a, b]) = self.children(i) {
            self.nodes[i].first = self.nodes[a].first;
            self.nodes[i].last = self.nodes[b].last;
            self.nodes[i].height = 1 + self.height(a).max(self.height(b));
            self.nodes[i].kind = Kind::Branch {
                split: self.nodes[b].first,
                children: [a, b],
            };
        }
    }
    fn fill(&mut self, i: usize) {
        self.steps += 1;
        assert!(self.nodes[i].point.is_none());
        let Some(children) = self.children(i) else {
            return;
        };
        let selected = match (self.nodes[children[0]].point, self.nodes[children[1]].point) {
            (None, None) => return,
            (Some(_), None) => children[0],
            (None, Some(_)) => children[1],
            (Some(a), Some(b)) => children[usize::from(self.end(a) < self.end(b))],
        };
        let point = self.take(selected).unwrap();
        self.set(i, point);
        self.fill(selected);
    }
    fn place(&mut self, i: usize, mut id: LinkId) {
        self.steps += 1;
        let Some(old) = self.nodes[i].point else {
            self.set(i, id);
            return;
        };
        if self.end(id) > self.end(old) {
            self.take(i);
            self.set(i, id);
            id = old;
        }
        match self.nodes[i].kind {
            Kind::Branch { split, children } => {
                self.place(children[usize::from(self.key(id) >= self.key(split))], id)
            }
            Kind::Leaf(key) => panic!("duplicate heap leaf {key:?} for {id:?}"),
        }
    }
    fn rotate(&mut self, a: usize, side: usize) -> usize {
        self.steps += 1;
        let mut ac = self.children(a).unwrap();
        let b = ac[side];
        let mut bc = self.children(b).unwrap();
        let pa = self.take(a);
        let pb = self.take(b);
        ac[side] = bc[1 - side];
        bc[1 - side] = a;
        self.nodes[a].kind = Kind::Branch {
            split: self.nodes[ac[1]].first,
            children: ac,
        };
        self.nodes[b].kind = Kind::Branch {
            split: self.nodes[bc[1]].first,
            children: bc,
        };
        self.refresh(a);
        self.refresh(b);
        if let Some(id) = pa {
            self.set(b, id);
        }
        self.fill(a);
        if let Some(id) = pb {
            self.place(b, id);
        }
        b
    }
    fn insert_key(&mut self, mut i: usize, id: LinkId, before: Option<LinkId>) -> usize {
        self.steps += 1;
        match self.nodes[i].kind {
            Kind::Leaf(old) => {
                let leaf = self.nodes.len();
                self.push_node(Node {
                    kind: Kind::Leaf(id),
                    point: None,
                    first: id,
                    last: id,
                    height: 1,
                });
                let children = if self.key(id) < self.key(old) {
                    self.slots[id.0].next = Some(old);
                    if let Some(p) = before {
                        self.slots[p.0].next = Some(id);
                    } else {
                        self.first = Some(id);
                    }
                    [leaf, i]
                } else {
                    self.slots[id.0].next = self.slots[old.0].next;
                    self.slots[old.0].next = Some(id);
                    [i, leaf]
                };
                let point = self.take(i);
                let branch = self.nodes.len();
                self.push_node(Node {
                    kind: Kind::Branch {
                        split: self.nodes[children[1]].first,
                        children,
                    },
                    point: None,
                    first: self.nodes[children[0]].first,
                    last: self.nodes[children[1]].last,
                    height: 2,
                });
                if let Some(p) = point {
                    self.set(branch, p);
                }
                return branch;
            }
            Kind::Branch {
                split,
                mut children,
            } => {
                let side = usize::from(self.key(id) >= self.key(split));
                let before = if side == 1 {
                    Some(self.nodes[children[0]].last)
                } else {
                    before
                };
                children[side] = self.insert_key(children[side], id, before);
                self.nodes[i].kind = Kind::Branch { split, children };
                self.refresh(i);
            }
        }
        let children = self.children(i).unwrap();
        let a = self.height(children[0]);
        let b = self.height(children[1]);
        if a.abs_diff(b) > 1 {
            let side = usize::from(a < b);
            let j = children[side];
            let grandchildren = self.children(j).unwrap();
            if self.height(grandchildren[1 - side]) > self.height(grandchildren[side]) {
                let j = self.rotate(j, 1 - side);
                let mut children = self.children(i).unwrap();
                children[side] = j;
                self.nodes[i].kind = Kind::Branch {
                    split: self.nodes[children[1]].first,
                    children,
                };
            }
            i = self.rotate(i, side);
        }
        i
    }
    fn insert_new(&mut self, source: Span) -> LinkId {
        let id = LinkId(self.slots.len());
        if self.slots.len() == self.slots.capacity() {
            self.allocations += 1;
        }
        self.slots.push(Slot {
            source,
            next: None,
            location: None,
            parent: None,
            pending: None,
            gate: Default::default(),
            nested: false,
            active: true,
        });
        let root = if let Some(root) = self.root {
            self.insert_key(root, id, None)
        } else {
            self.first = Some(id);
            self.push_node(Node {
                kind: Kind::Leaf(id),
                point: None,
                first: id,
                last: id,
                height: 1,
            });
            0
        };
        self.root = Some(root);
        self.place(root, id);
        id
    }
    fn update(&mut self, id: LinkId, end: usize) {
        let i = self.slots[id.0].location.unwrap();
        assert_eq!(self.take(i), Some(id));
        self.fill(i);
        self.slots[id.0].source.end = end;
        self.place(self.root.unwrap(), id);
    }
    pub(crate) fn covering(&self, offset: usize) -> bool {
        self.root
            .is_some_and(|root| self.covering_node(root, 0, offset))
    }
    fn covering_node(&self, i: usize, low: usize, offset: usize) -> bool {
        if low > offset {
            return false;
        }
        let Some(point) = self.nodes[i].point else {
            return false;
        };
        if self.end(point) <= offset {
            return false;
        }
        if self.slots[point.0].source.start <= offset {
            return true;
        }
        match self.nodes[i].kind {
            Kind::Leaf(_) => false,
            Kind::Branch { split, children } => {
                self.covering_node(children[0], low, offset)
                    || self.covering_node(children[1], self.slots[split.0].source.start, offset)
            }
        }
    }
    pub(crate) fn lower_bound(&self, start: usize, visits: &mut u64) -> Option<LinkId> {
        let mut i = self.root?;
        loop {
            *visits += 1;
            match self.nodes[i].kind {
                Kind::Leaf(id) => {
                    return if self.slots[id.0].source.start >= start {
                        Some(id)
                    } else {
                        self.slots[id.0].next
                    };
                }
                Kind::Branch { split, children } => {
                    i = children[usize::from((start, 0) >= self.key(split))]
                }
            }
        }
    }
}

/// Actual metadata-node visits and fixed-word sorting work for a link window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LinkWindowWork {
    pub index_visits: u64,
    pub sort_steps: u64,
    pub objects: u64,
}

/// One paid candidate, exhaustion, or the end of the requested source window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkStep<'a> {
    Object(LinkId, &'a LinkInfo),
    BudgetExhausted,
    Complete,
}

/// Revision-bound source-order overlap cursor. Construction is O(1); locating,
/// collecting earlier covering candidates and sorting them all spend the same
/// Budget before work. Raw metadata is returned, including unresolved candidates.
/// Two O(log L) seeks plus O(K) work and O(log L + K) cursor storage. Starts
/// inside the requested range follow source-order threads without an EOF sweep.
pub struct LinkWindow {
    document: DocumentId,
    revision: u64,
    source: Span,
    phase: u8,
    stack: Vec<(usize, usize)>,
    covering: Vec<LinkId>,
    scratch: Vec<LinkId>,
    counts: [usize; 256],
    pass: usize,
    at: usize,
    small_at: usize,
    small_value: Option<LinkId>,
    next: Option<LinkId>,
    work: LinkWindowWork,
}

impl LinkWindow {
    pub fn rebind_with(
        &mut self,
        document: &Document,
        resolver: &crate::LinkResolver,
    ) -> Result<(), LayoutError> {
        if self.document != document.id() {
            return Err(LayoutError::WrongDocument);
        }
        resolver.retained(document, self.revision)?;
        self.revision = document.revision();
        Ok(())
    }

    pub(crate) fn new(document: &Document, source: Span) -> Self {
        Self {
            document: document.id(),
            revision: document.revision(),
            source,
            phase: if source.is_empty() { 9 } else { 0 },
            stack: Vec::new(),
            covering: Vec::new(),
            scratch: Vec::new(),
            counts: [0; 256],
            pass: 0,
            at: 0,
            small_at: 0,
            small_value: None,
            next: None,
            work: LinkWindowWork::default(),
        }
    }

    pub fn work(&self) -> LinkWindowWork {
        self.work
    }

    // Called only under the window's paid base query (also by cache capture).
    // At most two reported points plus the two boundary paths are inspected;
    // the unbounded remainder and all output/sorting steps stay resumable.
    pub(crate) fn seed(&mut self, document: &Document) {
        if self.phase == 9 {
            return;
        }
        assert_eq!(self.phase, 0);
        let index = &document.link_order;
        self.next = index
            .lower_bound(self.source.start, &mut self.work.index_visits)
            .filter(|id| index.slots[id.0].source.start < self.source.end);
        if self.source.start != 0
            && let Some(root) = index.root
        {
            self.stack.push((root, 0));
        }
        while !self.stack.is_empty() && self.covering.len() < 2 {
            self.collect_one(index);
        }
        self.phase = 1;
    }
    fn collect_one(&mut self, index: &LinkIndex) {
        let (i, low) = self.stack.pop().unwrap();
        self.work.index_visits += 1;
        if low >= self.source.start {
            return;
        }
        let Some(point) = index.nodes[i].point else {
            return;
        };
        if index.end(point) <= self.source.start {
            return;
        }
        if index.slots[point.0].source.start < self.source.start {
            self.covering.push(point);
        }
        if let Kind::Branch { split, children } = index.nodes[i].kind {
            let right = index.slots[split.0].source.start;
            if right < self.source.start {
                self.stack.push((children[1], right));
            }
            self.stack.push((children[0], low));
        }
    }

    pub fn advance<'a>(
        &mut self,
        document: &'a Document,
        budget: &mut Budget,
    ) -> Result<LinkStep<'a>, LayoutError> {
        self.advance_with_recovery(document, budget, 0)
    }

    pub(crate) fn advance_with_recovery<'a>(
        &mut self,
        document: &'a Document,
        budget: &mut Budget,
        recovery: usize,
    ) -> Result<LinkStep<'a>, LayoutError> {
        if self.document != document.id() {
            return Err(LayoutError::WrongDocument);
        }
        if self.revision != document.revision() {
            return Err(LayoutError::StaleContinuation);
        }
        let index = &document.link_order;
        loop {
            match self.phase {
                9 => return Ok(LinkStep::Complete),
                1 if self.stack.is_empty() => {
                    self.at = 0;
                    self.phase = if self.covering.len() <= 1 {
                        7
                    } else if self.covering.len() <= 32 {
                        self.at = 1;
                        2
                    } else {
                        3
                    };
                    continue;
                }
                2 if self.at == self.covering.len() => {
                    self.at = 0;
                    self.phase = 7;
                    continue;
                }
                3 if self.scratch.len() == self.covering.len() => {
                    self.phase = 4;
                    continue;
                }
                5 if self.at == self.covering.len() => {
                    // Exactly 256 counters: fixed work independent of the document.
                    self.phase = 6;
                    continue;
                }
                7 if self.at == self.covering.len() => {
                    self.phase = 8;
                    continue;
                }
                8 if self.next.is_none() => {
                    self.phase = 9;
                    continue;
                }
                _ => {}
            }
            if !budget.charge(WorkLimit {
                queries: usize::from(self.phase == 0),
                objects: usize::from(self.phase != 0),
                recovery,
                ..Default::default()
            }) {
                return Ok(LinkStep::BudgetExhausted);
            }
            match self.phase {
                0 => self.seed(document),
                1 => self.collect_one(index),
                2 => {
                    self.work.sort_steps += 1;
                    if let Some(value) = self.small_value {
                        if self.small_at != 0
                            && index.key(self.covering[self.small_at - 1]) > index.key(value)
                        {
                            self.covering[self.small_at] = self.covering[self.small_at - 1];
                            self.small_at -= 1;
                        } else {
                            self.covering[self.small_at] = value;
                            self.small_value = None;
                            self.at += 1;
                        }
                    } else {
                        self.small_value = Some(self.covering[self.at]);
                        self.small_at = self.at;
                    }
                }
                3 => {
                    self.work.sort_steps += 1;
                    self.scratch.push(LinkId(0));
                }
                4 => {
                    self.counts.fill(0);
                    self.work.sort_steps += 256;
                    self.at = 0;
                    self.phase = 5;
                }
                5 => {
                    let value = self.byte(index, self.covering[self.at]);
                    self.counts[value] += 1;
                    self.at += 1;
                    self.work.sort_steps += 1;
                }
                6 => {
                    // Prefixing is another fixed 256-counter step. The phase
                    // reuses small_value to distinguish prefix and scatter.
                    if self.small_value.is_none() {
                        let mut sum = 0;
                        for count in &mut self.counts {
                            let old = *count;
                            *count = sum;
                            sum += old;
                        }
                        self.work.sort_steps += 256;
                        self.at = 0;
                        self.small_value = Some(LinkId(0));
                    } else {
                        let id = self.covering[self.at];
                        let byte = self.byte(index, id);
                        self.scratch[self.counts[byte]] = id;
                        self.counts[byte] += 1;
                        self.at += 1;
                        self.work.sort_steps += 1;
                        if self.at == self.covering.len() {
                            std::mem::swap(&mut self.covering, &mut self.scratch);
                            self.small_value = None;
                            self.pass += 1;
                            self.at = 0;
                            self.phase = if self.pass == 2 * size_of::<usize>() {
                                7
                            } else {
                                4
                            };
                        }
                    }
                }
                7 => {
                    let id = self.covering[self.at];
                    self.at += 1;
                    self.work.objects += 1;
                    return Ok(LinkStep::Object(id, &document.link_info[id.0]));
                }
                8 => {
                    if let Some(id) = self.next {
                        self.work.index_visits += 1;
                        self.next = index
                            .next(id)
                            .filter(|next| index.slots[next.0].source.start < self.source.end);
                        self.work.objects += 1;
                        return Ok(LinkStep::Object(id, &document.link_info[id.0]));
                    } else {
                        self.phase = 9;
                    }
                }
                _ => unreachable!(),
            }
        }
    }

    fn byte(&self, index: &LinkIndex, id: LinkId) -> usize {
        let width = size_of::<usize>();
        let key = if self.pass < width {
            id.0
        } else {
            index.slots[id.0].source.start
        };
        (key >> (8 * (self.pass % width))) & 255
    }
}

/// Borrowed source-order link/image candidates. Use `advance` to pay the
/// ancestor/covering prepass before the first object; Iterator is the explicit
/// unlimited-budget convenience path.
pub struct LinkObjects<'a> {
    document: &'a Document,
    cursor: LinkWindow,
}
impl<'a> LinkObjects<'a> {
    pub(crate) fn new(document: &'a Document, source: Span) -> Self {
        Self {
            document,
            cursor: LinkWindow::new(document, source),
        }
    }
    pub fn work(&self) -> LinkWindowWork {
        self.cursor.work()
    }
    pub fn advance(&mut self, budget: &mut Budget) -> LinkStep<'a> {
        self.cursor
            .advance(self.document, budget)
            .expect("borrowed document cannot change")
    }
}
impl<'a> Iterator for LinkObjects<'a> {
    type Item = (LinkId, &'a LinkInfo);
    fn next(&mut self) -> Option<Self::Item> {
        match self.advance(&mut Budget::new(WorkLimit::unlimited())) {
            LinkStep::Object(id, info) => Some((id, info)),
            LinkStep::Complete => None,
            LinkStep::BudgetExhausted => unreachable!(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LinkKind, LinkTarget, SyntaxState};
    fn add(doc: &mut Document, source: Span) -> LinkId {
        let id = LinkId(doc.links.len());
        doc.links.push(LinkTarget::Direct(Span::new(0, 0)));
        doc.link_info.push(LinkInfo {
            kind: LinkKind::Image,
            label: Span::new(source.start, source.start),
            source,
            title: None,
            state: SyntaxState::Sealed,
        });
        doc.link_order.insert(source, id);
        id
    }
    fn random(seed: &mut u64) -> usize {
        *seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (*seed >> 32) as usize
    }
    fn expected(doc: &Document, source: Span) -> Vec<LinkId> {
        let mut out: Vec<_> = doc
            .link_info
            .iter()
            .enumerate()
            .filter_map(|(i, info)| {
                (!source.is_empty()
                    && info.source.start < source.end
                    && source.start < info.source.end)
                    .then_some(LinkId(i))
            })
            .collect();
        out.sort_unstable_by_key(|id| (doc.link_info[id.0].source.start, id.0));
        out
    }
    fn paid(doc: &Document, source: Span, allowance: usize) -> (Vec<LinkId>, LinkWindowWork) {
        let mut cursor = doc.link_window(source);
        let mut out = Vec::new();
        let limit = WorkLimit {
            queries: allowance,
            objects: allowance,
            ..Default::default()
        };
        let mut calls = 0;
        loop {
            let mut budget = Budget::new(limit);
            match cursor.advance(doc, &mut budget).unwrap() {
                LinkStep::Object(id, info) => {
                    assert_eq!(doc.link_info(id), Some(info));
                    out.push(id);
                }
                LinkStep::Complete => break,
                LinkStep::BudgetExhausted => assert_ne!(budget.used(), WorkLimit::default()),
            }
            assert!(budget.used().objects <= allowance && budget.used().queries <= allowance);
            calls += 1;
            assert!(calls < 400 * (doc.links.len() + 1) + 100);
        }
        (out, cursor.work())
    }
    #[test]
    fn arbitrary_crossing_windows_updates_and_tied_starts_match_flat_oracle() {
        let mut seed = 371923;
        let mut windows = 0;
        for count in [1, 3, 30, 100, 300] {
            let mut doc = Document::new();
            for step in 0..count * 2 {
                if step < count {
                    let start = random(&mut seed) % (count * 20);
                    let end = start + 1 + random(&mut seed) % (count * 20);
                    add(&mut doc, Span::new(start, end));
                } else {
                    let id = LinkId(random(&mut seed) % count);
                    let start = doc.link_info[id.0].source.start;
                    let source = Span::new(start, start + 1 + random(&mut seed) % (count * 20));
                    doc.link_info[id.0].source = source;
                    doc.link_order.insert(source, id);
                }
                for _ in 0..8 {
                    let start = random(&mut seed) % (count * 40);
                    let source = Span::new(start, start + random(&mut seed) % (count * 10 + 1));
                    let want = expected(&doc, source);
                    assert_eq!(
                        doc.links_in(source).map(|(id, _)| id).collect::<Vec<_>>(),
                        want
                    );
                    for allowance in [1, 6] {
                        let (actual, work) = paid(&doc, source, allowance);
                        assert_eq!(actual, want);
                        let height = doc.link_order.height(doc.link_order.root.unwrap()) as u64;
                        assert!(work.index_visits <= 6 * (want.len() as u64 + height + 1));
                        assert!(work.sort_steps <= 400 * (want.len() as u64 + 1));
                    }
                    windows += 1;
                }
            }
        }
        eprintln!("{windows} exact source-order overlap windows with 1/6 budgets");
    }
    #[test]
    fn sparse_deep_chain_dense_history_zero_budget_and_staleness() {
        let mut doc = Document::new();
        let depth = 256;
        let width = 256;
        let cut = depth * (width + 1) + 2;
        for level in 0..depth {
            let start = level * (width + 1);
            add(&mut doc, Span::new(start, cut + depth - level));
            for j in 1..=width {
                add(&mut doc, Span::new(start + j, start + j + 1));
            }
        }
        let span = Span::new(cut, cut + 1);
        let mut cursor = doc.link_window(span);
        assert_eq!(cursor.work(), LinkWindowWork::default());
        let mut zero = Budget::new(WorkLimit::default());
        assert_eq!(
            cursor.advance(&doc, &mut zero),
            Ok(LinkStep::BudgetExhausted)
        );
        assert_eq!(cursor.work(), LinkWindowWork::default());
        let (out, work) = paid(&doc, span, 1);
        assert_eq!(out, expected(&doc, span));
        assert_eq!(out.len(), depth);
        assert!(work.index_visits < 4 * (depth as u64 + 24));
        assert!(work.sort_steps <= 400 * depth as u64);
        let other = Document::new();
        assert_eq!(
            cursor.advance(&other, &mut zero),
            Err(LayoutError::WrongDocument)
        );
        doc.append("x").unwrap();
        assert_eq!(
            cursor.advance(&doc, &mut zero),
            Err(LayoutError::StaleContinuation)
        );
        assert_eq!(doc.link_order.len(), doc.links.len());
    }
}
