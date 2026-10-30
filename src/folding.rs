use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
};

use crate::{
    BlockId, BlockKind, Budget, CheckpointIndex, Completion, ContainerId, Document, DocumentId,
    HeadingId, HeightIndex, LayoutError, Measurement, Span, SyntaxState, TableId, TileId, Update,
    WorkLimit,
};

/// Document-local stable identity. Arbitrary crossing ranges are unsupported.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FoldId {
    Heading(HeadingId),
    Code(BlockId),
    Table(TableId),
    Container(ContainerId),
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FoldCandidate {
    pub id: FoldId,
    pub parent: Option<FoldId>,
    /// Whole section, code block, table or container, including the retained heading.
    pub source: Span,
    /// Body replaced by a placeholder. Empty bodies produce no placeholder.
    pub hidden: Span,
    /// Title, fence info, table header or container range; borrowed source metadata.
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
    // At most six root sections and the active code/table block need individual
    // range refreshes. Containers use the separate shared tail below.
    growing: BTreeSet<FoldId>,
    containers: crate::container_folds::ContainerFolds,
    // All open containers form a single ancestor chain. Their complete-tile
    // union has one shared mask; no ancestor is refreshed on ordinary append.
    open_containers: BTreeSet<(usize, ContainerId)>,
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
                    parent: container.parent.map(FoldId::Container).or_else(|| {
                        self.headings()
                            .root_current(source.start)
                            .map(FoldId::Heading)
                    }),
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
                // Container heading folds need a separate bounded update model.
                if heading.container.is_some() {
                    return None;
                }
                let end = headings.section_end(id);
                let source = headings.section(id, self.source().len())?;
                Some(FoldCandidate {
                    id: FoldId::Heading(id),
                    parent: headings.parent(id).map(FoldId::Heading),
                    source,
                    hidden: Span::new(heading.source.end, source.end),
                    summary: Some(heading.content),
                    open: end.is_none() && !self.is_finished(),
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
                    parent: block.container.map(FoldId::Container).or_else(|| {
                        self.headings()
                            .root_current(source.start)
                            .map(FoldId::Heading)
                    }),
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
                    parent: block.container.map(FoldId::Container).or_else(|| {
                        self.headings()
                            .root_current(source.start)
                            .map(FoldId::Heading)
                    }),
                    source,
                    hidden: source,
                    summary: Some(table.row(0)?.source),
                    open,
                })
            }
        }
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
            hidden = document.fold_candidate(FoldId::Container(id));
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
        if let FoldId::Container(container) = id {
            self.containers.set(container, 0);
            self.open_containers
                .remove(&(mask.candidate.hidden.start, container));
            self.refresh_open_tiles(document, heights);
        }
        Some(mask)
    }

    fn insert(&mut self, document: &Document, heights: &mut HeightIndex, candidate: FoldCandidate) {
        let hidden = candidate.hidden;
        let open_container = matches!(candidate.id, FoldId::Container(_)) && candidate.open;
        let tiles = if open_container {
            0..0
        } else {
            Self::interior(document, hidden)
        };
        if let FoldId::Container(id) = candidate.id {
            self.containers.set(
                id,
                if candidate.open {
                    usize::MAX
                } else {
                    hidden.end
                },
            );
            if candidate.open {
                self.open_containers.insert((hidden.start, id));
                self.refresh_open_tiles(document, heights);
            }
        }
        if !hidden.is_empty() {
            self.starts.insert((hidden.start, candidate.id));
        }
        heights.cover(tiles.clone(), true);
        if candidate.open && !open_container {
            self.growing.insert(candidate.id);
        }
        self.collapsed
            .insert(candidate.id, Mask { candidate, tiles });
    }

    fn refresh_open_tiles(&mut self, document: &Document, heights: &mut HeightIndex) {
        let tiles = self.open_containers.first().map_or(0..0, |&(start, _)| {
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
        let mask = self.collapsed.get_mut(&candidate.id).unwrap();
        let tiles = Self::interior(document, candidate.hidden);
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
        if let FoldId::Container(id) = candidate.id {
            self.containers.set(id, candidate.hidden.end);
            self.open_containers.remove(&(candidate.hidden.start, id));
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
    /// Existing section/code/table refreshes, shared container-tail checks and
    /// collapsed container close events, independent of body size and open depth.
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
    /// fixed index operations, O(log H + log C + log F + log T), with O(1) extra storage.
    /// No source or covered tiles are scanned. An identical request is free.
    /// Rendering the newly visible window still needs its own shared budget.
    pub fn set_preview(
        &mut self,
        document: &Document,
        target: Option<FoldId>,
        budget: &mut Budget,
    ) -> Result<FoldPreviewProgress, LayoutError> {
        self.check(document)?;
        // Validate IDs in O(1), before charging or changing the old preview.
        if let Some(id) = target {
            let valid = match id {
                FoldId::Heading(id) => document
                    .headings()
                    .get(id)
                    .is_some_and(|heading| heading.container.is_none()),
                FoldId::Code(id) => document
                    .block(id)
                    .is_some_and(|block| matches!(block.kind, BlockKind::Code { .. })),
                FoldId::Table(id) => document.table(id).is_some(),
                FoldId::Container(id) => document.container(id).is_some(),
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
        debug_assert!(self.state.growing.len() <= 7);
        let mut growing = [None; 7];
        for (slot, &id) in growing.iter_mut().zip(&self.state.growing) {
            *slot = Some(id);
        }
        let mut dirty = self.state.open_containers.first().map(|&(start, _)| start);
        if dirty.is_some() {
            self.range_visits += 1; // One shared tail, independent of open depth.
        }
        for &id in &update.closed_containers {
            if self.state.collapsed.contains_key(&FoldId::Container(id)) {
                self.range_visits += 1;
                let new = document.fold_candidate(FoldId::Container(id)).unwrap();
                self.state.refresh(document, &mut self.heights, new);
                dirty = Some(dirty.unwrap_or(usize::MAX).min(new.hidden.start));
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
                if heading.container.is_none()
                    && self.defaults.heading_levels[usize::from(heading.level - 1)]
                {
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
        if let Some(depth) = self.defaults.container_depth {
            // Depth is stored on each node: neither historical containers nor
            // the active ancestor chain need visiting, even for a merged update.
            for index in update.new_containers.clone() {
                self.default_visits += 1;
                let id = ContainerId(index);
                if document.container(id).unwrap().depth >= depth {
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
