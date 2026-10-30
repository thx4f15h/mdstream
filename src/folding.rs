use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
};

use crate::{
    BlockId, BlockKind, Budget, CheckpointIndex, Completion, ContainerId, ContainerKind, Document,
    DocumentId, HeadingId, HeightIndex, LayoutError, Measurement, Span, SyntaxState, TableId,
    TileId, Update, WorkLimit,
};

/// Document-local stable identity. Arbitrary crossing ranges are unsupported.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FoldId {
    Heading(HeadingId),
    Code(BlockId),
    Table(TableId),
    Container(ContainerId),
    TableRows(TableRowRange),
}

/// Stable document-local body row interval, independent of source/visual width.
/// Row zero is the header and cannot be included. End is exclusive; all selected
/// rows must already be indexed. Register with FoldView::insert_table_rows.
/// The last row may still grow, but later rows never join this fixed interval.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TableRowRange {
    pub table: TableId,
    pub start: usize,
    pub end: usize,
}

/// Union of container classes. An empty filter selects nothing. `items`
/// includes task items; task flags select only items with that effective state.
/// Lists are selected independently of their children's completion states.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ContainerFoldFilter {
    pub quotes: bool,
    pub lists: bool,
    pub items: bool,
    pub completed_tasks: bool,
    pub incomplete_tasks: bool,
}

impl ContainerFoldFilter {
    pub(crate) fn matches(self, kind: ContainerKind, checked: Option<bool>) -> bool {
        match kind {
            ContainerKind::Quote => self.quotes,
            ContainerKind::List { .. } => self.lists,
            ContainerKind::Item { .. } => {
                self.items
                    || match checked {
                        Some(true) => self.completed_tasks,
                        Some(false) => self.incomplete_tasks,
                        None => false,
                    }
            }
        }
    }
}

/// Initial choices for candidates first discovered by a subsequent `apply`.
/// Existing choices are never overwritten, including explicit expansions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FoldDefaults {
    /// Index 0 is h1, index 5 is h6. True starts that heading collapsed.
    pub heading_levels: [bool; 6],
    pub code: bool,
    pub table: bool,
    /// Collapse newly identified containers at or below this structural depth
    /// (`Container::depth >= threshold`). Roots are depth 1; List and Item each
    /// count as a level. None disables this policy; Some(0) includes all roots.
    /// Applies to Quote/List/Item alike, regardless of ancestor choices or preview.
    pub container_depth: Option<usize>,
    /// Collapse new containers matching these classes OR container_depth.
    /// Task states start from the source marker. Neither later state overrides
    /// nor defaults changes reapply initialization to an existing container.
    pub containers: ContainerFoldFilter,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FoldCandidate {
    pub id: FoldId,
    pub parent: Option<FoldId>,
    /// Whole section, code block, table, row range or container, including the retained heading.
    pub source: Span,
    /// Body replaced by a placeholder. Empty bodies produce no placeholder.
    pub hidden: Span,
    /// Title, fence info, table header, first selected row or container range.
    pub summary: Option<Span>,
    pub open: bool,
}

impl FoldCandidate {
    // Preorder tie-break: containers on a physical opener line precede its
    // leaf block. This distinguishes identical List/Item/Quote/Table ranges.
    pub(crate) fn contains_candidate(&self, other: &Self) -> bool {
        let order = |candidate: &Self| match candidate.id {
            FoldId::Container(id) => (candidate.source.start, 0, id.0),
            _ => (candidate.source.start, 1, 0),
        };
        self.source.start <= other.source.start
            && other.source.end <= self.source.end
            && order(self) <= order(other)
    }

    /// Read only a bounded UTF-8 prefix; no line counting or body scanning.
    /// None means the budget cannot pay for this read (even with no summary).
    pub fn summary_text<'a>(
        &self,
        document: &'a Document,
        max_bytes: usize,
        budget: &mut Budget,
    ) -> Option<&'a str> {
        let span = self.summary.unwrap_or_default();
        let mut end = span.start.saturating_add(max_bytes).min(span.end);
        let source = document.source();
        if span.start > span.end || span.end > source.len() || !source.is_char_boundary(span.start)
        {
            return None;
        }
        while !source.is_char_boundary(end) {
            end -= 1;
        }
        if !budget.charge(WorkLimit {
            queries: 1,
            bytes: end - span.start,
            objects: 1,
            ..Default::default()
        }) {
            return None;
        }
        Some(&source[span.start..end])
    }
}

