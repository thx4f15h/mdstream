//! Cell spans are indexed by the parser. Preparing a suffix must never visit
//! its omitted columns or read the containing physical row envelope.
use super::plan::BlockPlan;
use super::table::table_row_window;
use super::table_widths::TableWidthSnapshot;
use mdtui3::{Anchor, BlockKind, Budget, Document, Span, Table, TableId, WorkLimit};
use std::collections::HashMap;
use std::ops::Range;

#[derive(Clone, Copy)]
pub(crate) struct TableView {
    pub anchor: Anchor,
    pub source: Option<usize>,
    pub rows: usize,
    pub tail: Option<usize>,
}

pub(crate) fn row_owner(row: &mdtui3::TableRow) -> usize {
    if row.source.len() > 4096 {
        row.source.start
    } else {
        row.source.end.saturating_sub(1)
    }
}

struct CellProjection {
    task: mdtui3::FoldProjection,
    source: Span,
    limit: usize,
    bytes: usize,
    pending: Option<Span>,
}
pub(crate) struct TableWindow {
    cell: Option<CellProjection>,
    cut: Option<usize>,
    completed_cells: std::collections::BTreeSet<(TableId, usize, usize)>,
    pub placeholders: Vec<mdtui3::FoldPlaceholder>,
    source: Span,
    fragments: Vec<Span>,
    fragment_contexts: HashMap<usize, Span>,
    pub owners: std::collections::BTreeSet<mdtui3::TileId>,
    fragment: usize,
    pub row_ranges: HashMap<TableId, Vec<Range<usize>>>,
    pub ranges: Vec<(Span, bool)>,
    pub contexts: HashMap<usize, Span>,
    pub cuts: Vec<usize>,
    pub rows: HashMap<TableId, Range<usize>>,
    pub skipped: Vec<(mdtui3::TileId, usize, u64)>,
    block: usize,
    row: usize,
    column: usize,
    cursor: usize,
    initialized: bool,
    virtual_index: usize,
}

fn charge(budget: &mut Budget) -> bool {
    budget.charge(WorkLimit {
        queries: 1,
        objects: 1,
        ..WorkLimit::default()
    })
}

impl TableWindow {
    pub fn new(source: Span) -> Self {
        Self {
            cell: None,
            cut: None,
            completed_cells: Default::default(),
            placeholders: Vec::new(),
            source,
            fragments: Vec::new(),
            fragment_contexts: HashMap::new(),
            owners: Default::default(),
            fragment: 0,
            row_ranges: HashMap::new(),
            ranges: Vec::new(),
            contexts: HashMap::new(),
            cuts: Vec::new(),
            rows: HashMap::new(),
            skipped: Vec::new(),
            block: 0,
            row: 0,
            column: 0,
            cursor: source.start,
            initialized: false,
            virtual_index: 0,
        }
    }

