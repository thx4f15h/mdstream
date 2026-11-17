//! Shared target predicates and paid, nonrecursive candidate selection.
//! Definitions change symbols only. Guard leaves follow stable target slots;
//! no reverse-use list or ancestor chain is copied into an object.
use crate::{Budget, Document, DocumentId, LayoutError, LinkId, LinkTarget, SymbolId, WorkLimit};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Guard {
    #[default]
    Never,
    Always,
    Node(usize),
}
#[derive(Clone, Copy, Debug)]
enum Node {
    Target(LinkId),
    Or(usize, usize),
}
#[derive(Default)]
pub(crate) struct Guards {
    nodes: Vec<Node>,
    allocations: u64,
    pub(crate) first_use: Option<usize>,
}
impl Guards {
    fn push(&mut self, node: Node) -> Guard {
        if self.nodes.len() == self.nodes.capacity() {
            self.allocations += 1;
        }
        self.nodes.push(node);
        Guard::Node(self.nodes.len() - 1)
    }
    pub(crate) fn target(&mut self, id: LinkId) -> Guard {
        self.push(Node::Target(id))
    }
    pub(crate) fn or(&mut self, a: Guard, b: Guard) -> Guard {
        match (a, b) {
            (Guard::Always, _) | (_, Guard::Always) => Guard::Always,
            (Guard::Never, b) => b,
            (a, Guard::Never) => a,
            (a, b) if a == b => a,
            (Guard::Node(a), Guard::Node(b)) => self.push(Node::Or(a, b)),
        }
    }
    pub(crate) fn use_at(&mut self, start: usize) {
        self.first_use = Some(self.first_use.map_or(start, |old| old.min(start)));
    }
    pub(crate) fn stats(&self) -> crate::ArenaStats {
        crate::ArenaStats {
            nodes: self.nodes.len(),
            capacity_bytes: self.nodes.capacity() * std::mem::size_of::<Node>(),
            allocation_calls: self.allocations,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Gate {
    pub(crate) root: Guard,
    pub(crate) positive: bool,
}
impl Gate {
    pub(crate) fn unless(root: Guard) -> Self {
        Self {
            root,
            positive: false,
        }
    }
    pub(crate) fn only(root: Guard) -> Self {
        Self {
            root,
            positive: true,
        }
    }
    pub(crate) fn unconditional(self) -> bool {
        matches!(
            (self.root, self.positive),
            (Guard::Never, false) | (Guard::Always, true)
        )
    }
    fn selected(self, value: bool) -> bool {
        self.positive == value
    }
}

/// Selection and resolution are separate from O(1) raw destination metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkDisposition {
    Active,
    Unresolved,
    Suppressed,
}
/// Actual conditional metadata work, independent of parser counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LinkResolutionWork {
    pub guard_steps: u64,
    pub target_reads: u64,
    pub parent_steps: u64,
    pub owner_steps: u64,
    /// Entries removed by explicit budgeted retirement; cumulative across reset.
    pub retired_entries: u64,
}
#[derive(Clone, Copy, Default)]
struct Owner {
    active: Option<LinkId>,
    unresolved: Option<LinkId>,
}
impl Owner {
    fn id(self) -> Option<LinkId> {
        self.active.or(self.unresolved)
    }
}
pub(crate) struct Resolution {
    pub(crate) styles: crate::inline_styles::Resolver,
    token: usize,
    retiring: bool,
    values: BTreeMap<usize, bool>,
    dispositions: BTreeMap<LinkId, LinkDisposition>,
    owners: BTreeMap<LinkId, Owner>,
    pub(crate) work: LinkResolutionWork,
    pub(crate) dependencies: Option<Vec<(SymbolId, u64)>>,
}
fn memo_token() -> usize {
    static NEXT: AtomicUsize = AtomicUsize::new(1);
    NEXT.try_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
        next.checked_add(1)
    })
    .expect("link resolver identity space exhausted")
}
impl Default for Resolution {
    fn default() -> Self {
        Self::empty(memo_token())
    }
}
impl Resolution {
    fn empty(token: usize) -> Self {
        Self {
            token,
            styles: crate::inline_styles::Resolver::default(),
            retiring: false,
            values: BTreeMap::new(),
            dispositions: BTreeMap::new(),
            owners: BTreeMap::new(),
            work: LinkResolutionWork::default(),
            dependencies: None,
        }
    }
    pub(crate) fn take(&mut self) -> Self {
        let empty = Self::empty(self.token);
        std::mem::replace(self, empty)
    }
    pub(crate) fn memo_entries(&self) -> usize {
        self.values.len() + self.dispositions.len() + self.owners.len() + self.styles.entries()
    }
    pub(crate) fn retire(&mut self, budget: &mut Budget) -> crate::Completion {
        if !self.retiring {
            self.retiring = true;
            self.token = memo_token();
        }
        let before = self.styles.work.retired_entries;
        let completion = self.styles.retire(budget);
        self.work.retired_entries += self.styles.work.retired_entries - before;
        if completion != crate::Completion::Covered {
            return crate::Completion::BudgetExhausted;
        }
        while self.memo_entries() != 0 {
            if !charge(budget, 0, 0) {
                return crate::Completion::BudgetExhausted;
            }
            if !self.values.is_empty() {
                self.values.pop_first();
            } else if !self.dispositions.is_empty() {
                self.dispositions.pop_first();
            } else {
                self.owners.pop_first();
            }
            self.work.retired_entries += 1;
        }
        // Copy-only payload vectors have one allocation release, no node walk.
        self.dependencies = None;
        crate::Completion::Covered
    }
}
pub(crate) fn available(document: &Document, id: LinkId) -> bool {
    if !document.link_order.active(id) {
        return false;
    }
    match document.links[id.0] {
        LinkTarget::Direct(_) | LinkTarget::Email(_) | LinkTarget::Www(_) => true,
        LinkTarget::Reference(symbol) => {
            crate::references::Lookup::resolved(&document.references, symbol)
        }
    }
}
fn charge(budget: &mut Budget, recovery: usize, dependencies: usize) -> bool {
    budget.charge(WorkLimit {
        queries: 1,
        objects: 1,
        recovery,
        dependencies,
        ..Default::default()
    })
}
struct Evaluation {
    root: Option<usize>,
    stack: crate::inline_styles::Pages<(usize, u8)>,
    result: Option<bool>,
}
impl Evaluation {
    fn new(root: Guard) -> Self {
        Self {
            root: match root {
                Guard::Node(i) => Some(i),
                _ => None,
            },
            stack: Default::default(),
            result: match root {
                Guard::Never => Some(false),
                Guard::Always => Some(true),
                _ => None,
            },
        }
    }
    fn advance(
        &mut self,
        document: &Document,
        resolution: &mut Resolution,
        budget: &mut Budget,
        recovery: usize,
    ) -> bool {
        while self.result.is_none() {
            let (id, phase) = self.stack.last().or(self.root.map(|i| (i, 0))).unwrap();
            let node = document.link_guards.nodes[id];
            let symbol = match node {
                Node::Target(link) => match document.links[link.0] {
                    LinkTarget::Reference(s) => Some(s),
                    _ => None,
                },
                _ => None,
            };
            if !charge(
                budget,
                recovery,
                usize::from(symbol.is_some() && resolution.dependencies.is_some()),
            ) {
                return false;
            }
            if self.stack.is_empty() {
                self.root = None;
            } else {
                self.stack.pop();
            }
            resolution.work.guard_steps += 1;
            if resolution.values.contains_key(&id) {
                if self.stack.is_empty() {
                    self.result = Some(resolution.values[&id]);
                }
                continue;
            }
            match node {
                Node::Target(link) => {
                    resolution.work.target_reads += 1;
                    if let (Some(symbol), Some(dependencies)) =
                        (symbol, &mut resolution.dependencies)
                    {
                        dependencies.push((symbol, document.references.semantic_revision(symbol)));
                    }
                    resolution.values.insert(id, available(document, link));
                }
                Node::Or(a, b) => match phase {
                    0 => {
                        self.stack.push((id, 1));
                        self.stack.push((a, 0));
                    }
                    1 if resolution.values[&a] => {
                        resolution.values.insert(id, true);
                    }
                    1 => {
                        self.stack.push((id, 2));
                        self.stack.push((b, 0));
                    }
                    _ => {
                        let value = resolution.values[&b];
                        resolution.values.insert(id, value);
                    }
                },
            }
            if self.stack.is_empty() {
                self.result = Some(resolution.values[&id]);
            }
        }
        true
    }
}

