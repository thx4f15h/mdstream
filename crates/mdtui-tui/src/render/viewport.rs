use std::collections::HashMap;
use std::fmt;

use mdtui3::{
    Anchor, Budget, Document, DocumentId, HeightIndex, LayoutError, Span, TILE_BYTES, TableId,
    TileId, Update, WorkLimit,
};

use crate::render::checkpoint::{
    CheckpointMaintenanceProgress, RecoveryPlan, RenderCheckpointStore,
};
use crate::render::first_frame::{
    FirstFrame, FirstFrameConfig, FirstFrameError, FirstFrameStatus, FirstFrameStep,
};
use crate::render::plan::{PlanError, TileSegmentKind};
use crate::render::sink::{LayoutSinkStats, SinkLine};
use crate::render::table_widths::{
    TableWidthAdvance, TableWidthCache, TableWidthError, TableWidthSnapshot,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewportConfig {
    pub width: u16,
    pub rows: u16,
    pub overscan_rows: u64,
    pub max_tiles: usize,
    pub max_replay_bytes: usize,
    pub row_slack: u64,
}

/// Non-zero height estimate installed for every tile after a content-width
/// change invalidates all width-dependent measurements.
pub const CONTENT_WIDTH_RESET_ESTIMATE: u64 = 1;

impl Default for ViewportConfig {
    fn default() -> Self {
        Self {
            width: 80,
            rows: 24,
            overscan_rows: 4,
            max_tiles: 256,
            max_replay_bytes: 256 * 1024,
            row_slack: 64,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopPosition {
    Empty,
    Exact {
        anchor: Anchor,
        line_start: Option<usize>,
    },
    ReplayFrom {
        anchor: Anchor,
        from: TileId,
        target: TileId,
    },
    ApproximateLineStart {
        anchor: Anchor,
        source: usize,
    },
}

impl TopPosition {
    #[must_use]
    pub const fn anchor(self) -> Option<Anchor> {
        match self {
            Self::Empty => None,
            Self::Exact { anchor, .. }
            | Self::ReplayFrom { anchor, .. }
            | Self::ApproximateLineStart { anchor, .. } => Some(anchor),
        }
    }

    #[must_use]
    pub const fn is_exact(self) -> bool {
        matches!(self, Self::Exact { .. })
    }

    #[must_use]
    pub const fn needs_replay(self) -> bool {
        matches!(self, Self::ReplayFrom { .. })
    }

    #[must_use]
    pub const fn is_approximate(self) -> bool {
        matches!(self, Self::ApproximateLineStart { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewportFrameStatus {
    Ready,
    BudgetExhausted,
    PendingScan,
    EndOfDocument,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EofCalibrationStatus {
    Calibrated,
    BudgetExhausted,
    PendingScan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EofCalibrationProgress {
    pub status: EofCalibrationStatus,
    pub revision: u64,
    pub top_source: Option<usize>,
    pub tables_stable_for_width: bool,
    pub frame: Option<ViewportFrame>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckpointRetentionWindow {
    /// Inclusive first retained tile.
    pub lo: TileId,
    /// Exclusive retained end tile.
    pub hi: TileId,
}

impl CheckpointRetentionWindow {
    #[must_use]
    pub const fn range(self) -> std::ops::Range<usize> {
        self.lo.0..self.hi.0
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ViewportFrameStats {
    pub plan_start: Option<TileId>,
    pub plan_end: Option<TileId>,
    pub visible_tiles: usize,
    pub planned_tiles: usize,
    pub rendered_tiles: usize,
    pub replay_tiles: usize,
    pub skipped_rows: u64,
    /// `Budget::used()` observed when this frame was produced. A fresh Budget
    /// per frame yields per-call usage; reusing one Budget yields cumulative use.
    pub layout_budget: WorkLimit,
    pub layout: LayoutSinkStats,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewportFrame {
    pub document: DocumentId,
    pub revision: u64,
    pub lines: Vec<SinkLine>,
    pub top: TopPosition,
    pub rows: u16,
    pub status: ViewportFrameStatus,
    pub stats: ViewportFrameStats,
}

impl ViewportFrame {
    #[must_use]
    pub fn matches(&self, document: &Document) -> bool {
        self.document == document.id() && self.revision == document.revision()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
/// Observable outcome of one incremental update. `top_reanchored` describes the
/// anchor tile/offset; a preserved anchor may still downgrade its recovery
/// classification after checkpoints are conservatively invalidated.
pub struct ViewportUpdateResult {
    pub previous_revision: u64,
    pub revision: u64,
    pub layout_changed: bool,
    pub discarded_render: bool,
    pub top_reanchored: bool,
    pub earliest_changed_tile: Option<TileId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentWidthChange {
    pub previous_width: u16,
    pub width: u16,
    pub changed: bool,
    pub top_source: Option<usize>,
    pub estimated_rows: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewportError {
    InvalidConfig,
    Stale,
    InvalidTop,
    WrongDocument,
    OutOfOrderUpdate,
    Layout(LayoutError),
    FirstFrame(FirstFrameError),
    Plan(PlanError),
    TableWidth(TableWidthError),
    NotFinished,
    CheckpointMaintenanceInProgress,
    MissingLineSource,
    IncompleteFrame,
    RowOverflow,
}

impl fmt::Display for ViewportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig => formatter.write_str("invalid viewport configuration"),
            Self::Stale => formatter.write_str("viewport revision is stale"),
            Self::InvalidTop => formatter.write_str("viewport top tile is outside the document"),
            Self::WrongDocument => {
                formatter.write_str("viewport update belongs to another document")
            }
            Self::OutOfOrderUpdate => formatter.write_str("viewport update is out of order"),
            Self::Layout(error) => error.fmt(formatter),
            Self::FirstFrame(error) => error.fmt(formatter),
            Self::Plan(error) => error.fmt(formatter),
            Self::TableWidth(error) => error.fmt(formatter),
            Self::NotFinished => formatter.write_str("document is not finished"),
            Self::CheckpointMaintenanceInProgress => {
                formatter.write_str("checkpoint maintenance is in progress")
            }
            Self::MissingLineSource => formatter.write_str("visible row has no source"),
            Self::IncompleteFrame => formatter.write_str("viewport frame did not reach its top"),
            Self::RowOverflow => formatter.write_str("viewport row count exceeds u16"),
        }
    }
}

impl std::error::Error for ViewportError {}

impl From<LayoutError> for ViewportError {
    fn from(error: LayoutError) -> Self {
        Self::Layout(error)
    }
}

impl From<FirstFrameError> for ViewportError {
    fn from(error: FirstFrameError) -> Self {
        Self::FirstFrame(error)
    }
}

impl From<PlanError> for ViewportError {
    fn from(error: PlanError) -> Self {
        Self::Plan(error)
    }
}

impl From<TableWidthError> for ViewportError {
    fn from(error: TableWidthError) -> Self {
        Self::TableWidth(error)
    }
}

pub struct ViewportState {
    summary: Option<super::fold_summary::FoldSummary>,
    pub(crate) shown_folds: Vec<mdtui3::FoldPlaceholder>,
    fold_anchor: Option<usize>,
    fold_reanchor: bool,
    anchor_refiner: Option<Box<super::refine::SourceRefiner>>,
    links: crate::render::objects::SharedLinks,
    document: DocumentId,
    revision: u64,
    config: ViewportConfig,
    checkpoints: RenderCheckpointStore,
    table_widths: TableWidthCache,
    pub(crate) shown_tables: Vec<super::table::TableWindowStatus>,
    top: Option<Anchor>,
    top_position: TopPosition,
    frame_source: Option<(Anchor, usize)>,
    refined_top: bool,
    render: Option<ViewportRender>,
    pending_table_snapshots: HashMap<TableId, TableWidthSnapshot>,
    preparing_tables: bool,
    calibration: Option<EofCalibrationState>,
    following: bool,
    end_position: Option<EndPosition>,
}

// One coalesced end request, never a queue per input update. The source limit
// freezes Ctrl+End even if another append arrives before its layout completes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EndWork {
    pub windows: usize,
    pub source_bytes: usize,
    pub planned_tiles: usize,
    pub checkpoint_entries: usize,
    pub checkpoint_nodes: usize,
}

struct EndPosition {
    projected_tail: bool,
    window_cut: usize,
    limit: usize,
    before: usize,
    next_before: usize,
    work: EndWork,
    stats: ViewportFrameStats,
    first: Option<FirstFrame>,
    selection: Option<EndSelection>,
    table_ids: Vec<TableId>,
    table_snapshots: HashMap<TableId, TableWidthSnapshot>,
    tables_ready: bool,
    table_windows: Vec<super::table::TableWindowStatus>,
    lines: Vec<(Anchor, SinkLine)>,
    frame: Option<ViewportFrame>,
}

impl EndPosition {
    fn new(limit: usize) -> Self {
        Self {
            projected_tail: false,
            window_cut: limit,
            limit,
            before: limit,
            next_before: limit,
            work: EndWork::default(),
            stats: ViewportFrameStats::default(),
            first: None,
            selection: None,
            table_ids: Vec::new(),
            table_snapshots: HashMap::new(),
            tables_ready: false,
            table_windows: Vec::new(),
            lines: Vec::new(),
            frame: None,
        }
    }
}

// Every suppression, measured tile, row selection and final publication is
// independently resumable. In particular a one-query frame can make progress.
struct EndSelection {
    zero_progress: super::first_frame::ZeroRangeProgress,
    segment: usize,
    row: usize,
    owners: Vec<(TileId, usize, u64)>,
    line: usize,
    found_tail: bool,
    selected: Vec<(Anchor, SinkLine)>,
}

struct EofCalibrationState {
    table_ids: Vec<TableId>,
    table_index: usize,
    frame: Option<ViewportFrame>,
}

impl ViewportState {
    pub fn new(document: &Document, config: ViewportConfig) -> Result<Self, ViewportError> {
        validate_config(config)?;
        let checkpoints = RenderCheckpointStore::new(document);
        let links = crate::render::objects::SharedLinks::new(document);
        let mut table_widths = TableWidthCache::new(document);
        table_widths.set_links(links.clone());
        let top = (document.tile_count() > 0).then_some(Anchor {
            tile: TileId(0),
            offset: 0,
        });
        let top_position = if let Some(anchor) = top {
            TopPosition::ApproximateLineStart {
                anchor,
                source: document.tile(TileId(0)).map_or(0, |span| span.start),
            }
        } else {
            TopPosition::Empty
        };
        Ok(Self {
            summary: None,
            shown_folds: Vec::new(),
            fold_anchor: None,
            fold_reanchor: false,
            anchor_refiner: None,
            links,
            document: document.id(),
            revision: document.revision(),
            config,
            checkpoints,
            table_widths,
            shown_tables: Vec::new(),
            top,
            top_position,
            frame_source: None,
            refined_top: false,
            render: None,
            pending_table_snapshots: HashMap::new(),
            preparing_tables: false,
            calibration: None,
            following: false,
            end_position: None,
        })
    }

    pub fn from_first_frame(
        document: &Document,
        first: FirstFrame,
        config: ViewportConfig,
    ) -> Result<Self, ViewportError> {
        validate_config(config)?;
        if !first.matches(document) {
            return Err(ViewportError::Stale);
        }
        let links = first.shared_links();
        let checkpoints = first.into_layout_parts();
        let mut table_widths = TableWidthCache::new(document);
        table_widths.set_links(links.clone());
        if checkpoints.heights().len() != document.tile_count() {
            return Err(ViewportError::InvalidConfig);
        }
        let top = (document.tile_count() > 0).then_some(Anchor {
            tile: TileId(0),
            offset: 0,
        });
        let top_position = match top {
            Some(anchor) => position_for(document, &checkpoints, anchor),
            None => TopPosition::Empty,
        };
        Ok(Self {
            summary: None,
            shown_folds: Vec::new(),
            fold_anchor: None,
            fold_reanchor: false,
            anchor_refiner: None,
            links,
            document: document.id(),
            revision: document.revision(),
            config,
            checkpoints,
            table_widths,
            shown_tables: Vec::new(),
            top,
            top_position,
            frame_source: None,
            refined_top: false,
            render: None,
            pending_table_snapshots: HashMap::new(),
            preparing_tables: false,
            calibration: None,
            following: false,
            end_position: None,
        })
    }

    pub fn folds(&self) -> &mdtui3::FoldView<super::checkpoint::RenderCheckpoint> {
        self.checkpoints.view()
    }
    pub fn folds_mut(&mut self) -> &mut mdtui3::FoldView<super::checkpoint::RenderCheckpoint> {
        self.checkpoints
            .view_mut()
            .expect("finish checkpoint maintenance before fold operations")
    }
    pub(crate) fn set_fold_summary(
        &mut self,
        document: &Document,
        summary: super::fold_summary::FoldSummary,
    ) -> Result<(), ViewportError> {
        summary.check_binding(document, self.folds())?;
        let candidate = self
            .folds()
            .fold_candidate(document, summary.id)?
            .ok_or(ViewportError::InvalidTop)?;
        let start = document
            .block_at(candidate.hidden.start)
            .and_then(|id| document.block(id))
            .map_or(candidate.hidden.start, |b| b.source.start);
        let tile = document.tile_at(start).map_or(0, |t| t.0);
        let end = document.tile_count();
        self.checkpoints.heights_mut(document)?.expire(tile..end)?;
        self.checkpoints.invalidate(document, tile..end)?;
        self.summary = Some(summary);
        self.render = None;
        self.restart_end(document);
        Ok(())
    }
    pub fn fold_changed(&mut self, document: &Document) {
        self.summary = None;
        if self.fold_anchor.is_none() {
            self.fold_anchor = self.top_source(document);
        }
        self.fold_reanchor = true;
        self.anchor_refiner = None;
        self.render = None;
        self.pending_table_snapshots.clear();
        self.preparing_tables = false;
        self.calibration = None;
        self.restart_end(document);
    }

    #[must_use]
    pub const fn following(&self) -> bool {
        self.following
    }

    /// End retains an intention across revisions; Ctrl+End freezes this source
    /// cut. Both first use the height estimate, then select actual laid-out rows.
    pub fn seek_end(&mut self, document: &Document, follow: bool) -> Result<bool, ViewportError> {
        self.check_document(document)?;
        if follow && self.following {
            return Ok(false);
        }
        let changed = self.following != follow;
        self.following = follow;
        let moved = self.end(document)?;
        self.end_position = Some(EndPosition::new(document.source().len()));
        self.render = None;
        self.calibration = None;
        Ok(changed || moved || !follow)
    }

    #[must_use]
    pub fn end_work(&self) -> EndWork {
        self.end_position
            .as_ref()
            .map_or(EndWork::default(), |end| end.work)
    }

    /// Pause at the displayed source cut. Focus/sidebar changes do not reflow
    /// the body to a different top just to reclaim the status row. A subsequent
    /// body scroll or explicit source navigation clears this frozen position.
    pub fn pause_following(&mut self) -> bool {
        let changed = self.following;
        self.following = false;
        changed
    }

    pub fn cancel_end(&mut self) -> bool {
        let changed = self.following || self.end_position.is_some();
        self.following = false;
        self.end_position = None;
        changed
    }

    fn restart_end(&mut self, document: &Document) {
        if self.following {
            self.end_position = Some(EndPosition::new(document.source().len()));
        } else if let Some(end) = &self.end_position {
            self.end_position = Some(EndPosition::new(end.limit));
        }
    }

    fn end_frame(
        &mut self,
        document: &Document,
        budget: &mut Budget,
    ) -> Result<ViewportFrame, ViewportError> {
        let mut end = self.end_position.take().ok_or(ViewportError::InvalidTop)?;
        let result = self.advance_end(document, &mut end, budget);
        self.end_position = Some(end);
        result
    }

    fn advance_end(
        &mut self,
        document: &Document,
        end: &mut EndPosition,
        budget: &mut Budget,
    ) -> Result<ViewportFrame, ViewportError> {
        if let Some(frame) = &end.frame {
            let mut frame = frame.clone();
            frame.stats.layout_budget = budget.used();
            return Ok(frame);
        }
        loop {
            if end.first.is_none() {
                if !end.projected_tail && end.before > 0 {
                    end.window_cut = end.before;
                    let mut source = end.before - 1;
                    while !document.source().is_char_boundary(source) {
                        source -= 1;
                    }
                    let anchor = mdtui3::FoldAnchor::new(document, source)?;
                    let Some(target) = anchor.resolve(
                        document,
                        self.checkpoints.view(),
                        u64::from(self.config.rows),
                        budget,
                    )?
                    else {
                        return Ok(self.end_pending(budget));
                    };
                    if target.hidden_by.is_some() {
                        end.before = target.target.source.unwrap_or(source).saturating_add(1);
                    }
                    end.projected_tail = true;
                }
                let block = match document.previous_content_block(end.before, budget) {
                    Ok(block) => block.and_then(|id| document.block(id)),
                    Err(_) => return Ok(self.end_pending(budget)),
                };
                let Some(block) = block else {
                    break;
                };
                let stop = block.source.end.min(end.before);
                let last = document
                    .tile_at(stop.saturating_sub(1))
                    .ok_or(ViewportError::InvalidTop)?;
                let want = usize::from(self.config.rows).saturating_sub(end.lines.len());
                let estimate = self
                    .checkpoints
                    .heights()
                    .prefix_rows(last.0 + 1)
                    .unwrap_or(0)
                    .saturating_sub(want as u128 + u128::from(self.config.overscan_rows));
                let estimated = self
                    .checkpoints
                    .heights()
                    .locate(estimate)
                    .map_or(last.0, |anchor| anchor.tile.0);
                // A small suffix bounds local recovery and scratch even at width 1.
                let earliest = last
                    .0
                    .saturating_sub(self.config.max_tiles.min(8).saturating_sub(1));
                let block_start = document
                    .tile_at(block.source.start)
                    .map_or(last.0, |tile| tile.0);
                let mut start = estimated.min(block_start).max(earliest).min(last.0);
                let mut tile_limit = last.0 - start + 1;
                end.next_before = document.tile(TileId(start)).map_or(0, |span| span.start);
                // FirstFrame already promotes an arbitrarily long hidden code
                // info prefix using bounded metadata. Keep that path reachable
                // for empty code labels without walking the old prefix tiles.
                if let mdtui3::BlockKind::Code { info: Some(info) } = block.kind
                    && document
                        .tile(TileId(earliest))
                        .is_some_and(|span| info.end >= span.start)
                    && block_start < earliest
                    && document
                        .tile_at(info.end.saturating_sub(1))
                        .is_some_and(|tile| tile.0 >= block_start.saturating_add(8))
                {
                    start = block_start;
                    tile_limit = self.config.max_tiles.min(8);
                    end.next_before = block.source.start;
                }
                if let mdtui3::BlockKind::Table { id } = block.kind
                    && let Some(table) = document.table(id)
                    && let Some(row) = table.rows().get(
                        table
                            .rows()
                            .partition_point(|row| row.source.start < stop)
                            .saturating_sub(1),
                    )
                {
                    let owner = document
                        .tile_at(super::table_window::row_owner(row))
                        .ok_or(ViewportError::InvalidTop)?;
                    if owner.0 < start {
                        start = owner.0;
                        tile_limit = self
                            .config
                            .max_tiles
                            .min(8)
                            .min(document.tile_count() - start);
                    }
                    end.next_before = row.source.start.min(end.next_before);
                }
                if !budget.charge(WorkLimit {
                    recovery: 1,
                    ..WorkLimit::default()
                }) {
                    return Ok(self.end_pending(budget));
                }
                let mut first = FirstFrame::new_for_view(
                    document,
                    FirstFrameConfig {
                        top_tile: TileId(start),
                        tile_limit,
                        width: self.config.width,
                        row_limit: (8 * (TILE_BYTES + 4) + self.config.row_slack as usize) as u64,
                    },
                    &self.checkpoints,
                )?;
                first.set_projection_range(Span::new(
                    document.tile(TileId(start)).map_or(0, |s| s.start),
                    end.window_cut,
                ));
                first.set_fold_summary(self.summary.clone());
                end.table_ids = first
                    .plan()
                    .blocks
                    .iter()
                    .filter_map(|b| {
                        if let mdtui3::BlockKind::Table { id } = b.kind {
                            Some(id)
                        } else {
                            None
                        }
                    })
                    .collect();
                if !self.checkpoints.view().has_collapsed()
                    && let mdtui3::BlockKind::Table { id } = block.kind
                    && !end.table_ids.contains(&id)
                {
                    end.table_ids.push(id);
                }
                let mut keep = end
                    .table_windows
                    .iter()
                    .map(|w| w.table_id)
                    .collect::<Vec<_>>();
                keep.extend_from_slice(&end.table_ids);
                self.table_widths.retain_tables(&keep);
                first.set_table_options(
                    document,
                    self.table_widths.policy(),
                    &self.table_widths.column_positions(),
                )?;
                first.set_table_view(super::table_window::TableView {
                    anchor: Anchor {
                        tile: TileId(start),
                        offset: 0,
                    },
                    source: None,
                    rows: usize::from(self.config.rows)
                        .saturating_add(self.config.overscan_rows as usize * 2)
                        .saturating_add(4),
                    tail: Some(end.window_cut),
                });
                first.set_math_floor(first.scanner().requested().start);
                first.set_links(self.links.clone());
                end.first = Some(first);
            }
            if !end.tables_ready {
                for &id in &end.table_ids {
                    if end.table_snapshots.contains_key(&id) {
                        continue;
                    }
                    let Some(snapshot) = self.table_widths.snapshot_budgeted(
                        document,
                        id,
                        usize::from(self.config.width),
                        budget,
                    )?
                    else {
                        return Ok(self.end_pending(budget));
                    };
                    end.table_snapshots.insert(id, snapshot);
                }
                end.first
                    .as_mut()
                    .ok_or(ViewportError::InvalidTop)?
                    .set_table_widths(std::mem::take(&mut end.table_snapshots))?;
                end.tables_ready = true;
            }
            if end.selection.is_none() {
                let first = end.first.as_mut().ok_or(ViewportError::InvalidTop)?;
                let step = first.advance_using(document, &mut self.checkpoints, budget)?;
                if matches!(
                    step.status,
                    FirstFrameStatus::PendingScan | FirstFrameStatus::BudgetExhausted
                ) {
                    let mut frame = self.end_pending(budget);
                    frame.stats.layout = first.sink_stats().unwrap_or_default();
                    return Ok(frame);
                }
                if !budget.charge(WorkLimit {
                    queries: 1,
                    objects: 1,
                    ..WorkLimit::default()
                }) {
                    return Ok(self.end_pending(budget));
                }
                if let Some(glyph) = first
                    .scanner()
                    .segments()
                    .first()
                    .filter(|g| g.text.is_indexed())
                {
                    end.next_before = end.next_before.min(glyph.source.start);
                }
                if let Some(meta) = first.scanner().skipped_math()
                    && meta.source.end < end.before
                {
                    end.next_before = end.next_before.max(meta.source.end);
                }
                if let Some(at) = first.scanner().first_math_source() {
                    end.next_before = end.next_before.min(at);
                }
                end.selection = Some(EndSelection {
                    zero_progress: Default::default(),
                    segment: 0,
                    row: 0,
                    owners: Vec::new(),
                    line: first.lines().len(),
                    found_tail: false,
                    selected: Vec::new(),
                });
            }
            if !self.select_end_window(document, end, budget)? {
                return Ok(self.end_pending(budget));
            }
            if end.lines.len() >= usize::from(self.config.rows) || end.before == 0 {
                let mut frame = self.finish_end(document, end);
                frame.stats.layout_budget = budget.used();
                end.frame = Some(frame.clone());
                return Ok(frame);
            }
        }
        let mut frame = self.finish_end(document, end);
        frame.stats.layout_budget = budget.used();
        end.frame = Some(frame.clone());
        Ok(frame)
    }

    fn select_end_window(
        &mut self,
        document: &Document,
        end: &mut EndPosition,
        budget: &mut Budget,
    ) -> Result<bool, ViewportError> {
        let first = end.first.as_ref().ok_or(ViewportError::InvalidTop)?;
        let selection = end.selection.as_mut().ok_or(ViewportError::InvalidTop)?;
        if !super::first_frame::measure_zero_ranges(
            first.zero_ranges(),
            self.checkpoints.heights_mut(document)?,
            &mut selection.zero_progress,
            budget,
        )? {
            return Ok(false);
        }
        while let Some(segment) = first.plan().segments.get(selection.segment) {
            if !budget.charge(WorkLimit {
                queries: 1,
                objects: 1,
                recovery: 1,
                ..WorkLimit::default()
            }) {
                return Ok(false);
            }
            // Same height publication as sync_layout_parts, one tile at a time.
            // Tail recovery checkpoints stay local, avoiding old-tail retention.

            let rows = first
                .sink()
                .ok_or(ViewportError::MissingLineSource)?
                .tile_rows(segment.tile)
                .unwrap_or(0);
            if rows > 0 {
                selection.owners.push((segment.tile, selection.row, rows));
            }
            selection.row += rows as usize;
            selection.segment += 1;
        }
        let want = usize::from(self.config.rows).saturating_sub(end.lines.len());
        while selection.line > 0 && selection.selected.len() < want {
            if !budget.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..WorkLimit::default()
            }) {
                return Ok(false);
            }
            selection.line -= 1;
            let line = &first.lines()[selection.line];
            if let Some(source) = line.source {
                if source.source.start >= end.before {
                    continue;
                }
                selection.found_tail = true;
            }
            // Source-less spacing before a future paragraph belongs beyond a
            // frozen Ctrl+End cut. It cannot move that jump on a later append.
            if !selection.found_tail {
                continue;
            }
            let logical_line = first
                .sink()
                .and_then(|sink| sink.line_offset(selection.line))
                .ok_or(ViewportError::MissingLineSource)? as usize;
            let owner = selection
                .owners
                .partition_point(|(_, start, _)| *start <= logical_line);
            let &(tile, start, _) = selection
                .owners
                .get(owner.saturating_sub(1))
                .ok_or(ViewportError::MissingLineSource)?;
            selection.selected.push((
                Anchor {
                    tile,
                    offset: (logical_line - start) as u64,
                },
                line.clone(),
            ));
        }
        if !budget.charge(WorkLimit {
            queries: 1,
            objects: 1,
            recovery: 1,
            ..WorkLimit::default()
        }) {
            return Ok(false);
        }
        let mut selection = end.selection.take().ok_or(ViewportError::InvalidTop)?;
        selection.selected.reverse();
        selection.selected.append(&mut end.lines);
        end.lines = selection.selected;
        if let Some(sink) = first.sink() {
            for status in sink.table_windows() {
                if let Some(range) = first.scanner().selected_rows(status.table_id)
                    && let Some(table) = document.table(status.table_id)
                    && first.plan().visible.start >= table.source.start
                    && let Some(row) = table.rows().get(range.start)
                {
                    end.next_before = end.next_before.min(row.source.start);
                }
                if let Some(existing) = end
                    .table_windows
                    .iter_mut()
                    .find(|w| w.table_id == status.table_id)
                {
                    *existing = *status;
                } else {
                    end.table_windows.push(*status);
                }
            }
        }
        end.before = end.next_before;
        end.projected_tail = false;
        end.work.windows += 1;
        end.work.planned_tiles += first.plan().segments.len();
        end.work.checkpoint_entries = end.work.checkpoint_entries.max(self.checkpoints.len());
        end.work.checkpoint_nodes = end
            .work
            .checkpoint_nodes
            .max(self.checkpoints.arena_stats().nodes);
        let scan = first.scanner().stats();
        end.work.source_bytes += scan.scanned_bytes
            + scan.edge_source_bytes
            + first
                .sink_stats()
                .map_or(0, |stats| stats.preparation_source_bytes);
        end.stats = ViewportFrameStats {
            plan_start: Some(first.plan().tile_start),
            plan_end: Some(first.plan().tile_end),
            planned_tiles: first.plan().segments.len(),
            visible_tiles: first.plan().segments.len(),
            rendered_tiles: first.processed_tiles(),
            layout: first.sink_stats().unwrap_or_default(),
            layout_budget: budget.used(),
            ..ViewportFrameStats::default()
        };
        // Ownership of this bounded FirstFrame is released only after its
        // measured rows have been published. The shared memo retires separately.
        end.first = None;
        end.tables_ready = false;
        end.table_ids.clear();
        self.links.request_retirement();
        Ok(true)
    }

    fn end_pending(&self, budget: &Budget) -> ViewportFrame {
        let mut frame = self.empty_frame();
        frame.status = ViewportFrameStatus::BudgetExhausted;
        frame.stats.layout_budget = budget.used();
        frame
    }

    fn finish_end(&mut self, document: &Document, end: &EndPosition) -> ViewportFrame {
        self.shown_tables = end
            .table_windows
            .iter()
            .copied()
            .filter(|status| {
                end.lines.iter().any(|(_, line)| {
                    line.source
                        .and_then(|s| s.block)
                        .and_then(|b| document.block(b))
                        .is_some_and(|b| {
                            b.kind
                                == mdtui3::BlockKind::Table {
                                    id: status.table_id,
                                }
                        })
                })
            })
            .collect();
        self.shown_tables.sort_unstable_by_key(|status| {
            document
                .table(status.table_id)
                .map_or(usize::MAX, |table| table.source.start)
        });
        self.table_widths.retain_tables(
            &self
                .shown_tables
                .iter()
                .map(|w| w.table_id)
                .collect::<Vec<_>>(),
        );
        let top = end.lines.first().map(|(anchor, _)| *anchor);
        let source = end
            .lines
            .first()
            .and_then(|(_, line)| line.source)
            .map(|source| source.source.start);
        self.top = top;
        self.frame_source = top.zip(source);
        self.top_position = match (top, source) {
            (Some(anchor), Some(source)) => {
                if matches!(
                    self.checkpoints.heights().height_state(anchor.tile),
                    Some(mdtui3::HeightState::Measured)
                ) {
                    TopPosition::Exact {
                        anchor,
                        line_start: Some(source),
                    }
                } else {
                    TopPosition::ApproximateLineStart { anchor, source }
                }
            }
            (Some(anchor), None) => TopPosition::ApproximateLineStart {
                anchor,
                source: document.tile(anchor.tile).map_or(0, |span| span.start),
            },
            _ => TopPosition::Empty,
        };
        self.refined_top = source.is_some();
        ViewportFrame {
            document: document.id(),
            revision: document.revision(),
            lines: end.lines.iter().map(|(_, line)| line.clone()).collect(),
            top: self.top_position,
            rows: end.lines.len() as u16,
            status: ViewportFrameStatus::EndOfDocument,
            stats: end.stats,
        }
    }

    pub fn initialize(
        &mut self,
        document: &Document,
        budget: &mut Budget,
    ) -> Result<bool, mdtui3::LayoutError> {
        self.links.prepare_shape(document, budget)
    }

    pub fn shared_links(&self) -> crate::render::objects::SharedLinks {
        self.links.clone()
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
    pub const fn config(&self) -> ViewportConfig {
        self.config
    }

    #[must_use]
    pub const fn top_anchor(&self) -> Option<Anchor> {
        self.top
    }

    #[must_use]
    pub const fn top_position(&self) -> TopPosition {
        self.top_position
    }

    #[must_use]
    pub fn top_row(&self) -> Option<u128> {
        self.top
            .and_then(|anchor| self.checkpoints.heights().row_of(anchor))
    }

    #[must_use]
    pub const fn viewport_rows(&self) -> u16 {
        self.config.rows
    }

    #[must_use]
    pub const fn overscan_rows(&self) -> u64 {
        self.config.overscan_rows
    }

    #[must_use]
    pub fn heights(&self) -> &HeightIndex {
        self.checkpoints.heights()
    }

    pub fn heights_mut(&mut self) -> &mut mdtui3::FoldView<super::checkpoint::RenderCheckpoint> {
        self.checkpoints
            .view_mut()
            .expect("layout maintenance must finish before measurement")
    }

    pub(crate) fn reading_parts_mut(
        &mut self,
    ) -> (&mut RenderCheckpointStore, &mut TableWidthCache) {
        (&mut self.checkpoints, &mut self.table_widths)
    }
    pub fn layout_owner_mut(&mut self) -> &mut RenderCheckpointStore {
        &mut self.checkpoints
    }

    #[must_use]
    pub fn checkpoints(&self) -> &RenderCheckpointStore {
        &self.checkpoints
    }

    #[must_use]
    pub fn table_widths(&self) -> &TableWidthCache {
        &self.table_widths
    }

    pub fn table_widths_mut(&mut self) -> &mut TableWidthCache {
        &mut self.table_widths
    }

    pub fn set_table_policy(
        &mut self,
        document: &Document,
        policy: super::table_widths::TableWidthPolicy,
    ) -> Result<bool, ViewportError> {
        self.check_document(document)?;
        if !self.table_widths.set_policy(policy) {
            return Ok(false);
        }
        let mut ids = self.table_widths.table_ids();
        for status in &self.shown_tables {
            if !ids.contains(&status.table_id) {
                ids.push(status.table_id);
            }
        }
        self.restart_end(document);
        for id in ids {
            self.invalidate_table(document, id)?;
        }
        Ok(true)
    }

    pub fn set_table_first_column(
        &mut self,
        document: &Document,
        id: TableId,
        first: usize,
    ) -> Result<bool, ViewportError> {
        if !self.table_widths.set_first_column(document, id, first)? {
            return Ok(false);
        }
        self.invalidate_table(document, id)?;
        Ok(true)
    }

    fn invalidate_table(&mut self, document: &Document, id: TableId) -> Result<(), ViewportError> {
        if let Some(table) = document.table(id) {
            let tiles = document.tiles_overlapping(table.source);
            self.checkpoints
                .heights_mut(document)?
                .expire(tiles.clone())?;
            self.checkpoints.invalidate(document, tiles)?;
        }
        self.restart_end(document);
        self.render = None;
        self.pending_table_snapshots.clear();
        self.preparing_tables = false;
        self.calibration = None;
        Ok(())
    }

    pub fn advance_table_widths(
        &mut self,
        document: &Document,
        table_id: TableId,
        content_width: usize,
        budget: &mut Budget,
    ) -> Result<TableWidthAdvance, ViewportError> {
        self.check_document(document)?;
        let was_provisional = self
            .table_widths
            .stats(document, table_id, content_width)?
            .provisional;
        let result = self
            .table_widths
            .advance(document, table_id, content_width, budget)?;
        if result.width_changed
            || result.visible_columns_before != result.visible_columns_after
            || was_provisional != result.provisional
        {
            self.invalidate_table(document, table_id)?;
            self.render = None;
            self.pending_table_snapshots.clear();
            self.preparing_tables = false;
            self.calibration = None;
        }
        Ok(result)
    }

    /// Retention covers two screens before and after the current viewport.
    #[must_use]
    pub fn checkpoint_window(&self, document: &Document) -> CheckpointRetentionWindow {
        let tile_count = document.tile_count().min(self.checkpoints.heights().len());
        if tile_count == 0 {
            return CheckpointRetentionWindow {
                lo: TileId(0),
                hi: TileId(0),
            };
        }
        let top = self.top_row().unwrap_or(0);
        let page = u128::from(self.config.rows.max(1));
        let start = top.saturating_sub(page.saturating_mul(2));
        let end = top
            .saturating_add(page.saturating_mul(3))
            .min(self.checkpoints.heights().total_rows());
        let visible =
            self.checkpoints
                .heights()
                .viewport(start..end, 0, self.config.max_tiles.max(1));
        let (lo, hi) = if let (Some(first), Some(last)) = (visible.first(), visible.last()) {
            (first.tile.0, last.tile.0.saturating_add(1))
        } else {
            let tile = self.top.map_or(0, |anchor| anchor.tile.0);
            (tile, tile.saturating_add(1))
        };
        CheckpointRetentionWindow {
            lo: TileId(lo.min(tile_count)),
            hi: TileId(hi.min(tile_count)),
        }
    }

    pub fn maintain_checkpoints(
        &mut self,
        document: &Document,
        budget: &mut Budget,
    ) -> Result<CheckpointMaintenanceProgress, ViewportError> {
        self.check_document(document)?;
        if !self.checkpoints.maintenance_in_progress() {
            let window = self.checkpoint_window(document);
            self.checkpoints
                .start_maintenance(document, window.range(), document.tile_count())?;
        }
        Ok(self.checkpoints.advance_maintenance(document, budget)?)
    }

    pub fn checkpoints_mut(&mut self) -> &mut RenderCheckpointStore {
        self.refined_top = false;
        self.render = None;
        &mut self.checkpoints
    }

    #[must_use]
    pub const fn has_pending_frame(&self) -> bool {
        self.render.is_some() || self.preparing_tables
    }

    #[must_use]
    pub fn top_source(&self, document: &Document) -> Option<usize> {
        if let Some((anchor, source)) = self.frame_source
            && self.top == Some(anchor)
        {
            return Some(source);
        }
        top_source_for(document, &self.checkpoints, self.top, self.top_position)
    }

    /// Apply one consecutive update to both layout stores. A repeated or empty
    /// update at the current revision is a no-op and preserves pending work; a
    /// real revision change drops the current bounded render before the next
    /// frame is requested.
    pub fn apply_update(
        &mut self,
        document: &Document,
        update: &Update,
    ) -> Result<ViewportUpdateResult, ViewportError> {
        if self.document != document.id() || update.document != Some(document.id()) {
            return Err(ViewportError::WrongDocument);
        }
        if self.checkpoints.maintenance_in_progress() {
            return Err(ViewportError::CheckpointMaintenanceInProgress);
        }
        if update.revision != document.revision() || update.base_revision > update.revision {
            return Err(ViewportError::OutOfOrderUpdate);
        }
        if self.revision == update.revision {
            return Ok(ViewportUpdateResult {
                previous_revision: self.revision,
                revision: self.revision,
                ..ViewportUpdateResult::default()
            });
        }
        if update.base_revision != self.revision {
            return Err(ViewportError::OutOfOrderUpdate);
        }
        self.links.apply_shape_update(document, update)?;
        self.anchor_refiner = None;
        self.summary = None;
        self.refined_top = false;
        self.restart_end(document);

        let previous_revision = self.revision;
        let old_top = self.top;
        let old_top_source = self.top_source(document);
        let earliest_source = update
            .changes
            .iter()
            .map(|change| change.source.start)
            .min()
            .or_else(|| (!update.appended.is_empty()).then_some(update.appended.start));
        let earliest_changed_tile = earliest_source.and_then(|source| document.tile_at(source));
        let layout_changed = earliest_source.is_some();
        let top_affected = match (old_top, earliest_changed_tile) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(top), Some(changed)) => changed <= top.tile,
        };

        self.checkpoints.apply(document, update)?;
        self.table_widths.apply_update(document, update)?;
        self.document = document.id();
        self.revision = update.revision;
        let discarded_render = self.render.take().is_some();
        self.calibration = None;
        let mut top_reanchored = false;

        if old_top.is_none() && document.tile_count() > 0 {
            let anchor = Anchor {
                tile: TileId(0),
                offset: 0,
            };
            self.top = Some(anchor);
            self.top_position = position_for(document, &self.checkpoints, anchor);
            top_reanchored = true;
        } else if top_affected && let Some(old_top) = old_top {
            let source = old_top_source
                .unwrap_or_else(|| document.tile(old_top.tile).map_or(0, |span| span.start));
            let Some(target_tile) = document
                .tile(old_top.tile)
                .map(|_| old_top.tile)
                .or_else(|| document.tile_at(source))
                .or_else(|| document.tile_count().checked_sub(1).map(TileId))
            else {
                self.top = None;
                self.top_position = TopPosition::Empty;
                return Ok(ViewportUpdateResult {
                    previous_revision,
                    revision: self.revision,
                    layout_changed,
                    discarded_render,
                    top_reanchored: true,
                    earliest_changed_tile,
                });
            };
            let height = self.checkpoints.heights().height(target_tile).unwrap_or(0);
            let offset = if target_tile == old_top.tile {
                old_top.offset.min(height.saturating_sub(1))
            } else {
                0
            };
            if target_tile == old_top.tile && height > 0 && offset == old_top.offset {
                self.top = Some(old_top);
                self.top_position = TopPosition::ApproximateLineStart {
                    anchor: old_top,
                    source,
                };
                return Ok(ViewportUpdateResult {
                    previous_revision,
                    revision: self.revision,
                    layout_changed,
                    discarded_render,
                    top_reanchored: false,
                    earliest_changed_tile,
                });
            }
            let row = self
                .checkpoints
                .heights()
                .prefix_rows(target_tile.0)
                .unwrap_or(0)
                .saturating_add(u128::from(offset));
            let anchor = self.checkpoints.heights().locate(row).or_else(|| {
                self.checkpoints
                    .heights()
                    .total_rows()
                    .checked_sub(1)
                    .and_then(|last| self.checkpoints.heights().locate(last))
            });
            if let Some(anchor) = anchor {
                self.top = Some(anchor);
                self.top_position = TopPosition::ApproximateLineStart { anchor, source };
                top_reanchored = true;
            } else {
                self.top = None;
                self.top_position = TopPosition::Empty;
                top_reanchored = true;
            }
        }

        Ok(ViewportUpdateResult {
            previous_revision,
            revision: self.revision,
            layout_changed,
            discarded_render,
            top_reanchored,
            earliest_changed_tile,
        })
    }

    pub fn begin_eof_calibration(
        &mut self,
        document: &Document,
        update: &Update,
    ) -> Result<ViewportUpdateResult, ViewportError> {
        if !document.is_finished() {
            return Err(ViewportError::NotFinished);
        }
        let result = self.apply_update(document, update)?;
        if result.previous_revision != result.revision {
            self.calibration = None;
        }
        Ok(result)
    }

    /// Finish once and perform one bounded final-calibration step. UI code may
    /// return on BudgetExhausted/PendingScan and resume later; no full-document
    /// scan is performed here.
    pub fn finish_and_calibrate(
        &mut self,
        document: &mut Document,
        budget: &mut Budget,
    ) -> Result<EofCalibrationProgress, ViewportError> {
        if self.checkpoints.maintenance_in_progress() {
            return Err(ViewportError::CheckpointMaintenanceInProgress);
        }
        if !document.is_finished() {
            let update = document.finish();
            self.begin_eof_calibration(document, &update)?;
        } else if !self.matches(document) {
            return Err(ViewportError::OutOfOrderUpdate);
        }
        self.advance_eof_calibration(document, budget)
    }

    pub fn advance_eof_calibration(
        &mut self,
        document: &Document,
        budget: &mut Budget,
    ) -> Result<EofCalibrationProgress, ViewportError> {
        self.check_document(document)?;
        if self.checkpoints.maintenance_in_progress() {
            return Err(ViewportError::CheckpointMaintenanceInProgress);
        }
        if !document.is_finished() {
            return Err(ViewportError::NotFinished);
        }
        if self.end_position.is_some() {
            let frame = self.frame(document, budget)?;
            let ready = matches!(
                frame.status,
                ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument
            );
            if ready {
                let ids = self
                    .shown_tables
                    .iter()
                    .map(|w| w.table_id)
                    .collect::<Vec<_>>();
                for id in ids {
                    let was_provisional = self
                        .table_widths
                        .stats(document, id, usize::from(self.config.width))?
                        .provisional;
                    let advance = self.advance_table_widths(
                        document,
                        id,
                        usize::from(self.config.width),
                        budget,
                    )?;
                    if !advance.stable_for_width
                        || advance.width_changed
                        || advance.visible_columns_before != advance.visible_columns_after
                        || was_provisional != advance.provisional
                    {
                        return Ok(self.calibration_progress(
                            EofCalibrationStatus::BudgetExhausted,
                            None,
                            false,
                            document,
                        ));
                    }
                }
            }
            return Ok(self.calibration_progress(
                if ready {
                    EofCalibrationStatus::Calibrated
                } else {
                    EofCalibrationStatus::BudgetExhausted
                },
                ready.then_some(frame),
                ready,
                document,
            ));
        }
        if self.calibration.is_none() {
            self.calibration = Some(self.new_calibration(document)?);
        }
        if let Some(frame) = self
            .calibration
            .as_ref()
            .and_then(|calibration| calibration.frame.clone())
        {
            return Ok(self.calibration_progress(
                EofCalibrationStatus::Calibrated,
                Some(frame),
                true,
                document,
            ));
        }

        loop {
            let (table_index, table_len) = {
                let calibration = self.calibration.as_ref().ok_or(ViewportError::InvalidTop)?;
                (calibration.table_index, calibration.table_ids.len())
            };
            if table_index >= table_len {
                break;
            }
            let table_id = self
                .calibration
                .as_ref()
                .ok_or(ViewportError::InvalidTop)?
                .table_ids[table_index];
            let advance = self.table_widths.advance(
                document,
                table_id,
                usize::from(self.config.width),
                budget,
            )?;
            if advance.width_changed
                || advance.visible_columns_before != advance.visible_columns_after
            {
                if let Some(table) = document.table(table_id) {
                    let range = document.tiles_overlapping(table.source);
                    self.checkpoints
                        .heights_mut(document)?
                        .expire(range.clone())?;
                    self.checkpoints.invalidate(document, range)?;
                }
                self.render = None;
                self.pending_table_snapshots.clear();
                self.preparing_tables = false;
            }
            if advance.completion == mdtui3::Completion::BudgetExhausted {
                return Ok(self.calibration_progress(
                    EofCalibrationStatus::BudgetExhausted,
                    None,
                    false,
                    document,
                ));
            }
            let calibration = self.calibration.as_mut().ok_or(ViewportError::InvalidTop)?;
            calibration.table_index = calibration.table_index.saturating_add(1);
        }

        let frame = self.frame(document, budget)?;
        if self.calibration.is_none() {
            // Automatic source reconciliation can move within a tile after a
            // width reset. Calibrate the resulting visible window once more.
            self.calibration = Some(self.new_calibration(document)?);
            return Ok(self.calibration_progress(
                EofCalibrationStatus::BudgetExhausted,
                None,
                false,
                document,
            ));
        }
        match frame.status {
            ViewportFrameStatus::BudgetExhausted => Ok(self.calibration_progress(
                EofCalibrationStatus::BudgetExhausted,
                None,
                true,
                document,
            )),
            ViewportFrameStatus::PendingScan => Ok(self.calibration_progress(
                EofCalibrationStatus::PendingScan,
                None,
                true,
                document,
            )),
            ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument => {
                let calibration = self.calibration.as_mut().ok_or(ViewportError::InvalidTop)?;
                calibration.frame = Some(frame.clone());
                Ok(self.calibration_progress(
                    EofCalibrationStatus::Calibrated,
                    Some(frame),
                    true,
                    document,
                ))
            }
        }
    }

    fn new_calibration(
        &mut self,
        document: &Document,
    ) -> Result<EofCalibrationState, ViewportError> {
        let Some(anchor) = self.top else {
            return Ok(EofCalibrationState {
                table_ids: Vec::new(),
                table_index: 0,
                frame: None,
            });
        };
        let position = position_for(document, &self.checkpoints, anchor);
        let mut table_ids = self.visible_table_ids(document, anchor, position);
        self.table_widths.retain_tables(&table_ids);
        table_ids.sort_unstable_by_key(|id| id.0);
        Ok(EofCalibrationState {
            table_ids,
            table_index: 0,
            frame: None,
        })
    }

    fn calibration_progress(
        &self,
        status: EofCalibrationStatus,
        frame: Option<ViewportFrame>,
        tables_stable_for_width: bool,
        document: &Document,
    ) -> EofCalibrationProgress {
        EofCalibrationProgress {
            status,
            revision: self.revision,
            top_source: self.top_source(document),
            tables_stable_for_width,
            frame,
        }
    }

    pub fn set_viewport_rows(&mut self, rows: u16) -> Result<(), ViewportError> {
        if rows == 0 {
            return Err(ViewportError::InvalidConfig);
        }
        if let Some(end) = &self.end_position {
            self.end_position = Some(EndPosition::new(end.limit));
        }
        self.config.rows = rows;
        self.refined_top = false;
        self.render = None;
        self.pending_table_snapshots.clear();
        self.preparing_tables = false;
        self.calibration = None;
        Ok(())
    }

    /// Change the body content width and drop every width-dependent cache.
    ///
    /// The top source is captured before checkpoints are cleared. A non-zero
    /// estimate is installed for every tile, old height/checkpoint tokens are
    /// invalidated, and layout resumes from an approximate source anchor. This
    /// method is also used by the TUI sidebar-width simulation before P50 wires
    /// an actual key.
    pub fn set_content_width(
        &mut self,
        document: &Document,
        width: u16,
    ) -> Result<ContentWidthChange, ViewportError> {
        self.check_document(document)?;
        if width == 0 {
            return Err(ViewportError::InvalidConfig);
        }
        let previous_width = self.config.width;
        let top_source = self.top_source(document);
        if width == previous_width {
            return Ok(ContentWidthChange {
                previous_width,
                width,
                changed: false,
                top_source,
                estimated_rows: CONTENT_WIDTH_RESET_ESTIMATE,
            });
        }

        self.restart_end(document);
        let old_top = self.top;
        self.table_widths.resize_window(usize::from(width));
        self.summary = None;
        self.checkpoints
            .reset_width(document, CONTENT_WIDTH_RESET_ESTIMATE)?;
        self.render = None;
        self.pending_table_snapshots.clear();
        self.preparing_tables = false;
        self.calibration = None;
        self.refined_top = top_source.is_some();
        self.frame_source = None;
        self.config.width = width;
        if self.end_position.is_none() {
            self.fold_anchor = top_source;
            self.fold_reanchor = top_source.is_some();
            self.anchor_refiner = None;
        }

        if document.tile_count() == 0 {
            self.top = None;
            self.top_position = TopPosition::Empty;
        } else {
            let tile = old_top
                .map(|anchor| anchor.tile)
                .filter(|tile| document.tile(*tile).is_some())
                .or_else(|| top_source.and_then(|source| document.tile_at(source)))
                .unwrap_or(TileId(0));
            let source =
                top_source.unwrap_or_else(|| document.tile(tile).map_or(0, |span| span.start));
            let anchor = Anchor { tile, offset: 0 };
            self.top = Some(anchor);
            self.top_position = TopPosition::ApproximateLineStart { anchor, source };
        }

        Ok(ContentWidthChange {
            previous_width,
            width,
            changed: true,
            top_source,
            estimated_rows: CONTENT_WIDTH_RESET_ESTIMATE,
        })
    }

    pub fn set_overscan_rows(&mut self, rows: u64) -> Result<(), ViewportError> {
        self.config.overscan_rows = rows;
        self.render = None;
        self.pending_table_snapshots.clear();
        self.preparing_tables = false;
        self.calibration = None;
        Ok(())
    }

    pub fn down(&mut self, document: &Document) -> Result<bool, ViewportError> {
        self.scroll_down(document, 1)
    }

    pub fn up(&mut self, document: &Document) -> Result<bool, ViewportError> {
        self.scroll_up(document, 1)
    }

    pub fn page_down(&mut self, document: &Document) -> Result<bool, ViewportError> {
        self.scroll_down(document, u64::from(self.config.rows))
    }

    pub fn page_up(&mut self, document: &Document) -> Result<bool, ViewportError> {
        self.scroll_up(document, u64::from(self.config.rows))
    }

    pub fn home(&mut self, document: &Document) -> Result<bool, ViewportError> {
        self.check_document(document)?;
        if self.checkpoints.heights().total_rows() == 0 {
            self.top = None;
            self.top_position = TopPosition::Empty;
            self.render = None;
            self.pending_table_snapshots.clear();
            self.preparing_tables = false;
            return Ok(false);
        }
        self.set_top_row(document, 0)
    }

    pub fn end(&mut self, document: &Document) -> Result<bool, ViewportError> {
        self.check_document(document)?;
        let target = self.max_top_row();
        if self.checkpoints.heights().total_rows() == 0 {
            self.top = None;
            self.top_position = TopPosition::Empty;
            self.render = None;
            self.pending_table_snapshots.clear();
            self.preparing_tables = false;
            return Ok(false);
        }
        self.set_top_row(document, target)
    }

    pub fn scroll_down(&mut self, document: &Document, rows: u64) -> Result<bool, ViewportError> {
        self.fold_anchor = None;
        self.check_document(document)?;
        let current = self.top_row().unwrap_or(0);
        let target = current
            .saturating_add(u128::from(rows))
            .min(self.max_top_row());
        self.set_top_row(document, target)
    }

    pub fn scroll_up(&mut self, document: &Document, rows: u64) -> Result<bool, ViewportError> {
        self.fold_anchor = None;
        self.check_document(document)?;
        let current = self.top_row().unwrap_or(0);
        let target = current.saturating_sub(u128::from(rows));
        self.set_top_row(document, target)
    }

    pub(crate) fn refresh_fragment_titles(&mut self) {
        self.render = None;
        self.calibration = None;
    }

    pub fn set_top_from_refinement(
        &mut self,
        document: &Document,
        anchor: Anchor,
        line_start: usize,
        source: usize,
        exact: bool,
    ) -> Result<bool, ViewportError> {
        self.check_document(document)?;
        self.fold_reanchor = false;
        self.anchor_refiner = None;
        let moved = self.top != Some(anchor) || self.top_source(document) != Some(line_start);
        if !moved && document.math_object_stats().active == 0 {
            return Ok(false);
        }
        // Repeating a source jump can retain its row while still needing the
        // math rectangle's horizontal focus after a layout/cache refresh.
        self.top = Some(anchor);
        self.frame_source = Some((anchor, line_start));
        self.top_position = if exact {
            TopPosition::Exact {
                anchor,
                line_start: Some(line_start),
            }
        } else {
            TopPosition::ApproximateLineStart { anchor, source }
        };
        self.refined_top = true;
        self.render = None;
        self.pending_table_snapshots.clear();
        self.preparing_tables = false;
        self.calibration = None;
        Ok(moved)
    }

    pub fn frame(
        &mut self,
        document: &Document,
        budget: &mut Budget,
    ) -> Result<ViewportFrame, ViewportError> {
        self.check_document(document)?;
        if self.checkpoints.maintenance_in_progress() {
            return Err(ViewportError::CheckpointMaintenanceInProgress);
        }
        if self.fold_reanchor && self.end_position.is_none() {
            if let Some(source) = self.fold_anchor {
                if self.anchor_refiner.is_none() {
                    let anchor = mdtui3::FoldAnchor::new(document, source)?;
                    let Some(target) = anchor.resolve(
                        document,
                        self.checkpoints.view(),
                        u64::from(self.config.rows),
                        budget,
                    )?
                    else {
                        return Ok(self.end_pending(budget));
                    };
                    if let Some(source) = target.target.source
                        && !(self.top.is_some_and(|a| {
                            a.offset == 0
                                && document.tile(a.tile).is_some_and(|t| t.start == source)
                        }) && source == self.fold_anchor.unwrap_or(source))
                    {
                        let config = super::refine::RefineConfig {
                            width: self.config.width,
                            max_tiles: self.config.max_tiles,
                            max_replay_bytes: self.config.max_replay_bytes,
                            row_slack: self.config.row_slack,
                        };
                        if let Ok(mut refiner) = super::refine::SourceRefiner::new(
                            document,
                            self.checkpoints.heights(),
                            &self.checkpoints,
                            source,
                            config,
                        ) {
                            refiner.set_links(self.links.clone());
                            self.anchor_refiner = Some(Box::new(refiner));
                        }
                    }
                }
                if let Some(refiner) = &mut self.anchor_refiner {
                    match refiner.advance_owned(document, &mut self.checkpoints, budget) {
                        Ok(super::refine::RefineProgress::Ready(target)) => {
                            self.fold_reanchor = false;
                            self.set_top_from_refinement(
                                document,
                                target.anchor,
                                target.line_start,
                                target.source,
                                target.status != super::refine::RefineStatus::Approximate,
                            )?;
                        }
                        Ok(
                            super::refine::RefineProgress::PendingScan
                            | super::refine::RefineProgress::BudgetExhausted,
                        ) => return Ok(self.end_pending(budget)),
                        Ok(super::refine::RefineProgress::NotFound) | Err(_) => {}
                    }
                }
            }
            self.anchor_refiner = None;
            self.fold_reanchor = false;
        }
        if self.end_position.is_some() {
            return self.end_frame(document, budget);
        }
        if self.checkpoints.heights().total_rows() == 0 {
            return Ok(self.empty_frame());
        }
        let anchor = self.top.ok_or(ViewportError::InvalidTop)?;
        if self
            .render
            .as_ref()
            .is_some_and(|render| !render.matches_anchor(anchor, self.config))
        {
            self.render = None;
            self.pending_table_snapshots.clear();
            self.preparing_tables = false;
        }
        if self.render.is_none() {
            let preserve_refined_source = self.refined_top;
            let position = if self.refined_top {
                self.top_position
            } else {
                position_for(document, &self.checkpoints, anchor)
            };
            self.top_position = position;
            let Some(table_widths) =
                self.table_width_snapshots(document, anchor, position, budget)?
            else {
                self.preparing_tables = true;
                return Ok(ViewportFrame {
                    document: self.document,
                    revision: self.revision,
                    lines: Vec::new(),
                    top: self.top_position,
                    rows: 0,
                    status: ViewportFrameStatus::BudgetExhausted,
                    stats: ViewportFrameStats {
                        layout_budget: budget.used(),
                        ..ViewportFrameStats::default()
                    },
                });
            };
            self.preparing_tables = false;
            self.refined_top = false;
            self.render = Some(ViewportRender::new(
                document,
                self.checkpoints.heights(),
                &self.checkpoints,
                anchor,
                position,
                preserve_refined_source,
                self.config,
                table_widths,
                self.table_widths.policy(),
                &self.table_widths.column_positions(),
                self.summary.as_ref(),
                self.links.clone(),
            )?);
        }

        let completed = {
            let render = self.render.as_mut().ok_or(ViewportError::InvalidTop)?;
            render.advance(document, &mut self.checkpoints, budget)?
        };
        let Some(mut frame) = completed else {
            let render = self.render.as_ref().ok_or(ViewportError::InvalidTop)?;
            let mut stats = render.stats();
            stats.layout_budget = budget.used();
            return Ok(ViewportFrame {
                document: self.document,
                revision: self.revision,
                lines: Vec::new(),
                top: self.top_position,
                rows: 0,
                status: render.pending_status(),
                stats,
            });
        };

        let render = self.render.as_mut().ok_or(ViewportError::InvalidTop)?;
        if !super::first_frame::measure_zero_ranges(
            render.first.zero_ranges(),
            self.checkpoints.heights_mut(document)?,
            &mut render.zero_progress,
            budget,
        )? {
            return Ok(ViewportFrame {
                document: self.document,
                revision: self.revision,
                lines: Vec::new(),
                top: self.top_position,
                rows: 0,
                status: ViewportFrameStatus::BudgetExhausted,
                stats: render.stats(),
            });
        }
        frame.stats.layout_budget = budget.used();
        let render = self.render.take().ok_or(ViewportError::InvalidTop)?;
        self.shown_tables = render.first.sink().map_or_else(Vec::new, |sink| {
            sink.table_windows()
                .iter()
                .copied()
                .filter(|status| {
                    frame.lines.iter().any(|line| {
                        line.source
                            .and_then(|s| s.block)
                            .and_then(|b| document.block(b))
                            .is_some_and(|b| {
                                b.kind
                                    == mdtui3::BlockKind::Table {
                                        id: status.table_id,
                                    }
                            })
                    })
                })
                .collect()
        });
        self.shown_folds = render.first.placeholders().to_vec();
        let preserve_source = render.preserve_refined_source;
        // EOF table precision can rebuild this completed window again. Keep
        // the intrinsic math source goal until an actual scroll/jump replaces
        // it, rather than consuming it after the first successful paint.
        self.refined_top = preserve_source
            && math_focus_source(frame.top)
                .is_some_and(|source| render.first.scanner().math_at_source(source).is_some());

        self.top = Some(frame.top.anchor().unwrap_or(anchor));
        self.top_position = frame.top;
        if !preserve_source {
            self.frame_source = self.top.zip(
                frame
                    .lines
                    .first()
                    .and_then(|line| line.source)
                    .map(|line| line.source.start),
            );
        }
        self.normalize_top(document)?;
        self.links.request_retirement();
        Ok(frame)
    }

    fn set_top_row(&mut self, document: &Document, row: u128) -> Result<bool, ViewportError> {
        let Some(anchor) = self.checkpoints.heights().locate(row) else {
            return Ok(false);
        };
        if self.top == Some(anchor) {
            return Ok(false);
        }
        self.top = Some(anchor);
        self.frame_source = None;
        self.top_position = position_for(document, &self.checkpoints, anchor);
        self.refined_top = false;
        self.render = None;
        self.pending_table_snapshots.clear();
        self.preparing_tables = false;
        self.calibration = None;
        Ok(true)
    }

    fn max_top_row(&self) -> u128 {
        let total = self.checkpoints.heights().total_rows();
        total.saturating_sub(u128::from(self.config.rows))
    }

    fn normalize_top(&mut self, document: &Document) -> Result<(), ViewportError> {
        let Some(anchor) = self.top else {
            return Ok(());
        };
        let previous_top = self.top;
        let previous_position = self.top_position;
        let Some(height) = self.checkpoints.heights().height(anchor.tile) else {
            return Err(ViewportError::InvalidTop);
        };
        if height == 0 {
            let row = self
                .checkpoints
                .heights()
                .prefix_rows(anchor.tile.0)
                .unwrap_or(0);
            self.top = self.checkpoints.heights().locate(row);
        } else if anchor.offset >= height {
            let row = self
                .checkpoints
                .heights()
                .prefix_rows(anchor.tile.0)
                .unwrap_or(0)
                .saturating_add(u128::from(height - 1));
            self.top = self.checkpoints.heights().locate(row);
        }
        if let Some(anchor) = self.top {
            self.top_position =
                if previous_top == self.top && previous_position.anchor() == Some(anchor) {
                    previous_position
                } else {
                    position_for(document, &self.checkpoints, anchor)
                };
        } else {
            self.top_position = TopPosition::Empty;
        }
        Ok(())
    }

    fn check_document(&self, document: &Document) -> Result<(), ViewportError> {
        if self.matches(document) {
            Ok(())
        } else {
            Err(ViewportError::Stale)
        }
    }

    fn table_width_snapshots(
        &mut self,
        document: &Document,
        anchor: Anchor,
        position: TopPosition,
        budget: &mut Budget,
    ) -> Result<Option<HashMap<TableId, TableWidthSnapshot>>, ViewportError> {
        if self.checkpoints.view().has_collapsed() {
            return Ok(Some(HashMap::new()));
        }
        let ids = self.visible_table_ids(document, anchor, position);
        self.table_widths.retain_tables(&ids);
        self.pending_table_snapshots
            .retain(|id, _| ids.contains(id));
        for id in ids {
            if self.pending_table_snapshots.contains_key(&id) {
                continue;
            }
            let Some(snapshot) = self.table_widths.snapshot_budgeted(
                document,
                id,
                usize::from(self.config.width),
                budget,
            )?
            else {
                return Ok(None);
            };
            self.pending_table_snapshots.insert(id, snapshot);
        }
        Ok(Some(std::mem::take(&mut self.pending_table_snapshots)))
    }

    fn visible_table_ids(
        &self,
        document: &Document,
        anchor: Anchor,
        position: TopPosition,
    ) -> Vec<TableId> {
        let Some(start_row) = self.checkpoints.heights().row_of(anchor) else {
            return Vec::new();
        };
        let visible = self.checkpoints.heights().viewport(
            start_row..start_row.saturating_add(u128::from(self.config.rows)),
            self.config.overscan_rows,
            self.config.max_tiles,
        );
        if visible.is_empty() {
            return Vec::new();
        }
        let (start, end) = render_tile_range(document, anchor, position, &visible, self.config);
        if start >= end {
            return Vec::new();
        }
        let Some(first) = document.tile(start) else {
            return Vec::new();
        };
        let Some(last) = document.tile(TileId(end.0.saturating_sub(1))) else {
            return Vec::new();
        };
        let blocks = document.blocks();
        let block_start = blocks.partition_point(|block| block.source.end <= first.start);
        let block_end = blocks.partition_point(|block| block.source.start < last.end);
        blocks[block_start..block_end]
            .iter()
            .filter_map(|block| {
                if let mdtui3::BlockKind::Table { id } = block.kind {
                    Some(id)
                } else {
                    None
                }
            })
            .collect()
    }

    fn empty_frame(&self) -> ViewportFrame {
        ViewportFrame {
            document: self.document,
            revision: self.revision,
            lines: Vec::new(),
            top: TopPosition::Empty,
            rows: 0,
            status: ViewportFrameStatus::Ready,
            stats: ViewportFrameStats::default(),
        }
    }
}

fn validate_config(config: ViewportConfig) -> Result<(), ViewportError> {
    if config.width == 0 || config.rows == 0 || config.max_tiles == 0 {
        return Err(ViewportError::InvalidConfig);
    }
    Ok(())
}

fn position_for(
    document: &Document,
    checkpoints: &RenderCheckpointStore,
    anchor: Anchor,
) -> TopPosition {
    match checkpoints.plan_recovery(anchor.tile) {
        RecoveryPlan::Exact(tile) => TopPosition::Exact {
            anchor,
            line_start: checkpoints
                .exact(tile)
                .and_then(|checkpoint| checkpoint.line_source_start)
                .or_else(|| approximate_source(document, anchor)),
        },
        RecoveryPlan::ReplayFrom { from, target } => TopPosition::ReplayFrom {
            anchor,
            from,
            target,
        },
        RecoveryPlan::ApproximateLineStart(_) => TopPosition::ApproximateLineStart {
            anchor,
            source: approximate_source(document, anchor)
                .unwrap_or_else(|| document.tile(anchor.tile).map_or(0, |span| span.start)),
        },
    }
}

fn top_source_for(
    document: &Document,
    checkpoints: &RenderCheckpointStore,
    top: Option<Anchor>,
    position: TopPosition,
) -> Option<usize> {
    let anchor = top?;
    match position {
        TopPosition::Exact { line_start, .. } => {
            line_start.or_else(|| document.tile(anchor.tile).map(|span| span.start))
        }
        TopPosition::ApproximateLineStart { source, .. } => Some(source),
        TopPosition::ReplayFrom { from, .. } => checkpoints
            .exact(from)
            .and_then(|checkpoint| checkpoint.line_source_start)
            .or_else(|| document.tile(anchor.tile).map(|span| span.start)),
        TopPosition::Empty => document.tile(anchor.tile).map(|span| span.start),
    }
}

fn approximate_source(document: &Document, anchor: Anchor) -> Option<usize> {
    let tile = document.tile(anchor.tile)?;
    let block = document.block_at(tile.start)?;
    Some(document.block(block)?.source.start)
}

struct ViewportRender {
    zero_progress: super::first_frame::ZeroRangeProgress,
    document: DocumentId,
    revision: u64,
    anchor: Anchor,
    top: TopPosition,
    preserve_refined_source: bool,
    config: ViewportConfig,
    first: FirstFrame,
    visible_tiles: usize,
    replay_tiles: usize,
    replay_seen: bool,
    replay_enabled: bool,
}

impl ViewportRender {
    #[allow(clippy::too_many_arguments)]
    fn new(
        document: &Document,
        heights: &HeightIndex,
        checkpoints: &RenderCheckpointStore,
        anchor: Anchor,
        top: TopPosition,
        preserve_refined_source: bool,
        config: ViewportConfig,
        table_widths: HashMap<TableId, TableWidthSnapshot>,
        policy: super::table_widths::TableWidthPolicy,
        positions: &[(TableId, usize)],
        summary: Option<&super::fold_summary::FoldSummary>,
        links: crate::render::objects::SharedLinks,
    ) -> Result<Self, ViewportError> {
        let start_row = heights.row_of(anchor).ok_or(ViewportError::InvalidTop)?;
        let end_row = start_row.saturating_add(u128::from(config.rows));
        let visible = heights.viewport(start_row..end_row, config.overscan_rows, config.max_tiles);
        if visible.is_empty() {
            return Err(ViewportError::InvalidTop);
        }
        let (start, end) = render_tile_range(document, anchor, top, &visible, config);
        let planned_tiles = end.0.saturating_sub(start.0);
        let row_limit = if checkpoints.view().has_collapsed() {
            u64::from(config.rows)
                .saturating_add(config.overscan_rows.saturating_mul(2))
                .saturating_add(config.row_slack)
                .max(1)
        } else {
            u64::try_from(planned_tiles)
                .unwrap_or(u64::MAX)
                .saturating_mul(TILE_BYTES as u64 + 4)
                .saturating_add(config.row_slack)
                .max(1)
        };
        let mut first = FirstFrame::new_for_view(
            document,
            FirstFrameConfig {
                top_tile: start,
                tile_limit: if checkpoints.view().has_collapsed() {
                    visible
                        .len()
                        .saturating_add(2)
                        .max(
                            usize::from(config.rows)
                                .saturating_add(config.overscan_rows as usize * 2)
                                .saturating_mul(usize::from(config.width))
                                .saturating_mul(4)
                                .div_ceil(TILE_BYTES),
                        )
                        .max(anchor.tile.0.saturating_sub(start.0).saturating_add(1))
                        .min(config.max_tiles)
                        .max(1)
                } else {
                    planned_tiles.max(1)
                },
                width: config.width,
                row_limit,
            },
            checkpoints,
        )?;
        first.set_table_options(document, policy, positions)?;
        first.set_row_limit_anchor(anchor.tile);
        first.set_fold_summary(summary.cloned());
        first.set_table_view(super::table_window::TableView {
            anchor,
            tail: None,
            source: if preserve_refined_source {
                top_line_source(top).or_else(|| {
                    if document.math_object_stats().active > 0 {
                        math_focus_source(top)
                    } else {
                        None
                    }
                })
            } else {
                None
            },
            rows: usize::from(config.rows)
                .saturating_add(config.overscan_rows as usize * 2)
                .saturating_add(4),
        });
        first.set_links(links);
        if heights.height_state(anchor.tile) == Some(mdtui3::HeightState::Local)
            && let Some(source) = checkpoints
                .exact(anchor.tile)
                .and_then(|c| c.line_source_start)
            && document
                .tile(anchor.tile)
                .is_some_and(|tile| tile.start < source && source < tile.end)
            && document
                .block_at(source)
                .and_then(|id| document.block(id))
                .is_some_and(|block| {
                    matches!(
                        block.kind,
                        mdtui3::BlockKind::Paragraph | mdtui3::BlockKind::Quote { .. }
                    )
                })
        {
            first.set_cluster_floor(source);
        }
        first.set_table_widths(table_widths)?;
        let replay_enabled = match top {
            TopPosition::ReplayFrom { from, target, .. } => {
                checkpoints.exact(from).is_some_and(|checkpoint| {
                    replay_span_bytes(document, from, target)
                        .is_some_and(|bytes| bytes <= config.max_replay_bytes)
                        && replay_span_has_no_tui_owned(&first, from, target)
                        && first
                            .begin_replay(from, target, *checkpoint, config.max_replay_bytes)
                            .is_ok()
                })
            }
            _ => false,
        };
        let top = match top {
            TopPosition::ReplayFrom { from, .. } if !replay_enabled => {
                TopPosition::ApproximateLineStart {
                    anchor,
                    source: checkpoints
                        .exact(from)
                        .and_then(|checkpoint| checkpoint.line_source_start)
                        .or_else(|| approximate_source(document, anchor))
                        .unwrap_or_else(|| document.tile(anchor.tile).map_or(0, |span| span.start)),
                }
            }
            top => top,
        };
        let replay_tiles = match top {
            TopPosition::ReplayFrom { from, target, .. } => {
                replay_span_tiles(heights, from, target, config)
            }
            _ => 0,
        };
        Ok(Self {
            zero_progress: Default::default(),
            document: document.id(),
            revision: document.revision(),
            anchor,
            top,
            preserve_refined_source,
            config,
            first,
            visible_tiles: visible.len(),
            replay_tiles,
            replay_seen: false,
            replay_enabled,
        })
    }

    fn matches_anchor(&self, anchor: Anchor, config: ViewportConfig) -> bool {
        self.anchor == anchor && self.config == config
    }

    fn advance(
        &mut self,
        document: &Document,
        owner: &mut RenderCheckpointStore,
        budget: &mut Budget,
    ) -> Result<Option<ViewportFrame>, ViewportError> {
        if self.document != document.id() || self.revision != document.revision() {
            return Err(ViewportError::Stale);
        }
        let step = self.first.advance_using(document, owner, budget)?;
        if self.replay_enabled && self.first.replay_used() {
            self.replay_seen = true;
        }
        match step.status {
            FirstFrameStatus::BudgetExhausted | FirstFrameStatus::PendingScan => Ok(None),
            FirstFrameStatus::Covered | FirstFrameStatus::EndOfDocument => {
                if matches!(self.top, TopPosition::ReplayFrom { .. }) && !self.first.replay_used() {
                    self.top = TopPosition::ApproximateLineStart {
                        anchor: self.anchor,
                        source: approximate_source(document, self.anchor).unwrap_or(0),
                    };
                }
                self.finish_frame(document, step)
            }
        }
    }

    fn finish_frame(
        &self,
        document: &Document,
        step: FirstFrameStep,
    ) -> Result<Option<ViewportFrame>, ViewportError> {
        let target = self.anchor.tile;
        let sink = self.first.sink().ok_or(ViewportError::MissingLineSource)?;
        let Some((resolved, target_rows)) =
            resolve_target_tile(&self.first, sink, target, self.preserve_refined_source)
        else {
            if self.first.rows() == 0 && self.first.plan().tile_end.0 == document.tile_count() {
                return Ok(Some(ViewportFrame {
                    document: self.document,
                    revision: self.revision,
                    lines: Vec::new(),
                    top: TopPosition::Empty,
                    rows: 0,
                    status: ViewportFrameStatus::Ready,
                    stats: self.stats(),
                }));
            }
            return Err(ViewportError::IncompleteFrame);
        };
        let mut skip = 0_u64;
        for segment in self
            .first
            .plan()
            .segments
            .iter()
            .take_while(|segment| segment.tile < resolved)
        {
            if sink.tile_rendered(segment.tile) {
                skip = skip.saturating_add(sink.tile_rows(segment.tile).unwrap_or(0));
            }
        }
        let mut offset = self.anchor.offset.min(target_rows - 1);
        if self.preserve_refined_source
            && let Some(source) = math_focus_source(self.top)
            && let Some(view) = self.first.scanner().math_at_source(source)
            && let Some(row) = sink.math_row(view.meta.id, view.rect.y)
        {
            // Resize and same-tile source restoration must keep the queried
            // intrinsic row, including sparse rows omitted from the bitmap.
            offset = row.saturating_sub(skip).min(target_rows - 1);
        }
        skip = skip.saturating_add(offset);
        let mut lines = sink.visible_lines(skip, usize::from(self.config.rows));
        let mut top = with_anchor(
            self.top,
            Anchor {
                tile: resolved,
                offset,
            },
        );
        if self.preserve_refined_source
            && let Some(source) = top_line_source(self.top)
            && let Some(first) = lines.first_mut()
            && first
                .source
                .is_none_or(|current| current.source.start > source)
        {
            first.source = line_source_at(document, source);
        }
        if !self.preserve_refined_source
            && let Some(source) = lines
                .first()
                .and_then(|line| line.source)
                .map(|line| line.source.start)
            && let Some(anchor) = top.anchor()
        {
            top = match top {
                TopPosition::Exact { .. } => TopPosition::Exact {
                    anchor,
                    line_start: Some(source),
                },
                TopPosition::ReplayFrom { .. } => top,
                _ => TopPosition::ApproximateLineStart { anchor, source },
            };
        }
        let rows = u16::try_from(lines.len()).map_err(|_| ViewportError::RowOverflow)?;
        let status = if step.status == FirstFrameStatus::EndOfDocument {
            ViewportFrameStatus::EndOfDocument
        } else {
            ViewportFrameStatus::Ready
        };
        Ok(Some(ViewportFrame {
            document: self.document,
            revision: self.revision,
            lines,
            top,
            rows,
            status,
            stats: ViewportFrameStats {
                plan_start: Some(self.first.plan().tile_start),
                plan_end: Some(self.first.plan().tile_end),
                visible_tiles: self.visible_tiles,
                planned_tiles: self.first.plan().segments.len(),
                rendered_tiles: self.first.processed_tiles(),
                replay_tiles: if self.replay_seen {
                    self.replay_tiles
                } else {
                    0
                },
                skipped_rows: skip,
                layout_budget: WorkLimit::default(),
                layout: self.first.sink_stats().unwrap_or_default(),
            },
        }))
    }

    fn pending_status(&self) -> ViewportFrameStatus {
        if self.first.scan_complete() {
            ViewportFrameStatus::BudgetExhausted
        } else {
            ViewportFrameStatus::PendingScan
        }
    }

    fn stats(&self) -> ViewportFrameStats {
        ViewportFrameStats {
            plan_start: Some(self.first.plan().tile_start),
            plan_end: Some(self.first.plan().tile_end),
            visible_tiles: self.visible_tiles,
            planned_tiles: self.first.plan().segments.len(),
            rendered_tiles: self.first.processed_tiles(),
            replay_tiles: if self.replay_seen {
                self.replay_tiles
            } else {
                0
            },
            skipped_rows: 0,
            layout_budget: WorkLimit::default(),
            layout: self.first.sink_stats().unwrap_or_default(),
        }
    }
}

fn math_focus_source(top: TopPosition) -> Option<usize> {
    match top {
        TopPosition::Exact { line_start, .. } => line_start,
        TopPosition::ApproximateLineStart { source, .. } => Some(source),
        TopPosition::ReplayFrom { .. } | TopPosition::Empty => None,
    }
}

fn top_line_source(top: TopPosition) -> Option<usize> {
    match top {
        TopPosition::Exact { line_start, .. } => line_start,
        TopPosition::ReplayFrom { .. } | TopPosition::ApproximateLineStart { .. } => None,
        TopPosition::Empty => None,
    }
}

fn with_anchor(top: TopPosition, anchor: Anchor) -> TopPosition {
    match top {
        TopPosition::Empty => TopPosition::Empty,
        TopPosition::Exact { line_start, .. } => TopPosition::Exact { anchor, line_start },
        TopPosition::ReplayFrom { from, target, .. } => TopPosition::ReplayFrom {
            anchor,
            from,
            target: target.max(anchor.tile),
        },
        TopPosition::ApproximateLineStart { source, .. } => {
            TopPosition::ApproximateLineStart { anchor, source }
        }
    }
}

fn line_source_at(document: &Document, source: usize) -> Option<crate::screen::LineSource> {
    let block = document.block_at(source)?;
    let block_source = document.block(block)?.source;
    let end = source.saturating_add(1).min(block_source.end);
    Some(crate::screen::LineSource::new(
        Span::new(source, end),
        block,
    ))
}

fn replay_span_tiles(
    heights: &HeightIndex,
    from: TileId,
    target: TileId,
    config: ViewportConfig,
) -> usize {
    let end = target.0.saturating_add(1).min(heights.len());
    end.saturating_sub(from.0).min(config.max_tiles)
}

fn replay_span_bytes(document: &Document, from: TileId, target: TileId) -> Option<usize> {
    let from = document.tile(from)?;
    let target = document.tile(target)?;
    Some(target.start.saturating_sub(from.start))
}

fn replay_span_has_no_tui_owned(first: &FirstFrame, from: TileId, target: TileId) -> bool {
    let plan = first.plan();
    let start = from.0.saturating_sub(plan.tile_start.0);
    let end = target
        .0
        .saturating_sub(plan.tile_start.0)
        .saturating_add(1)
        .min(plan.segments.len());
    plan.segments[start.min(end)..end]
        .iter()
        .all(|segment| segment.kind != TileSegmentKind::TuiOwned)
}

fn resolve_target_tile(
    first: &FirstFrame,
    sink: &crate::render::sink::TileLayoutSink,
    target: TileId,
    normalize_crossing: bool,
) -> Option<(TileId, u64)> {
    let plan = first.plan();
    let end = plan.segments.len();
    if normalize_crossing
        && let Some(target_source) = plan
            .segments
            .iter()
            .find(|s| s.tile == target)
            .map(|s| s.source.start)
        && let Some(glyph) = first.scanner().segments().first().filter(|g| {
            g.text.is_indexed() && g.source.start < target_source && target_source < g.source.end
        })
        && let Some(owner) = plan
            .segments
            .iter()
            .find(|s| s.source.start <= glyph.source.start && glyph.source.start < s.source.end)
        && let Some(rows) = sink.tile_rows(owner.tile).filter(|&rows| rows > 0)
    {
        return Some((owner.tile, rows));
    }
    let start = plan
        .segments
        .partition_point(|segment| segment.tile < target)
        .min(end);
    if let Some((segment, rows)) = plan.segments.get(start..end)?.iter().find_map(|segment| {
        sink.tile_rows(segment.tile)
            .filter(|rows| *rows > 0)
            .map(|rows| (segment.tile, rows))
    }) {
        return Some((segment, rows));
    }
    plan.segments
        .get(..start)?
        .iter()
        .rev()
        .find_map(|segment| {
            sink.tile_rows(segment.tile)
                .filter(|rows| *rows > 0)
                .map(|rows| (segment.tile, rows))
        })
}

fn render_tile_range(
    document: &Document,
    anchor: Anchor,
    top: TopPosition,
    visible: &[mdtui3::VisibleTile],
    config: ViewportConfig,
) -> (TileId, TileId) {
    let visible_start = visible.first().map_or(anchor.tile, |tile| tile.tile);
    let visible_end = visible
        .last()
        .map_or(anchor.tile.0.saturating_add(1), |tile| tile.tile.0 + 1);
    let mut start = visible_start;
    if let Some(recovery) = recovery_start_tile(document, top) {
        start = recovery.min(start);
    }
    start = block_start_tile(document, start).min(start);

    let mut end = visible_end.min(document.tile_count());
    // Backward overscan/recovery must leave room for the requested tile in
    // the bounded plan, including when projection counts visible tiles.
    let earliest = anchor
        .tile
        .0
        .saturating_sub(config.max_tiles.saturating_sub(1));
    if start.0 < earliest {
        start = TileId(earliest);
    }
    let limit = start
        .0
        .saturating_add(config.max_tiles)
        .min(document.tile_count());
    expand_block_end(document, start, &mut end, limit);

    (start, TileId(end.max(start.0.saturating_add(1))))
}

fn recovery_start_tile(document: &Document, top: TopPosition) -> Option<TileId> {
    match top {
        TopPosition::Exact { line_start, .. } => {
            line_start.and_then(|source| document.tile_at(source))
        }
        TopPosition::ReplayFrom { from, .. } => Some(from),
        TopPosition::ApproximateLineStart { source, .. } => document.tile_at(source),
        TopPosition::Empty => None,
    }
}

pub(crate) fn block_start_tile(document: &Document, tile: TileId) -> TileId {
    let Some(span) = document.tile(tile) else {
        return tile;
    };
    let Some(block_id) = document.block_at(span.start) else {
        return tile;
    };
    let Some(block) = document.block(block_id) else {
        return tile;
    };
    let mut source_start = block.source.start;
    if let Some(container) = block.container
        && let Some(container) = document.container(container)
    {
        source_start = source_start.min(container.source.start);
    }
    document.tile_at(source_start).unwrap_or(tile)
}

pub(crate) fn expand_block_end(document: &Document, start: TileId, end: &mut usize, limit: usize) {
    let Some(start_span) = document.tile(start) else {
        return;
    };
    let mut source_end = document
        .tile(TileId(end.saturating_sub(1)))
        .map_or(start_span.end, |span| span.end);
    loop {
        let blocks = document.blocks();
        let first = blocks.partition_point(|block| block.source.end <= start_span.start);
        let last = blocks.partition_point(|block| block.source.start < source_end);
        let mut changed = false;
        for block in &blocks[first..last] {
            let mut block_end = block.source.end;
            if let Some(container) = block.container
                && let Some(container) = document.container(container)
            {
                block_end = block_end.max(container.source.end);
            }
            if let Some(tile) = document.tile_at(block_end.saturating_sub(1)) {
                let next = tile.0.saturating_add(1).min(limit);
                if next > *end {
                    *end = next;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
        source_end = document
            .tile(TileId(end.saturating_sub(1)))
            .map_or(source_end, |span| span.end);
    }
}
