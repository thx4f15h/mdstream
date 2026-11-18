use std::collections::HashMap;
use std::fmt;

use mdtui3::{
    Budget, Completion, Document, DocumentId, HeightIndex, LayoutError, LayoutPass, Span, TileId,
    WorkLimit,
};

use crate::render::checkpoint::{RenderCheckpoint, RenderCheckpointStore};
use crate::render::context::WindowContexts;
use crate::render::display::{DisplayScanner, ScanStatus};
use crate::render::plan::{BlockPlan, PlanError, TileSegmentKind};
use crate::render::sink::{LayoutSinkStats, SinkError, TileLayoutSink};
use crate::render::table_widths::{TableWidthCache, TableWidthError, TableWidthSnapshot};
use crate::screen::Screen;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FirstFrameConfig {
    pub top_tile: TileId,
    pub tile_limit: usize,
    pub width: u16,
    pub row_limit: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirstFrameStatus {
    PendingScan,
    BudgetExhausted,
    Covered,
    EndOfDocument,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FirstFrameStep {
    pub status: FirstFrameStatus,
    pub rows: u64,
    pub processed_tiles: usize,
    pub layout_attempts: usize,
    pub direct_tiles: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FirstFrameError {
    InvalidConfig,
    Stale,
    InvalidTop,
    Plan(PlanError),
    Sink(SinkError),
    Layout(LayoutError),
    MissingSink,
    InvalidReplay,
    TableWidthsLocked,
    TableWidth(TableWidthError),
    TileRowsMismatch {
        tile: TileId,
        expected: u64,
        actual: u64,
    },
    UnexpectedCompletion(Completion),
}

impl fmt::Display for FirstFrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig => formatter.write_str("invalid first-frame configuration"),
            Self::Stale => formatter.write_str("first-frame progress is stale"),
            Self::InvalidTop => formatter.write_str("top tile is outside the document"),
            Self::Plan(error) => error.fmt(formatter),
            Self::Sink(error) => error.fmt(formatter),
            Self::Layout(error) => error.fmt(formatter),
            Self::MissingSink => formatter.write_str("layout sink is not ready"),
            Self::InvalidReplay => formatter.write_str("invalid replay range"),
            Self::TableWidth(error) => error.fmt(formatter),
            Self::TableWidthsLocked => {
                formatter.write_str("table widths must be set before layout starts")
            }
            Self::TileRowsMismatch {
                tile,
                expected,
                actual,
            } => write!(
                formatter,
                "tile {} height mismatch: expected {expected}, got {actual}",
                tile.0
            ),
            Self::UnexpectedCompletion(completion) => {
                write!(formatter, "unexpected layout completion: {completion:?}")
            }
        }
    }
}

impl std::error::Error for FirstFrameError {}

impl From<PlanError> for FirstFrameError {
    fn from(error: PlanError) -> Self {
        Self::Plan(error)
    }
}

impl From<SinkError> for FirstFrameError {
    fn from(error: SinkError) -> Self {
        Self::Sink(error)
    }
}

impl From<LayoutError> for FirstFrameError {
    fn from(error: LayoutError) -> Self {
        Self::Layout(error)
    }
}

impl From<TableWidthError> for FirstFrameError {
    fn from(error: TableWidthError) -> Self {
        Self::TableWidth(error)
    }
}

#[derive(Default)]
pub(crate) struct ZeroRangeProgress {
    at: usize,
    ticket: Option<mdtui3::ZeroHeightMeasurement>,
}
pub(crate) fn measure_zero_ranges(
    ranges: &[std::ops::Range<usize>],
    heights: &mut HeightIndex,
    progress: &mut ZeroRangeProgress,
    budget: &mut Budget,
) -> Result<bool, LayoutError> {
    while let Some(range) = ranges.get(progress.at) {
        if progress.ticket.is_none() {
            if !budget.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            }) {
                return Ok(false);
            }
            progress.ticket = Some(
                heights
                    .zero_height_ticket(range.clone())
                    .ok_or(LayoutError::InvalidRange)?,
            );
        }
        if heights.measure_zero_height(progress.ticket.unwrap(), false, budget)?
            == Completion::BudgetExhausted
        {
            return Ok(false);
        }
        progress.ticket = None;
        progress.at += 1;
    }
    Ok(true)
}

pub struct FirstFrame {
    binding: Option<mdtui3::FoldProjection>,
    projection_cut: Option<usize>,
    summary: Option<super::fold_summary::FoldSummary>,
    projected: bool,
    measurements: bool,
    projection: Option<mdtui3::FoldProjection>,
    projection_done: bool,
    projection_geometry_done: bool,
    fragment_contexts: HashMap<usize, Span>,
    fragment_context_cursor: usize,
    projected_tiles: usize,
    projected_last_tile: Option<TileId>,
    projected_end: usize,
    projection_edge: Option<ProjectionEdge>,
    code_jump: Option<(mdtui3::BlockId, usize, Option<usize>)>,
    last_code_jump: Option<mdtui3::BlockId>,
    block_jump: Option<(mdtui3::BlockId, usize)>,
    fragments: Vec<Span>,
    placeholders: Vec<mdtui3::FoldPlaceholder>,
    final_zero_progress: ZeroRangeProgress,
    final_zero_ranges: Vec<std::ops::Range<usize>>,
    final_zero_ready: bool,
    nonwrap_ready: bool,
    placeholders_inserted: bool,
    relocated: bool,
    width_cache: TableWidthCache,
    code_prefix_metadata_reads: usize,
    code_prefix_resolution: mdtui3::LinkResolutionWork,
    document: DocumentId,
    revision: u64,
    config: FirstFrameConfig,
    plan: BlockPlan,
    plan_ready: bool,
    promoted_code: Option<mdtui3::BlockId>,
    skipped_code_prefix: Option<std::ops::Range<usize>>,
    contexts: WindowContexts,
    scanner: DisplayScanner,
    table_window: Option<super::table_window::TableWindow>,
    table_view: Option<super::table_window::TableView>,
    selection_ready: bool,
    table_view_checked: bool,
    math_view_checked: bool,
    math_copy_view: Option<super::table_window::TableView>,
    scan_complete: bool,
    sink: Option<TileLayoutSink>,
    pass: Option<LayoutPass>,
    checkpoints: RenderCheckpointStore,
    cursor: usize,
    rows: u64,
    row_limit_anchor: Option<TileId>,
    leading_rows: u64,
    layout_attempts: usize,
    direct_tiles: usize,
    processed_tiles: usize,
    replay: Option<ReplayRequest>,
    replay_pass: Option<LayoutPass>,
    replay_used: bool,
    table_widths: HashMap<mdtui3::TableId, TableWidthSnapshot>,
    suppressed_heading_tiles: Vec<std::ops::Range<usize>>,
}

struct ProjectionEdge {
    source: Span,
    key: usize,
    context: Option<Span>,
    window: Option<Box<super::shape::ShapeWindow>>,
}

#[derive(Debug, Clone, Copy)]
struct ReplayRequest {
    from: TileId,
    target: TileId,
    checkpoint: RenderCheckpoint,
    max_bytes: usize,
}

impl FirstFrame {
    pub fn new(document: &Document, config: FirstFrameConfig) -> Result<Self, FirstFrameError> {
        Self::new_with_owner(document, config, true)
    }

    pub(crate) fn new_borrowed(
        document: &Document,
        config: FirstFrameConfig,
    ) -> Result<Self, FirstFrameError> {
        Self::new_with_owner(document, config, false)
    }

