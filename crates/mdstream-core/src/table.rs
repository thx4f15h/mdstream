use crate::inline::{Effects, Inline};
use crate::{BlockId, ChangeKind, ContainerId, Span, Style, TableId};

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
    /// Whole physical line, including any hidden container prefix.
    pub source: Span,
    /// Only physically present cells up to the schema width. Missing cells are
    /// implicit empty cells, avoiding rows * columns allocation for sparse rows.
    pub cells: Vec<TableCell>,
    pub complete: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    /// The block owns the table's exact container identity.
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
    // None only while the definition ledger owns this header source.
    pub owner: Option<crate::deferred::SourceOwner>,
    pub block: Option<BlockId>,
    pub source: Span,
    pub content: Span,
    pub container: Option<ContainerId>,
    pub columns: usize,
    // The first-cell prefix may already be consumed. Inherited emphasis openers
    // are discarded on confirmation; published semantics and objects are reused.
    // An inherited opaque code/math prefix is consumed as fresh cell syntax instead.
    pub parsed_end: usize,
    pub leading_break: Option<crate::inline::LineBreak>,
    // An eager root continuation grows its paragraph until table confirmation.
    pub paragraph_end: Option<usize>,
    pub paragraph_tail: Option<Span>,
    // Unconsumed columns of the first source byte (a split quote separator).
    pub virtual_spaces: u8,
}

pub(crate) struct StreamingHeader {
    pub header: HeaderCandidate,
    pub summary: HeaderSummary,
}

/// Column count for split/trim grammar, without storing cells or source text.
/// After the first pipe, defer semantic commitment until the delimiter arrives.
#[derive(Default)]
pub(crate) struct HeaderSummary {
    columns: usize,
    started: bool,
    escaped: bool,
    tail: bool,
    pub pipe: bool,
}

impl HeaderSummary {
    /// Return the first unescaped pipe offset in this chunk, if any.
    pub fn feed(&mut self, text: &str, work: &mut crate::Work) -> Option<usize> {
        let mut first_pipe = None;
        for (offset, byte) in text.bytes().enumerate() {
            work.table_header_bytes += 1;
            if byte == b'|' && !self.escaped {
                first_pipe.get_or_insert(offset);
                self.pipe = true;
                self.columns += usize::from(self.started);
                self.tail = false;
            } else if !byte.is_ascii_whitespace() {
                self.tail = true;
            }
            self.started |= !byte.is_ascii_whitespace();
            self.escaped = byte == b'\\' && !self.escaped;
        }
        first_pipe
    }

    pub fn columns(&self) -> Option<usize> {
        let count = self.columns + usize::from(self.tail);
        (self.pipe && count > 0).then_some(count)
    }
}

/// A long separator keeps only one finite-state cell and at most the
/// header's column count. Each candidate byte is examined once; no source copy.
pub(crate) struct DelimiterCandidate {
    pub prefix: Span,
    columns: usize,
    alignments: Vec<Alignment>,
    cell: DelimiterCell,
    started: bool,
    pipe: bool,
}

#[derive(Clone, Copy)]
enum DelimiterCell {
    Empty,
    Left,
    Dashes(bool),
    Right(bool),
    Space(Alignment),
}

impl DelimiterCell {
    fn alignment(self) -> Option<Alignment> {
        match self {
            Self::Dashes(false) => Some(Alignment::Default),
            Self::Dashes(true) => Some(Alignment::Left),
            Self::Right(false) => Some(Alignment::Right),
            Self::Right(true) => Some(Alignment::Center),
            Self::Space(alignment) => Some(alignment),
            Self::Empty | Self::Left => None,
        }
    }
}

impl DelimiterCandidate {
    pub fn new(prefix: Span, columns: usize) -> Self {
        Self {
            prefix,
            columns,
            alignments: Vec::new(),
            cell: DelimiterCell::Empty,
            started: false,
            pipe: false,
        }
    }

    pub fn feed(&mut self, text: &str, work: &mut crate::Work) -> bool {
        for byte in text.bytes() {
            work.table_candidate_bytes += 1;
            if byte.is_ascii_whitespace() {
                if !matches!(self.cell, DelimiterCell::Empty) {
                    let Some(alignment) = self.cell.alignment() else {
                        return false;
                    };
                    self.cell = DelimiterCell::Space(alignment);
                }
                continue;
            }
            if byte == b'|' {
                self.pipe = true;
                if self.started && !self.push_cell() {
                    return false;
                }
                self.cell = DelimiterCell::Empty;
            } else {
                self.cell = match (byte, self.cell) {
                    (b':', DelimiterCell::Empty) => DelimiterCell::Left,
                    (b'-', DelimiterCell::Empty) => DelimiterCell::Dashes(false),
                    (b'-', DelimiterCell::Left) => DelimiterCell::Dashes(true),
                    (b'-', DelimiterCell::Dashes(left)) => DelimiterCell::Dashes(left),
                    (b':', DelimiterCell::Dashes(left)) => DelimiterCell::Right(left),
                    _ => return false,
                };
            }
            self.started = true;
        }
        true
    }

