#[cfg(test)]
use crate::Reference;
use crate::references::Lookup;
use crate::{LinkId, LinkTarget, Span, Style};
use std::num::NonZeroUsize;

#[cfg(test)]
#[path = "../tests/support/annotation_split.rs"]
mod split_experiment;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Mark {
    pub style: Style,
    pub(crate) base_hidden: bool,
    pub(crate) delimiter_hidden: bool,
    pub emphasis_depth: usize,
    pub strong_depth: usize,
    pub link: Option<LinkId>,
    pub hide_when_target_available: Option<NonZeroUsize>,
    pub hide_alternative: Option<NonZeroUsize>,
    link_syntax: bool,
    pub revision: u64,
    // Several link rewrites may occur in one append/revision.
    serial: u64,
}

impl Mark {
    pub(crate) fn hide_owner(self) -> Option<LinkId> {
        self.hide_when_target_available
            .map(|id| LinkId(id.get() - 1))
    }
    pub(crate) fn alternative_owner(self) -> Option<LinkId> {
        self.hide_alternative.map(|id| LinkId(id.get() - 1))
    }
    pub fn effective_style<R: Lookup + ?Sized>(
        self,
        references: &R,
        links: &[LinkTarget],
    ) -> Style {
        if !self.style.contains(Style::HIDDEN)
            && self.hide_owner().is_some_and(|id| {
                target_available(id, references, links, &mut VisibilityWork::default())
            })
        {
            self.style.union(Style::HIDDEN)
        } else {
            self.style
        }
    }

    fn merge(self, other: Self) -> Self {
        // Only one published bracket owns any marker byte. Reused first-cell
        // candidates and qualified reference suffixes keep that stable ID. Unpublished header observers never install a gate.
        let hide_when_target_available = match (
            self.hide_when_target_available,
            other.hide_when_target_available,
        ) {
            (Some(a), Some(b)) => {
                assert_eq!(a, b, "overlapping shortcut marker owners");
                Some(a)
            }
            (a, b) => a.or(b),
        };
        // Bracket children are published before their enclosing label. Late
        // bare objects have disjoint label spans. Thus allocation priority is
        // sufficient ONLY for overlapping syntax labels, not for source order.
        let replace = other.link.is_some()
            && if self.link_syntax && other.link_syntax {
                other.link < self.link || (other.link == self.link && other.serial >= self.serial)
            } else {
                other.serial >= self.serial
            };
        let link = if replace { other.link } else { self.link };
        Self {
            style: self.style.union(other.style),
            base_hidden: self.base_hidden || other.base_hidden,
            delimiter_hidden: self.delimiter_hidden || other.delimiter_hidden,
            emphasis_depth: self.emphasis_depth + other.emphasis_depth,
            strong_depth: self.strong_depth + other.strong_depth,
            link,
            hide_when_target_available,
            hide_alternative: match (self.hide_alternative, other.hide_alternative) {
                (Some(a), Some(b)) => {
                    assert_eq!(a, b);
                    Some(a)
                }
                (a, b) => a.or(b),
            },
            link_syntax: if replace {
                other.link_syntax
            } else {
                self.link_syntax
            },
            revision: self.revision.max(other.revision),
            serial: self.serial.max(other.serial),
        }
    }
}

type LineCheckpoint = (
    usize,
    usize,
    usize,
    u64,
    crate::rewind::UndoLog<(usize, Node)>,
);

#[derive(Clone, Copy, Default)]
struct Node {
    saved_epoch: u64,
    children: [Option<NonZeroUsize>; 2],
    tag: Mark,
    newest: u64,
    hidden: bool,
}

impl Node {
    fn child(&self, side: usize) -> Option<usize> {
        self.children[side].map(|index| index.get() - 1)
    }
}

/// Dynamic binary segment tree over source bytes. Tags stay on their covering
/// node; updating a huge interval never visits its interior. No recursive AST.
pub(crate) struct Annotations {
    #[cfg(test)]
    experiment: split_experiment::Probe,
    pub(crate) shape: crate::DisplayShapeEffects,
    nodes: Vec<Node>,
    root: usize,
    capacity: usize,
    serial: u64,
    pub visits: u64,
    allocations: u64,
    checkpoint: Option<LineCheckpoint>,
    line_epoch: u64,
    undo_peak: crate::rewind::UndoStats,
}

impl Default for Annotations {
    fn default() -> Self {
        Self {
            #[cfg(test)]
            experiment: split_experiment::Probe::default(),
            shape: crate::DisplayShapeEffects::default(),
            nodes: vec![Node::default()],
            root: 0,
            capacity: 1,
            serial: 0,
            visits: 0,
            allocations: 1,
            checkpoint: None,
            line_epoch: 0,
            undo_peak: crate::rewind::UndoStats::default(),
        }
    }
}

impl Annotations {
    pub(crate) fn undo_stats(&self) -> crate::rewind::UndoStats {
        self.undo_peak.max(
            self.checkpoint
                .as_ref()
                .map_or(crate::rewind::UndoStats::default(), |c| c.4.stats()),
        )
    }

