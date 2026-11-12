use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::mem::size_of;
use std::ops::Range;

use mdtui3::{Budget, Completion, Document, DocumentId, Span, Table, TableId, Update, WorkLimit};

use crate::render::table::{TableError, table_width, visible_prefix_count};

pub const MAX_OUT_OF_ORDER_ROWS: usize = 32;

/// Even zero-width cells occupy three padding/border columns. Keep the largest
/// possible visible prefix plus two neighbors; this bound depends on the terminal.
pub fn column_window_limit(content_width: usize) -> usize {
    content_width / 3 + 2
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TableWidthWork {
    pub column_metadata_reads: usize,
    pub snapshot_columns_copied: usize,
    pub evicted_observations: usize,
    pub dropped_observations: usize,
    pub dropped_columns: usize,
}
impl TableWidthWork {
    fn add(&mut self, other: Self) {
        self.column_metadata_reads += other.column_metadata_reads;
        self.snapshot_columns_copied += other.snapshot_columns_copied;
        self.evicted_observations += other.evicted_observations;
        self.dropped_observations += other.dropped_observations;
        self.dropped_columns += other.dropped_columns;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableWidthSnapshot {
    pub table_id: TableId,
    pub widths: Vec<usize>,
    pub exact: Vec<bool>,
    pub provisional: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TableWidthStats {
    pub rows_read: usize,
    pub cells_read: usize,
    pub columns_complete: usize,
    pub columns_total: usize,
    pub stable_for_width: bool,
    pub provisional: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TableWidthMemoryStats {
    pub tables: usize,
    pub columns: usize,
    /// Out-of-order observed rows beyond each column scan frontier.
    pub observed_rows: usize,
    /// Vector/map capacity plus a conservative 256-byte allowance per sparse
    /// B-tree key (the standard library does not expose node capacity). Document
    /// source/table rows are excluded. Pending owned display buffers are counted;
    /// native private query scratch and shared resolver nodes are excluded and
    /// reported separately through object work/entry/retirement counters.
    pub tracked_capacity_bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisibleRowMeasure {
    pub completion: Completion,
    pub rows_read: usize,
    pub cells_read: usize,
    pub visible_columns: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableWidthAdvance {
    pub completion: Completion,
    pub rows_read: usize,
    pub cells_read: usize,
    pub completed_column: Option<usize>,
    pub width_changed: bool,
    pub visible_columns_before: usize,
    pub visible_columns_after: usize,
    pub stable_for_width: bool,
    pub provisional: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableWidthError {
    WrongDocument,
    OutOfOrderUpdate,
    MissingTable(TableId),
    Table(TableError),
}

impl fmt::Display for TableWidthError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongDocument => {
                formatter.write_str("table width cache belongs to another document")
            }
            Self::OutOfOrderUpdate => formatter.write_str("table width update is out of order"),
            Self::MissingTable(id) => write!(formatter, "table {} is missing", id.0),
            Self::Table(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for TableWidthError {}

impl From<TableError> for TableWidthError {
    fn from(error: TableError) -> Self {
        Self::Table(error)
    }
}

pub struct TableWidthCache {
    links: crate::render::objects::SharedLinks,
    document: DocumentId,
    revision: u64,
    tables: HashMap<TableId, TableWidthEntry>,
    retired_work: TableWidthWork,
}

#[derive(Default)]
struct TableWidthEntry {
    work: TableWidthWork,
    total_columns: usize,
    observed_nodes: usize,
    pending: Option<(usize, usize, Box<crate::render::display::DisplayScanner>)>,
    columns: Vec<ColumnMeasurement>,
    rows_read: usize,
    cells_read: usize,
}

#[derive(Clone, Debug, Default)]
struct ColumnMeasurement {
    hint_bound: usize,
    measured_max: usize,
    scan_row: usize,
    complete: bool,
    observed_rows: BTreeSet<usize>,
}

impl TableWidthCache {
    pub fn new(document: &Document) -> Self {
        Self {
            links: crate::render::objects::SharedLinks::new(document),
            document: document.id(),
            revision: document.revision(),
            tables: HashMap::new(),
            retired_work: TableWidthWork::default(),
        }
    }

    pub fn set_links(&mut self, links: crate::render::objects::SharedLinks) {
        self.links = links;
    }

    #[must_use]
    pub fn matches(&self, document: &Document) -> bool {
        self.document == document.id() && self.revision == document.revision()
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Cache contents are revision-bound. Repeated/empty updates at the current
    /// revision are no-ops; any real update clears all measured widths.
    pub fn apply_update(
        &mut self,
        document: &Document,
        update: &Update,
    ) -> Result<bool, TableWidthError> {
        if self.document != document.id() || update.document != Some(document.id()) {
            return Err(TableWidthError::WrongDocument);
        }
        if update.revision != document.revision() || update.base_revision > update.revision {
            return Err(TableWidthError::OutOfOrderUpdate);
        }
        if self.revision == update.revision {
            return Ok(false);
        }
        if update.base_revision != self.revision {
            return Err(TableWidthError::OutOfOrderUpdate);
        }
        self.document = document.id();
        self.revision = update.revision;
        self.retain_tables(&[]);
        Ok(true)
    }

    /// Explicit convenience for an 80-column prefix. Interactive callers use
    /// snapshot_budgeted so zero budget does not read or allocate column data.
    pub fn snapshot(
        &mut self,
        document: &Document,
        table_id: TableId,
    ) -> Result<TableWidthSnapshot, TableWidthError> {
        self.snapshot_budgeted(
            document,
            table_id,
            80,
            &mut Budget::new(WorkLimit::unlimited()),
        )?
        .ok_or(TableWidthError::MissingTable(table_id))
    }

    pub fn prepare_metadata(
        &mut self,
        document: &Document,
        table_id: TableId,
        content_width: usize,
        budget: &mut Budget,
    ) -> Result<bool, TableWidthError> {
        self.check_document(document)?;
        let table = document
            .table(table_id)
            .ok_or(TableWidthError::MissingTable(table_id))?;
        let limit = table.columns.len().min(column_window_limit(content_width));
        if !self.tables.contains_key(&table_id)
            && (budget.remaining().queries == 0 || budget.remaining().objects == 0)
        {
            return Ok(false);
        }
        let entry = self.tables.entry(table_id).or_default();
        trim_columns(entry, limit);
        entry.total_columns = table.columns.len();
        while entry.columns.len() < limit {
            if !charge(budget, None) {
                return Ok(false);
            }
            let column = &table.columns[entry.columns.len()];
            entry.columns.push(ColumnMeasurement {
                hint_bound: column.source_width_hint,
                complete: table.rows().is_empty(),
                ..ColumnMeasurement::default()
            });
            entry.work.column_metadata_reads += 1;
        }
        Ok(true)
    }

    pub fn snapshot_budgeted(
        &mut self,
        document: &Document,
        table_id: TableId,
        content_width: usize,
        budget: &mut Budget,
    ) -> Result<Option<TableWidthSnapshot>, TableWidthError> {
        if !self.prepare_metadata(document, table_id, content_width, budget)?
            || !charge(budget, None)
        {
            return Ok(None);
        }
        let entry = self
            .tables
            .get_mut(&table_id)
            .ok_or(TableWidthError::MissingTable(table_id))?;
        // One snapshot object, with a payload bounded by terminal width. Initial
        // hint reads are paid individually by prepare_metadata above.
        entry.work.snapshot_columns_copied += entry.columns.len();
        Ok(Some(snapshot(table_id, entry)))
    }

    pub fn work(&self) -> TableWidthWork {
        let mut work = self.retired_work;
        for entry in self.tables.values() {
            work.add(entry.work);
        }
        work
    }

    pub fn limit_columns(&mut self, content_width: usize) {
        let limit = column_window_limit(content_width);
        for entry in self.tables.values_mut() {
            trim_columns(entry, limit);
        }
    }

    /// Bounded metadata helper for callers that already own a prepared input.
    pub fn hint_snapshot_prefix(
        table_id: TableId,
        table: &Table,
        content_width: usize,
    ) -> TableWidthSnapshot {
        let limit = table.columns.len().min(column_window_limit(content_width));
        let widths = table.columns[..limit]
            .iter()
            .map(|column| column.source_width_hint)
            .collect::<Vec<_>>();
        TableWidthSnapshot {
            table_id,
            exact: vec![table.rows().is_empty(); limit],
            provisional: !table.rows().is_empty() || limit < table.columns.len(),
            widths,
        }
    }

    /// Conservative first-screen snapshot using only source byte hints. This
    /// reads no cell text but includes every column. Interactive callers use
    /// snapshot_budgeted or the terminal-prefix helper.
    pub fn hint_snapshot(table_id: TableId, table: &Table) -> TableWidthSnapshot {
        let widths = table
            .columns
            .iter()
            .map(|column| column.source_width_hint)
            .collect::<Vec<_>>();
        let exact = vec![table.rows().is_empty(); widths.len()];
        TableWidthSnapshot {
            table_id,
            provisional: !table.rows().is_empty(),
            widths,
            exact,
        }
    }

    pub fn observe_visible_rows(
        &mut self,
        document: &Document,
        table_id: TableId,
        content_width: usize,
        rows: Range<usize>,
        budget: &mut Budget,
    ) -> Result<VisibleRowMeasure, TableWidthError> {
        self.check_document(document)?;
        let table = document
            .table(table_id)
            .ok_or(TableWidthError::MissingTable(table_id))?;
        if !self.prepare_metadata(document, table_id, content_width, budget)? {
            return Ok(VisibleRowMeasure {
                completion: Completion::BudgetExhausted,
                rows_read: 0,
                cells_read: 0,
                visible_columns: 0,
            });
        }
        let links = self.links.clone();
        let entry = self.tables.get_mut(&table_id).expect("prepared table");
        let before_columns = visible_columns(entry, content_width);
        let visible_columns = before_columns;
        let end = rows.end.min(table.rows().len());
        let start = rows.start.min(end);
        let mut rows_read = 0;
        let mut cells_read = 0;
        for row_index in start..end {
            let cells = table
                .rows()
                .get(row_index)
                .map_or(0, |row| row.cells.len().min(visible_columns));
            if cells == 0 {
                if !charge(budget, None) {
                    return Ok(VisibleRowMeasure {
                        completion: Completion::BudgetExhausted,
                        rows_read,
                        cells_read,
                        visible_columns,
                    });
                }
            } else {
                for column in 0..cells {
                    if !measure_cell(
                        document, table, row_index, column, entry, true, &links, budget,
                    )? {
                        return Ok(VisibleRowMeasure {
                            completion: Completion::BudgetExhausted,
                            rows_read,
                            cells_read,
                            visible_columns,
                        });
                    }
                    cells_read += 1;
                    entry.cells_read += 1;
                }
            }
            rows_read += 1;
            entry.rows_read += 1;
        }
        Ok(VisibleRowMeasure {
            completion: Completion::Covered,
            rows_read,
            cells_read,
            visible_columns,
        })
    }

    /// Measure only columns that can still affect the visible prefix at this
    /// content width. The scan stops once the next excluded column cannot fit
    /// even at its measured lower bound.
    pub fn advance(
        &mut self,
        document: &Document,
        table_id: TableId,
        content_width: usize,
        budget: &mut Budget,
    ) -> Result<TableWidthAdvance, TableWidthError> {
        self.check_document(document)?;
        let table = document
            .table(table_id)
            .ok_or(TableWidthError::MissingTable(table_id))?;
        if !self.prepare_metadata(document, table_id, content_width, budget)? {
            return Ok(TableWidthAdvance {
                completion: Completion::BudgetExhausted,
                rows_read: 0,
                cells_read: 0,
                completed_column: None,
                width_changed: false,
                visible_columns_before: 0,
                visible_columns_after: 0,
                stable_for_width: false,
                provisional: true,
            });
        }
        let links = self.links.clone();
        let entry = self.tables.get_mut(&table_id).expect("prepared table");
        let visible_before = visible_columns(entry, content_width);
        let widths_before = safe_widths(entry);
        let mut rows_read = 0;
        let mut cells_read = 0;
        let mut completed_column = None;

        loop {
            let Some(column) = target_column(entry, content_width) else {
                let widths_after = safe_widths(entry);
                return Ok(TableWidthAdvance {
                    completion: Completion::Covered,
                    rows_read,
                    cells_read,
                    completed_column,
                    width_changed: widths_before != widths_after,
                    visible_columns_before: visible_before,
                    visible_columns_after: visible_columns(entry, content_width),
                    stable_for_width: true,
                    provisional: safe_provisional(entry),
                });
            };
            let rows_len = table.rows().len();
            if entry.columns[column].scan_row >= rows_len {
                entry.columns[column].complete = true;
                completed_column = Some(column);
                continue;
            }
            while entry.columns[column].scan_row < rows_len {
                let row = entry.columns[column].scan_row;
                if entry.columns[column].observed_rows.contains(&row) {
                    if !budget.charge(WorkLimit {
                        queries: 1,
                        ..WorkLimit::default()
                    }) {
                        let widths_after = safe_widths(entry);
                        return Ok(TableWidthAdvance {
                            completion: Completion::BudgetExhausted,
                            rows_read,
                            cells_read,
                            completed_column,
                            width_changed: widths_before != widths_after,
                            visible_columns_before: visible_before,
                            visible_columns_after: visible_columns(entry, content_width),
                            stable_for_width: false,
                            provisional: safe_provisional(entry),
                        });
                    }
                    if entry.columns[column].observed_rows.remove(&row) {
                        entry.observed_nodes -= 1;
                    }
                    entry.columns[column].scan_row += 1;
                    continue;
                }
                let source = table.cell(row, column).map(|cell| cell.source);
                if !measure_cell(document, table, row, column, entry, false, &links, budget)? {
                    let widths_after = safe_widths(entry);
                    return Ok(TableWidthAdvance {
                        completion: Completion::BudgetExhausted,
                        rows_read,
                        cells_read,
                        completed_column,
                        width_changed: widths_before != widths_after,
                        visible_columns_before: visible_before,
                        visible_columns_after: visible_columns(entry, content_width),
                        stable_for_width: false,
                        provisional: safe_provisional(entry),
                    });
                }
                entry.columns[column].scan_row += 1;
                rows_read += 1;
                entry.rows_read += 1;
                if source.is_some() {
                    cells_read += 1;
                    entry.cells_read += 1;
                }
            }
            entry.columns[column].complete = true;
            completed_column = Some(column);
        }
    }

    pub fn stats(
        &mut self,
        document: &Document,
        table_id: TableId,
        content_width: usize,
    ) -> Result<TableWidthStats, TableWidthError> {
        self.check_document(document)?;
        let table = document
            .table(table_id)
            .ok_or(TableWidthError::MissingTable(table_id))?;
        let Some(entry) = self.tables.get(&table_id) else {
            return Ok(TableWidthStats {
                columns_total: table.columns.len(),
                stable_for_width: table.columns.is_empty() || content_width == 0,
                provisional: !table.columns.is_empty(),
                ..TableWidthStats::default()
            });
        };
        let stable_for_width = entry.columns.len()
            == table.columns.len().min(column_window_limit(content_width))
            && target_column(entry, content_width).is_none();
        Ok(TableWidthStats {
            rows_read: entry.rows_read,
            cells_read: entry.cells_read,
            columns_complete: entry
                .columns
                .iter()
                .filter(|column| column.complete)
                .count(),
            columns_total: entry.total_columns,
            stable_for_width,
            provisional: safe_provisional(entry),
        })
    }

    #[must_use]
    pub fn memory_stats(&self) -> TableWidthMemoryStats {
        let mut columns = 0_usize;
        let mut observed_rows = 0_usize;
        let mut tracked_capacity_bytes =
            self.tables.capacity() * size_of::<Option<(TableId, TableWidthEntry)>>();
        for entry in self.tables.values() {
            if let Some((_, _, scanner)) = &entry.pending {
                tracked_capacity_bytes =
                    tracked_capacity_bytes.saturating_add(scanner.capacity_bytes());
            }
            columns = columns.saturating_add(entry.columns.len());
            tracked_capacity_bytes = tracked_capacity_bytes
                .saturating_add(entry.columns.capacity() * size_of::<ColumnMeasurement>());
            for column in &entry.columns {
                observed_rows = observed_rows.saturating_add(column.observed_rows.len());
                tracked_capacity_bytes = tracked_capacity_bytes
                    .saturating_add(column.observed_rows.len() * (size_of::<usize>() * 32));
            }
        }
        TableWidthMemoryStats {
            tables: self.tables.len(),
            columns,
            observed_rows,
            tracked_capacity_bytes,
        }
    }

    /// Drop measurement state for tables outside the current visible window.
    /// Widths are a lazy cache and are rebuilt if the table scrolls back in.
    pub fn retain_tables(&mut self, keep: &[TableId]) {
        let mut retired = TableWidthWork::default();
        self.tables.retain(|id, entry| {
            if keep.contains(id) {
                return true;
            }
            let mut work = entry.work;
            work.dropped_columns += entry.columns.len();
            work.dropped_observations += entry.observed_nodes;
            retired.add(work);
            false
        });
        self.retired_work.add(retired);
    }

    fn check_document(&self, document: &Document) -> Result<(), TableWidthError> {
        if self.document != document.id() {
            return Err(TableWidthError::WrongDocument);
        }
        if self.revision != document.revision() {
            return Err(TableWidthError::OutOfOrderUpdate);
        }
        Ok(())
    }
}

fn trim_columns(entry: &mut TableWidthEntry, limit: usize) {
    if entry.columns.len() <= limit {
        return;
    }
    entry.work.dropped_columns += entry.columns.len() - limit;
    for column in entry.columns.drain(limit..) {
        entry.work.dropped_observations += column.observed_rows.len();
        entry.observed_nodes -= column.observed_rows.len();
    }
    if entry
        .pending
        .as_ref()
        .is_some_and(|(_, column, _)| *column >= limit)
    {
        entry.pending = None;
    }
}

#[allow(clippy::too_many_arguments)]
fn measure_cell(
    document: &Document,
    table: &Table,
    row: usize,
    column: usize,
    entry: &mut TableWidthEntry,
    remember: bool,
    links: &crate::render::objects::SharedLinks,
    budget: &mut Budget,
) -> Result<bool, TableWidthError> {
    let Some(cell) = table.cell(row, column) else {
        return Ok(charge(budget, None));
    };
    // Keep the existing giant-cell provisional policy. No owned full-cell
    // buffer or synchronous conditional query is created above the frame cap.
    if cell.source.len() > crate::stream::FRAME_WORK.bytes {
        return Ok(false);
    }
    if entry.columns[column].scan_row > row || entry.columns[column].observed_rows.contains(&row) {
        return Ok(true);
    }
    if entry
        .pending
        .as_ref()
        .is_none_or(|(r, c, _)| (*r, *c) != (row, column))
    {
        entry.pending = Some((
            row,
            column,
            Box::new(
                crate::render::display::DisplayScanner::exact(document, cell.source, links.clone())
                    .ok_or(TableWidthError::Table(TableError::MissingContent))?,
            ),
        ));
    }
    let scanner = &mut entry.pending.as_mut().unwrap().2;
    if scanner
        .advance(document, budget)
        .map_err(|_| TableWidthError::Table(TableError::MissingContent))?
        != crate::render::display::ScanStatus::Complete
    {
        return Ok(false);
    }
    let tokens = scanner
        .segments()
        .iter()
        .map(|s| crate::render::input::InputToken {
            kind: s.kind,
            text: s.text.clone(),
            source: s.source,
            replacement: s.replacement,
            block: s.block,
            link: s.link,
            style: s.style,
        })
        .collect();
    let input = crate::render::visibility::visible_prepared_input(
        document,
        cell.source,
        crate::render::input::DisplayInput::from_tokens(tokens),
    )
    .ok_or(TableWidthError::Table(TableError::MissingContent))?;
    let width = crate::render::text::display_width(&input.visible_text());
    entry.pending = None;
    let measurement = &mut entry.columns[column];
    measurement.measured_max = measurement.measured_max.max(width);
    measurement.hint_bound = measurement.hint_bound.max(width);
    if remember && row >= measurement.scan_row {
        if measurement.observed_rows.len() == MAX_OUT_OF_ORDER_ROWS
            && measurement.observed_rows.pop_first().is_some()
        {
            entry.observed_nodes -= 1;
            entry.work.evicted_observations += 1;
        }
        if measurement.observed_rows.insert(row) {
            entry.observed_nodes += 1;
        }
    }
    Ok(true)
}

fn charge(budget: &mut Budget, source: Option<Span>) -> bool {
    budget.charge(WorkLimit {
        queries: 1,
        objects: 1,
        bytes: source.map_or(0, Span::len),
        ..WorkLimit::default()
    })
}

fn snapshot(table_id: TableId, entry: &TableWidthEntry) -> TableWidthSnapshot {
    TableWidthSnapshot {
        table_id,
        widths: safe_widths(entry),
        exact: entry.columns.iter().map(|column| column.complete).collect(),
        provisional: safe_provisional(entry),
    }
}

fn safe_widths(entry: &TableWidthEntry) -> Vec<usize> {
    entry
        .columns
        .iter()
        .map(|column| {
            if column.complete {
                column.measured_max
            } else {
                column.hint_bound.max(column.measured_max)
            }
        })
        .collect()
}

fn safe_provisional(entry: &TableWidthEntry) -> bool {
    entry.columns.len() < entry.total_columns || entry.columns.iter().any(|column| !column.complete)
}

fn visible_columns(entry: &TableWidthEntry, content_width: usize) -> usize {
    let widths = safe_widths(entry);
    visible_prefix_count(&widths, content_width)
}

fn target_column(entry: &TableWidthEntry, content_width: usize) -> Option<usize> {
    if entry.columns.is_empty() || content_width == 0 {
        return None;
    }
    let widths = safe_widths(entry);
    let visible = visible_prefix_count(&widths, content_width);
    if let Some(column) = (0..visible).find(|column| !entry.columns[*column].complete) {
        return Some(column);
    }
    if visible >= entry.columns.len() {
        return None;
    }
    let mut candidate = widths[..visible].to_vec();
    candidate.push(entry.columns[visible].measured_max);
    (table_width(&candidate) <= content_width).then_some(visible)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mdtui3::BlockKind;

    fn parse(source: &str) -> Document {
        let mut document = Document::new();
        document.append(source).unwrap();
        document.finish();
        document
    }

    fn first_table(document: &Document) -> TableId {
        document
            .blocks()
            .iter()
            .find_map(|block| match block.kind {
                BlockKind::Table { id } => Some(id),
                _ => None,
            })
            .unwrap()
    }

    fn tiny_budget() -> Budget {
        Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 256,
            ..WorkLimit::default()
        })
    }

    #[test]
    fn hint_snapshot_reads_no_cells_and_is_conservative() {
        let document = parse("| A | B |\n| --- | --- |\n| 中文 | 🙂 |\n");
        let id = first_table(&document);
        let table = document.table(id).unwrap();
        let hints = TableWidthCache::hint_snapshot(id, table);
        assert_eq!(
            hints.widths,
            table
                .columns
                .iter()
                .map(|c| c.source_width_hint)
                .collect::<Vec<_>>()
        );
        assert!(hints.provisional);

        let mut cache = TableWidthCache::new(&document);
        let snapshot = cache.snapshot(&document, id).unwrap();
        assert_eq!(snapshot, hints);
        let stats = cache.stats(&document, id, 80).unwrap();
        assert_eq!((stats.rows_read, stats.cells_read), (0, 0));
    }

    #[test]
    fn visible_row_observation_is_budgeted_and_counts_only_requested_rows() {
        let document = parse(concat!(
            "| A | B | C |\n",
            "| --- | --- | --- |\n",
            "| 中文 | 🙂 | x |\n",
            "| long | value | y |\n",
            "| tail | z | q |\n",
        ));
        let id = first_table(&document);
        let mut cache = TableWidthCache::new(&document);
        let mut budget = Budget::new(WorkLimit {
            queries: 16,
            objects: 16,
            bytes: 4096,
            ..WorkLimit::default()
        });
        let observed = cache
            .observe_visible_rows(&document, id, 80, 0..2, &mut budget)
            .unwrap();
        assert_eq!(observed.completion, Completion::Covered);
        assert_eq!(observed.rows_read, 2);
        let stats = cache.stats(&document, id, 80).unwrap();
        assert_eq!(stats.rows_read, 2);
        assert!(stats.cells_read >= 6);
    }

    #[test]
    fn tiny_budget_advances_one_cell_at_a_time_without_losing_state() {
        let document = parse("| A | B |\n| --- | --- |\n| one | two |\n| three | four |\n");
        let id = first_table(&document);
        let mut cache = TableWidthCache::new(&document);
        let mut exhausted = false;
        let mut steps = 0;
        while !cache.stats(&document, id, 20).unwrap().stable_for_width {
            let mut budget = tiny_budget();
            let result = cache.advance(&document, id, 20, &mut budget).unwrap();
            exhausted |= result.completion == Completion::BudgetExhausted;
            steps += 1;
            assert!(steps < 100);
        }
        assert!(exhausted);
        let snapshot = cache.snapshot(&document, id).unwrap();
        assert!(snapshot.widths.iter().all(|width| *width <= 5));
    }

    #[test]
    fn update_clears_revision_bound_widths_and_noop_preserves_them() {
        let mut document = Document::new();
        document
            .append("| A | B |\n| --- | --- |\n| one | two |\n")
            .unwrap();
        let id = first_table(&document);
        let mut cache = TableWidthCache::new(&document);
        let first = cache
            .advance(&document, id, 80, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap();
        assert!(first.stable_for_width);
        let before = cache.snapshot(&document, id).unwrap();
        let empty = document.append("").unwrap();
        assert!(!cache.apply_update(&document, &empty).unwrap());
        assert_eq!(cache.snapshot(&document, id).unwrap(), before);

        let update = document.append("| three | four |\n").unwrap();
        assert!(cache.apply_update(&document, &update).unwrap());
        let after = cache.snapshot(&document, id).unwrap();
        assert!(after.provisional);
        assert_ne!(after, before);
    }
}
