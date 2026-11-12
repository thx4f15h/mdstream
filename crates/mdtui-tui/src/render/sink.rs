use std::collections::HashMap;
use std::fmt;

use mdtui3::{BlockId, BlockKind, Document, LinkId, Span, TableId, TileId};

use crate::render::checkpoint::{
    CheckpointBlockKind, CheckpointPosition, CodeRenderState, ListMarkerState, PendingBlockEvents,
    RenderCheckpoint, TableRenderState,
};
use crate::render::code::{code_lines_from_input, render_code_lines};
use crate::render::context::ContainerContext;
use crate::render::display::{DisplayScanner, DisplaySegment};
use crate::render::heading::render_heading_input;
use crate::render::input::{DisplayInput, GlyphInput, InputToken, TokenKind, display_glyphs};
use crate::render::list::ListVisual;
use crate::render::plan::{BlockPlan, PlanEvent, PlanEventKind};
use crate::render::table::{render_table_with_geometry, table_geometry_window, table_row_window};
use crate::render::table_widths::{TableWidthCache, TableWidthSnapshot};
use crate::render::text::{GraphemeKind, graphemes};
use crate::screen::{Cell, ContentArea, LineSource, Screen, ScreenError, StyleRole};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SinkError {
    InvalidRevision,
    IncompleteScan,
    InvalidWidth,
    RowOverflow,
    Render(String),
    Screen(ScreenError),
}