/// Source-addressable drawable object. Its entire measured height belongs to
/// `owner`, even when the hidden range crosses many tiles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FoldPlaceholder {
    pub candidate: FoldCandidate,
    pub owner: TileId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FoldChange {
    pub version: u64,
    pub source: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FoldPreviewProgress {
    /// Covered means the requested preview is active (or has ended).
    /// BudgetExhausted leaves the previous preview and all choices untouched.
    pub completion: Completion,
    pub change: Option<FoldChange>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FoldLookup {
    Visible,
    Hidden(FoldCandidate),
    BudgetExhausted,
}

/// Bounded discovery of candidates whose opener block intersects a source
/// range, plus containers starting in it. Earlier ancestors are queried with
/// `fold_candidate_at` / `FoldCandidate::parent`, not duplicated in every page.
pub struct FoldCandidates {
    document: DocumentId,
    revision: u64,
    next: usize,
    end: usize,
    container_next: usize,
    container_end: usize,
}

impl FoldCandidates {
    pub(crate) fn is_done(&self) -> bool {
        self.next == self.end && self.container_next == self.container_end
    }

    /// Caller pays for one container opener or block, even without a candidate.
    pub(crate) fn next_candidate(&mut self, document: &Document) -> Option<FoldCandidate> {
        if self.container_next < self.container_end {
            let id = ContainerId(self.container_next);
            let container = document.container(id).unwrap();
            if self.next == self.end
                || container.source.start <= document.blocks()[self.next].source.start
            {
                self.container_next += 1;
                return document.fold_candidate(FoldId::Container(id));
            }
        }
        let block = &document.blocks()[self.next];
        let id = match block.kind {
            BlockKind::Code { .. } => Some(FoldId::Code(BlockId(self.next))),
            BlockKind::Table { id } => Some(FoldId::Table(id)),
            BlockKind::Heading { .. } => document
                .headings()
                .current(block.source.start)
                .map(FoldId::Heading),
            _ => None,
        };
        self.next += 1;
        id.and_then(|id| document.fold_candidate(id))
    }

    pub fn advance(
        &mut self,
        document: &Document,
        budget: &mut Budget,
        out: &mut Vec<FoldCandidate>,
    ) -> Result<crate::Completion, LayoutError> {
        if self.document != document.id() {
            return Err(LayoutError::WrongDocument);
        }
        if self.revision != document.revision() {
            return Err(LayoutError::StaleContinuation);
        }
        out.clear();
        while !self.is_done() {
            // Non-candidate blocks also consume work; discovery cannot scan an
            // arbitrarily large paragraph/list arena in one frame.
            if !budget.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            }) {
                return Ok(crate::Completion::BudgetExhausted);
            }
            if let Some(candidate) = self.next_candidate(document) {
                out.push(candidate);
            }
        }
        Ok(crate::Completion::Covered)
    }
}

struct Mask {
    candidate: FoldCandidate,
    tiles: Range<usize>,
}

#[derive(Default)]
pub(crate) struct FoldState {
    collapsed: BTreeMap<FoldId, Mask>,
    starts: BTreeSet<(usize, FoldId)>,
    // Registered disjoint ranges, including expanded and previewed choices.
    // Stable physical starts support overlap validation and point predecessor
    // queries without walking a table's rows or the other registered ranges.
    row_ranges: BTreeMap<usize, TableRowRange>,
    // A completed row can end exactly at EOF inside a still-growable tile.
    // Refresh that one mask on the next update, even if its source span is fixed.
    row_boundary: Option<FoldId>,
    // Six root sections, the active code/table and one incomplete row range
    // need individual refreshes. Containers and scoped sections share the tail.
    growing: BTreeSet<FoldId>,
    containers: crate::fold_ranges::FoldRanges,
    sections: crate::fold_ranges::FoldRanges,
    // Open containers and scoped sections all reach EOF. Their complete-tile
    // union has one shared mask; no ancestor is refreshed on ordinary append.
    open_ranges: BTreeSet<(usize, FoldId)>,
    open_tiles: Range<usize>,
}

#[derive(Clone, Copy)]
pub(crate) enum FoldSegment {
    Text { end: usize },
    Hidden(FoldCandidate),
}

impl FoldSegment {
    pub(crate) fn end(self) -> usize {
        match self {
            Self::Text { end } => end,
            Self::Hidden(c) => c.hidden.end,
        }
    }
}

impl Document {
    pub fn fold_candidates(&self, range: Span) -> Result<FoldCandidates, LayoutError> {
        if range.start > range.end || range.end > self.source().len() {
            return Err(LayoutError::InvalidRange);
        }
        let next = self
            .blocks()
            .partition_point(|block| block.source.end <= range.start);
        let end = if range.is_empty() {
            next
        } else {
            self.blocks()
                .partition_point(|block| block.source.start < range.end)
        };
        Ok(FoldCandidates {
            document: self.id(),
            revision: self.revision(),
            next,
            end,
            container_next: self.containers.starting_before(range.start),
            container_end: self.containers.starting_before(range.end),
        })
    }