    pub(crate) fn begin_line(&mut self) {
        #[cfg(test)]
        self.experiment.record(split_experiment::Op::Begin);
        self.shape.emit(crate::DisplayShapeEffectKind::Begin(
            crate::DisplayShapeDomain::Annotations,
        ));
        assert!(self.checkpoint.is_none());
        self.line_epoch += 1;
        self.checkpoint = Some((
            self.nodes.len(),
            self.root,
            self.capacity,
            self.serial,
            crate::rewind::UndoLog::default(),
        ));
    }
    pub(crate) fn commit_line(&mut self) -> u64 {
        #[cfg(test)]
        self.experiment.record(split_experiment::Op::Commit);
        self.checkpoint.take().map_or(0, |(_, _, _, _, undo)| {
            self.shape.emit(crate::DisplayShapeEffectKind::Commit(
                crate::DisplayShapeDomain::Annotations,
            ));
            self.undo_peak = self.undo_peak.max(undo.stats());
            undo.len() as u64
        })
    }
    pub(crate) fn rollback_line(&mut self) -> u64 {
        #[cfg(test)]
        self.experiment.record(split_experiment::Op::Rollback);
        let (len, root, capacity, serial, mut undo) = self.checkpoint.take().unwrap();
        self.shape.emit(crate::DisplayShapeEffectKind::Rollback(
            crate::DisplayShapeDomain::Annotations,
        ));
        self.undo_peak = self.undo_peak.max(undo.stats());
        let count = undo.len() as u64;
        while let Some((id, node)) = undo.pop() {
            self.nodes[id] = node;
        }
        self.nodes.truncate(len);
        self.root = root;
        self.capacity = capacity;
        self.serial = serial;
        count
    }

    pub fn apply(&mut self, span: Span, style: Style, link: Option<LinkId>, revision: u64) {
        self.apply_mark(
            span,
            Mark {
                style,
                base_hidden: style.contains(Style::HIDDEN),
                emphasis_depth: usize::from(style.contains(Style::EMPHASIS)),
                strong_depth: usize::from(style.contains(Style::STRONG)),
                link,
                revision,
                ..Mark::default()
            },
        );
    }

    pub(crate) fn delimiter(&mut self, span: Span, style: Style, revision: u64) {
        self.apply_mark(
            span,
            Mark {
                style,
                delimiter_hidden: style.contains(Style::HIDDEN),
                emphasis_depth: usize::from(style.contains(Style::EMPHASIS)),
                strong_depth: usize::from(style.contains(Style::STRONG)),
                revision,
                ..Mark::default()
            },
        );
    }

    pub(crate) fn apply_object(&mut self, span: Span, style: Style, id: LinkId, revision: u64) {
        self.apply_mark(
            span,
            Mark {
                style,
                link: Some(id),
                link_syntax: true,
                revision,
                emphasis_depth: usize::from(style.contains(Style::EMPHASIS)),
                strong_depth: usize::from(style.contains(Style::STRONG)),
                ..Mark::default()
            },
        );
    }
    pub(crate) fn hide_when_either(
        &mut self,
        span: Span,
        id: LinkId,
        alternative: LinkId,
        revision: u64,
    ) {
        self.apply_mark(
            span,
            Mark {
                hide_when_target_available: NonZeroUsize::new(id.0 + 1),
                hide_alternative: NonZeroUsize::new(alternative.0 + 1),
                revision,
                ..Mark::default()
            },
        );
    }
    pub fn hide_when_resolved(&mut self, span: Span, id: LinkId, revision: u64) {
        self.apply_mark(
            span,
            Mark {
                hide_when_target_available: NonZeroUsize::new(id.0 + 1),
                revision,
                ..Mark::default()
            },
        );
    }

    fn apply_mark(&mut self, span: Span, mut mark: Mark) {
        if span.is_empty() {
            return;
        }
        // Callers supply source String endpoints or validated HeightIndex
        // endpoints (<= isize::MAX); doubling can reach but not exceed its +1.
        while self.capacity < span.end {
            let root = self.nodes.len();
            self.push_node(Node {
                children: [NonZeroUsize::new(self.root + 1), None],
                newest: self.nodes[self.root].newest,
                ..Node::default()
            });
            self.root = root;
            self.capacity *= 2;
        }
        self.serial += 1;
        mark.serial = if mark.link.is_some() { self.serial } else { 0 };
        #[cfg(test)]
        self.experiment
            .record(split_experiment::Op::Apply(span, mark));
        self.shape.emit(crate::DisplayShapeEffectKind::Mark {
            source: span,
            mark: crate::DisplayShapeMark {
                style: mark.style,
                base_hidden: mark.base_hidden,
                delimiter_hidden: mark.delimiter_hidden,
                emphasis_depth: mark.emphasis_depth,
                strong_depth: mark.strong_depth,
                link: mark.link,
                hide_owner: mark.hide_owner(),
                hide_alternative: mark.alternative_owner(),
                object_syntax: mark.link_syntax,
                order: mark.serial,
            },
        });
        self.update(self.root, 0, self.capacity, span, mark);
    }

    fn update(&mut self, id: usize, lo: usize, hi: usize, span: Span, mark: Mark) {
        self.visits += 1;
        if let Some((len, _, _, _, undo)) = &mut self.checkpoint
            && id < *len
            && self.nodes[id].saved_epoch != self.line_epoch
        {
            #[cfg(test)]
            let old_bytes = undo.stats().bytes;
            undo.push((id, self.nodes[id]));
            #[cfg(test)]
            {
                self.experiment.undo_requests += u64::from(undo.stats().bytes > old_bytes);
            }
            self.nodes[id].saved_epoch = self.line_epoch;
        }
        self.nodes[id].newest = self.nodes[id].newest.max(mark.revision);
        if span.start <= lo && hi <= span.end {
            #[cfg(test)]
            {
                self.experiment.tag_writes += 1;
            }
            self.nodes[id].tag = self.nodes[id].tag.merge(mark);
            self.nodes[id].hidden |= mark.style.contains(Style::HIDDEN);
            return;
        }
        let mid = lo + (hi - lo) / 2;
        for (side, start, end) in [(0, lo, mid), (1, mid, hi)] {
            if span.start < end && start < span.end {
                let child = match self.nodes[id].child(side) {
                    Some(child) => child,
                    None => {
                        let child = self.nodes.len();
                        self.push_node(Node::default());
                        self.nodes[id].children[side] = NonZeroUsize::new(child + 1);
                        child
                    }
                };
                self.update(child, start, end, span, mark);
            }
        }
        self.nodes[id].hidden |= self.nodes[id]
            .children
            .iter()
            .all(|child| child.is_some_and(|child| self.nodes[child.get() - 1].hidden));
    }

