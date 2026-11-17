use super::{
    index::Index,
    pages::Id,
    projection::{Kind, Projection},
    rope::{Rope, Small},
    runtime::{Cx, Job, boxed},
    tape::{Tape, normalized},
};
use crate::display::slug_metadata::{MetadataCursor, Value};
use crate::{
    AnchorTarget, Budget, ChangeKind, Completion, DisplayShapeEffectKind as E, DisplayShapeEffects,
    Document, DocumentId, Heading, HeadingId, LayoutError, LinkId, LinkResolver, LinkTarget, Span,
    Update, WorkLimit,
};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, LinkedList},
    rc::Rc,
    sync::Arc,
    task::Poll,
};

struct Request {
    buffer: Option<usize>,
    node: Option<Id>,
}
struct Core {
    index: Index,
    tape: Tape,
    projection: Projection,
    requests: BTreeMap<usize, Request>,
    entities: BTreeMap<Span, Small>,
}
impl Core {
    fn sync(&self) {
        self.index.mem.cx.storage(
            7,
            self.requests.len() * std::mem::size_of::<(usize, Request)>()
                + self.entities.len() * std::mem::size_of::<(Span, Small)>(),
            self.requests.len() + self.entities.len(),
            self.requests.len(),
        );
    }
    fn new(cx: Cx) -> Self {
        Self {
            index: Index::new(cx.clone()),
            tape: Tape::default(),
            projection: Projection::new(cx),
            requests: BTreeMap::new(),
            entities: BTreeMap::new(),
        }
    }
}
type Lane = Job<'static, Core>;
struct StyleUpdate {
    since: u64,
    last: Option<(u64, usize)>,
    prepared: bool,
}
struct Batch {
    update: Arc<Update>,
    event: usize,
    change: usize,
    heading: usize,
}
#[derive(Default)]
struct Inbox {
    next: usize,
    cancel: LinkedList<usize>,
    closed: bool,
}

/// Current cold-greedy heading names. Subscribe once, observe every subsequent
/// host Update in order, and advance with the same Budget as fragment queries.
/// Body-local heading IDs are deliberately excluded. No Markdown is reparsed.
///
/// Results are published only at a coherent current revision. Pending work is
/// continued across appends; a revision never drops or clones the entire index.
/// Use `retire` before destruction when a frame must pay graph destruction.
pub struct HeadingSlugSession {
    document: DocumentId,
    observed: u64,
    sequence: usize,
    subscription: DisplayShapeEffects,
    cx: Cx,
    core: Option<Core>,
    job: Option<Lane>,
    inbox: Rc<RefCell<Inbox>>,
    batches: LinkedList<Batch>,
    retired: LinkedList<Update>,
    backlog_bytes: usize,
    dirty: BTreeMap<usize, usize>,
    titles: BTreeSet<usize>,
    headings: BTreeMap<usize, Heading>,
    cursor: Option<MetadataCursor>,
    cursor_span: Span,
    overlap: Option<(usize, usize)>,
    bootstrap_links: usize,
    bootstrap_headings: usize,
    bindings: BTreeMap<usize, (usize, usize)>,
    dependencies: BTreeMap<(usize, usize), usize>,
    reference: Option<(usize, Option<usize>)>,
    style_updates: BTreeMap<usize, StyleUpdate>,
    published_revision: u64,
    resolver: LinkResolver,
    preserved: bool,
    resolver_reset: bool,
    closing: bool,
    closed: bool,
}

/// Observable work and retained storage, including the paused mutation lane.
/// Storage inspection walks queued child Updates and resolver scopes; call for diagnostics,
/// not as an unbudgeted step in a rendering loop. Private BTreeMap slack/headers and allocator overhead are excluded; fixed page capacities and logical map payloads are included.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SlugSessionStats {
    pub work: super::runtime::SlugWork,
    pub headings: usize,
    pub requests: usize,
    pub owned_capacity_bytes: usize,
    pub directory_entries: usize,
    pub rope_nodes: usize,
    pub trie_nodes: usize,
    pub numeric_nodes: usize,
    pub pending_retirement: usize,
    pub event_backlog_bytes: usize,
    pub continuation_bytes: usize,
    pub continuation_peak: usize,
    pub resolver_entries: usize,
    pub canonical_style_work: crate::StyleResolutionWork,
}