/// Revision-bound candidate activation query. None from advance means the
/// shared budget was exhausted; its guard stack and memo survive that frame.
/// Construction and raw metadata lookup are O(1), without evaluating children.
pub struct LinkActivity {
    document: DocumentId,
    revision: u64,
    id: LinkId,
    gate: Gate,
    evaluation: Evaluation,
    stage: u8,
    result: Option<LinkDisposition>,
    resolution: Resolution,
    memo: Option<usize>,
}
impl LinkActivity {
    /// Pay removal of a paused guard stack before abandoning this activity.
    pub fn retire_continuation(&mut self, b: &mut Budget) -> crate::Completion {
        while !self.evaluation.stack.is_empty() {
            if !charge(b, 0, 0) {
                return crate::Completion::BudgetExhausted;
            }
            self.evaluation.stack.pop();
        }
        crate::Completion::Covered
    }
    pub fn continuation_capacity(&self) -> usize {
        self.evaluation.stack.capacity_bytes()
    }
    pub(crate) fn continuation_entries(&self) -> usize {
        self.evaluation.stack.len()
    }
    pub(crate) fn retire_one(&mut self) {
        self.evaluation.stack.pop();
    }
    pub fn rebind_with(
        &mut self,
        document: &Document,
        resolver: &LinkResolver,
    ) -> Result<(), LayoutError> {
        if self.document != document.id() {
            return Err(LayoutError::WrongDocument);
        }
        resolver.retained(document, self.revision)?;
        self.revision = document.revision();
        Ok(())
    }