    fn new_with_owner(
        document: &Document,
        config: FirstFrameConfig,
        owned: bool,
    ) -> Result<Self, FirstFrameError> {
        if config.tile_limit == 0 || config.width == 0 || config.row_limit == 0 {
            return Err(FirstFrameError::InvalidConfig);
        }
        if config.top_tile.0 > document.tile_count() {
            return Err(FirstFrameError::InvalidTop);
        }
        let original_start = config.top_tile.0;
        // Confirmed fence markers and info are indexed hidden spans. Jump over
        // a long hidden prefix using metadata, while keeping its bounded label
        // in the first body tile. Never read the hidden line to find the body.
        let mut code_prefix_metadata_reads = 0;
        let mut code_prefix_resolution = mdtui3::LinkResolutionWork::default();
        let promoted = document
            .tile(config.top_tile)
            .and_then(|span| document.block_at(span.start))
            .and_then(|id| document.block(id).map(|block| (id, block)))
            .filter(|(_, block)| matches!(block.kind, mdtui3::BlockKind::Code { .. }))
            .and_then(|(id, block)| {
                // A confirmed code span has unconditional literal annotations.
                // Limit the query before reading, including an empty body: a
                // following paragraph must never be evaluated by this probe.
                let mut runs = document.runs_in(block.source)?;
                let drawable = runs
                    .by_ref()
                    .take(3)
                    .inspect(|_| code_prefix_metadata_reads += 1)
                    .find(|run| !run.style.contains(mdtui3::Style::HIDDEN))
                    .map(|run| run.source.start);
                code_prefix_resolution = runs.link_resolution_work();
                let at = drawable.unwrap_or_else(|| block.source.end.saturating_sub(1));
                let tile = document.tile_at(at)?;
                (tile.0 >= original_start.saturating_add(config.tile_limit)).then_some((id, tile.0))
            });
        let start = promoted.map_or(original_start, |(_, tile)| tile);
        let promoted_code = promoted.map(|(id, _)| id);
        let skipped_code_prefix = (start > original_start).then_some(original_start..start);
        let end = start
            .saturating_add(config.tile_limit)
            .min(document.tile_count());
        let source = if start == end {
            Span::new(document.source().len(), document.source().len())
        } else {
            let first = document
                .tile(TileId(start))
                .ok_or(FirstFrameError::InvalidTop)?;
            let last = document
                .tile(TileId(end - 1))
                .ok_or(FirstFrameError::InvalidTop)?;
            Span::new(first.start, last.end)
        };
        let contexts = WindowContexts::new(document, source);
        let plan_ready = contexts.is_empty();
        let plan = if plan_ready {
            crate::render::plan::plan_with_contexts(
                document,
                source,
                &contexts.entries,
                promoted_code,
            )?
        } else {
            crate::render::plan::empty_plan(document, source, start, end)
        };
        let mut scanner =
            DisplayScanner::new(document, source).ok_or(FirstFrameError::InvalidTop)?;
        scanner.set_math_rect(super::math::Rect {
            x: 0,
            y: 0,
            width: usize::from(config.width),
            height: usize::try_from(config.row_limit).unwrap_or(usize::MAX),
        });
        let suppressed_heading_tiles = Vec::new();
        Ok(Self {
            binding: None,
            projection_cut: None,
            summary: None,
            projected: false,
            measurements: true,
            projection: None,
            projection_done: true,
            projection_geometry_done: true,
            fragment_contexts: HashMap::new(),
            fragment_context_cursor: 0,
            projected_tiles: 0,
            projected_last_tile: None,
            projected_end: 0,
            projection_edge: None,
            code_jump: None,
            last_code_jump: None,
            block_jump: None,
            fragments: Vec::new(),
            placeholders: Vec::new(),
            final_zero_progress: Default::default(),
            final_zero_ranges: Vec::new(),
            final_zero_ready: false,
            nonwrap_ready: false,
            placeholders_inserted: false,
            relocated: false,
            width_cache: TableWidthCache::new(document),
            code_prefix_metadata_reads,
            code_prefix_resolution,
            document: document.id(),
            revision: document.revision(),
            config,
            plan,
            plan_ready,
            promoted_code,
            skipped_code_prefix,
            contexts,
            scanner,
            table_window: None,
            table_view: None,
            selection_ready: false,
            table_view_checked: false,
            math_view_checked: false,
            math_copy_view: None,
            scan_complete: false,
            sink: None,
            pass: None,
            checkpoints: if owned {
                RenderCheckpointStore::new(document)
            } else {
                RenderCheckpointStore::borrowed()
            },
            cursor: 0,
            rows: 0,
            row_limit_anchor: None,
            leading_rows: 0,
            layout_attempts: 0,
            direct_tiles: 0,
            processed_tiles: 0,
            replay: None,
            replay_pass: None,
            replay_used: false,
            table_widths: HashMap::new(),
            suppressed_heading_tiles,
        })
    }

    pub(crate) fn set_math_floor(&mut self, source: usize) {
        self.scanner.set_math_floor(source);
    }
    pub(crate) fn new_for_view(
        document: &Document,
        config: FirstFrameConfig,
        owner: &RenderCheckpointStore,
    ) -> Result<Self, FirstFrameError> {
        if owner.maintenance_in_progress() {
            return Err(FirstFrameError::Layout(LayoutError::OutOfOrderUpdate));
        }
        let mut result = if owner.view().has_collapsed() {
            Self::deferred(document, config)?
        } else {
            Self::new_borrowed(document, config)?
        };
        result.binding = Some(mdtui3::FoldProjection::new(
            document,
            owner.view(),
            Span::new(0, 0),
        )?);
        Ok(result)
    }

    fn deferred(document: &Document, config: FirstFrameConfig) -> Result<Self, FirstFrameError> {
        if config.tile_limit == 0 || config.width == 0 || config.row_limit == 0 {
            return Err(FirstFrameError::InvalidConfig);
        }
        let start = document
            .tile(config.top_tile)
            .map_or(document.source().len(), |s| s.start);
        let source = Span::new(start, document.source().len());
        let empty = Span::new(start, start);
        Ok(Self {
            binding: None,
            projection_cut: None,
            summary: None,
            projected: true,
            measurements: true,
            projection: None,
            projection_done: false,
            projection_geometry_done: false,
            fragment_contexts: HashMap::new(),
            fragment_context_cursor: 0,
            projected_tiles: 0,
            projected_last_tile: None,
            projected_end: 0,
            projection_edge: None,
            code_jump: None,
            last_code_jump: None,
            block_jump: None,
            fragments: Vec::new(),
            placeholders: Vec::new(),
            final_zero_progress: Default::default(),
            final_zero_ranges: Vec::new(),
            final_zero_ready: false,
            nonwrap_ready: false,
            placeholders_inserted: false,
            relocated: true,
            width_cache: TableWidthCache::new(document),
            code_prefix_metadata_reads: 0,
            code_prefix_resolution: Default::default(),
            document: document.id(),
            revision: document.revision(),
            config,
            plan: crate::render::plan::empty_plan(document, source, 0, 0),
            plan_ready: false,
            promoted_code: None,
            skipped_code_prefix: None,
            contexts: WindowContexts::new(document, empty),
            scanner: DisplayScanner::exact(
                document,
                empty,
                crate::render::objects::SharedLinks::new(document),
            )
            .ok_or(FirstFrameError::InvalidTop)?,
            table_window: None,
            table_view: None,
            selection_ready: false,
            table_view_checked: false,
            math_view_checked: false,
            math_copy_view: None,
            scan_complete: false,
            sink: None,
            pass: None,
            checkpoints: RenderCheckpointStore::borrowed(),
            cursor: 0,
            rows: 0,
            row_limit_anchor: None,
            leading_rows: 0,
            layout_attempts: 0,
            direct_tiles: 0,
            processed_tiles: 0,
            replay: None,
            replay_pass: None,
            replay_used: false,
            table_widths: HashMap::new(),
            suppressed_heading_tiles: Vec::new(),
        })
    }