    #[cfg(test)]
    pub fn next_visible(&self, offset: usize, end: usize) -> Option<usize> {
        self.next_visible_resolving(offset, end, &[], &[])
    }

    pub fn next_visible_resolving<R: Lookup + ?Sized>(
        &self,
        offset: usize,
        end: usize,
        references: &R,
        links: &[LinkTarget],
    ) -> Option<usize> {
        self.visible_query(
            offset,
            end,
            (references, links),
            &mut VisibilityWork::default(),
        )
    }

    fn visible_query<R: Lookup + ?Sized>(
        &self,
        offset: usize,
        end: usize,
        targets: (&R, &[LinkTarget]),
        work: &mut VisibilityWork,
    ) -> Option<usize> {
        if offset >= end {
            return None;
        }
        if offset >= self.capacity {
            return Some(offset);
        }
        self.find_visible(
            self.root,
            Span::new(0, self.capacity),
            Span::new(offset, end),
            targets,
            work,
        )
        .or_else(|| (self.capacity < end).then_some(self.capacity))
    }

    fn find_visible<R: Lookup + ?Sized>(
        &self,
        id: usize,
        span: Span,
        query: Span,
        targets: (&R, &[LinkTarget]),
        work: &mut VisibilityWork,
    ) -> Option<usize> {
        self.find_visible_by(id, span, query, work, &mut |mark, work| {
            if let Some(owner) = mark.hide_owner() {
                work.gate();
                target_available(owner, targets.0, targets.1, work)
            } else {
                false
            }
        })
    }
    pub(crate) fn next_visible_using(
        &self,
        offset: usize,
        end: usize,
        hidden: &mut impl FnMut(Mark) -> bool,
    ) -> Option<usize> {
        if offset >= end {
            return None;
        }
        if offset >= self.capacity {
            return Some(offset);
        }
        self.find_visible_by(
            self.root,
            Span::new(0, self.capacity),
            Span::new(offset, end),
            &mut VisibilityWork::default(),
            &mut |mark, _| hidden(mark),
        )
        .or_else(|| (self.capacity < end).then_some(self.capacity))
    }
    fn find_visible_by(
        &self,
        id: usize,
        span: Span,
        query: Span,
        work: &mut VisibilityWork,
        hidden: &mut impl FnMut(Mark, &mut VisibilityWork) -> bool,
    ) -> Option<usize> {
        work.node();
        let (lo, hi) = (span.start, span.end);
        let (offset, end) = (query.start, query.end);
        if self.nodes[id].hidden || offset >= hi || end <= lo {
            return None;
        }
        if hidden(self.nodes[id].tag, work) {
            return None;
        }
        if hi - lo == 1 {
            return Some(lo);
        }
        let mid = lo + (hi - lo) / 2;
        for (side, start, stop) in [(0, lo, mid), (1, mid, hi)] {
            if offset < stop && start < end {
                let found = match self.nodes[id].child(side) {
                    Some(child) => {
                        self.find_visible_by(child, Span::new(start, stop), query, work, hidden)
                    }
                    None => Some(start.max(offset)),
                };
                if found.is_some() {
                    return found;
                }
            }
        }
        None
    }

    #[cfg(test)]
    pub fn at(&self, offset: usize) -> Mark {
        self.current_leaf(offset, usize::MAX).1
    }

    /// One root-to-leaf query, without the public Runs iterator's unbounded
    /// adjacent-run coalescing. Consumers pay each returned metadata fragment.
    pub(crate) fn current_leaf(&self, offset: usize, end: usize) -> (Span, Mark) {
        if offset >= self.capacity {
            return (Span::new(offset, end), Mark::default());
        }
        let (mut id, mut lo, mut hi) = (self.root, 0, self.capacity);
        let mut mark = Mark::default();
        loop {
            let node = &self.nodes[id];
            #[cfg(test)]
            self.experiment.lookup.set(self.experiment.lookup.get() + 1);
            mark = mark.merge(node.tag);
            let mid = lo + (hi - lo) / 2;
            let side = usize::from(offset >= mid);
            if node.children == [None; 2] {
                return (Span::new(offset, end.min(hi)), mark);
            }
            match node.child(side) {
                Some(child) => {
                    id = child;
                    if side == 0 {
                        hi = mid;
                    } else {
                        lo = mid;
                    }
                }
                None => {
                    return (
                        Span::new(offset, end.min(if side == 0 { mid } else { hi })),
                        mark,
                    );
                }
            }
        }
    }

    pub fn newest(&self, span: Span) -> u64 {
        self.query_revision(self.root, 0, self.capacity, span)
    }

    fn query_revision(&self, id: usize, lo: usize, hi: usize, span: Span) -> u64 {
        if span.start >= hi || span.end <= lo {
            return 0;
        }
        let node = &self.nodes[id];
        #[cfg(test)]
        self.experiment
            .revision
            .set(self.experiment.revision.get() + 1);
        if span.start <= lo && hi <= span.end {
            return node.newest;
        }
        let mid = lo + (hi - lo) / 2;
        let mut revision = node.tag.revision;
        for (side, start, end) in [(0, lo, mid), (1, mid, hi)] {
            if let Some(child) = node.child(side) {
                revision = revision.max(self.query_revision(child, start, end, span));
            }
        }
        revision
    }

    pub fn arena_stats(&self) -> crate::ArenaStats {
        crate::ArenaStats {
            nodes: self.nodes.len(),
            capacity_bytes: self.nodes.capacity() * std::mem::size_of::<Node>(),
            allocation_calls: self.allocations,
        }
    }

    fn push_node(&mut self, node: Node) {
        if self.nodes.len() == self.nodes.capacity() {
            self.allocations += 1;
        }
        self.nodes.push(node);
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn segments(&self, span: Span) -> Segments<'_> {
        self.segments_with_scratch(span, QueryScratch::default())
    }

