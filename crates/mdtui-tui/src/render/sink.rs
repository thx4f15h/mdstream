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
use crate::render::heading::{render_heading_input, render_live_heading_input};
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
    pub text: super::payload::TextPayload,
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
    pub math_object_callbacks: u64,
    pub document_queries_in_callbacks: u64,
    pub table_rows_prepared: usize,
    pub container_nodes_read: usize,
    /// Paid logical-object/predecessor queries performed by reading spacing.
    pub spacing_metadata_steps: usize,
    /// Source bytes requested by preparation in addition to DisplayScanner.
    /// Repeated code/list passes are counted separately.
    pub preparation_source_bytes: usize,
    pub prepared_display_bytes: usize,
    pub edge_placeholders: usize,
    pub scratch_capacity_cells: usize,
}

#[derive(Debug, Clone)]
struct OwnedGlyph {
    math: Option<std::rc::Rc<super::math::MathView>>,
    text: super::payload::TextPayload,
    width: u8,
    kind: GraphemeKind,
    style: StyleRole,
    link: Option<LinkId>,
    source: Span,
    block: Option<BlockId>,
}

#[derive(Debug, Clone)]
struct MathPlacement {
    id: mdtui3::MathObjectId,
    source: Span,
    top: u64,
    height: u64,
    viewport_width: usize,
}

#[derive(Debug, Clone)]
struct PreparedBlock {
    copy_offsets: Vec<u64>,
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
    MathRows {
        rows: Vec<PreparedRow>,
        gaps: Vec<u64>,
        placements: Vec<MathPlacement>,
    },
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
    PendingRow(usize),
    Row(BlockId, usize),
    SkippedRows(u64),
    MathOrigins(BlockId),
    MathObject(mdtui3::MathObjectMeta),
}

#[derive(Debug, Clone, Copy)]
struct WorkItem {
    source_start: usize,
    rank: u8,
    order: usize,
    kind: WorkKind,
}