    pub(crate) fn set_fold_summary(&mut self, summary: Option<super::fold_summary::FoldSummary>) {
        self.summary = summary;
    }
    /// Recovery/overscan rows precede the requested viewport. They must not
    /// consume its row allowance before the target tile has been measured.
    pub(crate) fn set_row_limit_anchor(&mut self, tile: TileId) {
        self.row_limit_anchor = Some(tile);
    }
    pub(crate) fn prepare_only(&mut self) {
        self.measurements = false;
    }
    pub(crate) fn set_projection_range(&mut self, source: Span) {
        if self.projected && self.projection.is_none() {
            self.plan.visible = source;
            self.projection_cut = Some(source.end);
        }
    }
    pub fn placeholders(&self) -> &[mdtui3::FoldPlaceholder] {
        &self.placeholders
    }

    fn accept_projected_tile(&mut self, tile: TileId) -> bool {
        if self.projected_last_tile != Some(tile) {
            if self.projected_tiles >= self.config.tile_limit {
                return false;
            }
            self.projected_tiles += 1;
            self.projected_last_tile = Some(tile);
        }
        true
    }
    fn advance_projection_edge(
        &mut self,
        document: &Document,
        edge: &mut ProjectionEdge,
        budget: &mut Budget,
    ) -> Result<bool, FirstFrameError> {
        let links = self.scanner.shared_links();
        if edge.context.is_none() {
            if let Some(context) = self.fragment_contexts.get(&edge.key) {
                edge.context = Some(*context);
            } else {
                let Some(context) = self.checkpoints.view().visible_fragment(
                    document,
                    edge.source.start,
                    budget,
                )?
                else {
                    return Ok(false);
                };
                self.fragment_contexts.insert(edge.key, context);
                edge.context = Some(context);
            }
        }
        let context = edge.context.unwrap();
        if context.end <= edge.source.end {
            return Ok(true);
        }
        if !links.prepare_shape(document, budget)? || !links.prepare(document, budget)? {
            return Ok(false);
        }
        let shape = links.shape().ok_or(FirstFrameError::InvalidTop)?;
        if edge.window.is_none() {
            if !budget.charge(WorkLimit {
                queries: 1,
                objects: 1,
                bytes: 4,
                ..Default::default()
            }) {
                return Ok(false);
            }
            let mut probe = edge.source.end - 1;
            while !document.source().is_char_boundary(probe) {
                probe -= 1;
            }
            let block = document
                .block_at(probe)
                .and_then(|id| document.block(id).map(|block| (id, block)));
            if let Some((id, block)) = block {
                if matches!(
                    block.kind,
                    mdtui3::BlockKind::Table { .. }
                        | mdtui3::BlockKind::Math { .. }
                        | mdtui3::BlockKind::Definition { .. }
                        | mdtui3::BlockKind::FootnoteDefinition { .. }
                ) {
                    self.block_jump = Some((id, probe));
                    return Ok(true);
                }
                if matches!(block.kind, mdtui3::BlockKind::Code { .. })
                    && self.last_code_jump != Some(id)
                {
                    self.last_code_jump = Some(id);
                    self.code_jump = Some((id, probe, None));
                    return Ok(true);
                }
            }
            let block = block.map_or(document.pending(), |(_, block)| block.source);
            let fragment = Span::new(
                context.start.max(block.start),
                context
                    .end
                    .min(block.end)
                    .min(self.projection_cut.unwrap_or(usize::MAX)),
            );
            if fragment.end <= edge.source.end || !shape.requires_shape(fragment) {
                return Ok(true);
            }
            edge.window = Some(Box::new(super::shape::ShapeWindow::new(
                &shape,
                Span::new(probe, edge.source.end),
                fragment,
            )?));
        }
        match edge
            .window
            .as_mut()
            .unwrap()
            .advance(&shape, document, &links, budget)?
        {
            super::shape::ShapeWindowStep::BudgetExhausted => Ok(false),
            super::shape::ShapeWindowStep::Complete => Ok(true),
            super::shape::ShapeWindowStep::Glyph(glyph) => {
                let end = self.projection.as_ref().unwrap().requested().end;
                let target = glyph.source.end.min(context.end).min(end);
                if target > self.projection.as_ref().unwrap().source_position() {
                    self.projection = Some(mdtui3::FoldProjection::new(
                        document,
                        self.checkpoints.view(),
                        Span::new(target, end),
                    )?);
                    self.projected_end = target;
                }
                Ok(true)
            }
        }
    }

    fn project(
        &mut self,
        document: &Document,
        budget: &mut Budget,
    ) -> Result<bool, FirstFrameError> {
        use mdtui3::FoldProjectionStep;
        if self.projection_done {
            return Ok(true);
        }
        if self.projection.is_none() {
            self.projected_end = self.plan.visible.start;
            self.projection = Some(mdtui3::FoldProjection::new(
                document,
                self.checkpoints.view(),
                self.plan.visible,
            )?);
        }
        while !self.projection_geometry_done {
            if let Some(mut edge) = self.projection_edge.take()
                && !self.advance_projection_edge(document, &mut edge, budget)?
            {
                self.projection_edge = Some(edge);
                return Ok(false);
            }
            if let Some((block_id, source)) = self.block_jump {
                let Some(context) = self
                    .checkpoints
                    .view()
                    .visible_fragment(document, source, budget)?
                else {
                    return Ok(false);
                };
                let end = self.projection.as_ref().unwrap().requested().end;
                let target = context
                    .end
                    .min(
                        document
                            .block(block_id)
                            .ok_or(FirstFrameError::InvalidTop)?
                            .source
                            .end,
                    )
                    .min(end);
                if target > self.projection.as_ref().unwrap().source_position() {
                    self.projection = Some(mdtui3::FoldProjection::new(
                        document,
                        self.checkpoints.view(),
                        Span::new(target, end),
                    )?);
                    self.projected_end = target;
                }
                self.block_jump = None;
            }
            if let Some((block_id, source, mut drawable)) = self.code_jump {
                if drawable.is_none() {
                    if !budget.charge(WorkLimit {
                        queries: 1,
                        objects: 1,
                        ..Default::default()
                    }) {
                        return Ok(false);
                    }
                    let block = document
                        .block(block_id)
                        .ok_or(FirstFrameError::InvalidTop)?;
                    let mut runs = document
                        .runs_in(block.source)
                        .ok_or(FirstFrameError::InvalidTop)?;
                    let mut at = None;
                    for run in runs.by_ref().take(3) {
                        self.code_prefix_metadata_reads += 1;
                        if !run.style.contains(mdtui3::Style::HIDDEN) {
                            at = Some(run.source.start);
                            break;
                        }
                    }
                    self.code_prefix_resolution = runs.link_resolution_work();
                    drawable = Some(at.unwrap_or(block.source.end));
                    self.code_jump = Some((block_id, source, drawable));
                }
                let Some(context) = self
                    .checkpoints
                    .view()
                    .visible_fragment(document, source, budget)?
                else {
                    return Ok(false);
                };
                let end = self.projection.as_ref().unwrap().requested().end;
                let target = drawable.unwrap().min(context.end).min(end);
                if target > self.projection.as_ref().unwrap().source_position() {
                    self.projection = Some(mdtui3::FoldProjection::new(
                        document,
                        self.checkpoints.view(),
                        Span::new(target, end),
                    )?);
                    self.projected_end = target;
                }
                self.code_jump = None;
            }
            match self.projection.as_mut().unwrap().next(
                document,
                self.checkpoints.view(),
                budget,
            )? {
                FoldProjectionStep::Text { source, tile } => {
                    if !self.accept_projected_tile(tile) {
                        self.projected_end = source.start;
                        self.projection_geometry_done = true;
                        break;
                    }
                    if let Some(last) = self.fragments.last_mut().filter(|s| s.end == source.start)
                    {
                        last.end = source.end;
                    } else {
                        self.fragments.push(source);
                    }
                    if let Some(block_id) = document.block_at(source.start)
                        && document.block(block_id).is_some_and(|block| {
                            matches!(block.kind, mdtui3::BlockKind::Table { .. })
                        })
                    {
                        self.block_jump = Some((block_id, source.start));
                    }
                    if let Some(block_id) = document.block_at(source.start)
                        && let Some(block) = document.block(block_id)
                        && matches!(block.kind, mdtui3::BlockKind::Code { .. })
                        && self.last_code_jump != Some(block_id)
                    {
                        self.code_jump = Some((block_id, source.start, None));
                        self.last_code_jump = Some(block_id);
                    }
                    if self.block_jump.is_none() && self.code_jump.is_none() {
                        self.projection_edge = Some(ProjectionEdge {
                            source,
                            key: self.fragments.last().unwrap().start,
                            context: None,
                            window: None,
                        });
                    }
                }
                FoldProjectionStep::Placeholder(placeholder) => {
                    if !self.accept_projected_tile(placeholder.owner) {
                        self.projected_end = placeholder.candidate.hidden.start;
                        self.projection_geometry_done = true;
                        break;
                    }
                    self.placeholders.push(placeholder);
                }
                FoldProjectionStep::Hidden { source, .. } => {
                    if let Some(last) = self.placeholders.last_mut() {
                        last.candidate.hidden.end = last.candidate.hidden.end.max(source.end);
                    }
                }
                FoldProjectionStep::ParagraphEnd(_) => {}
                FoldProjectionStep::Complete => {
                    self.projected_end = self.projection.as_ref().unwrap().source_position();
                    self.projection_geometry_done = true;
                    break;
                }
                FoldProjectionStep::BudgetExhausted => return Ok(false),
            }
            self.projected_end = self.projection.as_ref().unwrap().source_position();
        }
        while let Some(source) = self.fragments.get(self.fragment_context_cursor) {
            if self.fragment_contexts.contains_key(&source.start) {
                self.fragment_context_cursor += 1;
                continue;
            }
            let Some(context) =
                self.checkpoints
                    .view()
                    .visible_fragment(document, source.start, budget)?
            else {
                return Ok(false);
            };
            self.fragment_contexts.insert(source.start, context);
            self.fragment_context_cursor += 1;
        }
        self.plan.visible.end = self.projected_end;
        self.contexts = WindowContexts::fragments(document, &self.fragments);
        let math_rect = self.scanner.math_rect();
        let math_floor = self.scanner.math_floor();
        self.scanner = DisplayScanner::projected(
            document,
            self.plan.visible,
            &self.fragments,
            &self.fragment_contexts,
            self.scanner.shared_links(),
        )
        .ok_or(FirstFrameError::InvalidTop)?;
        self.scanner.set_math_rect(math_rect);
        self.scanner.set_math_floor(math_floor);
        self.scanner.set_source_cut(
            self.projection_cut
                .or_else(|| self.table_view.and_then(|view| view.tail)),
        );
        self.projection_done = true;
        Ok(true)
    }

