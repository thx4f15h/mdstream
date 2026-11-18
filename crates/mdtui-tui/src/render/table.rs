use std::fmt;
use std::ops::Range;

use mdtui3::{Alignment, Block, BlockId, BlockKind, Document, Span, TableId};

use crate::render::input::DisplayInput;
use crate::render::text::GraphemeKind;
use crate::render::visibility::visible_input;
use crate::screen::{Cell, LineSource, Screen, ScreenError, StyleRole};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableWindowStatus {
    pub table_id: TableId,
    pub first: usize,
    pub visible: usize,
    pub total: usize,
    pub provisional: bool,
}
impl TableWindowStatus {
    pub fn message(self) -> String {
        format!(
            "table {} cols {}-{} / {} [<{} >{}]{}",
            self.table_id.0 + 1,
            self.first + 1,
            self.first + self.visible,
            self.total,
            self.first,
            self.total.saturating_sub(self.first + self.visible),
            if self.provisional { " ~" } else { "" }
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableGeometry {
    pub table_id: TableId,
    pub block_id: BlockId,
    pub columns: Vec<TableColumnGeometry>,
    pub rows: Vec<TableRowGeometry>,
    pub total_columns: usize,
    pub first_column: usize,
    pub visible_columns: usize,
    /// Lower bound when the prepared column prefix is incomplete.
    pub full_width: usize,
    pub full_width_exact: bool,
    pub column_metadata_reads: usize,
    pub visible_width: usize,
    pub clipped_first_column: bool,
    pub omitted_columns: usize,
    pub row_start: usize,
    pub total_rows: usize,
    pub widths_provisional: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableColumnGeometry {
    pub alignment: Alignment,
    pub content_width: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableRowGeometry {
    pub source: Span,
    pub complete: bool,
    pub header: bool,
    pub cells: Vec<TableCellGeometry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableCellGeometry {
    pub source: Span,
    pub input: DisplayInput,
    pub content_width: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableError {
    NotTable,
    MissingTable,
    MissingContent,
    InvalidWidths,
    RowOverflow,
    Screen(ScreenError),
}

impl fmt::Display for TableError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotTable => formatter.write_str("block is not a table"),
            Self::MissingTable => formatter.write_str("table id is not present in the document"),
            Self::MissingContent => formatter.write_str("table cell source is not a valid range"),
            Self::InvalidWidths => formatter.write_str("table width snapshot has too few columns"),
            Self::RowOverflow => formatter.write_str("table output exceeds terminal rows"),
            Self::Screen(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for TableError {}

impl From<ScreenError> for TableError {
    fn from(error: ScreenError) -> Self {
        Self::Screen(error)
    }
}

pub fn table_geometry(
    document: &Document,
    block_id: BlockId,
    block: &Block,
    content_width: usize,
) -> Result<TableGeometry, TableError> {
    let BlockKind::Table { id } = block.kind else {
        return Err(TableError::NotTable);
    };
    let table = document.table(id).ok_or(TableError::MissingTable)?;
    let total_columns = table.columns.len();
    let mut widths = vec![0_usize; total_columns];
    for row in table.rows() {
        for (column, cell) in row.cells.iter().take(total_columns).enumerate() {
            let width = cell_width(document, cell.source)?;
            widths[column] = widths[column].max(width);
        }
    }
    table_geometry_with_widths(
        document,
        block_id,
        block,
        content_width,
        &widths,
        0..table.rows().len(),
        false,
    )
}

/// Geometry from a conservative width snapshot and a bounded row window. The
/// snapshot caller must provide safe upper bounds (column hints or exact widths)
/// so the visible prefix is never understated. Only rows in `row_range` read cell
/// text; omitted rows are not materialized into `TableGeometry::rows`.
#[allow(clippy::too_many_arguments)]
pub fn table_geometry_with_widths(
    document: &Document,
    block_id: BlockId,
    block: &Block,
    content_width: usize,
    widths: &[usize],
    row_range: Range<usize>,
    widths_provisional: bool,
) -> Result<TableGeometry, TableError> {
    table_geometry_window(
        document,
        block_id,
        block,
        content_width,
        widths,
        row_range,
        widths_provisional,
        None,
        None,
        0,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn table_geometry_window(
    document: &Document,
    block_id: BlockId,
    block: &Block,
    content_width: usize,
    widths: &[usize],
    row_range: Range<usize>,
    widths_provisional: bool,
    source_window: Option<Span>,
    input_window: Option<&DisplayInput>,
    first_column: usize,
) -> Result<TableGeometry, TableError> {
    let BlockKind::Table { id } = block.kind else {
        return Err(TableError::NotTable);
    };
    let table = document.table(id).ok_or(TableError::MissingTable)?;
    let total_columns = table.columns.len();
    let total_rows = table.rows().len();
    if widths.is_empty() && total_columns > 0 && content_width > 0 {
        return Err(TableError::InvalidWidths);
    }
    let first_column = first_column.min(total_columns.saturating_sub(1));
    let widths = &widths[..widths.len().min(total_columns.saturating_sub(first_column))];
    let full_width_exact =
        first_column == 0 && widths.len() == total_columns && !widths_provisional;
    let widths_provisional = widths_provisional || widths.len() < total_columns;
    let full_width = table_width(widths);
    if content_width == 0 || total_columns == 0 {
        return Ok(TableGeometry {
            table_id: id,
            block_id,
            columns: Vec::new(),
            rows: Vec::new(),
            total_columns,
            first_column,
            visible_columns: 0,
            full_width,
            full_width_exact,
            column_metadata_reads: 0,
            visible_width: 0,
            clipped_first_column: false,
            omitted_columns: total_columns,
            row_start: row_range.start.min(total_rows),
            total_rows,
            widths_provisional,
        });
    }

    let row_start = row_range.start.min(total_rows);
    let row_end = row_range.end.clamp(row_start, total_rows);
    let (visible_columns, visible_width) = visible_prefix(widths, content_width);
    let clipped_first_column = visible_width > content_width;
    let columns = table.columns[first_column..]
        .iter()
        .zip(widths.iter())
        .take(visible_columns)
        .map(|(column, width)| TableColumnGeometry {
            alignment: column.alignment,
            content_width: if input_window.is_some() || widths_provisional {
                (*width).min(content_width.max(1))
            } else {
                *width
            },
        })
        .collect();
    let rows = table
        .rows()
        .get(row_start..row_end)
        .unwrap_or(&[])
        .iter()
        .enumerate()
        .map(|(offset, row)| {
            let row_index = row_start + offset;
            let cells = (0..visible_columns)
                .map(|column| {
                    if let Some(cell) = row.cells.get(first_column + column) {
                        let source = source_window.map_or(cell.source, |window| {
                            let start = cell.source.start.max(window.start).min(cell.source.end);
                            let end = cell.source.end.min(window.end).max(start);
                            Span::new(start, end)
                        });
                        let input = if let Some(input) = input_window {
                            input.clipped(source)
                        } else {
                            visible_input(document, source).ok_or(TableError::MissingContent)?
                        };
                        let content_width = input.display_width();
                        Ok(TableCellGeometry {
                            source: cell.source,
                            input,
                            content_width,
                        })
                    } else {
                        Ok(TableCellGeometry {
                            source: Span::new(row.source.end, row.source.end),
                            input: DisplayInput::new(),
                            content_width: 0,
                        })
                    }
                })
                .collect::<Result<Vec<_>, TableError>>()?;
            Ok(TableRowGeometry {
                source: row.source,
                complete: row.complete,
                header: row_index == 0,
                cells,
            })
        })
        .collect::<Result<Vec<_>, TableError>>()?;

    Ok(TableGeometry {
        table_id: id,
        block_id,
        columns,
        rows,
        total_columns,
        first_column,
        visible_columns,
        full_width,
        full_width_exact,
        column_metadata_reads: visible_columns,
        visible_width,
        clipped_first_column,
        omitted_columns: total_columns - visible_columns,
        row_start,
        total_rows,
        widths_provisional,
    })
}

pub(crate) fn cell_width(document: &Document, source: Span) -> Result<usize, TableError> {
    let input = visible_input(document, source).ok_or(TableError::MissingContent)?;
    Ok(input.display_width())
}

pub(crate) fn table_width(widths: &[usize]) -> usize {
    if widths.is_empty() {
        return 0;
    }
    widths.iter().sum::<usize>() + widths.len() * 3 + 1
}

fn visible_prefix(widths: &[usize], content_width: usize) -> (usize, usize) {
    let mut content = 0_usize;
    let mut selected = 0_usize;
    for (index, width) in widths.iter().enumerate() {
        let candidate_content = content.saturating_add(*width);
        let candidate_columns = index + 1;
        let candidate_width = candidate_content.saturating_add(candidate_columns * 3 + 1);
        if candidate_width > content_width {
            break;
        }
        content = candidate_content;
        selected = candidate_columns;
    }
    if selected == 0 {
        (1, widths[0].saturating_add(4))
    } else {
        (selected, content + selected * 3 + 1)
    }
}

pub(crate) fn visible_prefix_count(widths: &[usize], content_width: usize) -> usize {
    if widths.is_empty() || content_width == 0 {
        0
    } else {
        visible_prefix(widths, content_width).0
    }
}

pub fn render_table(
    document: &Document,
    block_id: BlockId,
    block: &Block,
    screen: &mut Screen,
    start_row: u16,
) -> Result<u16, TableError> {
    let geometry = table_geometry(
        document,
        block_id,
        block,
        usize::from(screen.render_width()),
    )?;
    render_table_with_geometry(&geometry, screen, start_row)
}

pub fn render_table_with_geometry(
    geometry: &TableGeometry,
    screen: &mut Screen,
    start_row: u16,
) -> Result<u16, TableError> {
    if geometry.rows.is_empty() || geometry.visible_columns == 0 {
        return Ok(0);
    }
    let row_start = geometry.row_start;
    let row_end = row_start
        .checked_add(geometry.rows.len())
        .ok_or(TableError::RowOverflow)?;
    let mut total = 0_u16;
    if row_start == 0 {
        let header = &geometry.rows[0];
        let top = horizontal_border('┌', '┬', '┐', '─', &geometry.columns)?;
        let header_row = data_row(header, &geometry.columns)?;
        let separator = horizontal_border('╞', '╪', '╡', '═', &geometry.columns)?;
        write_line(screen, start_row, geometry.block_id, header.source, top)?;
        write_line(
            screen,
            start_row.checked_add(1).ok_or(TableError::RowOverflow)?,
            geometry.block_id,
            header.source,
            header_row,
        )?;
        write_line(
            screen,
            start_row.checked_add(2).ok_or(TableError::RowOverflow)?,
            geometry.block_id,
            header.source,
            separator,
        )?;
        total = 3;
    }
    for row_index in row_start.max(1)..row_end {
        let row = &geometry.rows[row_index - row_start];
        let output_row = start_row
            .checked_add(total)
            .ok_or(TableError::RowOverflow)?;
        write_line(
            screen,
            output_row,
            geometry.block_id,
            row.source,
            data_row(row, &geometry.columns)?,
        )?;
        total = total.checked_add(1).ok_or(TableError::RowOverflow)?;
    }
    if row_end == geometry.total_rows && geometry.total_rows > 1 {
        let last = geometry.rows.last().ok_or(TableError::RowOverflow)?;
        let bottom = horizontal_border('└', '┴', '┘', '─', &geometry.columns)?;
        let output_row = start_row
            .checked_add(total)
            .ok_or(TableError::RowOverflow)?;
        write_line(screen, output_row, geometry.block_id, last.source, bottom)?;
        total = total.checked_add(1).ok_or(TableError::RowOverflow)?;
    }
    Ok(total)
}

/// A partial table whose final rows are folded still closes after its PH.
pub(crate) fn render_folded_bottom(
    geometry: &TableGeometry,
    source: Span,
    screen: &mut Screen,
) -> Result<(), TableError> {
    let bottom = horizontal_border('└', '┴', '┘', '─', &geometry.columns)?;
    write_line(screen, 0, geometry.block_id, source, bottom)
}

/// Source-bounded table row window. Header row zero may be included when the
/// requested source overlaps it; otherwise only intersecting body rows survive.
#[must_use]
pub(crate) fn table_row_window(table: &mdtui3::Table, source: Span) -> Range<usize> {
    let total = table.rows().len();
    if total == 0
        || source.is_empty()
        || source.end <= table.source.start
        || source.start >= table.source.end
    {
        return 0..0;
    }
    let start = if source.start <= table.source.start {
        0
    } else {
        table.row_at(source.start).unwrap_or_else(|| {
            table
                .rows()
                .partition_point(|row| row.source.start < source.start)
                .min(total)
        })
    };
    let end = table
        .rows()
        .partition_point(|row| row.source.start < source.end)
        .min(total);
    start.min(end)..end
}

fn horizontal_border(
    left: char,
    junction: char,
    right: char,
    fill: char,
    columns: &[TableColumnGeometry],
) -> Result<Vec<Cell>, TableError> {
    let mut cells = Vec::new();
    cells.push(border_cell(left)?);
    for (index, column) in columns.iter().enumerate() {
        for _ in 0..column.content_width + 2 {
            cells.push(border_cell(fill)?);
        }
        cells.push(border_cell(if index + 1 == columns.len() {
            right
        } else {
            junction
        })?);
    }
    Ok(cells)
}

fn data_row(
    row: &TableRowGeometry,
    columns: &[TableColumnGeometry],
) -> Result<Vec<Cell>, TableError> {
    let mut cells = Vec::new();
    cells.push(border_cell('│')?);
    for (column, cell) in columns.iter().zip(row.cells.iter()) {
        cells.push(border_cell(' ')?);
        let slack = column.content_width.saturating_sub(cell.content_width);
        let leading = match column.alignment {
            Alignment::Center => slack / 2,
            Alignment::Right => slack,
            Alignment::Left | Alignment::Default => 0,
        };
        for _ in 0..leading {
            cells.push(border_cell(' ')?);
        }
        let mut used = leading;
        for glyph in cell.input.glyphs() {
            if matches!(glyph.kind, GraphemeKind::Control) {
                continue;
            }
            let width = usize::from(glyph.width);
            if used + width > column.content_width {
                break;
            }
            cells.push(Cell::new(
                glyph.text,
                glyph.style,
                glyph.width,
                glyph.link.filter(|_| {
                    glyph.style.contains(StyleRole::IMAGE)
                        || glyph.style.contains(StyleRole::LITERAL_LINK)
                }),
                Some(glyph.source),
            )?);
            used += width;
        }
        for _ in used..column.content_width {
            cells.push(border_cell(' ')?);
        }
        cells.push(border_cell(' ')?);
        cells.push(border_cell('│')?);
    }
    Ok(cells)
}

fn border_cell(text: char) -> Result<Cell, TableError> {
    let mut buffer = [0_u8; 4];
    Ok(Cell::new(
        text.encode_utf8(&mut buffer),
        StyleRole::TABLE_BORDER,
        1,
        None,
        None,
    )?)
}

fn write_line(
    screen: &mut Screen,
    row: u16,
    block_id: BlockId,
    source: Span,
    cells: Vec<Cell>,
) -> Result<(), TableError> {
    screen.write_unwrapped(row, screen.content_area().x, cells)?;
    screen.set_line_source(row, LineSource::new(source, block_id))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> Document {
        let mut document = Document::new();
        document.append(source).unwrap();
        document.finish();
        document
    }

    fn tables(document: &Document) -> Vec<(BlockId, &Block, TableId)> {
        document
            .blocks()
            .iter()
            .enumerate()
            .filter_map(|(block_index, block)| match block.kind {
                BlockKind::Table { id } => Some((BlockId(block_index), block, id)),
                _ => None,
            })
            .collect()
    }

    fn geometry(document: &Document, width: usize) -> TableGeometry {
        let (id, block, _) = tables(document)[0];
        table_geometry(document, id, block, width).unwrap()
    }

    fn row_text(screen: &Screen, row: u16) -> String {
        let content = screen.content_area();
        let mut text = String::new();
        for col in content.x..content.right() {
            text.push_str(&screen.cell(row, col).unwrap().text().collect_text());
        }
        text
    }

    #[test]
    fn formula_alignments_header_and_rows_are_exact() {
        let document = parse(include_str!("../../tests/fixtures/spec/tables.md"));
        let table = geometry(&document, 80);
        assert_eq!(table.total_columns, 3);
        assert_eq!(table.visible_columns, 3);
        assert_eq!(table.omitted_columns, 0);
        assert_eq!(table.columns[0].alignment, Alignment::Left);
        assert_eq!(table.columns[1].alignment, Alignment::Center);
        assert_eq!(table.columns[2].alignment, Alignment::Right);
        assert_eq!(table.columns[0].content_width, 5);
        assert_eq!(table.columns[1].content_width, 5);
        assert_eq!(table.columns[2].content_width, 6);
        assert_eq!(table.full_width, 5 + 5 + 6 + 3 * 3 + 1);
        assert_eq!(table.visible_width, table.full_width);
        assert!(!table.clipped_first_column);
        assert_eq!(table.rows.len(), 3);
        assert!(table.rows[0].header);
        assert!(!table.rows[1].header);
        assert!(table.rows.iter().all(|row| row.cells.len() == 3));
    }

    #[test]
    fn header_only_empty_missing_and_extra_cells_are_supported() {
        let header_only = parse("| Header only |\n| --- |\n");
        let geometry = geometry(&header_only, 80);
        assert_eq!(geometry.rows.len(), 1);
        assert!(geometry.rows[0].header);
        assert_eq!(geometry.columns[0].content_width, 11);
        assert_eq!(geometry.full_width, 11 + 4);

        let sparse = parse(concat!(
            "| A | B | C |\n",
            "| --- | --- | --- |\n",
            "| one |  | three |\n",
            "|  | two |  |\n",
            "\n",
            "| A | B |\n",
            "| --- | --- |\n",
            "| one | two | dropped extra cell |\n",
            "\n",
            "| A | B |\n",
            "| --- | --- |\n",
            "| one |\n",
        ));
        let all = tables(&sparse);
        assert_eq!(all.len(), 3);
        let first = table_geometry(&sparse, all[0].0, all[0].1, 80).unwrap();
        assert_eq!(first.rows.len(), 3);
        assert_eq!(first.rows[1].cells[1].content_width, 0);
        assert_eq!(first.rows[1].cells[1].input.visible_text(), "");

        let extra = table_geometry(&sparse, all[1].0, all[1].1, 80).unwrap();
        assert_eq!(extra.total_columns, 2);
        assert_eq!(extra.rows[1].cells.len(), 2);

        let missing = table_geometry(&sparse, all[2].0, all[2].1, 80).unwrap();
        assert_eq!(missing.rows[1].cells.len(), 2);
        assert_eq!(missing.rows[1].cells[1].content_width, 0);
        assert!(missing.rows[1].cells[1].input.is_empty());
    }

    #[test]
    fn cjk_emoji_inline_metadata_and_p16_filtering_are_measured() {
        let document = parse(include_str!("../../tests/fixtures/spec/tables.md"));
        let (id, block, _) = tables(&document)[4];
        let table = table_geometry(&document, id, block, 80).unwrap();
        assert_eq!(table.columns[0].content_width, 4);
        assert_eq!(table.rows[1].cells[0].content_width, 2);
        assert_eq!(table.rows[2].cells[0].content_width, 2);
        assert!(
            table.rows[1].cells[1]
                .input
                .tokens()
                .iter()
                .any(|token| token.style.contains(crate::screen::StyleRole::STRONG))
        );
        assert!(
            table.rows[1].cells[1]
                .input
                .tokens()
                .iter()
                .any(|token| token.style.contains(crate::screen::StyleRole::CODE))
        );
        assert!(
            table.rows[1].cells[2]
                .input
                .tokens()
                .iter()
                .any(|token| token.link.is_some())
        );

        let image_table = parse("| H |\n| --- |\n| a ![x](i) b |\n");
        let table = geometry(&image_table, 80);
        assert_eq!(table.columns[0].content_width, 5);
        assert_eq!(table.rows[1].cells[0].input.visible_text(), "a x b");
        assert!(
            table.rows[1].cells[0]
                .input
                .tokens()
                .iter()
                .any(|token| token.link.is_some())
        );
    }

    #[test]
    fn visible_prefix_always_keeps_first_column_and_clips_when_needed() {
        let document = parse(include_str!("../../tests/fixtures/spec/overwide.md"));
        let (id, block, _) = tables(&document)[0];
        let zero = table_geometry(&document, id, block, 0).unwrap();
        assert_eq!(zero.visible_columns, 0);
        assert!(zero.columns.is_empty());
        assert!(zero.rows.is_empty());
        assert_eq!(zero.omitted_columns, 20);

        let width_one = table_geometry(&document, id, block, 1).unwrap();
        assert_eq!(width_one.visible_columns, 1);
        assert_eq!(width_one.columns.len(), 1);
        assert_eq!(width_one.rows.len(), 3);
        assert!(width_one.rows.iter().all(|row| row.cells.len() == 1));
        assert!(width_one.clipped_first_column);
        assert!(width_one.visible_width > 1);
        assert_eq!(width_one.omitted_columns, 19);
        assert_eq!(width_one.full_width, 20 * 9 + 3 * 20 + 1);

        let width_20 = table_geometry(&document, id, block, 20).unwrap();
        assert_eq!(width_20.visible_columns, 1);
        assert_eq!(width_20.visible_width, 13);
        assert!(!width_20.clipped_first_column);

        let width_80 = table_geometry(&document, id, block, 80).unwrap();
        assert_eq!(width_80.visible_columns, 6);
        assert_eq!(width_80.visible_width, 73);
        assert_eq!(width_80.omitted_columns, 14);

        let width_120 = table_geometry(&document, id, block, 120).unwrap();
        assert_eq!(width_120.visible_columns, 9);
        assert_eq!(width_120.visible_width, 109);
        assert_eq!(width_120.omitted_columns, 11);
        assert!(
            width_120
                .rows
                .iter()
                .all(|row| row.cells.len() == width_120.visible_columns)
        );
    }

    #[test]
    fn malformed_tables_are_not_table_blocks_and_missing_ids_are_explicit() {
        let paragraph = parse("| A | B |\n| --- |\n| one | two |\n");
        let block = &paragraph.blocks()[0];
        assert_eq!(
            table_geometry(&paragraph, BlockId(0), block, 80),
            Err(TableError::NotTable)
        );

        let table = parse("| A |\n| --- |\n| one |\n");
        let (_, block, _) = tables(&table)[0];
        let missing = Block {
            source: block.source,
            kind: BlockKind::Table {
                id: TableId(usize::MAX),
            },
            state: block.state,
            container: block.container,
        };
        assert_eq!(
            table_geometry(&table, BlockId(0), &missing, 80),
            Err(TableError::MissingTable)
        );
    }

    #[test]
    fn fixture_tables_follow_the_geometry_formula_at_target_widths() {
        for source in [
            include_str!("../../tests/fixtures/spec/tables.md"),
            include_str!("../../tests/fixtures/spec/overwide.md"),
        ] {
            let document = parse(source);
            for width in [0_usize, 1, 20, 80, 120] {
                for (id, block, _) in tables(&document) {
                    let geometry = table_geometry(&document, id, block, width).unwrap();
                    assert!(geometry.full_width >= geometry.visible_width);
                    assert!(geometry.visible_columns <= geometry.total_columns);
                    assert_eq!(
                        geometry.omitted_columns,
                        geometry.total_columns - geometry.visible_columns
                    );
                    assert_eq!(geometry.columns.len(), geometry.visible_columns);
                    assert!(
                        geometry
                            .rows
                            .iter()
                            .all(|row| row.cells.len() == geometry.visible_columns)
                    );
                    if width == 0 {
                        assert_eq!(geometry.visible_columns, 0);
                        assert!(geometry.rows.is_empty());
                    } else if geometry.total_columns > 0 {
                        assert!(geometry.visible_columns >= 1);
                        assert!(geometry.visible_width <= width || geometry.clipped_first_column);
                    }
                }
            }
        }
    }

    #[test]
    fn two_by_two_grid_is_exact_and_border_styles_are_separate() {
        let document = parse("| A | B |\n| --- | --- |\n| 1 | 2 |\n");
        let (id, block, _) = tables(&document)[0];
        let mut screen = Screen::new(5, 20, crate::screen::ContentArea::new(0, 20)).unwrap();
        assert_eq!(render_table(&document, id, block, &mut screen, 0), Ok(5));
        let expected = [
            "┌───┬───┐",
            "│ A │ B │",
            "╞═══╪═══╡",
            "│ 1 │ 2 │",
            "└───┴───┘",
        ];
        for (row, line) in expected.iter().enumerate() {
            assert_eq!(row_text(&screen, row as u16).trim_end(), *line);
            assert!(screen.line_source(row as u16).is_some());
        }
        for col in [0_usize, 4, 8] {
            assert_eq!(
                screen.cell(1, col as u16).unwrap().style(),
                StyleRole::TABLE_BORDER
            );
        }
        assert_eq!(screen.cell(1, 1).unwrap().style(), StyleRole::TABLE_BORDER);
        assert_eq!(screen.cell(1, 2).unwrap().style(), StyleRole::DEFAULT);
        assert_eq!(screen.cell(1, 3).unwrap().style(), StyleRole::TABLE_BORDER);
        assert_eq!(screen.cell(1, 6).unwrap().style(), StyleRole::DEFAULT);
        assert_eq!(screen.cell(3, 2).unwrap().style(), StyleRole::DEFAULT);
        assert_eq!(screen.cell(3, 6).unwrap().style(), StyleRole::DEFAULT);
    }

    #[test]
    fn header_only_table_has_three_rows_without_bottom() {
        let document = parse("| Header only |\n| --- |\n");
        let (id, block, _) = tables(&document)[0];
        let mut screen = Screen::new(3, 40, crate::screen::ContentArea::new(0, 40)).unwrap();
        assert_eq!(render_table(&document, id, block, &mut screen, 0), Ok(3));
        assert_eq!(
            row_text(&screen, 0).trim_end(),
            format!("┌{}┐", "─".repeat(13))
        );
        assert_eq!(row_text(&screen, 1).trim_end(), "│ Header only │");
        assert_eq!(
            row_text(&screen, 2).trim_end(),
            format!("╞{}╡", "═".repeat(13))
        );
        assert!(!row_text(&screen, 2).contains('└'));
    }

    #[test]
    fn empty_missing_and_extra_cells_render_left_aligned() {
        let document = parse(concat!(
            "| A | B |\n",
            "| --- | --- |\n",
            "| one |\n",
            "| two | three | dropped extra cell |\n",
        ));
        let (id, block, _) = tables(&document)[0];
        let mut screen = Screen::new(6, 40, crate::screen::ContentArea::new(0, 40)).unwrap();
        assert_eq!(render_table(&document, id, block, &mut screen, 0), Ok(6));
        assert_eq!(row_text(&screen, 0).trim_end(), "┌─────┬───────┐");
        assert_eq!(row_text(&screen, 1).trim_end(), "│ A   │ B     │");
        assert_eq!(row_text(&screen, 2).trim_end(), "╞═════╪═══════╡");
        assert_eq!(row_text(&screen, 3).trim_end(), "│ one │       │");
        assert_eq!(row_text(&screen, 4).trim_end(), "│ two │ three │");
        assert_eq!(row_text(&screen, 5).trim_end(), "└─────┴───────┘");
        assert_eq!(screen.cell(3, 7).unwrap().style(), StyleRole::TABLE_BORDER);
    }

    #[test]
    fn cjk_emoji_padding_and_wide_continuations_are_styled() {
        let document = parse("| 中 | E |\n| --- | --- |\n| 🙂 | 文 |\n");
        let (id, block, _) = tables(&document)[0];
        let mut screen = Screen::new(5, 20, crate::screen::ContentArea::new(0, 20)).unwrap();
        assert_eq!(render_table(&document, id, block, &mut screen, 0), Ok(5));
        assert_eq!(screen.cell(1, 2).unwrap().text(), "中");
        assert!(screen.cell(1, 3).unwrap().is_continuation());
        assert_eq!(screen.cell(1, 3).unwrap().style(), StyleRole::DEFAULT);
        assert_eq!(screen.cell(1, 5).unwrap().style(), StyleRole::TABLE_BORDER);
        assert_eq!(screen.cell(3, 2).unwrap().text(), "🙂");
        assert!(screen.cell(3, 3).unwrap().is_continuation());
        assert_eq!(screen.cell(3, 3).unwrap().style(), StyleRole::DEFAULT);
        assert_eq!(screen.cell(3, 7).unwrap().text(), "文");
        assert!(screen.cell(3, 8).unwrap().is_continuation());
    }

    #[test]
    fn inline_styles_survive_but_table_links_have_no_link_id() {
        let document = parse("| H |\n| --- |\n| **b** `a b` [l](u) |\n");
        let (id, block, _) = tables(&document)[0];
        let mut screen = Screen::new(5, 30, crate::screen::ContentArea::new(0, 30)).unwrap();
        assert_eq!(render_table(&document, id, block, &mut screen, 0), Ok(5));
        assert_eq!(screen.cell(3, 2).unwrap().style(), StyleRole::STRONG);
        assert_eq!(screen.cell(3, 3).unwrap().style(), StyleRole::DEFAULT);
        assert_eq!(screen.cell(3, 4).unwrap().style(), StyleRole::CODE);
        assert_eq!(screen.cell(3, 5).unwrap().style(), StyleRole::CODE);
        assert_eq!(screen.cell(3, 5).unwrap().text(), " ");
        assert_eq!(screen.cell(3, 6).unwrap().style(), StyleRole::CODE);
        assert_eq!(screen.cell(3, 8).unwrap().style(), StyleRole::LINK);
        for col in 0..11 {
            assert_eq!(screen.cell(3, col).unwrap().link_id(), None);
        }
        assert_eq!(screen.cell(3, 9).unwrap().style(), StyleRole::TABLE_BORDER);
    }

    #[test]
    fn overwide_table_keeps_visible_column_prefix_and_never_wraps() {
        let document = parse(include_str!("../../tests/fixtures/spec/overwide.md"));
        let (id, block, _) = tables(&document)[0];
        for (width, visible_columns) in [(20_u16, 1_usize), (80, 6), (120, 9)] {
            let mut screen =
                Screen::new(6, width, crate::screen::ContentArea::new(0, width)).unwrap();
            assert_eq!(render_table(&document, id, block, &mut screen, 0), Ok(6));
            let geometry = table_geometry(&document, id, block, usize::from(width)).unwrap();
            assert_eq!(geometry.visible_columns, visible_columns);
            assert_eq!(screen.cell(0, 0).unwrap().text(), "┌");
            assert_eq!(screen.cell(2, 0).unwrap().text(), "╞");
            assert_eq!(screen.cell(5, 0).unwrap().text(), "└");
            for row in 0..6 {
                assert!(screen.line_source(row).is_some());
                assert!(screen.cell(row, width - 1).is_some());
            }
            assert!(geometry.visible_width <= usize::from(width));
        }
    }

    #[test]
    fn degenerate_widths_offset_and_quoted_table_are_safe() {
        let document = parse("| A | B |\n| --- | --- |\n| 1 | 2 |\n");
        let (id, block, _) = tables(&document)[0];
        let mut zero = Screen::new(1, 1, crate::screen::ContentArea::new(0, 0)).unwrap();
        assert_eq!(render_table(&document, id, block, &mut zero, 0), Ok(0));
        assert!(zero.line_source(0).is_none());

        for width in [1_u16, 2, 4] {
            let mut screen =
                Screen::new(5, width, crate::screen::ContentArea::new(0, width)).unwrap();
            assert_eq!(render_table(&document, id, block, &mut screen, 0), Ok(5));
            for row in 0..5 {
                assert!(screen.line_source(row).is_some());
            }
        }

        let mut offset = Screen::new(5, 14, crate::screen::ContentArea::new(5, 8)).unwrap();
        assert_eq!(render_table(&document, id, block, &mut offset, 0), Ok(5));
        assert_eq!(offset.cell(0, 5).unwrap().text(), "┌");
        for col in 0..5 {
            assert_eq!(offset.cell(0, col).unwrap().text(), " ");
        }
        for col in 13..14 {
            assert_eq!(offset.cell(0, col).unwrap().text(), " ");
        }

        let quoted = parse("> | A | B |\n> | --- | --- |\n> | 1 | 2 |\n");
        let (id, block, _) = tables(&quoted)[0];
        let mut screen = Screen::new(5, 20, crate::screen::ContentArea::new(0, 20)).unwrap();
        assert_eq!(render_table(&quoted, id, block, &mut screen, 0), Ok(5));
        assert_eq!(screen.cell(0, 0).unwrap().text(), "┌");
        assert_ne!(screen.cell(0, 0).unwrap().text(), "│");
        assert_eq!(screen.cell(1, 0).unwrap().text(), "│");
    }

    #[test]
    fn table_fixtures_render_expected_rows_without_overflow() {
        for source in [
            include_str!("../../tests/fixtures/spec/tables.md"),
            include_str!("../../tests/fixtures/spec/overwide.md"),
        ] {
            let document = parse(source);
            for width in [1_u16, 2, 20, 80, 120] {
                let mut screen =
                    Screen::new(400, width, crate::screen::ContentArea::new(0, width)).unwrap();
                let mut row = 0_u16;
                for (id, block, _) in tables(&document) {
                    let geometry =
                        table_geometry(&document, id, block, usize::from(width)).unwrap();
                    let body = geometry.rows.len().saturating_sub(1);
                    let expected = if geometry.visible_columns == 0 || geometry.rows.is_empty() {
                        0
                    } else {
                        3 + body + usize::from(body > 0)
                    };
                    let rendered = render_table(&document, id, block, &mut screen, row).unwrap();
                    assert_eq!(rendered as usize, expected, "width {width}");
                    for line in row..row + rendered {
                        assert!(screen.line_source(line).is_some());
                    }
                    row += rendered;
                }
                assert!(row > 0);
                assert!(row < 400);
            }
        }
    }

    #[test]
    fn oversized_first_column_and_wide_content_clip_without_splitting() {
        let document = parse("| abcdefgh |\n| --- |\n| 中 |\n");
        let (id, block, _) = tables(&document)[0];
        let mut screen = Screen::new(5, 4, crate::screen::ContentArea::new(0, 4)).unwrap();
        assert_eq!(render_table(&document, id, block, &mut screen, 0), Ok(5));
        assert_eq!(row_text(&screen, 0).trim_end(), "┌───");
        assert_eq!(row_text(&screen, 1).trim_end(), "│ ab");
        assert_eq!(screen.cell(3, 0).unwrap().text(), "│");
        assert_eq!(screen.cell(3, 1).unwrap().text(), " ");
        assert_eq!(screen.cell(3, 2).unwrap().text(), "中");
        assert!(screen.cell(3, 3).unwrap().is_continuation());
        for row in 0..5 {
            assert!(screen.cell(row, 4).is_none());
        }

        let mut width_two = Screen::new(5, 2, crate::screen::ContentArea::new(0, 2)).unwrap();
        assert_eq!(render_table(&document, id, block, &mut width_two, 0), Ok(5));
        assert_eq!(width_two.cell(3, 0).unwrap().text(), "│");
        assert_eq!(width_two.cell(3, 1).unwrap().text(), " ");
        assert!(width_two.cell(3, 2).is_none());
    }
}