    pub(crate) fn new(document: &Document, id: LinkId) -> Option<Self> {
        document.link_info(id)?;
        let gate = document.link_order.gate(id);
        Some(Self {
            document: document.id(),
            revision: document.revision(),
            id,
            gate,
            evaluation: Evaluation::new(gate.root),
            stage: 0,
            result: None,
            resolution: Resolution::default(),
            memo: None,
        })
    }
    pub(crate) fn rebind_shared_revision(&mut self, document: &Document) {
        self.revision = document.revision();
    }
    pub fn work(&self) -> LinkResolutionWork {
        self.resolution.work
    }
    /// Standalone advance owns a memo. Retire it before dropping a large query
    /// at a frame boundary; advance_with keeps the memo in its shared resolver.
    pub fn memo_entries(&self) -> usize {
        self.resolution.memo_entries()
    }
    pub fn retire(&mut self, budget: &mut Budget) -> crate::Completion {
        self.resolution.retiring = true;
        if self.retire_continuation(budget) != crate::Completion::Covered {
            return crate::Completion::BudgetExhausted;
        }
        self.evaluation = Evaluation::new(Guard::Never);
        self.result = None;
        self.stage = 0;
        self.resolution.retire(budget)
    }

    pub fn advance(
        &mut self,
        document: &Document,
        budget: &mut Budget,
    ) -> Result<Option<LinkDisposition>, LayoutError> {
        let mut resolution = self.resolution.take();
        let result = self.advance_in(document, &mut resolution, budget, 0);
        self.resolution = resolution;
        result
    }
    pub(crate) fn advance_in(
        &mut self,
        document: &Document,
        resolution: &mut Resolution,
        budget: &mut Budget,
        recovery: usize,
    ) -> Result<Option<LinkDisposition>, LayoutError> {
        if self.document != document.id() {
            return Err(LayoutError::WrongDocument);
        }
        if self.revision != document.revision() {
            return Err(LayoutError::StaleContinuation);
        }
        if self.resolution.retiring || resolution.retiring {
            return Err(LayoutError::StaleContinuation);
        }
        if self.memo != Some(resolution.token) {
            self.stage = 0;
            self.result = None;
            self.evaluation = Evaluation::new(self.gate.root);
            self.memo = Some(resolution.token);
        }
        if self.result.is_some() {
            return Ok(self.result);
        }
        if self.stage == 0 {
            if !charge(budget, recovery, 0) {
                return Ok(None);
            }
            if let Some(&result) = resolution.dispositions.get(&self.id) {
                self.result = Some(result);
                return Ok(self.result);
            }
            self.stage = 1;
        }
        if self.stage == 1 {
            if !self
                .evaluation
                .advance(document, resolution, budget, recovery)
            {
                return Ok(None);
            }
            if !self.gate.selected(self.evaluation.result.unwrap()) {
                self.result = Some(LinkDisposition::Suppressed);
                resolution
                    .dispositions
                    .insert(self.id, LinkDisposition::Suppressed);
                return Ok(self.result);
            }
            self.stage = 2;
        }
        let symbol = match document.links[self.id.0] {
            LinkTarget::Reference(s) => Some(s),
            _ => None,
        };
        if !charge(
            budget,
            recovery,
            usize::from(symbol.is_some() && resolution.dependencies.is_some()),
        ) {
            return Ok(None);
        }
        resolution.work.target_reads += 1;
        if let (Some(symbol), Some(dependencies)) = (symbol, &mut resolution.dependencies) {
            dependencies.push((symbol, document.references.semantic_revision(symbol)));
        }
        let result = if available(document, self.id) {
            LinkDisposition::Active
        } else {
            LinkDisposition::Unresolved
        };
        self.result = Some(result);
        resolution.dispositions.insert(self.id, result);
        Ok(Some(result))
    }
}

