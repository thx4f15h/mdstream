use crate::inline::{Effects, Inline};
use crate::{BlockId, Span, Style, TableId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alignment {
    Default,
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Column {
    pub alignment: Alignment,
    /// Monotonic maximum of committed trimmed source byte lengths. An inexpensive
    /// sizing hint, not terminal columns or exact shaped/visible text width.
    pub source_width_hint: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TableCell {
    pub source: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableRow {
    pub source: Span,
    /// Only physically present cells up to the schema width. Missing cells are
    /// implicit empty cells, avoiding rows * columns allocation for sparse rows.
    pub cells: Vec<TableCell>,
    pub complete: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    pub block: BlockId,
    pub source: Span,
    pub delimiter: Span,
    pub columns: Vec<Column>,
    rows: Vec<TableRow>,
}

impl Table {
    pub fn rows(&self) -> &[TableRow] {
        &self.rows
    }
    /// Row zero is the header. Both row and column access are O(1).
    pub fn row(&self, index: usize) -> Option<&TableRow> {
        self.rows.get(index)
    }
    pub fn cell(&self, row: usize, column: usize) -> Option<&TableCell> {
        self.rows.get(row)?.cells.get(column)
    }
    pub fn row_at(&self, offset: usize) -> Option<usize> {
        let i = self.rows.partition_point(|row| row.source.end <= offset);
        self.rows
            .get(i)
            .filter(|row| row.source.start <= offset)
            .map(|_| i)
    }
    pub(crate) fn new(
        block: BlockId,
        source: Span,
        delimiter: Span,
        alignments: Vec<Alignment>,
    ) -> Self {
        Self {
            block,
            source,
            delimiter,
            columns: alignments
                .into_iter()
                .map(|alignment| Column {
                    alignment,
                    source_width_hint: 0,
                })
                .collect(),
            rows: Vec::new(),
        }
    }
}

pub(crate) struct HeaderCandidate {
    pub block: Option<BlockId>,
    pub source: Span,
    pub columns: usize,
}

/// Used only on a bounded prospective header/separator, never on old body rows.
pub(crate) fn split(source: &str, span: Span) -> Option<Vec<Span>> {
    let text = &source[span.range()];
    let trimmed = text.trim_ascii();
    let start = span.start + text.len() - text.trim_ascii_start().len();
    let end = start + trimmed.len();
    let mut cells = Vec::new();
    let mut cell_start = start;
    let mut escaped = false;
    let mut has_pipe = false;
    for pos in start..end {
        let byte = source.as_bytes()[pos];
        if byte == b'|' && !escaped {
            has_pipe = true;
            if pos != start {
                cells.push(trim(source, Span::new(cell_start, pos)));
            }
            cell_start = pos + 1;
        }
        escaped = byte == b'\\' && !escaped;
    }
    if cell_start < end {
        cells.push(trim(source, Span::new(cell_start, end)));
    }
    if has_pipe && !cells.is_empty() {
        Some(cells)
    } else {
        None
    }
}

fn trim(source: &str, span: Span) -> Span {
    let text = &source[span.range()];
    let content = text.trim_ascii();
    let start = span.start + text.len() - text.trim_ascii_start().len();
    Span::new(start, start + content.len())
}

pub(crate) fn delimiter(source: &str, span: Span, columns: usize) -> Option<Vec<Alignment>> {
    let cells = split(source, span)?;
    if cells.len() != columns {
        return None;
    }
    cells
        .into_iter()
        .map(|cell| {
            let text = &source[cell.range()];
            let left = text.starts_with(':');
            let right = text.ends_with(':');
            let body = text.trim_matches(':');
            if body.is_empty()
                || !body.bytes().all(|b| b == b'-')
                || text.len() != body.len() + usize::from(left) + usize::from(right)
            {
                return None;
            }
            Some(match (left, right) {
                (true, true) => Alignment::Center,
                (true, false) => Alignment::Left,
                (false, true) => Alignment::Right,
                _ => Alignment::Default,
            })
        })
        .collect()
}

pub(crate) struct RowParser {
    pub table: TableId,
    row: usize,
    column: usize,
    start: usize,
    content_start: Option<usize>,
    content_end: usize,
    escaped: bool,
    leading: bool,
    after_pipe: bool,
    complete: bool,
    inline: Inline,
}

impl RowParser {
    pub fn new(id: TableId, table: &mut Table, start: usize) -> Self {
        let row = table.rows.len();
        table.rows.push(TableRow {
            source: Span::new(start, start),
            cells: vec![TableCell {
                source: Span::new(start, start),
            }],
            complete: false,
        });
        Self {
            table: id,
            row,
            column: 0,
            start,
            content_start: None,
            content_end: start,
            escaped: false,
            leading: true,
            after_pipe: false,
            complete: false,
            inline: Inline::default(),
        }
    }

    pub fn feed(&mut self, span: Span, table: &mut Table, effects: &mut Effects<'_>) {
        effects.work.table_bytes += span.len() as u64;
        table.source.end = span.end;
        table.rows[self.row].source.end = span.end;
        let mut segment = span.start;
        for pos in span.range() {
            let byte = effects.source.as_bytes()[pos];
            if byte == b'\n' {
                self.feed_content(Span::new(segment, pos), table, effects);
                self.finish(pos, table, effects);
                return;
            }
            if byte == b'|' && !self.escaped {
                self.feed_content(Span::new(segment, pos), table, effects);
                if self.leading {
                    self.inline.finish(effects);
                    effects.mark(Span::new(self.start, pos + 1), Style::HIDDEN, None);
                } else {
                    self.finish_cell(pos, table, effects);
                    effects.mark(Span::new(pos, pos + 1), Style::HIDDEN, None);
                    self.column += 1;
                    if self.column < table.columns.len() {
                        table.rows[self.row].cells.push(TableCell {
                            source: Span::new(pos + 1, pos + 1),
                        });
                    }
                }
                self.start = pos + 1;
                self.content_start = None;
                self.content_end = self.start;
                self.leading = false;
                self.after_pipe = true;
                segment = pos + 1;
            } else {
                if byte == b'|' && self.escaped {
                    effects.mark(Span::new(pos - 1, pos), Style::HIDDEN, None);
                }
                if !byte.is_ascii_whitespace() {
                    self.content_start.get_or_insert(pos);
                    self.content_end = pos + 1;
                    self.leading = false;
                    self.after_pipe = false;
                }
            }
            self.escaped = byte == b'\\' && !self.escaped;
        }
        self.feed_content(Span::new(segment, span.end), table, effects);
        self.sync_cell(table);
    }

    fn feed_content(&mut self, span: Span, table: &Table, effects: &mut Effects<'_>) {
        if self.column < table.columns.len() {
            self.inline.feed(span, effects);
        } else {
            effects.mark(span, Style::HIDDEN, None);
        }
    }

    fn sync_cell(&self, table: &mut Table) {
        if let Some(cell) = table.rows[self.row].cells.get_mut(self.column) {
            cell.source = Span::new(self.content_start.unwrap_or(self.start), self.content_end);
        }
    }

    fn finish_cell(&mut self, end: usize, table: &mut Table, effects: &mut Effects<'_>) {
        self.inline.finish(effects);
        self.sync_cell(table);
        effects.mark(
            Span::new(self.start, self.content_start.unwrap_or(end)),
            Style::HIDDEN,
            None,
        );
        effects.mark(Span::new(self.content_end, end), Style::HIDDEN, None);
        if let Some(column) = table.columns.get_mut(self.column) {
            let width = self.content_end - self.content_start.unwrap_or(self.content_end);
            if width > column.source_width_hint {
                column.source_width_hint = width;
                effects.mark(table.source, Style::default(), None);
                if effects.update.tables_changed.last() != Some(&self.table) {
                    effects.update.tables_changed.push(self.table);
                }
            }
        }
    }

    pub fn finish(&mut self, end: usize, table: &mut Table, effects: &mut Effects<'_>) {
        if self.complete {
            return;
        }
        if self.after_pipe && self.content_start.is_none() {
            self.inline.finish(effects);
            effects.mark(Span::new(self.start, end), Style::HIDDEN, None);
            if self.column < table.columns.len() {
                table.rows[self.row].cells.pop();
            }
        } else {
            self.finish_cell(end, table, effects);
        }
        self.complete = true;
        table.rows[self.row].complete = true;
    }
}
