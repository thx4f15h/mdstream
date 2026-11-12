use std::io::{self, Write};
use std::num::NonZeroU16;

use mdtui_tui::screen::Screen;
use mdtui_tui::theme::{MONOKAI, Rgb, Underline};
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalSnapshot {
    rows: Vec<String>,
}

impl TerminalSnapshot {
    #[must_use]
    pub fn has_visible_text(&self) -> bool {
        self.rows.iter().any(|row| !row.trim().is_empty())
    }

    #[must_use]
    pub fn row_text(&self, row: usize) -> Option<&str> {
        self.rows.get(row).map(String::as_str)
    }
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
    pub fn snapshot(&self) -> TerminalSnapshot {
        let (rows, cols) = self.size();
        let rows = (0..rows)
            .map(|row| {
                let mut text = String::new();
                for column in 0..cols {
                    if let Some(cell) = self.cell(row, column) {
                        text.push_str(&cell.text);
                    }
                }
                text.truncate(text.trim_end().len());
                text
            })
            .collect();
        TerminalSnapshot { rows }
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordedWrite {
    Ok(usize),
    Error {
        kind: io::ErrorKind,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteRecord {
    pub frame: usize,
    pub bytes: Vec<u8>,
    pub result: RecordedWrite,
    pub terminal: TerminalSnapshot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameOutcome {
    Committed,
    NoOutput,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameRecord {
    pub start_write: usize,
    pub end_write: usize,
    pub outcome: Option<FrameOutcome>,
}

pub struct Vt100Writer {
    terminal: TerminalScreen,
    writes: Vec<WriteRecord>,
    frames: Vec<FrameRecord>,
    current_frame: Option<usize>,
    fail_on_call: Option<usize>,
    write_calls: usize,
}

impl Vt100Writer {
    #[must_use]
    pub fn new(rows: u16, cols: u16) -> Self {
        Self {
            terminal: TerminalScreen::new(rows, cols),
            writes: Vec::new(),
            frames: Vec::new(),
            current_frame: None,
            fail_on_call: None,
            write_calls: 0,
        }
    }

    pub fn begin_frame(&mut self) -> usize {
        assert!(self.current_frame.is_none(), "previous frame is still open");
        let index = self.frames.len();
        self.frames.push(FrameRecord {
            start_write: self.writes.len(),
            end_write: self.writes.len(),
            outcome: None,
        });
        self.current_frame = Some(index);
        index
    }

    pub fn finish_frame(&mut self, outcome: FrameOutcome) {
        let index = self
            .current_frame
            .take()
            .expect("frame must be started before it is finished");
        let frame = &mut self.frames[index];
        frame.end_write = self.writes.len();
        frame.outcome = Some(outcome);
    }

    /// Fail the one-based write call number before any of its bytes are fed.
    pub fn fail_on_call(&mut self, call: usize) {
        assert!(call > 0);
        self.fail_on_call = Some(call);
    }

    pub fn clear_failure(&mut self) {
        self.fail_on_call = None;
    }

    #[must_use]
    pub fn terminal(&self) -> &TerminalScreen {
        &self.terminal
    }

    #[must_use]
    pub fn writes(&self) -> &[WriteRecord] {
        &self.writes
    }

    #[must_use]
    pub fn frames(&self) -> &[FrameRecord] {
        &self.frames
    }

    #[must_use]
    pub fn frame_writes(&self, frame: usize) -> &[WriteRecord] {
        let record = &self.frames[frame];
        &self.writes[record.start_write..record.end_write]
    }

    #[must_use]
    pub const fn write_calls(&self) -> usize {
        self.write_calls
    }
}

impl Write for Vt100Writer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let frame = self
            .current_frame
            .expect("begin_frame must be called before writing a frame");
        let call = self.write_calls + 1;
        self.write_calls = call;
        if self.fail_on_call == Some(call) {
            let error = io::Error::other("injected vt100 writer failure");
            self.writes.push(WriteRecord {
                frame,
                bytes: bytes.to_vec(),
                result: RecordedWrite::Error {
                    kind: error.kind(),
                    message: error.to_string(),
                },
                terminal: self.terminal.snapshot(),
            });
            return Err(error);
        }
        self.terminal.feed(bytes);
        self.writes.push(WriteRecord {
            frame,
            bytes: bytes.to_vec(),
            result: RecordedWrite::Ok(bytes.len()),
            terminal: self.terminal.snapshot(),
        });
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub fn assert_terminal_matches_screen(terminal: &TerminalScreen, screen: &Screen) {
    let (rows, cols) = terminal.size();
    assert_eq!((rows, cols), (screen.rows(), screen.cols()));
    for row in 0..rows {
        for column in 0..cols {
            let expected = screen
                .front_cell(row, column)
                .expect("committed screen cell must exist");
            let actual = terminal
                .cell(row, column)
                .expect("terminal cell must exist");
            if expected.is_continuation() {
                assert!(
                    actual.text.is_empty() || actual.text == " ",
                    "row {row} column {column}: continuation text mismatch"
                );
            } else if expected.text().trim().is_empty() {
                assert!(
                    actual.text.trim().is_empty(),
                    "row {row} column {column}: expected blank, got {:?}",
                    actual.text
                );
            } else {
                assert_eq!(
                    actual.text,
                    expected.text(),
                    "row {row} column {column}: text mismatch"
                );
            }
            let style = MONOKAI.resolve(expected.style());
            assert_eq!(actual.foreground, color(style.foreground));
            assert_eq!(actual.background, color(style.background));
            assert_eq!(actual.bold, style.bold);
            assert_eq!(actual.italic, style.italic);
            assert_eq!(
                actual.underline,
                style.underline != Underline::None,
                "row {row} column {column}: underline mismatch"
            );
            assert_eq!(actual.wide, expected.width() == 2);
            assert_eq!(actual.wide_continuation, expected.is_continuation());
        }
    }
}

fn color(value: Option<Rgb>) -> Color {
    value.map_or(Color::Default, |value| {
        Color::Rgb(value.red, value.green, value.blue)
    })
}