    pub(crate) fn set_cluster_floor(&mut self, source: usize) {
        self.scanner.set_cluster_floor(source);
    }
    pub(crate) fn zero_ranges(&self) -> &[std::ops::Range<usize>] {
        self.scanner.zero_ranges()
    }
    pub fn code_prefix_work(&self) -> (usize, mdtui3::LinkResolutionWork) {
        (self.code_prefix_metadata_reads, self.code_prefix_resolution)
    }

    pub fn initialize(
        &mut self,
        document: &Document,
        budget: &mut Budget,
    ) -> Result<bool, mdtui3::LayoutError> {
        self.scanner.shared_links().prepare_shape(document, budget)
    }

    pub fn shared_links(&self) -> crate::render::objects::SharedLinks {
        self.scanner.shared_links()
    }
    pub fn set_links(&mut self, links: crate::render::objects::SharedLinks) {
        self.width_cache.set_links(links.clone());
        self.scanner.set_links(links);
    }

    #[must_use]
    pub(crate) fn matches_view(
        &self,
        document: &Document,
        owner: &RenderCheckpointStore,
        width: u16,
        rows: u16,
    ) -> bool {
        self.matches(document)
            && self.config.width == width
            && self.config.row_limit == u64::from(rows)
            && self
                .binding
                .as_ref()
                .is_some_and(|binding| binding.check_binding(document, owner.view()).is_ok())
    }
    pub(crate) fn check_owner(
        &self,
        document: &Document,
        owner: &RenderCheckpointStore,
    ) -> Result<(), mdtui3::LayoutError> {
        if let Some(binding) = &self.binding {
            binding.check_binding(document, owner.view())?;
        }
        Ok(())
    }
    pub fn matches(&self, document: &Document) -> bool {
        self.document == document.id() && self.revision == document.revision()
    }

    #[must_use]
    pub fn plan(&self) -> &BlockPlan {
        &self.plan
    }

    #[must_use]
    pub fn scanner(&self) -> &DisplayScanner {
        &self.scanner
    }

    #[must_use]
    pub const fn scan_complete(&self) -> bool {
        self.scan_complete
    }

    #[must_use]
    pub fn sink(&self) -> Option<&TileLayoutSink> {
        self.sink.as_ref()
    }

    #[must_use]
    pub fn lines(&self) -> &[crate::render::sink::SinkLine] {
        self.sink.as_ref().map_or(&[], TileLayoutSink::lines)
    }

    pub(crate) fn prepared_copy_row(
        &self,
        block: mdtui3::BlockId,
        index: usize,
    ) -> Option<(&crate::render::sink::SinkLine, bool)> {
        self.sink.as_ref()?.prepared_copy_row(block, index)
    }

    pub fn write_to_screen(&self, screen: &mut Screen, start_row: u16) -> Result<u16, SinkError> {
        self.sink
            .as_ref()
            .map_or(Ok(0), |sink| sink.write_to_screen(screen, start_row))
    }

    #[must_use]
    pub fn sink_stats(&self) -> Option<LayoutSinkStats> {
        self.sink.as_ref().map(|sink| {
            let mut stats = sink.stats();
            stats.objects = self.scanner.shared_links().stats();
            stats.object_index = self.scanner.object_index_work();
            stats
        })
    }

    #[must_use]
    pub const fn rows(&self) -> u64 {
        self.rows
    }

    #[must_use]
    pub const fn processed_tiles(&self) -> usize {
        self.processed_tiles
    }

    #[must_use]
    pub const fn layout_attempts(&self) -> usize {
        self.layout_attempts
    }

    #[must_use]
    pub const fn direct_tiles(&self) -> usize {
        self.direct_tiles
    }

    #[must_use]
    pub const fn replay_used(&self) -> bool {
        self.replay_used
    }

    #[must_use]
    pub fn heights(&self) -> &HeightIndex {
        self.checkpoints.heights()
    }

    #[must_use]
    pub fn checkpoints(&self) -> &RenderCheckpointStore {
        &self.checkpoints
    }

    #[must_use]
    pub fn into_layout_parts(self) -> RenderCheckpointStore {
        self.checkpoints
    }

    pub fn suppressed_heading_tiles(&self) -> &[std::ops::Range<usize>] {
        &self.suppressed_heading_tiles
    }

    pub fn begin_replay(
        &mut self,
        from: TileId,
        target: TileId,
        checkpoint: RenderCheckpoint,
        max_bytes: usize,
    ) -> Result<(), FirstFrameError> {
        if self.projected {
            return Err(FirstFrameError::InvalidReplay);
        }

        if from.0 < self.plan.tile_start.0
            || target < from
            || target.0 >= self.plan.tile_end.0
            || max_bytes == 0
            || self
                .plan
                .segments
                .get(
                    from.0.saturating_sub(self.plan.tile_start.0)
                        ..target
                            .0
                            .saturating_sub(self.plan.tile_start.0)
                            .saturating_add(1),
                )
                .is_none_or(|segments| {
                    segments
                        .iter()
                        .any(|segment| segment.kind == TileSegmentKind::TuiOwned)
                })
        {
            return Err(FirstFrameError::InvalidReplay);
        }
        self.replay = Some(ReplayRequest {
            from,
            target,
            checkpoint,
            max_bytes,
        });
        self.replay_pass = None;
        self.replay_used = false;
        if let Some(sink) = &mut self.sink {
            sink.restore_checkpoint(checkpoint);
        }
        Ok(())
    }