    /// O(log headings), with at most six levels; never reads section contents.
    /// Open containers provisionally include unclassified source, like tables.
    pub fn fold_candidate(&self, id: FoldId) -> Option<FoldCandidate> {
        match id {
            FoldId::TableRows(range) => {
                if range.start == 0 || range.start >= range.end {
                    return None;
                }
                let table = self.table(range.table)?;
                let first = table.row(range.start)?;
                let last = table.row(range.end - 1)?;
                let source = Span::new(first.source.start, last.source.end);
                Some(FoldCandidate {
                    id,
                    parent: Some(FoldId::Table(range.table)),
                    source,
                    hidden: source,
                    summary: Some(first.source),
                    open: !last.complete && !self.is_finished(),
                })
            }
            FoldId::Container(id) => {
                let container = self.container(id)?;
                let open = container.state != SyntaxState::Sealed && !self.is_finished();
                let source = Span::new(
                    container.source.start,
                    if open {
                        self.source().len()
                    } else {
                        container.source.end
                    },
                );
                Some(FoldCandidate {
                    id: FoldId::Container(id),
                    parent: self.fold_parent(container.parent, source.start),
                    source,
                    hidden: source,
                    // A source reference only; summary_text still bounds reads.
                    summary: Some(source),
                    open,
                })
            }
            FoldId::Heading(id) => {
                let headings = self.headings();
                let heading = headings.get(id)?;
                let end = headings.section_end(id);
                let open = end.is_none()
                    && !self.is_finished()
                    && heading
                        .container
                        .is_none_or(|id| self.container(id).unwrap().state != SyntaxState::Sealed);
                let mut source = headings.section(id, self.source().len())?;
                if open {
                    source.end = self.source().len();
                }
                Some(FoldCandidate {
                    id: FoldId::Heading(id),
                    parent: headings
                        .parent(id)
                        .map(FoldId::Heading)
                        .or(heading.container.map(FoldId::Container)),
                    source,
                    hidden: Span::new(heading.source.end, source.end),
                    summary: Some(heading.content),
                    open,
                })
            }
            FoldId::Code(id) => {
                let block = self.block(id)?;
                let BlockKind::Code { info } = block.kind else {
                    return None;
                };
                let open = block.state != SyntaxState::Sealed && !self.is_finished();
                // Pending physical-line prefixes belong provisionally to an open
                // code block; classification can later retract that tail.
                let source = Span::new(
                    block.source.start,
                    if open {
                        self.source().len()
                    } else {
                        block.source.end
                    },
                );
                Some(FoldCandidate {
                    id: FoldId::Code(id),
                    parent: self.fold_parent(block.container, source.start),
                    source,
                    hidden: source,
                    summary: info,
                    open,
                })
            }
            FoldId::Table(id) => {
                let table = self.table(id)?;
                let block = self.block(table.block)?;
                let open = block.state != SyntaxState::Sealed && !self.is_finished();
                // As with code blocks, a pending physical line is provisionally
                // inside the table until its classification seals the block.
                let source = Span::new(
                    table.source.start,
                    if open {
                        self.source().len()
                    } else {
                        table.source.end
                    },
                );
                Some(FoldCandidate {
                    id: FoldId::Table(id),
                    parent: self.fold_parent(block.container, source.start),
                    source,
                    hidden: source,
                    summary: Some(table.row(0)?.source),
                    open,
                })
            }
        }
    }

    fn fold_parent(&self, container: Option<ContainerId>, offset: usize) -> Option<FoldId> {
        self.headings()
            .local_current(container, offset)
            .map(FoldId::Heading)
            .or(container.map(FoldId::Container))
    }

    /// Innermost supported candidate, including an open container/code/table tail.
    pub fn fold_candidate_at(&self, offset: usize) -> Option<FoldCandidate> {
        if offset >= self.source().len() {
            return None;
        }
        if let Some(block) = self.block_at(offset)
            && let Some(candidate) = self.leaf_fold_candidate(block)
        {
            return Some(candidate);
        }
        // The pending tail may be outside the last block's committed range.
        if let Some(index) = self.blocks().len().checked_sub(1)
            && let Some(candidate) = self.leaf_fold_candidate(BlockId(index))
            && candidate.open
            && candidate.source.range().contains(&offset)
        {
            return Some(candidate);
        }
        let container = self
            .block_at(offset)
            .and_then(|id| self.block(id)?.container)
            .or_else(|| self.containers.inner());
        if let Some(id) = self.headings().local_current(container, offset)
            && let Some(candidate) = self.fold_candidate(FoldId::Heading(id))
            && candidate.source.range().contains(&offset)
        {
            return Some(candidate);
        }
        if let Some(id) = container
            && let Some(candidate) = self.fold_candidate(FoldId::Container(id))
            && candidate.source.range().contains(&offset)
        {
            return Some(candidate);
        }
        self.fold_candidate(FoldId::Heading(self.headings().root_current(offset)?))
    }

    fn leaf_fold_candidate(&self, block: BlockId) -> Option<FoldCandidate> {
        self.fold_candidate(match self.block(block)?.kind {
            BlockKind::Code { .. } => FoldId::Code(block),
            BlockKind::Table { id } => FoldId::Table(id),
            _ => return None,
        })
    }
}

impl FoldState {
    pub(crate) fn segment(&self, document: &Document, offset: usize) -> FoldSegment {
        let mut hidden = None;
        let mut heading = document.headings().root_current(offset);
        while let Some(id) = heading {
            if let Some(mask) = self.collapsed.get(&FoldId::Heading(id))
                && mask.candidate.hidden.range().contains(&offset)
            {
                hidden = Some(mask.candidate);
            }
            heading = document.headings().parent(id);
        }
        if hidden.is_none()
            && offset < document.source().len()
            && let Some(id) = self
                .containers
                .covering(document.containers.starting_before(offset + 1), offset)
        {
            hidden = document.fold_candidate(FoldId::Container(ContainerId(id)));
        }
        if offset < document.source().len()
            && let Some(id) = self.sections.covering(
                document
                    .headings()
                    .entries()
                    .partition_point(|h| h.source.start <= offset),
                offset,
            )
            && let Some(candidate) = document.fold_candidate(FoldId::Heading(HeadingId(id)))
            && candidate.hidden.range().contains(&offset)
            && hidden.is_none_or(|outer| candidate.contains_candidate(&outer))
        {
            hidden = Some(candidate);
        }
        if hidden.is_none()
            && let Some(candidate) = document.fold_candidate_at(offset)
            && let Some(mask) = self.collapsed.get(&candidate.id)
            && mask.candidate.hidden.range().contains(&offset)
        {
            hidden = Some(mask.candidate);
        }
        if let Some(candidate) = hidden {
            return FoldSegment::Hidden(candidate);
        }
        if let Some((_, &range)) = self.row_ranges.range(..=offset).next_back()
            && let Some(mask) = self.collapsed.get(&FoldId::TableRows(range))
            && mask.candidate.hidden.range().contains(&offset)
        {
            return FoldSegment::Hidden(mask.candidate);
        }
        let end = self
            .starts
            .range((offset, FoldId::Heading(HeadingId(0)))..)
            .next()
            .map_or(document.source().len(), |&(start, _)| start);
        FoldSegment::Text { end }
    }

