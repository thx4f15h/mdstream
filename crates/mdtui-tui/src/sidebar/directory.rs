//! A sparse outline, independent of body FoldView and body search. All index
//! access is in resumable paid steps; only a screen of metadata/titles is kept.
use std::collections::{BTreeSet, VecDeque};

use mdtui3::{
    Budget, Document, DocumentId, Heading, HeadingId, HeadingSearch, HeadingSearchStep,
    LayoutError, Update, WorkLimit,
};

use super::{SidebarEntry, SidebarModel, SidebarStats};
use crate::render::objects::{LinkLease, SharedLinks};

pub const QUERY_BYTES: usize = 256;
pub const BREADCRUMB_TITLES: usize = 3;
pub const HEADER_ROWS: usize = 2;

fn charge(budget: &mut Budget) -> bool {
    budget.charge(WorkLimit {
        queries: 1,
        objects: 1,
        ..WorkLimit::default()
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectoryEvent {
    Move(isize),
    Home,
    End,
    Left,
    Right,
    Current,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NameStatus {
    #[default]
    Tree,
    Searching,
    Page,
    Complete,
    Stale,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DirectoryWork {
    pub metadata: usize,
    pub ancestor_steps: usize,
    pub subtree_skips: usize,
    pub search_steps: usize,
    pub peak_rows: usize,
}

/// Actual Vec/String capacities; B-tree wishes report key count separately
/// because std does not expose its node capacity. Excludes shared resolver and
/// the fixed-size core search run buffer; inspect SharedLinks::stats separately.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DirectoryMemory {
    pub rows: usize,
    pub row_capacity_bytes: usize,
    pub title_capacity_bytes: usize,
    pub query_capacity_bytes: usize,
    pub collapsed_keys: usize,
}

#[derive(Clone, Copy)]
struct Row {
    id: HeadingId,
    heading: Heading,
    depth: usize,
    child: bool,
    after: usize,
}

// A cursor checks one candidate and then its outline ancestors. It can jump
// across an entire hidden subtree in either direction, with no descendant walk.
struct Walk {
    before: usize,
    backwards: bool,
    candidate: Option<Row>,
    ancestor: Option<HeadingId>,
    hidden: Option<(HeadingId, usize)>,
}
impl Walk {
    fn new(before: usize, backwards: bool) -> Self {
        Self {
            before,
            backwards,
            candidate: None,
            ancestor: None,
            hidden: None,
        }
    }
    fn next(
        &mut self,
        doc: &Document,
        level: u8,
        collapsed: &BTreeSet<HeadingId>,
        budget: &mut Budget,
        work: &mut DirectoryWork,
    ) -> Option<Option<Row>> {
        loop {
            if self.candidate.is_none() {
                if !charge(budget) {
                    return None;
                }
                work.metadata += 1;
                let index = doc.headings();
                let id = (1..=level).filter_map(|l| {
                    let ids = index.at_level(l);
                    let i = ids.partition_point(|id| id.0 < self.before);
                    if self.backwards {
                        i.checked_sub(1).and_then(|i| ids.get(i)).copied()
                    } else {
                        ids.get(i).copied()
                    }
                });
                let found = if self.backwards { id.max() } else { id.min() };
                let Some(id) = found else {
                    return Some(None);
                };
                let heading = *index.get(id).expect("indexed heading");
                let after = after_section(doc, id);
                self.candidate = Some(Row {
                    id,
                    heading,
                    depth: 0,
                    child: after > id.0 + 1,
                    after,
                });
                self.ancestor = index.outline_parent(id);
                self.hidden = None;
            }
            while let Some(id) = self.ancestor {
                if !charge(budget) {
                    return None;
                }
                work.metadata += 1;
                work.ancestor_steps += 1;
                if collapsed.contains(&id) {
                    self.hidden = Some((id, after_section(doc, id)));
                }
                self.candidate.as_mut().unwrap().depth += 1;
                self.ancestor = doc.headings().outline_parent(id);
            }
            let row = self.candidate.take().unwrap();
            if let Some((hidden, after)) = self.hidden.take() {
                work.subtree_skips += 1;
                self.before = if self.backwards { hidden.0 + 1 } else { after };
                continue;
            }
            self.before = if self.backwards {
                row.id.0
            } else if collapsed.contains(&row.id) {
                work.subtree_skips += 1;
                row.after
            } else {
                row.id.0 + 1
            };
            return Some(Some(row));
        }
    }
}
fn after_section(doc: &Document, id: HeadingId) -> usize {
    let end = doc.headings().section(id, doc.source().len()).unwrap().end;
    doc.headings()
        .entries()
        .partition_point(|h| h.source.start < end)
        .max(id.0 + 1)
}

// A window is built around a selected ID. Reverse fill first, forward fill,
// then backfill at EOF. At most rows+1 metadata records survive any step.
struct TreeTask {
    walk: Walk,
    stage: u8,
    remaining: usize,
    selected: Option<Row>,
    rows: VecDeque<Row>,
}
impl TreeTask {
    fn new(seed: usize, backwards: bool, moves: usize) -> Self {
        Self {
            walk: Walk::new(seed, backwards),
            stage: 0,
            remaining: moves,
            selected: None,
            rows: VecDeque::new(),
        }
    }
    fn advance(
        &mut self,
        doc: &Document,
        level: u8,
        collapsed: &BTreeSet<HeadingId>,
        count: usize,
        budget: &mut Budget,
        work: &mut DirectoryWork,
    ) -> bool {
        loop {
            if self.stage == 1 && self.rows.len() >= count / 2 {
                let selected = self.selected.unwrap();
                self.rows.push_back(selected);
                self.walk = Walk::new(
                    if collapsed.contains(&selected.id) {
                        selected.after
                    } else {
                        selected.id.0 + 1
                    },
                    false,
                );
                self.stage = 2;
            }
            if self.stage >= 2 && self.rows.len() >= count {
                return true;
            }
            let Some(next) = self.walk.next(doc, level, collapsed, budget, work) else {
                return false;
            };
            match self.stage {
                0 => {
                    if let Some(row) = next {
                        self.selected = Some(row);
                        if self.remaining > 0 {
                            self.remaining -= 1;
                            continue;
                        }
                    }
                    let Some(selected) = self.selected else {
                        // A filtered seed before the first eligible title has
                        // no predecessor; fall forward once, without current.
                        if self.walk.backwards {
                            self.walk = Walk::new(0, false);
                            continue;
                        }
                        return true;
                    };
                    self.stage = 1;
                    self.walk = Walk::new(selected.id.0, true);
                }
                1 => {
                    if let Some(row) = next {
                        self.rows.push_front(row);
                    } else {
                        let selected = self.selected.unwrap();
                        self.rows.push_back(selected);
                        self.walk = Walk::new(
                            if collapsed.contains(&selected.id) {
                                selected.after
                            } else {
                                selected.id.0 + 1
                            },
                            false,
                        );
                        self.stage = 2;
                    }
                }
                2 => {
                    if let Some(row) = next {
                        self.rows.push_back(row);
                    } else {
                        self.walk = Walk::new(self.rows.front().map_or(0, |r| r.id.0), true);
                        self.stage = 3;
                    }
                }
                _ => {
                    if let Some(row) = next {
                        self.rows.push_front(row);
                    } else {
                        return true;
                    }
                }
            }
            work.peak_rows = work.peak_rows.max(self.rows.len());
        }
    }
}

pub struct Directory {
    links: SharedLinks,
    document: DocumentId,
    revision: u64,
    model: SidebarModel,
    breadcrumb: SidebarModel,
    rows: Vec<Row>,
    selected: Option<HeadingId>,
    current: Option<HeadingId>,
    current_source: Option<usize>,
    current_dirty: bool,
    crumb_next: Option<HeadingId>,
    crumb_rows: Vec<(HeadingId, Heading)>,
    crumb_more: bool,
    collapsed: BTreeSet<HeadingId>,
    max_level: u8,
    query: String,
    edit: Option<String>,
    pub status: NameStatus,
    search: Option<HeadingSearch>,
    lease: Option<LinkLease>,
    search_back_before: Option<usize>,
    search_to_end: bool,
    search_to_home: bool,
    search_rows: VecDeque<Row>,
    search_pending: Option<HeadingId>,
    search_end: bool,
    search_start: usize,
    start_has_prev: bool,
    search_has_prev: bool,
    search_seen: usize,
    scan_fresh: bool,
    event: Option<DirectoryEvent>,
    blocked_event: Option<DirectoryEvent>,
    running_event: Option<DirectoryEvent>,
    right_end: Option<usize>,
    total: usize,
    preparing_query: usize,
    task: Option<TreeTask>,
    dirty: bool,
    dirty_from: usize,
    count: usize,
    width: usize,
    work: DirectoryWork,
}
impl Directory {
    pub fn new(doc: &Document, links: SharedLinks) -> Self {
        let mut model = SidebarModel::empty(doc, 0);
        model.set_links(links.clone());
        let mut breadcrumb = SidebarModel::empty(doc, 0);
        breadcrumb.set_links(links.clone());
        Self {
            links,
            document: doc.id(),
            revision: doc.revision(),
            model,
            breadcrumb,
            rows: Vec::new(),
            selected: None,
            current: None,
            current_source: None,
            current_dirty: true,
            crumb_next: None,
            crumb_rows: Vec::new(),
            crumb_more: false,
            collapsed: BTreeSet::new(),
            max_level: 6,
            query: String::new(),
            edit: None,
            status: NameStatus::Tree,
            search: None,
            lease: None,
            search_back_before: None,
            search_to_end: false,
            search_to_home: false,
            search_rows: VecDeque::new(),
            search_pending: None,
            search_end: false,
            search_start: 0,
            start_has_prev: false,
            search_has_prev: false,
            search_seen: 0,
            scan_fresh: true,
            event: None,
            blocked_event: None,
            running_event: None,
            right_end: None,
            total: 0,
            preparing_query: 0,
            task: None,
            dirty: true,
            dirty_from: usize::MAX,
            count: 0,
            width: 0,
            work: DirectoryWork::default(),
        }
    }
    pub fn selection(&self) -> Option<HeadingId> {
        self.selected
    }
    pub fn current(&self) -> Option<HeadingId> {
        self.current.filter(|_| !self.current_dirty)
    }
    pub fn visible_current(&self) -> Option<HeadingId> {
        self.current()
            .filter(|id| self.rows.iter().any(|r| r.id == *id))
    }
    pub fn entries(&self) -> impl ExactSizeIterator<Item = &SidebarEntry> {
        self.model.entries()
    }
    pub fn entry(&self, i: usize) -> Option<&SidebarEntry> {
        self.model.entry(i)
    }
    pub fn depth(&self, i: usize) -> usize {
        self.rows.get(i).map_or(0, |r| r.depth)
    }
    pub fn marker(&self, i: usize) -> &'static str {
        self.rows.get(i).map_or(" ", |r| {
            if !self.query.is_empty() || !r.child {
                " "
            } else if self.collapsed.contains(&r.id) {
                "+"
            } else {
                "-"
            }
        })
    }
    pub fn window_start(&self) -> usize {
        self.rows.first().map_or(0, |r| r.id.0)
    }
    pub fn work(&self) -> DirectoryWork {
        self.work
    }
    pub fn memory(&self) -> DirectoryMemory {
        DirectoryMemory {
            rows: self.rows.len(),
            row_capacity_bytes: (self.rows.capacity()
                + self.search_rows.capacity()
                + self.task.as_ref().map_or(0, |t| t.rows.capacity()))
                * std::mem::size_of::<Row>()
                + self.crumb_rows.capacity() * std::mem::size_of::<(HeadingId, Heading)>(),
            title_capacity_bytes: self.model.capacity_bytes() + self.breadcrumb.capacity_bytes(),
            query_capacity_bytes: self.query.capacity()
                + self.edit.as_ref().map_or(0, String::capacity)
                + self
                    .search
                    .as_ref()
                    .map_or(0, HeadingSearch::scratch_capacity_bytes),
            collapsed_keys: self.collapsed.len(),
        }
    }
    pub fn stats(&self) -> SidebarStats {
        self.model.stats()
    }
    pub fn query(&self) -> &str {
        &self.query
    }
    pub fn editing(&self) -> Option<&str> {
        self.edit.as_deref()
    }
    pub fn level(&self) -> u8 {
        self.max_level
    }
    pub fn collapsed(&self, id: HeadingId) -> bool {
        self.collapsed.contains(&id)
    }
    pub fn selection_pending(&self) -> bool {
        self.dirty
            || self.task.is_some()
            || self.event.is_some()
            || self.status == NameStatus::Searching
    }
    pub fn pending(&self) -> bool {
        self.dirty
            || self.task.is_some()
            || self.event.is_some()
            || self.current_dirty
            || self.crumb_next.is_some()
            || !self.model.is_complete()
            || !self.breadcrumb.is_complete()
            || self.status == NameStatus::Searching
            || self.links.retirement_pending()
    }
    pub fn begin_edit(&mut self) -> bool {
        if self.edit.is_some() {
            return false;
        }
        self.edit = Some(self.query.clone());
        true
    }
    pub fn insert(&mut self, ch: char) -> bool {
        let Some(edit) = self.edit.as_mut() else {
            return false;
        };
        if ch.is_control() || edit.len() + ch.len_utf8() > QUERY_BYTES {
            return false;
        }
        edit.push(ch);
        true
    }
    pub fn delete(&mut self) -> bool {
        self.edit.as_mut().and_then(String::pop).is_some()
    }
    pub fn cancel_edit(&mut self) -> bool {
        self.edit.take().is_some()
    }
    pub fn submit(&mut self) -> bool {
        let Some(query) = self.edit.take() else {
            return false;
        };
        if self.query != query || self.status == NameStatus::Stale {
            self.query = query;
            self.restart();
        }
        true
    }
    pub fn clear_query(&mut self) -> bool {
        if self.query.is_empty() {
            return false;
        }
        self.query.clear();
        self.restart();
        true
    }
    pub fn set_level(&mut self, level: u8) -> bool {
        let level = if level == 0 { 6 } else { level.min(6) };
        if level == self.max_level {
            return false;
        }
        self.max_level = level;
        self.restart();
        true
    }
    fn stop_search(&mut self) {
        // Every search operation swaps the resolver back before yielding. Its
        // cursor/guard is empty of historical maps; pattern storage is bounded
        // by QUERY_BYTES (plus numeric KMP/mapping and annotation scratch).
        self.search = None;
        self.lease = None;
        self.search_pending = None;
        self.preparing_query = 0;
        self.links.request_retirement();
    }
    /// Hidden controls must not pin a partial guard/memo while the body keeps
    /// scrolling. Completed bounded pages and all user choices remain intact.
    pub fn suspend(&mut self) {
        self.model.cancel_pending();
        self.breadcrumb.cancel_pending();
        self.stop_search();
        if self.status == NameStatus::Searching {
            self.status = NameStatus::Stale;
            self.dirty = true;
        } else if !self.query.is_empty() {
            self.search_start = self.rows.last().map_or(0, |r| r.id.0 + 1);
            self.start_has_prev = true;
        }
    }
    fn restart(&mut self) {
        self.stop_search();
        self.task = None;
        self.event = None;
        self.running_event = None;
        self.right_end = None;
        self.blocked_event = None;
        self.search_rows.clear();
        self.search_back_before = None;
        self.search_to_end = false;
        self.search_to_home = false;
        self.search_end = false;
        self.search_start = 0;
        self.start_has_prev = false;
        self.search_has_prev = false;
        self.search_seen = 0;
        self.status = if self.query.is_empty() {
            NameStatus::Tree
        } else {
            NameStatus::Searching
        };
        self.dirty = true;
    }
    pub fn event(&mut self, event: DirectoryEvent) -> bool {
        if self.event == Some(event)
            || self.blocked_event == Some(event)
            || event == DirectoryEvent::Move(0)
            || self.task.is_some()
            || self.status == NameStatus::Searching
            || self.status == NameStatus::Stale
        {
            return false;
        }
        if !self.query.is_empty()
            && matches!(
                event,
                DirectoryEvent::Left | DirectoryEvent::Right | DirectoryEvent::Current
            )
        {
            return false;
        }
        if (self.total == 0 && self.current_source.is_some())
            || (event == DirectoryEvent::Current
                && (self.current().is_none() || self.current() == self.selected))
        {
            return false;
        }
        if self.query.is_empty()
            && let Some(row) = self.rows.iter().find(|r| Some(r.id) == self.selected)
            && ((event == DirectoryEvent::Right && !row.child)
                || (event == DirectoryEvent::Left && !row.child && row.depth == 0))
        {
            return false;
        }
        if !self.query.is_empty() {
            if (event == DirectoryEvent::Home && !self.search_has_prev)
                || (event == DirectoryEvent::End && self.search_end)
            {
                let target = if event == DirectoryEvent::Home {
                    self.rows.first()
                } else {
                    self.rows.last()
                }
                .map(|r| r.id);
                if self.selected == target {
                    return false;
                }
                self.selected = target;
                return true;
            }
            if matches!(event, DirectoryEvent::Move(d) if d < 0)
                && !self.search_has_prev
                && self.rows.first().map(|r| r.id) == self.selected
            {
                return false;
            }
        }
        if let Some(id) = self.selected
            && ((id.0 == 0
                && (event == DirectoryEvent::Home
                    || matches!(event, DirectoryEvent::Move(d) if d < 0)))
                || (id.0 + 1 == self.total
                    && (event == DirectoryEvent::End
                        || matches!(event, DirectoryEvent::Move(d) if d > 0))))
        {
            return false;
        }
        self.event = Some(event);
        true
    }
    fn interrupt_tree(&mut self) {
        if self.task.take().is_some() {
            self.event = if self.right_end.take().is_some() {
                Some(DirectoryEvent::Right)
            } else {
                self.running_event.take().or(self.event)
            };
        }
    }
    pub fn apply_update(&mut self, doc: &Document, update: &Update) {
        if self.revision == doc.revision() {
            return;
        }
        self.revision = doc.revision();
        self.blocked_event = None;
        self.current_dirty = true;
        self.interrupt_tree();
        self.model.cancel_pending();
        self.breadcrumb.cancel_pending();
        self.dirty_from = self.dirty_from.min(
            update
                .dirty
                .iter()
                .map(|r| r.start)
                .min()
                .unwrap_or(update.appended.start),
        );
        if !update.references_changed.is_empty() {
            self.dirty_from = 0;
        }
        if !self.query.is_empty() {
            self.stop_search();
            self.search_rows.clear();
            self.status = NameStatus::Stale;
        }
        self.dirty = true;
    }
    fn publish(&mut self, doc: &Document, rows: Vec<Row>) {
        let prepared: Vec<_> = rows.iter().map(|r| (r.id, r.heading)).collect();
        self.model
            .prepare_sparse(doc, &prepared, self.width, self.dirty_from);
        self.rows = rows;
        self.dirty_from = usize::MAX;
        self.dirty = false;
    }
    fn start_tree(&mut self, doc: &Document) {
        let seed = self.selected.or(self.current).map_or(0, |id| id.0);
        let event = self.event.take();
        self.running_event = event;
        let (seed, backwards, moves) = match event {
            Some(DirectoryEvent::Move(delta)) => {
                if delta < 0 {
                    (seed, true, delta.unsigned_abs().saturating_sub(1))
                } else {
                    let from = if self.collapsed.contains(&HeadingId(seed)) {
                        after_section(doc, HeadingId(seed))
                    } else {
                        seed + 1
                    };
                    (from, false, delta.unsigned_abs().saturating_sub(1))
                }
            }
            Some(DirectoryEvent::Current) => (self.current.map_or(0, |id| id.0 + 1), true, 0),
            Some(DirectoryEvent::Home) => (0, false, 0),
            Some(DirectoryEvent::End) => (doc.headings().entries().len(), true, 0),
            _ => (seed + 1, true, 0),
        };
        let mut task = TreeTask::new(seed, backwards, moves);
        // Preserve selection when moving at a boundary instead of selecting
        // current or wrapping. The paid walk will replace it if it finds a row.
        if matches!(event, Some(DirectoryEvent::Move(_))) {
            task.selected = self
                .rows
                .iter()
                .find(|r| Some(r.id) == self.selected)
                .copied();
        }
        self.task = Some(task);
    }
    fn tree_event(&mut self, doc: &Document, budget: &mut Budget) -> bool {
        let Some(event @ (DirectoryEvent::Left | DirectoryEvent::Right)) = self.event else {
            return true;
        };
        if !charge(budget) {
            return false;
        }
        self.work.metadata += 1;
        self.event = None;
        if let Some(id) = self.selected {
            let index = doc.headings();
            let child = index.entries().get(id.0 + 1).is_some_and(|h| {
                h.source.start < index.section(id, doc.source().len()).unwrap().end
            });
            if event == DirectoryEvent::Left {
                if child && !self.collapsed.contains(&id) {
                    self.collapsed.insert(id);
                } else if let Some(parent) = index.outline_parent(id) {
                    self.selected = Some(parent);
                } else {
                    self.blocked_event = Some(event);
                    return true;
                }
            } else if self.collapsed.remove(&id) {
            } else if child {
                self.right_end = Some(index.section(id, doc.source().len()).unwrap().end);
                self.event = Some(DirectoryEvent::Move(1));
            } else {
                self.blocked_event = Some(event);
                return true;
            }
            self.dirty = true;
        }
        true
    }
    pub fn advance(
        &mut self,
        doc: &Document,
        source: usize,
        rows: usize,
        width: usize,
        budget: &mut Budget,
    ) -> Result<(), LayoutError> {
        if self.document != doc.id() {
            return Err(LayoutError::WrongDocument);
        }
        if self.revision != doc.revision() {
            return Err(LayoutError::StaleContinuation);
        }
        self.current_dirty |= self.current_source != Some(source);
        let count = rows.saturating_sub(HEADER_ROWS).max(1);
        if self.width != width || self.count != count {
            self.width = width;
            self.count = count;
            self.dirty = true;
            self.interrupt_tree();
            if !self.query.is_empty() && self.status != NameStatus::Stale {
                // Restart at the retained page around selection, never at the
                // document start merely because geometry changed.
                let position = self
                    .rows
                    .iter()
                    .position(|r| Some(r.id) == self.selected)
                    .unwrap_or(0);
                let first = position.saturating_sub(count / 2);
                self.search_start = self.rows.get(first).map_or(0, |r| r.id.0);
                self.start_has_prev = self.search_has_prev || first > 0;
                self.stop_search();
                self.search_rows.clear();
                self.search_seen = 0;
                self.search_back_before = None;
                self.search_to_end = false;
                self.search_to_home = false;
                self.search_end = false;
                self.status = NameStatus::Searching;
            }
        }
        if !self.links.prepare(doc, budget)? {
            return Ok(());
        }
        if self.current_dirty {
            if !charge(budget) {
                return Ok(());
            }
            self.work.metadata += 1;
            self.total = doc.headings().entries().len();
            let current = doc.headings().current(source);
            if self.current != current {
                self.blocked_event = None;
            }
            self.current = current;
            self.current_source = Some(source);
            self.current_dirty = false;
            self.crumb_next = self.current;
            self.crumb_rows.clear();
            self.crumb_more = false;
        }
        // Breadcrumb output and work are bounded even for a very deep scope.
        while let Some(id) = self.crumb_next {
            if !charge(budget) {
                return Ok(());
            }
            self.work.metadata += 1;
            self.crumb_rows.push((id, *doc.headings().get(id).unwrap()));
            self.crumb_next = doc.headings().outline_parent(id);
            if self.crumb_rows.len() == BREADCRUMB_TITLES {
                self.crumb_more = self.crumb_next.is_some();
                self.crumb_next = None;
            }
        }
        let mut crumbs = self.crumb_rows.clone();
        crumbs.reverse();
        self.breadcrumb.prepare_sparse(
            doc,
            &crumbs,
            width.saturating_sub(3) / BREADCRUMB_TITLES,
            self.dirty_from,
        );
        self.breadcrumb.advance(doc, budget)?;
        if !self.query.is_empty() {
            self.advance_names(doc, budget)?;
        } else {
            if !self.tree_event(doc, budget) {
                return Ok(());
            }
            if self.task.is_none() && (self.dirty || self.event.is_some()) {
                if !charge(budget) {
                    return Ok(());
                }
                self.work.metadata += 1;
                self.start_tree(doc);
            }
            if let Some(task) = self.task.as_mut() {
                if !task.advance(
                    doc,
                    self.max_level,
                    &self.collapsed,
                    count,
                    budget,
                    &mut self.work,
                ) {
                    return Ok(());
                }
                let task = self.task.take().unwrap();
                let selected = task.selected.map(|r| r.id);
                if self
                    .right_end
                    .take()
                    .is_some_and(|end| task.selected.is_some_and(|r| r.heading.source.start >= end))
                {
                    self.blocked_event = Some(DirectoryEvent::Right);
                    self.task = None;
                    self.dirty = false;
                } else {
                    if selected == self.selected {
                        self.blocked_event = self.running_event;
                    } else {
                        self.blocked_event = None;
                    }
                    self.selected = selected;
                    self.publish(doc, task.rows.into_iter().collect());
                }
                self.running_event = None;
            }
        }
        self.model.advance(doc, budget)?;
        Ok(())
    }
    pub fn breadcrumb_text(&self) -> String {
        if self.current_dirty || self.crumb_next.is_some() {
            return "path …".to_owned();
        }
        let mut text = if self.crumb_more {
            String::from("…/")
        } else {
            String::new()
        };
        for (i, entry) in self.breadcrumb.entries().enumerate() {
            if i > 0 {
                text.push('/');
            }
            text.push_str(&entry.text);
        }
        text
    }
    pub fn notice(&self) -> String {
        if let Some(edit) = &self.edit {
            return format!("edit {}/{}B /{edit}", edit.len(), QUERY_BYTES);
        }
        let level = if self.max_level == 6 {
            "all".to_owned()
        } else {
            format!("<=h{}", self.max_level)
        };
        match self.status {
            NameStatus::Tree => format!(
                "{level} 0..6 / Left/Right{}",
                if self.pending() { " …" } else { "" }
            ),
            NameStatus::Searching => format!(
                "scan h{} @{} /{}",
                self.search.as_ref().map_or(0, |s| s.heading_position() + 1),
                self.search
                    .as_ref()
                    .and_then(HeadingSearch::source_position)
                    .unwrap_or(0),
                self.query,
            ),
            NameStatus::Stale => format!("stale / Enter: refresh [{}]", self.query),
            _ => format!(
                "names {}{} {level} /{}",
                self.rows.len(),
                if self.search_end { " end" } else { " more↓" },
                self.query
            ),
        }
    }
    fn advance_names(&mut self, doc: &Document, budget: &mut Budget) -> Result<(), LayoutError> {
        if self.status == NameStatus::Stale {
            if self.dirty {
                if !charge(budget) {
                    return Ok(());
                }
                self.publish(doc, Vec::new());
            }
            return Ok(());
        }
        if let Some(event) = self.event.take() {
            let position = self
                .rows
                .iter()
                .position(|r| Some(r.id) == self.selected)
                .unwrap_or(0);
            let next = match event {
                DirectoryEvent::Move(delta) if delta < 0 => {
                    position.checked_sub(delta.unsigned_abs())
                }
                DirectoryEvent::Move(delta) => position
                    .checked_add(delta.unsigned_abs())
                    .filter(|i| *i < self.rows.len()),
                _ => None,
            };
            if let Some(i) = next {
                self.selected = Some(self.rows[i].id);
                return Ok(());
            }
            let back = matches!(event, DirectoryEvent::Move(d) if d < 0);
            if back && !self.search_has_prev {
                return Ok(());
            }
            if !back
                && self.search_end
                && !matches!(event, DirectoryEvent::Home | DirectoryEvent::End)
            {
                return Ok(());
            }
            let previous = back.then(|| self.rows.first().map_or(0, |r| r.id.0));
            if back || matches!(event, DirectoryEvent::Home | DirectoryEvent::End) {
                self.stop_search();
                self.search_start = 0;
                self.start_has_prev = false;
            }
            self.search_back_before = previous;
            self.search_to_end = event == DirectoryEvent::End;
            self.search_to_home = event == DirectoryEvent::Home;
            self.search_rows.clear();
            self.search_seen = 0;
            self.search_end = false;
            self.scan_fresh = false;
            self.status = NameStatus::Searching;
            self.dirty = true;
        }
        if self.status != NameStatus::Searching {
            return Ok(());
        }
        if self.search.is_none() {
            while self.preparing_query < self.query.len() {
                if !budget.charge(WorkLimit {
                    bytes: 1,
                    ..Default::default()
                }) {
                    return Ok(());
                }
                self.preparing_query += 1;
            }
            // Query preprocessing is bounded by the explicit 256-byte UI limit;
            // charge those bytes incrementally before constructing its KMP arrays.
            if !charge(budget) {
                return Ok(());
            }
            self.search = Some(HeadingSearch::from_heading(
                doc,
                &self.query,
                self.max_level,
                HeadingId(self.search_start),
            )?);
            self.lease = Some(self.links.hold());
            self.scan_fresh = true;
        }
        if self.lease.is_none() {
            self.lease = Some(self.links.hold());
        }
        loop {
            if let Some(id) = self.search_pending {
                if !charge(budget) {
                    return Ok(());
                }
                self.work.metadata += 1;
                self.search_pending = None;
                if self.search_back_before.is_some_and(|before| id.0 >= before) {
                    self.search_pending = Some(id);
                    self.finish_names(doc);
                    return Ok(());
                }
                self.search_seen = self.search_seen.saturating_add(1);
                self.search_rows.push_back(Row {
                    id,
                    heading: *doc.headings().get(id).unwrap(),
                    depth: 0,
                    child: false,
                    after: id.0 + 1,
                });
                if self.search_rows.len() > self.count {
                    self.search_rows.pop_front();
                }
                self.work.peak_rows = self.work.peak_rows.max(self.search_rows.len());
                if self.search_rows.len() == self.count
                    && self.search_back_before.is_none()
                    && !self.search_to_end
                {
                    self.finish_names(doc);
                    return Ok(());
                }
            }
            let step = self.links.with_resolver(|resolver| {
                let search = self.search.as_mut().unwrap();
                search.swap_link_resolver(resolver)?;
                let result = search.next_match(doc, budget);
                search.swap_link_resolver(resolver)?;
                result
            })?;
            self.work.search_steps += 1;
            match step {
                HeadingSearchStep::Match(hit) => self.search_pending = Some(hit.heading),
                HeadingSearchStep::BudgetExhausted => return Ok(()),
                HeadingSearchStep::Complete => {
                    self.search_end = true;
                    self.finish_names(doc);
                    return Ok(());
                }
            }
        }
    }
    fn finish_names(&mut self, doc: &Document) {
        self.lease = None;
        self.links.request_retirement();
        let keep_page = self.search_rows.is_empty()
            && !self.rows.is_empty()
            && (!self.scan_fresh || self.search_back_before.is_some());
        if !keep_page {
            self.search_has_prev = if self.search_back_before.is_some() || self.search_to_end {
                self.search_seen > self.count
            } else {
                self.start_has_prev || !self.scan_fresh
            };
        }
        let rows: Vec<_> = if keep_page {
            self.rows.clone()
        } else {
            self.search_rows.drain(..).collect()
        };
        if self.search_to_home {
            self.selected = rows.first().map(|r| r.id);
        } else if self.search_back_before.is_some() || self.search_to_end {
            self.selected = rows.last().map(|r| r.id);
        } else if !rows.iter().any(|r| Some(r.id) == self.selected) {
            self.selected = rows.first().map(|r| r.id);
        }
        self.status = if self.search_end {
            NameStatus::Complete
        } else {
            NameStatus::Page
        };
        self.publish(doc, rows);
    }
}
