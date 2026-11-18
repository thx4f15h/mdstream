//! Stable row registrations. Selected intervals with a common real endpoint
//! share one physical mask; logical choices never depend on its representative.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Opaque process-unique identity of one view's row registration. Never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TableRowFoldId(usize);
impl TableRowFoldId {
    pub fn index(self) -> usize {
        self.0
    }
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        Self(
            NEXT.try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
                .expect("table row identity space exhausted"),
        )
    }
}

/// Body row zero is forbidden; the header is row zero and the delimiter is not
/// a row. Fixed ends are exclusive. Only real indexed rows are eligible.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableRowExtent {
    Fixed {
        start: usize,
        end: usize,
    },
    /// May be dormant before start exists. Follows this TableId, never EOF.
    Live {
        start: usize,
    },
    /// Created only by sealing a dormant live range. Exact left-affinity cut.
    SealedEmpty {
        start: usize,
        source: usize,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TableRowPolicy {
    pub visible_body_rows: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableRowFoldInfo {
    pub id: TableRowFoldId,
    pub table: TableId,
    pub extent: TableRowExtent,
    pub rows: Option<Range<usize>>,
    pub closed: bool,
    pub collapsed: bool,
    pub candidate: FoldCandidate,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TableRowWork {
    pub registrations: usize,
    pub selected_groups: usize,
    pub watched_groups: usize,
    pub row_index_nodes: usize,
    pub group_refreshes: u64,
    pub row_endpoint_reads: u64,
    pub row_index_visits: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TableRowFoldProgress {
    pub completion: Completion,
    pub id: Option<FoldId>,
    pub change: Option<FoldChange>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TableRowCandidateStep {
    Candidate(TableRowFoldInfo),
    Complete,
    BudgetExhausted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum End {
    Fixed(usize),
    Live,
}
type GroupKey = (TableId, End);
#[derive(Clone, Copy)]
struct Entry {
    document: DocumentId,
    owner: DocumentId,
    table: TableId,
    extent: TableRowExtent,
    collapsed: bool,
}
impl Entry {
    fn group(self) -> Option<GroupKey> {
        Some((
            self.table,
            match self.extent {
                TableRowExtent::Fixed { end, .. } => End::Fixed(end),
                TableRowExtent::Live { .. } => End::Live,
                TableRowExtent::SealedEmpty { .. } => return None,
            },
        ))
    }
    fn start(self) -> usize {
        match self.extent {
            TableRowExtent::Fixed { start, .. }
            | TableRowExtent::Live { start }
            | TableRowExtent::SealedEmpty { start, .. } => start,
        }
    }
    fn row_key(self, id: TableRowFoldId) -> Option<crate::fold_intervals::Key> {
        let end = match self.extent {
            TableRowExtent::Fixed { end, .. } => end,
            TableRowExtent::Live { .. } => usize::MAX,
            TableRowExtent::SealedEmpty { .. } => return None,
        };
        Some(crate::fold_intervals::key(
            self.start(),
            end,
            FoldId::TableRange(id),
        ))
    }
}
#[derive(Default)]
struct Group {
    members: BTreeSet<(usize, TableRowFoldId)>,
    representative: Option<FoldId>,
}
#[derive(Default)]
pub(super) struct TableRowState {
    entries: BTreeMap<TableRowFoldId, Entry>,
    tables: BTreeSet<(TableId, TableRowFoldId)>,
    rows: BTreeMap<TableId, crate::fold_intervals::Intervals>,
    groups: BTreeMap<GroupKey, Group>,
    watches: BTreeSet<GroupKey>,
    automatic: BTreeMap<TableId, TableRowFoldId>,
    policy: Option<TableRowPolicy>,
    refreshes: u64,
    endpoint_reads: u64,
}
fn frontier(table: &crate::Table) -> usize {
    table.rows().last().map_or(table.delimiter.end, |r| {
        r.source.end.max(table.delimiter.end)
    })
}
fn cost() -> WorkLimit {
    WorkLimit {
        queries: 1,
        objects: 1,
        ..Default::default()
    }
}
impl TableRowState {
    pub(super) fn has(&self, id: TableRowFoldId) -> bool {
        self.entries.contains_key(&id)
    }
    pub(super) fn owns(&self, document: DocumentId, owner: DocumentId, id: TableRowFoldId) -> bool {
        self.entries
            .get(&id)
            .is_some_and(|e| e.document == document && e.owner == owner)
    }
    pub(super) fn collapsed(&self, id: TableRowFoldId) -> bool {
        self.entries.get(&id).is_some_and(|e| e.collapsed)
    }
    pub(super) fn candidate(&self, doc: &Document, id: TableRowFoldId) -> Option<FoldCandidate> {
        let e = *self.entries.get(&id)?;
        if e.document != doc.id() {
            return None;
        }
        let t = doc.table(e.table)?;
        let (source, open, summary) = match e.extent {
            TableRowExtent::SealedEmpty { source, .. } => (Span::new(source, source), false, None),
            TableRowExtent::Fixed { start, end } => {
                let first = t.row(start)?;
                let last = t.row(end - 1)?;
                (
                    Span::new(first.source.start, last.source.end),
                    !last.complete && !doc.is_finished(),
                    Some(first.source),
                )
            }
            TableRowExtent::Live { start } => {
                let open = doc.block(t.block)?.state != SyntaxState::Sealed && !doc.is_finished();
                if let Some(first) = t.row(start) {
                    (
                        Span::new(first.source.start, frontier(t)),
                        open,
                        Some(first.source),
                    )
                } else {
                    let at = frontier(t);
                    (Span::new(at, at), open, None)
                }
            }
        };
        Some(FoldCandidate {
            id: FoldId::TableRange(id),
            parent: Some(FoldId::Table(e.table)),
            source,
            hidden: source,
            summary,
            open,
        })
    }
    fn info(&self, doc: &Document, id: TableRowFoldId) -> Option<TableRowFoldInfo> {
        let e = *self.entries.get(&id)?;
        let candidate = self.candidate(doc, id)?;
        let rows = match e.extent {
            TableRowExtent::Fixed { start, end } => Some(start..end),
            TableRowExtent::Live { start } => {
                let end = doc.table(e.table)?.rows().len();
                (start < end).then_some(start..end)
            }
            TableRowExtent::SealedEmpty { .. } => None,
        };
        Some(TableRowFoldInfo {
            id,
            table: e.table,
            extent: e.extent,
            rows,
            closed: !candidate.open,
            collapsed: e.collapsed,
            candidate,
        })
    }
    pub(super) fn overlaps(&self, table: TableId, rows: Range<usize>) -> bool {
        self.rows
            .get(&table)
            .is_some_and(|index| index.next(Bound::Unbounded, rows.start, rows.end).is_some())
    }
}

#[derive(Clone, Copy)]
struct Binding {
    document: DocumentId,
    view: DocumentId,
    revision: u64,
    version: u64,
}
impl Binding {
    fn new<S>(doc: &Document, view: &FoldView<S>) -> Result<Self, LayoutError> {
        view.check(doc)?;
        Ok(Self {
            document: doc.id(),
            view: view.heights.version().0,
            revision: doc.revision(),
            version: view.version,
        })
    }
    fn check<S>(self, doc: &Document, view: &FoldView<S>) -> Result<(), LayoutError> {
        if self.document != doc.id() {
            return Err(LayoutError::WrongDocument);
        }
        if self.view != view.heights.version().0 {
            return Err(LayoutError::WrongView);
        }
        if self.revision != doc.revision() || self.version != view.version {
            return Err(LayoutError::StaleContinuation);
        }
        view.check(doc)
    }
}
fn validate(doc: &Document, table: TableId, extent: TableRowExtent) -> Result<(), LayoutError> {
    let t = doc.table(table).ok_or(LayoutError::InvalidRange)?;
    match extent {
        TableRowExtent::Fixed { start, end }
            if start > 0 && start < end && end <= t.rows().len() =>
        {
            Ok(())
        }
        TableRowExtent::Live { start } if start > 0 && start < usize::MAX => Ok(()),
        _ => Err(LayoutError::InvalidRange),
    }
}

/// Atomic fixed-overlay/live/prefix registration. Construction reads only
/// indexed metadata. advance pays before allocating an ID or changing any view.
/// Dropping a pending task is free and publishes no partial state.
pub struct TableRowRegistration {
    binding: Binding,
    table: TableId,
    extent: TableRowExtent,
    prefix: bool,
    result: Option<TableRowFoldProgress>,
}
impl TableRowRegistration {
    pub fn fixed_overlay<S>(
        doc: &Document,
        view: &FoldView<S>,
        table: TableId,
        rows: Range<usize>,
    ) -> Result<Self, LayoutError> {
        Self::new(
            doc,
            view,
            table,
            TableRowExtent::Fixed {
                start: rows.start,
                end: rows.end,
            },
            false,
        )
    }
    /// Explicit live tail; start is an existing actual body row. Use prefix for
    /// a dormant first-N policy, never infer eligibility from pending source.
    pub fn live<S>(
        doc: &Document,
        view: &FoldView<S>,
        table: TableId,
        start: usize,
    ) -> Result<Self, LayoutError> {
        let task = Self::new(doc, view, table, TableRowExtent::Live { start }, false)?;
        doc.table(table)
            .and_then(|t| t.row(start))
            .ok_or(LayoutError::InvalidRange)?;
        Ok(task)
    }
    /// Retain the header plus exactly N actual body rows. Reuses this view's
    /// current policy registration for this table, preserving its stable ID.
    /// This explicit action collapses that range; defaults initialize new only.
    pub fn prefix<S>(
        doc: &Document,
        view: &FoldView<S>,
        table: TableId,
        visible_body_rows: usize,
    ) -> Result<Self, LayoutError> {
        let start = visible_body_rows
            .checked_add(1)
            .ok_or(LayoutError::InvalidRange)?;
        Self::new(doc, view, table, TableRowExtent::Live { start }, true)
    }
    fn new<S>(
        doc: &Document,
        view: &FoldView<S>,
        table: TableId,
        extent: TableRowExtent,
        prefix: bool,
    ) -> Result<Self, LayoutError> {
        let binding = Binding::new(doc, view)?;
        validate(doc, table, extent)?;
        Ok(Self {
            binding,
            table,
            extent,
            prefix,
            result: None,
        })
    }
    pub fn advance<S>(
        &mut self,
        doc: &Document,
        view: &mut FoldView<S>,
        budget: &mut Budget,
    ) -> Result<TableRowFoldProgress, LayoutError> {
        self.binding.check(doc, view)?;
        if let Some(result) = self.result {
            return Ok(result);
        }
        if !budget.charge(cost()) {
            return Ok(TableRowFoldProgress {
                completion: Completion::BudgetExhausted,
                id: None,
                change: None,
            });
        }
        let (id, change) = view.publish_table_rows(doc, self.table, self.extent, self.prefix)?;
        self.binding.version = view.version;
        let result = TableRowFoldProgress {
            completion: Completion::Covered,
            id: Some(FoldId::TableRange(id)),
            change: Some(change),
        };
        self.result = Some(result);
        Ok(result)
    }
}

#[derive(Clone, Copy)]
enum Edit {
    Retarget(TableRowExtent),
    Seal,
    Remove,
}
/// Same-ID atomic edits. Invalid/foreign/stale tasks fail before budget use;
/// zero budget leaves choices, masks, CPs, version and ID allocation untouched.
pub struct TableRowEdit {
    binding: Binding,
    id: TableRowFoldId,
    edit: Edit,
    result: Option<TableRowFoldProgress>,
}
impl TableRowEdit {
    pub fn retarget<S>(
        doc: &Document,
        view: &FoldView<S>,
        id: TableRowFoldId,
        extent: TableRowExtent,
    ) -> Result<Self, LayoutError> {
        let task = Self::new(doc, view, id, Edit::Retarget(extent))?;
        validate(doc, view.checked_row_entry(doc, id)?.table, extent)?;
        Ok(task)
    }
    pub fn seal<S>(
        doc: &Document,
        view: &FoldView<S>,
        id: TableRowFoldId,
    ) -> Result<Self, LayoutError> {
        let task = Self::new(doc, view, id, Edit::Seal)?;
        if !matches!(
            view.checked_row_entry(doc, id)?.extent,
            TableRowExtent::Live { .. }
        ) {
            return Err(LayoutError::InvalidRange);
        }
        Ok(task)
    }
    pub fn remove<S>(
        doc: &Document,
        view: &FoldView<S>,
        id: TableRowFoldId,
    ) -> Result<Self, LayoutError> {
        Self::new(doc, view, id, Edit::Remove)
    }
    fn new<S>(
        doc: &Document,
        view: &FoldView<S>,
        id: TableRowFoldId,
        edit: Edit,
    ) -> Result<Self, LayoutError> {
        let binding = Binding::new(doc, view)?;
        view.checked_row_entry(doc, id)?;
        Ok(Self {
            binding,
            id,
            edit,
            result: None,
        })
    }
    pub fn advance<S>(
        &mut self,
        doc: &Document,
        view: &mut FoldView<S>,
        budget: &mut Budget,
    ) -> Result<TableRowFoldProgress, LayoutError> {
        self.binding.check(doc, view)?;
        if let Some(r) = self.result {
            return Ok(r);
        }
        if !budget.charge(cost()) {
            return Ok(TableRowFoldProgress {
                completion: Completion::BudgetExhausted,
                id: None,
                change: None,
            });
        }
        let change = view.edit_table_rows(doc, self.id, self.edit)?;
        self.binding.version = view.version;
        let r = TableRowFoldProgress {
            completion: Completion::Covered,
            id: Some(FoldId::TableRange(self.id)),
            change: Some(change),
        };
        self.result = Some(r);
        Ok(r)
    }
}

/// One existing registration per paid step, including duplicate starts and
/// expanded choices. O(1) cursor; no row or overlap collection is materialized.
pub struct TableRowCandidates {
    binding: Binding,
    table: TableId,
    window: Option<Range<usize>>,
    after: Option<crate::fold_intervals::Key>,
    after_id: Option<TableRowFoldId>,
    done: bool,
}
impl TableRowCandidates {
    pub fn all<S>(doc: &Document, view: &FoldView<S>, table: TableId) -> Result<Self, LayoutError> {
        Self::new(doc, view, table, None)
    }
    pub fn intersecting<S>(
        doc: &Document,
        view: &FoldView<S>,
        table: TableId,
        rows: Range<usize>,
    ) -> Result<Self, LayoutError> {
        view.check(doc)?;
        let t = doc.table(table).ok_or(LayoutError::InvalidRange)?;
        if rows.start > rows.end || rows.end > t.rows().len() {
            return Err(LayoutError::InvalidRange);
        }
        Self::new(doc, view, table, Some(rows))
    }
    fn new<S>(
        doc: &Document,
        view: &FoldView<S>,
        table: TableId,
        window: Option<Range<usize>>,
    ) -> Result<Self, LayoutError> {
        let binding = Binding::new(doc, view)?;
        doc.table(table).ok_or(LayoutError::InvalidRange)?;
        Ok(Self {
            binding,
            table,
            window,
            after: None,
            after_id: None,
            done: false,
        })
    }
    pub fn next<S>(
        &mut self,
        doc: &Document,
        view: &FoldView<S>,
        budget: &mut Budget,
    ) -> Result<TableRowCandidateStep, LayoutError> {
        self.binding.check(doc, view)?;
        if self.done {
            return Ok(TableRowCandidateStep::Complete);
        }
        if !budget.charge(cost()) {
            return Ok(TableRowCandidateStep::BudgetExhausted);
        }
        let state = &view.state.table_rows;
        let id = if let Some(rows) = &self.window {
            state
                .rows
                .get(&self.table)
                .and_then(|index| {
                    index.next(
                        self.after.map_or(Bound::Unbounded, Bound::Excluded),
                        rows.start,
                        rows.end,
                    )
                })
                .map(|k| {
                    self.after = Some(k);
                    let FoldId::TableRange(id) = k.3 else {
                        unreachable!()
                    };
                    id
                })
        } else {
            let lo = self
                .after_id
                .map_or(Bound::Included((self.table, TableRowFoldId(0))), |id| {
                    Bound::Excluded((self.table, id))
                });
            state
                .tables
                .range((
                    lo,
                    Bound::Included((self.table, TableRowFoldId(usize::MAX))),
                ))
                .next()
                .map(|&(_, id)| {
                    self.after_id = Some(id);
                    id
                })
        };
        match id {
            Some(id) => Ok(TableRowCandidateStep::Candidate(
                view.table_row_fold_info(doc, id)?.unwrap(),
            )),
            None => {
                self.done = true;
                Ok(TableRowCandidateStep::Complete)
            }
        }
    }
}

/// Paid new-only policy initialization over Update::new_tables. Does not add a
/// policy field to FoldDefaults or enumerate old tables on ordinary append.
pub struct TableRowAutoFold {
    binding: Binding,
    next: usize,
    end: usize,
    policy: Option<TableRowPolicy>,
}
impl TableRowAutoFold {
    /// Host scheduling may rebind remaining new-table work after an append,
    /// resize or another paid view operation. Stable TableIds and the policy
    /// captured at recognition are retained; foreign documents/views reject.
    pub fn rebind<S>(&mut self, doc: &Document, view: &FoldView<S>) -> Result<(), LayoutError> {
        if self.binding.document != doc.id() {
            return Err(LayoutError::WrongDocument);
        }
        if self.binding.view != view.heights.version().0 {
            return Err(LayoutError::WrongView);
        }
        self.binding = Binding::new(doc, view)?;
        Ok(())
    }
    pub fn for_update<S>(
        doc: &Document,
        view: &FoldView<S>,
        update: &Update,
    ) -> Result<Self, LayoutError> {
        let binding = Binding::new(doc, view)?;
        if update.document != Some(doc.id()) {
            return Err(LayoutError::WrongDocument);
        }
        if update.revision != doc.revision() {
            return Err(LayoutError::StaleContinuation);
        }
        if update.new_tables.start > update.new_tables.end
            || update.new_tables.end > doc.tables().len()
        {
            return Err(LayoutError::InvalidRange);
        }
        Ok(Self {
            binding,
            next: update.new_tables.start,
            end: update.new_tables.end,
            policy: view.state.table_rows.policy,
        })
    }
    pub fn remaining_tables(&self) -> Range<usize> {
        self.next..self.end
    }
    pub fn advance<S>(
        &mut self,
        doc: &Document,
        view: &mut FoldView<S>,
        budget: &mut Budget,
    ) -> Result<TableRowFoldProgress, LayoutError> {
        self.binding.check(doc, view)?;
        let mut change: Option<FoldChange> = None;
        if let Some(policy) = self.policy {
            while self.next < self.end {
                if !budget.charge(cost()) {
                    return Ok(TableRowFoldProgress {
                        completion: Completion::BudgetExhausted,
                        id: None,
                        change,
                    });
                }
                let table = TableId(self.next);
                self.next += 1;
                if view.state.table_rows.automatic.contains_key(&table) {
                    continue;
                }
                let (_, c) = view.publish_table_rows(
                    doc,
                    table,
                    TableRowExtent::Live {
                        start: policy.visible_body_rows + 1,
                    },
                    true,
                )?;
                self.binding.version = view.version;
                change = Some(match change {
                    None => c,
                    Some(old) => FoldChange {
                        version: c.version,
                        source: Span::new(
                            old.source.start.min(c.source.start),
                            old.source.end.max(c.source.end),
                        ),
                    },
                });
            }
        }
        Ok(TableRowFoldProgress {
            completion: Completion::Covered,
            id: None,
            change,
        })
    }
}

impl<S> FoldView<S> {
    fn checked_row_entry(&self, doc: &Document, id: TableRowFoldId) -> Result<Entry, LayoutError> {
        self.check(doc)?;
        let e = *self
            .state
            .table_rows
            .entries
            .get(&id)
            .ok_or(LayoutError::InvalidRange)?;
        if e.document != doc.id() {
            return Err(LayoutError::WrongDocument);
        }
        if e.owner != self.heights.version().0 {
            return Err(LayoutError::WrongView);
        }
        Ok(e)
    }
    pub fn table_row_fold_info(
        &self,
        doc: &Document,
        id: TableRowFoldId,
    ) -> Result<Option<TableRowFoldInfo>, LayoutError> {
        self.check(doc)?;
        if !self.state.table_rows.has(id) {
            return Ok(None);
        }
        self.checked_row_entry(doc, id)?;
        Ok(self.state.table_rows.info(doc, id))
    }
    pub fn table_row_policy(&self) -> Option<TableRowPolicy> {
        self.state.table_rows.policy
    }
    pub fn set_table_row_policy(
        &mut self,
        doc: &Document,
        policy: Option<TableRowPolicy>,
        budget: &mut Budget,
    ) -> Result<bool, LayoutError> {
        self.check(doc)?;
        if policy.is_some_and(|p| p.visible_body_rows >= usize::MAX - 1) {
            return Err(LayoutError::InvalidRange);
        }
        if self.state.table_rows.policy == policy {
            return Ok(true);
        }
        if !budget.charge(cost()) {
            return Ok(false);
        }
        self.state.table_rows.policy = policy;
        self.version += 1;
        Ok(true)
    }
    /// Explicit diagnostics. Index-node/visit totals inspect current per-table
    /// indexes only; this is not used by append, layout or frame scheduling.
    pub fn table_row_work(&self) -> TableRowWork {
        let r = &self.state.table_rows;
        TableRowWork {
            registrations: r.entries.len(),
            selected_groups: r.groups.len(),
            watched_groups: r.watches.len(),
            row_index_nodes: r.rows.values().map(|i| i.node_count()).sum(),
            row_index_visits: r.rows.values().map(|i| i.visits()).sum(),
            group_refreshes: r.refreshes,
            row_endpoint_reads: r.endpoint_reads,
        }
    }
    fn row_index_insert(&mut self, id: TableRowFoldId, e: Entry) {
        self.state.table_rows.tables.insert((e.table, id));
        if let Some(k) = e.row_key(id) {
            self.state
                .table_rows
                .rows
                .entry(e.table)
                .or_default()
                .insert(k);
        }
    }
    fn row_index_remove(&mut self, id: TableRowFoldId, e: Entry) {
        self.state.table_rows.tables.remove(&(e.table, id));
        if let Some(k) = e.row_key(id) {
            let index = self.state.table_rows.rows.get_mut(&e.table).unwrap();
            index.remove(k);
            if index.node_count() == 0 {
                self.state.table_rows.rows.remove(&e.table);
            }
        }
    }
    fn row_detach(&mut self, id: TableRowFoldId, e: Entry) {
        if let Some(gk) = e.group()
            && let Some(g) = self.state.table_rows.groups.get_mut(&gk)
        {
            g.members.remove(&(e.start(), id));
        }
    }
    fn row_attach(&mut self, id: TableRowFoldId, e: Entry) {
        if e.collapsed
            && self.preview() != Some(FoldId::TableRange(id))
            && let Some(gk) = e.group()
        {
            self.state
                .table_rows
                .groups
                .entry(gk)
                .or_default()
                .members
                .insert((e.start(), id));
        }
    }
    fn refresh_row_group(&mut self, doc: &Document, gk: GroupKey) -> Option<Span> {
        let old = self
            .state
            .table_rows
            .groups
            .get_mut(&gk)?
            .representative
            .take();
        let mut dirty = old
            .and_then(|id| self.state.remove(doc, &mut self.heights, id))
            .map(|m| m.candidate.hidden);
        self.state.table_rows.refreshes += 1;
        let first = self.state.table_rows.groups[&gk].members.first().copied();
        let Some((_, id)) = first else {
            self.state.table_rows.groups.remove(&gk);
            self.state.table_rows.watches.remove(&gk);
            return dirty;
        };
        let candidate = self.state.table_rows.candidate(doc, id).unwrap();
        self.state.table_rows.endpoint_reads += if candidate.hidden.is_empty() { 1 } else { 2 };
        if !candidate.hidden.is_empty() {
            self.state.insert(doc, &mut self.heights, candidate);
            self.state
                .table_rows
                .groups
                .get_mut(&gk)
                .unwrap()
                .representative = Some(candidate.id);
        }
        dirty = Some(dirty.map_or(candidate.hidden, |old| {
            Span::new(
                old.start.min(candidate.hidden.start),
                old.end.max(candidate.hidden.end),
            )
        }));
        let watch =
            candidate.open || candidate.hidden.end == doc.source().len() && !doc.is_finished();
        if watch {
            self.state.table_rows.watches.insert(gk);
        } else {
            self.state.table_rows.watches.remove(&gk);
        }
        dirty
    }
    fn row_groups_changed(
        &mut self,
        doc: &Document,
        old: Option<GroupKey>,
        new: Option<GroupKey>,
        source: Span,
    ) -> Span {
        let mut source = source;
        for (i, gk) in [old, new].into_iter().enumerate() {
            if i == 1 && new == old {
                continue;
            }
            if let Some(gk) = gk
                && let Some(s) = self.refresh_row_group(doc, gk)
            {
                source = Span::new(source.start.min(s.start), source.end.max(s.end));
            }
        }
        source
    }
    fn finish_row_change(
        &mut self,
        doc: &Document,
        source: Span,
    ) -> Result<FoldChange, LayoutError> {
        self.version += 1;
        self.expire_from(doc, source.start)?;
        Ok(FoldChange {
            version: self.version,
            source,
        })
    }
    fn publish_table_rows(
        &mut self,
        doc: &Document,
        table: TableId,
        extent: TableRowExtent,
        prefix: bool,
    ) -> Result<(TableRowFoldId, FoldChange), LayoutError> {
        if prefix && let Some(&id) = self.state.table_rows.automatic.get(&table) {
            let c = self.edit_table_rows(doc, id, Edit::Retarget(extent))?;
            let c = self
                .set_table_rows_collapsed(doc, id, true)?
                .map_or(c, |next| FoldChange {
                    version: next.version,
                    source: Span::new(
                        c.source.start.min(next.source.start),
                        c.source.end.max(next.source.end),
                    ),
                });
            return Ok((id, c));
        }
        let id = TableRowFoldId::new();
        let e = Entry {
            document: doc.id(),
            owner: self.heights.version().0,
            table,
            extent,
            collapsed: prefix,
        };
        self.state.table_rows.entries.insert(id, e);
        self.row_index_insert(id, e);
        if prefix {
            self.state.table_rows.automatic.insert(table, id);
        }
        self.row_attach(id, e);
        let source = self.state.table_rows.candidate(doc, id).unwrap().hidden;
        let source =
            self.row_groups_changed(doc, None, if prefix { e.group() } else { None }, source);
        Ok((id, self.finish_row_change(doc, source)?))
    }
    pub(super) fn set_table_rows_collapsed(
        &mut self,
        doc: &Document,
        id: TableRowFoldId,
        collapsed: bool,
    ) -> Result<Option<FoldChange>, LayoutError> {
        let old = self.checked_row_entry(doc, id)?;
        if old.collapsed == collapsed {
            return Ok(None);
        }
        let source = self.state.table_rows.candidate(doc, id).unwrap().hidden;
        self.row_detach(id, old);
        let e = Entry { collapsed, ..old };
        self.state.table_rows.entries.insert(id, e);
        if let Some((target, choice)) = &mut self.preview
            && *target == FoldId::TableRange(id)
        {
            *choice = collapsed;
        }
        self.row_attach(id, e);
        let source = self.row_groups_changed(doc, old.group(), e.group(), source);
        Ok(Some(self.finish_row_change(doc, source)?))
    }
    fn edit_table_rows(
        &mut self,
        doc: &Document,
        id: TableRowFoldId,
        edit: Edit,
    ) -> Result<FoldChange, LayoutError> {
        let old = self.checked_row_entry(doc, id)?;
        let old_source = self.state.table_rows.candidate(doc, id).unwrap().hidden;
        self.row_detach(id, old);
        self.row_index_remove(id, old);
        let new = match edit {
            Edit::Remove => None,
            Edit::Retarget(extent) => Some(Entry { extent, ..old }),
            Edit::Seal => {
                let TableRowExtent::Live { start } = old.extent else {
                    unreachable!()
                };
                let table = doc.table(old.table).unwrap();
                let end = table.rows().len();
                let extent = if start < end {
                    TableRowExtent::Fixed { start, end }
                } else {
                    TableRowExtent::SealedEmpty {
                        start,
                        source: frontier(table),
                    }
                };
                Some(Entry { extent, ..old })
            }
        };
        // A sealed/retargeted fixed registration is no longer the active
        // first-N policy slot. A later policy action must not move its anchor.
        if new.is_none_or(|e| !matches!(e.extent, TableRowExtent::Live { .. }))
            && self.state.table_rows.automatic.get(&old.table) == Some(&id)
        {
            self.state.table_rows.automatic.remove(&old.table);
        }
        let new_source = if let Some(e) = new {
            self.state.table_rows.entries.insert(id, e);
            self.row_index_insert(id, e);
            self.row_attach(id, e);
            self.state.table_rows.candidate(doc, id).unwrap().hidden
        } else {
            self.state.table_rows.entries.remove(&id);
            if self.state.table_rows.automatic.get(&old.table) == Some(&id) {
                self.state.table_rows.automatic.remove(&old.table);
            }
            if self.preview() == Some(FoldId::TableRange(id)) {
                self.preview = None;
            }
            old_source
        };
        let source = Span::new(
            old_source.start.min(new_source.start),
            old_source.end.max(new_source.end),
        );
        let source = self.row_groups_changed(doc, old.group(), new.and_then(Entry::group), source);
        self.finish_row_change(doc, source)
    }
    pub(super) fn refresh_table_row_groups(&mut self, doc: &Document, dirty: &mut Option<usize>) {
        // Previously current live table and the sole Growing/old-EOF fixed end.
        debug_assert!(self.state.table_rows.watches.len() <= 2);
        let mut keys = [None; 2];
        for (slot, &key) in keys.iter_mut().zip(&self.state.table_rows.watches) {
            *slot = Some(key);
        }
        for key in keys.into_iter().flatten() {
            if let Some(s) = self.refresh_row_group(doc, key) {
                *dirty = Some(dirty.unwrap_or(usize::MAX).min(s.start));
            }
        }
    }
    pub(super) fn preview_table_rows(
        &mut self,
        doc: &Document,
        target: Option<FoldId>,
    ) -> Result<FoldPreviewProgress, LayoutError> {
        let mut dirty = None;
        if let Some((id, collapsed)) = self.preview.take() {
            let c = self.candidate(doc, id).unwrap();
            dirty = Some(c.hidden.start);
            if let FoldId::TableRange(id) = id {
                let e = self.checked_row_entry(doc, id)?;
                self.row_attach(id, e);
                self.row_groups_changed(doc, e.group(), e.group(), c.hidden);
            } else if collapsed {
                self.state.insert(doc, &mut self.heights, c);
            }
        }
        if let Some(id) = target {
            let c = self.candidate(doc, id).unwrap();
            dirty = Some(dirty.unwrap_or(usize::MAX).min(c.hidden.start));
            let collapsed = if let FoldId::TableRange(id) = id {
                let e = self.checked_row_entry(doc, id)?;
                self.row_detach(id, e);
                self.row_groups_changed(doc, e.group(), e.group(), c.hidden);
                e.collapsed
            } else {
                self.state.remove(doc, &mut self.heights, id).is_some()
            };
            self.preview = Some((id, collapsed));
        }
        let start = dirty.expect("changed preview");
        let c = self.finish_row_change(doc, Span::new(start, doc.source().len()))?;
        Ok(FoldPreviewProgress {
            completion: Completion::Covered,
            change: Some(c),
        })
    }
    pub(crate) fn next_table_row_fold(
        &self,
        doc: &Document,
        after: Option<(TableId, TableRowFoldId)>,
        window: Span,
    ) -> Option<(TableId, TableRowFoldId, FoldCandidate)> {
        // TableId order is the parser's source order. Restrict a batch to its
        // source scope before visiting registrations; no historical table scan.
        let tables = doc.tables();
        let lo = tables.partition_point(|t| {
            t.source.end < window.start || !window.is_empty() && t.source.end == window.start
        });
        let hi = tables.partition_point(|t| t.source.start < window.end);
        if lo >= hi {
            return None;
        }
        let bound = after.map_or(
            Bound::Included((TableId(lo), TableRowFoldId(0))),
            Bound::Excluded,
        );
        self.state
            .table_rows
            .tables
            .range((bound, Bound::Excluded((TableId(hi), TableRowFoldId(0)))))
            .next()
            .map(|&(table, id)| (table, id, self.state.table_rows.candidate(doc, id).unwrap()))
    }
}