    fn push_cell(&mut self) -> bool {
        let Some(alignment) = self.cell.alignment() else {
            return false;
        };
        if self.alignments.len() == self.columns {
            return false;
        }
        self.alignments.push(alignment);
        true
    }

    pub fn finish(mut self) -> Option<Vec<Alignment>> {
        if !matches!(self.cell, DelimiterCell::Empty) && !self.push_cell() {
            return None;
        }
        (self.pipe && self.alignments.len() == self.columns).then_some(self.alignments)
    }
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
    parsed_end: usize,
}

impl RowParser {
    pub fn new(id: TableId, table: &mut Table, start: usize, content_start: usize) -> Self {
        let row = table.rows.len();
        table.rows.push(TableRow {
            source: Span::new(start, start),
            cells: vec![TableCell {
                source: Span::new(content_start, content_start),
            }],
            complete: false,
        });
        Self {
            table: id,
            row,
            column: 0,
            start: content_start,
            content_start: None,
            content_end: content_start,
            escaped: false,
            leading: true,
            after_pipe: false,
            complete: false,
            inline: Inline::default(),
            parsed_end: content_start,
        }
    }

    pub fn resume_header(&mut self, inline: Inline, parsed_end: usize) {
        self.inline = inline;
        self.parsed_end = parsed_end;
    }

    pub fn feed(&mut self, span: Span, table: &mut Table, effects: &mut Effects<'_>) {
        effects.update.row_changed(self.table, self.row);
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
            if span.end > self.parsed_end {
                self.inline.feed(
                    Span::new(span.start.max(self.parsed_end), span.end),
                    effects,
                );
            }
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
                // Keep semantic keys conservative, but distinguish the sizing hint.
                effects.annotations.apply(
                    table.source,
                    Style::default(),
                    None,
                    effects.update.revision,
                );
                effects.update.dirty.push(table.source);
                effects.update.change(table.source, ChangeKind::TableWidth);
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
        effects.update.row_changed(self.table, self.row);
        self.complete = true;
        table.rows[self.row].complete = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_summary_matches_split_grammar_with_constant_storage() {
        let alphabet = b"a|\\ \t\r\n";
        assert!(std::mem::size_of::<HeaderSummary>() <= 2 * std::mem::size_of::<usize>());
        for mut value in 0..7_usize.pow(6) {
            let mut bytes = [b' '; 6];
            for byte in &mut bytes {
                *byte = alphabet[value % 7];
                value /= 7;
            }
            let text = std::str::from_utf8(&bytes).unwrap();
            let mut summary = HeaderSummary::default();
            let mut work = crate::Work::default();
            let mut first_pipe = None;
            for (at, byte) in text.as_bytes().chunks(1).enumerate() {
                if summary
                    .feed(std::str::from_utf8(byte).unwrap(), &mut work)
                    .is_some()
                {
                    first_pipe.get_or_insert(at);
                }
            }
            let expected = bytes.iter().enumerate().position(|(at, byte)| {
                *byte == b'|'
                    && bytes[..at]
                        .iter()
                        .rev()
                        .take_while(|&&b| b == b'\\')
                        .count()
                        % 2
                        == 0
            });
            assert_eq!(first_pipe, expected, "{text:?}");
            let mut whole = HeaderSummary::default();
            assert_eq!(whole.feed(text, &mut crate::Work::default()), expected);
            assert_eq!(
                summary.columns(),
                split(text, Span::new(0, 6)).map(|c| c.len()),
                "{text:?}"
            );
            assert_eq!(summary.columns(), whole.columns());
            assert_eq!(work.table_header_bytes, 6);
        }
    }

    #[test]
    fn streaming_delimiter_matches_bounded_grammar_exhaustively() {
        // Independent existing split/trim parser is the oracle. Enumerate all
        // six-byte combinations, including empty cells and internal whitespace.
        let alphabet = b"-:| \t\rx";
        for mut value in 0..7_usize.pow(6) {
            let mut bytes = [b' '; 6];
            for byte in &mut bytes {
                *byte = alphabet[value % 7];
                value /= 7;
            }
            let text = std::str::from_utf8(&bytes).unwrap();
            for columns in 1..=3 {
                let mut candidate = DelimiterCandidate::new(Span::new(0, 6), columns);
                let mut work = crate::Work::default();
                let actual = candidate
                    .feed(text, &mut work)
                    .then(|| candidate.finish())
                    .flatten();
                assert_eq!(
                    actual,
                    delimiter(text, Span::new(0, 6), columns),
                    "{text:?}"
                );
                assert!(work.table_candidate_bytes <= 6);
            }
        }
    }
}