    fn remove(
        &mut self,
        document: &Document,
        heights: &mut HeightIndex,
        id: FoldId,
    ) -> Option<Mask> {
        let mask = self.collapsed.remove(&id)?;
        heights.cover(mask.tiles.clone(), false);
        self.starts.remove(&(mask.candidate.hidden.start, id));
        self.growing.remove(&id);
        if self.row_boundary == Some(id) {
            self.row_boundary = None;
        }
        if self.shared_range(document, id) {
            self.range_index(id).set(Self::range_id(id), 0);
            self.open_ranges.remove(&(mask.candidate.hidden.start, id));
            self.refresh_open_tiles(document, heights);
        }
        Some(mask)
    }

    fn insert(&mut self, document: &Document, heights: &mut HeightIndex, candidate: FoldCandidate) {
        let hidden = candidate.hidden;
        let shared = self.shared_range(document, candidate.id);
        let open_shared = shared && candidate.open;
        let tiles = if open_shared {
            0..0
        } else {
            Self::interior(document, hidden)
        };
        if shared {
            self.range_index(candidate.id).set(
                Self::range_id(candidate.id),
                if candidate.open {
                    usize::MAX
                } else {
                    hidden.end
                },
            );
            if candidate.open {
                self.open_ranges.insert((hidden.start, candidate.id));
                self.refresh_open_tiles(document, heights);
            }
        }
        if !hidden.is_empty() {
            self.starts.insert((hidden.start, candidate.id));
        }
        heights.cover(tiles.clone(), true);
        if candidate.open && !open_shared {
            self.growing.insert(candidate.id);
        }
        self.collapsed
            .insert(candidate.id, Mask { candidate, tiles });
        self.track_row_boundary(document, candidate);
    }

    fn track_row_boundary(&mut self, document: &Document, candidate: FoldCandidate) {
        if matches!(candidate.id, FoldId::TableRows(_)) {
            if !candidate.open
                && candidate.hidden.end == document.source().len()
                && !document.is_finished()
            {
                self.row_boundary = Some(candidate.id);
            } else if self.row_boundary == Some(candidate.id) {
                self.row_boundary = None;
            }
        }
    }

    fn shared_range(&self, document: &Document, id: FoldId) -> bool {
        match id {
            FoldId::Container(_) => true,
            FoldId::Heading(id) => document.headings().get(id).unwrap().container.is_some(),
            _ => false,
        }
    }

    fn range_id(id: FoldId) -> usize {
        match id {
            FoldId::Container(id) => id.0,
            FoldId::Heading(id) => id.0,
            _ => unreachable!(),
        }
    }

    fn range_index(&mut self, id: FoldId) -> &mut crate::fold_ranges::FoldRanges {
        match id {
            FoldId::Container(_) => &mut self.containers,
            FoldId::Heading(_) => &mut self.sections,
            _ => unreachable!(),
        }
    }

    fn refresh_open_tiles(&mut self, document: &Document, heights: &mut HeightIndex) {
        let tiles = self.open_ranges.first().map_or(0..0, |&(start, _)| {
            Self::interior(document, Span::new(start, document.source().len()))
        });
        if self.open_tiles != tiles {
            heights.cover(self.open_tiles.clone(), false);
            heights.cover(tiles.clone(), true);
            self.open_tiles = tiles;
        }
    }

    fn interior(document: &Document, hidden: Span) -> Range<usize> {
        if hidden.is_empty() {
            0..0
        } else {
            // The placeholder's tile is always retained. Only complete hidden
            // interiors are masked; the ending tile may contain visible text.
            let start = document.tile_at(hidden.start).unwrap().0 + 1;
            let end = document
                .tile_at(hidden.end)
                .map_or(document.tile_count(), |t| t.0);
            start..end.max(start)
        }
    }

    fn refresh(
        &mut self,
        document: &Document,
        heights: &mut HeightIndex,
        candidate: FoldCandidate,
    ) {
        let shared = self.shared_range(document, candidate.id);
        let mask = self.collapsed.get_mut(&candidate.id).unwrap();
        let old = mask.candidate;
        let tiles = if shared && candidate.open {
            0..0
        } else {
            Self::interior(document, candidate.hidden)
        };
        if mask.tiles != tiles {
            heights.cover(mask.tiles.clone(), false);
            heights.cover(tiles.clone(), true);
            mask.tiles = tiles;
        }
        let old_start = (!mask.candidate.hidden.is_empty())
            .then_some((mask.candidate.hidden.start, candidate.id));
        let new_start =
            (!candidate.hidden.is_empty()).then_some((candidate.hidden.start, candidate.id));
        if old_start != new_start {
            if let Some(key) = old_start {
                self.starts.remove(&key);
            }
            if let Some(key) = new_start {
                self.starts.insert(key);
            }
        }
        if !candidate.open {
            self.growing.remove(&candidate.id);
        }
        mask.candidate = candidate;
        self.track_row_boundary(document, candidate);
        if shared {
            if old.open != candidate.open || !candidate.open {
                self.range_index(candidate.id).set(
                    Self::range_id(candidate.id),
                    if candidate.open {
                        usize::MAX
                    } else {
                        candidate.hidden.end
                    },
                );
            }
            self.open_ranges.remove(&(old.hidden.start, candidate.id));
            if candidate.open {
                self.open_ranges
                    .insert((candidate.hidden.start, candidate.id));
            }
        }
    }
}