    pub fn segments_with_scratch(&self, span: Span, mut scratch: QueryScratch) -> Segments<'_> {
        scratch.stack.clear();
        let initial_allocations = scratch.allocations;
        if span.end > self.capacity && !span.is_empty() {
            scratch.push(Frame {
                node: None,
                span: Span::new(span.start.max(self.capacity), span.end),
                inherited: Mark::default(),
            });
        }
        if span.start < self.capacity && !span.is_empty() {
            scratch.push(Frame {
                node: Some(self.root),
                span: Span::new(0, self.capacity),
                inherited: Mark::default(),
            });
        }
        Segments {
            tree: self,
            query: span,
            initial_allocations,
            scratch,
            visits: 0,
        }
    }
}

struct Frame {
    node: Option<usize>,
    span: Span,
    inherited: Mark,
}

// Private query instrumentation; erased in non-test builds.
#[derive(Default)]
struct VisibilityWork {
    #[cfg(test)]
    nodes: usize,
    #[cfg(test)]
    gates: usize,
    #[cfg(test)]
    links: usize,
    #[cfg(test)]
    symbols: usize,
}

impl VisibilityWork {
    fn node(&mut self) {
        #[cfg(test)]
        {
            self.nodes += 1;
        }
    }
    fn gate(&mut self) {
        #[cfg(test)]
        {
            self.gates += 1;
        }
    }
    fn link(&mut self) {
        #[cfg(test)]
        {
            self.links += 1;
        }
    }
    fn symbol(&mut self) {
        #[cfg(test)]
        {
            self.symbols += 1;
        }
    }
}

// Gate owners follow the object's current target, including same-ID upgrades.
// Callers short-circuit permanent hiding before entering this predicate.
fn target_available<R: Lookup + ?Sized>(
    id: LinkId,
    references: &R,
    links: &[LinkTarget],
    work: &mut VisibilityWork,
) -> bool {
    work.link();
    match links[id.0] {
        LinkTarget::Direct(_) | LinkTarget::Email(_) | LinkTarget::Www(_) => true,
        LinkTarget::Reference(symbol) => {
            work.symbol();
            references.resolved(symbol)
        }
    }
}

/// Reusable annotation traversal storage for semantic and display queries.
///
/// Holds no document references. A query exclusively borrows this buffer and
/// returns its capacity on drop, even after partial iteration or unwinding.
/// Returned text only borrows the document, so it may outlive that query. The
/// buffer can be reused after document mutation or with another document; no
/// old traversal is resumed. Storage grows with tree depth, not result count,
/// and is retained until this value is dropped (or replaced with Default).
///
/// A live iterator exclusively borrows scratch:
/// ```compile_fail
/// use mdstream::{Document, QueryScratch, Span};
/// let doc = Document::new();
/// let mut scratch = QueryScratch::default();
/// let mut first = doc.runs_in_with_scratch(Span::new(0, 0), &mut scratch).unwrap();
/// let second = doc.display_runs_in_with_scratch(Span::new(0, 0), &mut scratch);
/// first.next();
/// ```
/// The document remains immutably borrowed during traversal:
/// ```compile_fail
/// use mdstream::{Document, QueryScratch, Span};
/// let mut doc = Document::new();
/// let mut scratch = QueryScratch::default();
/// let mut runs = doc.display_runs_in_with_scratch(Span::new(0, 0), &mut scratch).unwrap();
/// doc.append("changed").unwrap();
/// runs.next();
/// ```
#[derive(Default)]
pub struct QueryScratch {
    stack: Vec<Frame>,
    allocations: u64,
}

impl QueryScratch {
    /// O(1) retained capacity and cumulative allocation/growth requests since
    /// construction. `nodes` is zero between queries; active queries report
    /// their pending frames and per-query allocations via `scratch_stats`.
    /// Excludes allocator overhead and all storage other than this Vec.
    pub fn stats(&self) -> crate::ArenaStats {
        crate::ArenaStats {
            nodes: self.stack.len(),
            capacity_bytes: self.stack.capacity() * std::mem::size_of::<Frame>(),
            allocation_calls: self.allocations,
        }
    }

    fn push(&mut self, frame: Frame) {
        if self.stack.len() == self.stack.capacity() {
            self.allocations += 1;
        }
        self.stack.push(frame);
    }
}

/// Ordered constant-mark intervals. Each relevant tree node is visited once,
/// including absent children (uniform inherited tags), with O(log n) state.
/// Internal intervals may split UTF-8; the public run iterator coalesces them.
pub(crate) struct Segments<'a> {
    tree: &'a Annotations,
    query: Span,
    scratch: QueryScratch,
    initial_allocations: u64,
    pub visits: u64,
}

impl Segments<'_> {
    pub fn scratch_stats(&self) -> crate::ArenaStats {
        crate::ArenaStats {
            allocation_calls: self.scratch.allocations - self.initial_allocations,
            ..self.scratch.stats()
        }
    }

    pub fn recycle_into(&mut self, scratch: &mut QueryScratch) {
        self.scratch.stack.clear();
        *scratch = std::mem::take(&mut self.scratch);
    }
}

impl Iterator for Segments<'_> {
    type Item = (Span, Mark);

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(frame) = self.scratch.stack.pop() {
            self.visits += 1;
            let mut mark = frame.inherited;
            if let Some(id) = frame.node {
                let node = &self.tree.nodes[id];
                mark = mark.merge(node.tag);
                if node.children != [None; 2] {
                    let mid = frame.span.start + frame.span.len() / 2;
                    for (side, start, end) in [(1, mid, frame.span.end), (0, frame.span.start, mid)]
                    {
                        if start < self.query.end && self.query.start < end {
                            self.scratch.push(Frame {
                                node: node.child(side),
                                span: Span::new(start, end),
                                inherited: mark,
                            });
                        }
                    }
                    continue;
                }
            }
            return Some((
                Span::new(
                    frame.span.start.max(self.query.start),
                    frame.span.end.min(self.query.end),
                ),
                mark,
            ));
        }
        None
    }
}

