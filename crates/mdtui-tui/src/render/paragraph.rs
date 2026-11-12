use std::fmt;

use mdtui3::{Block, BlockId, BlockKind, Document};

use crate::render::input::{DisplayInput, GlyphInput, TokenKind, display_glyphs};
use crate::render::text::{GraphemeKind, TextCursor, TextError, fit_prefix, graphemes};
use crate::render::thematic::is_thematic_break_text;
use crate::render::visibility::{is_html_block, visible_input};
use crate::screen::{LineSource, Screen, ScreenError, StyleRole};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParagraphError {
    NotParagraph,
    NotQuote,
    MissingContent,
    RowOverflow,
    Text(TextError),
    Screen(ScreenError),
}

impl fmt::Display for ParagraphError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotParagraph => formatter.write_str("block is not a paragraph"),
            Self::NotQuote => formatter.write_str("block is not a quote paragraph"),
            Self::MissingContent => formatter.write_str("paragraph source is not a valid range"),
            Self::RowOverflow => formatter.write_str("paragraph output exceeds terminal rows"),
            Self::Text(error) => error.fmt(formatter),
            Self::Screen(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ParagraphError {}

impl From<TextError> for ParagraphError {
    fn from(error: TextError) -> Self {
        Self::Text(error)
    }
}

impl From<ScreenError> for ParagraphError {
    fn from(error: ScreenError) -> Self {
        Self::Screen(error)
    }
}

struct StyledGrapheme {
    text: String,
    width: usize,
    kind: GraphemeKind,
    style: StyleRole,
    link: Option<mdtui3::LinkId>,
    source: mdtui3::Span,
    block: Option<BlockId>,
}

pub fn render_paragraph(
    document: &Document,
    block_id: BlockId,
    block: &Block,
    screen: &mut Screen,
    start_row: u16,
) -> Result<u16, ParagraphError> {
    if block.kind != BlockKind::Paragraph {
        return Err(ParagraphError::NotParagraph);
    }
    if is_html_block(document, block) {
        return Ok(0);
    }
    let input = visible_input(document, block.source).ok_or(ParagraphError::MissingContent)?;
    layout_paragraph(&input, block_id, screen, start_row)
}

pub fn render_quote_paragraph(
    document: &Document,
    block_id: BlockId,
    block: &Block,
    screen: &mut Screen,
    start_row: u16,
) -> Result<u16, ParagraphError> {
    if !matches!(block.kind, BlockKind::Quote { .. }) {
        return Err(ParagraphError::NotQuote);
    }
    let Some(input) = quote_paragraph_input(document, block_id, block)? else {
        return Ok(0);
    };
    layout_flow(&input, block_id, screen, start_row, true)
}

fn layout_paragraph(
    input: &DisplayInput,
    block_id: BlockId,
    screen: &mut Screen,
    start_row: u16,
) -> Result<u16, ParagraphError> {
    layout_flow(input, block_id, screen, start_row, false)
}

fn layout_flow(
    input: &DisplayInput,
    block_id: BlockId,
    screen: &mut Screen,
    start_row: u16,
    quote: bool,
) -> Result<u16, ParagraphError> {
    let content = screen.content_area();
    let content_width = usize::from(content.width);
    if content_width == 0 {
        return Ok(0);
    }
    let body_width = if quote {
        content_width.saturating_sub(2)
    } else {
        content_width
    };
    let body_x = if quote {
        if content_width >= 2 {
            content.x + 2
        } else {
            content.right()
        }
    } else {
        content.x
    };

    let mut graphemes = styled_graphemes(input, block_id);
    let Some(first) = graphemes
        .iter()
        .position(|grapheme| !trim_whitespace(grapheme))
    else {
        return Ok(0);
    };
    let last = graphemes
        .iter()
        .rposition(|grapheme| !trim_whitespace(grapheme))
        .map_or(first, |index| index + 1);
    graphemes.truncate(last);
    graphemes.drain(..first);
    if graphemes.is_empty() {
        return Ok(0);
    }

    if body_width == 0 {
        if !quote {
            return Ok(0);
        }
        write_quote_prefix(screen, start_row, content.x, content_width)?;
        let (source, block) = (graphemes[0].source, graphemes[0].block.unwrap_or(block_id));
        screen.set_line_source(start_row, LineSource::new(source, block))?;
        return Ok(1);
    }

    let mut cursor = TextCursor::new(body_x);
    let mut line_width = 0;
    let mut line_source: Option<(mdtui3::Span, BlockId)> = None;
    let mut prefix_written = false;
    let mut lines = 0_u16;

    for grapheme in &graphemes {
        if grapheme.kind == GraphemeKind::Control {
            continue;
        }
        if grapheme.width == 0 {
            if line_source.is_none() {
                continue;
            }
            let row = start_row
                .checked_add(lines)
                .ok_or(ParagraphError::RowOverflow)?;
            if let Err(error) = cursor.place(
                screen,
                row,
                glyph(grapheme),
                grapheme.style,
                grapheme.link,
                Some(grapheme.source),
            ) && error != TextError::NoBaseForZeroWidth
            {
                return Err(error.into());
            }
            continue;
        }
        if grapheme.width > body_width {
            continue;
        }
        if line_source.is_some() && line_width + grapheme.width > body_width {
            finish_line(screen, start_row, lines, line_source.take())?;
            lines = lines.checked_add(1).ok_or(ParagraphError::RowOverflow)?;
            cursor = TextCursor::new(body_x);
            line_width = 0;
            prefix_written = false;
        }

        let row = start_row
            .checked_add(lines)
            .ok_or(ParagraphError::RowOverflow)?;
        if quote && !prefix_written {
            write_quote_prefix(screen, row, content.x, content_width)?;
            prefix_written = true;
        }
        if let Err(error) = cursor.place(
            screen,
            row,
            glyph(grapheme),
            grapheme.style,
            grapheme.link,
            Some(grapheme.source),
        ) {
            return Err(error.into());
        }
        if line_source.is_none() {
            line_source = Some((grapheme.source, grapheme.block.unwrap_or(block_id)));
        }
        line_width += grapheme.width;
    }

    if line_source.is_some() {
        finish_line(screen, start_row, lines, line_source.take())?;
        lines = lines.checked_add(1).ok_or(ParagraphError::RowOverflow)?;
    }
    Ok(lines)
}

fn write_quote_prefix(
    screen: &mut Screen,
    row: u16,
    x: u16,
    width: usize,
) -> Result<(), ParagraphError> {
    let prefix = "│ ";
    let fit = fit_prefix(prefix, width);
    let mut cursor = TextCursor::new(x);
    for grapheme in graphemes(&prefix[..fit.end]) {
        cursor.place(screen, row, grapheme, StyleRole::QUOTE, None, None)?;
    }
    Ok(())
}

enum QuoteBlockState {
    Paragraph(DisplayInput),
    Empty,
    ThematicBreak,
}

fn quote_paragraph_input(
    document: &Document,
    block_id: BlockId,
    block: &Block,
) -> Result<Option<DisplayInput>, ParagraphError> {
    if is_quote_run_continuation(document, block_id, block)? {
        return Ok(None);
    }
    let container = block.container;
    let mut tokens = Vec::new();
    let mut index = block_id.0;
    while let Some(candidate) = document.block(BlockId(index)) {
        if candidate.container != container || !matches!(candidate.kind, BlockKind::Quote { .. }) {
            break;
        }
        match quote_block_state(document, candidate)? {
            QuoteBlockState::Paragraph(input) => tokens.extend(input.into_tokens()),
            QuoteBlockState::Empty | QuoteBlockState::ThematicBreak => break,
        }
        index += 1;
    }
    if tokens.is_empty() {
        Ok(None)
    } else {
        Ok(Some(DisplayInput::from_tokens(tokens)))
    }
}

fn is_quote_run_continuation(
    document: &Document,
    block_id: BlockId,
    block: &Block,
) -> Result<bool, ParagraphError> {
    let Some(previous_id) = block_id.0.checked_sub(1) else {
        return Ok(false);
    };
    let Some(previous) = document.block(BlockId(previous_id)) else {
        return Ok(false);
    };
    if previous.container != block.container || !matches!(previous.kind, BlockKind::Quote { .. }) {
        return Ok(false);
    }
    Ok(matches!(
        quote_block_state(document, previous)?,
        QuoteBlockState::Paragraph(_)
    ))
}

fn quote_block_state(
    document: &Document,
    block: &Block,
) -> Result<QuoteBlockState, ParagraphError> {
    let input = visible_input(document, block.source).ok_or(ParagraphError::MissingContent)?;
    if !input.tokens().iter().any(|token| {
        token.kind == TokenKind::Text
            && !token
                .text
                .chars()
                .all(|character| character.is_ascii_whitespace())
    }) {
        return Ok(QuoteBlockState::Empty);
    }
    if is_thematic_break_text(&input.visible_text()) {
        return Ok(QuoteBlockState::ThematicBreak);
    }
    Ok(QuoteBlockState::Paragraph(input))
}

fn styled_graphemes(input: &DisplayInput, block_id: BlockId) -> Vec<StyledGrapheme> {
    display_glyphs(input.tokens().iter().map(|token| GlyphInput {
        text: &token.text,
        source: token.source,
        replacement: token.replacement,
        block: token.block.or(Some(block_id)),
        link: token.link,
        style: token.style,
        kind: token.kind,
    }))
    .into_iter()
    .map(|glyph| StyledGrapheme {
        text: glyph.text,
        width: usize::from(glyph.width),
        kind: glyph.kind,
        style: glyph.style,
        link: glyph.link,
        source: glyph.source,
        block: glyph.block,
    })
    .collect()
}

fn glyph(grapheme: &StyledGrapheme) -> crate::render::text::Grapheme<'_> {
    crate::render::text::Grapheme {
        text: &grapheme.text,
        start: 0,
        end: grapheme.text.len(),
        width: u8::try_from(grapheme.width).unwrap_or(2),
        raw_width: grapheme.width,
        kind: grapheme.kind,
    }
}