/// Each view owns its folding choices, height estimates and checkpoints. Parent
/// toggles preserve child choices. No fold operation mutates Document or HIDDEN.
/// S is the renderer's checkpoint state; use `new` for the stateless case.
pub struct FoldView<S = ()> {
    document: DocumentId,
    revision: u64,
    version: u64,
    pub(crate) heights: HeightIndex,
    checkpoints: CheckpointIndex<S>,
    pub(crate) state: FoldState,
    range_visits: u64,
    defaults: FoldDefaults,
    default_visits: u64,
    // Only values differing from the source marker; bounded by task item count.
    task_overrides: BTreeMap<ContainerId, bool>,
    // The active node is temporarily absent from the effective masks. Keep only
    // its local choice; ancestor/descendant choices remain in the usual index.
    preview: Option<(FoldId, bool)>,
}

impl FoldView<()> {
    pub fn new(document: &Document, estimated_rows: u64) -> Result<Self, LayoutError> {
        Self::with_state(document, estimated_rows)
    }
}

impl<S> FoldView<S> {
    pub fn with_state(document: &Document, estimated_rows: u64) -> Result<Self, LayoutError> {
        let mut heights = HeightIndex::new(estimated_rows)?;
        heights.rebuild(document)?;
        let mut checkpoints = CheckpointIndex::new();
        checkpoints.rebuild(document);
        Ok(Self {
            document: document.id(),
            revision: document.revision(),
            version: 0,
            heights,
            checkpoints,
            state: FoldState::default(),
            range_visits: 0,
            defaults: FoldDefaults::default(),
            default_visits: 0,
            task_overrides: BTreeMap::new(),
            preview: None,
        })
    }

    pub fn heights(&self) -> &HeightIndex {
        &self.heights
    }
    pub fn checkpoints(&self) -> &CheckpointIndex<S> {
        &self.checkpoints
    }
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Register a fixed, nonempty body row interval, initially expanded. Exact
    /// reinsertion preserves its choice. Overlap (including nesting) with any
    /// registered range in this view is rejected, even while expanded/previewed.
    /// Adjacent ranges remain distinct placeholders. No source is read: O(log R).
    /// Registration invalidates view tasks; no document syntax is changed.
    pub fn insert_table_rows(
        &mut self,
        document: &Document,
        table: TableId,
        rows: Range<usize>,
    ) -> Result<FoldId, LayoutError> {
        self.check(document)?;
        let range = TableRowRange {
            table,
            start: rows.start,
            end: rows.end,
        };
        let id = FoldId::TableRows(range);
        let candidate = document
            .fold_candidate(id)
            .ok_or(LayoutError::InvalidRange)?;
        let start = candidate.source.start;
        if let Some((_, &previous)) = self.state.row_ranges.range(..=start).next_back() {
            if previous == range {
                return Ok(id);
            }
            if previous.table == table && previous.end > rows.start {
                return Err(LayoutError::InvalidRange);
            }
        }
        if let Some((_, next)) = self.state.row_ranges.range(start..).next()
            && next.table == table
            && next.start < rows.end
        {
            return Err(LayoutError::InvalidRange);
        }
        self.state.row_ranges.insert(start, range);
        self.version += 1;
        Ok(id)
    }

    /// Forget a registered row interval and its local choice, including preview.
    /// This permits replacing its bounds. Other folds retain their choices.
    /// O(log R + log F + log T), with no scan of covered rows or tiles.
    pub fn remove_table_rows(
        &mut self,
        document: &Document,
        range: TableRowRange,
    ) -> Result<Option<FoldChange>, LayoutError> {
        self.check(document)?;
        let id = FoldId::TableRows(range);
        let candidate = document
            .fold_candidate(id)
            .ok_or(LayoutError::InvalidRange)?;
        if !self.has_table_rows(document, range) {
            return Ok(None);
        }
        self.state.remove(document, &mut self.heights, id);
        if self.preview() == Some(id) {
            self.preview = None;
        }
        self.state.row_ranges.remove(&candidate.source.start);
        self.version += 1;
        self.expire_from(document, candidate.hidden.start)?;
        Ok(Some(FoldChange {
            version: self.version,
            source: candidate.hidden,
        }))
    }

    /// Storage is bounded by the number of indexed body rows, not toggle count.
    pub fn table_row_range_count(&self) -> usize {
        self.state.row_ranges.len()
    }

    pub(crate) fn has_table_rows(&self, document: &Document, range: TableRowRange) -> bool {
        document
            .fold_candidate(FoldId::TableRows(range))
            .is_some_and(|c| self.state.row_ranges.get(&c.source.start) == Some(&range))
    }

    pub(crate) fn next_table_rows(
        &self,
        start: usize,
        end: usize,
    ) -> Option<(usize, TableRowRange)> {
        self.state
            .row_ranges
            .range(start..end)
            .next()
            .map(|(&at, &range)| (at, range))
    }
    /// Root section/code/table/row refreshes, a completed EOF row boundary,
    /// shared tail checks and event-
    /// driven scoped-section/container refreshes; independent of body size and
    /// still-open depth. Title growth refreshes at most one scoped heading.
    pub fn range_visits(&self) -> u64 {
        self.range_visits
    }
    /// Container index nodes visited by mutations and visibility queries.
    pub fn container_index_visits(&self) -> u64 {
        self.state.containers.visits()
    }
    /// Retained sparse index nodes, bounded by O(container_count), not toggles.
    pub fn container_index_nodes(&self) -> usize {
        self.state.containers.node_count()
    }
    /// Scoped-section index visits; ordinary body growth performs no writes.
    pub fn section_index_visits(&self) -> u64 {
        self.state.sections.visits()
    }
    /// Retained sparse nodes, bounded by O(heading_count), not toggles.
    pub fn section_index_nodes(&self) -> usize {
        self.state.sections.node_count()
    }
    /// Local choice, including the choice temporarily overridden by preview.
    /// Use hidden_at to query actual visibility under ancestors and preview.
    pub fn is_collapsed(&self, id: FoldId) -> bool {
        match self.preview {
            Some((target, collapsed)) if target == id => collapsed,
            _ => self.state.collapsed.contains_key(&id),
        }
    }

