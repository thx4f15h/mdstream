use mdtui3::{BlockId, BlockKind, Document, Span};

use crate::cli::LinkMode;
use crate::render::code::{code_label, code_lines};
use crate::render::document::{RenderStats, StaticRenderError, render_static_screen};
use crate::render::heading::{HeadingError, compose_heading_text};
use crate::render::link::encode_link;
use crate::render::table::table_geometry;
use crate::render::text::display_width;
use crate::render::visibility::{is_hidden_block, visible_input};
use crate::screen::{Cell, ContentArea, Screen, StyleRole};
use crate::theme::MONOKAI;

pub const ONCE_CLEAR: &[u8] = b"\x1b[2J\x1b[H";

#[derive(Debug)]
pub struct StaticRender {
    pub screen: Screen,
    pub stats: RenderStats,
}

pub fn render_static(
    document: &Document,
    width: u16,
    height: u16,
) -> Result<StaticRender, StaticRenderError> {
    let mut screen = Screen::new(height, width, ContentArea::new(0, width))?;
    let stats = render_static_screen(document, &mut screen)?;
    Ok(StaticRender { screen, stats })
}

/// Render the complete reference ANSI stream used by `--once`.
///
/// Paragraph-like blocks wrap at `width`; headings, code lines and tables use
/// the larger storage width computed from the document and therefore overflow
/// instead of being clipped. The height still bounds the total row count.
pub fn render_once_ansi(
    document: &Document,
    width: u16,
    height: u16,
    link_mode: LinkMode,
) -> Result<Vec<u8>, StaticRenderError> {
    let storage_width = once_storage_width(document, width)?;
    let mut screen = Screen::new_unclipped(height, storage_width, width)?;
    let stats = render_static_screen(document, &mut screen)?;
    Ok(encode_once(&screen, stats, document, link_mode))
}

fn once_storage_width(document: &Document, width: u16) -> Result<u16, StaticRenderError> {
    let mut max_width = usize::from(width);
    for (index, block) in document.blocks().iter().enumerate() {
        if is_hidden_block(block) || block.kind == BlockKind::Blank {
            continue;
        }
        let block_id = BlockId(index);
        match block.kind {
            BlockKind::Heading { .. } => {
                let Some(heading) = document
                    .headings()
                    .entries()
                    .iter()
                    .find(|heading| heading.block == block_id)
                else {
                    return Err(StaticRenderError::MissingHeading(block_id));
                };
                let input = visible_input(document, heading.content)
                    .ok_or(StaticRenderError::Heading(HeadingError::MissingContent))?;
                max_width = max_width.max(display_width(&compose_heading_text(&input)));
            }
            BlockKind::Code { .. } => {
                if let Some(label) = code_label(document, block)? {
                    max_width = max_width.max(display_width(&label));
                }
                if let Some(lines) = code_lines(document, block)? {
                    max_width = max_width.max(
                        lines
                            .iter()
                            .map(|line| display_width(&line.text).saturating_add(2))
                            .max()
                            .unwrap_or(0),
                    );
                }
            }
            BlockKind::Table { .. } => {
                max_width = max_width
                    .max(table_geometry(document, block_id, block, usize::MAX)?.full_width);
            }
            _ => {}
        }
    }
    u16::try_from(max_width).map_err(|_| StaticRenderError::ColumnOverflow)
}

fn encode_once(
    screen: &Screen,
    stats: RenderStats,
    document: &Document,
    link_mode: LinkMode,
) -> Vec<u8> {
    if stats.rows == 0 {
        return Vec::new();
    }
    let mut output = String::from_utf8_lossy(ONCE_CLEAR).into_owned();
    for row in 0..stats.rows {
        encode_row(&mut output, screen, document, link_mode, row);
        if table_continues_on_next_row(screen, row, document) {
            output.push('\n');
        } else {
            output.push_str("\r\n");
        }
    }
    output.into_bytes()
}

fn encode_row(
    output: &mut String,
    screen: &Screen,
    document: &Document,
    link_mode: LinkMode,
    row: u16,
) {
    let Some(cells) = screen.back_row(row) else {
        return;
    };
    let mut end = 0;
    for (column, cell) in cells.iter().enumerate() {
        if !cell.is_continuation() && significant(cell) {
            end = column + 1;
        }
    }
    if end == 0 {
        return;
    }

    // Code block bytes historically group padding and body in one SGR run.
    // Source metadata now identifies selectable glyphs without changing that
    // raw ANSI contract; inline code in other blocks keeps source boundaries.
    let code_row = screen
        .line_source(row)
        .and_then(|source| source.block)
        .and_then(|id| document.block(id))
        .is_some_and(|block| matches!(block.kind, BlockKind::Code { .. }));
    let mut segment = RawSegment::new(&cells[0]);
    for cell in &cells[1..end] {
        if cell.is_continuation() {
            continue;
        }
        if segment.accepts(cell, code_row) {
            segment.text.push_str(cell.text());
        } else {
            segment.write(output, document, link_mode);
            segment = RawSegment::new(cell);
        }
    }
    segment.write(output, document, link_mode);
}

struct RawSegment {
    style: StyleRole,
    link_id: Option<mdtui3::LinkId>,
    text: String,
    last_source: Option<Span>,
}

impl RawSegment {
    fn new(cell: &Cell) -> Self {
        Self {
            style: cell.style(),
            link_id: cell.link_id(),
            text: cell.text().to_owned(),
            last_source: cell.source(),
        }
    }

    fn accepts(&mut self, cell: &Cell, code_row: bool) -> bool {
        if self.style != cell.style() || self.link_id != cell.link_id() {
            return false;
        }
        if code_row || self.style == StyleRole::DEFAULT {
            self.last_source = cell.source();
            return true;
        }
        let accepts = match (self.last_source, cell.source()) {
            (None, None) => true,
            (Some(previous), Some(next)) => next.start <= previous.end,
            _ => false,
        };
        if accepts {
            self.last_source = cell.source();
        }
        accepts
    }

    fn write(&mut self, output: &mut String, document: &Document, link_mode: LinkMode) {
        if self.text.is_empty() {
            return;
        }
        if let Some(link_id) = self.link_id {
            if let Some(encoded) =
                encode_link(document, link_id, &self.text, self.style, link_mode, true)
            {
                output.push_str(&encoded);
            } else {
                output.push_str(
                    &MONOKAI.encode_segment(self.style.with(StyleRole::LINK), &self.text),
                );
            }
        } else {
            output.push_str(&MONOKAI.encode_segment(self.style, &self.text));
        }
    }
}

fn significant(cell: &Cell) -> bool {
    !(cell.text() == " " && cell.style() == StyleRole::DEFAULT && cell.link_id().is_none())
}

fn table_continues_on_next_row(screen: &Screen, row: u16, document: &Document) -> bool {
    let current = screen.line_source(row).and_then(|source| source.block);
    let next = screen
        .line_source(row.saturating_add(1))
        .and_then(|source| source.block);
    current.is_some()
        && current == next
        && current.is_some_and(|block| {
            document
                .block(block)
                .is_some_and(|block| matches!(block.kind, BlockKind::Table { .. }))
        })
}
