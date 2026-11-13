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

pub(crate) struct TableWindow {
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

    #[allow(clippy::too_many_arguments)]
    pub fn advance(
        &mut self,
        doc: &Document,
        plan: &BlockPlan,
        widths: &HashMap<TableId, TableWidthSnapshot>,
        width: usize,
        row_limit: usize,
        view: Option<TableView>,
        budget: &mut Budget,
    ) -> bool {
        while let Some(block) = plan.blocks.get(self.block) {
            let BlockKind::Table { id } = block.kind else {
                self.block += 1;
                continue;
            };
            let table = doc.table(id).expect("planned table");
            let snapshot = widths.get(&id).expect("prepared metadata");
            if !self.initialized {
                if !charge(budget) {
                    return false;
                }
                if self.cursor < block.source.start {
                    self.ranges.push((
                        Span::new(self.cursor, block.source.start.min(plan.visible.end)),
                        false,
                    ));
                }
                let mut rows = table_row_window(table, plan.visible);
                if let Some(view) = view {
                    if let Some(cut) = view.tail {
                        rows.end = rows
                            .end
                            .min(table.rows().partition_point(|r| r.source.start < cut));
                        rows.start = rows.start.max(rows.end.saturating_sub(view.rows.max(1)));
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
                self.rows.insert(id, rows);
                self.initialized = true;
            }
            let rows = &self.rows[&id];
            let columns = if snapshot.widths.is_empty() {
                0
            } else {
                snapshot.widths.len()
            };
            while self.row < rows.end {
                while self.column < columns {
                    if !budget.charge(WorkLimit {
                        queries: 1,
                        objects: 1,
                        bytes: 4,
                        ..WorkLimit::default()
                    }) {
                        return false;
                    }
                    if let Some(cell) = table.cell(self.row, snapshot.first_column + self.column) {
                        let mut end = cell
                            .source
                            .end
                            .min(view.and_then(|v| v.tail).unwrap_or(usize::MAX))
                            .max(cell.source.start)
                            .min(
                                cell.source.start.saturating_add(
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
                        if cell.source.start < end {
                            self.contexts.insert(
                                cell.source.start,
                                Span::new(
                                    cell.source.start,
                                    cell.source
                                        .end
                                        .min(view.and_then(|v| v.tail).unwrap_or(usize::MAX))
                                        .max(cell.source.start),
                                ),
                            );
                            self.ranges.push((Span::new(cell.source.start, end), true));
                        }
                    }
                    self.column += 1;
                }
                self.row += 1;
                self.column = 0;
            }
            let all = table_row_window(table, plan.visible);
            let omitted = [all.start..rows.start, rows.end..all.end];
            while self.virtual_index < plan.segments.len() * 2 {
                if !charge(budget) {
                    return false;
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
                        self.skipped
                            .push((segment.tile, row_owner(&table.rows()[before]), count));
                    }
                }
                self.virtual_index += 1;
            }
            self.virtual_index = 0;
            self.cursor = block.source.end.min(plan.visible.end);
            self.block += 1;
            self.initialized = false;
        }
        if self.cursor < plan.visible.end {
            self.ranges
                .push((Span::new(self.cursor, plan.visible.end), false));
            self.cursor = plan.visible.end;
        }
        true
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