    pub fn preview(&self) -> Option<FoldId> {
        self.preview.map(|(id, _)| id)
    }

    /// Temporarily expand one node, preserving ancestor/descendant choices.
    /// None ends preview; switching restores the previous node first. Explicit
    /// set_collapsed/toggle/batch commands update the saved choice even during
    /// preview. Appends and reset preserve preview, using current ranges on exit.
    ///
    /// A change atomically costs one query and one object for at most two nodes'
    /// fixed index operations, O(log H + log C + log R + log F + log T), with O(1) extra storage.
    /// No source or covered tiles are scanned. An identical request is free.
    /// Rendering the newly visible window still needs its own shared budget.
    pub fn set_preview(
        &mut self,
        document: &Document,
        target: Option<FoldId>,
        budget: &mut Budget,
    ) -> Result<FoldPreviewProgress, LayoutError> {
        self.check(document)?;
        // Validate before charging or changing preview; row registration costs O(log R).
        if let Some(id) = target {
            let valid = match id {
                FoldId::Heading(id) => document.headings().get(id).is_some(),
                FoldId::Code(id) => document
                    .block(id)
                    .is_some_and(|block| matches!(block.kind, BlockKind::Code { .. })),
                FoldId::Table(id) => document.table(id).is_some(),
                FoldId::Container(id) => document.container(id).is_some(),
                FoldId::TableRows(range) => self.has_table_rows(document, range),
            };
            if !valid {
                return Err(LayoutError::InvalidRange);
            }
        }
        if self.preview() == target {
            return Ok(FoldPreviewProgress {
                completion: Completion::Covered,
                change: None,
            });
        }
        if !budget.charge(WorkLimit {
            queries: 1,
            objects: 1,
            ..Default::default()
        }) {
            return Ok(FoldPreviewProgress {
                completion: Completion::BudgetExhausted,
                change: None,
            });
        }
        let mut source = None;
        if let Some((id, collapsed)) = self.preview.take() {
            let candidate = document
                .fold_candidate(id)
                .expect("stable folding identity");
            source = Some(candidate.hidden);
            if collapsed {
                self.state.insert(document, &mut self.heights, candidate);
            }
        }
        if let Some(id) = target {
            let candidate = document.fold_candidate(id).unwrap();
            let collapsed = self.state.remove(document, &mut self.heights, id).is_some();
            self.preview = Some((id, collapsed));
            source = Some(source.map_or(candidate.hidden, |old| {
                Span::new(
                    old.start.min(candidate.hidden.start),
                    old.end.max(candidate.hidden.end),
                )
            }));
        }
        let source = source.expect("changed preview has an old or new target");
        self.version += 1;
        self.expire_from(document, source.start)?;
        Ok(FoldPreviewProgress {
            completion: Completion::Covered,
            change: Some(FoldChange {
                version: self.version,
                source,
            }),
        })
    }

    pub fn defaults(&self) -> FoldDefaults {
        self.defaults
    }

    /// This view's effective task state, or None for a non-task container.
    /// O(log U), where U is the number of overridden task items. Invalid IDs
    /// and unsynchronized/wrong documents are rejected before lookup.
    pub fn task_checked(
        &self,
        document: &Document,
        id: ContainerId,
    ) -> Result<Option<bool>, LayoutError> {
        self.check(document)?;
        let container = document.container(id).ok_or(LayoutError::InvalidRange)?;
        Ok(match container.kind {
            ContainerKind::Item {
                task: Some(task), ..
            } => Some(
                self.task_overrides
                    .get(&id)
                    .copied()
                    .unwrap_or(task.checked),
            ),
            _ => None,
        })
    }

    /// Override a task marker in this view without modifying source, syntax,
    /// fold choices or preview. Use an explicit filtered batch to refold items.
    /// Returning to the source state removes the override. A change invalidates
    /// view continuations and affected measurements; identical values are free.
    /// Returns the marker range for consumers drawing the effective state.
    /// O(log U + log T), with no scan of descendants or source text.
    pub fn set_task_checked(
        &mut self,
        document: &Document,
        id: ContainerId,
        checked: bool,
    ) -> Result<Option<FoldChange>, LayoutError> {
        let current = self
            .task_checked(document, id)?
            .ok_or(LayoutError::InvalidRange)?;
        if current == checked {
            return Ok(None);
        }
        let ContainerKind::Item {
            task: Some(task), ..
        } = document.container(id).unwrap().kind
        else {
            unreachable!()
        };
        if checked == task.checked {
            self.task_overrides.remove(&id);
        } else {
            self.task_overrides.insert(id, checked);
        }
        self.version += 1;
        self.expire_from(document, task.source.start)?;
        Ok(Some(FoldChange {
            version: self.version,
            source: task.source,
        }))
    }

    /// O(1). Applies only to newly identified headings/code blocks/tables/containers in future
    /// updates, including late syntax recognition. Existing choices stay intact;
    /// use FoldBatch to change them explicitly. Invalidates pending view tasks.
    pub fn set_defaults(&mut self, defaults: FoldDefaults) {
        if self.defaults != defaults {
            self.defaults = defaults;
            self.version += 1;
        }
    }

