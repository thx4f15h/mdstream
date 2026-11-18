use std::fmt;

use mdtui3::{
    Anchor, BlockId, Budget, Document, DocumentId, Heading, HeightIndex, LayoutError, Navigator,
    ScrollTarget, TILE_BYTES, TileId,
};

use crate::render::checkpoint::{RecoveryPlan, RenderCheckpointStore};
use crate::render::first_frame::{FirstFrame, FirstFrameConfig, FirstFrameError, FirstFrameStatus};
use crate::render::viewport::{block_start_tile, expand_block_end};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefineConfig {
    pub width: u16,
    pub max_tiles: usize,
    pub max_replay_bytes: usize,
    pub row_slack: u64,
}

impl Default for RefineConfig {
    fn default() -> Self {
        Self {
            width: 80,
            max_tiles: 64,
            max_replay_bytes: 256 * 1024,
            row_slack: 64,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefineStatus {
    Exact,
    ReplayFrom { from: TileId, target: TileId },
    Approximate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefinedTarget {
    pub source: usize,
    pub line_start: usize,
    pub row: u128,
    pub anchor: Anchor,
    pub status: RefineStatus,
    pub plan_start: TileId,
    pub plan_end: TileId,
    pub rendered_tiles: usize,
}

impl RefinedTarget {
    /// Apply a successful refinement through the core Navigator contract.
    /// Callers must not call this for `BudgetExhausted`, `PendingScan` or
    /// `NotFound`; those results carry no safe anchor.
    pub fn apply_to_navigator(
        self,
        navigator: &mut Navigator,
        heights: &HeightIndex,
        viewport_rows: u64,
    ) -> ScrollTarget {
        navigator.reconcile_source(heights, self.row, viewport_rows, self.source)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefineProgress {
    /// The bounded display scan is still incomplete.
    PendingScan,
    /// The caller's shared budget was exhausted; call `advance` again with the
    /// same document revision and stores.
    BudgetExhausted,
    Ready(RefinedTarget),
    /// The target block has no rendered line (for example a definition block).
    NotFound,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefineError {
    InvalidConfig,
    InvalidSource,
    EmptyDocument,
    Stale,
    FirstFrame(FirstFrameError),
    Layout(LayoutError),
    Inconsistent(&'static str),
}

impl fmt::Display for RefineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig => formatter.write_str("invalid source-refinement configuration"),
            Self::InvalidSource => formatter.write_str("source refinement target is invalid"),
            Self::EmptyDocument => {
                formatter.write_str("source refinement needs a non-empty document")
            }
            Self::Stale => formatter.write_str("source refinement revision is stale"),
            Self::FirstFrame(error) => error.fmt(formatter),
            Self::Layout(error) => error.fmt(formatter),
            Self::Inconsistent(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for RefineError {}

impl From<FirstFrameError> for RefineError {
    fn from(error: FirstFrameError) -> Self {
        Self::FirstFrame(error)
    }
}

impl From<LayoutError> for RefineError {
    fn from(error: LayoutError) -> Self {
        Self::Layout(error)
    }
}

/// Resumable source-to-row refinement. Construction only plans a bounded tile
/// window around the target block. `advance` reuses P28/P31 layout and can pause
/// at any shared-budget boundary; it never lays out the whole document.
pub struct SourceRefiner {
    document: DocumentId,
    revision: u64,
    source: usize,
    lookup_source: usize,
    block: Option<BlockId>,
    block_source: mdtui3::Span,
    config: RefineConfig,
    status: RefineStatus,
    first: FirstFrame,
    result: Option<Option<RefinedTarget>>,
    finish_line: usize,
    finish_cell: usize,
    finish_started: bool,
    finish_candidate: Option<(usize, crate::screen::LineSource)>,
    finish_first: Option<(usize, crate::screen::LineSource)>,
    finish_content: Option<(usize, crate::screen::LineSource)>,
    zero_progress: super::first_frame::ZeroRangeProgress,
    normalized_source: Option<usize>,
}

impl SourceRefiner {
    pub fn set_links(&mut self, links: crate::render::objects::SharedLinks) {
        self.first.set_links(links);
    }

    pub(crate) fn set_table_navigation(
        &mut self,
        document: &Document,
        policy: crate::render::table_widths::TableWidthPolicy,
        positions: &[(mdtui3::TableId, usize)],
        rows: usize,
    ) -> Result<(), FirstFrameError> {
        self.first.set_table_options(document, policy, positions)?;
        if let Some(tile) = document.tile_at(self.lookup_source) {
            self.first
                .set_table_view(crate::render::table_window::TableView {
                    anchor: Anchor { tile, offset: 0 },
                    source: Some(self.lookup_source),
                    rows,
                    tail: None,
                });
        }
        Ok(())
    }

    pub fn new(
        document: &Document,
        heights: &HeightIndex,
        checkpoints: &RenderCheckpointStore,
        source: usize,
        config: RefineConfig,
    ) -> Result<Self, RefineError> {
        if config.width == 0 || config.max_tiles == 0 {
            return Err(RefineError::InvalidConfig);
        }
        if heights.len() != document.tile_count() {
            return Err(RefineError::Inconsistent(
                "height index does not match the document",
            ));
        }
        if source > document.source().len() || !document.source().is_char_boundary(source) {
            return Err(RefineError::InvalidSource);
        }
        if document.tile_count() == 0 || document.source().is_empty() {
            return Err(RefineError::EmptyDocument);
        }
        let lookup_source = source.min(document.source().len() - 1);
        let heading = heading_source_for(document, lookup_source);
        // Heading markers can end one tile before the title starts. Reserve
        // the bounded window for content, so recovery overscan cannot spend
        // every tile on the hidden prefix. Keep `source` as the navigation key.
        let target_source = heading.map_or(lookup_source, |heading| {
            lookup_source.clamp(
                heading.content.start,
                heading
                    .content
                    .end
                    .saturating_sub(1)
                    .max(heading.content.start),
            )
        });
        let target_tile = document
            .tile_at(target_source.min(document.source().len() - 1))
            .ok_or(RefineError::InvalidSource)?;
        let pending = document.pending();
        let in_pending = pending.start <= lookup_source && lookup_source < pending.end;
        let block = heading
            .map(|heading| heading.block)
            .or_else(|| document.block_at(lookup_source))
            .or_else(|| {
                (!in_pending)
                    .then(|| {
                        document
                            .tile(target_tile)
                            .and_then(|tile| document.block_at(tile.start))
                    })
                    .flatten()
            });
        let paragraph = document.paragraph_at(lookup_source);
        let block = paragraph.map(|p| p.block).or(block);
        let block_source = heading
            .map(|heading| heading.source)
            .or_else(|| paragraph.map(|p| p.source))
            .or_else(|| block.and_then(|id| document.block(id)).map(|b| b.source))
            .or_else(|| in_pending.then_some(pending))
            .ok_or(RefineError::InvalidSource)?;

        let (mut status, recovery_start) = match checkpoints.plan_recovery(target_tile) {
            RecoveryPlan::Exact(tile) => {
                let line_start = checkpoints
                    .exact(tile)
                    .and_then(|checkpoint| checkpoint.line_source_start);
                (
                    RefineStatus::Exact,
                    line_start
                        .and_then(|source| document.tile_at(source))
                        .unwrap_or(tile),
                )
            }
            RecoveryPlan::ReplayFrom { from, target } => {
                (RefineStatus::ReplayFrom { from, target }, from)
            }
            RecoveryPlan::ApproximateLineStart(_) => (RefineStatus::Approximate, target_tile),
        };

        let block_start = document.tile_at(block_source.start).unwrap_or(target_tile);
        let desired_start = recovery_start
            .min(block_start)
            .min(block_start_tile(document, target_tile));
        let earliest = target_tile
            .0
            .saturating_sub(config.max_tiles.saturating_sub(1));
        // A measured zero-height suffix can share a row with a distant PH.
        // Reuse the same view's checkpoint source instead of replaying the raw
        // hidden gap. The projection still limits actual visible tiles.
        let sparse_start = (checkpoints.view().has_collapsed()
            && heights.height(target_tile) == Some(0))
        .then(|| {
            checkpoints
                .previous(target_tile)
                .and_then(|(_, cp)| cp.line_source_start)
                .and_then(|source| document.tile_at(source))
        })
        .flatten()
        .filter(|start| *start >= block_start && start.0 < earliest)
        .filter(|start| {
            heights
                .prefix_rows(target_tile.0)
                .zip(heights.prefix_rows(start.0))
                .is_some_and(|(end, begin)| {
                    end.saturating_sub(begin) <= u128::from(config.row_slack)
                })
        });
        let start = sparse_start.unwrap_or(TileId(desired_start.0.max(earliest)));
        if start > desired_start {
            status = RefineStatus::Approximate;
        }
        let limit = if sparse_start.is_some() {
            document.tile_count()
        } else {
            start
                .0
                .saturating_add(config.max_tiles)
                .min(document.tile_count())
        };
        let mut end = target_tile.0.saturating_add(1).min(document.tile_count());
        expand_block_end(document, start, &mut end, limit);
        if let Some(heading) = heading
            && let Some(tile) = document.tile_at(heading.source.end.saturating_sub(1))
        {
            end = end.max(tile.0.saturating_add(1).min(limit));
        }
        let planned_tiles = if sparse_start.is_some() {
            config.max_tiles
        } else {
            end.saturating_sub(start.0)
        };
        let row_limit = u64::try_from(planned_tiles)
            .unwrap_or(u64::MAX)
            .saturating_mul(TILE_BYTES as u64 + 4)
            .saturating_add(config.row_slack)
            .max(1);
        let mut first = FirstFrame::new_for_view(
            document,
            FirstFrameConfig {
                top_tile: start,
                tile_limit: planned_tiles.max(1),
                width: config.width,
                row_limit,
            },
            checkpoints,
        )?;
        first.set_row_limit_anchor(target_tile);

        if document.math_object_stats().active > 0 {
            // An intrinsic source locator can query the target row directly;
            // refinement never needs a full tall-object bitmap.
            first.set_table_view(crate::render::table_window::TableView {
                anchor: Anchor {
                    tile: target_tile,
                    offset: 0,
                },
                source: Some(lookup_source),
                rows: 1,
                tail: None,
            });
        }

        if let RefineStatus::ReplayFrom { from, target } = status {
            let bytes = document
                .tile(target)
                .zip(document.tile(from))
                .map(|(target, from)| target.start.saturating_sub(from.start));
            let replay_ready = from >= start
                && bytes.is_some_and(|bytes| bytes <= config.max_replay_bytes)
                && checkpoints.exact(from).is_some_and(|checkpoint| {
                    first
                        .begin_replay(from, target, *checkpoint, config.max_replay_bytes)
                        .is_ok()
                });
            if !replay_ready {
                status = RefineStatus::Approximate;
            }
        }

        Ok(Self {
            document: document.id(),
            revision: document.revision(),
            source,
            lookup_source,
            block,
            block_source,
            config,
            status,
            first,
            result: None,
            finish_line: 0,
            finish_cell: 0,
            finish_started: false,
            finish_candidate: None,
            finish_first: None,
            finish_content: None,
            zero_progress: Default::default(),
            normalized_source: None,
        })
    }

    #[must_use]
    pub fn matches(&self, document: &Document) -> bool {
        self.document == document.id() && self.revision == document.revision()
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub const fn source(&self) -> usize {
        self.source
    }

    #[must_use]
    pub const fn config(&self) -> RefineConfig {
        self.config
    }

    pub fn advance(
        &mut self,
        document: &Document,
        heights: &mut HeightIndex,
        checkpoints: &mut RenderCheckpointStore,
        budget: &mut Budget,
    ) -> Result<RefineProgress, RefineError> {
        let result = self.advance_owned(document, checkpoints, budget);
        // Compatibility for callers maintaining a separate test/client index.
        // The production viewport passes only the owned view via advance_owned.
        if matches!(result, Ok(RefineProgress::Ready(_))) {
            for segment in &self.first.plan().segments {
                if let Some(ticket) = heights.ticket(segment.tile)
                    && let Some(rows) = checkpoints.heights().height(segment.tile)
                {
                    if checkpoints.heights().height_state(segment.tile)
                        == Some(mdtui3::HeightState::Local)
                    {
                        heights.measure_local(ticket, rows)?;
                    } else {
                        heights.measure(ticket, rows)?;
                    }
                }
            }
        }
        result
    }

    pub fn advance_owned(
        &mut self,
        document: &Document,
        checkpoints: &mut RenderCheckpointStore,
        budget: &mut Budget,
    ) -> Result<RefineProgress, RefineError> {
        if !self.matches(document) {
            return Err(RefineError::Stale);
        }
        self.first.check_owner(document, checkpoints)?;
        if let Some(result) = &self.result {
            return Ok(result.as_ref().map_or(RefineProgress::NotFound, |target| {
                RefineProgress::Ready(*target)
            }));
        }
        let step = self.first.advance_using(document, checkpoints, budget)?;
        match step.status {
            FirstFrameStatus::PendingScan => Ok(RefineProgress::PendingScan),
            FirstFrameStatus::BudgetExhausted => Ok(RefineProgress::BudgetExhausted),
            FirstFrameStatus::Covered | FirstFrameStatus::EndOfDocument => {
                self.finish(checkpoints.heights_mut(document)?, budget)
            }
        }
    }

    fn finish(
        &mut self,
        heights: &mut HeightIndex,
        budget: &mut Budget,
    ) -> Result<RefineProgress, RefineError> {
        if matches!(self.status, RefineStatus::ReplayFrom { .. }) && !self.first.replay_used() {
            self.status = RefineStatus::Approximate;
        }
        let sink = self.first.sink().ok_or(RefineError::Inconsistent(
            "source refinement sink is missing",
        ))?;
        let composite_math = self.first.scanner().intersects_math(self.block_source);
        let lines = self.first.lines();
        while let Some(line) = lines.get(self.finish_line) {
            if !self.finish_started {
                if !budget.charge(mdtui3::WorkLimit {
                    queries: 1,
                    objects: 1,
                    ..Default::default()
                }) {
                    return Ok(RefineProgress::BudgetExhausted);
                }
                self.finish_started = true;
                if let Some(source) = line.source.filter(|source| source.block == self.block) {
                    self.finish_first.get_or_insert((self.finish_line, source));
                    if source.source.start <= self.lookup_source {
                        self.finish_candidate = Some((self.finish_line, source));
                    }
                }
            }
            if let Some(source) = line.source.filter(|source| source.block == self.block) {
                while let Some(cell) = line.cells.get(self.finish_cell) {
                    if !budget.charge(mdtui3::WorkLimit {
                        queries: 1,
                        objects: 1,
                        ..Default::default()
                    }) {
                        return Ok(RefineProgress::BudgetExhausted);
                    }
                    self.finish_cell += 1;
                    if let Some(mapped) = cell.source.filter(|mapped| !mapped.is_empty()) {
                        if source.source.start <= self.lookup_source
                            && self.lookup_source < source.source.end
                        {
                            self.finish_content
                                .get_or_insert((self.finish_line, source));
                        }
                        if mapped.start <= self.lookup_source && self.lookup_source < mapped.end {
                            if cell.text.is_indexed() || composite_math {
                                self.normalized_source = Some(mapped.start);
                            }
                            self.finish_candidate = Some((self.finish_line, source));
                            self.finish_content = self.finish_candidate;
                            self.finish_line = lines.len();
                            break;
                        }
                    }
                }
            }
            if self.finish_line == lines.len() {
                break;
            }
            self.finish_line += 1;
            self.finish_cell = 0;
            self.finish_started = false;
        }
        // Real mapped cells take precedence over table borders sharing a row's
        // source envelope. A clipped target falls back to that row's content.
        let candidate = self
            .finish_content
            .or(self.finish_candidate)
            .or(self.finish_first);
        let Some((line_index, line_source)) = candidate else {
            self.result = Some(None);
            return Ok(RefineProgress::NotFound);
        };
        if !super::first_frame::measure_zero_ranges(
            self.first.zero_ranges(),
            heights,
            &mut self.zero_progress,
            budget,
        )? {
            return Ok(RefineProgress::BudgetExhausted);
        }
        let render_start = match (self.status, self.first.replay_used()) {
            (RefineStatus::ReplayFrom { from, .. }, true) => from,
            _ => self.first.plan().tile_start,
        };
        let Some(row) = row_for_line(&self.first, sink, heights, render_start, line_index) else {
            return Err(RefineError::Inconsistent(
                "rendered line could not be mapped to a row",
            ));
        };
        let Some(anchor) = heights.locate(row) else {
            return Err(RefineError::Inconsistent(
                "refined row is outside the height index",
            ));
        };
        if heights.row_of(anchor) != Some(row) {
            return Err(RefineError::Inconsistent(
                "refined anchor does not round-trip through HeightIndex",
            ));
        }
        let target = RefinedTarget {
            source: self.normalized_source.unwrap_or(self.source),
            // A math row can project nonmonotone sources and be horizontally
            // clipped. Retain the whole source atom as its presentation anchor.
            line_start: if composite_math {
                self.normalized_source.unwrap_or(self.lookup_source)
            } else {
                line_source.source.start
            },
            row,
            anchor,
            status: self.status,
            plan_start: self.first.plan().tile_start,
            plan_end: self.first.plan().tile_end,
            rendered_tiles: self.first.processed_tiles(),
        };
        self.result = Some(Some(target));
        Ok(RefineProgress::Ready(target))
    }
}

fn heading_source_for(document: &Document, source: usize) -> Option<Heading> {
    let entries = document.headings().entries();
    let index = entries
        .partition_point(|heading| heading.source.start <= source)
        .checked_sub(1)?;
    let heading = *entries.get(index)?;
    (source < heading.source.end).then_some(heading)
}

fn row_for_line(
    first: &FirstFrame,
    sink: &crate::render::sink::TileLayoutSink,
    heights: &HeightIndex,
    render_start: TileId,
    line_index: usize,
) -> Option<u128> {
    let plan = first.plan();
    let start = plan
        .segments
        .partition_point(|segment| segment.tile < render_start);
    let mut remaining = sink.line_offset(line_index)?;
    for segment in plan.segments.get(start..)? {
        let rows = sink.tile_rows(segment.tile)?;
        if remaining < rows {
            return heights
                .prefix_rows(segment.tile.0)?
                .checked_add(remaining as u128);
        }
        remaining -= rows;
    }
    None
}