fn step(b: &mut Budget) -> bool {
    b.charge(WorkLimit {
        queries: 1,
        objects: 1,
        ..Default::default()
    })
}
impl HeadingSlugSession {
    /// O(1) subscription. Existing source is normalized once under `advance`;
    /// existing canonical metadata is seeded lazily, without resetting on append.
    pub fn new(doc: &mut Document) -> Self {
        doc.subscribe_display_shape_effects();
        let cx = Cx::default();
        let mut dirty = BTreeMap::new();
        if !doc.source().is_empty() {
            dirty.insert(0, doc.source().len());
        }
        Self {
            document: doc.id(),
            observed: doc.revision(),
            sequence: doc.display_shape_stats().emitted,
            subscription: doc.shape.subscription(),
            core: Some(Core::new(cx.clone())),
            cx,
            job: None,
            inbox: Rc::new(RefCell::new(Inbox::default())),
            batches: LinkedList::new(),
            retired: LinkedList::new(),
            backlog_bytes: 0,
            dirty,
            titles: BTreeSet::new(),
            headings: BTreeMap::new(),
            cursor: None,
            cursor_span: Span::new(0, 0),
            overlap: None,
            bootstrap_links: 0,
            bootstrap_headings: 0,
            bindings: BTreeMap::new(),
            dependencies: BTreeMap::new(),
            reference: None,
            style_updates: BTreeMap::new(),
            published_revision: doc.revision(),
            resolver: doc.link_resolver(),
            preserved: true,
            resolver_reset: false,
            closing: false,
            closed: false,
        }
    }
    fn check(&self, doc: &Document) -> Result<(), LayoutError> {
        if doc.id() != self.document {
            return Err(LayoutError::WrongDocument);
        }
        if doc.revision() != self.observed {
            return Err(LayoutError::OutOfOrderUpdate);
        }
        if self.closing {
            return Err(LayoutError::InvalidRange);
        }
        Ok(())
    }
    /// Enqueue ownership without walking effects, titles or historical uses.
    /// Merged contiguous updates are accepted. Wrong/out-of-order input leaves
    /// state and the caller's Budget unchanged.
    pub fn observe(&mut self, doc: &Document, update: Arc<Update>) -> Result<(), LayoutError> {
        if doc.id() != self.document || update.document != Some(self.document) {
            return Err(LayoutError::WrongDocument);
        }
        if self.closing
            || update.base_revision != self.observed
            || update.revision != doc.revision()
            || update.revision <= self.observed
            || !self.subscription.same_subscription(&update.display_shape)
        {
            return Err(LayoutError::OutOfOrderUpdate);
        }
        let events = update.display_shape.events();
        if events
            .first()
            .is_some_and(|e| e.sequence != self.sequence + 1)
        {
            return Err(LayoutError::OutOfOrderUpdate);
        }
        if events.last().map_or(self.sequence, |e| e.sequence) != doc.display_shape_stats().emitted
        {
            return Err(LayoutError::OutOfOrderUpdate);
        }
        if let Some(last) = events.last() {
            self.sequence = last.sequence;
        }
        self.observed = update.revision;
        self.backlog_bytes += update.display_shape.capacity_bytes()
            + update.changes.capacity() * std::mem::size_of::<crate::Change>()
            + std::mem::size_of::<Update>();
        let heading = update.changed_headings.start;
        self.batches.push_back(Batch {
            update,
            event: 0,
            change: 0,
            heading,
        });
        Ok(())
    }
    fn dirty(&mut self, source: Span) {
        if !source.is_empty() {
            self.dirty
                .entry(source.start)
                .and_modify(|end| *end = (*end).max(source.end))
                .or_insert(source.end);
        }
    }
    fn scope(&mut self, doc: &Document, key: usize) {
        if let Some(span) = doc.inline_styles.slug_scope_extent(key) {
            self.dirty(span);
        }
    }
    fn record_binding(&mut self, doc: &Document, id: LinkId) {
        let next = doc.link_info(id).and_then(|info| {
            if !doc.link_order.active(id) {
                return None;
            }
            let headings = doc.headings().entries();
            let at = headings.partition_point(|h| h.content.end <= info.label.start);
            let heading = headings.get(at)?;
            if info.label.start < heading.content.start {
                return None;
            }
            let (scope, _) = doc.inline_styles.slug_scope(info.label.start)?;
            match doc.links().get(id.0)? {
                LinkTarget::Reference(symbol) => Some((symbol.0, scope)),
                _ => None,
            }
        });
        if self.bindings.get(&id.0).copied() == next {
            return;
        }
        if let Some(old) = self.bindings.remove(&id.0) {
            let count = self.dependencies.get_mut(&old).unwrap();
            *count -= 1;
            if *count == 0 {
                self.dependencies.remove(&old);
            }
        }
        if let Some(next) = next {
            self.bindings.insert(id.0, next);
            *self.dependencies.entry(next).or_default() += 1;
        }
    }
    fn link(&mut self, doc: &Document, id: LinkId) {
        if let Some(&(_, scope)) = self.bindings.get(&id.0) {
            self.scope(doc, scope);
        }
        if let Some(info) = doc.link_info(id) {
            self.dirty(info.source);
        }
        self.record_binding(doc, id);
    }
    fn heading(&mut self, id: usize, h: Heading) {
        let old = self.headings.insert(id, h);
        if old == Some(h) {
            return;
        }
        self.titles.insert(id);
        if let Some(old) = old {
            // Envelope growth is not an invalidation of the old title's
            // canonical metadata. Producer mark/replacement/style deltas pay
            // any actual edits within that overlap independently.
            if h.content.start < old.content.start {
                self.dirty(Span::new(
                    h.content.start,
                    h.content.end.min(old.content.start),
                ));
            }
            if h.content.end > old.content.end {
                self.dirty(Span::new(
                    h.content.start.max(old.content.end),
                    h.content.end,
                ));
            }
        } else {
            self.dirty(h.content);
        }
    }
    fn style(&mut self, doc: &Document, scope: usize, event: crate::DisplayShapeStyleEvent) {
        use crate::DisplayShapeStyleEvent as S;
        let Some(span) = doc.inline_styles.slug_scope_extent(scope) else {
            return;
        };
        let headings = doc.headings().entries();
        let at = headings.partition_point(|h| h.content.end <= span.start);
        if headings.get(at).is_none_or(|h| h.content.start >= span.end) {
            return;
        }
        if doc.inline_styles.conditional(scope).is_some() {
            self.style_updates
                .entry(scope)
                .and_modify(|u| u.prepared = false)
                .or_insert(StyleUpdate {
                    since: self.published_revision,
                    last: None,
                    prepared: false,
                });
            // A changed binding or rewritten old event is a genuine recovery
            // invalidation; append-only events publish their exact new markers.
            if !self.resolver.slug_append_style(doc, scope, event) {
                self.scope(doc, scope);
            }
            return;
        }
        match event {
            S::Run { source, .. } => self.dirty(source),
            S::Pair(pair) => {
                self.dirty(pair.open);
                self.dirty(pair.close);
            }
            S::Bind { .. } => self.scope(doc, scope),
            S::Enter { .. } | S::Leave { .. } => (),
        }
    }
    fn effect(&mut self, doc: &Document, e: &E) {
        let (_, old_source, old_links) = self.resolver.slug_binding();
        self.preserved &= !match *e {
            E::ScopeEvent { scope, event, .. } => {
                !self.resolver.slug_append_style(doc, scope, event)
            }
            E::Mark { source, mark } => {
                if source.start < old_source {
                    self.resolver.slug_invalidate_continuations(doc);
                }
                source.start < old_source
                    && (mark.base_hidden
                        || mark.link.is_some()
                        || mark.hide_owner.is_some()
                        || mark.hide_alternative.is_some())
            }
            E::Replacement { source, .. } => source.start < old_source,
            E::TruncateReplacements { .. }
            | E::Rollback(_)
            | E::Math { .. }
            | E::Footnote { .. }
            | E::FootnoteReference { .. } => true,
            E::Link { id, .. }
            | E::LinkGate { id, .. }
            | E::LinkParent { id, .. }
            | E::LinkActive { id, .. }
            | E::LinkState { id, .. } => id.0 < old_links,
            E::Reference { id, .. } => doc
                .references()
                .get(id.0)
                .is_some_and(|r| r.first_use.is_some_and(|at| at < old_source)),
            _ => false,
        };
        match *e {
            E::SourceAppend(source) => self.dirty(source),
            E::Mark { source, .. }
            | E::Replacement { source, .. }
            | E::Structure { source, .. } => self.dirty(source),
            E::ScopeEvent { scope, event, .. } => self.style(doc, scope, event),
            E::Link { id, .. }
            | E::LinkSource { id, .. }
            | E::LinkGate { id, .. }
            | E::LinkParent { id, .. }
            | E::LinkActive { id, .. }
            | E::LinkState { id, .. } => self.link(doc, id),
            E::Reference { id, .. } => self.reference = Some((id.0, None)),
            // Range rollback facts are supplied by Update.changes. Extent-only
            // growth is covered by appended source, not the old literal body.
            _ => (),
        }
    }
    fn start(&mut self, job: Lane) {
        self.job = Some(job);
    }
    fn normalize(&mut self, ch: char) {
        let mut c = self.core.take().unwrap();
        let cx = self.cx.clone();
        self.start(boxed(&cx, async move {
            let old = c.tape.len;
            let changed = c
                .tape
                .append(&mut c.index.mem, ch)
                .await
                .unwrap_or(Span::new(old, c.tape.len));
            c.projection
                .extend(&c.tape, &mut c.index.mem, changed)
                .await;
            c.index.mem.retire().await;
            c
        }));
    }
    fn assign(&mut self, source: Span, value: Value) {
        let mut c = self.core.take().unwrap();
        let cx = self.cx.clone();
        self.start(boxed(&cx, async move {
            let kind = match value {
                Value::Raw => Kind::Raw,
                Value::Hidden => Kind::Hidden,
                Value::Code(context) => Kind::Code(context),
                Value::Spaces(n) => {
                    for _ in 0..n {
                        c.index.mem.cx.charge(super::runtime::Op::Copy).await;
                    }
                    Kind::Atom {
                        anchor: source.start,
                        text: Small::from_bytes(&[b'-'; 8][..usize::from(n)]),
                    }
                }
                Value::Entity {
                    source: entity,
                    utf8,
                    len,
                } => {
                    let mut out = c.entities.get(&entity).copied().unwrap_or(Small::EMPTY);
                    if !c.entities.contains_key(&entity) {
                        // Immutable entities contain at most two scalars. Raw source
                        // was already normalized by Tape; generated text is bounded.
                        for ch in std::str::from_utf8(&utf8[..usize::from(len)])
                            .unwrap()
                            .chars()
                        {
                            c.index
                                .mem
                                .cx
                                .charge(super::runtime::Op::Generated(ch.len_utf8()))
                                .await;
                            let text = normalized(ch);
                            for i in 0..usize::from(text.len) {
                                c.index.mem.cx.charge(super::runtime::Op::Emit).await;
                                out.bytes[usize::from(out.len)] = text.get(i);
                                out.len += 1;
                            }
                        }
                        c.entities.insert(entity, out);
                        c.sync();
                    }
                    Kind::Atom {
                        anchor: entity.start,
                        text: out,
                    }
                }
            };
            c.projection
                .assign(source, kind, &c.tape, &mut c.index.mem)
                .await;
            c.projection.retire(&mut c.index.mem).await;
            c.index.mem.retire().await;
            c
        }));
    }
    fn title(&mut self, id: usize, heading: Heading) {
        let mut c = self.core.take().unwrap();
        let cx = self.cx.clone();
        self.start(boxed(&cx, async move {
            let rope = c
                .projection
                .range(heading.content, &c.tape, &mut c.index.mem)
                .await;
            c.index.set(id, rope).await;
            c.index.mem.retire().await;
            c
        }));
    }
    fn coherent(&self, doc: &Document) -> bool {
        !self.closing
            && self.observed == doc.revision()
            && self.job.is_none()
            && self.batches.is_empty()
            && self.reference.is_none()
            && self.style_updates.is_empty()
            && self.bootstrap_links == doc.links().len()
            && self.bootstrap_headings == doc.headings().entries().len()
            && self.dirty.is_empty()
            && self.cursor.is_none()
            && self.overlap.is_none()
            && self.titles.is_empty()
            && self
                .core
                .as_ref()
                .is_some_and(|c| c.tape.len == doc.source().len() && c.index.dirty.is_empty())
    }
    pub fn is_current(&self, doc: &Document) -> bool {
        doc.id() == self.document && self.coherent(doc)
    }
    pub fn indexed_headings(&self) -> usize {
        self.headings.len()
    }
    /// All producer discovery, source normalization, collision cascades and
    /// retirement share this Budget. Covered means current, including preEOF.
    pub fn advance(&mut self, doc: &Document, b: &mut Budget) -> Result<Completion, LayoutError> {
        self.check(doc)?;
        loop {
            if let Some(job) = &mut self.job {
                match self.cx.poll(job.as_mut(), b) {
                    Poll::Pending => return Ok(Completion::BudgetExhausted),
                    Poll::Ready(c) => {
                        self.job = None;
                        self.core = Some(c);
                    }
                }
                continue;
            }
            let cancelled = self.inbox.borrow().cancel.front().copied();
            if let Some(id) = cancelled {
                if !step(b) {
                    return Ok(Completion::BudgetExhausted);
                }
                self.inbox.borrow_mut().cancel.pop_front();
                let mut c = self.core.take().unwrap();
                let cx = self.cx.clone();
                self.job = Some(boxed(&cx, async move {
                    if let Some(r) = c.requests.remove(&id) {
                        c.sync();
                        if let Some(n) = r.node {
                            c.index.unregister(n).await;
                        }
                        if let Some(buffer) = r.buffer {
                            c.index.mem.drop_buffer_owner(buffer);
                        }
                        c.index.mem.retire().await;
                    }
                    c.sync();
                    c
                }));
                continue;
            }
            if let Some((symbol, last)) = self.reference {
                if !step(b) {
                    return Ok(Completion::BudgetExhausted);
                }
                use std::ops::Bound::{Excluded, Included, Unbounded};
                let low = last.map_or(Included((symbol, 0)), |s| Excluded((symbol, s)));
                let next = self
                    .dependencies
                    .range((low, Unbounded))
                    .next()
                    .map(|(&(s, k), _)| (s, k));
                if let Some((s, k)) = next
                    && s == symbol
                {
                    self.scope(doc, k);
                    self.reference = Some((symbol, Some(k)));
                } else {
                    self.reference = None;
                }
                continue;
            }
            if let Some(batch) = self.batches.front_mut() {
                if !step(b) {
                    return Ok(Completion::BudgetExhausted);
                }
                if let Some(e) = batch.update.display_shape.events().get(batch.event) {
                    let effect = e.kind.clone();
                    batch.event += 1;
                    self.cx.effect();
                    self.effect(doc, &effect);
                    continue;
                }
                if let Some(change) = batch.update.changes.get(batch.change).copied() {
                    batch.change += 1;
                    if change.kind != ChangeKind::Reference {
                        self.dirty(change.source);
                    }
                    continue;
                }
                if batch.heading < batch.update.changed_headings.end {
                    let id = batch.heading;
                    batch.heading += 1;
                    if let Some(h) = doc.headings().get(HeadingId(id)).copied() {
                        self.cx.heading();
                        self.heading(id, h);
                    }
                    continue;
                }
                let batch = self.batches.pop_front().unwrap();
                self.backlog_bytes -= batch.update.display_shape.capacity_bytes()
                    + batch.update.changes.capacity() * std::mem::size_of::<crate::Change>()
                    + std::mem::size_of::<Update>();
                if let Ok(update) = Arc::try_unwrap(batch.update) {
                    self.retired.push_back(update);
                }
                continue;
            }
            if self.bootstrap_links < doc.links().len() {
                if !step(b) {
                    return Ok(Completion::BudgetExhausted);
                }
                let id = LinkId(self.bootstrap_links);
                self.bootstrap_links += 1;
                self.link(doc, id);
                continue;
            }
            if self.bootstrap_headings < doc.headings().entries().len() {
                if !step(b) {
                    return Ok(Completion::BudgetExhausted);
                }
                let id = self.bootstrap_headings;
                self.bootstrap_headings += 1;
                let h = *doc.headings().get(HeadingId(id)).unwrap();
                self.cx.heading();
                self.heading(id, h);
                continue;
            }
            if self.resolver.slug_binding().0 != doc.revision() {
                if !self.resolver_reset {
                    if !step(b) {
                        return Ok(Completion::BudgetExhausted);
                    }
                    let base = self.resolver.slug_binding().0;
                    if self.resolver.slug_rebind(doc, base, self.preserved)? {
                        self.preserved = true;
                        continue;
                    }
                    self.resolver_reset = true;
                }
                if let Some(cursor) = &mut self.cursor
                    && !cursor.retire(b)
                {
                    return Ok(Completion::BudgetExhausted);
                }
                if self.resolver.retire(b) != Completion::Covered {
                    return Ok(Completion::BudgetExhausted);
                }
                self.resolver.reset(doc)?;
                self.resolver_reset = false;
                self.preserved = true;
                continue;
            }
            if self.core.as_ref().unwrap().tape.len < doc.source().len() {
                if !step(b) {
                    return Ok(Completion::BudgetExhausted);
                }
                let at = self.core.as_ref().unwrap().tape.len;
                let ch = doc.source()[at..].chars().next().unwrap();
                self.normalize(ch);
                continue;
            }
            if let Some((&key, update)) = self.style_updates.first_key_value() {
                if !update.prepared {
                    if !self.resolver.resolution.prepare_styles(doc, key, b)? {
                        return Ok(Completion::BudgetExhausted);
                    }
                    self.style_updates.get_mut(&key).unwrap().prepared = true;
                }
                if !step(b) {
                    return Ok(Completion::BudgetExhausted);
                }
                let update = &self.style_updates[&key];
                let change =
                    self.resolver
                        .resolution
                        .styles
                        .changed_marker(key, update.since, update.last);
                if let Some((at, span)) = change {
                    self.style_updates.get_mut(&key).unwrap().last = Some(at);
                    self.dirty(span);
                } else {
                    self.style_updates.remove(&key);
                }
                continue;
            }
            if let Some((at, end)) = self.overlap {
                if !step(b) {
                    return Ok(Completion::BudgetExhausted);
                }
                if at < end {
                    self.titles.insert(at);
                    self.overlap = Some((at + 1, end));
                } else {
                    self.overlap = None;
                }
                continue;
            }
            if let Some(cursor) = &mut self.cursor {
                match cursor.next(doc, &mut self.resolver, b)? {
                    Err(_) => return Ok(Completion::BudgetExhausted),
                    Ok(Some(unit)) => {
                        self.cx.metadata();
                        for id in unit.links.into_iter().flatten() {
                            self.record_binding(doc, id);
                        }
                        self.assign(unit.source, unit.value);
                        continue;
                    }
                    Ok(None) => {
                        if !cursor.retire(b) {
                            return Ok(Completion::BudgetExhausted);
                        }
                        self.cursor = None;
                        let span = self.cursor_span;
                        let hs = doc.headings().entries();
                        if !step(b) {
                            self.cursor = Some(MetadataCursor::new(Span::new(span.end, span.end)));
                            return Ok(Completion::BudgetExhausted);
                        }
                        let start = hs.partition_point(|h| h.content.end <= span.start);
                        let end = hs.partition_point(|h| h.content.start < span.end);
                        self.overlap = Some((start, end));
                        continue;
                    }
                }
            }
            if let Some((&start, &end)) = self.dirty.first_key_value() {
                if !step(b) {
                    return Ok(Completion::BudgetExhausted);
                }
                self.dirty.remove(&start);
                // Coalesce one overlapping pending interval per paid iteration.
                if let Some((&next, &tail)) = self.dirty.first_key_value()
                    && next <= end
                {
                    self.dirty.remove(&next);
                    self.dirty.insert(start, end.max(tail));
                    continue;
                }
                let end = end.min(doc.source().len());
                let headings = doc.headings().entries();
                let at = headings.partition_point(|h| h.content.end <= start);
                if let Some(h) = headings.get(at)
                    && h.content.start < end
                {
                    let clipped = Span::new(start.max(h.content.start), end.min(h.content.end));
                    if clipped.end < end {
                        self.dirty(Span::new(clipped.end, end));
                    }
                    if !clipped.is_empty() {
                        self.cursor_span = clipped;
                        self.cursor = Some(MetadataCursor::new(clipped));
                    }
                }
                continue;
            }
            if let Some(&id) = self.titles.first() {
                if !step(b) {
                    return Ok(Completion::BudgetExhausted);
                }
                self.titles.remove(&id);
                let h = self.headings[&id];
                self.title(id, h);
                continue;
            }
            if !self.core.as_ref().unwrap().index.dirty.is_empty() {
                if !step(b) {
                    return Ok(Completion::BudgetExhausted);
                }
                let mut c = self.core.take().unwrap();
                let cx = self.cx.clone();
                self.job = Some(boxed(&cx, async move {
                    c.index.maintain().await;
                    c.index.mem.retire().await;
                    c
                }));
                continue;
            }
            if !self.retired.is_empty() {
                if !step(b) {
                    return Ok(Completion::BudgetExhausted);
                }
                let u = self.retired.back_mut().unwrap();
                if let Some(body) = u.body_updates.pop() {
                    self.retired.push_back(body.into_local());
                } else {
                    self.retired.pop_back();
                }
                continue;
            }
            self.published_revision = doc.revision();
            return Ok(Completion::Covered);
        }
    }
    pub fn stats(&self) -> SlugSessionStats {
        let (f, peak) = self.cx.future_bytes();
        let mut s = SlugSessionStats {
            work: self.cx.work(),
            headings: self.headings.len(),
            event_backlog_bytes: self.backlog_bytes,
            continuation_bytes: f,
            continuation_peak: peak,
            resolver_entries: self.resolver.memo_entries(),
            canonical_style_work: self.resolver.style_work(),
            ..Default::default()
        };
        s.owned_capacity_bytes = std::mem::size_of::<Self>()
            + self.cx.capacity_bytes()
            + std::mem::size_of::<RefCell<Inbox>>()
            + self.resolver.capacity_bytes()
            + f
            + self.cursor.as_ref().map_or(0, MetadataCursor::capacity);
        let mut updates: Vec<&Update> = self
            .batches
            .iter()
            .map(|b| b.update.as_ref())
            .chain(self.retired.iter())
            .collect();
        s.event_backlog_bytes = 0;
        while let Some(u) = updates.pop() {
            s.event_backlog_bytes += update_bytes(u);
            updates.extend(u.body_updates.iter().map(|u| u.facts().value));
        }
        s.owned_capacity_bytes += self.batches.len()
            * (std::mem::size_of::<Batch>() + 2 * std::mem::size_of::<usize>())
            + self.retired.len() * 2 * std::mem::size_of::<usize>();
        s.owned_capacity_bytes += s.event_backlog_bytes
            + self.inbox.borrow().cancel.len() * std::mem::size_of::<(usize, usize, usize)>();
        s.owned_capacity_bytes +=
            self.style_updates.len() * std::mem::size_of::<(usize, StyleUpdate)>();
        s.directory_entries = self.style_updates.len()
            + self.dirty.len()
            + self.titles.len()
            + self.headings.len()
            + self.bindings.len()
            + self.dependencies.len()
            + self.batches.len()
            + self.retired.len();
        let stores = self.cx.stores();
        s.owned_capacity_bytes += stores.iter().map(|v| v[0]).sum::<usize>();
        s.directory_entries += stores.iter().map(|v| v[1]).sum::<usize>();
        s.owned_capacity_bytes += stores[1][2].saturating_sub(1) * std::mem::size_of::<(u8, Id)>();
        s.directory_entries += stores[1][2].saturating_sub(1);
        s.rope_nodes = stores[0][2];
        s.trie_nodes = stores[1][2];
        s.numeric_nodes = stores[2][2];
        s.requests = stores[7][2];
        s.pending_retirement = stores[4][2] + stores[8][2];
        s.owned_capacity_bytes += self.dirty.len() * std::mem::size_of::<(usize, usize)>()
            + self.titles.len() * std::mem::size_of::<usize>()
            + self.headings.len() * std::mem::size_of::<(usize, Heading)>()
            + self.bindings.len() * std::mem::size_of::<(usize, (usize, usize))>()
            + self.dependencies.len() * std::mem::size_of::<((usize, usize), usize)>();
        s
    }
}