    /// New heading/block/table/container metadata inspected to initialize default choices.
    /// No historical candidates or source text are scanned.
    pub fn default_visits(&self) -> u64 {
        self.default_visits
    }

    pub(crate) fn check(&self, document: &Document) -> Result<(), LayoutError> {
        if self.document != document.id() {
            return Err(LayoutError::WrongDocument);
        }
        if self.revision != document.revision() {
            return Err(LayoutError::OutOfOrderUpdate);
        }
        Ok(())
    }

    pub fn hidden_at(
        &self,
        document: &Document,
        source: usize,
        budget: &mut Budget,
    ) -> Result<FoldLookup, LayoutError> {
        self.check(document)?;
        if source > document.source().len() {
            return Err(LayoutError::InvalidRange);
        }
        if !budget.charge(WorkLimit {
            queries: 1,
            ..Default::default()
        }) {
            return Ok(FoldLookup::BudgetExhausted);
        }
        Ok(match self.state.segment(document, source) {
            FoldSegment::Hidden(c) => FoldLookup::Hidden(c),
            _ => FoldLookup::Visible,
        })
    }

    pub fn set_collapsed(
        &mut self,
        document: &Document,
        id: FoldId,
        collapsed: bool,
    ) -> Result<Option<FoldChange>, LayoutError> {
        self.check(document)?;
        let candidate = document
            .fold_candidate(id)
            .ok_or(LayoutError::InvalidRange)?;
        if let FoldId::TableRows(range) = id
            && !self.has_table_rows(document, range)
        {
            return Err(LayoutError::InvalidRange);
        }
        if self.is_collapsed(id) == collapsed {
            return Ok(None);
        }
        if let Some((target, choice)) = &mut self.preview
            && *target == id
        {
            *choice = collapsed;
        } else if collapsed {
            self.state.insert(document, &mut self.heights, candidate);
        } else {
            self.state.remove(document, &mut self.heights, id);
        }
        self.version += 1;
        self.expire_from(document, candidate.hidden.start)?;
        Ok(Some(FoldChange {
            version: self.version,
            source: candidate.hidden,
        }))
    }

    pub fn toggle(&mut self, document: &Document, id: FoldId) -> Result<FoldChange, LayoutError> {
        Ok(self
            .set_collapsed(document, id, !self.is_collapsed(id))?
            .unwrap())
    }

    fn expire_from(&mut self, document: &Document, source: usize) -> Result<(), LayoutError> {
        // Even an empty EOF section changes view intent. Invalidate its final
        // tile so existing tickets and continuations cannot cross that change.
        if let Some(tile) = document
            .tile_at(source)
            .or_else(|| document.tile_at(source.saturating_sub(1)))
        {
            self.heights.expire(tile.0..self.heights.len())?;
            // Includes any out-of-document states installed by a renderer.
            self.checkpoints.invalidate(tile.0..isize::MAX as usize)?;
        }
        Ok(())
    }

    fn refresh_section(&mut self, document: &Document, id: HeadingId, dirty: &mut Option<usize>) {
        let id = FoldId::Heading(id);
        let Some(mask) = self.state.collapsed.get(&id) else {
            return;
        };
        if !mask.candidate.open {
            return;
        }
        self.range_visits += 1;
        let old = mask.candidate;
        let new = document.fold_candidate(id).unwrap();
        self.state.refresh(document, &mut self.heights, new);
        *dirty = Some(
            dirty
                .unwrap_or(usize::MAX)
                .min(old.hidden.start)
                .min(new.hidden.start),
        );
    }

