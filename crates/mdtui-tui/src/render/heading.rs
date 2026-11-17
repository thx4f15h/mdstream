use std::fmt;

use mdtui3::{Document, Heading};

use crate::render::input::{DisplayInput, TokenKind};
use crate::render::text::{TextCursor, TextError};
use crate::render::visibility::visible_input;
use crate::screen::{LineSource, Screen, ScreenError, StyleRole};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeadingError {
    InvalidLevel(u8),
    MissingContent,
    Text(TextError),
    Screen(ScreenError),
}

impl fmt::Display for HeadingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLevel(level) => write!(formatter, "invalid heading level {level}"),
            Self::MissingContent => formatter.write_str("heading content is not a valid range"),
            Self::Text(error) => error.fmt(formatter),
            Self::Screen(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for HeadingError {}

impl From<TextError> for HeadingError {
    fn from(error: TextError) -> Self {
        Self::Text(error)
    }
}

impl From<ScreenError> for HeadingError {
    fn from(error: ScreenError) -> Self {
        Self::Screen(error)
    }
}

/// Preserve source and object identity while applying the inherited style's
/// base-then-emphasis order. Images remain visible and clickable in every block.
pub(crate) fn compose_inline_input(input: &DisplayInput) -> DisplayInput {
    compose_input(input, false)
}
fn compose_input(input: &DisplayInput, keep_links: bool) -> DisplayInput {
    let mut base = Vec::new();
    let mut emphasis = Vec::new();
    let mut current: Option<(u8, Vec<crate::render::input::InputToken>)> = None;
    for token in input.tokens() {
        let image = token.style.contains(StyleRole::IMAGE);
        if !keep_links
            && token.link.is_some()
            && !image
            && !token.style.contains(StyleRole::LITERAL_LINK)
        {
            if let Some((_, group)) = current.take() {
                emphasis.push(group);
            }
            continue;
        }
        let mask = emphasis_mask(token.style);
        if token.kind != TokenKind::Text
            || token.style.contains(StyleRole::CODE)
            || image
            || mask == 0
        {
            if let Some((_, group)) = current.take() {
                emphasis.push(group);
            }
            let mut token = token.clone();
            if token.kind != TokenKind::Text {
                token.kind = TokenKind::Text;
                token.text = " ".into();
                token.replacement = Some(token.source);
            }
            base.push(token);
        } else {
            match &mut current {
                Some((old, group)) if *old == mask => group.push(token.clone()),
                _ => {
                    if let Some((_, group)) = current.take() {
                        emphasis.push(group);
                    }
                    current = Some((mask, vec![token.clone()]));
                }
            }
        }
    }
    if let Some((_, group)) = current {
        emphasis.push(group);
    }
    for group in emphasis.into_iter().rev() {
        base.extend(group);
    }
    DisplayInput::from_tokens(base)
}

pub fn compose_inline_text(input: &DisplayInput) -> String {
    compose_inline_input(input).visible_text()
}

pub fn compose_heading_text(input: &DisplayInput) -> String {
    compose_inline_text(input)
}

pub fn render_heading(
    document: &Document,
    heading: &Heading,
    screen: &mut Screen,
    row: u16,
) -> Result<u16, HeadingError> {
    if !(1..=6).contains(&heading.level) {
        return Err(HeadingError::InvalidLevel(heading.level));
    }
    let input = visible_input(document, heading.content).ok_or(HeadingError::MissingContent)?;
    render_heading_input(heading, &input, screen, row)
}

pub(crate) fn render_heading_input(
    heading: &Heading,
    input: &DisplayInput,
    screen: &mut Screen,
    row: u16,
) -> Result<u16, HeadingError> {
    render_composed_heading(heading, &compose_inline_input(input), screen, row)
}
/// Live destinations must remain readable when the inherited presentation
/// would omit every non-whitespace label. Keep the old formatter contract for
/// mixed headings; the fallback preserves visible label source and LinkId.
pub(crate) fn render_live_heading_input(
    heading: &Heading,
    input: &DisplayInput,
    screen: &mut Screen,
    row: u16,
) -> Result<u16, HeadingError> {
    let mut composed = compose_inline_input(input);
    if composed.tokens().iter().all(|t| t.text.is_whitespace())
        && input
            .tokens()
            .iter()
            .any(|t| t.link.is_some() && !t.text.is_whitespace())
    {
        composed = compose_input(input, true);
    }
    render_composed_heading(heading, &composed, screen, row)
}
fn render_composed_heading(
    heading: &Heading,
    input: &DisplayInput,
    screen: &mut Screen,
    row: u16,
) -> Result<u16, HeadingError> {
    let content = screen.content_area();
    let style = StyleRole::heading(heading.level);
    let mut cursor = TextCursor::new(content.x);
    for glyph in
        super::input::display_glyphs(input.tokens().iter().map(|token| super::input::GlyphInput {
            text: &token.text,
            source: token.source,
            replacement: token.replacement,
            block: token.block,
            link: token.link,
            style: token.style,
            kind: token.kind,
        }))
    {
        let available = content
            .x
            .saturating_add(screen.render_width())
            .saturating_sub(cursor.next_col());
        if u16::from(glyph.width) > available {
            break;
        }
        if let Err(error) = cursor.place_payload(
            screen,
            row,
            &glyph.text,
            glyph.width,
            glyph.kind,
            style,
            glyph.link,
            Some(glyph.source),
        ) && error != TextError::NoBaseForZeroWidth
        {
            return Err(error.into());
        }
    }
    screen.set_line_source(row, LineSource::new(heading.source, heading.block))?;
    Ok(1)
}

fn emphasis_mask(style: StyleRole) -> u8 {
    u8::from(style.contains(StyleRole::STRONG))
        | (u8::from(style.contains(StyleRole::EMPHASIS)) << 1)
        | (u8::from(style.contains(StyleRole::STRIKE)) << 2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen::ContentArea;

    fn parse(source: &str) -> Document {
        let mut document = Document::new();
        document.append(source).unwrap();
        document.finish();
        document
    }

    fn heading_text(document: &Document, heading: &Heading) -> String {
        let input = visible_input(document, heading.content).unwrap();
        compose_heading_text(&input)
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
    fn specification_example_merges_base_then_reversed_emphasis_blocks() {
        let document = parse("# **B** *I* `c` [l](u) ~~s~~\n");
        let heading = document.headings().entries()[0];
        assert_eq!(heading_text(&document, &heading), "  c  sIB");

        let mut screen = Screen::new(1, 20, ContentArea::new(0, 20)).unwrap();
        assert_eq!(
            render_heading(&document, &heading, &mut screen, 0).unwrap(),
            1
        );
        assert_eq!(row_text(&screen, 0).trim_end_matches(' '), "  c  sIB");
        assert_eq!(screen.cell(0, 0).unwrap().style(), StyleRole::heading(1));
        assert_eq!(
            screen.line_source(0),
            Some(LineSource::new(heading.source, heading.block))
        );
    }

    #[test]
    fn all_levels_use_uniform_heading_style_without_inline_roles() {
        let source = "# **H1**\n## *H2*\n### ~~H3~~\n#### `H4`\n##### H5\n###### H6\n";
        let document = parse(source);
        for heading in document.headings().entries() {
            let mut screen = Screen::new(1, 20, ContentArea::new(0, 20)).unwrap();
            assert_eq!(
                render_heading(&document, heading, &mut screen, 0).unwrap(),
                1
            );
            let style = StyleRole::heading(heading.level);
            for col in 0..20 {
                let cell = screen.cell(0, col).unwrap();
                if cell.text() != " " {
                    assert_eq!(cell.style(), style);
                    assert!(!cell.style().contains(StyleRole::STRONG));
                    assert!(!cell.style().contains(StyleRole::EMPHASIS));
                    assert!(!cell.style().contains(StyleRole::STRIKE));
                    assert!(!cell.style().contains(StyleRole::CODE));
                }
            }
        }
    }

    #[test]
    fn setext_multiline_text_keeps_images_and_drops_link_text() {
        let document = parse(
            "Setext **one**\nsecond [drop](u) ![drop](i) line &amp;\n================================\n",
        );
        let heading = document.headings().entries()[0];
        let text = heading_text(&document, &heading);
        assert!(text.contains("Setext"));
        assert!(text.contains("second"));
        assert!(text.contains('&'));
        assert!(text.ends_with("one"));
        assert_eq!(text.matches("drop").count(), 1);
        assert!(!text.contains('\n'));

        let mut screen = Screen::new(1, 80, ContentArea::new(0, 80)).unwrap();
        assert_eq!(
            render_heading(&document, &heading, &mut screen, 0).unwrap(),
            1
        );
        assert_eq!(row_text(&screen, 0).trim_end_matches(' '), text);
    }

    #[test]
    fn links_images_entities_and_code_keep_only_base_text_and_surrounding_spaces() {
        let document = parse("# a [x](u) b ![i](i) `a  b` &amp; **z**\n");
        let heading = document.headings().entries()[0];
        assert_eq!(heading_text(&document, &heading), "a  b i a  b & z");
    }

    #[test]
    fn cjk_emoji_entity_and_emphasis_compose_without_byte_splitting() {
        let document = parse("# 中文🙂 &amp; **尾**\n");
        let heading = document.headings().entries()[0];
        let text = heading_text(&document, &heading);
        assert_eq!(text, "中文🙂 & 尾");

        let mut screen = Screen::new(1, 12, ContentArea::new(0, 12)).unwrap();
        render_heading(&document, &heading, &mut screen, 0).unwrap();
        assert_eq!(row_text(&screen, 0).trim_end_matches(' '), "中文🙂 & 尾");
    }

    #[test]
    fn empty_heading_still_occupies_one_line_with_source() {
        let document = parse("# [drop](u)\n");
        let heading = document.headings().entries()[0];
        assert_eq!(heading_text(&document, &heading), "");
        let mut screen = Screen::new(1, 8, ContentArea::new(0, 8)).unwrap();
        assert_eq!(
            render_heading(&document, &heading, &mut screen, 0).unwrap(),
            1
        );
        assert_eq!(row_text(&screen, 0), "        ");
        assert_eq!(
            screen.line_source(0),
            Some(LineSource::new(heading.source, heading.block))
        );
    }

    #[test]
    fn image_only_heading_still_occupies_one_line() {
        let document = parse("# ![drop](image.png)\n");
        let heading = document.headings().entries()[0];
        assert_eq!(heading_text(&document, &heading), "drop");
        let mut screen = Screen::new(1, 8, ContentArea::new(0, 8)).unwrap();
        assert_eq!(
            render_heading(&document, &heading, &mut screen, 0).unwrap(),
            1
        );
        assert_eq!(row_text(&screen, 0), "drop    ");
        assert_eq!(
            screen.line_source(0),
            Some(LineSource::new(heading.source, heading.block))
        );
    }

    #[test]
    fn defined_footnotes_and_inline_html_are_cleaned_in_headings() {
        let document = parse("# a <b>h</b> b[^note] c\n\n[^note]: definition\n");
        let heading = document.headings().entries()[0];
        assert_eq!(heading_text(&document, &heading), "a h b c");
        let mut screen = Screen::new(1, 20, ContentArea::new(0, 20)).unwrap();
        render_heading(&document, &heading, &mut screen, 0).unwrap();
        assert_eq!(row_text(&screen, 0).trim_end(), "a h b c");
        for col in 0..7 {
            assert_eq!(screen.cell(0, col).unwrap().style(), StyleRole::heading(1));
        }

        let undefined = parse("# a[^missing]\n");
        let heading = undefined.headings().entries()[0];
        assert_eq!(heading_text(&undefined, &heading), "a[^missing]");
    }

    #[test]
    fn content_x_offset_keeps_sidebar_columns_untouched() {
        let document = parse("# abcdefgh\n");
        let heading = document.headings().entries()[0];
        let mut screen = Screen::new(1, 12, ContentArea::new(3, 6)).unwrap();
        assert_eq!(
            render_heading(&document, &heading, &mut screen, 0).unwrap(),
            1
        );
        assert_eq!(row_text(&screen, 0), "abcdef");
        for col in 0..3 {
            assert_eq!(screen.cell(0, col).unwrap().text(), " ");
        }
        for col in 9..12 {
            assert_eq!(screen.cell(0, col).unwrap().text(), " ");
        }
    }

    #[test]
    fn overwide_fixture_heading_clips_at_20_80_and_120_cells() {
        let document = parse(include_str!("../../tests/fixtures/spec/overwide.md"));
        let heading = document.headings().entries()[0];
        assert_eq!(heading.level, 1);
        for width in [20, 80, 120] {
            let mut screen = Screen::new(1, width, ContentArea::new(0, width)).unwrap();
            assert_eq!(
                render_heading(&document, &heading, &mut screen, 0).unwrap(),
                1,
                "width {width}"
            );
            assert!(screen.cell(0, width - 1).is_some());
            assert_eq!(
                screen.line_source(0),
                Some(LineSource::new(heading.source, heading.block))
            );
        }
    }
}