struct OwnerTask {
    current: Option<LinkId>,
    path: crate::inline_styles::Pages<(LinkId, bool)>,
    activity: Option<LinkActivity>,
    stage: u8,
    base: Owner,
    result: Option<Option<LinkId>>,
}
impl OwnerTask {
    fn new(id: LinkId) -> Self {
        Self {
            current: Some(id),
            path: Default::default(),
            activity: None,
            stage: 0,
            base: Owner::default(),
            result: None,
        }
    }
    fn advance(
        &mut self,
        doc: &Document,
        resolution: &mut Resolution,
        budget: &mut Budget,
        recovery: usize,
    ) -> bool {
        while self.result.is_none() {
            if self.stage == 3 {
                if self.path.is_empty() {
                    self.result = Some(self.base.id());
                    break;
                }
                if !charge(budget, recovery, 0) {
                    return false;
                }
                let (id, unresolved) = self.path.pop().unwrap();
                if self.base.active.is_none() && unresolved {
                    self.base.unresolved = Some(id);
                }
                resolution.owners.insert(id, self.base);
                resolution.work.owner_steps += 1;
                continue;
            }
            let Some(id) = self.current else {
                self.stage = 3;
                continue;
            };
            match self.stage {
                0 => {
                    if !charge(budget, recovery, 0) {
                        return false;
                    }
                    resolution.work.owner_steps += 1;
                    if let Some(&owner) = resolution.owners.get(&id) {
                        self.base = owner;
                        self.stage = 3;
                        continue;
                    }
                    self.activity = LinkActivity::new(doc, id);
                    self.stage = 1;
                }
                1 => {
                    let Some(disposition) = self
                        .activity
                        .as_mut()
                        .unwrap()
                        .advance_in(doc, resolution, budget, recovery)
                        .unwrap()
                    else {
                        return false;
                    };
                    if disposition == LinkDisposition::Active {
                        self.base = Owner {
                            active: Some(id),
                            unresolved: None,
                        };
                        resolution.owners.insert(id, self.base);
                        self.stage = 3;
                    } else {
                        self.path
                            .push((id, disposition == LinkDisposition::Unresolved));
                        self.activity = None;
                        self.stage = 2;
                    }
                }
                _ => {
                    if !charge(budget, recovery, 0) {
                        return false;
                    }
                    resolution.work.parent_steps += 1;
                    self.current = doc.link_order.parent(id);
                    self.stage = 0;
                }
            }
        }
        true
    }
}