    /// Consume each consecutive append/finish (or one merged batch) before any
    /// query. New tiles inherit all open folds before this call returns.
    pub fn apply(&mut self, document: &Document, update: &Update) -> Result<(), LayoutError> {
        let changed = update.validate(document, Some(self.document), Some(self.revision))?;
        self.heights.validate_update(document, update)?;
        self.checkpoints.validate_update(document, update)?;
        if !changed {
            return Ok(());
        }
        crate::apply_layout_update(document, update, &mut self.heights, &mut self.checkpoints)?;
        // Six root sections, one code/table, and at most one disjoint range
        // containing the sole incomplete table row. No historical range scan.
        debug_assert!(self.state.growing.len() <= 8);
        let mut growing = [None; 8];
        for (slot, &id) in growing.iter_mut().zip(&self.state.growing) {
            *slot = Some(id);
        }
        let mut dirty = self.state.open_ranges.first().map(|&(start, _)| start);
        if dirty.is_some() {
            self.range_visits += 1; // One shared tail, independent of open depth.
        }
        if let Some(id) = self.state.row_boundary.take() {
            self.range_visits += 1;
            let candidate = document.fold_candidate(id).unwrap();
            // Tile membership can change while the fixed row endpoint does not.
            self.state.refresh(document, &mut self.heights, candidate);
            dirty = Some(dirty.unwrap_or(usize::MAX).min(candidate.hidden.start));
        }
        for &id in &update.closed_containers {
            if self.state.collapsed.contains_key(&FoldId::Container(id)) {
                self.range_visits += 1;
                let new = document.fold_candidate(FoldId::Container(id)).unwrap();
                self.state.refresh(document, &mut self.heights, new);
                dirty = Some(dirty.unwrap_or(usize::MAX).min(new.hidden.start));
            }
            // Six level tails, not every heading ever seen in this scope.
            for heading in document.headings().scope_tails(id).into_iter().flatten() {
                self.refresh_section(document, heading, &mut dirty);
            }
        }
        for index in update.new_headings.clone() {
            let id = HeadingId(index);
            if document.headings().get(id).unwrap().container.is_some() {
                for previous in document.headings().closing_before(id).into_iter().flatten() {
                    self.refresh_section(document, previous, &mut dirty);
                }
            }
        }
        // Only the previous last title can grow. Later headings are new to this
        // view. Body-only growth changes the shared end, never the index leaves.
        if let Some(index) = update.new_headings.start.checked_sub(1) {
            let id = HeadingId(index);
            let heading = document.headings().get(id).unwrap();
            if heading.container.is_some()
                && let Some(mask) = self.state.collapsed.get(&FoldId::Heading(id))
                && mask.candidate.open
                && (mask.candidate.hidden.start != heading.source.end
                    || mask.candidate.hidden.is_empty())
            {
                self.refresh_section(document, id, &mut dirty);
            }
        }
        self.state.refresh_open_tiles(document, &mut self.heights);
        for id in growing.into_iter().flatten() {
            self.range_visits += 1;
            let old = self.state.collapsed[&id].candidate;
            let new = document
                .fold_candidate(id)
                .expect("stable folding identity");
            if old == new {
                continue;
            }
            self.state.refresh(document, &mut self.heights, new);
            dirty = Some(
                dirty
                    .unwrap_or(usize::MAX)
                    .min(old.hidden.start)
                    .min(new.hidden.start),
            );
        }
        if self
            .defaults
            .heading_levels
            .iter()
            .any(|&collapsed| collapsed)
        {
            for index in update.new_headings.clone() {
                self.default_visits += 1;
                let heading = document.headings().get(HeadingId(index)).unwrap();
                if self.defaults.heading_levels[usize::from(heading.level - 1)] {
                    let candidate = document
                        .fold_candidate(FoldId::Heading(HeadingId(index)))
                        .unwrap();
                    self.state.insert(document, &mut self.heights, candidate);
                    dirty = Some(dirty.unwrap_or(usize::MAX).min(candidate.hidden.start));
                }
            }
        }
        if self.defaults.code {
            for index in update.new_blocks.clone() {
                self.default_visits += 1;
                if let Some(candidate) = document.fold_candidate(FoldId::Code(BlockId(index))) {
                    self.state.insert(document, &mut self.heights, candidate);
                    dirty = Some(dirty.unwrap_or(usize::MAX).min(candidate.hidden.start));
                }
            }
        }
        if self.defaults.table {
            // A header can convert an existing paragraph block into a table;
            // new_blocks alone would miss that late confirmation.
            for index in update.new_tables.clone() {
                self.default_visits += 1;
                let candidate = document
                    .fold_candidate(FoldId::Table(TableId(index)))
                    .unwrap();
                self.state.insert(document, &mut self.heights, candidate);
                dirty = Some(dirty.unwrap_or(usize::MAX).min(candidate.hidden.start));
            }
        }
        if self.defaults.container_depth.is_some()
            || self.defaults.containers != ContainerFoldFilter::default()
        {
            // All predicates use stored metadata. Neither historical nodes nor
            // the active ancestor chain need visiting, even for a merged update.
            for index in update.new_containers.clone() {
                self.default_visits += 1;
                let id = ContainerId(index);
                let container = document.container(id).unwrap();
                let checked = match container.kind {
                    ContainerKind::Item { task, .. } => task.map(|t| t.checked),
                    _ => None,
                };
                if self
                    .defaults
                    .container_depth
                    .is_some_and(|depth| container.depth >= depth)
                    || self.defaults.containers.matches(container.kind, checked)
                {
                    let candidate = document.fold_candidate(FoldId::Container(id)).unwrap();
                    self.state.insert(document, &mut self.heights, candidate);
                    dirty = Some(dirty.unwrap_or(usize::MAX).min(candidate.hidden.start));
                }
            }
        }
        if let Some(start) = dirty {
            self.expire_from(document, start)?;
        }
        self.revision = update.revision;
        self.version += 1;
        Ok(())
    }

    pub fn reset(&mut self, estimated_rows: u64) -> Result<(), LayoutError> {
        self.heights.reset(estimated_rows)?;
        self.checkpoints.clear();
        self.version += 1;
        Ok(())
    }

    pub fn measure(&mut self, ticket: Measurement, rows: u64) -> Result<(), LayoutError> {
        self.heights.measure_from(ticket, rows, &self.checkpoints)
    }

    pub fn insert_checkpoint(&mut self, tile: TileId, state: S) -> Result<Option<S>, LayoutError> {
        self.checkpoints
            .insert_for_layout(&mut self.heights, tile, state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_override_storage_tracks_current_differences_not_toggle_history() {
        let mut doc = Document::new();
        doc.append(&"- [ ] task\n".repeat(4096)).unwrap();
        doc.finish();
        let mut view = FoldView::new(&doc, 1).unwrap();
        for i in 1..=4096 {
            view.set_task_checked(&doc, ContainerId(i), true).unwrap();
        }
        assert_eq!(view.task_overrides.len(), 4096);
        for i in 1..=4096 {
            view.set_task_checked(&doc, ContainerId(i), false).unwrap();
        }
        assert!(view.task_overrides.is_empty());
        let nodes = view.heights().node_count();
        for _ in 0..1000 {
            view.set_task_checked(&doc, ContainerId(1), true).unwrap();
            assert_eq!(view.task_overrides.len(), 1);
            view.set_task_checked(&doc, ContainerId(1), false).unwrap();
            assert!(view.task_overrides.is_empty());
        }
        assert_eq!(view.heights().node_count(), nodes);
    }
}