/// A durable lookup owns an unoccupied trie endpoint, never an old target.
/// The caller supplies an Arc so creation does not copy an arbitrarily long
/// fragment. Percent escapes and the complete UTF-8 tail are consumed once.
/// Dropping enqueues constant-size cancellation; `advance`/`retire` pays release.
enum FragmentText {
    Shared(Arc<str>),
    Owned(Arc<String>),
}
impl FragmentText {
    fn text(&self) -> &str {
        match self {
            Self::Shared(s) => s,
            Self::Owned(s) => s,
        }
    }
}
impl std::ops::Deref for FragmentText {
    type Target = str;
    fn deref(&self) -> &str {
        self.text()
    }
}
/// Revision-spanning percent/UTF-8 continuation and unoccupied endpoint lease.
/// Read through HeadingSlugSession::advance_fragment to get its current owner.
pub struct CurrentFragment {
    inbox: Rc<RefCell<Inbox>>,
    id: usize,
    raw: FragmentText,
    position: usize,
    utf8: [u8; 4],
    utf8_len: usize,
    utf8_need: usize,
    invalid: bool,
    sealed: bool,
}
impl HeadingSlugSession {
    pub fn fragment(&self, raw: Arc<str>) -> Result<CurrentFragment, LayoutError> {
        self.make_fragment(FragmentText::Shared(raw))
    }
    /// Move an existing flat destination buffer without copying its bytes.
    pub fn fragment_owned(&self, raw: Arc<String>) -> Result<CurrentFragment, LayoutError> {
        self.make_fragment(FragmentText::Owned(raw))
    }
    fn make_fragment(&self, raw: FragmentText) -> Result<CurrentFragment, LayoutError> {
        if self.closing || !raw.starts_with('#') {
            return Err(LayoutError::InvalidRange);
        }
        let mut inbox = self.inbox.borrow_mut();
        let id = inbox.next;
        inbox.next = id.checked_add(1).unwrap();
        Ok(CurrentFragment {
            inbox: self.inbox.clone(),
            id,
            raw,
            position: 1,
            utf8: [0; 4],
            utf8_len: 0,
            utf8_need: 0,
            invalid: false,
            sealed: false,
        })
    }
    pub fn advance_fragment(
        &mut self,
        doc: &Document,
        request: &mut CurrentFragment,
        b: &mut Budget,
    ) -> Result<crate::FragmentStep, LayoutError> {
        self.check(doc)?;
        if !Rc::ptr_eq(&self.inbox, &request.inbox) {
            return Err(LayoutError::WrongDocument);
        }
        if request.invalid {
            return Err(LayoutError::InvalidRange);
        }
        if self.advance(doc, b)? != Completion::Covered {
            return Ok(crate::FragmentStep::BudgetExhausted);
        }
        if request.raw.len() == 1 {
            if !step(b) {
                return Ok(crate::FragmentStep::BudgetExhausted);
            }
            return Ok(crate::FragmentStep::Resolved(AnchorTarget {
                heading: None,
                source: 0,
                provisional: false,
            }));
        }
        let c = self.core.as_mut().unwrap();
        if !c.requests.contains_key(&request.id) {
            if !step(b) {
                return Ok(crate::FragmentStep::BudgetExhausted);
            }
            let buffer = c.index.mem.new_buffer();
            c.requests.insert(
                request.id,
                Request {
                    buffer: Some(buffer),
                    node: None,
                },
            );
            c.sync();
        }
        while request.position < request.raw.len() {
            let raw = request.raw.as_bytes();
            let pos = request.position;
            let encoded = raw[pos] == b'%';
            let n = if encoded { 3 } else { 1 }.min(raw.len() - pos);
            if !b.charge(WorkLimit {
                bytes: n,
                objects: 1,
                ..Default::default()
            }) {
                return Ok(crate::FragmentStep::BudgetExhausted);
            }
            let byte = if encoded {
                match raw.get(pos + 1..pos + n) {
                    Some([a, z]) => match (hex(*a), hex(*z)) {
                        (Some(a), Some(z)) => (a << 4) | z,
                        _ => {
                            request.invalid = true;
                            return Err(LayoutError::InvalidRange);
                        }
                    },
                    _ => {
                        request.invalid = true;
                        return Err(LayoutError::InvalidRange);
                    }
                }
            } else {
                raw[pos]
            };
            request.position += n;
            self.cx.query(n);
            if !request.valid(byte) {
                request.invalid = true;
                return Err(LayoutError::InvalidRange);
            }
            let buffer = c.requests[&request.id].buffer.unwrap();
            c.index.mem.push(buffer, byte);
        }
        if request.utf8_len != 0 {
            request.invalid = true;
            return Err(LayoutError::InvalidRange);
        }
        if !request.sealed {
            if !step(b) {
                return Ok(crate::FragmentStep::BudgetExhausted);
            }
            request.sealed = true;
            let id = request.id;
            let mut c = self.core.take().unwrap();
            let cx = self.cx.clone();
            self.job = Some(boxed(&cx, async move {
                let buffer = c.requests[&id].buffer.unwrap();
                let len = c.index.mem.buffer_len(buffer);
                let rope = c.index.mem.span(buffer, 0, len);
                let node = c.index.register(rope).await;
                c.requests.get_mut(&id).unwrap().node = Some(node);
                c.index.mem.drop_buffer_owner(buffer);
                c.requests.get_mut(&id).unwrap().buffer = None;
                c.index.mem.retire().await;
                c
            }));
            if self.advance(doc, b)? != Completion::Covered {
                return Ok(crate::FragmentStep::BudgetExhausted);
            }
        }
        if !step(b) {
            return Ok(crate::FragmentStep::BudgetExhausted);
        }
        let c = self.core.as_ref().unwrap();
        let owner = c.requests[&request.id]
            .node
            .and_then(|n| c.index.current(n));
        Ok(if let Some(id) = owner {
            crate::FragmentStep::Resolved(AnchorTarget {
                heading: Some(HeadingId(id)),
                source: self.headings[&id].source.start,
                provisional: !doc.is_finished(),
            })
        } else if doc.is_finished() {
            crate::FragmentStep::NotFound
        } else {
            crate::FragmentStep::Unresolved
        })
    }
}
impl CurrentFragment {
    fn valid(&mut self, byte: u8) -> bool {
        if self.utf8_len == 0 {
            self.utf8_need = match byte {
                0..=0x7f => return true,
                0xc2..=0xdf => 2,
                0xe0..=0xef => 3,
                0xf0..=0xf4 => 4,
                _ => return false,
            };
        } else if byte & 0xc0 != 0x80 {
            return false;
        }
        self.utf8[self.utf8_len] = byte;
        self.utf8_len += 1;
        if self.utf8_len == self.utf8_need {
            let valid = std::str::from_utf8(&self.utf8[..self.utf8_len]).is_ok();
            self.utf8_len = 0;
            return valid;
        }
        true
    }
    pub fn capacity_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.input_capacity_bytes()
    }
    pub fn input_capacity_bytes(&self) -> usize {
        match &self.raw {
            FragmentText::Shared(s) => s.len(),
            FragmentText::Owned(s) => s.capacity(),
        }
    }
}
impl Drop for CurrentFragment {
    fn drop(&mut self) {
        let mut inbox = self.inbox.borrow_mut();
        if !inbox.closed {
            inbox.cancel.push_back(self.id);
        }
    }
}
fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// A borrowed current base and decimal suffix. Obtaining it is O(log H), with
/// no long string copy. Each traversal/output byte is paid by `next_byte`.
/// The borrow prevents observing a stale rope across a session mutation.
pub struct CurrentSlug<'a> {
    pub target: AnchorTarget,
    memory: &'a super::rope::Memory,
    base: Rope,
    suffix: Small,
    position: usize,
    walking: Option<(Rope, usize)>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlugByte {
    Byte(u8),
    BudgetExhausted,
    End,
}
impl CurrentSlug<'_> {
    pub fn len(&self) -> usize {
        self.memory.len(self.base) + usize::from(self.suffix.len)
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn next_byte(&mut self, b: &mut Budget) -> SlugByte {
        if self.position == self.len() {
            return SlugByte::End;
        }
        let n = self.memory.len(self.base);
        if self.position >= n {
            if !b.charge(WorkLimit {
                objects: 1,
                bytes: 1,
                ..Default::default()
            }) {
                return SlugByte::BudgetExhausted;
            }
            let byte = self.suffix.get(self.position - n);
            self.position += 1;
            return SlugByte::Byte(byte);
        }
        loop {
            let (rope, at) = self.walking.unwrap_or((self.base, self.position));
            if !b.charge(WorkLimit {
                objects: 1,
                bytes: 1,
                ..Default::default()
            }) {
                return SlugByte::BudgetExhausted;
            }
            match self.memory.hop(rope, at) {
                Ok(byte) => {
                    self.walking = None;
                    self.position += 1;
                    return SlugByte::Byte(byte);
                }
                Err(next) => self.walking = Some(next),
            }
        }
    }
}
impl HeadingSlugSession {
    pub fn get<'a>(
        &'a self,
        doc: &'a Document,
        id: HeadingId,
    ) -> Result<Option<CurrentSlug<'a>>, LayoutError> {
        self.check(doc)?;
        if !self.coherent(doc) {
            return Err(LayoutError::StaleContinuation);
        }
        let c = self.core.as_ref().unwrap();
        let Some(entry) = c.index.entries.get(&id.0) else {
            return Ok(None);
        };
        let suffix = if entry.ordinal == 0 {
            Small::EMPTY
        } else {
            let mut reverse = [0; 32];
            let mut n = entry.ordinal;
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
            out.bytes[0] = b'-';
            out.len = (len + 1) as u8;
            for at in 0..len {
                out.bytes[at + 1] = reverse[len - at - 1];
            }
            out
        };
        Ok(Some(CurrentSlug {
            target: AnchorTarget {
                heading: Some(id),
                source: self.headings[&id.0].source.start,
                provisional: !doc.is_finished(),
            },
            memory: &c.index.mem,
            base: entry.rope,
            suffix,
            position: 0,
            walking: None,
        }))
    }
    /// Close the owner and drain continuations, active leases, normalized pages,
    /// indexes, dependency maps and queued child updates under Budget. Tokens
    /// become invalid immediately. Repeated calls continue the same retirement.
    pub fn retire(&mut self, doc: &Document, b: &mut Budget) -> Result<Completion, LayoutError> {
        if doc.id() != self.document {
            return Err(LayoutError::WrongDocument);
        }
        if self.closed {
            return Ok(Completion::Covered);
        }
        if !self.closing {
            if !step(b) {
                return Ok(Completion::BudgetExhausted);
            }
            self.closing = true;
            self.inbox.borrow_mut().closed = true;
        }
        loop {
            if let Some(job) = &mut self.job {
                match self.cx.poll(job.as_mut(), b) {
                    Poll::Pending => return Ok(Completion::BudgetExhausted),
                    Poll::Ready(c) => {
                        self.job = None;
                        self.core = Some(c);
                    }
                }
                continue;
            }
            if let Some(cursor) = &mut self.cursor {
                if !cursor.retire(b) {
                    return Ok(Completion::BudgetExhausted);
                }
                self.cursor = None;
            }
            if self.resolver.retire(b) != Completion::Covered {
                return Ok(Completion::BudgetExhausted);
            }
            if !step(b) {
                return Ok(Completion::BudgetExhausted);
            }
            if let Some(batch) = self.batches.pop_front() {
                if let Ok(u) = Arc::try_unwrap(batch.update) {
                    self.retired.push_back(u);
                }
                continue;
            }
            if let Some(u) = self.retired.back_mut() {
                if let Some(body) = u.body_updates.pop() {
                    self.retired.push_back(body.into_local());
                } else {
                    self.retired.pop_back();
                }
                continue;
            }
            if self.dirty.pop_first().is_some()
                || self.titles.pop_first().is_some()
                || self.headings.pop_first().is_some()
                || self.bindings.pop_first().is_some()
                || self.dependencies.pop_first().is_some()
                || self.style_updates.pop_first().is_some()
                || self.inbox.borrow_mut().cancel.pop_front().is_some()
            {
                continue;
            }
            if let Some(c) = &mut self.core {
                // Move the graph once; never drop a suspended mutation lane.
                if c.tape.len != usize::MAX {
                    let mut c = self.core.take().unwrap();
                    let cx = self.cx.clone();
                    self.job = Some(boxed(&cx, async move {
                        while let Some((&id, _)) = c.requests.first_key_value() {
                            c.index.mem.cx.charge(super::runtime::Op::Retire).await;
                            let r = c.requests.remove(&id).unwrap();
                            c.sync();
                            if let Some(n) = r.node {
                                c.index.unregister(n).await;
                            }
                            if let Some(buffer) = r.buffer {
                                c.index.mem.drop_buffer_owner(buffer);
                            }
                        }
                        while !c.entities.is_empty() {
                            c.index.mem.cx.charge(super::runtime::Op::Retire).await;
                            c.entities.pop_first();
                            c.sync();
                        }
                        c.index.close().await;
                        c.projection.close(&c.index.mem);
                        c.projection.retire(&mut c.index.mem).await;
                        c.index.mem.retire().await;
                        loop {
                            c.index.mem.cx.charge(super::runtime::Op::Retire).await;
                            if !c.tape.retire_page() {
                                break;
                            }
                            c.index.mem.cx.storage(
                                5,
                                c.tape.capacity(),
                                c.tape.directory_entries(),
                                0,
                            );
                        }
                        c.index.mem.close_buffers().await;
                        c.tape.len = usize::MAX;
                        c
                    }));
                    continue;
                }
            }
            self.backlog_bytes = 0;
            self.closed = true;
            return Ok(Completion::Covered);
        }
    }
}

