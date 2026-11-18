use std::{
    collections::{BTreeMap, BTreeSet},
    ops::{Bound, Range},
};

use crate::{
    BlockId, BlockKind, Budget, CheckpointIndex, Completion, ContainerId, ContainerKind, Document,
    DocumentId, HeadingId, HeightIndex, LayoutError, Measurement, SourceFoldId, Span, SyntaxState,
    TableId, TileId, Update, WorkLimit,
};

/// Syntax IDs are document-local; Source IDs identify one view registration.
/// Source overlays can nest/cross syntax and other source overlays; original
/// strict registration APIs retain their conservative validation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FoldId {
    Heading(HeadingId),
    Code(BlockId),
    Math(BlockId),
    Table(TableId),
    Container(ContainerId),
    TableRows(TableRowRange),
    Source(SourceFoldId),
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

/// Initial choices for syntax candidates first discovered by a subsequent
/// `apply`, and paragraph registrations created by ParagraphAutoFold.
/// Existing choices are never overwritten, including explicit expansions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FoldDefaults {
    /// Index 0 is h1, index 5 is h6. True starts that heading collapsed.
    pub heading_levels: [bool; 6],
    pub code: bool,
    pub math: bool,
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
    /// Opt-in source-byte paragraph policy. Unlike syntax defaults, registration
    /// requires running ParagraphAutoFold with a shared budget. Only newly
    /// registered paragraphs are initialized; existing choices stay intact.
    pub paragraphs: Option<crate::ParagraphFoldPolicy>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FoldCandidate {
    pub id: FoldId,
    pub parent: Option<FoldId>,
    /// Whole section, code/math block, table, registered range or container, including the retained heading.
    pub source: Span,
    /// Body replaced by a placeholder. Empty bodies produce no placeholder.
    /// Registry queries return the actual interval. Visibility/layout queries
    /// return a union projection: its start is the component's placeholder and
    /// its end is the currently discovered right frontier, possibly beyond this
    /// owner's source envelope. Later extensions are separate budgeted steps.
    pub hidden: Span,
    /// Title, fence info, table header, first selected row, source or container range.
    pub summary: Option<Span>,
    pub open: bool,
}

impl FoldCandidate {
    // Preorder tie-break: containers on a physical opener line precede its
    // leaf block, then source registrations. Strict containing source ranges
    // precede their contents, including when they share a start.
    pub(crate) fn contains_candidate(&self, other: &Self) -> bool {
        let order = |candidate: &Self| match candidate.id {
            FoldId::Container(id) => (candidate.source.start, 0, id.0),
            FoldId::Source(id) => (candidate.source.start, 2, id.0),
            _ => (candidate.source.start, 1, 0),
        };
        self.source.start <= other.source.start
            && other.source.end <= self.source.end
            && (self.source != other.source || order(self) <= order(other))
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
    /// A registered paragraph suffix ends this paragraph. The renderer should
    /// finish its placeholder's line in `owner`, even when EOF has no newline.
    /// Other fold kinds leave line/placeholder policy to the renderer.
    pub paragraph: Option<BlockId>,
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
            BlockKind::Math { .. } => Some(FoldId::Math(BlockId(self.next))),
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
    boundary: Option<(u64, bool)>,
}

struct SourceBoundary {
    members: usize,
    covers: usize,
    tile: Option<usize>,
    end: usize,
}

#[derive(Default)]
pub(crate) struct FoldState {
    collapsed: BTreeMap<FoldId, Mask>,
    starts: BTreeSet<(usize, FoldId)>,
    // Row registrations remain disjoint. Source envelopes and selected hidden
    // intervals have independent augmented indexes, including duplicate starts.
    // The strict-only index keeps legacy row validation independent of overlays.
    row_ranges: BTreeMap<usize, TableRowRange>,
    source_ranges: crate::fold_intervals::Intervals,
    hidden_ranges: crate::fold_intervals::Intervals,
    strict_source_ranges: crate::fold_intervals::Intervals,
    source_candidates: BTreeMap<SourceFoldId, SourceCandidate>,
    // The sole open logical paragraph, even if expanded or previewed.
    open_paragraph: Option<SourceFoldId>,
    // Document-tail IDs share EOF in both registry and selected mask indexes.
    open_source_tails: BTreeSet<SourceFoldId>,
    // A generation groups collapsed fixed cuts at the same old EOF. One shared
    // last-tile cover is corrected on the next apply; surviving complete tiles
    // retain it until their last member is removed. No history/range scan.
    source_boundary_generation: u64,
    source_boundaries: BTreeMap<u64, SourceBoundary>,
    // A completed row range or code/math block can end at EOF inside a growable
    // tile. These cannot share EOF, so one slot suffices for the next refresh.
    fixed_boundary: Option<FoldId>,
    // Six root sections, the active code/math/table and one incomplete row range
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
struct SourceCandidate {
    keep_empty: bool,
    candidate: FoldCandidate,
    paragraph: Option<BlockId>,
    flexible: bool,
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
            FoldId::Source(_) => None, // Registrations belong to a FoldView.
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
            FoldId::Code(block_id) | FoldId::Math(block_id) => {
                let block = self.block(block_id)?;
                let summary = match (id, &block.kind) {
                    (FoldId::Code(_), BlockKind::Code { info }) => *info,
                    (FoldId::Math(_), BlockKind::Math { .. }) => None,
                    _ => return None,
                };
                let open = block.state != SyntaxState::Sealed && !self.is_finished();
                // Pending physical-line prefixes belong provisionally to an open
                // code/math block; classification can later retract that tail.
                let source = Span::new(
                    block.source.start,
                    if open {
                        self.source().len()
                    } else {
                        block.source.end
                    },
                );
                Some(FoldCandidate {
                    id,
                    parent: self.fold_parent(block.container, source.start),
                    source,
                    hidden: source,
                    summary,
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
            BlockKind::Math { .. } => FoldId::Math(block),
            BlockKind::Table { id } => FoldId::Table(id),
            _ => return None,
        })
    }
}

impl FoldState {
    fn source_key(entry: SourceCandidate) -> crate::fold_intervals::Key {
        let c = entry.candidate;
        crate::fold_intervals::key(
            c.source.start,
            if c.open && entry.paragraph.is_none() {
                usize::MAX
            } else {
                c.source.end
            },
            c.id,
        )
    }