#[derive(Clone)]
struct PrepareContext<'a> {
    document: &'a Document,
    width: u16,
    fallback_tile: TileId,
    plan_source: Span,
    table_widths: &'a HashMap<TableId, TableWidthSnapshot>,
    table_rows: Option<std::ops::Range<usize>>,
    source_cut: Option<usize>,
    table_row_ranges: Option<Vec<std::ops::Range<usize>>>,
    table_fold_tail: Option<Span>,
    table_windows: &'a std::cell::RefCell<Vec<super::table::TableWindowStatus>>,
    containers: &'a HashMap<mdtui3::ContainerId, ContainerContext>,
    display_bytes: &'a std::cell::Cell<usize>,
    scratch_cells: &'a std::cell::Cell<usize>,
    fragment_titles: bool,
    reading: bool,
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
    pending_rows: Vec<PreparedRow>,
    tile_rows: Vec<u64>,
    boundary_checkpoints: Vec<RenderCheckpoint>,
    lines: Vec<SinkLine>,
    line_offsets: Vec<u64>,
    logical_rows: u64,
    math_placements: std::collections::BTreeMap<mdtui3::MathObjectId, MathPlacement>,
    table_windows: Vec<super::table::TableWindowStatus>,
    rendered_tiles: Vec<bool>,
    current: Option<TileId>,
    tile_start: TileId,
    tile_end: TileId,
    tiles: Vec<TileId>,
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
        if !plan.projected && plan.tile_start.0 < plan.tile_end.0 {
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
                    && ((segment.source.start >= pending_visible.start
                        && segment.source.end <= pending_visible.end)
                        || segment.text.is_indexed()
                            && segment.source.end > pending_visible.start
                            && segment.source.start < pending_visible.end)
                {
                    pending_segments.push(segment);
                }
                continue;
            };
            let owner = plan.flow_owners.get(&block).copied().unwrap_or_else(|| {
                document
                    .block(block)
                    .filter(|block| block.kind == BlockKind::Paragraph)
                    .and_then(|block| block.container)
                    .and_then(|container| item_owners.get(&container))
                    .copied()
                    .unwrap_or(block)
            });
            segments_by_block.entry(owner).or_default().push(segment);
        }

        let fold_owners = scanner
            .inline_placeholders()
            .iter()
            .map(|p| ((p.candidate.hidden.start, p.candidate.hidden.end), p.owner))
            .collect::<std::collections::BTreeMap<_, _>>();
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
            if plan
                .flow_owners
                .get(&planned.id)
                .is_some_and(|owner| *owner != planned.id)
            {
                continue;
            }
            let mut flow_block = block.clone();
            if let Some(source) = plan.flow_sources.get(&planned.id) {
                flow_block.source = *source;
            }
            let block = &flow_block;
            let render_id = plan
                .flow_ids
                .get(&planned.id)
                .copied()
                .unwrap_or(planned.id);
            let has_label = plan.events.iter().any(|event| {
                event.block == Some(planned.id)
                    && matches!(event.kind, PlanEventKind::CodeLabel { .. })
            });
            let has_rule = plan.events.iter().any(|event| {
                event.block == Some(planned.id)
                    && matches!(event.kind, PlanEventKind::ThematicBreak)
            });
            let context = PrepareContext {
                document,
                width,
                fallback_tile: plan.tile_start,
                plan_source,
                table_widths,
                source_cut: scanner.source_cut(),
                table_fold_tail: if let BlockKind::Table { id } = block.kind {
                    document
                        .table(id)
                        .and_then(|t| t.rows().last())
                        .and_then(|last| {
                            plan.events
                                .iter()
                                .filter(|e| matches!(e.kind, PlanEventKind::FoldLabel { .. }))
                                .filter_map(|e| e.display_source)
                                .find(|s| s.start <= last.source.start && s.end >= last.source.end)
                        })
                } else {
                    None
                },
                table_row_ranges: if let BlockKind::Table { id } = block.kind {
                    scanner.selected_row_ranges(id)
                } else {
                    None
                },
                table_windows: &table_windows,
                table_rows: if let BlockKind::Table { id } = block.kind {
                    scanner.selected_rows(id)
                } else {
                    None
                },
                containers,
                display_bytes: &display_bytes,
                scratch_cells: &scratch_cells,
                fragment_titles: scanner.shared_links().fragment_titles(),
                reading: !plan.flow_owners.is_empty(),
            };
            let mut glyphs = block_glyphs(
                document,
                block,
                segments_by_block.remove(&planned.id).unwrap_or_default(),
                render_id,
                scanner.math_views(),
            );
            if matches!(block.kind, BlockKind::Table { .. }) {
                for glyph in &mut glyphs {
                    if !glyph.text.is_indexed()
                        && (glyph.text.len() > super::edges::MAX_EDGE_OVERSCAN
                            || scanner.cell_cut(glyph.source.end))
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
                if !glyph.text.is_indexed()
                    && (cut_start || cut_end)
                    && !matches!(block.kind, BlockKind::Table { .. })
                {
                    edge_placeholders += 1;
                    glyph.text = "…".into();
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
            if uncertain && !glyphs.is_empty() && !glyphs.iter().all(|g| g.text.is_indexed()) {
                edge_placeholders += 1;
                glyphs.clear();
                glyphs.push(OwnedGlyph {
                    math: None,
                    text: "…".into(),
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
            let Some(mut content) =
                prepare_block(&context, render_id, block, glyphs, has_label, has_rule)?
            else {
                continue;
            };
            if let PreparedContent::Streamed { quote, glyphs } = &content
                && (plan.projected || glyphs.iter().any(|g| g.text.is_indexed()))
            {
                content = PreparedContent::Rows(source_owned_rows(
                    document,
                    stream_lines(glyphs, *quote, width),
                    plan.tile_start,
                    &fold_owners,
                ));
            }
            if !matches!(block.kind, BlockKind::Table { .. })
                && let PreparedContent::Rows(rows) | PreparedContent::MathRows { rows, .. } =
                    &mut content
            {
                for row in rows {
                    if let Some(owner) = fold_row_owner(&row.line, &fold_owners) {
                        row.owner_tile = owner;
                    }
                }
            }
            if matches!(block.kind, BlockKind::Table { .. })
                && let PreparedContent::Rows(rows) | PreparedContent::MathRows { rows, .. } =
                    &content
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
                PreparedContent::Streamed { glyphs, .. } => {
                    let owner = if glyphs.len() == 1 && glyphs[0].text.is_indexed() {
                        document
                            .tile_at(glyphs[0].source.start)
                            .filter(|tile| tile_index(plan, *tile).is_some())
                            .unwrap_or(plan.tile_start)
                    } else {
                        owner_tile(
                            document,
                            Span::new(block.source.start, block.source.end.min(plan_source.end)),
                            plan.tile_start,
                        )
                    };
                    let owner = projected_owner(plan, owner);
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
                PreparedContent::Rows(rows) | PreparedContent::MathRows { rows, .. } => {
                    if matches!(content, PreparedContent::MathRows { .. }) {
                        let owner = rows.first().map_or(plan.tile_start, |row| row.owner_tile);
                        let origin = rows
                            .first()
                            .map_or(block.source.start, |row| row.source_start);
                        if let Some(index) = tile_index(plan, projected_owner(plan, owner)) {
                            work[index].push(WorkItem {
                                source_start: origin,
                                rank: 2,
                                order,
                                kind: WorkKind::MathOrigins(planned.id),
                            });
                            order += 1;
                        }
                    }
                    for (row_index, row) in rows.iter().enumerate() {
                        let Some(index) = tile_index(plan, projected_owner(plan, row.owner_tile))
                        else {
                            continue;
                        };
                        if let PreparedContent::MathRows { gaps, .. } = &content {
                            work[index].push(WorkItem {
                                source_start: row.source_start,
                                rank: 2,
                                order,
                                kind: WorkKind::SkippedRows(gaps[row_index]),
                            });
                            order += 1;
                        }
                        work[index].push(WorkItem {
                            source_start: row.source_start,
                            rank: 2,
                            order,
                            kind: WorkKind::Row(planned.id, row_index),
                        });
                        order += 1;
                    }
                    if let PreparedContent::MathRows { gaps, .. } = &content {
                        let owner = rows.last().map_or(plan.tile_start, |row| row.owner_tile);
                        let origin = rows
                            .last()
                            .map_or(block.source.start, |row| row.source_start);
                        if let Some(index) = tile_index(plan, projected_owner(plan, owner)) {
                            work[index].push(WorkItem {
                                source_start: origin,
                                rank: 2,
                                order,
                                kind: WorkKind::SkippedRows(*gaps.last().unwrap()),
                            });
                            order += 1;
                        }
                    }
                }
            }
            let copy_offsets = if let PreparedContent::MathRows { rows, gaps, .. } = &content {
                let mut at = 0;
                rows.iter()
                    .enumerate()
                    .map(|(i, _)| {
                        at += gaps[i];
                        let offset = at;
                        at += 1;
                        offset
                    })
                    .collect()
            } else {
                Vec::new()
            };
            blocks.insert(
                planned.id,
                PreparedBlock {
                    copy_offsets,
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

        let pending_glyphs = math_segments_glyphs(pending_segments, None, scanner.math_views());
        let mut pending_rows = Vec::new();
        let pending = if plan.projected && !pending_glyphs.is_empty() {
            pending_rows = source_owned_rows(
                document,
                stream_lines(&pending_glyphs, false, width),
                plan.tile_start,
                &fold_owners,
            );
            for (row, prepared) in pending_rows.iter().enumerate() {
                if let Some(index) = tile_index(plan, projected_owner(plan, prepared.owner_tile)) {
                    work[index].push(WorkItem {
                        source_start: prepared.source_start,
                        rank: 3,
                        order,
                        kind: WorkKind::PendingRow(row),
                    });
                    order += 1;
                }
            }
            None
        } else if pending_glyphs.is_empty() {
            None
        } else {
            let source_owner = if pending_glyphs.len() == 1 && pending_glyphs[0].text.is_indexed() {
                pending_glyphs[0].source.start
            } else {
                pending_visible.end.saturating_sub(1)
            };
            let owner = document
                .tile_at(source_owner)
                .filter(|tile| tile_index(plan, *tile).is_some())
                .unwrap_or(plan.tile_start);
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

        for view in scanner.math_views().values() {
            if let Some(tile) = document.tile_at(view.meta.source.start)
                && plan
                    .segment(tile)
                    .is_some_and(|s| s.kind == super::plan::TileSegmentKind::TuiOwned)
                && let Some(index) = tile_index(plan, tile)
            {
                work[index].push(WorkItem {
                    source_start: view.meta.source.start,
                    rank: 1,
                    order,
                    kind: WorkKind::MathObject(view.meta),
                });
                order += 1;
            }
        }
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

        let (tile_rows, math_placements) = calculate_tile_rows(
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
            pending_rows,
            tile_rows,
            boundary_checkpoints,
            lines: Vec::new(),
            line_offsets: Vec::new(),
            logical_rows: 0,
            math_placements,
            table_windows: table_windows.into_inner(),
            rendered_tiles: vec![false; plan.segments.len()],
            current: None,
            tile_start: plan.tile_start,
            tile_end: plan.tile_end,
            tiles: plan.segments.iter().map(|s| s.tile).collect(),
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

    pub(crate) fn math_rectangle_after(
        &self,
        after: Option<mdtui3::MathObjectId>,
        view: super::table_window::TableView,
        width: usize,
    ) -> Option<(mdtui3::MathObjectId, super::math::Rect, Option<usize>)> {
        use std::ops::Bound::{Excluded, Unbounded};
        let (_, p) = self
            .math_placements
            .range((after.map_or(Unbounded, Excluded), Unbounded))
            .next()?;
        let prefix = self
            .tile_rows
            .iter()
            .take(view.anchor.tile.0.saturating_sub(self.tile_start.0))
            .sum::<u64>();
        let top = if view.tail.is_some() {
            self.tile_rows
                .iter()
                .sum::<u64>()
                .saturating_sub(view.rows as u64)
        } else {
            prefix + view.anchor.offset
        };
        let start = top.saturating_sub(p.top).min(p.height);
        let end = top
            .saturating_add(view.rows as u64)
            .saturating_sub(p.top)
            .min(p.height);
        let source = view
            .source
            .filter(|&at| p.source.start <= at && at < p.source.end);
        Some((
            p.id,
            super::math::Rect {
                x: 0,
                y: start as usize,
                width: width.min(p.viewport_width),
                height: if source.is_some() {
                    view.rows
                } else {
                    end.saturating_sub(start) as usize
                },
            },
            source,
        ))
    }

    pub(crate) fn math_row(&self, id: mdtui3::MathObjectId, row: usize) -> Option<u64> {
        let placement = self.math_placements.get(&id)?;
        ((row as u64) < placement.height).then(|| placement.top + row as u64)
    }

    pub(crate) fn prepared_copy_row_offset(&self, block: BlockId, index: usize) -> Option<u64> {
        let prepared = self.blocks.get(&block)?;
        if let PreparedContent::MathRows { .. } = &prepared.content {
            prepared.copy_offsets.get(index).copied()
        } else {
            Some(index as u64)
        }
    }
    pub(crate) fn copy_logical_rows(&self) -> u64 {
        self.math_placements
            .values()
            .map(|p| p.top + p.height)
            .max()
            .unwrap_or(0)
            .max(self.logical_rows)
    }
    /// Already prepared rows can belong to a physical tile outside the copy
    /// window. Reading them preserves table ownership without expanding the
    /// window across an arbitrarily large row envelope.
    pub(crate) fn prepared_copy_row(
        &self,
        block: BlockId,
        index: usize,
    ) -> Option<(&SinkLine, bool)> {
        let rows = match &self.blocks.get(&block)?.content {
            PreparedContent::Rows(rows) | PreparedContent::MathRows { rows, .. } => rows,
            _ => return None,
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
            let index = self.tiles.binary_search(&view.anchor.tile).ok()?;
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
            let offset = view.anchor.offset.saturating_sub(counts.0[0]);
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
        self.current = Some(tile);
        let rows = self.render_tile(tile);
        self.current = None;
        rows
    }

    #[must_use]
    pub(crate) fn tile_rendered(&self, tile: TileId) -> bool {
        self.tiles
            .binary_search(&tile)
            .ok()
            .and_then(|at| self.rendered_tiles.get(at))
            .copied()
            .unwrap_or(false)
    }

    pub fn tile_rows(&self, tile: TileId) -> Option<u64> {
        let index = self.tiles.binary_search(&tile).ok()?;
        if tile >= self.tile_end {
            return None;
        }
        self.tile_rows.get(index).copied()
    }

    pub fn write_to_screen(&self, screen: &mut Screen, start_row: u16) -> Result<u16, SinkError> {
        let content = screen.content_area();
        for (offset, line) in self.line_offsets.iter().zip(&self.lines) {
            let offset = u16::try_from(*offset).map_err(|_| SinkError::RowOverflow)?;
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
        u16::try_from(self.logical_rows).map_err(|_| SinkError::RowOverflow)
    }

    fn render_tile(&mut self, tile: TileId) -> u64 {
        let Ok(index) = self.tiles.binary_search(&tile) else {
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
                WorkKind::PendingRow(row) => self.lines.push(self.pending_rows[row].line.clone()),
                WorkKind::Row(block, row) => self.render_row(block, row),
                WorkKind::SkippedRows(count) => self.logical_rows += count,
                WorkKind::MathOrigins(_) => {}
                WorkKind::MathObject(meta) => mdtui3::LayoutSink::math_object(self, meta),
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
            PlanEventKind::FoldLabel { label, summary } => {
                let label = if self.width < 3 { "+" } else { label.as_str() };
                for text in std::iter::once(label).chain(summary.iter().map(String::as_str)) {
                    let source = event.display_source.unwrap_or(event.source);
                    let mut line =
                        text_line(text, StyleRole::META, None, source, event.block, self.width);
                    for cell in &mut line.cells {
                        cell.source = Some(source);
                    }
                    self.lines.push(line);
                }
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
            PreparedContent::Rows(_) | PreparedContent::MathRows { .. } => {}
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
        let rows = match &prepared.content {
            PreparedContent::Rows(rows) | PreparedContent::MathRows { rows, .. } => rows,
            _ => return,
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

    fn math_object(&mut self, _object: mdtui3::MathObjectMeta) {
        if self.current.is_some() {
            self.stats.math_object_callbacks += 1;
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

fn projected_owner(plan: &BlockPlan, owner: TileId) -> TileId {
    if !plan.projected || plan.segment(owner).is_some() {
        return owner;
    }
    plan.segments
        .get(
            plan.segments
                .partition_point(|s| s.tile <= owner)
                .saturating_sub(1),
        )
        .map_or(plan.tile_start, |s| s.tile)
}

fn tile_index(plan: &BlockPlan, tile: TileId) -> Option<usize> {
    plan.segments
        .binary_search_by_key(&tile, |segment| segment.tile)
        .ok()
}

fn calculate_tile_rows(
    events: &[PlanEvent],
    work: &[Vec<WorkItem>],
    blocks: &HashMap<BlockId, PreparedBlock>,
    pending: Option<&[OwnedGlyph]>,
    width: u16,
    _tile_start: TileId,
    _tile_end: TileId,
) -> Result<
    (
        Vec<u64>,
        std::collections::BTreeMap<mdtui3::MathObjectId, MathPlacement>,
    ),
    SinkError,
> {
    let mut rows = vec![0_u64; work.len()];
    let mut placements = std::collections::BTreeMap::new();
    let mut total = 0u64;
    for (index, items) in work.iter().enumerate() {
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
                    PlanEventKind::FoldLabel { summary, .. } => 1 + summary.len() as u64,
                },
                WorkKind::Row(_, _) | WorkKind::PendingRow(_) => 1,
                WorkKind::SkippedRows(count) => count,
                WorkKind::MathObject(_) => 0,
                WorkKind::MathOrigins(id) => {
                    if let PreparedContent::MathRows {
                        placements: local, ..
                    } = &blocks[&id].content
                    {
                        for p in local {
                            let mut p = p.clone();
                            p.top = p.top.checked_add(total).ok_or(SinkError::RowOverflow)?;
                            placements.insert(p.id, p);
                        }
                    }
                    0
                }
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
            total = total.checked_add(added).ok_or(SinkError::RowOverflow)?;
            rows[index] = rows[index].saturating_add(added);
        }
    }
    Ok((rows, placements))
}

fn source_owned_rows(
    document: &Document,
    lines: Vec<SinkLine>,
    fallback: TileId,
    folds: &std::collections::BTreeMap<(usize, usize), TileId>,
) -> Vec<PreparedRow> {
    lines
        .into_iter()
        .map(|line| {
            let start = line.source.map_or(0, |s| s.source.start);
            let owner = fold_row_owner(&line, folds)
                .or_else(|| document.tile_at(start))
                .unwrap_or(fallback);
            PreparedRow {
                line,
                owner_tile: owner,
                source_start: start,
            }
        })
        .collect()
}

fn fold_row_owner(
    line: &SinkLine,
    folds: &std::collections::BTreeMap<(usize, usize), TileId>,
) -> Option<TileId> {
    line.cells
        .iter()
        .filter_map(|cell| cell.source)
        .find_map(|source| folds.get(&(source.start, source.end)))
        .copied()
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
    math: &std::collections::BTreeMap<usize, std::rc::Rc<super::math::MathView>>,
) -> Vec<OwnedGlyph> {
    let BlockKind::Table { id: table_id } = block.kind else {
        return math_segments_glyphs(segments, Some(id), math);
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
            output.extend(math_segments_glyphs(
                std::mem::take(&mut group),
                Some(id),
                math,
            ));
        }
        group.push(segment);
        previous = Some(key);
    }
    output.extend(math_segments_glyphs(group, Some(id), math));
    output
}

fn math_segments_glyphs(
    segments: Vec<&DisplaySegment>,
    block: Option<BlockId>,
    math: &std::collections::BTreeMap<usize, std::rc::Rc<super::math::MathView>>,
) -> Vec<OwnedGlyph> {
    let mut result = Vec::new();
    let mut group = Vec::new();
    for segment in segments {
        if let Some(view) = math.get(&segment.source.start) {
            result.extend(segments_glyphs(std::mem::take(&mut group), block));
            result.push(OwnedGlyph {
                math: Some(view.clone()),
                text: Default::default(),
                width: 0,
                kind: GraphemeKind::ZeroWidth,
                style: segment.style,
                link: segment.link,
                source: segment.source,
                block,
            });
        } else {
            group.push(segment)
        }
    }
    result.extend(segments_glyphs(group, block));
    result
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
        math: None,
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
    if glyphs.iter().any(|g| g.math.is_some()) && !matches!(block.kind, BlockKind::Table { .. }) {
        return Ok(Some(math_flow(context, block_id, block, &glyphs)?));
    }
    if has_rule {
        return Ok(None);
    }
    match &block.kind {
        BlockKind::Paragraph
            if !context.reading
                && is_item_continuation(document, block)
                && block
                    .container
                    .and_then(|id| document.container(id))
                    .is_some_and(|item| item.source.start >= context.plan_source.start) =>
        {
            Ok(None)
        }
        BlockKind::Paragraph if context.reading && is_item_continuation(document, block) => {
            prepare_list_paragraph(context, block_id, block, glyphs, true)
        }
        BlockKind::Paragraph => Ok(Some(PreparedContent::Streamed {
            quote: context.reading
                && block
                    .container
                    .and_then(|id| document.container(id))
                    .is_some_and(|c| matches!(c.kind, mdtui3::ContainerKind::Quote)),
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
            let render = if context.fragment_titles {
                render_live_heading_input
            } else {
                render_heading_input
            };
            let rows = render(heading, &input, &mut screen, 0)
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
            prepare_list_paragraph(context, block_id, block, glyphs, false)
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
        BlockKind::Table { .. } if glyphs.iter().any(|g| g.math.is_some()) => {
            Ok(Some(math_table(context, block_id, block, &glyphs)?))
        }
        BlockKind::Table { .. } => {
            let BlockKind::Table { id } = block.kind else {
                unreachable!()
            };
            let table = document
                .table(id)
                .ok_or_else(|| SinkError::Render("missing table".to_owned()))?;
            let row_windows = context.table_row_ranges.clone().unwrap_or_else(|| {
                vec![
                    context
                        .table_rows
                        .clone()
                        .unwrap_or_else(|| table_row_window(table, context.plan_source)),
                ]
            });
            let row_window = row_windows.first().cloned().unwrap_or(0..0);
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
            if context.table_row_ranges.is_none()
                && row_window.start == 0
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
                            .map(|cell| input.clipped(cell.source).display_width())
                            .max()
                            .unwrap_or(0),
                    );
                }
                widths.provisional =
                    widths.first_column > 0 || widths.widths.len() < table.columns.len();
            }
            let mut prepared = Vec::new();
            let needs_fold_bottom = table.rows().len() > 1
                && row_windows
                    .last()
                    .is_some_and(|r| r.end < table.rows().len());
            let mut fold_bottom = None;
            for row_window in row_windows {
                let mut geometry = table_geometry_window(
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
                if let Some(cut) = context.source_cut {
                    for row in &mut geometry.rows {
                        let original_end = row.source.end;
                        row.source.end = row.source.end.min(cut).max(row.source.start);
                        row.complete &= row.source.end == original_end;
                    }
                }
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
                let rows: Vec<_> = extract_rows(&screen, 0, Some(rows))?
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
                prepared.extend(rows);
                if needs_fold_bottom && let Some(source) = context.table_fold_tail {
                    let mut screen = scratch_screen_rows(1, context.width)?;
                    super::table::render_folded_bottom(&geometry, source, &mut screen)
                        .map_err(|error| SinkError::Render(error.to_string()))?;
                    let mut rows = extract_rows(&screen, 0, Some(1))?;
                    fold_bottom = rows.pop().map(|line| PreparedRow {
                        line,
                        owner_tile: document
                            .tile_at(source.start)
                            .unwrap_or(context.fallback_tile),
                        source_start: source.end.saturating_sub(1),
                    });
                }
            }
            if let Some(bottom) = fold_bottom {
                prepared.push(bottom);
            }
            Ok(Some(PreparedContent::Rows(prepared)))
        }
        _ => Ok(None),
    }
}

fn math_table(
    context: &PrepareContext<'_>,
    block_id: BlockId,
    block: &mdtui3::Block,
    glyphs: &[OwnedGlyph],
) -> Result<PreparedContent, SinkError> {
    use std::collections::BTreeMap;
    let BlockKind::Table { id } = block.kind else {
        unreachable!()
    };
    let table = context
        .document
        .table(id)
        .ok_or_else(|| SinkError::Render("missing table".into()))?;
    let range = context
        .table_rows
        .clone()
        .unwrap_or_else(|| table_row_window(table, context.plan_source));
    let mut widths = context.table_widths.get(&id).cloned().unwrap_or_else(|| {
        TableWidthCache::hint_snapshot_prefix(id, table, usize::from(context.width))
    });
    let cell_glyphs = |source: Span| {
        let start = glyphs.partition_point(|g| g.source.start < source.start);
        let end = glyphs.partition_point(|g| g.source.start < source.end);
        &glyphs[start..end]
    };
    for (col, w) in widths.widths.iter_mut().enumerate() {
        let column = widths.first_column + col;
        let measured = table.rows()[range.clone()]
            .iter()
            .filter_map(|row| row.cells.get(column))
            .map(|cell| {
                cell_glyphs(cell.source)
                    .iter()
                    .map(|g| {
                        g.math
                            .as_ref()
                            .map_or(usize::from(g.width), |v| v.metrics.width)
                    })
                    .sum()
            })
            .max()
            .unwrap_or(0);
        *w = widths.policy.apply(measured);
    }
    let input = DisplayInput::from_tokens(Vec::new());
    let geometry = table_geometry_window(
        context.document,
        block_id,
        block,
        usize::from(context.width),
        &widths.widths,
        range.clone(),
        widths.provisional,
        None,
        Some(&input),
        widths.first_column,
    )
    .map_err(|e| SinkError::Render(e.to_string()))?;
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
    let mut output: BTreeMap<usize, (SinkLine, Span)> = BTreeMap::new();
    let border = |left, junction, right, fill| -> Result<SinkLine, SinkError> {
        Ok(SinkLine {
            cells: super::table::horizontal_border(left, junction, right, fill, &geometry.columns)
                .map_err(|e| SinkError::Render(e.to_string()))?
                .into_iter()
                .map(|c| SinkCell {
                    text: c.text().clone(),
                    width: c.width(),
                    style: c.style(),
                    link: c.link_id(),
                    source: c.source(),
                })
                .collect(),
            source: None,
        })
    };
    let mut y = 0usize;
    let mut placements = Vec::new();
    for (ri, row) in geometry.rows.iter().enumerate() {
        if range.start == 0 && ri == 0 {
            output.insert(y, (border('┌', '┬', '┐', '─')?, row.source));
            y += 1;
        }
        let mut cells = Vec::new();
        let mut ascent = 0usize;
        let mut descent = 0usize;
        for (ci, column) in geometry.columns.iter().enumerate() {
            let source = table.rows()[range.start + ri]
                .cells
                .get(widths.first_column + ci)
                .map_or(Span::new(row.source.end, row.source.end), |c| c.source);
            let group = cell_glyphs(source);
            let mut local = context.clone();
            local.width =
                u16::try_from(column.content_width.max(1)).map_err(|_| SinkError::RowOverflow)?;
            let plain = mdtui3::Block {
                kind: BlockKind::Paragraph,
                ..block.clone()
            };
            let PreparedContent::MathRows {
                rows,
                gaps,
                placements: local_placements,
            } = math_flow(&local, block_id, &plain, group)?
            else {
                unreachable!()
            };
            let mut at = 0usize;
            let mut indexed = BTreeMap::new();
            for (i, r) in rows.into_iter().enumerate() {
                at += gaps[i] as usize;
                indexed.insert(at, r.line);
                at += 1;
            }
            at += *gaps.last().unwrap() as usize;
            let base = group
                .iter()
                .find_map(|g| g.math.as_ref().map(|v| v.metrics.baseline))
                .unwrap_or(0);
            ascent = ascent.max(base);
            descent = descent.max(at.max(1).saturating_sub(base + 1));
            cells.push((base, indexed, local_placements));
        }
        let height = ascent + descent + 1;
        let mut offsets = std::collections::BTreeSet::new();
        for (base, rows, local) in &cells {
            for p in local {
                let mut p = p.clone();
                p.top += (y + ascent - base) as u64;
                placements.push(p);
            }
            for at in rows.keys() {
                offsets.insert(ascent - base + at);
            }
        }
        if offsets.is_empty() {
            offsets.insert(0);
        }
        for offset in offsets {
            let mut out = vec![SinkCell {
                text: "│".into(),
                width: 1,
                style: StyleRole::TABLE_BORDER,
                ..Default::default()
            }];
            for ((base, rows, _), column) in cells.iter().zip(&geometry.columns) {
                let line = offset
                    .checked_sub(ascent - base)
                    .and_then(|at| rows.get(&at));
                let used = line.map_or(0, |l| {
                    l.cells.iter().map(|c| usize::from(c.width)).sum::<usize>()
                });
                let slack = column.content_width.saturating_sub(used);
                let leading = match column.alignment {
                    mdtui3::Alignment::Right => slack,
                    mdtui3::Alignment::Center => slack / 2,
                    _ => 0,
                };
                for _ in 0..leading + 1 {
                    out.push(SinkCell {
                        text: " ".into(),
                        width: 1,
                        ..Default::default()
                    });
                }
                if let Some(line) = line {
                    out.extend(line.cells.iter().cloned());
                }
                for _ in 0..slack - leading + 1 {
                    out.push(SinkCell {
                        text: " ".into(),
                        width: 1,
                        ..Default::default()
                    });
                }
                out.push(SinkCell {
                    text: "│".into(),
                    width: 1,
                    style: StyleRole::TABLE_BORDER,
                    ..Default::default()
                });
            }
            output.insert(
                y + offset,
                (
                    SinkLine {
                        cells: out,
                        source: Some(LineSource::new(row.source, block_id)),
                    },
                    row.source,
                ),
            );
        }
        y = y.checked_add(height).ok_or(SinkError::RowOverflow)?;
        if range.start == 0 && ri == 0 {
            output.insert(y, (border('╞', '╪', '╡', '═')?, row.source));
            y += 1;
        }
    }
    if range.end == table.rows().len() && table.rows().len() > 1 {
        let source = geometry.rows.last().map_or(block.source, |r| r.source);
        output.insert(y, (border('└', '┴', '┘', '─')?, source));
        y += 1;
    }
    let mut rows = Vec::new();
    let mut gaps = Vec::new();
    let mut at = 0;
    for (offset, (mut line, source)) in output {
        // The native table policy permits an oversized first column and clips
        // its complete cells at the viewport edge, including border/padding.
        let mut used = 0usize;
        let keep = line
            .cells
            .iter()
            .take_while(|cell| {
                let end = used + usize::from(cell.width);
                if end > usize::from(context.width) {
                    return false;
                }
                used = end;
                true
            })
            .count();
        line.cells.truncate(keep);
        gaps.push((offset - at) as u64);
        at = offset + 1;
        line.source.get_or_insert(LineSource::new(source, block_id));
        rows.push(PreparedRow {
            line,
            owner_tile: table
                .row_at(source.start)
                .and_then(|at| table.row(at))
                .and_then(|row| {
                    context
                        .document
                        .tile_at(super::table_window::row_owner(row))
                })
                .unwrap_or(context.fallback_tile),
            source_start: source.start,
        });
    }
    gaps.push(y.saturating_sub(at) as u64);
    Ok(PreparedContent::MathRows {
        rows,
        gaps,
        placements,
    })
}

/// Baseline-aligned atomic boxes. Only requested math rows are materialized;
/// gaps retain their intrinsic height in the same tile-height work stream.
fn math_flow(
    context: &PrepareContext<'_>,
    block_id: BlockId,
    block: &mdtui3::Block,
    glyphs: &[OwnedGlyph],
) -> Result<PreparedContent, SinkError> {
    use std::collections::BTreeMap;
    let quote = matches!(block.kind, BlockKind::Quote { .. })
        || block
            .container
            .and_then(|id| context.document.container(id))
            .is_some_and(|c| matches!(c.kind, mdtui3::ContainerKind::Quote));
    let list = if matches!(block.kind, BlockKind::ListItem { .. }) {
        Some(prepared_list_visual(
            context.document,
            block,
            context.containers,
        )?)
    } else if context.reading && is_item_continuation(context.document, block) {
        Some(continuation_visual(context, block)?)
    } else {
        None
    };
    let inherited = if matches!(block.kind, BlockKind::Math { .. }) {
        block
            .container
            .and_then(|id| context.containers.get(&id))
            .map_or(0, |c| c.body_indent())
    } else {
        0
    };
    let quote_cells = if quote {
        quote_prefix(usize::from(context.width))
    } else {
        Vec::new()
    };
    let first_prefix = list
        .as_ref()
        .map_or(quote_cells.len() + inherited, |v| v.body_indent)
        .min(usize::from(context.width));
    let continued_prefix = list
        .as_ref()
        .map_or(first_prefix, |v| v.continuation_indent)
        .min(usize::from(context.width));
    let mut prefix_width = first_prefix;
    let mut width = usize::from(context.width).saturating_sub(prefix_width);
    let mut first_baseline = None;
    let mut first_height = None;
    let mut placed: BTreeMap<usize, Vec<(usize, SinkCell)>> = BTreeMap::new();
    let mut placements = Vec::new();
    let mut line: Vec<(&OwnedGlyph, usize)> = Vec::new();
    let mut x = 0;
    let mut y = 0usize;
    let mut ascent = 0usize;
    let mut descent = 0usize;
    let flush = |line: &mut Vec<(&OwnedGlyph, usize)>,
                 y: usize,
                 ascent: usize,
                 placed: &mut BTreeMap<usize, Vec<(usize, SinkCell)>>,
                 placements: &mut Vec<MathPlacement>,
                 prefix_width: usize,
                 width: usize| {
        for (g, x) in line.drain(..) {
            if let Some(view) = &g.math {
                let top = y + ascent - view.metrics.baseline;
                placements.push(MathPlacement {
                    id: view.meta.id,
                    source: view.meta.source,
                    top: top as u64,
                    height: view.metrics.height as u64,
                    viewport_width: width.saturating_sub(x),
                });
                let visible_end = view
                    .rect
                    .y
                    .saturating_add(view.rect.height)
                    .min(view.metrics.height);
                for row in view.rect.y..visible_end {
                    placed.entry(top + row).or_default();
                }
                for c in view.cells.iter() {
                    let cx = c.x.saturating_sub(view.rect.x);
                    if x + cx + usize::from(c.width) <= width && x + cx < width {
                        placed.entry(top + c.y).or_default().push((
                            prefix_width + x + cx,
                            SinkCell {
                                text: c.text.clone(),
                                style: if let BlockKind::Heading { level } = block.kind {
                                    StyleRole::heading(level).with(StyleRole::MATH)
                                } else {
                                    g.style.with(
                                        list.as_ref().map_or(StyleRole::DEFAULT, |v| v.body_style),
                                    )
                                },
                                width: c.width,
                                link: g.link,
                                source: Some(c.source),
                            },
                        ));
                    }
                }
            } else {
                placed.entry(y + ascent).or_default().push((
                    prefix_width + x,
                    SinkCell {
                        text: g.text.clone(),
                        style: if let BlockKind::Heading { level } = block.kind {
                            StyleRole::heading(level)
                        } else {
                            g.style
                                .with(list.as_ref().map_or(StyleRole::DEFAULT, |v| v.body_style))
                        },
                        width: g.width,
                        link: g.link,
                        source: Some(g.source),
                    },
                ));
            }
        }
    };
    let visible: Vec<_> = glyphs
        .iter()
        .filter(|g| g.math.is_some() || g.kind != GraphemeKind::Control)
        .collect();
    let first = visible
        .iter()
        .position(|g| !trim_glyph(g))
        .unwrap_or(visible.len());
    let last = visible
        .iter()
        .rposition(|g| !trim_glyph(g))
        .map_or(first, |i| i + 1);
    for g in &visible[first..last] {
        let (mut gw, ga, gd) = g.math.as_ref().map_or((usize::from(g.width), 0, 0), |v| {
            (
                v.metrics.width.min(width),
                v.metrics.baseline,
                v.metrics.height.saturating_sub(v.metrics.baseline + 1),
            )
        });
        if matches!(block.kind, BlockKind::Heading { .. }) && x + gw > width {
            if g.math.is_none() || x >= width {
                break;
            }
            gw = width - x;
        }
        if !matches!(block.kind, BlockKind::Heading { .. }) && !line.is_empty() && x + gw > width {
            first_baseline.get_or_insert(y + ascent);
            first_height.get_or_insert(y + ascent + descent + 1);
            flush(
                &mut line,
                y,
                ascent,
                &mut placed,
                &mut placements,
                prefix_width,
                width,
            );
            y = y
                .checked_add(ascent + descent + 1)
                .ok_or(SinkError::RowOverflow)?;
            prefix_width = continued_prefix;
            width = usize::from(context.width).saturating_sub(prefix_width);
            x = 0;
            ascent = 0;
            descent = 0;
            gw = g
                .math
                .as_ref()
                .map_or(usize::from(g.width), |v| v.metrics.width.min(width));
        }
        if g.math.is_none() && gw > width {
            continue;
        }
        line.push((g, x));
        x += gw;
        ascent = ascent.max(ga);
        descent = descent.max(gd);
    }
    if !line.is_empty() {
        first_baseline.get_or_insert(y + ascent);
        first_height.get_or_insert(y + ascent + descent + 1);
        flush(
            &mut line,
            y,
            ascent,
            &mut placed,
            &mut placements,
            prefix_width,
            width,
        );
        y = y
            .checked_add(ascent + descent + 1)
            .ok_or(SinkError::RowOverflow)?;
    }
    let owner = context
        .document
        .tile_at(block.source.start)
        .filter(|t| {
            context.plan_source.start <= block.source.start
                && context.plan_source.end > block.source.start
                && context.document.tile(*t).is_some()
        })
        .unwrap_or(context.fallback_tile);
    let mut rows = Vec::new();
    let mut gaps = Vec::new();
    let mut next = 0usize;
    for (offset, mut cells) in placed {
        gaps.push(u64::try_from(offset - next).map_err(|_| SinkError::RowOverflow)?);
        next = offset + 1;
        cells.sort_by_key(|(x, _)| *x);
        let row_prefix = if offset < first_height.unwrap_or(0) {
            first_prefix
        } else {
            continued_prefix
        };
        let mut out = quote_cells.clone();
        let mut prefix_columns = out.iter().map(|c| usize::from(c.width)).sum::<usize>();
        while prefix_columns < row_prefix {
            prefix_columns += 1;
            out.push(SinkCell {
                text: " ".into(),
                width: 1,
                ..Default::default()
            });
        }
        if let Some(visual) = &list
            && Some(offset) == first_baseline
        {
            out.clear();
            for _ in 0..visual.indent.min(row_prefix) {
                out.push(SinkCell {
                    text: " ".into(),
                    width: 1,
                    ..Default::default()
                });
            }
            let mut used = visual.indent;
            for g in graphemes(&visual.marker) {
                if used + usize::from(g.width) > row_prefix {
                    break;
                }
                out.push(SinkCell {
                    text: g.text.into(),
                    width: g.width,
                    style: visual.marker_style,
                    ..Default::default()
                });
                used += usize::from(g.width);
            }
            while used < row_prefix {
                out.push(SinkCell {
                    text: " ".into(),
                    width: 1,
                    ..Default::default()
                });
                used += 1;
            }
        }
        let mut col = row_prefix;
        let source = cells
            .iter()
            .find_map(|(_, c)| c.source)
            .unwrap_or(block.source);
        for (x, c) in cells {
            while col < x {
                out.push(SinkCell {
                    text: " ".into(),
                    width: 1,
                    ..Default::default()
                });
                col += 1;
            }
            col = col.max(x + usize::from(c.width));
            out.push(c);
        }
        rows.push(PreparedRow {
            line: SinkLine {
                cells: out,
                source: Some(LineSource::new(source, block_id)),
            },
            owner_tile: owner,
            source_start: block.source.start,
        });
    }
    gaps.push(u64::try_from(y.saturating_sub(next)).map_err(|_| SinkError::RowOverflow)?);
    Ok(PreparedContent::MathRows {
        rows,
        gaps,
        placements,
    })
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

fn continuation_visual(
    context: &PrepareContext<'_>,
    block: &mdtui3::Block,
) -> Result<ListVisual, SinkError> {
    let node = block
        .container
        .and_then(|id| context.document.container(id))
        .expect("item continuation");
    let mdtui3::ContainerKind::Item { number, task } = node.kind else {
        unreachable!()
    };
    let mut marker = block.clone();
    marker.kind = BlockKind::ListItem {
        indent: 0,
        number,
        task,
    };
    let mut visual = prepared_list_visual(context.document, &marker, context.containers)?;
    visual.marker.clear();
    visual.indent = visual.continuation_indent;
    visual.body_indent = visual.continuation_indent;
    visual.marker_width = 0;
    Ok(visual)
}
fn prepare_list_paragraph(
    context: &PrepareContext<'_>,
    block_id: BlockId,
    block: &mdtui3::Block,
    glyphs: Vec<OwnedGlyph>,
    continuation: bool,
) -> Result<Option<PreparedContent>, SinkError> {
    let document = context.document;
    let window = Span::new(
        block.source.start.max(context.plan_source.start),
        block.source.end.min(context.plan_source.end),
    );
    let item = block.container.and_then(|id| document.container(id));
    let full_item = item.is_none_or(|item| {
        item.source.start >= context.plan_source.start && item.source.end <= context.plan_source.end
    });
    let mut visual = if continuation {
        continuation_visual(context, block)?
    } else {
        prepared_list_visual(document, block, context.containers)?
    };
    if continuation {
        visual.marker.clear();
    }
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
    let owner = if full_item && !context.reading {
        list_item_owner_tile(document, block, context.fallback_tile)
    } else {
        owner_tile(document, window, context.fallback_tile)
    };
    let mut rows = prepared_rows(
        extract_rows(&screen, 0, Some(rows))?,
        owner,
        block.source.start,
    );
    if context.reading && rows.len() > 1 {
        for row in &mut rows {
            let mut sources = row.line.cells.iter().filter_map(|cell| cell.source);
            if let Some(first) = sources.next() {
                let source = sources.fold(first, |a, b| {
                    Span::new(a.start.min(b.start), a.end.max(b.end))
                });
                row.source_start = source.start;
                row.line.source = Some(LineSource::new(source, block_id));
            }
        }
    }
    Ok(Some(PreparedContent::Rows(rows)))
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
    if plan.projected {
        return Ok(plan.visible);
    }
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
            if let Some(zero) = screen.zero_cell(row, column) {
                cells.push(SinkCell {
                    text: zero.text().clone(),
                    style: zero.style(),
                    width: 0,
                    link: zero.link_id(),
                    source: zero.source(),
                });
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
        text: "│".into(),
        style: StyleRole::QUOTE,
        width: 1,
        link: None,
        source: None,
    }];
    if width >= 2 {
        cells.push(SinkCell {
            text: " ".into(),
            style: StyleRole::QUOTE,
            width: 1,
            link: None,
            source: None,
        });
    }
    cells
}

fn trim_glyph(glyph: &OwnedGlyph) -> bool {
    if glyph.math.is_some() {
        return false;
    }
    glyph.text.is_ascii_whitespace()
}

fn text_line(
    text: &str,
    style: StyleRole,
    link: Option<LinkId>,
    source: Span,
    block: Option<BlockId>,
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
            text: grapheme.text.into(),
            style,
            width: grapheme.width,
            link,
            source: None,
        });
        used += grapheme_width;
    }
    SinkLine {
        cells,
        source: Some(block.map_or_else(
            || LineSource::pending(source),
            |block| LineSource::new(source, block),
        )),
    }
}

fn rule_line(source: Span, block: Option<BlockId>, width: u16) -> SinkLine {
    SinkLine {
        cells: (0..width)
            .map(|_| SinkCell {
                text: "─".into(),
                style: StyleRole::RULE,
                width: 1,
                link: None,
                source: None,
            })
            .collect(),
        source: Some(block.map_or_else(
            || LineSource::pending(source),
            |block| LineSource::new(source, block),
        )),
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
            // A code checkpoint restores the physical source line crossing the
            // boundary, not the block opener. Unfolded rows belong to the
            // newline tile; an inline PH below keeps its global-start owner.
            if matches!(block.kind, BlockKind::Code { .. })
                && let Some(PreparedBlock {
                    content: PreparedContent::Rows(rows),
                    ..
                }) = blocks.get(&block.id)
            {
                let at = rows.partition_point(|row| {
                    row.line.source.is_some_and(|s| s.source.end <= next_start)
                });
                if let Some(row) = rows.get(at) {
                    checkpoint.line_source_start = Some(row.source_start);
                }
            }
        }
        if plan.projected
            && let Some(row) = blocks
                .values()
                .filter_map(|block| match &block.content {
                    PreparedContent::Rows(rows) => Some(rows),
                    _ => None,
                })
                .flatten()
                .filter(|row| row.owner_tile == segment.tile)
                .max_by_key(|row| row.source_start)
        {
            checkpoint.line_source_start = Some(row.source_start);
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
                        PlanEventKind::FoldLabel { .. } => {}
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
            text: "🙂x".into(),
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