impl HeadingSlugSession {
    /// Temporarily use the renderer's one canonical resolver. Both owners must
    /// refer to the same document; the memo is always returned, including errors.
    pub fn advance_with_resolver(
        &mut self,
        doc: &Document,
        resolver: &mut LinkResolver,
        b: &mut Budget,
    ) -> Result<Completion, LayoutError> {
        self.check(doc)?;
        resolver.check(doc)?;
        std::mem::swap(&mut self.resolver, resolver);
        let result = self.advance(doc, b);
        std::mem::swap(&mut self.resolver, resolver);
        result
    }
    pub fn fragment_with_resolver(
        &mut self,
        doc: &Document,
        resolver: &mut LinkResolver,
        r: &mut CurrentFragment,
        b: &mut Budget,
    ) -> Result<crate::FragmentStep, LayoutError> {
        self.check(doc)?;
        resolver.check(doc)?;
        std::mem::swap(&mut self.resolver, resolver);
        let result = self.advance_fragment(doc, r, b);
        std::mem::swap(&mut self.resolver, resolver);
        result
    }
}

fn update_bytes(u: &Update) -> usize {
    std::mem::size_of::<Update>()
        + u.display_shape.capacity_bytes()
        + u.dirty.capacity() * std::mem::size_of::<Span>()
        + u.references_changed.capacity() * std::mem::size_of::<crate::SymbolId>()
        + u.tables_changed.capacity() * std::mem::size_of::<crate::TableId>()
        + u.footnotes_changed.capacity() * std::mem::size_of::<crate::FootnoteId>()
        + u.closed_containers.capacity() * std::mem::size_of::<crate::ContainerId>()
        + u.loosened_lists.capacity() * std::mem::size_of::<crate::ContainerId>()
        + u.rows_changed.capacity() * std::mem::size_of::<crate::RowChange>()
        + u.changes.capacity() * std::mem::size_of::<crate::Change>()
        + u.body_updates.capacity() * std::mem::size_of::<crate::BodyUpdate>()
}