    pub(crate) fn set_table_options(
        &mut self,
        document: &Document,
        policy: super::table_widths::TableWidthPolicy,
        positions: &[(mdtui3::TableId, usize)],
    ) -> Result<(), FirstFrameError> {
        self.width_cache.set_policy(policy);
        for &(id, first) in positions {
            self.width_cache.set_first_column(document, id, first)?;
        }
        Ok(())
    }

    pub(crate) fn set_math_copy_window(&mut self, offset: u64, rows: usize) {
        self.math_copy_view = Some(super::table_window::TableView {
            anchor: mdtui3::Anchor {
                tile: self.plan.tile_start,
                offset,
            },
            source: None,
            rows,
            tail: None,
        });
        if !self.scan_complete {
            self.scanner.set_math_rect(super::math::Rect {
                x: 0,
                y: 0,
                width: usize::from(self.config.width),
                height: rows,
            });
        }
        self.math_view_checked = false;
        self.sink = None;
        self.pass = None;
        self.cursor = 0;
        self.rows = 0;
        self.leading_rows = 0;
        self.processed_tiles = 0;
        self.replay = None;
        self.replay_pass = None;
    }
    pub(crate) fn prepared_copy_row_offset(
        &self,
        block: mdtui3::BlockId,
        index: usize,
    ) -> Option<u64> {
        self.sink.as_ref()?.prepared_copy_row_offset(block, index)
    }
    pub(crate) fn copy_logical_rows(&self) -> u64 {
        self.sink
            .as_ref()
            .map_or(0, |sink| sink.copy_logical_rows())
    }
    pub(crate) fn set_table_view(&mut self, view: super::table_window::TableView) {
        self.scanner.set_math_rect(super::math::Rect {
            x: 0,
            y: 0,
            width: usize::from(self.config.width),
            height: view.rows,
        });
        self.scanner.set_source_cut(view.tail);
        self.table_view = Some(view);
    }

    pub fn set_table_widths(
        &mut self,
        table_widths: HashMap<mdtui3::TableId, TableWidthSnapshot>,
    ) -> Result<(), FirstFrameError> {
        if self.sink.is_some() || self.processed_tiles > 0 {
            return Err(FirstFrameError::TableWidthsLocked);
        }
        self.table_widths = table_widths;
        Ok(())
    }

    pub(crate) fn advance_using(
        &mut self,
        document: &Document,
        owner: &mut RenderCheckpointStore,
        budget: &mut Budget,
    ) -> Result<FirstFrameStep, FirstFrameError> {
        if owner.maintenance_in_progress() {
            return Err(FirstFrameError::Layout(LayoutError::OutOfOrderUpdate));
        }
        if let Some(binding) = &self.binding {
            binding.check_binding(document, owner.view())?;
        }
        std::mem::swap(&mut self.checkpoints, owner);
        let result = self.advance(document, budget);
        std::mem::swap(&mut self.checkpoints, owner);
        result
    }

    pub fn advance(
        &mut self,
        document: &Document,
        budget: &mut Budget,
    ) -> Result<FirstFrameStep, FirstFrameError> {
        if !self.matches(document) {
            return Err(FirstFrameError::Stale);
        }
        if let Some(binding) = &self.binding {
            binding.check_binding(document, self.checkpoints.view())?;
        }
        if !self.project(document, budget)? {
            return Ok(self.step(FirstFrameStatus::BudgetExhausted));
        }
        if let Some(status) = self.ensure_ready(document, budget)? {
            return Ok(self.step(status));
        }
        if self.replay.is_some()
            && !self.replay_used
            && let Some(status) = self.advance_replay(document, budget)?
        {
            return Ok(self.step(status));
        }
        loop {
            if self.rows.saturating_sub(self.leading_rows) >= self.config.row_limit {
                return self.finish_measurements(document, budget, FirstFrameStatus::Covered);
            }
            let Some(segment) = self.plan.segments.get(self.cursor) else {
                let status = if self.plan.tile_end.0 >= document.tile_count() {
                    FirstFrameStatus::EndOfDocument
                } else {
                    FirstFrameStatus::Covered
                };
                return self.finish_measurements(document, budget, status);
            };
            let tile = segment.tile;
            let sink = self.sink.as_ref().ok_or(FirstFrameError::MissingSink)?;
            let expected = sink.tile_rows(tile).ok_or(FirstFrameError::MissingSink)?;

            if self.measurements
                && !self.projected
                && segment.kind == TileSegmentKind::Drawable
                && expected > 0
            {
                if self.pass.is_none() {
                    self.layout_attempts = self.layout_attempts.saturating_add(1);
                    let restored = tile.0 == 0 || self.checkpoints.exact(tile).is_some();
                    self.pass = Some(LayoutPass::new(
                        document,
                        self.checkpoints.heights(),
                        tile,
                        expected,
                        restored,
                    )?);
                }
                let completion = {
                    let pass = self.pass.as_mut().ok_or(FirstFrameError::MissingSink)?;
                    let sink = self.sink.as_mut().ok_or(FirstFrameError::MissingSink)?;
                    self.scanner.shared_links().with_resolver(|resolver| {
                        pass.swap_link_resolver(resolver)?;
                        let result = pass.advance(
                            document,
                            self.checkpoints.heights_mut(document)?,
                            budget,
                            sink,
                        );
                        pass.swap_link_resolver(resolver)?;
                        result
                    })?
                };
                match completion {
                    Completion::Covered
                        if self
                            .pass
                            .as_ref()
                            .is_some_and(|pass| pass.next_tile() == TileId(tile.0 + 1)) =>
                    {
                        self.pass = None;
                    }
                    Completion::BudgetExhausted => {
                        return Ok(self.step(FirstFrameStatus::BudgetExhausted));
                    }
                    completion => {
                        return Err(FirstFrameError::UnexpectedCompletion(completion));
                    }
                }
                let actual = self.checkpoints.heights().height(tile).unwrap_or(0);
                if actual != expected {
                    return Err(FirstFrameError::TileRowsMismatch {
                        tile,
                        expected,
                        actual,
                    });
                }
            } else {
                if !budget.charge(WorkLimit {
                    queries: 1,
                    tiles: 1,
                    objects: 1,
                    ..WorkLimit::default()
                }) {
                    return Ok(self.step(FirstFrameStatus::BudgetExhausted));
                }
                let sink = self.sink.as_mut().ok_or(FirstFrameError::MissingSink)?;
                let actual = sink.render_tui_owned_tile(tile);
                if actual != expected {
                    return Err(FirstFrameError::TileRowsMismatch {
                        tile,
                        expected,
                        actual,
                    });
                }
                if self.measurements {
                    let ticket = self
                        .checkpoints
                        .heights()
                        .ticket(tile)
                        .ok_or(FirstFrameError::InvalidTop)?;
                    self.checkpoints
                        .heights_mut(document)?
                        .measure(ticket, actual)?;
                }
                self.direct_tiles = self.direct_tiles.saturating_add(1);
            }

            if self.measurements
                && self
                    .sink
                    .as_ref()
                    .is_some_and(|sink| sink.stats().edge_placeholders > 0)
                && let Some(ticket) = self.checkpoints.heights().ticket(tile)
            {
                self.checkpoints
                    .heights_mut(document)?
                    .measure_local(ticket, expected)?;
            }
            if self.measurements
                && self.scanner.cluster_floor().is_some_and(|floor| {
                    document
                        .tile(tile)
                        .is_some_and(|s| s.start < floor && floor < s.end)
                })
                && let Some(ticket) = self.checkpoints.heights().ticket(tile)
            {
                self.checkpoints
                    .heights_mut(document)?
                    .measure_local(ticket, expected)?;
            }
            let checkpoint = self
                .sink
                .as_ref()
                .ok_or(FirstFrameError::MissingSink)?
                .checkpoint();
            if self.measurements {
                self.checkpoints
                    .checkpoint_after_tile(tile, checkpoint, document)?;
            }
            self.rows = self.rows.saturating_add(expected);
            if self.row_limit_anchor.is_some_and(|anchor| tile < anchor) {
                self.leading_rows = self.rows;
            }
            self.processed_tiles = self.processed_tiles.saturating_add(1);
            self.cursor += 1;
        }
    }