fn trim_whitespace(grapheme: &StyledGrapheme) -> bool {
    grapheme
        .text
        .chars()
        .all(|character| character.is_ascii_whitespace())
}

fn finish_line(
    screen: &mut Screen,
    start_row: u16,
    lines: u16,
    source: Option<(mdtui3::Span, BlockId)>,
) -> Result<(), ParagraphError> {
    let Some((source, block)) = source else {
        return Ok(());
    };
    let row = start_row
        .checked_add(lines)
        .ok_or(ParagraphError::RowOverflow)?;
    screen.set_line_source(row, LineSource::new(source, block))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen::ContentArea;
    use crate::theme::MONOKAI;
    use mdtui3::{DisplayRun, LinkId, Span, Style};

    fn parse(source: &str) -> Document {
        let mut document = Document::new();
        document.append(source).unwrap();
        document.finish();
        document
    }

    fn paragraph(document: &Document) -> (BlockId, &Block) {
        document
            .blocks()
            .iter()
            .enumerate()
            .find(|(_, block)| block.kind == BlockKind::Paragraph)
            .map(|(id, block)| (BlockId(id), block))
            .expect("paragraph block")
    }

    fn quote_blocks(document: &Document) -> Vec<(BlockId, &Block)> {
        document
            .blocks()
            .iter()
            .enumerate()
            .filter(|(_, block)| matches!(block.kind, BlockKind::Quote { .. }))
            .map(|(id, block)| (BlockId(id), block))
            .collect()
    }

    fn row_text(screen: &Screen, row: u16) -> String {
        let content = screen.content_area();
        let mut text = String::new();
        for col in content.x..content.right() {
            text.push_str(screen.cell(row, col).unwrap().text());
        }
        text
    }

    fn text_run(
        input: &mut DisplayInput,
        text: &str,
        style: Style,
        link: Option<LinkId>,
        source: Span,
    ) {
        input.push_run(DisplayRun {
            source,
            replacement: None,
            text,
            block: Some(BlockId(7)),
            style,
            emphasis_depth: 0,
            strong_depth: 0,
            style_pair: None,
            link,
        });
    }

    fn wrap_text(input: &DisplayInput, width: u16, rows: u16) -> (Screen, u16) {
        let mut screen = Screen::new(rows, width, ContentArea::new(0, width)).unwrap();
        let lines = layout_paragraph(input, BlockId(7), &mut screen, 0).unwrap();
        (screen, lines)
    }

    #[test]
    fn soft_break_becomes_space_and_hard_break_disappears() {
        let soft = parse("soft\nline\n");
        let (id, block) = paragraph(&soft);
        let mut screen = Screen::new(2, 20, ContentArea::new(0, 20)).unwrap();
        assert_eq!(render_paragraph(&soft, id, block, &mut screen, 0), Ok(1));
        assert_eq!(row_text(&screen, 0).trim_end(), "soft line");

        let hard = parse("one  \ntwo\n");
        let (id, block) = paragraph(&hard);
        let mut screen = Screen::new(2, 20, ContentArea::new(0, 20)).unwrap();
        assert_eq!(render_paragraph(&hard, id, block, &mut screen, 0), Ok(1));
        assert_eq!(row_text(&screen, 0).trim_end(), "onetwo");
    }

    #[test]
    fn trim_ascii_edges_preserve_internal_spaces_and_nbsp() {
        let mut input = DisplayInput::new();
        text_run(
            &mut input,
            "  a  b  ",
            Style::default(),
            None,
            Span::new(10, 18),
        );
        let (screen, lines) = wrap_text(&input, 20, 2);
        assert_eq!(lines, 1);
        assert_eq!(row_text(&screen, 0).trim_end(), "a  b");
        assert_eq!(
            screen.line_source(0),
            Some(LineSource::new(Span::new(12, 13), BlockId(7)))
        );

        let mut nbsp = DisplayInput::new();
        text_run(
            &mut nbsp,
            " \u{a0}x ",
            Style::default(),
            None,
            Span::new(20, 25),
        );
        let (screen, lines) = wrap_text(&nbsp, 20, 2);
        assert_eq!(lines, 1);
        assert_eq!(row_text(&screen, 0).trim_end(), "\u{a0}x");
    }

    #[test]
    fn greedy_wrap_preserves_spaces_and_can_start_a_line_with_space() {
        let document = parse("abcd efgh\n");
        let (id, block) = paragraph(&document);
        let mut screen = Screen::new(4, 4, ContentArea::new(0, 4)).unwrap();
        assert_eq!(
            render_paragraph(&document, id, block, &mut screen, 0),
            Ok(3)
        );
        assert_eq!(row_text(&screen, 0), "abcd");
        assert_eq!(row_text(&screen, 1), " efg");
        assert_eq!(row_text(&screen, 2).trim_end(), "h");
    }

    #[test]
    fn cjk_and_emoji_wrap_whole_clusters() {
        let document = parse("中🙂a\n");
        let (id, block) = paragraph(&document);
        let mut screen = Screen::new(4, 2, ContentArea::new(0, 2)).unwrap();
        assert_eq!(
            render_paragraph(&document, id, block, &mut screen, 0),
            Ok(3)
        );
        assert_eq!(row_text(&screen, 0), "中");
        assert_eq!(row_text(&screen, 1), "🙂");
        assert_eq!(row_text(&screen, 2).trim_end(), "a");
    }

    #[test]
    fn styles_links_sources_and_blocks_survive_wraps() {
        let mut input = DisplayInput::new();
        text_run(&mut input, "ab", Style::STRONG, None, Span::new(0, 2));
        text_run(
            &mut input,
            "cd",
            Style::default(),
            Some(LinkId(9)),
            Span::new(2, 4),
        );
        let (screen, lines) = wrap_text(&input, 2, 3);
        assert_eq!(lines, 2);
        assert!(
            screen
                .cell(0, 0)
                .unwrap()
                .style()
                .contains(StyleRole::STRONG)
        );
        assert!(
            screen
                .cell(0, 1)
                .unwrap()
                .style()
                .contains(StyleRole::STRONG)
        );
        assert_eq!(screen.cell(1, 0).unwrap().link_id(), Some(LinkId(9)));
        assert_eq!(screen.cell(1, 1).unwrap().link_id(), Some(LinkId(9)));
        assert_eq!(
            screen.line_source(0),
            Some(LineSource::new(Span::new(0, 1), BlockId(7)))
        );
        assert_eq!(
            screen.line_source(1),
            Some(LineSource::new(Span::new(2, 3), BlockId(7)))
        );
    }

    #[test]
    fn width_one_renders_narrow_glyphs_and_skips_wide_glyphs() {
        let document = parse("中a\n");
        let (id, block) = paragraph(&document);
        let mut screen = Screen::new(3, 1, ContentArea::new(0, 1)).unwrap();
        assert_eq!(
            render_paragraph(&document, id, block, &mut screen, 0),
            Ok(1)
        );
        assert_eq!(row_text(&screen, 0), "a");

        let narrow = parse("ab\n");
        let (id, block) = paragraph(&narrow);
        let mut screen = Screen::new(3, 1, ContentArea::new(0, 1)).unwrap();
        assert_eq!(render_paragraph(&narrow, id, block, &mut screen, 0), Ok(2));
        assert_eq!(row_text(&screen, 0), "a");
        assert_eq!(row_text(&screen, 1), "b");
    }

    #[test]
    fn width_zero_and_empty_paragraphs_write_no_rows() {
        let document = parse("ab\n");
        let (id, block) = paragraph(&document);
        let mut screen = Screen::new(2, 2, ContentArea::new(1, 0)).unwrap();
        assert_eq!(
            render_paragraph(&document, id, block, &mut screen, 0),
            Ok(0)
        );

        let mut blank = DisplayInput::new();
        text_run(&mut blank, "   ", Style::default(), None, Span::new(0, 3));
        let (_, lines) = wrap_text(&blank, 10, 2);
        assert_eq!(lines, 0);
        let (_, lines) = wrap_text(&DisplayInput::new(), 10, 2);
        assert_eq!(lines, 0);
    }

    #[test]
    fn content_x_offset_never_writes_sidebar_columns() {
        let document = parse("abcdef\n");
        let (id, block) = paragraph(&document);
        let mut screen = Screen::new(3, 10, ContentArea::new(3, 4)).unwrap();
        assert_eq!(
            render_paragraph(&document, id, block, &mut screen, 0),
            Ok(2)
        );
        assert_eq!(row_text(&screen, 0), "abcd");
        assert_eq!(row_text(&screen, 1).trim_end(), "ef");
        for col in 0..3 {
            assert_eq!(screen.cell(0, col).unwrap().text(), " ");
        }
        for col in 7..10 {
            assert_eq!(screen.cell(0, col).unwrap().text(), " ");
        }
    }

    #[test]
    fn paragraph_fixture_has_no_overflow_at_20_80_120() {
        let document = parse(include_str!("../../tests/fixtures/spec/paragraphs.md"));
        for width in [20, 80, 120] {
            let mut screen = Screen::new(120, width, ContentArea::new(0, width)).unwrap();
            let mut row = 0;
            let mut paragraphs = 0;
            for (id, block) in document
                .blocks()
                .iter()
                .enumerate()
                .map(|(id, block)| (BlockId(id), block))
                .filter(|(_, block)| block.kind == BlockKind::Paragraph)
            {
                let lines = render_paragraph(&document, id, block, &mut screen, row).unwrap();
                assert!(row as u32 + u32::from(lines) <= 120, "width {width}");
                for line in row..row + lines {
                    assert!(
                        screen.line_source(line).is_some(),
                        "width {width} row {line}"
                    );
                }
                row += lines;
                paragraphs += 1;
            }
            assert!(paragraphs > 0);
            assert!(row > 0);
        }
    }

    #[test]
    fn images_are_visible_and_defined_footnotes_are_filtered_before_layout() {
        let document = parse("**a** ![x](y) *b*[^n] c\n\n[^n]: note\n");
        let (id, block) = paragraph(&document);
        let mut screen = Screen::new(2, 20, ContentArea::new(0, 20)).unwrap();
        assert_eq!(
            render_paragraph(&document, id, block, &mut screen, 0),
            Ok(1)
        );
        assert_eq!(row_text(&screen, 0).trim_end(), "a x b c");
        assert_eq!(screen.cell(0, 0).unwrap().style(), StyleRole::STRONG);
        assert_eq!(screen.cell(0, 4).unwrap().style(), StyleRole::EMPHASIS);
        assert_eq!(screen.cell(0, 5).unwrap().style(), StyleRole::DEFAULT);
        for col in 0..7 {
            assert_eq!(
                screen.cell(0, col).unwrap().link_id(),
                (col == 2).then_some(LinkId(0))
            );
        }
        assert!(screen.line_source(0).is_some());
    }

    #[test]
    fn image_only_paragraphs_keep_label_or_placeholder() {
        for source in ["![x](image.png)\n", "![](empty.png)\n"] {
            let document = parse(source);
            let (id, block) = paragraph(&document);
            let mut screen = Screen::new(2, 20, ContentArea::new(0, 20)).unwrap();
            assert_eq!(
                render_paragraph(&document, id, block, &mut screen, 0),
                Ok(1),
                "{source:?}"
            );
            assert_eq!(
                row_text(&screen, 0).trim_end(),
                if source.contains("[x]") {
                    "x"
                } else {
                    "[image]"
                }
            );
            assert_eq!(screen.cell(0, 0).unwrap().link_id(), Some(LinkId(0)));
        }
    }

    #[test]
    fn undefined_footnote_marker_remains_visible() {
        let document = parse("Missing[^none].\n");
        let (id, block) = paragraph(&document);
        let mut screen = Screen::new(2, 30, ContentArea::new(0, 30)).unwrap();
        assert_eq!(
            render_paragraph(&document, id, block, &mut screen, 0),
            Ok(1)
        );
        assert_eq!(row_text(&screen, 0).trim_end(), "Missing[^none].");
    }

    #[test]
    fn html_blocks_return_zero_and_inline_html_is_cleaned() {
        let document = parse(concat!(
            "<div class=\"block\">\n",
            "hidden body\n",
            "</div>\n\n",
            "Inline <b>h</b> and <span class=\"x\">i</span>.\n",
        ));
        let paragraphs: Vec<_> = document
            .blocks()
            .iter()
            .enumerate()
            .filter(|(_, block)| block.kind == BlockKind::Paragraph)
            .map(|(id, block)| (BlockId(id), block))
            .collect();
        let mut screen = Screen::new(4, 40, ContentArea::new(0, 40)).unwrap();
        assert_eq!(
            render_paragraph(&document, paragraphs[0].0, paragraphs[0].1, &mut screen, 0),
            Ok(0)
        );
        assert_eq!(
            render_paragraph(&document, paragraphs[1].0, paragraphs[1].1, &mut screen, 0),
            Ok(1)
        );
        assert_eq!(row_text(&screen, 0).trim_end(), "Inline h and i.");
    }

    #[test]
    fn invisible_fixtures_render_without_overflow_or_marker_leaks() {
        let cases = [
            include_str!("../../tests/fixtures/spec/images.md"),
            include_str!("../../tests/fixtures/spec/footnotes.md"),
            include_str!("../../tests/fixtures/spec/html.md"),
        ];
        for source in cases {
            let document = parse(source);
            let mut screen = Screen::new(200, 80, ContentArea::new(0, 80)).unwrap();
            let mut row = 0;
            for (id, block) in document
                .blocks()
                .iter()
                .enumerate()
                .map(|(id, block)| (BlockId(id), block))
                .filter(|(_, block)| block.kind == BlockKind::Paragraph)
            {
                let lines = render_paragraph(&document, id, block, &mut screen, row).unwrap();
                assert!(row as u32 + u32::from(lines) <= 200);
                for line in row..row + lines {
                    assert!(screen.line_source(line).is_some());
                }
                row += lines;
            }
            let rendered: String = (0..row).map(|line| row_text(&screen, line)).collect();
            assert!(!rendered.contains("<b>"));
            assert!(!rendered.contains("<div"));
            assert!(!rendered.contains("[^1]"));
            assert!(!rendered.contains("comment block should"));
            if source.contains("Paragraph after the HTML block") {
                assert!(rendered.contains("Paragraph after the HTML block."));
            }
        }
    }

    #[test]
    fn quote_prefix_is_single_plain_two_cells_and_body_starts_after_it() {
        let document = parse("> quote text\n");
        let (id, block) = quote_blocks(&document)[0];
        let mut screen = Screen::new(1, 20, ContentArea::new(0, 20)).unwrap();
        assert_eq!(
            render_quote_paragraph(&document, id, block, &mut screen, 0),
            Ok(1)
        );
        assert_eq!(row_text(&screen, 0).trim_end(), "│ quote text");
        assert_eq!(screen.cell(0, 0).unwrap().text(), "│");
        assert_eq!(screen.cell(0, 1).unwrap().text(), " ");
        assert_eq!(screen.cell(0, 2).unwrap().text(), "q");
        assert_eq!(screen.cell(0, 0).unwrap().style(), StyleRole::QUOTE);
        assert_eq!(screen.cell(0, 1).unwrap().style(), StyleRole::QUOTE);
        assert!(MONOKAI.resolve(StyleRole::QUOTE).is_plain());
        assert_eq!(screen.line_source(0).unwrap().block, Some(id));
    }

    #[test]
    fn nested_quote_still_uses_one_prefix() {
        let document = parse("> > nested quote text\n");
        let (id, block) = quote_blocks(&document)[0];
        let mut screen = Screen::new(1, 30, ContentArea::new(0, 30)).unwrap();
        render_quote_paragraph(&document, id, block, &mut screen, 0).unwrap();
        assert_eq!(row_text(&screen, 0).trim_end(), "│ nested quote text");
        assert!(!row_text(&screen, 0).starts_with("│ │"));
    }

    #[test]
    fn wrapped_quote_rewrites_prefix_on_every_line() {
        let document = parse("> abcdefgh\n");
        let (id, block) = quote_blocks(&document)[0];
        let mut screen = Screen::new(2, 8, ContentArea::new(0, 8)).unwrap();
        assert_eq!(
            render_quote_paragraph(&document, id, block, &mut screen, 0),
            Ok(2)
        );
        assert_eq!(row_text(&screen, 0), "│ abcdef");
        assert_eq!(row_text(&screen, 1), "│ gh    ");
        for row in 0..2 {
            assert_eq!(screen.cell(row, 0).unwrap().text(), "│");
            assert_eq!(screen.cell(row, 1).unwrap().text(), " ");
            assert_eq!(screen.cell(row, 0).unwrap().style(), StyleRole::QUOTE);
            assert_eq!(screen.cell(row, 1).unwrap().style(), StyleRole::QUOTE);
            assert!(screen.line_source(row).is_some());
        }
    }

    #[test]
    fn quote_soft_hard_breaks_and_paragraph_separation_follow_paragraph_rules() {
        let soft = parse("> soft one\n> soft two\n");
        let mut screen = Screen::new(1, 40, ContentArea::new(0, 40)).unwrap();
        let mut row = 0;
        for (id, block) in quote_blocks(&soft) {
            row += render_quote_paragraph(&soft, id, block, &mut screen, row).unwrap();
        }
        assert_eq!(row, 1);
        assert_eq!(row_text(&screen, 0).trim_end(), "│ soft one soft two");

        let hard = parse("> hard one  \n> hard two\n");
        let mut screen = Screen::new(1, 40, ContentArea::new(0, 40)).unwrap();
        let mut row = 0;
        for (id, block) in quote_blocks(&hard) {
            row += render_quote_paragraph(&hard, id, block, &mut screen, row).unwrap();
        }
        assert_eq!(row, 1);
        assert_eq!(row_text(&screen, 0).trim_end(), "│ hard onehard two");

        let paragraphs = parse("> one\n>\n> two\n");
        let mut screen = Screen::new(2, 20, ContentArea::new(0, 20)).unwrap();
        let mut row = 0;
        for (id, block) in quote_blocks(&paragraphs) {
            row += render_quote_paragraph(&paragraphs, id, block, &mut screen, row).unwrap();
        }
        assert_eq!(row, 2);
        assert_eq!(row_text(&screen, 0).trim_end(), "│ one");
        assert_eq!(row_text(&screen, 1).trim_end(), "│ two");
    }

    #[test]
    fn image_quote_lines_remain_visible_while_empty_and_defined_footnotes_do_not() {
        let document = parse(">\n> ![](image.png)\n> [^note]\n\n[^note]: body\n");
        let mut screen = Screen::new(3, 20, ContentArea::new(0, 20)).unwrap();
        for (id, block) in quote_blocks(&document) {
            screen.clear_back();
            let image = document
                .links_in(block.source)
                .any(|(_, info)| info.kind == mdtui3::LinkKind::Image);
            assert_eq!(
                render_quote_paragraph(&document, id, block, &mut screen, 0),
                Ok(u16::from(image))
            );
            assert_eq!(
                row_text(&screen, 0).trim(),
                if image { "│ [image]" } else { "" }
            );
            assert_eq!(screen.line_source(0).is_some(), image);
        }

        let mixed = parse("> a ![](image.png) b[^note]\n\n[^note]: body\n");
        let (id, block) = quote_blocks(&mixed)[0];
        let mut screen = Screen::new(1, 20, ContentArea::new(0, 20)).unwrap();
        render_quote_paragraph(&mixed, id, block, &mut screen, 0).unwrap();
        assert_eq!(row_text(&screen, 0).trim_end(), "│ a [image] b");
    }

    #[test]
    fn quote_degenerate_widths_and_content_offset_do_not_overflow() {
        let document = parse("> abc\n");
        let (id, block) = quote_blocks(&document)[0];
        let mut zero = Screen::new(1, 1, ContentArea::new(0, 0)).unwrap();
        assert_eq!(
            render_quote_paragraph(&document, id, block, &mut zero, 0),
            Ok(0)
        );
        assert!(zero.line_source(0).is_none());

        let mut one = Screen::new(1, 1, ContentArea::new(0, 1)).unwrap();
        assert_eq!(
            render_quote_paragraph(&document, id, block, &mut one, 0),
            Ok(1)
        );
        assert_eq!(one.cell(0, 0).unwrap().text(), "│");
        assert!(one.line_source(0).is_some());

        let mut two = Screen::new(1, 2, ContentArea::new(0, 2)).unwrap();
        assert_eq!(
            render_quote_paragraph(&document, id, block, &mut two, 0),
            Ok(1)
        );
        assert_eq!(two.cell(0, 0).unwrap().text(), "│");
        assert_eq!(two.cell(0, 1).unwrap().text(), " ");

        let mut three = Screen::new(3, 3, ContentArea::new(0, 3)).unwrap();
        assert_eq!(
            render_quote_paragraph(&document, id, block, &mut three, 0),
            Ok(3)
        );
        assert_eq!(row_text(&three, 0), "│ a");
        assert_eq!(row_text(&three, 1), "│ b");
        assert_eq!(row_text(&three, 2), "│ c");

        let offset_document = parse("> abcdefgh\n");
        let (id, block) = quote_blocks(&offset_document)[0];
        let mut offset = Screen::new(2, 14, ContentArea::new(5, 8)).unwrap();
        assert_eq!(
            render_quote_paragraph(&offset_document, id, block, &mut offset, 0),
            Ok(2)
        );
        assert_eq!(offset.cell(0, 5).unwrap().text(), "│");
        assert_eq!(offset.cell(0, 6).unwrap().text(), " ");
        assert_eq!(offset.cell(0, 7).unwrap().text(), "a");
        for col in 0..5 {
            assert_eq!(offset.cell(0, col).unwrap().text(), " ");
        }
        for col in 13..14 {
            assert_eq!(offset.cell(0, col).unwrap().text(), " ");
        }
    }

    #[test]
    fn quoted_inner_blocks_do_not_enter_quote_paragraph_path() {
        let document = parse(include_str!("../../tests/fixtures/spec/quotes.md"));
        let mut screen = Screen::new(300, 40, ContentArea::new(0, 40)).unwrap();
        let mut row = 0_u16;
        let mut quote_rows = 0_u16;
        for (id, block) in document.blocks().iter().enumerate() {
            let id = BlockId(id);
            match block.kind {
                BlockKind::Heading { .. }
                | BlockKind::ListItem { .. }
                | BlockKind::Code { .. }
                | BlockKind::Table { .. }
                | BlockKind::ThematicBreak => {
                    assert_eq!(
                        render_quote_paragraph(&document, id, block, &mut screen, row),
                        Err(ParagraphError::NotQuote)
                    );
                }
                BlockKind::Quote { .. } => {
                    let visible = visible_input(&document, block.source)
                        .unwrap()
                        .visible_text();
                    if visible.trim() == "---" {
                        assert_eq!(
                            render_quote_paragraph(&document, id, block, &mut screen, row),
                            Ok(0)
                        );
                        continue;
                    }
                    let lines =
                        render_quote_paragraph(&document, id, block, &mut screen, row).unwrap();
                    for line in row..row + lines {
                        assert_eq!(screen.cell(line, 0).unwrap().text(), "│");
                        assert_eq!(screen.cell(line, 1).unwrap().text(), " ");
                        assert!(screen.line_source(line).is_some());
                    }
                    quote_rows += lines;
                    row += lines;
                }
                _ => {}
            }
        }
        assert!(quote_rows > 0);
        assert!(row < 300);

        let thematic = parse("---\n");
        let block = &thematic.blocks()[0];
        let mut screen = Screen::new(1, 10, ContentArea::new(0, 10)).unwrap();
        assert_eq!(
            render_quote_paragraph(&thematic, BlockId(0), block, &mut screen, 0),
            Err(ParagraphError::NotQuote)
        );
    }
}