impl fmt::Display for SinkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRevision => formatter.write_str("plan or display scan is stale"),
            Self::IncompleteScan => formatter.write_str("display scan does not cover the plan"),
            Self::InvalidWidth => formatter.write_str("layout width must be non-zero"),
            Self::RowOverflow => formatter.write_str("rendered rows exceed the supported range"),
            Self::Render(error) => formatter.write_str(error),
            Self::Screen(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for SinkError {}

impl From<ScreenError> for SinkError {
    fn from(error: ScreenError) -> Self {
        Self::Screen(error)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SinkCell {
    pub text: String,
    pub style: StyleRole,
    pub width: u8,
    pub link: Option<LinkId>,
    pub source: Option<Span>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SinkLine {
    pub cells: Vec<SinkCell>,
    pub source: Option<LineSource>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LayoutSinkStats {
    pub objects: crate::render::objects::ObjectStats,
    pub object_index: mdtui3::LinkWindowWork,
    pub segments_consumed: usize,
    pub events_consumed: usize,
    pub rows: u64,
    pub tiles_rendered: usize,
    pub begin_tile_callbacks: u64,
    pub raw_text_callbacks: u64,
    pub object_callbacks: u64,
    pub document_queries_in_callbacks: u64,
    pub table_rows_prepared: usize,
    pub container_nodes_read: usize,
    /// Source bytes requested by preparation in addition to DisplayScanner.
    /// Repeated code/list passes are counted separately.
    pub preparation_source_bytes: usize,
    pub prepared_display_bytes: usize,
    pub edge_placeholders: usize,
    pub scratch_capacity_cells: usize,
}

#[derive(Debug, Clone)]
struct OwnedGlyph {
    text: String,
    width: u8,
    kind: GraphemeKind,
    style: StyleRole,
    link: Option<LinkId>,
    source: Span,
    block: Option<BlockId>,
}

#[derive(Debug, Clone)]
struct PreparedBlock {
    checkpoint: RenderCheckpoint,
    content: PreparedContent,
}

#[derive(Debug, Clone)]
enum PreparedContent {
    Streamed {
        quote: bool,
        glyphs: Vec<OwnedGlyph>,
    },
    Rows(Vec<PreparedRow>),
}

#[derive(Debug, Clone)]
struct PreparedRow {
    line: SinkLine,
    owner_tile: TileId,
    source_start: usize,
}

#[derive(Debug, Clone, Copy)]
enum WorkKind {
    Event(usize),
    FinishStream(BlockId),
    FinishPending,
    Row(BlockId, usize),
    SkippedRows(u64),
}

#[derive(Debug, Clone, Copy)]
struct WorkItem {
    source_start: usize,
    rank: u8,
    order: usize,
    kind: WorkKind,
}

struct PrepareContext<'a> {
    document: &'a Document,
    width: u16,
    fallback_tile: TileId,
    plan_source: Span,
    table_widths: &'a HashMap<TableId, TableWidthSnapshot>,
    table_rows: Option<std::ops::Range<usize>>,
    table_windows: &'a std::cell::RefCell<Vec<super::table::TableWindowStatus>>,
    containers: &'a HashMap<mdtui3::ContainerId, ContainerContext>,
    display_bytes: &'a std::cell::Cell<usize>,
    scratch_cells: &'a std::cell::Cell<usize>,
}

impl PrepareContext<'_> {
    fn display(&self, bytes: usize) {
        self.display_bytes
            .set(self.display_bytes.get().saturating_add(bytes));
    }
    fn scratch(&self, screen: &Screen) {
        let stats = screen.buffer_stats();
        self.scratch_cells.set(
            self.scratch_cells
                .get()
                .max(stats.front_capacity_cells + stats.back_capacity_cells),
        );
    }
}

pub struct TileLayoutSink {
    width: u16,
    events: Vec<PlanEvent>,
    work: Vec<Vec<WorkItem>>,
    blocks: HashMap<BlockId, PreparedBlock>,
    pending: Option<Vec<OwnedGlyph>>,
    tile_rows: Vec<u64>,
    boundary_checkpoints: Vec<RenderCheckpoint>,
    lines: Vec<SinkLine>,
    line_offsets: Vec<u64>,
    logical_rows: u64,
    table_windows: Vec<super::table::TableWindowStatus>,
    rendered_tiles: Vec<bool>,
    current: Option<TileId>,
    tile_start: TileId,
    tile_end: TileId,
    checkpoint: RenderCheckpoint,
    stats: LayoutSinkStats,
}

impl TileLayoutSink {
    pub fn new(
        document: &Document,
        plan: &BlockPlan,
        scanner: &DisplayScanner,
        width: u16,
    ) -> Result<Self, SinkError> {
        Self::new_with_table_widths(document, plan, scanner, width, &HashMap::new())
    }

    pub fn new_with_table_widths(
        document: &Document,
        plan: &BlockPlan,
        scanner: &DisplayScanner,
        width: u16,
        table_widths: &HashMap<TableId, TableWidthSnapshot>,
    ) -> Result<Self, SinkError> {
        Self::new_with_contexts(
            document,
            plan,
            scanner,
            width,
            table_widths,
            &HashMap::new(),
            0,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_with_contexts(
        document: &Document,
        plan: &BlockPlan,
        scanner: &DisplayScanner,
        width: u16,
        table_widths: &HashMap<TableId, TableWidthSnapshot>,
        containers: &HashMap<mdtui3::ContainerId, ContainerContext>,
        container_nodes_read: usize,
    ) -> Result<Self, SinkError> {
        if width == 0 {
            return Err(SinkError::InvalidWidth);
        }
        if !plan.matches(document) || !scanner.matches(document) || !scanner.is_complete() {
            return Err(SinkError::InvalidRevision);
        }
        if plan.tile_start.0 < plan.tile_end.0 {
            let source_start = document
                .tile(plan.tile_start)
                .ok_or(SinkError::IncompleteScan)?
                .start;
            let source_end = document
                .tile(TileId(plan.tile_end.0 - 1))
                .ok_or(SinkError::IncompleteScan)?
                .end;
            if scanner.requested().start > source_start || scanner.requested().end < source_end {
                return Err(SinkError::IncompleteScan);
            }
        }

        let plan_source = planned_source(document, plan)?;
        let pending_span = document.pending();
        let pending_visible = Span::new(
            pending_span.start.max(plan_source.start),
            if pending_span.is_empty() {
                pending_span.end.min(plan_source.end)
            } else {
                document.source().len().min(plan_source.end)
            },
        );
        let mut segments_consumed = 0_usize;
        let mut segments_by_block: HashMap<BlockId, Vec<&DisplaySegment>> = HashMap::new();
        let mut pending_segments = Vec::new();
        let item_owners: HashMap<_, _> = plan
            .blocks
            .iter()
            .filter_map(|planned| {
                let block = document.block(planned.id)?;
                matches!(block.kind, BlockKind::ListItem { .. })
                    .then_some((block.container?, planned.id))
            })
            .collect();
        for segment in scanner.layout_segments() {
            if plan.segment(segment.tile).is_none()
                && segment
                    .block
                    .is_none_or(|id| !plan.blocks.iter().any(|block| block.id == id))
            {
                continue;
            }
            segments_consumed += 1;
            let Some(block) = segment.block else {
                if !pending_visible.is_empty()
                    && segment.source.start >= pending_visible.start
                    && segment.source.end <= pending_visible.end
                {
                    pending_segments.push(segment);
                }
                continue;
            };
            let owner = document
                .block(block)
                .filter(|block| block.kind == BlockKind::Paragraph)
                .and_then(|block| block.container)
                .and_then(|container| item_owners.get(&container))
                .copied()
                .unwrap_or(block);
            segments_by_block.entry(owner).or_default().push(segment);
        }

        let mut blocks = HashMap::new();
        let table_windows = std::cell::RefCell::new(Vec::new());
        let mut table_rows_prepared = 0_usize;
        let mut edge_placeholders = 0usize;
        let display_bytes = std::cell::Cell::new(0);
        let scratch_cells = std::cell::Cell::new(0);
        let mut work = vec![Vec::new(); plan.segments.len()];
        let mut order = 0_usize;
        for planned in &plan.blocks {
            let Some(block) = document.block(planned.id) else {
                continue;
            };
            let has_label = plan.events.iter().any(|event| {
                event.block == planned.id && matches!(event.kind, PlanEventKind::CodeLabel { .. })
            });
            let has_rule = plan.events.iter().any(|event| {
                event.block == planned.id && matches!(event.kind, PlanEventKind::ThematicBreak)
            });
            let context = PrepareContext {
                document,
                width,
                fallback_tile: plan.tile_start,
                plan_source,
                table_widths,
                table_windows: &table_windows,
                table_rows: if let BlockKind::Table { id } = block.kind {
                    scanner.selected_rows(id)
                } else {
                    None
                },
                containers,
                display_bytes: &display_bytes,
                scratch_cells: &scratch_cells,
            };
            let mut glyphs = block_glyphs(
                document,
                block,
                segments_by_block.remove(&planned.id).unwrap_or_default(),
                planned.id,
            );
            if matches!(block.kind, BlockKind::Table { .. }) {
                for glyph in &mut glyphs {
                    if glyph.text.len() > super::edges::MAX_EDGE_OVERSCAN
                        || scanner.cell_cut(glyph.source.end)
                    {
                        edge_placeholders += 1;
                        glyph.text = "…".into();
                        glyph.width = 1;
                        glyph.kind = GraphemeKind::Narrow;
                        glyph.link = None;
                        glyph.style = StyleRole::META;
                    }
                }
            }
            let (omit_start, omit_end) = scanner.omitted_edges();
            let context_source = scanner.layout_source();
            for glyph in &mut glyphs {
                let overlaps_start =
                    glyph.source.start < plan_source.start && glyph.source.end > plan_source.start;
                let overlaps_end =
                    glyph.source.start < plan_source.end && glyph.source.end > plan_source.end;
                let cut_start = overlaps_start
                    && context_source.start > 0
                    && (omit_start || glyph.source.start == context_source.start);
                let cut_end = overlaps_end
                    && context_source.end < document.source().len()
                    && (omit_end || glyph.source.end == context_source.end);
                if (cut_start || cut_end) && !matches!(block.kind, BlockKind::Table { .. }) {
                    edge_placeholders += 1;
                    glyph.text = "…".to_owned();
                    glyph.width = 1;
                    glyph.kind = GraphemeKind::Narrow;
                    glyph.source = Span::new(
                        glyph.source.start.max(plan_source.start),
                        glyph.source.end.min(plan_source.end),
                    );
                    glyph.link = None;
                    glyph.style = StyleRole::META;
                }
            }
            if !matches!(block.kind, BlockKind::Table { .. }) {
                glyphs.retain(|glyph| {
                    glyph.source.end > plan_source.start && glyph.source.start < plan_source.end
                });
            }
            let (limited_start, limited_end) = scanner.limited_context();
            let uncertain = !matches!(block.kind, BlockKind::Table { .. })
                && ((limited_start
                    && block.source.start <= context_source.start
                    && block.source.end > context_source.start)
                    || (limited_end
                        && block.source.start < context_source.end
                        && block.source.end >= context_source.end));
            if uncertain && !glyphs.is_empty() {
                edge_placeholders += 1;
                glyphs.clear();
                glyphs.push(OwnedGlyph {
                    text: "…".to_owned(),
                    width: 1,
                    kind: GraphemeKind::Narrow,
                    style: StyleRole::META,
                    link: None,
                    block: Some(planned.id),
                    source: Span::new(
                        block.source.start.max(plan_source.start),
                        block.source.end.min(plan_source.end),
                    ),
                });
            }
            let Some(content) =
                prepare_block(&context, planned.id, block, glyphs, has_label, has_rule)?
            else {
                continue;
            };
            if matches!(block.kind, BlockKind::Table { .. })
                && let PreparedContent::Rows(rows) = &content
            {
                table_rows_prepared = table_rows_prepared.saturating_add(rows.len());
            }
            let checkpoint = checkpoint_for_block(
                document,
                planned.id,
                &planned.kind,
                block.source,
                containers,
            )?;
            match &content {
                PreparedContent::Streamed { .. } => {
                    let owner = owner_tile(
                        document,
                        Span::new(block.source.start, block.source.end.min(plan_source.end)),
                        plan.tile_start,
                    );
                    if let Some(index) = tile_index(plan, owner) {
                        work[index].push(WorkItem {
                            source_start: block.source.end.saturating_sub(1),
                            rank: 2,
                            order,
                            kind: WorkKind::FinishStream(planned.id),
                        });
                        order += 1;
                    }
                }
                PreparedContent::Rows(rows) => {
                    for (row_index, row) in rows.iter().enumerate() {
                        let Some(index) = tile_index(plan, row.owner_tile) else {
                            continue;
                        };
                        work[index].push(WorkItem {
                            source_start: row.source_start,
                            rank: 2,
                            order,
                            kind: WorkKind::Row(planned.id, row_index),
                        });
                        order += 1;
                    }
                }
            }
            blocks.insert(
                planned.id,
                PreparedBlock {
                    checkpoint,
                    content,
                },
            );
        }

        for &(tile, source_start, count) in scanner.skipped_rows() {
            if let Some(index) = tile_index(plan, tile) {
                work[index].push(WorkItem {
                    source_start,
                    rank: 2,
                    order,
                    kind: WorkKind::SkippedRows(count),
                });
                order += 1;
            }
        }

        let pending_glyphs = segments_glyphs(pending_segments, None);
        let pending = if pending_glyphs.is_empty() {
            None
        } else {
            let owner = document
                .tile_at(pending_visible.end.saturating_sub(1))
                .unwrap_or(TileId(plan.tile_end.0.saturating_sub(1)));
            if let Some(index) = tile_index(plan, owner) {
                work[index].push(WorkItem {
                    source_start: pending_visible.start,
                    rank: 3,
                    order,
                    kind: WorkKind::FinishPending,
                });
                order += 1;
            }
            Some(pending_glyphs)
        };

        let events = plan.events.clone();
        for (index, event) in events.iter().enumerate() {
            let Some(tile) = document.tile_at(event.source.start) else {
                continue;
            };
            let Some(tile_index) = tile_index(plan, tile) else {
                continue;
            };
            let rank = match event.kind {
                PlanEventKind::Spacing { .. } => 0,
                _ => 1,
            };
            work[tile_index].push(WorkItem {
                source_start: event.source.start,
                rank,
                order,
                kind: WorkKind::Event(index),
            });
            order += 1;
        }
        for items in &mut work {
            items.sort_by_key(|item| (item.source_start, item.rank, item.order));
        }

        let tile_rows = calculate_tile_rows(
            &events,
            &work,
            &blocks,
            pending.as_deref(),
            width,
            plan.tile_start,
            plan.tile_end,
        )?;
        let boundary_checkpoints = boundary_checkpoints(plan, &events, &blocks);
        Ok(Self {
            width,
            events,
            work,
            blocks,
            pending,
            tile_rows,
            boundary_checkpoints,
            lines: Vec::new(),
            line_offsets: Vec::new(),
            logical_rows: 0,
            table_windows: table_windows.into_inner(),
            rendered_tiles: vec![false; plan.segments.len()],
            current: None,
            tile_start: plan.tile_start,
            tile_end: plan.tile_end,
            checkpoint: RenderCheckpoint::line_start(),
            stats: LayoutSinkStats {
                segments_consumed,
                table_rows_prepared,
                container_nodes_read,
                preparation_source_bytes: plan.code_info_source_bytes,
                prepared_display_bytes: display_bytes.get(),
                edge_placeholders,
                scratch_capacity_cells: scratch_cells.get(),
                ..LayoutSinkStats::default()
            },
        })
    }

    #[must_use]
    pub fn lines(&self) -> &[SinkLine] {
        &self.lines
    }

    /// Already prepared rows can belong to a physical tile outside the copy
    /// window. Reading them preserves table ownership without expanding the
    /// window across an arbitrarily large row envelope.
    pub(crate) fn prepared_copy_row(
        &self,
        block: BlockId,
        index: usize,
    ) -> Option<(&SinkLine, bool)> {
        let PreparedContent::Rows(rows) = &self.blocks.get(&block)?.content else {
            return None;
        };
        let row = rows.get(index)?;
        Some((
            &row.line,
            row.owner_tile < self.tile_start || row.owner_tile >= self.tile_end,
        ))
    }

    /// Translate an anchor's tile-local row offset using already prepared
    /// prefix geometry. A preceding paragraph may share this tile with a table.
    pub(crate) fn corrected_table_source(
        &self,
        document: &Document,
        view: super::table_window::TableView,
    ) -> Option<(TableId, usize, usize)> {
        let span = document.tile(view.anchor.tile)?;
        for status in &self.table_windows {
            let table = document.table(status.table_id)?;
            if table.source.start >= span.end || table.source.end <= span.start {
                continue;
            }
            let first = table
                .rows()
                .partition_point(|row| super::table_window::row_owner(row) < span.start);
            let row = table.rows().get(first)?;
            let index = view.anchor.tile.0.checked_sub(self.tile_start.0)?;
            let prefix: Vec<_> = self
                .work
                .get(index)?
                .iter()
                .filter(|item| {
                    item.source_start < row.source.start
                        || (item.source_start == row.source.start && item.rank < 2)
                })
                .copied()
                .collect();
            let counts = calculate_tile_rows(
                &self.events,
                &[prefix],
                &self.blocks,
                self.pending.as_deref(),
                self.width,
                view.anchor.tile,
                TileId(view.anchor.tile.0 + 1),
            )
            .ok()?;
            let offset = view.anchor.offset.saturating_sub(counts[0]);
            let at = super::table_window::row_from_offset(table, span.start, offset);
            return Some((status.table_id, at, table.rows()[at].source.start));
        }
        None
    }

    pub(crate) fn line_offset(&self, line: usize) -> Option<u64> {
        self.line_offsets.get(line).copied()
    }

    pub(crate) fn table_windows(&self) -> &[super::table::TableWindowStatus] {
        &self.table_windows
    }

    pub(crate) fn visible_lines(&self, skip: u64, rows: usize) -> Vec<SinkLine> {
        let start = self.line_offsets.partition_point(|&at| at < skip);
        self.lines[start..]
            .iter()
            .zip(&self.line_offsets[start..])
            .take_while(|(_, at)| **at < skip.saturating_add(rows as u64))
            .map(|(line, _)| line.clone())
            .collect()
    }

    #[must_use]
    pub fn stats(&self) -> LayoutSinkStats {
        self.stats
    }

    #[must_use]
    pub const fn checkpoint(&self) -> RenderCheckpoint {
        self.checkpoint
    }

    pub fn restore_checkpoint(&mut self, checkpoint: RenderCheckpoint) {
        self.checkpoint = checkpoint;
    }

    pub fn render_tui_owned_tile(&mut self, tile: TileId) -> u64 {
        self.render_tile(tile)
    }

    #[must_use]
    pub fn tile_rows(&self, tile: TileId) -> Option<u64> {
        let index = tile.0.checked_sub(self.tile_start.0)?;
        if tile >= self.tile_end {
            return None;
        }
        self.tile_rows.get(index).copied()
    }

    pub fn write_to_screen(&self, screen: &mut Screen, start_row: u16) -> Result<u16, SinkError> {
        let content = screen.content_area();
        for (offset, line) in self.lines.iter().enumerate() {
            let offset = u16::try_from(offset).map_err(|_| SinkError::RowOverflow)?;
            let row = start_row
                .checked_add(offset)
                .ok_or(SinkError::RowOverflow)?;
            let mut column = content.x;
            for cell in &line.cells {
                screen.write_cell(
                    row,
                    column,
                    Cell::new(
                        cell.text.clone(),
                        cell.style,
                        cell.width,
                        cell.link,
                        cell.source,
                    )?,
                )?;
                column = column
                    .checked_add(u16::from(cell.width))
                    .ok_or(SinkError::RowOverflow)?;
            }
            if let Some(source) = line.source {
                screen.set_line_source(row, source)?;
            }
        }
        u16::try_from(self.lines.len()).map_err(|_| SinkError::RowOverflow)
    }

    fn render_tile(&mut self, tile: TileId) -> u64 {
        let Some(index) = tile.0.checked_sub(self.tile_start.0) else {
            return 0;
        };
        if index >= self.work.len() || self.rendered_tiles[index] || tile >= self.tile_end {
            return 0;
        }
        self.rendered_tiles[index] = true;
        let before = self.logical_rows;
        let items = self.work[index].clone();
        for item in items {
            let line_start = self.lines.len();
            match item.kind {
                WorkKind::Event(event) => self.render_event(event),
                WorkKind::FinishStream(block) => self.render_stream_finish(block),
                WorkKind::FinishPending => self.render_finish_pending(),
                WorkKind::Row(block, row) => self.render_row(block, row),
                WorkKind::SkippedRows(count) => self.logical_rows += count,
            }
            for _ in line_start..self.lines.len() {
                self.line_offsets.push(self.logical_rows);
                self.logical_rows += 1;
            }
        }
        let rows = self.logical_rows.saturating_sub(before);
        self.stats.rows = self.stats.rows.saturating_add(rows);
        self.stats.tiles_rendered += 1;
        if let Some(checkpoint) = self.boundary_checkpoints.get(index) {
            self.checkpoint = *checkpoint;
        }
        rows
    }

    fn render_event(&mut self, event: usize) {
        self.stats.events_consumed += 1;
        let event = &self.events[event];
        match &event.kind {
            PlanEventKind::Spacing { blank_lines } => {
                for _ in 0..*blank_lines {
                    self.lines.push(SinkLine::default());
                }
            }
            PlanEventKind::CodeLabel { label } => {
                self.lines.push(text_line(
                    label,
                    StyleRole::META,
                    None,
                    event.display_source.unwrap_or(event.source),
                    event.block,
                    self.width,
                ));
            }
            PlanEventKind::ThematicBreak => {
                self.lines
                    .push(rule_line(event.source, event.block, self.width));
            }
        }
    }

    fn render_stream_finish(&mut self, block: BlockId) {
        let Some(prepared) = self.blocks.remove(&block) else {
            return;
        };
        match prepared.content {
            PreparedContent::Streamed { quote, glyphs } => {
                self.lines.extend(stream_lines(&glyphs, quote, self.width));
            }
            PreparedContent::Rows(_) => {}
        }
    }

    fn render_finish_pending(&mut self) {
        let Some(glyphs) = self.pending.take() else {
            return;
        };
        self.lines.extend(stream_lines(&glyphs, false, self.width));
    }

    fn render_row(&mut self, block: BlockId, index: usize) {
        let Some(prepared) = self.blocks.get(&block) else {
            return;
        };
        let PreparedContent::Rows(rows) = &prepared.content else {
            return;
        };
        if let Some(row) = rows.get(index) {
            self.lines.push(row.line.clone());
        }
    }
}

impl mdtui3::LayoutSink for TileLayoutSink {
    fn begin_tile(&mut self, tile: TileId) {
        self.current = Some(tile);
        self.stats.begin_tile_callbacks = self.stats.begin_tile_callbacks.saturating_add(1);
    }

    fn text(&mut self, _run: mdtui3::RenderRun<'_>) {
        if self.current.is_some() {
            self.stats.raw_text_callbacks = self.stats.raw_text_callbacks.saturating_add(1);
        }
    }

    fn object(&mut self, _id: LinkId, _object: &mdtui3::LinkInfo) {
        if self.current.is_some() {
            self.stats.object_callbacks = self.stats.object_callbacks.saturating_add(1);
        }
    }

    fn end_tile(&mut self, tile: TileId) -> u64 {
        let rows = if self.current == Some(tile) {
            self.render_tile(tile)
        } else {
            0
        };
        self.current = None;
        rows
    }
}

fn tile_index(plan: &BlockPlan, tile: TileId) -> Option<usize> {
    let index = tile.0.checked_sub(plan.tile_start.0)?;
    (tile < plan.tile_end && index < plan.segments.len()).then_some(index)
}

fn calculate_tile_rows(
    events: &[PlanEvent],
    work: &[Vec<WorkItem>],
    blocks: &HashMap<BlockId, PreparedBlock>,
    pending: Option<&[OwnedGlyph]>,
    width: u16,
    tile_start: TileId,
    tile_end: TileId,
) -> Result<Vec<u64>, SinkError> {
    let mut rows = vec![0_u64; work.len()];
    for (index, items) in work.iter().enumerate() {
        let tile = TileId(tile_start.0.saturating_add(index));
        if tile >= tile_end {
            break;
        }
        for item in items {
            let added = match item.kind {
                WorkKind::Event(event) => match &events
                    .get(event)
                    .ok_or_else(|| {
                        SinkError::Render("tile references a missing plan event".to_owned())
                    })?
                    .kind
                {
                    PlanEventKind::Spacing { blank_lines } => u64::from(*blank_lines),
                    PlanEventKind::CodeLabel { .. } | PlanEventKind::ThematicBreak => 1,
                },
                WorkKind::Row(_, _) => 1,
                WorkKind::SkippedRows(count) => count,
                WorkKind::FinishStream(block) => {
                    let prepared = blocks.get(&block).ok_or_else(|| {
                        SinkError::Render("tile references a missing block".to_owned())
                    })?;
                    let PreparedContent::Streamed { quote, glyphs } = &prepared.content else {
                        return Err(SinkError::Render(
                            "stream finish references row content".to_owned(),
                        ));
                    };
                    u64::try_from(stream_lines(glyphs, *quote, width).len())
                        .map_err(|_| SinkError::RowOverflow)?
                }
                WorkKind::FinishPending => {
                    let glyphs = pending.ok_or_else(|| {
                        SinkError::Render("tile references missing pending content".to_owned())
                    })?;
                    u64::try_from(stream_lines(glyphs, false, width).len())
                        .map_err(|_| SinkError::RowOverflow)?
                }
            };
            rows[index] = rows[index].saturating_add(added);
        }
    }
    Ok(rows)
}

fn owner_tile(document: &Document, source: Span, fallback: TileId) -> TileId {
    document
        .tile_at(source.end.saturating_sub(1))
        .unwrap_or(fallback)
}

fn block_glyphs(
    document: &Document,
    block: &mdtui3::Block,
    segments: Vec<&DisplaySegment>,
    id: BlockId,
) -> Vec<OwnedGlyph> {
    let BlockKind::Table { id: table_id } = block.kind else {
        return segments_glyphs(segments, Some(id));
    };
    let Some(table) = document.table(table_id) else {
        return Vec::new();
    };
    let mut output = Vec::new();
    let mut group = Vec::new();
    let mut previous = None;
    for segment in segments {
        let row = table.row_at(segment.source.start);
        let cell = row.and_then(|row| table.row(row)).map(|row| {
            row.cells
                .partition_point(|cell| cell.source.end <= segment.source.start)
        });
        let key = (row, cell);
        if !group.is_empty() && previous != Some(key) {
            output.extend(segments_glyphs(std::mem::take(&mut group), Some(id)));
        }
        group.push(segment);
        previous = Some(key);
    }
    output.extend(segments_glyphs(group, Some(id)));
    output
}

fn segments_glyphs(segments: Vec<&DisplaySegment>, block: Option<BlockId>) -> Vec<OwnedGlyph> {
    display_glyphs(segments.into_iter().map(|segment| GlyphInput {
        text: &segment.text,
        source: segment.source,
        replacement: segment.replacement,
        block,
        link: segment.link,
        style: segment.style,
        kind: segment.kind,
    }))
    .into_iter()
    .map(|glyph| OwnedGlyph {
        text: glyph.text,
        source: glyph.source,
        block: glyph.block,
        link: glyph.link,
        style: glyph.style,
        width: glyph.width,
        kind: glyph.kind,
    })
    .collect()
}

#[cfg(test)]
fn push_segment_glyphs_into(
    glyphs: &mut Vec<OwnedGlyph>,
    block: Option<BlockId>,
    segment: &DisplaySegment,
) {
    glyphs.extend(segments_glyphs(vec![segment], block));
}

fn prepare_block(
    context: &PrepareContext<'_>,
    block_id: BlockId,
    block: &mdtui3::Block,
    glyphs: Vec<OwnedGlyph>,
    _has_label: bool,
    has_rule: bool,
) -> Result<Option<PreparedContent>, SinkError> {
    let document = context.document;
    if crate::render::visibility::is_hidden_block(block) || block.kind == BlockKind::Blank {
        return Ok(None);
    }
    if matches!(block.kind, BlockKind::Math { .. }) {
        return Ok(None);
    }
    if has_rule {
        return Ok(None);
    }
    match &block.kind {
        BlockKind::Paragraph
            if is_item_continuation(document, block)
                && block
                    .container
                    .and_then(|id| document.container(id))
                    .is_some_and(|item| item.source.start >= context.plan_source.start) =>
        {
            Ok(None)
        }
        BlockKind::Paragraph => Ok(Some(PreparedContent::Streamed {
            quote: false,
            glyphs,
        })),
        BlockKind::Quote { .. } => Ok(Some(PreparedContent::Streamed {
            quote: true,
            glyphs,
        })),
        BlockKind::Heading { .. } => {
            let heading = heading_for_block(document, block_id)
                .ok_or_else(|| SinkError::Render("missing heading".to_owned()))?;
            let input = prepared_input(context, glyphs)?;
            let end = heading
                .content
                .start
                .saturating_add(4096)
                .min(heading.content.end);
            // These tokens are whole graphemes. Keep the final cluster atomic
            // even when the bounded title prefix ends inside its source range.
            let input = DisplayInput::from_tokens(
                input
                    .into_tokens()
                    .into_iter()
                    .filter(|token| {
                        token.source.end > heading.content.start && token.source.start < end
                    })
                    .collect(),
            );
            let mut screen = scratch_screen_rows(1, context.width)?;
            context.scratch(&screen);
            let rows = render_heading_input(heading, &input, &mut screen, 0)
                .map_err(|error| SinkError::Render(error.to_string()))?;
            let owner_source = if heading.source.len() > 4096 {
                Span::new(heading.source.start, heading.source.start + 1)
            } else {
                heading.source
            };
            let owner = owner_tile(document, owner_source, context.fallback_tile);
            Ok(Some(PreparedContent::Rows(prepared_rows(
                extract_rows(&screen, 0, Some(rows))?,
                owner,
                heading.source.start,
            ))))
        }
        BlockKind::ListItem { .. } => {
            let window = Span::new(
                block.source.start.max(context.plan_source.start),
                block.source.end.min(context.plan_source.end),
            );
            let item = block.container.and_then(|id| document.container(id));
            let full_item = item.is_none_or(|item| {
                item.source.start >= context.plan_source.start
                    && item.source.end <= context.plan_source.end
            });
            let visual = prepared_list_visual(document, block, context.containers)?;
            let input = prepared_input(context, glyphs)?;
            let display_bytes = input
                .tokens()
                .iter()
                .map(|token| token.text.len())
                .sum::<usize>();
            let capacity = usize::from(context.width)
                .saturating_sub(visual.continuation_indent)
                .max(1);
            let rows_bound = display_bytes.saturating_mul(2) / capacity + 4;
            let mut screen = scratch_screen_rows(
                u16::try_from(rows_bound).map_err(|_| SinkError::RowOverflow)?,
                context.width,
            )?;
            context.scratch(&screen);
            let rows = crate::render::list::render_list_input_with_visual(
                block_id,
                block,
                &input,
                visual,
                &mut screen,
                0,
            )
            .map_err(|error| SinkError::Render(error.to_string()))?;
            let owner = if full_item {
                list_item_owner_tile(document, block, context.fallback_tile)
            } else {
                owner_tile(document, window, context.fallback_tile)
            };
            Ok(Some(PreparedContent::Rows(prepared_rows(
                extract_rows(&screen, 0, Some(rows))?,
                owner,
                block.source.start,
            ))))
        }
        BlockKind::Code { .. } => {
            let input = prepared_input(context, glyphs)?;
            let lines = code_lines_from_input(&input, context.plan_source)
                .map_err(|error| SinkError::Render(error.to_string()))?;
            let line_count = lines.as_ref().map_or(0, Vec::len);
            let mut screen = scratch_screen_rows(
                u16::try_from(line_count.max(1)).map_err(|_| SinkError::RowOverflow)?,
                context.width,
            )?;
            context.scratch(&screen);
            let sources = lines
                .as_ref()
                .map(|lines| {
                    lines
                        .iter()
                        .map(|line| (line.source, line.end))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let rows = render_code_lines(block_id, block, None, lines, &mut screen, 0)
                .map_err(|error| SinkError::Render(error.to_string()))?;
            let rows = extract_rows(&screen, 0, Some(rows))?
                .into_iter()
                .zip(sources)
                .map(|(line, (source, end))| PreparedRow {
                    line,
                    owner_tile: owner_tile(document, end, context.fallback_tile),
                    source_start: source.start,
                })
                .collect();
            Ok(Some(PreparedContent::Rows(rows)))
        }
        BlockKind::Table { .. } => {
            let BlockKind::Table { id } = block.kind else {
                unreachable!()
            };
            let table = document
                .table(id)
                .ok_or_else(|| SinkError::Render("missing table".to_owned()))?;
            let row_window = context
                .table_rows
                .clone()
                .unwrap_or_else(|| table_row_window(table, context.plan_source));
            // Table tokens have already passed the paid per-cell visibility
            // preparation. This callback cannot consult remote source/guards.
            let input = DisplayInput::from_tokens(
                glyphs
                    .into_iter()
                    .map(|g| InputToken {
                        kind: TokenKind::Text,
                        text: g.text,
                        source: g.source,
                        replacement: Some(g.source),
                        block: g.block,
                        link: g.link,
                        style: g.style,
                    })
                    .collect(),
            );
            context.display(input.tokens().iter().map(|t| t.text.len()).sum());
            let mut widths = context.table_widths.get(&id).cloned().unwrap_or_else(|| {
                TableWidthCache::hint_snapshot_prefix(id, table, usize::from(context.width))
            });
            if row_window.start == 0
                && row_window.end == table.rows().len()
                && table.rows().iter().all(|row| {
                    row.cells
                        .iter()
                        .skip(widths.first_column)
                        .take(widths.widths.len())
                        .all(|cell| {
                            cell.source.len()
                                <= usize::from(context.width) * 4 + super::edges::MAX_EDGE_OVERSCAN
                        })
                })
            {
                for (column, width) in widths.widths.iter_mut().enumerate() {
                    *width = widths.policy.apply(
                        table
                            .rows()
                            .iter()
                            .filter_map(|row| row.cells.get(widths.first_column + column))
                            .map(|cell| {
                                crate::render::text::display_width(
                                    &input.clipped(cell.source).visible_text(),
                                )
                            })
                            .max()
                            .unwrap_or(0),
                    );
                }
                widths.provisional =
                    widths.first_column > 0 || widths.widths.len() < table.columns.len();
            }
            let geometry = table_geometry_window(
                document,
                block_id,
                block,
                usize::from(context.width),
                &widths.widths,
                row_window,
                widths.provisional,
                None,
                Some(&input),
                widths.first_column,
            )
            .map_err(|error| SinkError::Render(error.to_string()))?;
            context
                .table_windows
                .borrow_mut()
                .push(super::table::TableWindowStatus {
                    table_id: id,
                    first: geometry.first_column,
                    visible: geometry.visible_columns,
                    total: geometry.total_columns,
                    provisional: geometry.widths_provisional,
                });
            let mut screen = scratch_screen_rows(
                u16::try_from(geometry.rows.len().saturating_add(3))
                    .map_err(|_| SinkError::RowOverflow)?,
                context.width,
            )?;
            context.scratch(&screen);
            let rows = render_table_with_geometry(&geometry, &mut screen, 0)
                .map_err(|error| SinkError::Render(error.to_string()))?;
            let rows = extract_rows(&screen, 0, Some(rows))?
                .into_iter()
                .map(|line| {
                    let source = line.source.map_or(block.source, |source| source.source);
                    PreparedRow {
                        line,
                        owner_tile: owner_tile(
                            document,
                            if source.len() > 4096 {
                                Span::new(source.start, source.start + 1)
                            } else {
                                source
                            },
                            context.fallback_tile,
                        ),
                        source_start: source.start,
                    }
                })
                .collect();
            Ok(Some(PreparedContent::Rows(rows)))
        }
        _ => Ok(None),
    }
}

fn prepared_input(
    context: &PrepareContext<'_>,
    glyphs: Vec<OwnedGlyph>,
) -> Result<DisplayInput, SinkError> {
    let bytes = glyphs.iter().map(|glyph| glyph.text.len()).sum();
    context.display(bytes);
    let input = DisplayInput::from_tokens(
        glyphs
            .into_iter()
            .map(|glyph| InputToken {
                kind: TokenKind::Text,
                replacement: (glyph.text.len() != glyph.source.len()).then_some(glyph.source),
                text: glyph.text,
                source: glyph.source,
                block: glyph.block,
                link: glyph.link,
                style: glyph.style,
            })
            .collect(),
    );
    crate::render::visibility::visible_prepared_input(context.document, context.plan_source, input)
        .ok_or_else(|| SinkError::Render("invalid prepared visibility window".to_owned()))
}

fn prepared_rows(
    lines: Vec<SinkLine>,
    owner_tile: TileId,
    source_start: usize,
) -> Vec<PreparedRow> {
    lines
        .into_iter()
        .map(|line| PreparedRow {
            line,
            owner_tile,
            source_start,
        })
        .collect()
}

fn list_item_owner_tile(document: &Document, block: &mdtui3::Block, fallback: TileId) -> TileId {
    block
        .container
        .and_then(|container| document.container(container))
        .map_or_else(
            || owner_tile(document, block.source, fallback),
            |container| owner_tile(document, container.source, fallback),
        )
}

fn is_item_continuation(document: &Document, block: &mdtui3::Block) -> bool {
    block.kind == BlockKind::Paragraph
        && block
            .container
            .and_then(|container| document.container(container))
            .is_some_and(|container| matches!(container.kind, mdtui3::ContainerKind::Item { .. }))
}

fn heading_for_block(document: &Document, block: BlockId) -> Option<&mdtui3::Heading> {
    let headings = document.headings().entries();
    let index = headings.partition_point(|heading| heading.block < block);
    headings.get(index).filter(|heading| heading.block == block)
}

fn scratch_screen_rows(rows: u16, width: u16) -> Result<Screen, SinkError> {
    let rows = rows.max(1);
    Ok(Screen::new(rows, width, ContentArea::new(0, width))?)
}

fn planned_source(document: &Document, plan: &BlockPlan) -> Result<Span, SinkError> {
    if plan.tile_start.0 == plan.tile_end.0 {
        return Ok(Span::new(0, 0));
    }
    let start = document
        .tile(plan.tile_start)
        .ok_or(SinkError::IncompleteScan)?
        .start;
    let end = document
        .tile(TileId(plan.tile_end.0 - 1))
        .ok_or(SinkError::IncompleteScan)?
        .end;
    Ok(Span::new(start, end))
}

fn extract_rows(
    screen: &Screen,
    start: usize,
    count: Option<u16>,
) -> Result<Vec<SinkLine>, SinkError> {
    let count = count.unwrap_or(screen.rows().saturating_sub(start as u16));
    let content = screen.content_area();
    let mut rows = Vec::with_capacity(usize::from(count));
    for offset in 0..count {
        let row = u16::try_from(start).map_err(|_| SinkError::RowOverflow)? + offset;
        let mut cells = Vec::new();
        for column in content.x..content.right() {
            let cell = screen.cell(row, column).ok_or(SinkError::RowOverflow)?;
            if cell.is_continuation() {
                continue;
            }
            cells.push(SinkCell {
                text: cell.text().to_owned(),
                style: cell.style(),
                width: cell.width(),
                link: cell.link_id(),
                source: cell.source(),
            });
        }
        rows.push(SinkLine {
            cells,
            source: screen.line_source(row),
        });
    }
    Ok(rows)
}

fn stream_lines(glyphs: &[OwnedGlyph], quote: bool, width: u16) -> Vec<SinkLine> {
    let width = usize::from(width);
    if width == 0 {
        return Vec::new();
    }
    let Some(first) = glyphs.iter().position(|glyph| !trim_glyph(glyph)) else {
        return Vec::new();
    };
    let last = glyphs
        .iter()
        .rposition(|glyph| !trim_glyph(glyph))
        .map_or(first, |index| index + 1);
    let glyphs = &glyphs[first..last];
    if glyphs.is_empty() {
        return Vec::new();
    }

    let prefix = if quote {
        quote_prefix(width)
    } else {
        Vec::new()
    };
    let prefix_width: usize = prefix.iter().map(|cell| usize::from(cell.width)).sum();
    let body_width = width.saturating_sub(prefix_width);
    if body_width == 0 {
        if quote {
            let source = glyphs
                .iter()
                .find(|glyph| glyph.width > 0)
                .map(glyph_line_source);
            return vec![SinkLine {
                cells: prefix,
                source,
            }];
        }
        return Vec::new();
    }

    let mut rows = Vec::new();
    let mut line: Vec<SinkCell> = Vec::new();
    let mut line_width = 0_usize;
    let mut line_source = None;
    for glyph in glyphs {
        if glyph.kind == GraphemeKind::Control {
            continue;
        }
        if glyph.width == 0 {
            if let Some(last) = line.last_mut() {
                last.text.push_str(&glyph.text);
            }
            continue;
        }
        let glyph_width = usize::from(glyph.width);
        if glyph_width > body_width {
            continue;
        }
        if line_source.is_some() && line_width + glyph_width > body_width {
            rows.push(assemble_line(
                &prefix,
                std::mem::take(&mut line),
                line_source.take(),
            ));
            line_width = 0;
        }
        if line_source.is_none() {
            line_source = Some(glyph_line_source(glyph));
        }
        line.push(SinkCell {
            text: glyph.text.clone(),
            style: glyph.style,
            width: glyph.width,
            link: glyph.link,
            source: Some(glyph.source),
        });
        line_width += glyph_width;
    }
    if line_source.is_some() {
        rows.push(assemble_line(&prefix, line, line_source));
    }
    rows
}

fn glyph_line_source(glyph: &OwnedGlyph) -> LineSource {
    glyph.block.map_or_else(
        || LineSource::pending(glyph.source),
        |block| LineSource::new(glyph.source, block),
    )
}

fn assemble_line(prefix: &[SinkCell], body: Vec<SinkCell>, source: Option<LineSource>) -> SinkLine {
    let mut cells = prefix.to_vec();
    cells.extend(body);
    SinkLine { cells, source }
}

fn quote_prefix(width: usize) -> Vec<SinkCell> {
    if width == 0 {
        return Vec::new();
    }
    let mut cells = vec![SinkCell {
        text: "│".to_owned(),
        style: StyleRole::QUOTE,
        width: 1,
        link: None,
        source: None,
    }];
    if width >= 2 {
        cells.push(SinkCell {
            text: " ".to_owned(),
            style: StyleRole::QUOTE,
            width: 1,
            link: None,
            source: None,
        });
    }
    cells
}

fn trim_glyph(glyph: &OwnedGlyph) -> bool {
    glyph
        .text
        .chars()
        .all(|character| character.is_ascii_whitespace())
}

fn text_line(
    text: &str,
    style: StyleRole,
    link: Option<LinkId>,
    source: Span,
    block: BlockId,
    width: u16,
) -> SinkLine {
    let mut cells = Vec::new();
    let mut used = 0_usize;
    for grapheme in graphemes(text) {
        let grapheme_width = usize::from(grapheme.width);
        if grapheme_width > usize::from(width).saturating_sub(used) {
            break;
        }
        cells.push(SinkCell {
            text: grapheme.text.to_owned(),
            style,
            width: grapheme.width,
            link,
            source: None,
        });
        used += grapheme_width;
    }
    SinkLine {
        cells,
        source: Some(LineSource::new(source, block)),
    }
}

fn rule_line(source: Span, block: BlockId, width: u16) -> SinkLine {
    SinkLine {
        cells: (0..width)
            .map(|_| SinkCell {
                text: "─".to_owned(),
                style: StyleRole::RULE,
                width: 1,
                link: None,
                source: None,
            })
            .collect(),
        source: Some(LineSource::new(source, block)),
    }
}

fn prepared_list_visual(
    document: &Document,
    block: &mdtui3::Block,
    containers: &HashMap<mdtui3::ContainerId, ContainerContext>,
) -> Result<ListVisual, SinkError> {
    let visual = if let Some(context) = block.container.and_then(|id| containers.get(&id)) {
        crate::render::list::build_list_visual_with_indent(block, context.indent)
    } else {
        crate::render::list::build_list_visual(document, block)
    };
    visual.map_err(|error| SinkError::Render(error.to_string()))
}

fn checkpoint_for_block(
    document: &Document,
    block_id: BlockId,
    kind: &BlockKind,
    source: Span,
    containers: &HashMap<mdtui3::ContainerId, ContainerContext>,
) -> Result<RenderCheckpoint, SinkError> {
    let mut checkpoint = RenderCheckpoint::line_start();
    checkpoint.position = CheckpointPosition::BlockInner;
    checkpoint.block = Some(block_id);
    checkpoint.block_kind = checkpoint_kind(kind);
    checkpoint.line_source_start = Some(source.start);
    match kind {
        BlockKind::Heading { level } => checkpoint.heading_level = *level,
        BlockKind::ListItem { .. } => {
            let block = document
                .block(block_id)
                .ok_or_else(|| SinkError::Render("missing list item".to_owned()))?;
            let visual = prepared_list_visual(document, block, containers)?;
            apply_list_visual(&mut checkpoint, &visual);
        }
        BlockKind::Quote { .. } => checkpoint.quote_prefix = true,
        BlockKind::Code { .. } => checkpoint.code = CodeRenderState::Body,
        BlockKind::Table { .. } => checkpoint.table = TableRenderState::Body,
        _ => {}
    }
    Ok(checkpoint)
}

fn checkpoint_kind(kind: &BlockKind) -> CheckpointBlockKind {
    match kind {
        BlockKind::Paragraph => CheckpointBlockKind::Paragraph,
        BlockKind::Heading { .. } => CheckpointBlockKind::Heading,
        BlockKind::Quote { .. } => CheckpointBlockKind::Quote,
        BlockKind::ListItem { .. } => CheckpointBlockKind::ListItem,
        BlockKind::Code { .. } => CheckpointBlockKind::Code,
        BlockKind::Table { .. } => CheckpointBlockKind::Table,
        BlockKind::ThematicBreak => CheckpointBlockKind::ThematicBreak,
        BlockKind::Math { .. } => CheckpointBlockKind::Math,
        BlockKind::Blank => CheckpointBlockKind::Blank,
        BlockKind::Definition { .. } => CheckpointBlockKind::Definition,
        BlockKind::FootnoteDefinition { .. } => CheckpointBlockKind::FootnoteDefinition,
    }
}

fn apply_list_visual(checkpoint: &mut RenderCheckpoint, visual: &ListVisual) {
    checkpoint.list_indent = u16::try_from(visual.indent).unwrap_or(u16::MAX);
    checkpoint.list_marker_width = u8::try_from(visual.marker_width).unwrap_or(u8::MAX);
    checkpoint.list_body_indent = u16::try_from(visual.body_indent).unwrap_or(u16::MAX);
    checkpoint.list_continuation_indent =
        u16::try_from(visual.continuation_indent).unwrap_or(u16::MAX);
    checkpoint.list_marker = match visual.task_checked {
        Some(true) => ListMarkerState::TaskChecked,
        Some(false) => ListMarkerState::TaskUnchecked,
        None if visual.number.is_some() => ListMarkerState::Ordered,
        None => ListMarkerState::Bullet,
    };
}

fn boundary_checkpoints(
    plan: &BlockPlan,
    events: &[PlanEvent],
    blocks: &HashMap<BlockId, PreparedBlock>,
) -> Vec<RenderCheckpoint> {
    let mut checkpoints = vec![RenderCheckpoint::line_start(); plan.segments.len()];
    for (index, segment) in plan.segments.iter().enumerate() {
        let next_start = plan
            .segments
            .get(index + 1)
            .map_or(segment.source.end, |next| next.source.start);
        let mut checkpoint = RenderCheckpoint::line_start();
        if let Some(block) = plan
            .blocks
            .iter()
            .find(|block| block.source.start < next_start && block.source.end > segment.source.end)
        {
            checkpoint = blocks.get(&block.id).map_or_else(
                || {
                    let mut fallback =
                        RenderCheckpoint::block_inner(block.id, checkpoint_kind(&block.kind));
                    fallback.line_source_start = Some(block.source.start);
                    fallback
                },
                |block| block.checkpoint,
            );
        }
        if let Some(next) = plan.segments.get(index + 1) {
            let mut pending = PendingBlockEvents::default();
            for event in events {
                if event.source.start >= next.source.start && event.source.start < next.source.end {
                    match event.kind {
                        PlanEventKind::Spacing { blank_lines } => {
                            pending.spacing_rows =
                                pending.spacing_rows.saturating_add(blank_lines as u8);
                        }
                        PlanEventKind::CodeLabel { .. } => pending.code_label = true,
                        PlanEventKind::ThematicBreak => pending.thematic_break = true,
                    }
                }
            }
            pending.group_start = plan.blocks.iter().any(|block| {
                block.group_start
                    && block.source.start >= next.source.start
                    && block.source.start < next.source.end
            });
            checkpoint.pending = pending;
        }
        checkpoints[index] = checkpoint;
    }
    checkpoints
}

#[cfg(test)]
mod pending_metadata_tests {
    use super::*;
    use crate::render::display::DisplaySegment;
    use mdtui3::{LinkId, Span};

    #[test]
    fn synthetic_pending_run_preserves_style_link_wide_cell_and_optional_line_block() {
        let segment = DisplaySegment {
            tile: TileId(0),
            source: Span::new(10, 15),
            replacement: None,
            text: "🙂x".to_owned(),
            block: None,
            link: Some(LinkId(0)),
            style: StyleRole::STRONG,
            kind: TokenKind::Text,
        };
        let mut glyphs = Vec::new();
        push_segment_glyphs_into(&mut glyphs, None, &segment);
        let lines = stream_lines(&glyphs, false, 20);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].cells[0].text, "🙂");
        assert_eq!(lines[0].cells[0].width, 2);
        assert_eq!(lines[0].cells[0].style, StyleRole::STRONG);
        assert_eq!(lines[0].cells[0].link, Some(LinkId(0)));
        assert_eq!(lines[0].cells[1].text, "x");
        let source = lines[0].source.unwrap();
        assert_eq!(source.block, None);
        assert_eq!(source.source, Span::new(10, 14));
    }
}
