use std::fmt;

use mdtui3::{LinkId, Span};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::screen::{Cell, Screen, ScreenError, StyleRole};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphemeKind {
    Narrow,
    Wide,
    ZeroWidth,
    Tab,
    Control,
    Oversize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grapheme<'a> {
    pub text: &'a str,
    pub start: usize,
    pub end: usize,
    pub width: u8,
    pub raw_width: usize,
    pub kind: GraphemeKind,
}

impl Grapheme<'_> {
    #[must_use]
    pub const fn len(self) -> usize {
        self.end - self.start
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextFit {
    pub end: usize,
    pub cells: usize,
    pub clipped: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    Written,
    Combining,
    SkippedControl,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextError {
    NoBaseForZeroWidth,
    Screen(ScreenError),
}

impl fmt::Display for TextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoBaseForZeroWidth => {
                formatter.write_str("zero-width grapheme requires an existing base cell")
            }
            Self::Screen(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for TextError {}

impl From<ScreenError> for TextError {
    fn from(error: ScreenError) -> Self {
        Self::Screen(error)
    }
}

pub struct TextCursor {
    next_col: u16,
    last_base: Option<u16>,
}

impl TextCursor {
    #[must_use]
    pub const fn new(start_col: u16) -> Self {
        Self {
            next_col: start_col,
            last_base: None,
        }
    }

    #[must_use]
    pub const fn next_col(&self) -> u16 {
        self.next_col
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn place_payload(
        &mut self,
        screen: &mut Screen,
        row: u16,
        text: &super::payload::TextPayload,
        width: u8,
        kind: GraphemeKind,
        style: StyleRole,
        link_id: Option<LinkId>,
        source: Option<Span>,
    ) -> Result<Placement, TextError> {
        if kind == GraphemeKind::Control {
            return Ok(Placement::SkippedControl);
        }
        if kind == GraphemeKind::ZeroWidth {
            screen.write_cell(
                row,
                self.next_col,
                Cell::new(text, style, 0, link_id, source)?,
            )?;
            return Ok(Placement::Written);
        }
        let base = self.next_col;
        screen.write_cell(row, base, Cell::new(text, style, width, link_id, source)?)?;
        self.last_base = Some(base);
        self.next_col += u16::from(width);
        Ok(Placement::Written)
    }

    pub fn place(
        &mut self,
        screen: &mut Screen,
        row: u16,
        grapheme: Grapheme<'_>,
        style: StyleRole,
        link_id: Option<LinkId>,
        source: Option<Span>,
    ) -> Result<Placement, TextError> {
        if grapheme.kind == GraphemeKind::Control {
            return Ok(Placement::SkippedControl);
        }
        if grapheme.kind == GraphemeKind::ZeroWidth {
            let base = self.last_base.ok_or(TextError::NoBaseForZeroWidth)?;
            screen.append_combining(row, base, grapheme.text)?;
            return Ok(Placement::Combining);
        }

        let base = self.next_col;
        let cell = Cell::new(grapheme.text, style, grapheme.width, link_id, source)?;
        screen.write_cell(row, base, cell)?;
        self.last_base = Some(base);
        self.next_col = self
            .next_col
            .checked_add(u16::from(grapheme.width))
            .expect("terminal width fits u16");
        Ok(Placement::Written)
    }
}

pub fn graphemes(text: &str) -> impl Iterator<Item = Grapheme<'_>> {
    text.grapheme_indices(true)
        .map(|(start, grapheme)| measure(start, grapheme))
}

#[must_use]
pub fn display_width(text: &str) -> usize {
    graphemes(text)
        .map(|grapheme| usize::from(grapheme.width))
        .sum()
}

#[must_use]
pub fn fit_prefix(text: &str, max_cells: usize) -> TextFit {
    let mut fit = TextFit {
        end: 0,
        cells: 0,
        clipped: false,
    };
    for grapheme in graphemes(text) {
        let cells = usize::from(grapheme.width);
        if fit.cells + cells > max_cells {
            fit.clipped = true;
            return fit;
        }
        fit.end = grapheme.end;
        fit.cells += cells;
    }
    fit.clipped = fit.end < text.len();
    fit
}

fn measure(start: usize, text: &str) -> Grapheme<'_> {
    let end = start + text.len();
    let forced: Option<usize> = match text {
        "•" | "│" | "─" | "═" | "┌" | "┬" | "┐" | "└" | "┴" | "┘" | "╞" | "╪" | "╡" | " "
        | "\u{a0}" | "é" => Some(1),
        "⬜" | "✅" => Some(2),
        _ => None,
    };
    let raw_width = forced.unwrap_or_else(|| UnicodeWidthStr::width(text));

    let kind = if text == "\t" {
        GraphemeKind::Tab
    } else if text.chars().all(char::is_control) {
        GraphemeKind::Control
    } else if raw_width > 2 {
        GraphemeKind::Oversize
    } else {
        match raw_width {
            0 => GraphemeKind::ZeroWidth,
            1 => GraphemeKind::Narrow,
            2 => GraphemeKind::Wide,
            _ => unreachable!(),
        }
    };
    let width = match kind {
        GraphemeKind::Narrow | GraphemeKind::Tab => 1,
        GraphemeKind::Wide | GraphemeKind::Oversize => 2,
        GraphemeKind::ZeroWidth | GraphemeKind::Control => 0,
    };
    let raw_width = if kind == GraphemeKind::Tab {
        1
    } else {
        raw_width
    };

    Grapheme {
        text,
        start,
        end,
        width,
        raw_width,
        kind,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen::ContentArea;

    fn one(text: &str) -> Grapheme<'_> {
        let mut graphemes = graphemes(text);
        let grapheme = graphemes.next().unwrap();
        assert!(graphemes.next().is_none());
        grapheme
    }

    #[test]
    fn reference_width_table_is_explicit() {
        for symbol in [
            "•", "│", "─", "═", "┌", "┬", "┐", "└", "┴", "┘", "╞", "╪", "╡", " ", "\u{a0}", "é",
        ] {
            assert_eq!(one(symbol).width, 1, "{symbol:?}");
        }
        for symbol in ["中", "。", "、", "あ", "한", "🙂", "⬜", "✅"] {
            assert_eq!(one(symbol).width, 2, "{symbol:?}");
        }
        assert_eq!(display_width("•│─═┌┬┐└┴┘╞╪╡é"), 14);
    }

    #[test]
    fn extended_graphemes_keep_combining_and_zwj_sequences_intact() {
        let text = "e\u{301}👨‍👩‍👧‍👦x";
        let graphemes: Vec<_> = graphemes(text).collect();
        assert_eq!(graphemes.len(), 3);
        assert_eq!(graphemes[0].text, "e\u{301}");
        assert_eq!((graphemes[0].start, graphemes[0].end), (0, 3));
        assert_eq!(graphemes[0].width, 1);
        assert_eq!(graphemes[1].text, "👨‍👩‍👧‍👦");
        assert_eq!(graphemes[1].width, 2);
        assert_eq!(graphemes[1].start, 3);
        assert_eq!(graphemes[1].end, 3 + "👨‍👩‍👧‍👦".len());
        assert_eq!(graphemes[2].text, "x");
        assert_eq!(graphemes[2].start, graphemes[1].end);
    }

    #[test]
    fn tab_control_zero_width_and_oversize_have_explicit_policies() {
        let tab = one("\t");
        assert_eq!(
            (tab.kind, tab.width, tab.raw_width),
            (GraphemeKind::Tab, 1, 1)
        );
        let control = one("\u{0}");
        assert_eq!(control.kind, GraphemeKind::Control);
        assert_eq!(control.width, 0);
        let zero = one("\u{301}");
        assert_eq!(zero.kind, GraphemeKind::ZeroWidth);
        assert_eq!(zero.width, 0);
        let oversize = one("\u{17D8}");
        assert_eq!(oversize.kind, GraphemeKind::Oversize);
        assert_eq!(oversize.raw_width, 3);
        assert_eq!(oversize.width, 2);
        assert_eq!(display_width("a\u{0}b"), 2);
    }

    #[test]
    fn prefix_fit_never_splits_a_wide_grapheme() {
        assert_eq!(
            fit_prefix("abc", 0),
            TextFit {
                end: 0,
                cells: 0,
                clipped: true
            }
        );
        assert_eq!(
            fit_prefix("abc", 1),
            TextFit {
                end: 1,
                cells: 1,
                clipped: true
            }
        );
        assert_eq!(
            fit_prefix("中a", 1),
            TextFit {
                end: 0,
                cells: 0,
                clipped: true
            }
        );
        assert_eq!(
            fit_prefix("a中", 1),
            TextFit {
                end: 1,
                cells: 1,
                clipped: true
            }
        );
        assert_eq!(
            fit_prefix("a中", 3),
            TextFit {
                end: 4,
                cells: 3,
                clipped: false
            }
        );
    }

    #[test]
    fn cursor_writes_wide_cells_with_continuation() {
        let mut screen = Screen::new(1, 4, ContentArea::new(0, 4)).unwrap();
        let mut cursor = TextCursor::new(0);
        assert_eq!(
            cursor
                .place(
                    &mut screen,
                    0,
                    one("中"),
                    StyleRole::DEFAULT,
                    None,
                    Some(Span::new(0, 3)),
                )
                .unwrap(),
            Placement::Written
        );
        let first = screen.cell(0, 0).unwrap();
        assert_eq!(first.text(), "中");
        assert_eq!(first.width(), 2);
        assert!(screen.cell(0, 1).unwrap().is_continuation());
        assert_eq!(cursor.next_col(), 2);
    }

    #[test]
    fn content_width_one_rejects_wide_grapheme_without_half_cell() {
        let mut screen = Screen::new(1, 3, ContentArea::new(1, 1)).unwrap();
        let mut cursor = TextCursor::new(1);
        assert_eq!(
            cursor.place(&mut screen, 0, one("中"), StyleRole::DEFAULT, None, None,),
            Err(TextError::Screen(ScreenError::OutsideContent))
        );
        assert_eq!(screen.cell(0, 1).unwrap().text(), " ");
        assert!(!screen.cell(0, 1).unwrap().is_continuation());
        assert_eq!(cursor.next_col(), 1);
    }

    #[test]
    fn zero_width_cluster_requires_and_reuses_a_base_cell() {
        let mut screen = Screen::new(1, 2, ContentArea::new(0, 2)).unwrap();
        let mut leading = TextCursor::new(0);
        assert_eq!(
            leading.place(
                &mut screen,
                0,
                one("\u{301}"),
                StyleRole::DEFAULT,
                None,
                None,
            ),
            Err(TextError::NoBaseForZeroWidth)
        );
        assert_eq!(screen.cell(0, 0).unwrap().text(), " ");

        let mut cursor = TextCursor::new(0);
        cursor
            .place(
                &mut screen,
                0,
                one("e"),
                StyleRole::DEFAULT,
                None,
                Some(Span::new(0, 1)),
            )
            .unwrap();
        assert_eq!(
            cursor
                .place(
                    &mut screen,
                    0,
                    one("\u{301}"),
                    StyleRole::DEFAULT,
                    None,
                    None,
                )
                .unwrap(),
            Placement::Combining
        );
        assert_eq!(screen.cell(0, 0).unwrap().text(), "e\u{301}");
        assert_eq!(screen.cell(0, 0).unwrap().width(), 1);
        assert_eq!(screen.cell(0, 1).unwrap().text(), " ");
        assert_eq!(cursor.next_col(), 1);
    }

    #[test]
    fn controls_are_skipped_without_advancing_the_cursor() {
        let mut screen = Screen::new(1, 2, ContentArea::new(0, 2)).unwrap();
        let mut cursor = TextCursor::new(0);
        assert_eq!(
            cursor
                .place(&mut screen, 0, one("\u{0}"), StyleRole::DEFAULT, None, None,)
                .unwrap(),
            Placement::SkippedControl
        );
        assert_eq!(screen.cell(0, 0).unwrap().text(), " ");
        assert_eq!(cursor.next_col(), 0);
    }
}
