use std::fmt;

use mdtui3::{Block, BlockId, BlockKind, Document};

use crate::render::visibility::visible_input;
use crate::screen::{Cell, LineSource, Screen, ScreenError, StyleRole};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThematicError {
    NotThematicBreak,
    MissingContent,
    Screen(ScreenError),
}

impl fmt::Display for ThematicError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotThematicBreak => formatter.write_str("block is not a thematic break"),
            Self::MissingContent => formatter.write_str("quote source is not a valid range"),
            Self::Screen(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ThematicError {}

impl From<ScreenError> for ThematicError {
    fn from(error: ScreenError) -> Self {
        Self::Screen(error)
    }
}

pub(crate) fn is_thematic_break_text(text: &str) -> bool {
    let mut markers = text
        .chars()
        .filter(|character| !character.is_ascii_whitespace());
    let Some(marker) = markers.next() else {
        return false;
    };
    if !matches!(marker, '-' | '*' | '_') {
        return false;
    }
    let mut count = 1;
    for character in markers {
        if character != marker {
            return false;
        }
        count += 1;
    }
    count >= 3
}

pub(crate) fn is_thematic_break_block(
    document: &Document,
    block: &Block,
) -> Result<bool, ThematicError> {
    match block.kind {
        BlockKind::ThematicBreak => Ok(true),
        BlockKind::Quote { .. } => {
            if block.source.len() > mdtui3::BLOCK_LOOKAHEAD {
                return Ok(false);
            }
            let input =
                visible_input(document, block.source).ok_or(ThematicError::MissingContent)?;
            Ok(is_thematic_break_text(&input.visible_text()))
        }
        _ => Ok(false),
    }
}

pub fn render_thematic_break(
    document: &Document,
    block_id: BlockId,
    block: &Block,
    screen: &mut Screen,
    start_row: u16,
) -> Result<u16, ThematicError> {
    if !is_thematic_break_block(document, block)? {
        return Err(ThematicError::NotThematicBreak);
    }

    let content = screen.content_area();
    if content.width == 0 {
        return Ok(0);
    }
    for offset in 0..content.width {
        let column = content
            .x
            .checked_add(offset)
            .ok_or(ThematicError::Screen(ScreenError::OutsideContent))?;
        screen.write_cell(
            start_row,
            column,
            Cell::new("─", StyleRole::RULE, 1, None, None)?,
        )?;
    }
    screen.set_line_source(start_row, LineSource::new(block.source, block_id))?;
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::paragraph::{ParagraphError, render_quote_paragraph};
    use crate::screen::ContentArea;
    use crate::theme::MONOKAI;

    fn parse(source: &str) -> Document {
        let mut document = Document::new();
        document.append(source).unwrap();
        document.finish();
        document
    }

    fn first_block(document: &Document) -> (BlockId, &Block) {
        (BlockId(0), &document.blocks()[0])
    }

    fn row_text(screen: &Screen, row: u16) -> String {
        let content = screen.content_area();
        let mut text = String::new();
        for col in content.x..content.right() {
            text.push_str(screen.cell(row, col).unwrap().text());
        }
        text
    }

    fn is_quote_break(document: &Document, block: &Block) -> bool {
        block.kind == BlockKind::ThematicBreak
            || (matches!(block.kind, BlockKind::Quote { .. })
                && visible_input(document, block.source)
                    .is_some_and(|input| is_thematic_break_text(&input.visible_text())))
    }

    #[test]
    fn top_level_three_syntaxes_render_the_same_rule() {
        for source in ["---\n", "***\n", "___\n", "- - -\n"] {
            let document = parse(source);
            let (id, block) = first_block(&document);
            let mut screen = Screen::new(1, 5, ContentArea::new(0, 5)).unwrap();
            assert_eq!(
                render_thematic_break(&document, id, block, &mut screen, 0),
                Ok(1),
                "{source:?}"
            );
            assert_eq!(row_text(&screen, 0), "─────");
            for col in 0..5 {
                assert_eq!(screen.cell(0, col).unwrap().style(), StyleRole::RULE);
                assert_ne!(screen.cell(0, col).unwrap().text(), "-");
                assert_ne!(screen.cell(0, col).unwrap().text(), "*");
                assert_ne!(screen.cell(0, col).unwrap().text(), "_");
            }
            assert_eq!(
                screen.line_source(0),
                Some(LineSource::new(block.source, id))
            );
        }
    }

    #[test]
    fn quoted_three_syntaxes_render_without_quote_prefix() {
        for source in ["> ---\n", "> ***\n", "> ___\n"] {
            let document = parse(source);
            let (id, block) = first_block(&document);
            assert!(block.container.is_some(), "{source:?}");
            assert_eq!(block.kind, BlockKind::ThematicBreak);
            assert!(is_quote_break(&document, block), "{source:?}");
            let mut screen = Screen::new(1, 5, ContentArea::new(0, 5)).unwrap();
            assert_eq!(
                render_thematic_break(&document, id, block, &mut screen, 0),
                Ok(1),
                "{source:?}"
            );
            assert_eq!(row_text(&screen, 0), "─────");
            assert_ne!(screen.cell(0, 0).unwrap().text(), "│");
            assert_eq!(screen.cell(0, 0).unwrap().style(), StyleRole::RULE);
            assert_eq!(
                render_quote_paragraph(&document, id, block, &mut screen, 0),
                Err(crate::render::paragraph::ParagraphError::NotQuote),
                "{source:?}"
            );
        }
    }

    #[test]
    fn widths_zero_one_two_and_80_fill_content_area() {
        let document = parse("---\n");
        let (id, block) = first_block(&document);
        let mut zero = Screen::new(1, 1, ContentArea::new(0, 0)).unwrap();
        assert_eq!(
            render_thematic_break(&document, id, block, &mut zero, 0),
            Ok(0)
        );
        assert!(zero.line_source(0).is_none());

        for width in [1_u16, 2, 80] {
            let mut screen = Screen::new(1, width, ContentArea::new(0, width)).unwrap();
            assert_eq!(
                render_thematic_break(&document, id, block, &mut screen, 0),
                Ok(1)
            );
            assert_eq!(row_text(&screen, 0), "─".repeat(usize::from(width)));
            for col in 0..width {
                assert_eq!(screen.cell(0, col).unwrap().style(), StyleRole::RULE);
            }
        }

        let mut offset = Screen::new(1, 14, ContentArea::new(5, 8)).unwrap();
        assert_eq!(
            render_thematic_break(&document, id, block, &mut offset, 0),
            Ok(1)
        );
        for col in 5..13 {
            assert_eq!(offset.cell(0, col).unwrap().text(), "─");
        }
        for col in 0..5 {
            assert_eq!(offset.cell(0, col).unwrap().text(), " ");
        }
        for col in 13..14 {
            assert_eq!(offset.cell(0, col).unwrap().text(), " ");
        }
        assert!(MONOKAI.resolve(StyleRole::RULE).background.is_none());
    }

    #[test]
    fn ordinary_blocks_and_non_break_quotes_are_rejected() {
        for source in ["plain paragraph\n", "> ordinary quote text\n", "> ---x\n"] {
            let document = parse(source);
            let (id, block) = first_block(&document);
            let mut screen = Screen::new(1, 20, ContentArea::new(0, 20)).unwrap();
            assert_eq!(
                render_thematic_break(&document, id, block, &mut screen, 0),
                Err(ThematicError::NotThematicBreak),
                "{source:?}"
            );
            assert!(screen.line_source(0).is_none());
        }
        assert!(is_thematic_break_text("---"));
        assert!(is_thematic_break_text("- - -"));
        assert!(is_thematic_break_text("***"));
        assert!(is_thematic_break_text("___"));
        assert!(!is_thematic_break_text("---x"));

        let paragraph = parse("plain paragraph\n");
        let mut screen = Screen::new(1, 20, ContentArea::new(0, 20)).unwrap();
        assert_eq!(
            render_quote_paragraph(
                &paragraph,
                BlockId(0),
                &paragraph.blocks()[0],
                &mut screen,
                0
            ),
            Err(ParagraphError::NotQuote)
        );
    }

    #[test]
    fn hr_fixture_renders_all_breaks_without_overflow() {
        let document = parse(include_str!("../../tests/fixtures/spec/hr.md"));
        for width in [1_u16, 2, 20, 80] {
            let mut screen = Screen::new(20, width, ContentArea::new(0, width)).unwrap();
            let mut row = 0_u16;
            for (id, block) in document.blocks().iter().enumerate() {
                let id = BlockId(id);
                match render_thematic_break(&document, id, block, &mut screen, row) {
                    Ok(lines) => {
                        assert_eq!(lines, 1);
                        assert_eq!(row_text(&screen, row), "─".repeat(usize::from(width)));
                        assert!(screen.line_source(row).is_some());
                        row += lines;
                    }
                    Err(ThematicError::NotThematicBreak) => {}
                    Err(error) => panic!("unexpected error: {error}"),
                }
            }
            assert_eq!(row, 4, "width {width}");
            assert!(row <= 20);
        }
    }
}