    fn finish_measurements(
        &mut self,
        document: &Document,
        budget: &mut Budget,
        status: FirstFrameStatus,
    ) -> Result<FirstFrameStep, FirstFrameError> {
        if !self.measurements {
            return Ok(self.step(status));
        }
        if !self.final_zero_ready {
            if !budget.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            }) {
                return Ok(self.step(FirstFrameStatus::BudgetExhausted));
            }
            let sink = self.sink.as_ref().ok_or(FirstFrameError::MissingSink)?;
            let positive = self
                .plan
                .segments
                .iter()
                .filter(|s| sink.tile_rows(s.tile).is_some_and(|rows| rows > 0))
                .map(|s| s.tile.0)
                .collect::<Vec<_>>();
            let mut ranges = self.scanner.zero_ranges().to_vec();
            ranges.extend(self.suppressed_heading_tiles.iter().cloned());
            ranges.sort_by_key(|r| r.start);
            let mut merged: Vec<std::ops::Range<usize>> = Vec::new();
            for range in ranges {
                if let Some(last) = merged.last_mut().filter(|last| range.start <= last.end) {
                    last.end = last.end.max(range.end);
                } else {
                    merged.push(range);
                }
            }
            for range in merged {
                let mut start = range.start;
                for &owner in positive
                    .iter()
                    .skip_while(|&&owner| owner < range.start)
                    .take_while(|&&owner| owner < range.end)
                {
                    if start < owner {
                        self.final_zero_ranges.push(start..owner);
                    }
                    start = owner + 1;
                }
                if start < range.end {
                    self.final_zero_ranges.push(start..range.end);
                }
            }
            self.final_zero_ready = true;
        }
        if !measure_zero_ranges(
            &self.final_zero_ranges,
            self.checkpoints.heights_mut(document)?,
            &mut self.final_zero_progress,
            budget,
        )? {
            return Ok(self.step(FirstFrameStatus::BudgetExhausted));
        }
        Ok(self.step(status))
    }

    fn ensure_ready(
        &mut self,
        document: &Document,
        budget: &mut Budget,
    ) -> Result<Option<FirstFrameStatus>, FirstFrameError> {
        if !self.plan_ready {
            if !self.contexts.advance(document, budget) {
                return Ok(Some(FirstFrameStatus::BudgetExhausted));
            }
            self.plan = if self.projected {
                crate::render::plan::projected_plan(
                    document,
                    self.plan.visible,
                    &self.fragments,
                    &self.placeholders,
                    &self.contexts.entries,
                )?
            } else {
                crate::render::plan::plan_with_contexts(
                    document,
                    self.scanner.requested(),
                    &self.contexts.entries,
                    self.promoted_code,
                )?
            };
            if self.projected {
                self.plan.clip_info_prefixes(&self.fragment_contexts);
            }
            if let Some(summary) = &self.summary {
                for placeholder in &self.placeholders {
                    if placeholder.candidate.id == summary.id {
                        for event in &mut self.plan.events {
                            if event.display_source == Some(placeholder.candidate.hidden)
                                && let crate::render::plan::PlanEventKind::FoldLabel {
                                    summary: lines,
                                    ..
                                } = &mut event.kind
                            {
                                *lines = summary.lines.clone();
                            }
                        }
                    }
                }
            }
            self.plan_ready = true;
        }
        if !self.nonwrap_ready {
            self.suppressed_heading_tiles = suppress_nonwrapping(document, &self.plan)?;
            if let Some(range) = &self.skipped_code_prefix {
                self.suppressed_heading_tiles.push(range.clone());
            }
            self.nonwrap_ready = true;
        }
        if !self.plan.advance_info(document, budget) {
            return Ok(Some(FirstFrameStatus::BudgetExhausted));
        }
        for block in &self.plan.blocks {
            if let mdtui3::BlockKind::Table { id } = block.kind
                && !self.table_widths.contains_key(&id)
            {
                let Some(snapshot) = self.width_cache.snapshot_budgeted(
                    document,
                    id,
                    usize::from(self.config.width),
                    budget,
                )?
                else {
                    return Ok(Some(FirstFrameStatus::BudgetExhausted));
                };
                self.table_widths.insert(id, snapshot);
            }
        }
        if !self.selection_ready {
            if self
                .plan
                .blocks
                .iter()
                .any(|b| matches!(b.kind, mdtui3::BlockKind::Table { .. }))
            {
                let window = self.table_window.get_or_insert_with(|| {
                    if self.projected {
                        super::table_window::TableWindow::projected(
                            self.scanner.requested(),
                            &self.fragments,
                            &self.fragment_contexts,
                            self.projection_cut
                                .or_else(|| self.table_view.and_then(|view| view.tail)),
                        )
                    } else {
                        super::table_window::TableWindow::new(self.scanner.requested())
                    }
                });
                if !window.advance(
                    document,
                    &self.plan,
                    &self.table_widths,
                    usize::from(self.config.width),
                    usize::try_from(self.config.row_limit).unwrap_or(usize::MAX),
                    self.table_view,
                    self.projected.then(|| self.checkpoints.view()),
                    budget,
                )? {
                    return Ok(Some(FirstFrameStatus::BudgetExhausted));
                }
                if self.projected {
                    let mut window = self.table_window.take().unwrap();
                    for owner in &window.owners {
                        if self
                            .plan
                            .segments
                            .binary_search_by_key(owner, |s| s.tile)
                            .is_err()
                        {
                            self.plan.segments.push(super::plan::TileSegment {
                                tile: *owner,
                                source: document.tile(*owner).ok_or(FirstFrameError::InvalidTop)?,
                                kind: super::plan::TileSegmentKind::TuiOwned,
                                drawable_at: None,
                                blocks: Vec::new(),
                            });
                            self.plan.segments.sort_by_key(|s| s.tile);
                        }
                    }
                    if let Some(last) = self.plan.segments.last() {
                        self.plan.tile_end = TileId(last.tile.0 + 1);
                    }
                    for placeholder in window.placeholders.drain(..) {
                        if !self.placeholders.iter().any(|old| {
                            old.candidate.hidden.start == placeholder.candidate.hidden.start
                        }) {
                            self.placeholders.push(placeholder);
                        }
                    }
                    self.placeholders.sort_by_key(|p| p.candidate.hidden.start);
                    self.scanner.select_projected(window);
                } else {
                    self.scanner.select(self.table_window.take().unwrap());
                }
            }
            self.selection_ready = true;
        }
        if !self.scan_complete {
            match self.scanner.advance(document, budget)? {
                ScanStatus::BudgetExhausted => return Ok(Some(FirstFrameStatus::PendingScan)),
                ScanStatus::Complete => self.scan_complete = true,
            }
        }
        if self.projected && !self.placeholders_inserted {
            let blocks = self.plan.blocks.iter().map(|b| b.id).collect::<Vec<_>>();
            for placeholder in &self.placeholders {
                if crate::render::plan::inline_placeholder(document, placeholder, &blocks) {
                    let label = crate::render::plan::placeholder_label(placeholder.candidate);
                    let fit =
                        crate::render::text::fit_prefix(&label, usize::from(self.config.width));
                    let mut label = if self.config.width < 3 {
                        "+".to_owned()
                    } else {
                        label[..fit.end].to_owned()
                    };
                    if let Some(summary) = self
                        .summary
                        .as_ref()
                        .filter(|summary| summary.id == placeholder.candidate.id)
                    {
                        let preview = summary.lines.join(" / ");
                        let fit = crate::render::text::fit_prefix(
                            &preview,
                            usize::from(self.config.width),
                        );
                        label.push(' ');
                        label.push_str(&preview[..fit.end]);
                    }
                    self.scanner
                        .insert_placeholder(document, *placeholder, label);
                }
            }
            self.placeholders_inserted = true;
        }
        if !self.relocated
            && let Some(first) = self.scanner.segments().first().filter(|s| {
                (s.text.is_indexed() || self.scanner.math_views().contains_key(&s.source.start))
                    && s.source.start < self.plan.visible.start
            })
        {
            let owner = document
                .tile_at(first.source.start)
                .ok_or(FirstFrameError::InvalidTop)?;
            let floor = first.source.start;
            let extra = usize::from(self.config.top_tile.0.saturating_sub(owner.0) == 1);
            let config = FirstFrameConfig {
                top_tile: owner,
                tile_limit: self.config.tile_limit.saturating_add(extra),
                ..self.config
            };
            let links = self.scanner.shared_links();
            let widths = self.table_widths.clone();
            let view = self.table_view;
            let copy_view = self.math_copy_view;
            let mut replacement = Self::new_borrowed(document, config)?;
            replacement.row_limit_anchor = self.row_limit_anchor;
            replacement.set_links(links);
            replacement.set_table_widths(widths)?;
            if let Some(view) = view {
                replacement.set_table_view(view);
            }
            replacement.scanner.set_cluster_floor(floor);
            let mut checkpoint = RenderCheckpoint::line_start();
            checkpoint.line_source_start = Some(floor);
            replacement.binding = self.binding.take();
            std::mem::swap(&mut replacement.checkpoints, &mut self.checkpoints);
            replacement.checkpoints.insert(owner, checkpoint)?;
            if let Some(view) = copy_view {
                replacement.set_math_copy_window(view.anchor.offset, view.rows);
            }
            replacement.relocated = true;
            *self = replacement;
            return Ok(Some(FirstFrameStatus::PendingScan));
        }
        if self.sink.is_none() {
            // Dynamic selection is now prepared. Classify from that window,
            // rather than evaluating unbudgeted next_drawable predicates.
            let mut drawable = vec![None; self.plan.segments.len()];
            for segment in self.scanner.segments() {
                if let Ok(index) = self
                    .plan
                    .segments
                    .binary_search_by_key(&segment.tile, |s| s.tile)
                {
                    drawable[index].get_or_insert(segment.source.start);
                }
            }
            let mut events = vec![false; self.plan.segments.len()];
            for event in &self.plan.events {
                if let Some(tile) = document.tile_at(event.source.start)
                    && let Ok(index) = self.plan.segments.binary_search_by_key(&tile, |s| s.tile)
                {
                    events[index] = true;
                }
            }
            for (index, segment) in self.plan.segments.iter_mut().enumerate() {
                segment.drawable_at = drawable[index];
                segment.kind = if self.scanner.intersects_math(segment.source)
                    || self.plan.blocks.iter().any(|b| {
                        matches!(b.kind, mdtui3::BlockKind::Table { .. })
                            && b.source.start < segment.source.end
                            && segment.source.start < b.source.end
                    }) {
                    TileSegmentKind::TuiOwned
                } else if drawable[index].is_some() {
                    TileSegmentKind::Drawable
                } else if events[index] {
                    TileSegmentKind::TuiOwned
                } else {
                    TileSegmentKind::ZeroRow
                };
            }
            if !budget.charge(WorkLimit {
                objects: 1,
                ..WorkLimit::default()
            }) {
                return Ok(Some(FirstFrameStatus::BudgetExhausted));
            }
            self.sink = Some(TileLayoutSink::new_with_contexts(
                document,
                &self.plan,
                &self.scanner,
                self.config.width,
                &self.table_widths,
                &self.contexts.entries,
                self.contexts.reads,
            )?);
            if let Some(replay) = self.replay
                && !self.replay_used
            {
                self.sink
                    .as_mut()
                    .ok_or(FirstFrameError::MissingSink)?
                    .restore_checkpoint(replay.checkpoint);
            }
        }
        if !self.projected
            && !self.table_view_checked
            && let Some(view) = self.table_view
            && view.tail.is_none()
        {
            if !budget.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            }) {
                return Ok(Some(FirstFrameStatus::BudgetExhausted));
            }
            self.table_view_checked = true;
            if !(view.source.is_some() && !self.scanner.math_views().is_empty())
                && let Some((id, row, source)) = self
                    .sink
                    .as_ref()
                    .and_then(|sink| sink.corrected_table_source(document, view))
                && self
                    .scanner
                    .selected_rows(id)
                    .is_some_and(|range| !range.contains(&row))
            {
                self.table_view.as_mut().unwrap().source = Some(source);
                let requested = self.scanner.requested();
                let links = self.scanner.shared_links();
                self.scanner =
                    DisplayScanner::new(document, requested).ok_or(FirstFrameError::InvalidTop)?;
                self.scanner.set_links(links);
                self.sink = None;
                self.scan_complete = false;
                self.selection_ready = false;
                return Ok(Some(FirstFrameStatus::PendingScan));
            }
        }
        if !self.math_view_checked {
            if let Some(view) = self.math_copy_view.or(self.table_view) {
                match self.scanner.refresh_math(
                    document,
                    self.sink.as_ref().unwrap(),
                    view,
                    budget,
                )? {
                    None => return Ok(Some(FirstFrameStatus::PendingScan)),
                    Some(changed) => {
                        self.math_view_checked = true;
                        if changed {
                            self.sink = None;
                            return Ok(Some(FirstFrameStatus::PendingScan));
                        }
                    }
                }
            } else {
                self.math_view_checked = true;
            }
        }
        Ok(None)
    }

    fn advance_replay(
        &mut self,
        document: &Document,
        budget: &mut Budget,
    ) -> Result<Option<FirstFrameStatus>, FirstFrameError> {
        let replay = self.replay.ok_or(FirstFrameError::InvalidReplay)?;
        if self
            .sink
            .as_ref()
            .and_then(|sink| sink.tile_rows(replay.target))
            .unwrap_or(0)
            == 0
        {
            self.replay = None;
            return Ok(None);
        }
        if self.replay_pass.is_none() {
            let Some(pass) = LayoutPass::from_checkpoint(
                document,
                self.checkpoints.heights(),
                replay.from,
                replay.target,
                1,
                replay.max_bytes,
            )?
            else {
                return Err(FirstFrameError::InvalidReplay);
            };
            self.replay_pass = Some(pass);
        }
        let completion = {
            let pass = self
                .replay_pass
                .as_mut()
                .ok_or(FirstFrameError::InvalidReplay)?;
            let sink = self.sink.as_mut().ok_or(FirstFrameError::MissingSink)?;
            self.scanner.shared_links().with_resolver(|resolver| {
                pass.swap_link_resolver(resolver)?;
                let result = pass.advance(
                    document,
                    self.checkpoints.heights_mut(document)?,
                    budget,
                    sink,
                );
                pass.swap_link_resolver(resolver)?;
                result
            })?
        };
        match completion {
            Completion::BudgetExhausted => Ok(Some(FirstFrameStatus::BudgetExhausted)),
            Completion::Covered | Completion::EndOfDocument => {
                let next = self
                    .replay_pass
                    .as_ref()
                    .ok_or(FirstFrameError::InvalidReplay)?
                    .next_tile();
                let measured = self
                    .replay_pass
                    .as_ref()
                    .ok_or(FirstFrameError::InvalidReplay)?
                    .measured_rows();
                self.replay_pass = None;
                self.replay_used = true;
                self.rows = u64::try_from(measured).unwrap_or(u64::MAX);
                if self.row_limit_anchor == Some(replay.target) {
                    self.leading_rows = self.rows.saturating_sub(
                        self.sink
                            .as_ref()
                            .and_then(|sink| sink.tile_rows(replay.target))
                            .unwrap_or(0),
                    );
                }
                self.cursor = next
                    .0
                    .saturating_sub(self.plan.tile_start.0)
                    .min(self.plan.segments.len());
                self.processed_tiles = self.processed_tiles.max(
                    replay
                        .target
                        .0
                        .saturating_sub(replay.from.0)
                        .saturating_add(1),
                );
                let checkpoint = self
                    .sink
                    .as_ref()
                    .ok_or(FirstFrameError::MissingSink)?
                    .checkpoint();
                self.checkpoints
                    .checkpoint_after_tile(replay.target, checkpoint, document)?;
                Ok(None)
            }
        }
    }

    fn step(&self, status: FirstFrameStatus) -> FirstFrameStep {
        FirstFrameStep {
            status,
            rows: self.rows,
            processed_tiles: self.processed_tiles,
            layout_attempts: self.layout_attempts,
            direct_tiles: self.direct_tiles,
        }
    }
}