    pub fn projected(
        source: Span,
        fragments: &[Span],
        contexts: &HashMap<usize, Span>,
        cut: Option<usize>,
    ) -> Self {
        let mut result = Self::new(source);
        result.fragments = fragments.to_vec();
        result.fragment_contexts = contexts.clone();
        result.cut = cut;
        result
    }
    fn advance_cell<S>(
        &mut self,
        doc: &Document,
        view: &mdtui3::FoldView<S>,
        source: Span,
        width: usize,
        budget: &mut Budget,
    ) -> Result<bool, mdtui3::LayoutError> {
        let source = Span::new(
            source.start,
            source
                .end
                .min(self.cut.unwrap_or(usize::MAX))
                .max(source.start),
        );
        if self.cell.is_none() {
            if !budget.charge(WorkLimit {
                queries: 1,
                objects: 1,
                bytes: 4,
                ..Default::default()
            }) {
                return Ok(false);
            }
            let task = mdtui3::FoldProjection::new(doc, view, source)?;
            self.cell = Some(CellProjection {
                task,
                source,
                limit: width
                    .saturating_mul(4)
                    .saturating_add(super::edges::MAX_EDGE_OVERSCAN),
                bytes: 0,
                pending: None,
            });
        }
        loop {
            let cell = self.cell.as_mut().unwrap();
            if let Some(mut part) = cell.pending {
                let Some(context) = view.visible_fragment(doc, part.start, budget)? else {
                    return Ok(false);
                };
                let mut end = part
                    .end
                    .min(part.start.saturating_add(cell.limit - cell.bytes));
                while end > part.start && !doc.source().is_char_boundary(end) {
                    end -= 1;
                }
                if end == part.start {
                    self.cell = None;
                    return Ok(true);
                }
                part.end = end;
                let context = Span::new(
                    context.start.max(cell.source.start),
                    context.end.min(cell.source.end),
                );
                let merged = self.ranges.last_mut().filter(|(last, table)| {
                    *table
                        && last.end == part.start
                        && self.contexts.get(&last.start) == Some(&context)
                });
                if let Some((last, _)) = merged {
                    last.end = part.end;
                } else {
                    self.contexts.insert(part.start, context);
                    self.ranges.push((part, true));
                }
                if part.end < context.end.min(cell.source.end) {
                    self.cuts.push(part.end);
                }
                cell.bytes += part.len();
                cell.pending = None;
                if cell.bytes >= cell.limit {
                    self.cell = None;
                    return Ok(true);
                }
            }
            match cell.task.next(doc, view, budget)? {
                mdtui3::FoldProjectionStep::Text { source, .. } => cell.pending = Some(source),
                mdtui3::FoldProjectionStep::Placeholder(p) => {
                    if !self
                        .placeholders
                        .iter()
                        .any(|old| old.candidate.hidden.start == p.candidate.hidden.start)
                    {
                        self.placeholders.push(p);
                    }
                }
                mdtui3::FoldProjectionStep::Hidden { source, .. } => {
                    if let Some(last) = self.placeholders.last_mut() {
                        last.candidate.hidden.end = last.candidate.hidden.end.max(source.end);
                    }
                }
                mdtui3::FoldProjectionStep::Complete => {
                    self.cell = None;
                    return Ok(true);
                }
                mdtui3::FoldProjectionStep::ParagraphEnd(_) => {}
                mdtui3::FoldProjectionStep::BudgetExhausted => return Ok(false),
            }
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub fn advance(
        &mut self,
        doc: &Document,
        plan: &BlockPlan,
        widths: &HashMap<TableId, TableWidthSnapshot>,
        width: usize,
        row_limit: usize,
        view: Option<TableView>,
        folds: Option<&mdtui3::FoldView<super::checkpoint::RenderCheckpoint>>,
        budget: &mut Budget,
    ) -> Result<bool, mdtui3::LayoutError> {
        loop {
            let visible = self
                .fragments
                .get(self.fragment)
                .copied()
                .unwrap_or(self.source);
            while let Some(block) = plan.blocks.get(self.block) {
                if block.source.end <= visible.start || block.source.start >= visible.end {
                    if !charge(budget) {
                        return Ok(false);
                    }
                    self.block += 1;
                    continue;
                }
                let BlockKind::Table { id } = block.kind else {
                    if !charge(budget) {
                        return Ok(false);
                    }
                    self.block += 1;
                    continue;
                };
                let table = doc.table(id).expect("planned table");
                let snapshot = widths.get(&id).expect("prepared metadata");
                if !self.initialized {
                    if !charge(budget) {
                        return Ok(false);
                    }
                    if self.cursor < block.source.start {
                        self.ranges.push((
                            Span::new(self.cursor, block.source.start.min(visible.end)),
                            false,
                        ));
                    }
                    let metadata_visible =
                        self.fragment_contexts
                            .get(&visible.start)
                            .map_or(visible, |context| {
                                Span::new(
                                    visible.start,
                                    context.end.min(self.cut.unwrap_or(usize::MAX)),
                                )
                            });
                    let mut rows = table_row_window(table, metadata_visible);
                    if let Some(view) = view {
                        if let Some(cut) = view.tail {
                            rows.end = rows
                                .end
                                .min(table.rows().partition_point(|r| r.source.start < cut));
                            rows.start = rows.start.max(rows.end.saturating_sub(view.rows.max(1)));
                        } else if folds.is_some() {
                            if let Some(row) = view.source.and_then(|source| table.row_at(source)) {
                                rows.start = rows.start.max(row).min(rows.end);
                            }
                            // The offset counts the projected rows in the anchor tile.
                            // Raw row offsets would jump across hidden TableRows.
                            let limit = view.rows.saturating_add(
                                usize::try_from(view.anchor.offset).unwrap_or(usize::MAX),
                            );
                            rows.end = rows.end.min(rows.start.saturating_add(limit.max(1)));
                        } else {
                            let tile_start = doc.tile(view.anchor.tile).map_or(0, |s| s.start);
                            if block.source.start < doc.tile(view.anchor.tile).map_or(0, |s| s.end)
                                && tile_start < block.source.end
                            {
                                rows.start = view
                                    .source
                                    .and_then(|source| table.row_at(source))
                                    .unwrap_or_else(|| {
                                        row_from_offset(table, tile_start, view.anchor.offset)
                                    });
                            }
                            rows.end = rows.end.min(rows.start.saturating_add(view.rows.max(1)));
                        }
                    } else {
                        rows.end = rows.end.min(rows.start.saturating_add(row_limit.max(1)));
                    }
                    rows.start = rows.start.min(rows.end);
                    self.row = rows.start;
                    self.row_ranges.entry(id).or_default().push(rows.clone());
                    self.rows.insert(id, rows);
                    self.initialized = true;
                }
                let rows = self.rows[&id].clone();
                let columns = if snapshot.widths.is_empty() {
                    0
                } else {
                    snapshot.widths.len()
                };
                while self.row < rows.end {
                    while self.column < columns {
                        if let Some(folds) = folds {
                            let key = (id, self.row, snapshot.first_column + self.column);
                            if self.completed_cells.contains(&key) {
                                if !charge(budget) {
                                    return Ok(false);
                                }
                                self.column += 1;
                                continue;
                            }
                            if let Some(cell) =
                                table.cell(self.row, snapshot.first_column + self.column)
                            {
                                if !self.advance_cell(doc, folds, cell.source, width, budget)? {
                                    return Ok(false);
                                }
                            } else if !charge(budget) {
                                return Ok(false);
                            }
                            self.completed_cells.insert(key);
                            self.column += 1;
                            continue;
                        }
                        if !budget.charge(WorkLimit {
                            queries: 1,
                            objects: 1,
                            bytes: 4,
                            ..WorkLimit::default()
                        }) {
                            return Ok(false);
                        }
                        if let Some(cell) =
                            table.cell(self.row, snapshot.first_column + self.column)
                        {
                            let start = if self.fragments.is_empty() {
                                cell.source.start
                            } else {
                                cell.source.start.max(visible.start).min(cell.source.end)
                            };
                            let mut end = cell
                                .source
                                .end
                                .min(if self.fragments.is_empty() {
                                    usize::MAX
                                } else {
                                    visible.end
                                })
                                .min(view.and_then(|v| v.tail).unwrap_or(usize::MAX))
                                .max(cell.source.start)
                                .min(
                                    start.saturating_add(
                                        width
                                            .saturating_mul(4)
                                            .saturating_add(super::edges::MAX_EDGE_OVERSCAN),
                                    ),
                                );
                            while !doc.source().is_char_boundary(end) {
                                end -= 1;
                            }
                            if end < cell.source.end {
                                self.cuts.push(end);
                            }
                            if start < end {
                                self.contexts.insert(
                                    start,
                                    Span::new(
                                        start,
                                        cell.source
                                            .end
                                            .min(if self.fragments.is_empty() {
                                                usize::MAX
                                            } else {
                                                visible.end
                                            })
                                            .min(view.and_then(|v| v.tail).unwrap_or(usize::MAX))
                                            .max(cell.source.start),
                                    ),
                                );
                                self.ranges.push((Span::new(start, end), true));
                            }
                        }
                        self.column += 1;
                    }
                    if folds.is_some()
                        && let Some(owner) = doc.tile_at(row_owner(&table.rows()[self.row]))
                    {
                        self.owners.insert(owner);
                    }
                    self.row += 1;
                    self.column = 0;
                }
                let all = table_row_window(table, visible);
                let omitted = [all.start..rows.start, rows.end..all.end];
                while self.virtual_index < plan.segments.len() * 2 {
                    if !charge(budget) {
                        return Ok(false);
                    }
                    let range = &omitted[self.virtual_index / plan.segments.len()];
                    let segment = &plan.segments[self.virtual_index % plan.segments.len()];
                    if !range.is_empty() {
                        let before = table.rows()[range.clone()]
                            .partition_point(|row| row_owner(row) < segment.source.start)
                            + range.start;
                        let after = table.rows()[range.clone()]
                            .partition_point(|row| row_owner(row) < segment.source.end)
                            + range.start;
                        if before < after {
                            let count = (after - before) as u64
                                + if before == 0 { 2 } else { 0 }
                                + if after == table.rows().len() && after > 1 {
                                    1
                                } else {
                                    0
                                };
                            self.skipped.push((
                                segment.tile,
                                row_owner(&table.rows()[before]),
                                count,
                            ));
                        }
                    }
                    self.virtual_index += 1;
                }
                self.virtual_index = 0;
                self.cursor = block.source.end.min(visible.end);
                self.block += 1;
                self.initialized = false;
            }
            if self.cursor < visible.end {
                self.ranges
                    .push((Span::new(self.cursor, visible.end), false));
                self.cursor = visible.end;
            }
            if self.fragment + 1 < self.fragments.len() {
                self.fragment += 1;
                self.source = self.fragments[self.fragment];
                self.block = 0;
                self.cursor = self.source.start;
                self.initialized = false;
                self.row = 0;
                self.column = 0;
                self.virtual_index = 0;
                continue;
            }
            return Ok(true);
        }
    }
}

pub(crate) fn row_from_offset(table: &Table, tile_start: usize, offset: u64) -> usize {
    let first = table
        .rows()
        .partition_point(|row| row_owner(row) < tile_start);
    let offset = usize::try_from(offset).unwrap_or(usize::MAX);
    if first == 0 {
        offset.saturating_sub(2)
    } else {
        first.saturating_add(offset)
    }
    .min(table.rows().len().saturating_sub(1))
}