    fn hidden_key(&self, c: FoldCandidate) -> crate::fold_intervals::Key {
        crate::fold_intervals::key(
            c.hidden.start,
            if c.open || matches!(c.id, FoldId::Source(id) if self.open_source_tails.contains(&id))
            {
                usize::MAX
            } else {
                c.hidden.end
            },
            c.id,
        )
    }

    fn source_candidate(&self, document: &Document, id: SourceFoldId) -> Option<FoldCandidate> {
        let entry = *self.source_candidates.get(&id)?;
        let mut c = entry.candidate;
        if self.open_source_tails.contains(&id) {
            c.source.end = document.source().len();
            c.hidden.end = c.source.end;
            c.summary = Some(c.source);
            c.open = !document.is_finished();
        }
        let syntax = c.parent.and_then(|id| document.fold_candidate(id));
        c.parent = syntax
            .filter(|p| p.source.start <= c.source.start && c.source.end <= p.source.end)
            .map(|p| p.id);
        if let Some(parent) = self.source_ranges.parent(Self::source_key(entry)) {
            let outer = Span::new(parent.0, parent.1.0.min(document.source().len()));
            if c.parent.is_none()
                || syntax
                    .is_some_and(|p| p.source.start <= outer.start && outer.end <= p.source.end)
            {
                c.parent = Some(parent.3);
            }
        }
        Some(c)
    }

    fn mask_interior(&self, document: &Document, c: FoldCandidate) -> Range<usize> {
        let mut tiles = Self::interior(document, c.hidden);
        if matches!(c.id, FoldId::Source(_))
            && !c.open
            && c.hidden.end == document.source().len()
            && !document.is_finished()
            && let Some(last) = document.tile_at(c.hidden.end.saturating_sub(1))
        {
            tiles.end = tiles.end.min(last.0).max(tiles.start);
        }
        tiles
    }

    fn add_source_boundary(
        &mut self,
        document: &Document,
        heights: &mut HeightIndex,
        c: FoldCandidate,
    ) -> Option<(u64, bool)> {
        if matches!(c.id, FoldId::Source(_))
            && !c.open
            && c.hidden.end == document.source().len()
            && !document.is_finished()
        {
            let group = self
                .source_boundaries
                .entry(self.source_boundary_generation)
                .or_insert(SourceBoundary {
                    members: 0,
                    covers: 0,
                    tile: None,
                    end: c.hidden.end,
                });
            group.members += 1;
            let full = Self::interior(document, c.hidden);
            let covered = full.end > full.start;
            if covered {
                let tile = full.end - 1;
                if group.covers == 0 {
                    group.tile = Some(tile);
                    heights.cover(tile..tile + 1, true);
                }
                group.covers += 1;
            }
            return Some((self.source_boundary_generation, covered));
        }
        None
    }

    pub(crate) fn paragraph(&self, id: FoldId) -> Option<BlockId> {
        let FoldId::Source(id) = id else { return None };
        self.source_candidates.get(&id)?.paragraph
    }

    pub(crate) fn covering(&self, document: &Document, offset: usize) -> Option<FoldCandidate> {
        if offset >= document.source().len() {
            return None;
        }
        let key = self
            .hidden_ranges
            .next(Bound::Unbounded, offset, offset + 1)?;
        match key.3 {
            FoldId::Source(id) => self.source_candidate(document, id),
            id => document.fold_candidate(id),
        }
    }

    // The shared EOF sentinel must not demote a real syntax ancestor when
    // finish makes that ancestor finite. Existing topology indexes resolve the
    // outer syntax owner without walking a container/scoped-heading chain.
    fn syntax_hidden(&self, document: &Document, offset: usize) -> Option<FoldCandidate> {
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
        if hidden.is_none()
            && let Some((_, &range)) = self.row_ranges.range(..=offset).next_back()
            && let Some(mask) = self.collapsed.get(&FoldId::TableRows(range))
            && mask.candidate.hidden.range().contains(&offset)
        {
            hidden = Some(mask.candidate);
        }
        hidden
    }