fn suppress_nonwrapping(
    document: &Document,
    plan: &BlockPlan,
) -> Result<Vec<std::ops::Range<usize>>, FirstFrameError> {
    let mut ranges = Vec::new();
    for block in &plan.blocks {
        let mut sources = Vec::new();
        match block.kind {
            mdtui3::BlockKind::Heading { .. } => sources.push(block.source),
            mdtui3::BlockKind::Table { id } => {
                if let Some(table) = document.table(id) {
                    for segment in &plan.segments {
                        if let Some(row) = table
                            .row_at(segment.source.start)
                            .and_then(|at| table.row(at))
                            && row.source.len() > 4096
                            && sources.last() != Some(&row.source)
                        {
                            sources.push(row.source);
                        }
                    }
                }
            }
            _ => {}
        }
        for source in sources {
            if source.len() <= 4096 {
                continue;
            }
            let first = document
                .tile_at(source.start)
                .ok_or(FirstFrameError::InvalidTop)?;
            let last = document
                .tile_at(source.end - 1)
                .ok_or(FirstFrameError::InvalidTop)?;
            let last_span = document.tile(last).ok_or(FirstFrameError::InvalidTop)?;
            let end = last.0 + usize::from(last_span.end <= source.end);
            let range = first.0 + 1..end;
            if !range.is_empty() {
                ranges.push(range);
            }
        }
    }
    Ok(ranges)
}