/// Read-only current annotation seed. The saved physical-line checkpoint is
/// emitted separately, so a late subscriber can honor its next real rollback.
pub(crate) struct ShapeSeed {
    undo_at: usize,
    old: std::collections::BTreeMap<usize, Node>,
    stack: Vec<(usize, Span)>,
    phase: u8,
    pub(crate) work: crate::display_shape::DisplayShapeSeedWork,
}
impl ShapeSeed {
    pub(crate) fn new() -> Self {
        Self {
            undo_at: 0,
            old: Default::default(),
            stack: Vec::new(),
            phase: 0,
            work: Default::default(),
        }
    }
    pub(crate) fn capacity_bytes(&self) -> usize {
        self.stack.capacity() * std::mem::size_of::<(usize, Span)>()
            + self.old.len() * (std::mem::size_of::<(usize, Node)>() + 256)
    }
    pub(crate) fn retire(&mut self, budget: &mut crate::Budget) -> crate::Completion {
        self.phase = 3;
        while !self.old.is_empty() || !self.stack.is_empty() {
            if !budget.charge(crate::WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            }) {
                return crate::Completion::BudgetExhausted;
            }
            self.work.steps += 1;
            self.work.retired_nodes += 1;
            if self.old.pop_first().is_none() {
                self.stack.pop();
            }
        }
        crate::Completion::Covered
    }
    pub(crate) fn next(
        &mut self,
        annotations: &Annotations,
        budget: &mut crate::Budget,
    ) -> Result<Option<crate::display_shape::DisplayShapeSeedItem>, crate::Completion> {
        use crate::display_shape::DisplayShapeSeedItem as Item;
        loop {
            if !budget.charge(crate::WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            }) {
                return Err(crate::Completion::BudgetExhausted);
            }
            self.work.steps += 1;
            if self.phase == 0 {
                if let Some((_, _, _, _, undo)) = &annotations.checkpoint {
                    if let Some(&(id, node)) = undo.get(self.undo_at) {
                        self.old.insert(id, node);
                        self.undo_at += 1;
                        self.work.undo_copies += 1;
                        continue;
                    }
                    let (_, root, capacity, _, _) = annotations.checkpoint.as_ref().unwrap();
                    self.stack.push((*root, Span::new(0, *capacity)));
                    self.phase = 1;
                    return Ok(Some(Item::AnnotationCheckpoint));
                }
                self.phase = 2;
                self.stack
                    .push((annotations.root, Span::new(0, annotations.capacity)));
            }
            if let Some((id, source)) = self.stack.pop() {
                self.work.annotation_nodes += 1;
                let baseline = self.phase == 1;
                let node = if baseline {
                    self.old.get(&id).copied().unwrap_or(annotations.nodes[id])
                } else {
                    annotations.nodes[id]
                };
                let mid = source.start + (source.len() / 2);
                for side in (0..2).rev() {
                    if let Some(child) = node.child(side) {
                        self.stack.push((
                            child,
                            if side == 0 {
                                Span::new(source.start, mid)
                            } else {
                                Span::new(mid, source.end)
                            },
                        ));
                    }
                }
                let mark = node.tag;
                if mark != Mark::default() {
                    return Ok(Some(Item::Mark {
                        source,
                        baseline,
                        mark: crate::DisplayShapeMark {
                            style: mark.style,
                            base_hidden: mark.base_hidden,
                            delimiter_hidden: mark.delimiter_hidden,
                            emphasis_depth: mark.emphasis_depth,
                            strong_depth: mark.strong_depth,
                            link: mark.link,
                            hide_owner: mark.hide_owner(),
                            hide_alternative: mark.alternative_owner(),
                            object_syntax: mark.link_syntax,
                            order: mark.serial,
                        },
                    }));
                }
                continue;
            }
            if self.phase == 1 {
                self.phase = 2;
                self.stack
                    .push((annotations.root, Span::new(0, annotations.capacity)));
                continue;
            }
            if self.old.pop_first().is_some() {
                self.work.retired_nodes += 1;
                continue;
            }
            return Ok(None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_visibility_grammar_witness_and_logarithmic_query_work() {
        // A nonempty normalized label leaves a base-visible scalar: escapes
        // retain punctuation/LF, emphasis retains content, all-space code/math
        // retains a space. Autolinks, footnotes and nested openers replace the
        // bracket; overflow/blank labels publish no shortcut. end_label freezes
        // internal delimiters before later syntax can consume that witness.
        let labels = [
            "a", "中🙂", "*a*", "a\\]", "\\\n", "\\\r\n", "\\\n\\\n", "$ $", "` `", "** **",
            "\t\n\\\n", "\\\n  ",
        ];
        for prefix in ["", "> ", "- ", "# ", "| "] {
            for label in labels {
                let mut doc = crate::Document::new();
                doc.append(&format!("{prefix}[{label}]![{label}]***\n"))
                    .unwrap();
                doc.finish();
                for info in &doc.link_info {
                    assert!(
                        doc.annotations
                            .segments(info.label)
                            .any(|(_, m)| !m.style.contains(Style::HIDDEN))
                    );
                }
                // Resolve in place only for this private grammar query: actual
                // definition protocols and revision are integration-tested.
                for reference in &mut doc.references.entries {
                    reference.destination = Some(Span::new(0, 0));
                }
                let visible: Vec<_> = doc
                    .source
                    .char_indices()
                    .filter_map(|(at, _)| {
                        (!doc
                            .annotations
                            .at(at)
                            .effective_style(&doc.references.entries, &doc.links)
                            .contains(Style::HIDDEN))
                        .then_some(at)
                    })
                    .collect();
                for (at, _) in doc.source.char_indices() {
                    let mut work = VisibilityWork::default();
                    let actual = doc.annotations.visible_query(
                        at,
                        doc.source.len(),
                        (&doc.references.entries, &doc.links),
                        &mut work,
                    );
                    assert_eq!(actual, visible.iter().copied().find(|&p| p >= at));
                    assert!(
                        work.symbols <= 3,
                        "{prefix:?} {label:?} at {at}: {}",
                        work.symbols
                    );
                    assert!(work.nodes <= 12 * (doc.source.len().ilog2() as usize + 2));
                }
            }
        }

        // Twenty thousand adjacent old uses do not become a query-time scan.
        let mut doc = crate::Document::new();
        doc.append(&"[a]![a]".repeat(20_000)).unwrap();
        doc.append("\n\n[a]: /u\n").unwrap();
        for at in [0, 2, 6, 70_002, 139_999] {
            let mut work = VisibilityWork::default();
            doc.annotations.visible_query(
                at,
                doc.source.len(),
                (&doc.references.entries, &doc.links),
                &mut work,
            );
            assert!(work.gates <= 4 * (doc.source.len().ilog2() as usize + 2));
            assert_eq!(work.links, work.gates);
            assert_eq!(work.symbols, work.links);
            assert!(work.nodes <= 12 * (doc.source.len().ilog2() as usize + 2));
        }
    }

    #[test]
    fn permanent_hiding_masks_gates_and_never_reads_the_old_symbol() {
        let mut tree = Annotations::default();
        tree.hide_when_resolved(Span::new(1, 2), LinkId(999), 1);
        tree.apply(Span::new(0, 1_000_000), Style::HIDDEN, None, 2);
        assert_eq!(tree.at(1).effective_style(&[], &[]), Style::HIDDEN);
        let mut work = VisibilityWork::default();
        assert_eq!(
            tree.visible_query(0, 1_000_001, (&[], &[]), &mut work),
            Some(1_000_000)
        );
        assert_eq!(work.symbols, 0);
        assert_eq!(work.gates, 0);
        assert_eq!(work.links, 0);
        assert!(work.nodes <= 64);
        // No dynamic resolved bit is written into the permanent hidden summary.
        let mut tree = Annotations::default();
        tree.hide_when_resolved(Span::new(0, 1), LinkId(0), 1);
        assert_eq!(
            tree.next_visible_resolving(0, 2, &[], &[LinkTarget::Direct(Span::new(0, 0))]),
            Some(1)
        );
        assert_eq!(
            tree.next_visible_resolving(
                0,
                2,
                &[Reference {
                    label: "a".into(),
                    destination: None,
                    title: None,
                    revision: 0,
                    first_use: None
                }],
                &[LinkTarget::Reference(crate::SymbolId(0))]
            ),
            Some(0)
        );
        assert!(!tree.nodes[tree.root].hidden);
    }

    #[test]
    fn stable_gate_reads_only_current_target_and_direct_never_reads_symbols() {
        let mut doc = crate::Document::new();
        doc.append("[a][b]\n\n[a]: /a\n").unwrap();
        let LinkTarget::Reference(b) = doc.links[0] else {
            panic!()
        };
        let a = crate::SymbolId(0);
        let mark = doc.annotations.at(0);
        assert_eq!(mark.hide_owner(), Some(LinkId(0)));
        for target in [
            LinkTarget::Reference(a),
            LinkTarget::Reference(b),
            LinkTarget::Direct(Span::new(0, 0)),
            LinkTarget::Email(Span::new(0, 0)),
        ] {
            let mut work = VisibilityWork::default();
            let references = if matches!(target, LinkTarget::Reference(_)) {
                &doc.references.entries[..]
            } else {
                &[]
            };
            let hidden = target_available(LinkId(0), references, &[target], &mut work);
            assert_eq!(hidden, target != LinkTarget::Reference(b));
            assert_eq!(work.links, 1);
            assert_eq!(
                work.symbols,
                usize::from(matches!(target, LinkTarget::Reference(_)))
            );
            assert_eq!(
                mark.effective_style(references, &[target])
                    .contains(Style::HIDDEN),
                hidden
            );
            assert_eq!(
                doc.annotations
                    .next_visible_resolving(0, 6, references, &[target]),
                Some(usize::from(hidden))
            );
        }
        assert!(!doc.annotations.nodes[doc.annotations.root].hidden);
        assert_eq!(Mark::default().effective_style(&[], &[]), Style::default());
    }

    #[test]
    fn qualified_full_witnesses_and_long_suffixes_bound_all_visibility_lookups() {
        let labels = [
            "a", "中🙂", "*a*", "a\\]", "\\\n", "\\\r\n", "` `", "$ $", "** **",
        ];
        for prefix in ["", "> ", "- ", "# ", "| "] {
            for label in labels {
                let mut doc = crate::Document::new();
                doc.append(&format!(
                    "{prefix}[{label}][{}]![{label}][]***\n",
                    "b".repeat(999)
                ))
                .unwrap();
                doc.finish();
                for info in &doc.link_info {
                    assert!(
                        doc.annotations
                            .segments(info.label)
                            .any(|(_, m)| !m.style.contains(Style::HIDDEN))
                    );
                }
                for reference in &mut doc.references.entries {
                    reference.destination = Some(Span::new(0, 0));
                }
                let visible: Vec<_> = doc
                    .source
                    .char_indices()
                    .filter_map(|(at, _)| {
                        (!doc
                            .annotations
                            .at(at)
                            .effective_style(&doc.references.entries, &doc.links)
                            .contains(Style::HIDDEN))
                        .then_some(at)
                    })
                    .collect();
                for (at, _) in doc.source.char_indices() {
                    let mut work = VisibilityWork::default();
                    let next = doc.annotations.visible_query(
                        at,
                        doc.source.len(),
                        (&doc.references.entries, &doc.links),
                        &mut work,
                    );
                    assert_eq!(next, visible.iter().copied().find(|&v| v >= at));
                    let height = doc.source.len().next_power_of_two().ilog2() as usize + 1;
                    assert!(work.nodes <= 12 * height);
                    assert!(work.gates <= 4 * height);
                    assert_eq!((work.links, work.symbols), (work.gates, work.gates));
                }
            }
        }
        let mut doc = crate::Document::new();
        let mut source = String::new();
        for i in 0..20_000 {
            source.push_str(&format!("[a][b{i}]"));
        }
        doc.append(&source).unwrap();
        doc.append("\n\n").unwrap();
        doc.append(&"x".repeat(1_000_000)).unwrap();
        let before = doc.work();
        for _ in 0..1000 {
            doc.append("x").unwrap();
        }
        assert_eq!(doc.work().inline_bytes - before.inline_bytes, 1000);
        assert_eq!(doc.work().tree_visits, before.tree_visits);
        for reference in &mut doc.references.entries {
            reference.destination = Some(Span::new(0, 0));
        }
        assert_eq!(doc.links.len(), 20_000);
        for info in doc.link_info.iter().step_by(37) {
            for at in [info.source.start, info.label.end, info.source.end - 1] {
                let mut work = VisibilityWork::default();
                let next = doc.annotations.visible_query(
                    at,
                    doc.source.len(),
                    (&doc.references.entries, &doc.links),
                    &mut work,
                );
                assert_eq!(next, doc.next_visible(at));
                let height = doc.source.len().next_power_of_two().ilog2() as usize + 1;
                assert!(work.nodes <= 12 * height);
                assert!(work.gates <= 4 * height);
                assert_eq!((work.links, work.symbols), (work.gates, work.gates));
            }
        }
        // Nonqualified empty full objects keep permanent aggregation. No target
        // lookup is allowed even with twenty thousand distinct owners.
        let mut empty = crate::Document::new();
        let mut source = String::new();
        for i in 0..20_000 {
            source.push_str(&format!("[][b{i}]"));
        }
        empty.append(&source).unwrap();
        empty.finish();
        let mut work = VisibilityWork::default();
        assert_eq!(
            empty
                .annotations
                .visible_query(0, source.len(), (&[], &[]), &mut work),
            None
        );
        assert_eq!((work.gates, work.links, work.symbols), (0, 0, 0));
        assert!(work.nodes <= 64);
    }

    #[test]
    fn suffix_range_work_depends_on_tree_height_not_suffix_length_or_999() {
        for n in [1, 999, 65535, 1_048_575] {
            let mut tree = Annotations::default();
            tree.hide_when_resolved(Span::new(0, 1), LinkId(0), 1);
            tree.hide_when_resolved(Span::new(2, n + 3), LinkId(0), 1);
            let before = tree.visits;
            let mut work = VisibilityWork::default();
            assert_eq!(
                tree.visible_query(
                    2,
                    n + 4,
                    (&[], &[LinkTarget::Direct(Span::new(0, 0))]),
                    &mut work
                ),
                Some(n + 3)
            );
            let height = (n + 4).next_power_of_two().ilog2() as usize + 1;
            assert!(work.nodes <= 12 * height);
            assert!(work.gates <= 4 * height);
            assert_eq!(work.gates, work.links);
            assert_eq!(work.symbols, 0);
            assert_eq!(tree.visits, before);
        }
    }

    #[test]
    #[should_panic(expected = "overlapping shortcut marker owners")]
    fn different_marker_owners_are_an_internal_invariant_violation() {
        let mut tree = Annotations::default();
        tree.hide_when_resolved(Span::new(0, 1), LinkId(0), 1);
        tree.hide_when_resolved(Span::new(0, 1), LinkId(1), 1);
    }

    #[test]
    fn conditional_mark_and_query_frame_space_is_constant() {
        type LegacyMark = (Style, usize, usize, Option<LinkId>, u64, u64);
        let mark = std::mem::size_of::<Mark>();
        let frame = std::mem::size_of::<Frame>();
        assert!(mark <= std::mem::size_of::<LegacyMark>() + std::mem::size_of::<Option<LinkId>>());
        assert_eq!(
            frame,
            mark + std::mem::size_of::<Span>() + std::mem::size_of::<Option<usize>>()
        );
        eprintln!(
            "legacy Mark={} conditional Mark={mark} Frame={frame}",
            std::mem::size_of::<LegacyMark>()
        );
    }

    #[test]
    fn overlapping_tags_and_revisions_match_a_naive_array() {
        let mut tree = Annotations::default();
        let mut model = vec![Mark::default(); 1024];
        let mut seed = 17_u64;
        let mut random = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed as usize
        };
        for serial in 1..3000 {
            let a = random() % 1024;
            let b = random() % 1024;
            let span = Span::new(a.min(b), a.max(b) + 1);
            let style = Style(1 << (random() % 7));
            let link = (random() % 3 == 0).then_some(LinkId(serial as usize));
            // Many writes share a document revision, including link rewrites.
            let revision = serial / 7 + 1;
            tree.apply(span, style, link, revision);
            for point in &mut model[span.range()] {
                point.style = point.style.union(style);
                point.emphasis_depth += usize::from(style.contains(Style::EMPHASIS));
                point.strong_depth += usize::from(style.contains(Style::STRONG));
                if link.is_some() {
                    point.link = link;
                }
                point.revision = revision;
            }
            for _ in 0..8 {
                let at = random() % 1024;
                let actual = tree.at(at);
                assert_eq!(actual.style, model[at].style);
                assert_eq!(actual.emphasis_depth, model[at].emphasis_depth);
                assert_eq!(actual.strong_depth, model[at].strong_depth);
                assert_eq!(actual.link, model[at].link);
                assert_eq!(actual.revision, model[at].revision);
                let visible = (at..1024).find(|&i| !model[i].style.contains(Style::HIDDEN));
                assert_eq!(tree.next_visible(at, 1024), visible);
            }
            let lo = random() % 1024;
            let hi = random() % 1024;
            let query = Span::new(lo.min(hi), lo.max(hi) + 1);
            assert_eq!(
                tree.newest(query),
                model[query.range()]
                    .iter()
                    .map(|m| m.revision)
                    .max()
                    .unwrap()
            );
            if serial % 31 == 0 {
                let mut cursor = query.start;
                let mut segments = tree.segments(query);
                for (span, mark) in segments.by_ref() {
                    assert_eq!(span.start, cursor);
                    assert!(span.end > span.start);
                    for &expected in &model[span.range()] {
                        assert_eq!(mark.style, expected.style);
                        assert_eq!(mark.emphasis_depth, expected.emphasis_depth);
                        assert_eq!(mark.strong_depth, expected.strong_depth);
                        assert_eq!(mark.link, expected.link);
                        assert_eq!(mark.revision, expected.revision);
                    }
                    cursor = span.end;
                }
                assert_eq!(cursor, query.end);
                assert!(segments.visits <= 2 * query.len() as u64 + 32);
            }
        }
    }

    #[test]
    fn visibility_can_jump_past_capacity_and_respects_the_query_end() {
        let mut tree = Annotations::default();
        tree.apply(Span::new(0, 8), Style::HIDDEN, None, 1);
        assert_eq!(tree.next_visible(0, 12), Some(8));
        assert_eq!(tree.next_visible(0, 8), None);
        tree.apply(Span::new(9, 12), Style::HIDDEN, None, 2);
        assert_eq!(tree.next_visible(9, 11), None);
        assert_eq!(tree.next_visible(9, 13), Some(12));
    }

    #[test]
    fn segments_cover_unallocated_tail_and_empty_ranges() {
        let mut tree = Annotations::default();
        tree.apply(Span::new(0, 8), Style::STRONG, None, 1);
        let segments: Vec<_> = tree.segments(Span::new(5, 13)).collect();
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].0, Span::new(5, 8));
        assert_eq!(segments[0].1.style, Style::STRONG);
        assert_eq!(segments[1].0, Span::new(8, 13));
        assert_eq!(segments[1].1, Mark::default());
        assert_eq!(tree.segments(Span::new(13, 13)).count(), 0);
    }

    #[test]
    fn reusable_scratch_at_maximum_tree_depth_matches_sparse_write_log() {
        let end = isize::MAX as usize;
        let mut tree = Annotations::default();
        let mut writes = Vec::new();
        for bit in 0..usize::BITS - 1 {
            let at = (1_usize << bit) - 1;
            let span = Span::new(at, end - at);
            let style = if bit % 2 == 0 {
                Style::STRONG
            } else {
                Style::EMPHASIS
            };
            let link = Some(LinkId(bit as usize));
            tree.apply(span, style, link, u64::from(bit) + 1);
            writes.push((span, style, link));
        }
        let mut scratch = QueryScratch::default();
        for _ in 0..3 {
            let before = scratch.stats();
            // The end extends past the allocated tree, exercising both initial
            // frames and an unannotated tail without allocating huge input.
            let mut segments = tree.segments_with_scratch(Span::new(0, usize::MAX), scratch);
            let mut cursor = 0;
            for (span, mark) in segments.by_ref() {
                assert_eq!(span.start, cursor);
                for at in [span.start, span.end - 1] {
                    let mut expected = Mark::default();
                    for (i, (range, style, link)) in writes.iter().enumerate() {
                        if range.range().contains(&at) {
                            expected.style = expected.style.union(*style);
                            expected.link = *link;
                            expected.revision = i as u64 + 1;
                        }
                    }
                    assert_eq!(
                        (mark.style, mark.link, mark.revision),
                        (expected.style, expected.link, expected.revision)
                    );
                }
                cursor = span.end;
            }
            assert_eq!(cursor, usize::MAX);
            assert!(segments.visits <= 8 * u64::from(usize::BITS).pow(2));
            scratch = QueryScratch::default();
            segments.recycle_into(&mut scratch);
            assert_eq!(scratch.stats().nodes, 0);
            assert!(scratch.stats().capacity_bytes <= 2 * (usize::BITS as usize + 4) * 128);
            if before.capacity_bytes > 0 {
                assert_eq!(scratch.stats(), before);
            }
        }
    }
}