    pub(crate) fn segment(&self, document: &Document, offset: usize) -> FoldSegment {
        if offset < document.source().len()
            && let Some(component) = self.hidden_ranges.component(offset)
        {
            let id = component.owner.3;
            let mut candidate = match id {
                FoldId::Source(id) => self
                    .source_candidate(document, id)
                    .expect("registered source"),
                _ => document.fold_candidate(id).expect("stable syntax identity"),
            };
            if component.owner.1.0 == usize::MAX
                && matches!(candidate.id, FoldId::Source(_))
                && let Some(parent) = self.syntax_hidden(document, component.owner.0)
                && parent.hidden.start == component.owner.0
                && parent.source.start <= candidate.source.start
                && candidate.source.end <= parent.source.end
            {
                candidate = parent;
            }
            candidate.hidden = Span::new(
                component.owner.0,
                component.end.min(document.source().len()),
            );
            return FoldSegment::Hidden(candidate);
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
        self.hidden_ranges.remove(self.hidden_key(mask.candidate));
        if let Some((generation, covered)) = mask.boundary {
            let group = self
                .source_boundaries
                .get_mut(&generation)
                .expect("paired source boundary");
            group.members -= 1;
            if covered {
                group.covers -= 1;
                if group.covers == 0
                    && let Some(tile) = group.tile.take()
                {
                    heights.cover(tile..tile + 1, false);
                }
            }
            if group.members == 0 {
                self.source_boundaries.remove(&generation);
            }
        }
        if self.fixed_boundary == Some(id) {
            self.fixed_boundary = None;
        }
        if self.shared_range(document, id) {
            if !matches!(id, FoldId::Source(_)) {
                self.range_index(id).set(Self::range_id(id), 0);
            }
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
            self.mask_interior(document, candidate)
        };
        if shared && !matches!(candidate.id, FoldId::Source(_)) {
            self.range_index(candidate.id).set(
                Self::range_id(candidate.id),
                if candidate.open {
                    usize::MAX
                } else {
                    hidden.end
                },
            );
        }
        if open_shared {
            self.open_ranges.insert((hidden.start, candidate.id));
            self.refresh_open_tiles(document, heights);
        }
        if !hidden.is_empty() {
            self.starts.insert((hidden.start, candidate.id));
        }
        heights.cover(tiles.clone(), true);
        if candidate.open && !open_shared && !matches!(candidate.id, FoldId::Source(_)) {
            self.growing.insert(candidate.id);
        }
        self.hidden_ranges.insert(self.hidden_key(candidate));
        let boundary = self.add_source_boundary(document, heights, candidate);
        self.collapsed.insert(
            candidate.id,
            Mask {
                candidate,
                tiles,
                boundary,
            },
        );
        self.track_fixed_boundary(document, candidate);
    }

    fn track_fixed_boundary(&mut self, document: &Document, candidate: FoldCandidate) {
        if matches!(
            candidate.id,
            FoldId::TableRows(_) | FoldId::Code(_) | FoldId::Math(_)
        ) {
            if !candidate.open
                && candidate.hidden.end == document.source().len()
                && !document.is_finished()
            {
                self.fixed_boundary = Some(candidate.id);
            } else if self.fixed_boundary == Some(candidate.id) {
                self.fixed_boundary = None;
            }
        }
    }

    fn shared_range(&self, document: &Document, id: FoldId) -> bool {
        match id {
            FoldId::Container(_) => true,
            FoldId::Heading(id) => document.headings().get(id).unwrap().container.is_some(),
            FoldId::Source(id) => self.open_source_tails.contains(&id),
            _ => false,
        }
    }

    fn range_id(id: FoldId) -> usize {
        match id {
            FoldId::Container(id) => id.0,
            FoldId::Heading(id) => id.0,
            FoldId::Source(_) => 0,
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
        self.remove(document, heights, candidate.id)
            .expect("existing mask");
        self.insert(document, heights, candidate);
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
    pub(crate) paragraph_auto_excluded: BTreeSet<BlockId>,
    // Only values differing from the source marker; bounded by task item count.
    task_overrides: BTreeMap<ContainerId, bool>,
    // The active node is temporarily absent from the effective masks. Keep only
    // its local choice; ancestor/descendant choices remain in the usual index.
    preview: Option<(FoldId, bool)>,
}

/// Owns the entire view while its checkpoint arena is compacted. Heights,
/// masks, defaults and choices are moved, never copied. No mutable view is
/// accessible until finish, preventing queries/updates/toggles during maintenance.
pub struct FoldViewCompaction<S> {
    view: FoldView<S>,
    task: crate::CheckpointCompaction<S>,
}

impl<S> FoldViewCompaction<S> {
    /// Read-only geometry remains owned by this same paused view. It is safe
    /// for a host to retain source/scroll anchors while queuing a width change.
    pub fn heights(&self) -> &HeightIndex {
        self.view.heights()
    }

    /// Read the retained checkpoint while its owner is in maintenance.
    pub fn checkpoint(&self, tile: TileId) -> Option<&S> {
        self.task.get(tile)
    }

    pub fn previous_checkpoint(&self, tile: TileId) -> Option<(TileId, &S)> {
        self.task.previous(tile)
    }

    pub fn checkpoint_token(&self, tile: TileId) -> Option<crate::CheckpointToken> {
        self.task.token(tile)
    }

    pub fn validates_checkpoint(&self, token: crate::CheckpointToken) -> bool {
        self.task.validates(token)
    }

    pub fn checkpoint_len(&self) -> usize {
        self.task.len()
    }

    pub fn stats(&self) -> crate::CheckpointCompactionStats {
        self.task.stats()
    }

    /// The document must remain at the captured revision. Retired payloads are
    /// returned to the host, whose destructors are not preempted by Budget.
    pub fn advance(
        &mut self,
        document: &Document,
        budget: &mut Budget,
        retired: &mut Vec<S>,
    ) -> Result<crate::Completion, LayoutError> {
        self.view.check(document)?;
        Ok(self.task.advance(budget, retired))
    }

    /// Restore only this owned view and its original checkpoint identity/tokens.
    /// An unfinished task or a mismatched document/revision returns the task
    /// unchanged. Do not append/finish the document during maintenance.
    #[allow(clippy::result_large_err)] // Preserve ownership without allocating another box.
    pub fn finish(self, document: &Document) -> Result<FoldView<S>, Self> {
        if self.view.check(document).is_err() {
            return Err(self);
        }
        match self.task.finish() {
            Ok(checkpoints) => {
                let mut view = self.view;
                view.checkpoints = checkpoints;
                Ok(view)
            }
            Err(task) => Err(Self {
                view: self.view,
                task,
            }),
        }
    }
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
            paragraph_auto_excluded: BTreeSet::new(),
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

    /// Borrow the single authoritative layout indexes for a trusted renderer.
    /// Use measurement/Local and base-only zero-range measurement, expire/invalidate, checkpoint insertion
    /// and bounded drain operations. Incoming-state changes must expire heights
    /// AND invalidate downstream checkpoints (or use insert_for_layout).
    ///
    /// Never rebuild/reset/suppress the heights, apply updates directly, or
    /// take/replace either index: that bypasses fold masks and lifecycle checks.
    /// Use FoldView::apply/reset/fold operations and into_checkpoint_compaction
    /// instead. Measurement tickets and checkpoint tokens retain their existing
    /// dependency/generation validation; a semantic cache key alone is insufficient.
    pub fn layout_parts_mut(
        &mut self,
        document: &Document,
    ) -> Result<(&mut HeightIndex, &mut CheckpointIndex<S>), LayoutError> {
        self.check(document)?;
        Ok((&mut self.heights, &mut self.checkpoints))
    }

    /// Move this view into budgeted checkpoint maintenance in O(1). The host
    /// must hold the document revision stable until finish; no full height tree
    /// or fold state is cloned and there is no unchecked replacement setter.
    pub fn into_checkpoint_compaction(mut self) -> FoldViewCompaction<S> {
        let checkpoints = std::mem::take(&mut self.checkpoints);
        FoldViewCompaction {
            view: self,
            task: checkpoints.into_compaction(),
        }
    }
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Whether any local collapsed choice can affect projection. Preview may
    /// temporarily expose one of these choices without deleting it.
    pub fn has_collapsed(&self) -> bool {
        !self.state.collapsed.is_empty()
    }

    /// The contiguous unfolded context containing a known visible source byte.
    /// Unlike a projection's tile/window cuts these are actual fold boundaries.
    /// Display/shaping consumers intersect this context with their block/cell
    /// boundary; they must not use a viewport cut as a grapheme boundary.
    pub fn visible_fragment(
        &self,
        document: &Document,
        source: usize,
        budget: &mut Budget,
    ) -> Result<Option<Span>, LayoutError> {
        self.check(document)?;
        if source >= document.source().len() {
            return Err(LayoutError::InvalidRange);
        }
        if !budget.charge(crate::WorkLimit {
            queries: 1,
            objects: 1,
            ..Default::default()
        }) {
            return Ok(None);
        }
        match self.state.segment(document, source) {
            FoldSegment::Hidden(_) => Err(LayoutError::InvalidRange),
            FoldSegment::Text { end } => Ok(Some(Span::new(
                self.state.hidden_ranges.preceding_end(source),
                end,
            ))),
        }
    }

    /// Query a candidate in this synchronized view, including registered source
    /// and table-row ranges. Unknown/removed/other-view registrations return None.
    pub fn fold_candidate(
        &self,
        document: &Document,
        id: FoldId,
    ) -> Result<Option<FoldCandidate>, LayoutError> {
        self.check(document)?;
        Ok(self.candidate(document, id))
    }

    pub(crate) fn candidate(&self, document: &Document, id: FoldId) -> Option<FoldCandidate> {
        match id {
            FoldId::Source(id) => self.state.source_candidate(document, id),
            FoldId::TableRows(range) if !self.has_table_rows(document, range) => None,
            _ => document.fold_candidate(id),
        }
    }

    /// First registered envelope containing a byte, even when expanded: earliest
    /// start, longest stored end, then ID. Live tails use an unbounded end rank,
    /// retained after finish; explicit sealing makes it finite. Use budgeted
    /// SourceFoldCandidates to discover every intersecting envelope.
    /// O(log R), no ancestor walk or body read. EOF belongs to no interval.
    pub fn source_fold_at(
        &self,
        document: &Document,
        offset: usize,
    ) -> Result<Option<FoldCandidate>, LayoutError> {
        self.check(document)?;
        if offset > document.source().len() {
            return Err(LayoutError::InvalidRange);
        }
        Ok(if offset == document.source().len() {
            None
        } else {
            self.source_at(document, offset)
        })
    }

    fn source_at(&self, document: &Document, offset: usize) -> Option<FoldCandidate> {
        let key = self
            .state
            .source_ranges
            .next(Bound::Unbounded, offset, offset + 1)?;
        let FoldId::Source(id) = key.3 else {
            unreachable!()
        };
        self.state.source_candidate(document, id)
    }

    pub(crate) fn next_source_fold(
        &self,
        document: &Document,
        after: Bound<crate::fold_intervals::Key>,
        start: usize,
        end: usize,
    ) -> Option<FoldCandidate> {
        let key = self.state.source_ranges.next(after, start, end)?;
        let FoldId::Source(id) = key.3 else {
            unreachable!()
        };
        self.state.source_candidate(document, id)
    }

    pub(crate) fn has_open_paragraph(&self) -> bool {
        self.state.open_paragraph.is_some()
    }

    pub(crate) fn source_fold_is_tail(&self, id: SourceFoldId) -> bool {
        self.state.open_source_tails.contains(&id)
    }

    pub(crate) fn source_fold_is_flexible(&self, id: SourceFoldId) -> bool {
        self.state.source_candidates[&id].flexible
    }

    pub(crate) fn contains_candidate(&self, outer: &FoldCandidate, inner: &FoldCandidate) -> bool {
        if outer.source == inner.source
            && matches!(outer.id, FoldId::Source(_))
            && matches!(inner.id, FoldId::Source(_))
        {
            // Live/finished tail sentinels retain their envelope order until
            // explicitly sealed; batches must agree with lazy parent lookup.
            self.source_cursor(outer.id) <= self.source_cursor(inner.id)
        } else {
            outer.contains_candidate(inner)
        }
    }

    pub(crate) fn source_cursor(&self, id: FoldId) -> crate::fold_intervals::Key {
        let FoldId::Source(id) = id else {
            unreachable!()
        };
        FoldState::source_key(self.state.source_candidates[&id])
    }

    /// Stable paragraph block bound to a source registration, or None for a
    /// manual/removed/other-view ID. O(log R), no body read.
    pub fn paragraph_fold_block(
        &self,
        document: &Document,
        id: SourceFoldId,
    ) -> Result<Option<BlockId>, LayoutError> {
        self.check(document)?;
        Ok(self.state.paragraph(FoldId::Source(id)))
    }

    /// Active registrations only; removing a range releases its registry entries.
    pub fn source_fold_count(&self) -> usize {
        self.state.source_candidates.len()
    }

    pub(crate) fn validate_source_span(&self, source: Span) -> Result<(), LayoutError> {
        if self
            .state
            .source_ranges
            .next(Bound::Unbounded, source.start, source.end)
            .is_some()
        {
            return Err(LayoutError::InvalidRange);
        }
        Ok(())
    }

    pub(crate) fn row_candidate_at(
        &self,
        document: &Document,
        offset: usize,
    ) -> Option<FoldCandidate> {
        let (_, &rows) = self.state.row_ranges.range(..=offset).next_back()?;
        let candidate = document.fold_candidate(FoldId::TableRows(rows))?;
        candidate
            .source
            .range()
            .contains(&offset)
            .then_some(candidate)
    }

    pub(crate) fn register_source_fold(
        &mut self,
        source: Span,
        hidden: Span,
        parent: Option<FoldId>,
        paragraph: Option<BlockId>,
        open: bool,
        flexible: bool,
    ) -> FoldId {
        let id = SourceFoldId::new();
        self.state.source_candidates.insert(
            id,
            SourceCandidate {
                keep_empty: false,
                candidate: FoldCandidate {
                    id: FoldId::Source(id),
                    parent,
                    source,
                    hidden,
                    summary: Some(if paragraph.is_some() {
                        Span::new(source.start, hidden.start)
                    } else {
                        source
                    }),
                    open,
                },
                paragraph,
                flexible,
            },
        );
        self.state
            .source_ranges
            .insert(FoldState::source_key(self.state.source_candidates[&id]));
        if !self.state.source_candidates[&id].flexible {
            self.state
                .strict_source_ranges
                .insert(FoldState::source_key(self.state.source_candidates[&id]));
        }
        if open {
            if paragraph.is_some() {
                debug_assert!(self.state.open_paragraph.is_none());
                self.state.open_paragraph = Some(id);
            } else {
                self.state.open_source_tails.insert(id);
            }
        }
        self.version += 1;
        FoldId::Source(id)
    }

    pub(crate) fn live_source_tail(
        &self,
        document: &Document,
        id: SourceFoldId,
    ) -> Option<FoldCandidate> {
        (self.state.open_source_tails.contains(&id) && !document.is_finished())
            .then(|| self.state.source_candidate(document, id).unwrap())
    }

    pub(crate) fn seal_source_tail(
        &mut self,
        document: &Document,
        id: SourceFoldId,
    ) -> Result<FoldId, LayoutError> {
        let mut candidate = self
            .live_source_tail(document, id)
            .ok_or(LayoutError::InvalidRange)?;
        let collapsed = self
            .state
            .remove(document, &mut self.heights, candidate.id)
            .is_some();
        self.state
            .source_ranges
            .remove(FoldState::source_key(self.state.source_candidates[&id]));
        self.state
            .strict_source_ranges
            .remove(FoldState::source_key(self.state.source_candidates[&id]));
        candidate.open = false;
        candidate.parent = self.state.source_candidates[&id].candidate.parent;
        self.state.source_candidates.get_mut(&id).unwrap().candidate = candidate;
        self.state.open_source_tails.remove(&id);
        self.state
            .source_ranges
            .insert(FoldState::source_key(self.state.source_candidates[&id]));
        if !self.state.source_candidates[&id].flexible {
            self.state
                .strict_source_ranges
                .insert(FoldState::source_key(self.state.source_candidates[&id]));
        }
        if collapsed {
            self.state.insert(document, &mut self.heights, candidate);
        }
        self.version += 1;
        self.expire_from(document, candidate.hidden.start)?;
        Ok(candidate.id)
    }

    /// Move only the prefix cut of an existing flexible paragraph registration.
    /// The host, envelope, ID, saved choice and preview remain the same. The
    /// renderer supplies a complete display-atom cut; core checks UTF-8/CRLF.
    /// A cut at the current end is allowed and has no placeholder until growth.
    /// One query/object pays fixed indexed mask/checkpoint work, no body scan.
    pub fn set_paragraph_fold_cut(
        &mut self,
        document: &Document,
        id: SourceFoldId,
        cut: usize,
        budget: &mut crate::Budget,
    ) -> Result<Option<FoldChange>, LayoutError> {
        self.check(document)?;
        let entry = self
            .state
            .source_candidates
            .get(&id)
            .copied()
            .filter(|e| e.flexible && e.paragraph.is_some())
            .ok_or(LayoutError::InvalidRange)?;
        let old = entry.candidate;
        if cut < old.source.start
            || cut > old.source.end
            || !document.source().is_char_boundary(cut)
            || cut > old.source.start
                && cut < old.source.end
                && document.source().as_bytes()[cut - 1] == b'\r'
                && document.source().as_bytes()[cut] == b'\n'
        {
            return Err(LayoutError::InvalidRange);
        }
        if cut == old.hidden.start {
            return Ok(Some(FoldChange {
                version: self.version,
                source: Span::new(cut, cut),
            }));
        }
        if !budget.charge(crate::WorkLimit {
            queries: 1,
            objects: 1,
            ..Default::default()
        }) {
            return Ok(None);
        }
        let selected = self
            .state
            .remove(document, &mut self.heights, old.id)
            .is_some();
        let candidate = FoldCandidate {
            hidden: Span::new(cut, old.hidden.end),
            summary: Some(Span::new(old.source.start, cut)),
            ..old
        };
        let entry = self.state.source_candidates.get_mut(&id).unwrap();
        entry.candidate = candidate;
        entry.keep_empty |= candidate.hidden.is_empty();
        if selected {
            self.state.insert(document, &mut self.heights, candidate);
        }
        self.version += 1;
        let start = old.hidden.start.min(cut);
        self.expire_from(document, start)?;
        Ok(Some(FoldChange {
            version: self.version,
            source: Span::new(start, old.hidden.end),
        }))
    }

    /// Remove one registration, choice and preview; other IDs never change and
    /// this ID is never reused. A paragraph removal also excludes its BlockId
    /// from automatic registration; clear with set_paragraph_auto_excluded.
    /// O(log R + log F + log T + log E), no covered-tile scan.
    pub fn remove_source_fold(
        &mut self,
        document: &Document,
        id: SourceFoldId,
    ) -> Result<Option<FoldChange>, LayoutError> {
        self.check(document)?;
        let Some(candidate) = self.state.source_candidate(document, id) else {
            return Ok(None);
        };
        let entry = self.state.source_candidates.remove(&id).unwrap();
        if let Some(block) = entry.paragraph {
            self.paragraph_auto_excluded.insert(block);
        }
        self.state
            .source_ranges
            .remove(FoldState::source_key(entry));
        self.state
            .strict_source_ranges
            .remove(FoldState::source_key(entry));
        if self.state.open_paragraph == Some(id) {
            self.state.open_paragraph = None;
        }
        self.state
            .remove(document, &mut self.heights, FoldId::Source(id));
        self.state.open_source_tails.remove(&id);
        if self.preview() == Some(FoldId::Source(id)) {
            self.preview = None;
        }
        self.version += 1;
        self.expire_from(document, candidate.hidden.start)?;
        Ok(Some(FoldChange {
            version: self.version,
            source: candidate.hidden,
        }))
    }

    /// Register a fixed, nonempty body row interval, initially expanded. Exact
    /// reinsertion preserves its choice. Overlap (including nesting) with any
    /// registered row range in this view is rejected, even while expanded/previewed.
    /// Crossing a source registration is also rejected; containment is allowed.
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
        for offset in [candidate.source.start, candidate.source.end - 1] {
            if let Some(key) =
                self.state
                    .strict_source_ranges
                    .next(Bound::Unbounded, offset, offset + 1)
                && let FoldId::Source(manual_id) = key.3
                && let Some(manual) = self.state.source_candidate(document, manual_id)
            {
                // A live root tail also grows with an incomplete row. Only
                // this slot can contain future growth past the current EOF;
                // fixed ranges and paragraph overlays keep their validator.
                let live_contains = matches!(manual.id, FoldId::Source(id) if self.state.open_source_tails.contains(&id) || self.state.source_candidates[&id].flexible)
                    && manual.source.start <= candidate.source.start
                    && candidate.source.end <= manual.source.end;
                if !live_contains {
                    crate::source_folds::validate_pair(manual.source, candidate)?;
                }
            }
        }
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

    /// Registered fixed row range containing this byte, including an expanded
    /// range. O(log R) metadata only; a UI can select the exact ID for removal.
    pub fn table_row_fold_at(
        &self,
        document: &Document,
        source: usize,
    ) -> Result<Option<FoldCandidate>, LayoutError> {
        self.check(document)?;
        Ok(self.row_candidate_at(document, source))
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
    /// Root section/code/table/row refreshes, completed EOF row/source boundaries,
    /// shared tail checks and event-
    /// driven scoped-section/container refreshes; independent of body size and
    /// still-open depth. Title growth refreshes at most one scoped heading.
    pub fn range_visits(&self) -> u64 {
        self.range_visits
    }
    /// Container topology index mutation and equal-start Source/syntax owner
    /// resolution visits. Union visibility uses the augmented interval index.
    pub fn container_index_visits(&self) -> u64 {
        self.state.containers.visits()
    }
    /// Retained sparse index nodes, bounded by O(container_count), not toggles.
    pub fn container_index_nodes(&self) -> usize {
        self.state.containers.node_count()
    }
    /// Scoped-section topology mutation/owner-resolution visits; body growth
    /// writes none.
    pub fn section_index_visits(&self) -> u64 {
        self.state.sections.visits()
    }
    /// Retained sparse nodes, bounded by O(heading_count), not toggles.
    pub fn section_index_nodes(&self) -> usize {
        self.state.sections.node_count()
    }
    /// Actual AVL nodes visited by all selected-interval mutations and queries.
    /// Rotations add only constant node work per visited path element.
    pub fn fold_index_visits(&self) -> u64 {
        self.state.hidden_ranges.visits()
    }
    /// One owned AVL node per selected interval; removed nodes are freed.
    pub fn fold_index_nodes(&self) -> usize {
        self.state.hidden_ranges.node_count()
    }
    /// All source envelope/strict-validation/parent lookup AVL visits.
    pub fn source_index_visits(&self) -> u64 {
        self.state.source_ranges.visits() + self.state.strict_source_ranges.visits()
    }
    /// Owned envelope nodes, including a second entry for each strict range.
    pub fn source_index_nodes(&self) -> usize {
        self.state.source_ranges.node_count() + self.state.strict_source_ranges.node_count()
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
                FoldId::Math(id) => document
                    .block(id)
                    .is_some_and(|block| matches!(block.kind, BlockKind::Math { .. })),
                FoldId::Table(id) => document.table(id).is_some(),
                FoldId::Container(id) => document.container(id).is_some(),
                FoldId::TableRows(range) => self.has_table_rows(document, range),
                FoldId::Source(id) => self.state.source_candidates.contains_key(&id),
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
            let candidate = self
                .candidate(document, id)
                .expect("stable folding identity");
            source = Some(candidate.hidden);
            if collapsed {
                self.state.insert(document, &mut self.heights, candidate);
            }
        }
        if let Some(id) = target {
            let candidate = self.candidate(document, id).unwrap();
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

    /// Metadata-only admission check shared with renderer-owned line policies.
    pub fn paragraph_auto_excluded(
        &self,
        document: &Document,
        block: BlockId,
    ) -> Result<bool, LayoutError> {
        self.check(document)?;
        Ok(self.paragraph_auto_excluded.contains(&block))
    }

    /// Opt a logical paragraph out of (or back into) automatic registration,
    /// even before its length reaches the threshold. Existing registrations and
    /// their choices are unchanged. O(log E), one entry per excluded BlockId;
    /// survives reset/default changes and never transfers to another paragraph.
    /// Manual registration remains available. Changes invalidate view tasks.
    pub fn set_paragraph_auto_excluded(
        &mut self,
        document: &Document,
        block: BlockId,
        excluded: bool,
    ) -> Result<(), LayoutError> {
        self.check(document)?;
        if document.open_paragraph_source(block).is_none() {
            return Err(LayoutError::InvalidRange);
        }
        let changed = if excluded {
            self.paragraph_auto_excluded.insert(block)
        } else {
            self.paragraph_auto_excluded.remove(&block)
        };
        if changed {
            self.version += 1;
        }
        Ok(())
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
    /// use FoldBatch to change them explicitly. Paragraph policy is consumed by
    /// ParagraphAutoFold, not apply. Invalidates pending view tasks.
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
        let candidate = self
            .candidate(document, id)
            .ok_or(LayoutError::InvalidRange)?;
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

    fn refresh_paragraph(&mut self, document: &Document, dirty: &mut Option<usize>) {
        let Some(id) = self.state.open_paragraph else {
            return;
        };
        self.range_visits += 1;
        let entry = self.state.source_candidates[&id];
        let old = entry.candidate;
        let next = document.open_paragraph_source(entry.paragraph.unwrap());
        if let Some((source, open)) = next
            && (source.end > old.hidden.start || entry.keep_empty && source.end == old.hidden.start)
        {
            let new = FoldCandidate {
                source,
                hidden: Span::new(old.hidden.start, source.end),
                open,
                ..old
            };
            self.state
                .source_ranges
                .remove(FoldState::source_key(entry));
            self.state
                .strict_source_ranges
                .remove(FoldState::source_key(entry));
            self.state.source_candidates.get_mut(&id).unwrap().candidate = new;
            self.state
                .source_ranges
                .insert(FoldState::source_key(self.state.source_candidates[&id]));
            if !entry.flexible {
                self.state
                    .strict_source_ranges
                    .insert(FoldState::source_key(self.state.source_candidates[&id]));
            }
            if !open {
                self.state.open_paragraph = None;
            }
            if self.state.collapsed.contains_key(&old.id) {
                // Refresh even for an unchanged endpoint: a formerly EOF tile
                // may now include visible bytes following the closed paragraph.
                self.state.refresh(document, &mut self.heights, new);
                *dirty = Some(dirty.unwrap_or(usize::MAX).min(old.hidden.start));
            }
        } else {
            self.state.remove(document, &mut self.heights, old.id);
            self.state.source_candidates.remove(&id);
            self.state
                .source_ranges
                .remove(FoldState::source_key(entry));
            self.state
                .strict_source_ranges
                .remove(FoldState::source_key(entry));
            self.state.open_paragraph = None;
            if self.preview() == Some(old.id) {
                self.preview = None;
            }
            *dirty = Some(dirty.unwrap_or(usize::MAX).min(old.hidden.start));
        }
    }

    fn refresh_source_tail(&mut self, _document: &Document, dirty: &mut Option<usize>) {
        if !self.state.open_source_tails.is_empty() {
            // Endpoints are shared sentinels, resolved against this document.
            // No registration, selected mask or ancestor is copied/refreshed.
            self.range_visits += 1;
            if let Some(&(start, _)) = self.state.open_ranges.first() {
                *dirty = Some(dirty.unwrap_or(usize::MAX).min(start));
            }
        }
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
        let mut dirty = None;
        self.refresh_paragraph(document, &mut dirty);
        self.refresh_source_tail(document, &mut dirty);
        // Six root sections, one code/math/table, and at most one disjoint range
        // containing the sole incomplete table row. No historical range scan.
        debug_assert!(self.state.growing.len() <= 8);
        let mut growing = [None; 8];
        for (slot, &id) in growing.iter_mut().zip(&self.state.growing) {
            *slot = Some(id);
        }
        if let Some(&(start, _)) = self.state.open_ranges.first() {
            dirty = Some(dirty.unwrap_or(usize::MAX).min(start));
            if self.state.open_source_tails.is_empty() {
                self.range_visits += 1; // One shared tail, independent of open depth.
            }
        }
        if let Some(group) = self
            .state
            .source_boundaries
            .get_mut(&self.state.source_boundary_generation)
        {
            self.range_visits += 1;
            if let Some(tile) = group.tile {
                let span = document.tile(TileId(tile)).unwrap();
                if span.end > group.end {
                    self.heights.cover(tile..tile + 1, false);
                    group.tile = None;
                }
                dirty = Some(dirty.unwrap_or(usize::MAX).min(span.start));
            }
        }
        self.state.source_boundary_generation += 1;
        if let Some(id) = self.state.fixed_boundary.take() {
            self.range_visits += 1;
            let candidate = document.fold_candidate(id).unwrap();
            // Tile membership can change while the fixed endpoint does not.
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
        if self.defaults.code || self.defaults.math {
            for index in update.new_blocks.clone() {
                self.default_visits += 1;
                let id = match document.blocks()[index].kind {
                    BlockKind::Code { .. } if self.defaults.code => FoldId::Code(BlockId(index)),
                    BlockKind::Math { .. } if self.defaults.math => FoldId::Math(BlockId(index)),
                    _ => continue,
                };
                if let Some(candidate) = document.fold_candidate(id) {
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
