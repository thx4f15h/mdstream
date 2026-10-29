use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
};

use crate::{
    BlockId, BlockKind, Budget, CheckpointIndex, Document, DocumentId, HeadingId, HeightIndex,
    LayoutError, Measurement, Span, SyntaxState, TileId, Update, WorkLimit,
};

/// Document-local stable identity. Containers and arbitrary crossing ranges are
/// deliberately outside this first folding dialect.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FoldId {
    Heading(HeadingId),
    Code(BlockId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FoldCandidate {
    pub id: FoldId,
    pub parent: Option<FoldId>,
    /// Whole section or code block, including the retained heading.
    pub source: Span,
    /// Body replaced by a placeholder. Empty bodies produce no placeholder.
    pub hidden: Span,
    /// Heading title or fence info; this is a source reference, not copied text.
    pub summary: Option<Span>,
    pub open: bool,
}

impl FoldCandidate {
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
pub enum FoldLookup {
    Visible,
    Hidden(FoldCandidate),
    BudgetExhausted,
}

/// Bounded discovery of candidates whose opener block intersects a source
/// range. Parent sections starting before the range are queried with
/// `fold_candidate_at` / `FoldCandidate::parent`, not duplicated in every page.
pub struct FoldCandidates {
    document: DocumentId,
    revision: u64,
    next: usize,
    end: usize,
}

impl FoldCandidates {
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
        while self.next < self.end {
            // Non-candidate blocks also consume work; discovery cannot scan an
            // arbitrarily large paragraph/list arena in one frame.
            if !budget.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            }) {
                return Ok(crate::Completion::BudgetExhausted);
            }
            let block = &document.blocks()[self.next];
            let id = match block.kind {
                BlockKind::Code { .. } => Some(FoldId::Code(BlockId(self.next))),
                BlockKind::Heading { .. } => document
                    .headings()
                    .current(block.source.start)
                    .map(FoldId::Heading),
                _ => None,
            };
            if let Some(candidate) = id.and_then(|id| document.fold_candidate(id)) {
                out.push(candidate);
            }
            self.next += 1;
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
    // Only the six still-open heading sections and the active code block can
    // change existing ranges on append. Closed historical folds are untouched.
    growing: BTreeSet<FoldId>,
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
        })
    }

    /// O(log headings), with at most six levels; never reads section contents.
    pub fn fold_candidate(&self, id: FoldId) -> Option<FoldCandidate> {
        match id {
            FoldId::Heading(id) => {
                let headings = self.headings();
                let heading = headings.get(id)?;
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
                    parent: self.headings().current(source.start).map(FoldId::Heading),
                    source,
                    hidden: source,
                    summary: info,
                    open,
                })
            }
        }
    }

    /// Innermost candidate at a source position; pending text uses its section.
    pub fn fold_candidate_at(&self, offset: usize) -> Option<FoldCandidate> {
        if offset >= self.source().len() {
            return None;
        }
        if let Some(block) = self.block_at(offset)
            && let Some(candidate) = self.fold_candidate(FoldId::Code(block))
        {
            return Some(candidate);
        }
        // The pending tail may be outside the last block's committed range.
        if let Some(index) = self.blocks().len().checked_sub(1)
            && let Some(candidate) = self.fold_candidate(FoldId::Code(BlockId(index)))
            && candidate.open
            && candidate.source.range().contains(&offset)
        {
            return Some(candidate);
        }
        self.fold_candidate(FoldId::Heading(self.headings().current(offset)?))
    }
}

impl FoldState {
    pub(crate) fn segment(&self, document: &Document, offset: usize) -> FoldSegment {
        let mut hidden = None;
        let mut heading = document.headings().current(offset);
        while let Some(id) = heading {
            if let Some(mask) = self.collapsed.get(&FoldId::Heading(id))
                && mask.candidate.hidden.range().contains(&offset)
            {
                hidden = Some(mask.candidate);
            }
            heading = document.headings().parent(id);
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

    fn remove(&mut self, heights: &mut HeightIndex, id: FoldId) -> Option<Mask> {
        let mask = self.collapsed.remove(&id)?;
        heights.cover(mask.tiles.clone(), false);
        self.starts.remove(&(mask.candidate.hidden.start, id));
        self.growing.remove(&id);
        Some(mask)
    }

    fn insert(&mut self, document: &Document, heights: &mut HeightIndex, candidate: FoldCandidate) {
        let hidden = candidate.hidden;
        let tiles = Self::interior(document, hidden);
        if !hidden.is_empty() {
            self.starts.insert((hidden.start, candidate.id));
        }
        heights.cover(tiles.clone(), true);
        if candidate.open {
            self.growing.insert(candidate.id);
        }
        self.collapsed
            .insert(candidate.id, Mask { candidate, tiles });
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
    /// Existing open ranges inspected during updates, independent of body size.
    pub fn range_visits(&self) -> u64 {
        self.range_visits
    }
    pub fn is_collapsed(&self, id: FoldId) -> bool {
        self.state.collapsed.contains_key(&id)
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
        if collapsed {
            self.state.insert(document, &mut self.heights, candidate);
        } else {
            self.state.remove(&mut self.heights, id);
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
        let mut dirty = None::<usize>;
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