#[cfg(test)]
mod line_undo_tests {
    use super::*;
    #[test]
    fn epoch_saves_each_old_annotation_node_once_without_an_index_lookup() {
        let mut a = Annotations::default();
        for pos in 0..4096 {
            a.apply(Span::new(pos, pos + 1), Style::default(), None, 1);
        }
        let nodes = a.node_count();
        for commit in [false, true, false] {
            let original: Vec<_> = (0..4096).map(|p| a.at(p)).collect();
            a.begin_line();
            let before = a.visits;
            for i in 0..20_000 {
                a.apply(Span::new(i % 4096, i % 4096 + 1), Style::STRONG, None, 2);
            }
            let count = a.checkpoint.as_ref().unwrap().4.len();
            assert!(count <= nodes);
            assert_eq!(count, nodes);
            assert!(a.visits - before >= 20_000 * 12);
            if commit {
                assert_eq!(a.commit_line(), count as u64);
            } else {
                assert_eq!(a.rollback_line(), count as u64);
                for (p, old) in original.iter().enumerate() {
                    let now = a.at(p);
                    assert_eq!(
                        (now.style, now.strong_depth, now.revision),
                        (old.style, old.strong_depth, old.revision)
                    );
                }
            }
        }
        assert!(a.undo_stats().entries <= nodes);
    }
}
