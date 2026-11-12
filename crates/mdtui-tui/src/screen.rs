use std::fmt;

use mdtui3::{BlockId, LinkId, Span};

const HEADING_FLAG: u32 = 1 << 0;
const STRONG_FLAG: u32 = 1 << 1;
const EMPHASIS_FLAG: u32 = 1 << 2;
const STRIKE_FLAG: u32 = 1 << 3;
const CODE_FLAG: u32 = 1 << 4;
const TASK_CHECKED_FLAG: u32 = 1 << 12;
const LINK_FLAG: u32 = 1 << 5;
const QUOTE_FLAG: u32 = 1 << 6;
const LIST_MARKER_FLAG: u32 = 1 << 7;
const RULE_FLAG: u32 = 1 << 8;
const TABLE_BORDER_FLAG: u32 = 1 << 9;
const META_FLAG: u32 = 1 << 10;
const MATH_FLAG: u32 = 1 << 11;
const SIDEBAR_FLAG: u32 = 1 << 13;
const SIDEBAR_CURRENT_FLAG: u32 = 1 << 14;
const LINK_FOCUS_FLAG: u32 = 1 << 15;
const SIDEBAR_SELECTION_FLAG: u32 = 1 << 16;
const IMAGE_FLAG: u32 = 1 << 17;
const LITERAL_LINK_FLAG: u32 = 1 << 18;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StyleRole {
    flags: u32,
    heading_level: u8,
}

impl StyleRole {
    pub const DEFAULT: Self = Self::new(0, 0);
    pub const HEADING: Self = Self::new(HEADING_FLAG, 0);
    pub const STRONG: Self = Self::new(STRONG_FLAG, 0);
    pub const EMPHASIS: Self = Self::new(EMPHASIS_FLAG, 0);
    pub const STRIKE: Self = Self::new(STRIKE_FLAG, 0);
    pub const CODE: Self = Self::new(CODE_FLAG, 0);
    pub const TASK_CHECKED: Self = Self::new(TASK_CHECKED_FLAG, 0);
    pub const LINK: Self = Self::new(LINK_FLAG, 0);
    pub const IMAGE: Self = Self::new(IMAGE_FLAG, 0);
    pub const LITERAL_LINK: Self = Self::new(LITERAL_LINK_FLAG, 0);
    pub const QUOTE: Self = Self::new(QUOTE_FLAG, 0);
    pub const LIST_MARKER: Self = Self::new(LIST_MARKER_FLAG, 0);
    pub const RULE: Self = Self::new(RULE_FLAG, 0);
    pub const TABLE_BORDER: Self = Self::new(TABLE_BORDER_FLAG, 0);
    pub const META: Self = Self::new(META_FLAG, 0);
    pub const MATH: Self = Self::new(MATH_FLAG, 0);
    pub const SIDEBAR: Self = Self::new(SIDEBAR_FLAG, 0);
    pub const SIDEBAR_CURRENT: Self = Self::new(SIDEBAR_CURRENT_FLAG, 0);
    pub const SIDEBAR_SELECTION: Self = Self::new(SIDEBAR_SELECTION_FLAG, 0);
    pub const LINK_FOCUS: Self = Self::new(LINK_FOCUS_FLAG, 0);

    const fn new(flags: u32, heading_level: u8) -> Self {
        Self {
            flags,
            heading_level,
        }
    }

    #[must_use]
    pub const fn heading(level: u8) -> Self {
        assert!(level >= 1 && level <= 6);
        Self::new(HEADING_FLAG, level)
    }

    #[must_use]
    pub const fn with(self, other: Self) -> Self {
        let heading_level = if other.heading_level == 0 {
            self.heading_level
        } else {
            other.heading_level
        };
        Self::new(self.flags | other.flags, heading_level)
    }

    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.flags & other.flags == other.flags
    }

    #[must_use]
    pub const fn heading_level(self) -> Option<u8> {
        if self.flags & HEADING_FLAG == 0 {
            None
        } else {
            Some(self.heading_level)
        }
    }

    #[must_use]
    pub const fn is_default(self) -> bool {
        self.flags == 0
    }
}

impl Default for StyleRole {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    text: String,
    style: StyleRole,
    width: u8,
    continuation: bool,
    link_id: Option<LinkId>,
    source: Option<Span>,
}