pub(crate) struct MarkTask {
    marker: [Option<LinkId>; 2],
    marker_at: usize,
    activity: Option<LinkActivity>,
    owner: Option<OwnerTask>,
    hidden: bool,
    pub(crate) result: Option<(bool, Option<LinkId>)>,
    original: crate::annotations::Mark,
    memo: Option<usize>,
}
impl MarkTask {
    pub(crate) fn valid_memo(&self, r: &Resolution) -> bool {
        self.memo.is_none_or(|t| t == r.token)
    }
    pub(crate) fn retain_revision(&mut self, doc: &Document) {
        if let Some(a) = &mut self.activity {
            a.rebind_shared_revision(doc);
        }
        if let Some(o) = &mut self.owner
            && let Some(a) = &mut o.activity
        {
            a.rebind_shared_revision(doc);
        }
    }
    pub(crate) fn retire_continuation(&mut self, budget: &mut Budget) -> bool {
        loop {
            let a = self
                .activity
                .as_mut()
                .filter(|a| !a.evaluation.stack.is_empty());
            if let Some(a) = a {
                if !charge(budget, 0, 0) {
                    return false;
                }
                a.evaluation.stack.pop();
                continue;
            }
            if let Some(owner) = &mut self.owner {
                if !owner.path.is_empty() {
                    if !charge(budget, 0, 0) {
                        return false;
                    }
                    owner.path.pop();
                    continue;
                }
                if let Some(a) = owner
                    .activity
                    .as_mut()
                    .filter(|a| !a.evaluation.stack.is_empty())
                {
                    if !charge(budget, 0, 0) {
                        return false;
                    }
                    a.evaluation.stack.pop();
                    continue;
                }
            }
            return true;
        }
    }
    pub(crate) fn continuation_capacity(&self) -> usize {
        self.activity
            .as_ref()
            .map_or(0, |a| a.evaluation.stack.capacity_bytes())
            + self.owner.as_ref().map_or(0, |o| {
                o.path.capacity_bytes()
                    + o.activity
                        .as_ref()
                        .map_or(0, |a| a.evaluation.stack.capacity_bytes())
            })
    }
    pub(crate) fn new(mark: crate::annotations::Mark) -> Self {
        Self {
            marker: [mark.hide_owner(), mark.alternative_owner()],
            marker_at: 0,
            activity: None,
            owner: mark.link.map(OwnerTask::new),
            hidden: mark.style.contains(crate::Style::HIDDEN),
            result: None,
            original: mark,
            memo: None,
        }
    }
    pub(crate) fn advance(
        &mut self,
        doc: &Document,
        resolution: &mut Resolution,
        budget: &mut Budget,
        recovery: usize,
    ) -> bool {
        if self.memo != Some(resolution.token) {
            let original = self.original;
            *self = Self::new(original);
            self.memo = Some(resolution.token);
        }
        if self.result.is_some() {
            return true;
        }
        while !self.hidden && self.marker_at < 2 {
            let Some(id) = self.marker[self.marker_at] else {
                self.marker_at += 1;
                continue;
            };
            let activity = self
                .activity
                .get_or_insert_with(|| LinkActivity::new(doc, id).unwrap());
            let Some(disposition) = activity
                .advance_in(doc, resolution, budget, recovery)
                .unwrap()
            else {
                return false;
            };
            self.hidden = disposition == LinkDisposition::Active;
            self.activity = None;
            self.marker_at += 1;
        }
        let owner = if let Some(owner) = &mut self.owner {
            if !owner.advance(doc, resolution, budget, recovery) {
                return false;
            }
            owner.result.unwrap()
        } else {
            None
        };
        self.result = Some((self.hidden, owner));
        true
    }
}
impl Resolution {
    pub(crate) fn capturing() -> Self {
        Self {
            dependencies: Some(Vec::new()),
            ..Self::default()
        }
    }
    pub(crate) fn needs(doc: &Document, mark: crate::annotations::Mark) -> bool {
        if mark.hide_alternative.is_some() {
            return true;
        }
        if !mark.style.contains(crate::Style::HIDDEN)
            && mark
                .hide_owner()
                .is_some_and(|id| !doc.link_order.gate(id).unconditional())
        {
            return true;
        }
        mark.link.is_some_and(|id| {
            !doc.link_order.gate(id).unconditional()
                || (!available(doc, id) && doc.link_order.parent(id).is_some())
        })
    }
    pub(crate) fn mark(
        &mut self,
        doc: &Document,
        mut mark: crate::annotations::Mark,
    ) -> crate::annotations::Mark {
        if Self::needs(doc, mark) {
            let mut task = MarkTask::new(mark);
            assert!(task.advance(doc, self, &mut Budget::new(WorkLimit::unlimited()), 0));
            let (hidden, owner) = task.result.unwrap();
            if hidden {
                mark.style = mark.style.union(crate::Style::HIDDEN);
            }
            mark.link = owner;
        } else {
            mark.style = mark.effective_style(&doc.references, &doc.links);
        }
        mark
    }
}

