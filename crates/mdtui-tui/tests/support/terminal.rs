use std::num::NonZeroU16;

use vt100::Color;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellView {
    pub text: String,
    pub foreground: Color,
    pub background: Color,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
    pub wide: bool,
    pub wide_continuation: bool,
}

impl CellView {
    pub fn expect_text(&self, expected: &str) -> Result<(), String> {
        let actual = &self.text;
        if actual == expected {
            Ok(())
        } else {
            Err(format!(
                "cell text mismatch: expected {expected:?}, actual {actual:?}"
            ))
        }
    }

    #[must_use]
    pub fn has_visible_data(&self) -> bool {
        !self.text.is_empty()
            || self.foreground != Color::Default
            || self.background != Color::Default
            || self.bold
            || self.dim
            || self.italic
            || self.underline
            || self.inverse
    }

    #[must_use]
    pub fn style_label(&self) -> String {
        let foreground = self.foreground;
        let background = self.background;
        let bold = self.bold;
        let dim = self.dim;
        let italic = self.italic;
        let underline = self.underline;
        let inverse = self.inverse;
        let wide = self.wide;
        let wide_continuation = self.wide_continuation;
        format!(
            "fg={foreground:?} bg={background:?} bold={bold} dim={dim} italic={italic} underline={underline} inverse={inverse} wide={wide} continuation={wide_continuation}"
        )
    }
}

impl From<&vt100::Cell> for CellView {
    fn from(cell: &vt100::Cell) -> Self {
        Self {
            text: cell.contents().to_owned(),
            foreground: cell.fgcolor(),
            background: cell.bgcolor(),
            bold: cell.bold(),
            dim: cell.dim(),
            italic: cell.italic(),
            underline: cell.underline(),
            inverse: cell.inverse(),
            wide: cell.is_wide(),
            wide_continuation: cell.is_wide_continuation(),
        }
    }
}

pub struct TerminalScreen {
    parser: vt100::Parser,
}

impl TerminalScreen {
    #[must_use]
    pub fn new(rows: u16, cols: u16) -> Self {
        Self::with_scrollback(rows, cols, 0)
    }

    #[must_use]
    pub fn with_scrollback(rows: u16, cols: u16, scrollback: usize) -> Self {
        let rows = NonZeroU16::new(rows).expect("terminal rows must be non-zero");
        let cols = NonZeroU16::new(cols).expect("terminal columns must be non-zero");
        Self {
            parser: vt100::Parser::new(rows, cols, scrollback),
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        let rows = NonZeroU16::new(rows).expect("terminal rows must be non-zero");
        let cols = NonZeroU16::new(cols).expect("terminal columns must be non-zero");
        self.parser.set_size(rows, cols);
    }

    #[must_use]
    pub fn size(&self) -> (u16, u16) {
        let (rows, cols) = self.parser.screen().size();
        (rows.get(), cols.get())
    }

    #[must_use]
    pub fn cursor(&self) -> (u16, u16) {
        self.parser.screen().cursor_position()
    }

    #[must_use]
    pub fn cell(&self, row: u16, col: u16) -> Option<CellView> {
        self.parser.screen().cell(row, col).map(CellView::from)
    }

    #[must_use]
    pub fn text(&self) -> String {
        self.parser.screen().contents()
    }

    #[must_use]
    pub fn raw_screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }

    #[must_use]
    pub fn dump(&self) -> String {
        let (rows, cols) = self.size();
        let (cursor_row, cursor_col) = self.cursor();
        let mut output = format!("size={rows}x{cols}\ncursor={cursor_row},{cursor_col}\n");
        for row in 0..rows {
            let mut line = String::new();
            for col in 0..cols {
                if let Some(cell) = self.cell(row, col) {
                    line.push_str(&cell.text);
                }
            }
            let line = line.trim_end();
            output.push_str(&format!("{row:02}|{line}|\n"));
        }
        for row in 0..rows {
            for col in 0..cols {
                if let Some(cell) = self.cell(row, col)
                    && cell.has_visible_data()
                {
                    let text = &cell.text;
                    let style = cell.style_label();
                    output.push_str(&format!("cell=({row},{col}) text={text:?} {style}\n"));
                }
            }
        }
        output
    }
}