impl Cell {
    pub fn new(
        text: impl Into<String>,
        style: StyleRole,
        width: u8,
        link_id: Option<LinkId>,
        source: Option<Span>,
    ) -> Result<Self, ScreenError> {
        let text = text.into();
        if text.is_empty() {
            return Err(ScreenError::EmptyCellText);
        }
        if !matches!(width, 1 | 2) {
            return Err(ScreenError::InvalidCellWidth(width));
        }
        Ok(Self {
            text,
            style,
            width,
            continuation: false,
            link_id,
            source,
        })
    }

    #[must_use]
    pub fn blank() -> Self {
        Self {
            text: " ".to_owned(),
            style: StyleRole::DEFAULT,
            width: 1,
            continuation: false,
            link_id: None,
            source: None,
        }
    }

    fn continuation(style: StyleRole, link_id: Option<LinkId>, source: Option<Span>) -> Self {
        Self {
            text: String::new(),
            style,
            width: 0,
            continuation: true,
            link_id,
            source,
        }
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub fn style(&self) -> StyleRole {
        self.style
    }

    #[must_use]
    pub fn width(&self) -> u8 {
        self.width
    }

    #[must_use]
    pub fn is_continuation(&self) -> bool {
        self.continuation
    }

    #[must_use]
    pub fn link_id(&self) -> Option<LinkId> {
        self.link_id
    }

    #[must_use]
    pub fn source(&self) -> Option<Span> {
        self.source
    }

    #[must_use]
    pub fn same_visual(&self, other: &Self) -> bool {
        self.text == other.text
            && self.style == other.style
            && self.width == other.width
            && self.continuation == other.continuation
            && self.link_id == other.link_id
    }
}

impl Default for Cell {
    fn default() -> Self {
        Self::blank()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentArea {
    pub x: u16,
    pub width: u16,
}

impl ContentArea {
    #[must_use]
    pub const fn new(x: u16, width: u16) -> Self {
        Self { x, width }
    }

    #[must_use]
    pub const fn right(self) -> u16 {
        self.x.saturating_add(self.width)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineSource {
    pub source: Span,
    pub block: Option<BlockId>,
}

impl LineSource {
    #[must_use]
    pub const fn new(source: Span, block: BlockId) -> Self {
        Self {
            source,
            block: Some(block),
        }
    }

    /// A visible provisional source row not yet owned by a committed block.
    #[must_use]
    pub const fn pending(source: Span) -> Self {
        Self {
            source,
            block: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriteOutcome {
    pub cells: usize,
    pub columns: u16,
    pub clipped: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScreenError {
    InvalidGeometry,
    InvalidContentArea,
    InvalidCellWidth(u8),
    EmptyCellText,
    RowOutOfBounds(u16),
    OutsideContent,
    CannotWriteContinuation,
    CombiningOnContinuation,
}

impl fmt::Display for ScreenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidGeometry => {
                formatter.write_str("terminal rows and columns must be non-zero")
            }
            Self::InvalidContentArea => {
                formatter.write_str("content area must fit inside the terminal")
            }
            Self::InvalidCellWidth(width) => {
                write!(formatter, "cell width must be 1 or 2, got {width}")
            }
            Self::EmptyCellText => formatter.write_str("cell text must not be empty"),
            Self::RowOutOfBounds(row) => write!(formatter, "row {row} is outside the screen"),
            Self::OutsideContent => formatter.write_str("cell must stay inside the content area"),
            Self::CannotWriteContinuation => {
                formatter.write_str("wide-cell continuation cannot be written directly")
            }
            Self::CombiningOnContinuation => {
                formatter.write_str("combining text must be appended to a base cell")
            }
        }
    }
}

impl std::error::Error for ScreenError {}

#[derive(Debug, Clone)]
struct Buffer {
    cells: Vec<Cell>,
    line_sources: Vec<Option<LineSource>>,
}

impl Buffer {
    fn blank(rows: u16, cols: u16) -> Self {
        let cells = usize::from(rows) * usize::from(cols);
        Self {
            cells: vec![Cell::blank(); cells],
            line_sources: vec![None; usize::from(rows)],
        }
    }

    fn clear(&mut self) {
        self.cells.fill(Cell::blank());
        self.line_sources.fill(None);
    }
}

#[derive(Debug)]
pub struct Screen {
    rows: u16,
    cols: u16,
    content: ContentArea,
    write_right: u16,
    front: Buffer,
    back: Buffer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenBufferStats {
    pub rows: u16,
    pub cols: u16,
    pub front_cells: usize,
    pub front_capacity_cells: usize,
    pub back_cells: usize,
    pub back_capacity_cells: usize,
    pub line_sources: usize,
    pub line_source_capacity: usize,
}

impl Screen {
    pub fn new(rows: u16, cols: u16, content: ContentArea) -> Result<Self, ScreenError> {
        validate_geometry(rows, cols, content)?;
        Ok(Self::new_with_write_right(
            rows,
            cols,
            content,
            content.right(),
        ))
    }

    pub(crate) fn new_unclipped(
        rows: u16,
        cols: u16,
        layout_width: u16,
    ) -> Result<Self, ScreenError> {
        validate_geometry(rows, cols, ContentArea::new(0, layout_width))?;
        Ok(Self::new_with_write_right(
            rows,
            cols,
            ContentArea::new(0, layout_width),
            cols,
        ))
    }

    fn new_with_write_right(rows: u16, cols: u16, content: ContentArea, write_right: u16) -> Self {
        Self {
            rows,
            cols,
            content,
            write_right,
            front: Buffer::blank(rows, cols),
            back: Buffer::blank(rows, cols),
        }
    }

    pub fn resize(
        &mut self,
        rows: u16,
        cols: u16,
        content: ContentArea,
    ) -> Result<(), ScreenError> {
        validate_geometry(rows, cols, content)?;
        self.rows = rows;
        self.cols = cols;
        self.content = content;
        self.write_right = content.right();
        self.front = Buffer::blank(rows, cols);
        self.back = Buffer::blank(rows, cols);
        Ok(())
    }

    #[must_use]
    pub fn rows(&self) -> u16 {
        self.rows
    }

    #[must_use]
    pub fn cols(&self) -> u16 {
        self.cols
    }

    #[must_use]
    pub fn content_area(&self) -> ContentArea {
        self.content
    }

    #[must_use]
    pub(crate) fn render_width(&self) -> u16 {
        self.write_right.saturating_sub(self.content.x)
    }

    /// Fixed back/front buffer storage for the current terminal geometry.
    #[must_use]
    pub fn buffer_stats(&self) -> ScreenBufferStats {
        ScreenBufferStats {
            rows: self.rows,
            cols: self.cols,
            front_cells: self.front.cells.len(),
            front_capacity_cells: self.front.cells.capacity(),
            back_cells: self.back.cells.len(),
            back_capacity_cells: self.back.cells.capacity(),
            line_sources: self.front.line_sources.len(),
            line_source_capacity: self.front.line_sources.capacity(),
        }
    }

    #[must_use]
    pub fn cell(&self, row: u16, col: u16) -> Option<&Cell> {
        let index = self.index(row, col)?;
        self.back.cells.get(index)
    }

    #[must_use]
    pub fn front_cell(&self, row: u16, col: u16) -> Option<&Cell> {
        let index = self.index(row, col)?;
        self.front.cells.get(index)
    }

    #[must_use]
    pub fn back_row(&self, row: u16) -> Option<&[Cell]> {
        let start = usize::from(row) * usize::from(self.cols);
        let end = start.checked_add(usize::from(self.cols))?;
        (row < self.rows).then(|| &self.back.cells[start..end])
    }

    #[must_use]
    pub fn front_row(&self, row: u16) -> Option<&[Cell]> {
        let start = usize::from(row) * usize::from(self.cols);
        let end = start.checked_add(usize::from(self.cols))?;
        (row < self.rows).then(|| &self.front.cells[start..end])
    }

    #[must_use]
    pub fn line_source(&self, row: u16) -> Option<LineSource> {
        self.back
            .line_sources
            .get(usize::from(row))
            .copied()
            .flatten()
    }

    #[must_use]
    pub fn front_line_source(&self, row: u16) -> Option<LineSource> {
        self.front
            .line_sources
            .get(usize::from(row))
            .copied()
            .flatten()
    }

    pub fn set_line_source(&mut self, row: u16, source: LineSource) -> Result<(), ScreenError> {
        let slot = self
            .back
            .line_sources
            .get_mut(usize::from(row))
            .ok_or(ScreenError::RowOutOfBounds(row))?;
        *slot = Some(source);
        Ok(())
    }

    pub fn clear_back(&mut self) {
        self.back.clear();
    }

    pub fn swap_buffers(&mut self) {
        std::mem::swap(&mut self.front, &mut self.back);
        self.back.clear();
    }

    pub fn write_cell(&mut self, row: u16, col: u16, cell: Cell) -> Result<(), ScreenError> {
        self.require_row(row)?;
        if cell.is_continuation() {
            return Err(ScreenError::CannotWriteContinuation);
        }
        if !self.fits_content(col, cell.width()) {
            return Err(ScreenError::OutsideContent);
        }

        let style = cell.style();
        let link_id = cell.link_id();
        let source = cell.source();
        let width = cell.width();
        let end = col + u16::from(width);
        self.clear_span(row, col, end);

        let index = self.index(row, col).expect("validated screen position");
        self.back.cells[index] = cell;
        if width == 2 {
            self.back.cells[index + 1] = Cell::continuation(style, link_id, source);
        }
        Ok(())
    }

    /// Change only the visual style of a cell. Wide-cell continuations always
    /// receive the same style as their base cell.
    pub fn set_cell_style(
        &mut self,
        row: u16,
        col: u16,
        style: StyleRole,
    ) -> Result<(), ScreenError> {
        self.require_row(row)?;
        let Some(cell) = self.cell(row, col) else {
            return Err(ScreenError::OutsideContent);
        };
        let base_col = if cell.is_continuation() {
            col.checked_sub(1).ok_or(ScreenError::OutsideContent)?
        } else {
            col
        };
        let Some(base) = self.cell(row, base_col) else {
            return Err(ScreenError::OutsideContent);
        };
        if base.is_continuation() {
            return Err(ScreenError::OutsideContent);
        }
        let width = base.width();
        let index = self
            .index(row, base_col)
            .ok_or(ScreenError::OutsideContent)?;
        self.back.cells[index].style = style;
        if width == 2 {
            let continuation = index.checked_add(1).ok_or(ScreenError::OutsideContent)?;
            let cell = self
                .back
                .cells
                .get_mut(continuation)
                .ok_or(ScreenError::OutsideContent)?;
            if cell.is_continuation() {
                cell.style = style;
            }
        }
        Ok(())
    }

    pub fn write_unwrapped(
        &mut self,
        row: u16,
        start_col: u16,
        cells: impl IntoIterator<Item = Cell>,
    ) -> Result<WriteOutcome, ScreenError> {
        self.require_row(row)?;
        if start_col < self.content.x || start_col >= self.write_right {
            return Err(ScreenError::OutsideContent);
        }

        let mut outcome = WriteOutcome {
            cells: 0,
            columns: 0,
            clipped: false,
        };
        let mut col = start_col;
        for cell in cells {
            if cell.is_continuation() {
                return Err(ScreenError::CannotWriteContinuation);
            }
            let width = u16::from(cell.width());
            let end = u32::from(col) + u32::from(width);
            if end > u32::from(self.write_right) {
                outcome.clipped = true;
                break;
            }
            self.write_cell(row, col, cell)?;
            outcome.cells += 1;
            outcome.columns += width;
            col = u16::try_from(end).expect("cell end fits the terminal width");
        }
        Ok(outcome)
    }

    pub fn append_combining(&mut self, row: u16, col: u16, text: &str) -> Result<(), ScreenError> {
        self.require_row(row)?;
        if col < self.content.x || col >= self.write_right {
            return Err(ScreenError::OutsideContent);
        }
        if text.is_empty() {
            return Err(ScreenError::EmptyCellText);
        }
        let index = self.index(row, col).expect("validated screen position");
        let cell = &mut self.back.cells[index];
        if cell.is_continuation() {
            return Err(ScreenError::CombiningOnContinuation);
        }
        cell.text.push_str(text);
        Ok(())
    }

    fn require_row(&self, row: u16) -> Result<(), ScreenError> {
        if row < self.rows {
            Ok(())
        } else {
            Err(ScreenError::RowOutOfBounds(row))
        }
    }

    fn fits_content(&self, col: u16, width: u8) -> bool {
        col >= self.content.x && u32::from(col) + u32::from(width) <= u32::from(self.write_right)
    }

    fn index(&self, row: u16, col: u16) -> Option<usize> {
        (row < self.rows && col < self.cols)
            .then(|| usize::from(row) * usize::from(self.cols) + usize::from(col))
    }

    fn clear_span(&mut self, row: u16, start: u16, end: u16) {
        for col in start..end {
            self.clear_cell_at(row, col);
        }
    }

    fn clear_cell_at(&mut self, row: u16, col: u16) {
        let Some(index) = self.index(row, col) else {
            return;
        };
        let row_start = usize::from(row) * usize::from(self.cols);
        let row_end = row_start + usize::from(self.cols);
        let cell = &self.back.cells[index];
        if cell.width() == 2 {
            self.back.cells[index] = Cell::blank();
            if index + 1 < row_end {
                self.back.cells[index + 1] = Cell::blank();
            }
        } else if cell.is_continuation() {
            self.back.cells[index] = Cell::blank();
            if index > row_start {
                self.back.cells[index - 1] = Cell::blank();
            }
        } else {
            self.back.cells[index] = Cell::blank();
        }
    }
}

fn validate_geometry(rows: u16, cols: u16, content: ContentArea) -> Result<(), ScreenError> {
    if rows == 0 || cols == 0 {
        return Err(ScreenError::InvalidGeometry);
    }
    if u32::from(content.x) + u32::from(content.width) > u32::from(cols) {
        return Err(ScreenError::InvalidContentArea);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn narrow(text: &str) -> Cell {
        Cell::new(text, StyleRole::DEFAULT, 1, None, None).unwrap()
    }

    fn wide(text: &str) -> Cell {
        Cell::new(text, StyleRole::DEFAULT, 2, None, None).unwrap()
    }

    #[test]
    fn style_roles_combine_and_keep_heading_level() {
        let style = StyleRole::heading(3)
            .with(StyleRole::STRONG)
            .with(StyleRole::STRIKE);
        assert!(style.contains(StyleRole::HEADING));
        assert!(style.contains(StyleRole::STRONG));
        assert!(style.contains(StyleRole::STRIKE));
        assert_eq!(style.heading_level(), Some(3));
        assert!(!style.is_default());
        assert!(StyleRole::DEFAULT.is_default());
    }

    #[test]
    fn cjk_and_emoji_write_wide_continuations() {
        let mut screen = Screen::new(2, 6, ContentArea::new(0, 6)).unwrap();
        screen.write_cell(0, 0, wide("你")).unwrap();
        screen.write_cell(0, 2, wide("🙂")).unwrap();

        let cjk = screen.cell(0, 0).unwrap();
        assert_eq!(cjk.text(), "你");
        assert_eq!(cjk.width(), 2);
        assert!(!cjk.is_continuation());
        assert!(screen.cell(0, 1).unwrap().is_continuation());

        let emoji = screen.cell(0, 2).unwrap();
        assert_eq!(emoji.text(), "🙂");
        assert_eq!(emoji.width(), 2);
        assert!(screen.cell(0, 3).unwrap().is_continuation());
    }

    #[test]
    fn combining_marks_append_to_a_base_cell() {
        let mut screen = Screen::new(1, 4, ContentArea::new(0, 4)).unwrap();
        screen.write_cell(0, 0, narrow("e")).unwrap();
        screen.append_combining(0, 0, "\u{301}").unwrap();

        let cell = screen.cell(0, 0).unwrap();
        assert_eq!(cell.text(), "e\u{301}");
        assert_eq!(cell.width(), 1);
        assert!(!screen.cell(0, 1).unwrap().is_continuation());
        assert_eq!(screen.cell(0, 1).unwrap().text(), " ");

        assert_eq!(
            Cell::new("\u{301}", StyleRole::DEFAULT, 0, None, None).unwrap_err(),
            ScreenError::InvalidCellWidth(0)
        );
        screen.write_cell(0, 2, wide("你")).unwrap();
        assert_eq!(
            screen.append_combining(0, 3, "\u{301}").unwrap_err(),
            ScreenError::CombiningOnContinuation
        );
    }

    #[test]
    fn overwriting_wide_cells_clears_both_halves() {
        let mut screen = Screen::new(1, 4, ContentArea::new(0, 4)).unwrap();
        screen.write_cell(0, 0, wide("你")).unwrap();
        screen.write_cell(0, 1, narrow("x")).unwrap();
        assert_eq!(screen.cell(0, 0).unwrap().text(), " ");
        assert_eq!(screen.cell(0, 1).unwrap().text(), "x");
        assert!(!screen.cell(0, 1).unwrap().is_continuation());

        screen.write_cell(0, 0, wide("🙂")).unwrap();
        screen.write_cell(0, 0, narrow("y")).unwrap();
        assert_eq!(screen.cell(0, 0).unwrap().text(), "y");
        assert_eq!(screen.cell(0, 1).unwrap().text(), " ");
        assert!(!screen.cell(0, 1).unwrap().is_continuation());
    }

    #[test]
    fn content_bounds_reject_sidebar_and_terminal_edges() {
        let mut screen = Screen::new(3, 20, ContentArea::new(5, 10)).unwrap();
        assert_eq!(
            screen.write_cell(0, 4, narrow("x")).unwrap_err(),
            ScreenError::OutsideContent
        );
        assert_eq!(
            screen.write_cell(0, 15, narrow("x")).unwrap_err(),
            ScreenError::OutsideContent
        );
        assert_eq!(
            screen.write_cell(0, 14, wide("你")).unwrap_err(),
            ScreenError::OutsideContent
        );
        assert_eq!(
            screen.write_cell(3, 5, narrow("x")).unwrap_err(),
            ScreenError::RowOutOfBounds(3)
        );

        screen.write_cell(0, 5, narrow("x")).unwrap();
        assert_eq!(screen.cell(0, 5).unwrap().text(), "x");
        for col in 0..5 {
            assert_eq!(screen.cell(0, col).unwrap().text(), " ");
        }
        assert_eq!(screen.cell(0, 15).unwrap().text(), " ");
    }

    #[test]
    fn unwrapped_text_clips_at_content_right_and_preserves_source() {
        let mut screen = Screen::new(2, 8, ContentArea::new(2, 4)).unwrap();
        let source = LineSource::new(Span::new(10, 30), BlockId(7));
        screen.set_line_source(0, source).unwrap();
        let cells = (0..10).map(|_| narrow("x"));
        let outcome = screen.write_unwrapped(0, 2, cells).unwrap();

        assert_eq!(
            outcome,
            WriteOutcome {
                cells: 4,
                columns: 4,
                clipped: true
            }
        );
        assert_eq!(screen.line_source(0), Some(source));
        for col in 2..6 {
            assert_eq!(screen.cell(0, col).unwrap().text(), "x");
        }
        for col in [0, 1, 6, 7] {
            assert_eq!(screen.cell(0, col).unwrap().text(), " ");
        }
        assert!(
            screen
                .back_row(1)
                .unwrap()
                .iter()
                .all(|cell| cell.text() == " ")
        );
    }

    #[test]
    fn unclipped_screen_extends_storage_without_changing_layout_width() {
        let mut unclipped = Screen::new_unclipped(1, 10, 4).unwrap();
        assert_eq!(unclipped.content_area(), ContentArea::new(0, 4));
        assert_eq!(unclipped.render_width(), 10);
        unclipped
            .write_cell(0, 9, narrow("x"))
            .expect("unclipped storage accepts the cell");
        assert_eq!(unclipped.cell(0, 9).unwrap().text(), "x");

        let mut clipped = Screen::new(1, 10, ContentArea::new(0, 4)).unwrap();
        assert_eq!(clipped.render_width(), 4);
        assert_eq!(
            clipped.write_cell(0, 4, narrow("x")),
            Err(ScreenError::OutsideContent)
        );
    }

    #[test]
    fn unwrapped_wide_cell_is_clipped_without_splitting() {
        let mut screen = Screen::new(1, 5, ContentArea::new(0, 4)).unwrap();
        let cells = vec![narrow("a"), wide("你"), wide("🙂")];
        let outcome = screen.write_unwrapped(0, 0, cells).unwrap();

        assert_eq!(
            outcome,
            WriteOutcome {
                cells: 2,
                columns: 3,
                clipped: true
            }
        );
        assert_eq!(screen.cell(0, 1).unwrap().text(), "你");
        assert!(screen.cell(0, 2).unwrap().is_continuation());
        for col in 3..5 {
            assert_eq!(screen.cell(0, col).unwrap().text(), " ");
        }
    }

    #[test]
    fn resize_reinitializes_buffers_and_sources() {
        let mut screen = Screen::new(2, 6, ContentArea::new(0, 6)).unwrap();
        screen.write_cell(0, 0, narrow("x")).unwrap();
        screen
            .set_line_source(0, LineSource::new(Span::new(0, 1), BlockId(1)))
            .unwrap();

        screen.resize(3, 8, ContentArea::new(2, 4)).unwrap();
        assert_eq!((screen.rows(), screen.cols()), (3, 8));
        assert_eq!(screen.content_area(), ContentArea::new(2, 4));
        for row in 0..3 {
            assert!(
                screen
                    .back_row(row)
                    .unwrap()
                    .iter()
                    .all(|cell| cell.text() == " ")
            );
            assert!(
                screen
                    .front_row(row)
                    .unwrap()
                    .iter()
                    .all(|cell| cell.text() == " ")
            );
            assert_eq!(screen.line_source(row), None);
            assert_eq!(screen.front_line_source(row), None);
        }
    }

    #[test]
    fn input_options_are_validated() {
        assert_eq!(
            Screen::new(0, 4, ContentArea::new(0, 4)).unwrap_err(),
            ScreenError::InvalidGeometry
        );
        assert_eq!(
            Screen::new(2, 4, ContentArea::new(2, 3)).unwrap_err(),
            ScreenError::InvalidContentArea
        );
        assert_eq!(
            Cell::new("", StyleRole::DEFAULT, 1, None, None).unwrap_err(),
            ScreenError::EmptyCellText
        );
        assert_eq!(
            Cell::new("x", StyleRole::DEFAULT, 3, None, None).unwrap_err(),
            ScreenError::InvalidCellWidth(3)
        );
    }

    #[test]
    fn double_buffer_swap_preserves_front_and_clears_back() {
        let mut screen = Screen::new(1, 4, ContentArea::new(0, 4)).unwrap();
        let source = LineSource::new(Span::new(5, 9), BlockId(2));
        screen.write_cell(0, 0, narrow("x")).unwrap();
        screen.set_line_source(0, source).unwrap();
        assert_eq!(screen.front_cell(0, 0).unwrap().text(), " ");
        assert_eq!(screen.front_line_source(0), None);

        screen.swap_buffers();
        assert_eq!(screen.front_cell(0, 0).unwrap().text(), "x");
        assert_eq!(screen.front_line_source(0), Some(source));
        assert_eq!(screen.cell(0, 0).unwrap().text(), " ");
        assert_eq!(screen.line_source(0), None);
    }

    #[test]
    fn link_style_and_source_metadata_survive_wide_writes() {
        let link = LinkId(4);
        let source = Span::new(20, 24);
        let style = StyleRole::CODE.with(StyleRole::STRONG);
        let mut screen = Screen::new(1, 4, ContentArea::new(0, 4)).unwrap();
        let cell = Cell::new("你", style, 2, Some(link), Some(source)).unwrap();
        screen.write_cell(0, 0, cell).unwrap();

        for col in 0..2 {
            let cell = screen.cell(0, col).unwrap();
            assert_eq!(cell.style(), style);
            assert_eq!(cell.link_id(), Some(link));
            assert_eq!(cell.source(), Some(source));
        }
        assert!(screen.cell(0, 1).unwrap().is_continuation());

        let left = Cell::new("x", style, 1, None, Some(Span::new(0, 1))).unwrap();
        let right = Cell::new("x", style, 1, None, Some(Span::new(1, 2))).unwrap();
        assert_ne!(left, right);
        assert!(left.same_visual(&right));
    }
}