/// A view/task-local memo shared by candidate queries, display and layout.
/// Construction is O(1). It is bound to one immutable document revision; an
/// append/finish requires budgeted retirement and reset (or a fresh resolver).
/// Each map entry costs one query + object to retire. Drop itself is O(entries):
/// move the memo out of an old pass/cursor and retire it before dropping it at
/// a frame boundary. Do not accumulate abandoned revision memos. Cross-revision
/// payload caches use DependencyKey for their actual nested dependencies.
pub struct LinkResolver {
    retained_revision: u64,
    source_len: usize,
    link_count: usize,
    document: DocumentId,
    revision: u64,
    pub(crate) resolution: Resolution,
}
impl LinkResolver {
    pub(crate) fn slug_binding(&self) -> (u64, usize, usize) {
        (self.revision, self.source_len, self.link_count)
    }
    /// The caller has visited every intervening typed effect under Budget.
    pub(crate) fn slug_rebind(
        &mut self,
        doc: &Document,
        base: u64,
        preserved: bool,
    ) -> Result<bool, LayoutError> {
        if self.document != doc.id() {
            return Err(LayoutError::WrongDocument);
        }
        if self.revision != base {
            return Err(LayoutError::StaleContinuation);
        }
        if !preserved || self.resolution.retiring {
            return Ok(false);
        }
        self.revision = doc.revision();
        self.source_len = doc.source().len();
        self.link_count = doc.links().len();
        Ok(true)
    }
    pub(crate) fn slug_invalidate_continuations(&mut self, doc: &Document) {
        self.retained_revision = doc.revision();
    }
    pub(crate) fn slug_append_style(
        &mut self,
        doc: &Document,
        scope: usize,
        event: crate::DisplayShapeStyleEvent,
    ) -> bool {
        self.retained_revision = doc.revision();
        self.resolution.styles.append_style(doc, scope, event)
    }
    /// Retain memo values and append-only canonical style recovery, while
    /// invalidating older prepared display continuations. The legacy stricter
    /// rebind_display_shape contract remains unchanged.
    pub fn rebind_current_slugs(
        &mut self,
        document: &Document,
        update: &crate::Update,
    ) -> Result<bool, LayoutError> {
        use crate::DisplayShapeEffectKind as E;
        if self.document != document.id() || update.document != Some(self.document) {
            return Err(LayoutError::WrongDocument);
        }
        if self.revision != update.base_revision || document.revision() != update.revision {
            return Err(LayoutError::StaleContinuation);
        }
        if self.resolution.retiring || !update.display_shape.enabled() {
            return Ok(false);
        }
        for effect in update.display_shape.events() {
            let changed = match effect.kind {
                E::ScopeEvent { scope, event, .. } => {
                    !self.slug_append_style(document, scope, event)
                }
                E::Mark { source, mark } => {
                    if source.start < self.source_len {
                        self.retained_revision = document.revision();
                    }
                    source.start < self.source_len
                        && (mark.base_hidden
                            || mark.link.is_some()
                            || mark.hide_owner.is_some()
                            || mark.hide_alternative.is_some())
                }
                E::Replacement { source, .. } => source.start < self.source_len,
                E::TruncateReplacements { .. }
                | E::Rollback(_)
                | E::Math { .. }
                | E::Footnote { .. }
                | E::FootnoteReference { .. } => true,
                E::Link { id, .. }
                | E::LinkGate { id, .. }
                | E::LinkParent { id, .. }
                | E::LinkActive { id, .. }
                | E::LinkState { id, .. } => id.0 < self.link_count,
                E::Reference { id, .. } => document
                    .references()
                    .get(id.0)
                    .is_some_and(|r| r.first_use.is_some_and(|at| at < self.source_len)),
                _ => false,
            };
            if changed {
                return Ok(false);
            }
        }
        self.revision = update.revision;
        self.source_len = document.source().len();
        self.link_count = document.links().len();
        Ok(true)
    }
    /// Retain the current memo only when typed effects prove all previous
    /// bindings and classified scopes unchanged. Does not replay source text.
    pub fn rebind_display_shape(
        &mut self,
        document: &Document,
        update: &crate::Update,
    ) -> Result<bool, LayoutError> {
        use crate::DisplayShapeEffectKind as E;
        if self.document != document.id() || update.document != Some(self.document) {
            return Err(LayoutError::WrongDocument);
        }
        if self.revision != update.base_revision || document.revision() != update.revision {
            return Err(LayoutError::StaleContinuation);
        }
        if self.resolution.retiring || !update.display_shape.enabled() {
            return Ok(false);
        }
        for event in update.display_shape.events() {
            let changed = match event.kind {
                E::ScopeEvent { scope, .. } => scope < self.source_len,
                E::Mark { source, .. } | E::Replacement { source, .. } => {
                    source.start < self.source_len
                }
                E::TruncateReplacements { .. } => true,
                E::Link { id, .. }
                | E::LinkGate { id, .. }
                | E::LinkParent { id, .. }
                | E::LinkActive { id, .. }
                | E::LinkState { id, .. } => id.0 < self.link_count,
                E::Reference { id, .. } => document
                    .references()
                    .get(id.0)
                    .is_some_and(|r| r.first_use.is_some_and(|at| at < self.source_len)),
                E::Footnote { .. }
                | E::FootnoteReference { .. }
                | E::Math { .. }
                | E::Rollback(_) => true,
                _ => false,
            };
            if changed {
                return Ok(false);
            }
        }
        self.revision = update.revision;
        self.source_len = document.source().len();
        self.link_count = document.links().len();
        Ok(true)
    }
    pub fn can_rebind(&self, document: &Document, revision: u64) -> bool {
        self.retained(document, revision).is_ok()
    }
    pub fn document_id(&self) -> DocumentId {
        self.document
    }
    pub(crate) fn retained(&self, document: &Document, revision: u64) -> Result<(), LayoutError> {
        self.check(document)?;
        if revision < self.retained_revision || revision > self.revision {
            return Err(LayoutError::StaleContinuation);
        }
        Ok(())
    }
    pub(crate) fn new(document: &Document) -> Self {
        Self {
            retained_revision: document.revision(),
            source_len: document.source().len(),
            link_count: document.links().len(),
            document: document.id(),
            revision: document.revision(),
            resolution: Resolution::default(),
        }
    }
    pub(crate) fn check(&self, document: &Document) -> Result<(), LayoutError> {
        self.check_binding(document.id(), document.revision())
    }
    pub(crate) fn check_binding(
        &self,
        document: DocumentId,
        revision: u64,
    ) -> Result<(), LayoutError> {
        if self.document != document {
            Err(LayoutError::WrongDocument)
        } else if self.revision != revision || self.resolution.retiring {
            Err(LayoutError::StaleContinuation)
        } else {
            Ok(())
        }
    }
    pub(crate) fn empty_like(&self) -> Self {
        Self {
            retained_revision: self.retained_revision,
            source_len: self.source_len,
            link_count: self.link_count,
            document: self.document,
            revision: self.revision,
            resolution: Resolution::default(),
        }
    }
    /// Diagnostic payload capacity; excludes private BTreeMap slack/headers.
    pub fn capacity_bytes(&self) -> usize {
        self.resolution.values.len() * std::mem::size_of::<(usize, bool)>()
            + self.resolution.dispositions.len() * std::mem::size_of::<(LinkId, LinkDisposition)>()
            + self.resolution.owners.len() * std::mem::size_of::<(LinkId, Owner)>()
            + self.resolution.styles.capacity_bytes()
    }
    pub fn memo_entries(&self) -> usize {
        self.resolution.memo_entries()
    }
    /// Cancel selection work and remove at most the paid number of map entries.
    /// Once Covered, destruction/reset has no historical map-node traversal.
    pub fn retire(&mut self, budget: &mut Budget) -> crate::Completion {
        self.resolution.retire(budget)
    }
    /// O(1) rebinding after retirement. A nonempty old memo returns InvalidRange.
    pub fn reset(&mut self, document: &Document) -> Result<(), LayoutError> {
        if self.memo_entries() != 0 {
            return Err(LayoutError::InvalidRange);
        }
        self.retained_revision = document.revision();
        self.source_len = document.source().len();
        self.link_count = document.links().len();
        self.document = document.id();
        self.revision = document.revision();
        self.resolution.token = memo_token();
        self.resolution.retiring = false;
        self.resolution.dependencies = None;
        Ok(())
    }
    pub fn work(&self) -> LinkResolutionWork {
        self.resolution.work
    }
    /// Current conditional-style memo entries, maintained in O(1). A consumer
    /// spanning several source tiles can retain its lease until that scope ends.
    pub fn style_memo_entries(&self) -> usize {
        self.resolution.styles.entries()
    }
    pub fn style_work(&self) -> crate::StyleResolutionWork {
        self.resolution.styles.work
    }
}
impl LinkActivity {
    pub fn advance_with(
        &mut self,
        document: &Document,
        resolver: &mut LinkResolver,
        budget: &mut Budget,
    ) -> Result<Option<LinkDisposition>, LayoutError> {
        resolver.check(document)?;
        self.advance_in(document, &mut resolver.resolution, budget, 0)
    }
}
impl Resolution {
    pub(crate) fn hidden(&mut self, doc: &Document, mark: crate::annotations::Mark) -> bool {
        if mark.style.contains(crate::Style::HIDDEN) {
            return true;
        }
        for id in [mark.hide_owner(), mark.alternative_owner()]
            .into_iter()
            .flatten()
        {
            if doc.link_order.gate(id).unconditional() {
                if available(doc, id) {
                    return true;
                }
            } else {
                let mut activity = LinkActivity::new(doc, id).unwrap();
                if activity
                    .advance_in(doc, self, &mut Budget::new(WorkLimit::unlimited()), 0)
                    .unwrap()
                    == Some(LinkDisposition::Active)
                {
                    return true;
                }
            }
        }
        false
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    fn entries(resolver: &LinkResolver) -> usize {
        resolver.resolution.values.len()
            + resolver.resolution.dispositions.len()
            + resolver.resolution.owners.len()
    }
    #[test]
    fn review_revision_memo_replacement_requires_paid_retirement() {
        let mut doc = Document::new();
        doc.append(&format!("[a {}](u)\n\n", "[r] ".repeat(512)))
            .unwrap();
        let parent = doc.links_in(crate::Span::new(0, 1)).next().unwrap().0;
        let mut resolver = doc.link_resolver();
        let mut activity = doc.link_activity(parent).unwrap();
        assert_eq!(
            activity
                .advance_with(
                    &doc,
                    &mut resolver,
                    &mut Budget::new(WorkLimit::unlimited())
                )
                .unwrap(),
            Some(LinkDisposition::Active)
        );
        let old = entries(&resolver);
        assert!(old > 1000);
        doc.append("x").unwrap();
        let mut zero = Budget::new(WorkLimit::default());
        assert_eq!(
            resolver.retire(&mut zero),
            crate::Completion::BudgetExhausted
        );
        assert_eq!(entries(&resolver), old);
        assert_eq!(zero.used(), WorkLimit::default());
        assert_eq!(resolver.reset(&doc), Err(LayoutError::InvalidRange));
        let before = resolver.work().retired_entries;
        for left in (0..old).rev() {
            let mut budget = Budget::new(WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            });
            let done = resolver.retire(&mut budget);
            assert_eq!(entries(&resolver), left);
            assert_eq!(budget.used().objects, 1);
            assert_eq!(budget.used().queries, 1);
            assert_eq!(
                done,
                if left == 0 {
                    crate::Completion::Covered
                } else {
                    crate::Completion::BudgetExhausted
                }
            );
        }
        assert_eq!(resolver.work().retired_entries - before, old as u64);
        assert_eq!(resolver.retire(&mut zero), crate::Completion::Covered);
        resolver.reset(&doc).unwrap();
        assert_eq!(resolver.memo_entries(), 0);
        assert_eq!(resolver.work().retired_entries - before, old as u64);
    }
}