#[cfg(test)]
mod giant_fold_seam_tests {
    use super::*;
    use mdtui3::{FoldId, FoldView, HeadingId, TILE_BYTES};
    #[test]
    fn giant_continuations_measure_the_fold_views_same_tree_without_erasing_masks() {
        let mut d = Document::new();
        let cluster = format!("a{}", "\u{301}".repeat(10_000));
        d.append(&format!("# A\n{cluster}\n# B\ntail\n")).unwrap();
        d.finish();
        let mut fold = FoldView::<()>::with_state(&d, 2).unwrap();
        fold.set_collapsed(&d, FoldId::Heading(HeadingId(0)), true)
            .unwrap();
        let range = 1..(4 + cluster.len()) / TILE_BYTES;
        let (heights, _) = fold.layout_parts_mut(&d).unwrap();
        let before: Vec<_> = (0..d.tile_count())
            .map(|t| heights.height(TileId(t)))
            .collect();
        let ranges = [range.clone()];
        let mut progress = ZeroRangeProgress::default();
        assert!(
            !measure_zero_ranges(
                &ranges,
                heights,
                &mut progress,
                &mut Budget::new(WorkLimit::default())
            )
            .unwrap()
        );
        for _ in 0..4 {
            let mut b = Budget::new(WorkLimit {
                queries: 1,
                objects: 1,
                bytes: 6,
                ..Default::default()
            });
            if measure_zero_ranges(&ranges, heights, &mut progress, &mut b).unwrap() {
                break;
            }
        }
        assert_eq!(progress.at, 1);
        for (t, old) in before.into_iter().enumerate() {
            assert_eq!(
                heights.base_height(TileId(t)),
                Some(if range.contains(&t) { 0 } else { 2 })
            );
            assert_eq!(
                heights.height(TileId(t)),
                if range.contains(&t) { Some(0) } else { old }
            );
        }
        fold.set_collapsed(&d, FoldId::Heading(HeadingId(0)), false)
            .unwrap();
        let (heights, _) = fold.layout_parts_mut(&d).unwrap();
        // A fold change invalidates layout, so capture and pay a fresh
        // measurement rather than treating the old proof as permanent.
        let mut progress = ZeroRangeProgress::default();
        assert!(
            measure_zero_ranges(
                &ranges,
                heights,
                &mut progress,
                &mut Budget::new(WorkLimit::unlimited())
            )
            .unwrap()
        );
        for t in range {
            assert_eq!(heights.height(TileId(t)), Some(0));
        }
    }
    #[test]
    fn real_math_continuations_use_fold_owner_zero_tickets_and_preserve_coverage() {
        let mut d = Document::new();
        d.append(&format!(
            "# A\n\\[\n\\frac{{a}}{{b}}{}\n\\]\n# B\ntail\n",
            " ".repeat(TILE_BYTES * 4)
        ))
        .unwrap();
        d.finish();
        let mut first = FirstFrame::new(
            &d,
            FirstFrameConfig {
                top_tile: TileId(0),
                tile_limit: d.tile_count(),
                width: 20,
                row_limit: 100,
            },
        )
        .unwrap();
        loop {
            let step = first
                .advance(&d, &mut Budget::new(WorkLimit::unlimited()))
                .unwrap();
            if matches!(
                step.status,
                FirstFrameStatus::Covered | FirstFrameStatus::EndOfDocument
            ) {
                break;
            }
        }
        let ranges = first.zero_ranges().to_vec();
        assert_eq!(ranges.len(), 1);
        let mut fold = FoldView::<()>::with_state(&d, 2).unwrap();
        let id = FoldId::Heading(HeadingId(0));
        fold.set_collapsed(&d, id, true).unwrap();
        let (heights, _) = fold.layout_parts_mut(&d).unwrap();
        let before: Vec<_> = (0..d.tile_count())
            .map(|t| heights.height(TileId(t)))
            .collect();
        let mut progress = ZeroRangeProgress::default();
        while !measure_zero_ranges(
            &ranges,
            heights,
            &mut progress,
            &mut Budget::new(WorkLimit {
                queries: 1,
                objects: 1,
                bytes: 6,
                ..Default::default()
            }),
        )
        .unwrap()
        {}
        for (t, &old_height) in before.iter().enumerate() {
            assert_eq!(heights.height(TileId(t)), old_height);
            if ranges[0].contains(&t) {
                assert_eq!(heights.base_height(TileId(t)), Some(0));
            }
        }
        assert!(fold.is_collapsed(id));
        fold.set_collapsed(&d, id, false).unwrap();
        let (heights, _) = fold.layout_parts_mut(&d).unwrap();
        let mut fresh = ZeroRangeProgress::default();
        assert!(
            measure_zero_ranges(
                &ranges,
                heights,
                &mut fresh,
                &mut Budget::new(WorkLimit::unlimited())
            )
            .unwrap()
        );
        let ticket = heights.ticket(TileId(0)).unwrap();
        let rows = first.sink().unwrap().tile_rows(TileId(0)).unwrap();
        heights.measure(ticket, rows).unwrap();
        assert_eq!(heights.height(TileId(0)), Some(rows));
        assert!(rows >= 3);
        for t in ranges[0].clone() {
            assert_eq!(heights.height(TileId(t)), Some(0));
        }
    }
}
