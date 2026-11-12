use std::fmt;

use mdtui3::{Block, BlockId, BlockKind, Document};

use crate::render::input::DisplayInput;
use crate::render::text::{TextCursor, TextError, fit_prefix, graphemes};
use crate::screen::{LineSource, Screen, ScreenError, StyleRole};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeError {
    NotCode,
    MissingContent,
    RowOverflow,
    Text(TextError),
    Screen(ScreenError),
}

impl fmt::Display for CodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotCode => formatter.write_str("block is not a code block"),
            Self::MissingContent => formatter.write_str("code block source is not a valid range"),
            Self::RowOverflow => formatter.write_str("code block output exceeds terminal rows"),
            Self::Text(error) => error.fmt(formatter),
            Self::Screen(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for CodeError {}

impl From<TextError> for CodeError {
    fn from(error: TextError) -> Self {
        Self::Text(error)
    }
}

impl From<ScreenError> for CodeError {
    fn from(error: ScreenError) -> Self {
        Self::Screen(error)
    }
}

pub fn render_code_block(
    document: &Document,
    block_id: BlockId,
    block: &Block,
    screen: &mut Screen,
    start_row: u16,
) -> Result<u16, CodeError> {
    if !matches!(block.kind, BlockKind::Code { .. }) {
        return Err(CodeError::NotCode);
    }
    if screen.content_area().width == 0 {
        return Ok(0);
    }
    let label = code_label(document, block)?;
    let lines = code_lines(document, block)?;
    render_code_lines(block_id, block, label, lines, screen, start_row)
}

pub(crate) fn render_code_lines(
    block_id: BlockId,
    block: &Block,
    label: Option<String>,
    lines: Option<Vec<CodeLine>>,
    screen: &mut Screen,
    start_row: u16,
) -> Result<u16, CodeError> {
    if label.is_none() && lines.is_none() {
        return Ok(0);
    }
    let content = screen.content_area();
    let width = usize::from(screen.render_width());
    let mut row_offset = 0_u16;
    if let Some(label) = label {
        let fit = fit_prefix(&label, width);
        let mut cursor = TextCursor::new(content.x);
        for grapheme in graphemes(&label[..fit.end]) {
            cursor.place(screen, start_row, grapheme, StyleRole::META, None, None)?;
        }
        screen.set_line_source(start_row, LineSource::new(block.source, block_id))?;
        row_offset = 1;
    }

    let Some(lines) = lines else {
        return Ok(row_offset);
    };
    let count = u16::try_from(lines.len()).map_err(|_| CodeError::RowOverflow)?;
    let prefix_fit = fit_prefix("  ", width);

    for (offset, line) in lines.into_iter().enumerate() {
        let offset = u16::try_from(offset).map_err(|_| CodeError::RowOverflow)?;
        let row = start_row
            .checked_add(row_offset)
            .and_then(|row| row.checked_add(offset))
            .ok_or(CodeError::RowOverflow)?;
        let mut cursor = TextCursor::new(content.x);
        for grapheme in graphemes(&"  "[..prefix_fit.end]) {
            cursor.place(screen, row, grapheme, StyleRole::CODE, None, None)?;
        }
        let remaining = width.saturating_sub(prefix_fit.cells);
        if remaining > 0 {
            let fit = fit_prefix(&line.text, remaining);
            for grapheme in graphemes(&line.text[..fit.end]) {
                let first = line
                    .mappings
                    .partition_point(|(offset, _)| *offset <= grapheme.start)
                    .saturating_sub(1);
                let last = line
                    .mappings
                    .partition_point(|(offset, _)| *offset < grapheme.end)
                    .saturating_sub(1);
                let source = line
                    .mappings
                    .get(first)
                    .zip(line.mappings.get(last))
                    .map(|(first, last)| mdtui3::Span::new(first.1.start, last.1.end));
                cursor.place(screen, row, grapheme, StyleRole::CODE, None, source)?;
            }
        }
        screen.set_line_source(row, LineSource::new(line.source, block_id))?;
    }
    row_offset.checked_add(count).ok_or(CodeError::RowOverflow)
}

pub(crate) fn code_label(document: &Document, block: &Block) -> Result<Option<String>, CodeError> {
    let BlockKind::Code { info } = block.kind else {
        return Err(CodeError::NotCode);
    };
    let Some(info) = info else {
        return Ok(None);
    };
    let text = document.text(info).ok_or(CodeError::MissingContent)?;
    Ok(format_info_label(text))
}

pub(crate) const MAX_INFO_SOURCE_BYTES: usize = 4096;

/// Interactive metadata prefix. A long opener may now retain arbitrarily long
/// info; neither trim nor allocation may inspect that complete source span.
pub(crate) fn code_label_prefix(
    document: &Document,
    block: &Block,
) -> Result<(Option<String>, usize), CodeError> {
    let BlockKind::Code { info } = block.kind else {
        return Err(CodeError::NotCode);
    };
    let Some(info) = info else {
        return Ok((None, 0));
    };
    let text = document.text(info).ok_or(CodeError::MissingContent)?;
    let mut end = text.len().min(MAX_INFO_SOURCE_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let prefix = &text[..end];
    Ok((
        format_info_prefix(prefix, end < text.len()),
        prefix.len().saturating_mul(3),
    ))
}

pub(crate) fn format_info_prefix(prefix: &str, truncated: bool) -> Option<String> {
    let mut label = format_info_label(prefix);
    if truncated {
        label = Some(label.map_or_else(
            || "[…]".to_owned(),
            |mut label| {
                label.pop();
                label.push('…');
                label.push(']');
                label
            },
        ));
    }
    label
}

fn format_info_label(text: &str) -> Option<String> {
    let trimmed = text.trim_matches(|character: char| character.is_ascii_whitespace());
    (!trimmed.is_empty()).then(|| format!("[{trimmed}]"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CodeLine {
    pub text: String,
    pub source: mdtui3::Span,
    pub end: mdtui3::Span,
    mappings: Vec<(usize, mdtui3::Span)>,
}

pub(crate) fn code_lines(
    document: &Document,
    block: &Block,
) -> Result<Option<Vec<CodeLine>>, CodeError> {
    let input =
        DisplayInput::from_document(document, block.source).ok_or(CodeError::MissingContent)?;
    code_lines_from_input(&input, block.source)
}

pub(crate) fn code_lines_from_input(
    input: &DisplayInput,
    source_fallback: mdtui3::Span,
) -> Result<Option<Vec<CodeLine>>, CodeError> {
    let mut lines = Vec::new();
    let mut text = String::new();
    let mut mappings = Vec::new();
    let mut start = None;
    let mut end = None;
    let mut pending_cr = None;
    let mut saw_text = false;

    for run in input.tokens() {
        for (offset, character) in run.text.char_indices() {
            saw_text = true;
            let source = if run.replacement.is_none() && run.source.len() == run.text.len() {
                mdtui3::Span::new(
                    run.source.start + offset,
                    run.source.start + offset + character.len_utf8(),
                )
            } else {
                run.source
            };
            if character == '\r' {
                pending_cr = Some(source);
                continue;
            }
            if character == '\n' {
                let newline = pending_cr
                    .take()
                    .map_or(source, |cr| mdtui3::Span::new(cr.start, source.end));
                lines.push(CodeLine {
                    text: std::mem::take(&mut text),
                    source: start.unwrap_or(newline),
                    end: newline,
                    mappings: std::mem::take(&mut mappings),
                });
                start = None;
                end = None;
                continue;
            }
            pending_cr = None;
            start.get_or_insert(source);
            end = Some(source);
            mappings.push((text.len(), source));
            text.push(character);
        }
    }
    if !text.is_empty() || start.is_some() {
        let source = start.unwrap_or(source_fallback);
        let end = end.unwrap_or(source);
        lines.push(CodeLine {
            text,
            source,
            end,
            mappings,
        });
    }
    if lines.is_empty() && !saw_text {
        Ok(None)
    } else {
        Ok(Some(lines))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen::ContentArea;
    use crate::theme::MONOKAI;

    fn parse(source: &str) -> Document {
        let mut document = Document::new();
        document.append(source).unwrap();
        document.finish();
        document
    }

    fn code_blocks(document: &Document) -> Vec<(BlockId, &Block)> {
        document
            .blocks()
            .iter()
            .enumerate()
            .filter(|(_, block)| matches!(block.kind, BlockKind::Code { .. }))
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

    #[test]
    fn backtick_tilde_and_indented_code_render_identically() {
        let cases = [
            "```\nalpha\n  beta\n```\n",
            "~~~\nalpha\n  beta\n~~~\n",
            "    alpha\n      beta\n",
        ];
        for source in cases {
            let document = parse(source);
            let (id, block) = code_blocks(&document)[0];
            let mut screen = Screen::new(2, 20, ContentArea::new(0, 20)).unwrap();
            assert_eq!(
                render_code_block(&document, id, block, &mut screen, 0),
                Ok(2)
            );
            assert_eq!(row_text(&screen, 0).trim_end(), "  alpha");
            assert_eq!(row_text(&screen, 1).trim_end(), "    beta");
            assert_eq!(screen.cell(0, 0).unwrap().style(), StyleRole::CODE);
            assert_eq!(screen.cell(0, 1).unwrap().style(), StyleRole::CODE);
            let lines = code_lines(&document, block).unwrap().unwrap();
            assert_eq!(
                screen.line_source(0),
                Some(LineSource::new(lines[0].source, id))
            );
            assert_eq!(
                screen.line_source(1),
                Some(LineSource::new(lines[1].source, id))
            );
        }
    }

    #[test]
    fn crlf_is_normalized_and_never_written_as_a_cell() {
        let document = parse("```\r\none\r\n\r\ntwo\r\n```\r\n");
        let (id, block) = code_blocks(&document)[0];
        let mut screen = Screen::new(3, 20, ContentArea::new(0, 20)).unwrap();
        assert_eq!(
            render_code_block(&document, id, block, &mut screen, 0),
            Ok(3)
        );
        assert_eq!(row_text(&screen, 0).trim_end(), "  one");
        assert_eq!(row_text(&screen, 1).trim_end(), "");
        assert_eq!(screen.cell(1, 0).unwrap().style(), StyleRole::CODE);
        assert_eq!(screen.cell(1, 1).unwrap().style(), StyleRole::CODE);
        assert_eq!(row_text(&screen, 2).trim_end(), "  two");
        for row in 0..3 {
            for col in 0..20 {
                assert_ne!(screen.cell(row, col).unwrap().text(), "\r");
            }
        }
    }

    #[test]
    fn trailing_lfs_and_empty_code_lines_follow_code_block_rules() {
        let document = parse("```\nline one\n\nline three\n```\n");
        let (id, block) = code_blocks(&document)[0];
        let mut screen = Screen::new(3, 20, ContentArea::new(0, 20)).unwrap();
        assert_eq!(
            render_code_block(&document, id, block, &mut screen, 0),
            Ok(3)
        );
        assert_eq!(row_text(&screen, 0).trim_end(), "  line one");
        assert_eq!(row_text(&screen, 1).trim_end(), "");
        assert_eq!(screen.cell(1, 0).unwrap().style(), StyleRole::CODE);
        assert_eq!(screen.cell(1, 1).unwrap().style(), StyleRole::CODE);
        assert_eq!(row_text(&screen, 2).trim_end(), "  line three");

        let trailing_blank = parse("```\nline one\n\n```\n");
        let (id, block) = code_blocks(&trailing_blank)[0];
        let mut screen = Screen::new(2, 20, ContentArea::new(0, 20)).unwrap();
        assert_eq!(
            render_code_block(&trailing_blank, id, block, &mut screen, 0),
            Ok(2)
        );
        assert_eq!(row_text(&screen, 1).trim_end(), "");
        assert_eq!(screen.cell(1, 0).unwrap().style(), StyleRole::CODE);
        assert_eq!(screen.cell(1, 1).unwrap().style(), StyleRole::CODE);

        let empty = parse("```\n```\n");
        let (id, block) = code_blocks(&empty)[0];
        let mut screen = Screen::new(1, 20, ContentArea::new(0, 20)).unwrap();
        assert_eq!(render_code_block(&empty, id, block, &mut screen, 0), Ok(0));
        assert!(screen.line_source(0).is_none());

        let info_only = parse("```python\n```\n");
        let (id, block) = code_blocks(&info_only)[0];
        let mut screen = Screen::new(1, 20, ContentArea::new(0, 20)).unwrap();
        assert_eq!(
            render_code_block(&info_only, id, block, &mut screen, 0),
            Ok(1)
        );
        assert_eq!(row_text(&screen, 0).trim_end(), "[python]");
        assert!(screen.line_source(0).is_some());

        let one_blank = parse("```\n\n```\n");
        let (id, block) = code_blocks(&one_blank)[0];
        let mut screen = Screen::new(1, 20, ContentArea::new(0, 20)).unwrap();
        assert_eq!(
            render_code_block(&one_blank, id, block, &mut screen, 0),
            Ok(1)
        );
        assert_eq!(row_text(&screen, 0).trim_end(), "");
        assert_eq!(screen.cell(0, 0).unwrap().style(), StyleRole::CODE);
        assert_eq!(screen.cell(0, 1).unwrap().style(), StyleRole::CODE);
    }

    #[test]
    fn unclosed_fence_renders_to_eof() {
        let document = parse("```\ncode stays code\n");
        let (id, block) = code_blocks(&document)[0];
        let mut screen = Screen::new(1, 30, ContentArea::new(0, 30)).unwrap();
        assert_eq!(
            render_code_block(&document, id, block, &mut screen, 0),
            Ok(1)
        );
        assert_eq!(row_text(&screen, 0).trim_end(), "  code stays code");
    }

    #[test]
    fn long_lines_clip_whole_graphemes_at_widths_one_two_20_80_120() {
        let source = format!("```\n{}\n```\n", "abcdefghijklmnopqrstuvwxyz0123456789");
        let document = parse(&source);
        let (id, block) = code_blocks(&document)[0];
        for width in [1_u16, 2, 20, 80, 120] {
            let mut screen = Screen::new(1, width, ContentArea::new(0, width)).unwrap();
            assert_eq!(
                render_code_block(&document, id, block, &mut screen, 0),
                Ok(1)
            );
            let expected = if width <= 2 {
                " ".repeat(usize::from(width))
            } else {
                let code = "abcdefghijklmnopqrstuvwxyz0123456789";
                format!("  {}", &code[..usize::from(width - 2).min(code.len())])
            };
            assert_eq!(row_text(&screen, 0).trim_end(), expected.trim_end());
            assert_eq!(screen.cell(0, 0).unwrap().style(), StyleRole::CODE);
            if width >= 2 {
                assert_eq!(screen.cell(0, 1).unwrap().style(), StyleRole::CODE);
            }
        }

        let wide = parse("```\nabc中\n```\n");
        let (id, block) = code_blocks(&wide)[0];
        let mut screen = Screen::new(1, 6, ContentArea::new(0, 6)).unwrap();
        render_code_block(&wide, id, block, &mut screen, 0).unwrap();
        assert_eq!(row_text(&screen, 0).trim_end(), "  abc");
        assert_eq!(screen.cell(0, 5).unwrap().text(), " ");

        let mut fits = Screen::new(1, 7, ContentArea::new(0, 7)).unwrap();
        render_code_block(&wide, id, block, &mut fits, 0).unwrap();
        assert_eq!(fits.cell(0, 5).unwrap().text(), "中");
        assert!(fits.cell(0, 6).unwrap().is_continuation());
        assert_eq!(fits.cell(0, 5).unwrap().style(), StyleRole::CODE);
        assert_eq!(fits.cell(0, 6).unwrap().style(), StyleRole::CODE);

        let mut zero = Screen::new(1, 1, ContentArea::new(0, 0)).unwrap();
        assert_eq!(render_code_block(&wide, id, block, &mut zero, 0), Ok(0));
        assert!(zero.line_source(0).is_none());
    }

    #[test]
    fn tabs_internal_spaces_and_html_like_code_are_preserved() {
        let document = parse("```\na  \tb\n<b>raw</b>\n```\n");
        let (id, block) = code_blocks(&document)[0];
        let mut screen = Screen::new(2, 30, ContentArea::new(0, 30)).unwrap();
        assert_eq!(
            render_code_block(&document, id, block, &mut screen, 0),
            Ok(2)
        );
        assert_eq!(row_text(&screen, 0).trim_end(), "  a  \tb");
        assert_eq!(row_text(&screen, 1).trim_end(), "  <b>raw</b>");
        assert_eq!(screen.cell(0, 5).unwrap().text(), "\t");
        assert_eq!(screen.cell(0, 5).unwrap().style(), StyleRole::CODE);
    }

    #[test]
    fn quoted_code_uses_code_prefix_not_quote_prefix() {
        let document = parse("> ```rust\n> quoted code\n> ```\n");
        let (id, block) = code_blocks(&document)[0];
        let mut screen = Screen::new(2, 30, ContentArea::new(0, 30)).unwrap();
        assert_eq!(
            render_code_block(&document, id, block, &mut screen, 0),
            Ok(2)
        );
        assert_eq!(row_text(&screen, 0).trim_end(), "[rust]");
        assert_eq!(row_text(&screen, 1).trim_end(), "  quoted code");
        assert_ne!(screen.cell(0, 0).unwrap().text(), "│");
        assert_eq!(screen.cell(0, 0).unwrap().style(), StyleRole::META);
    }

    #[test]
    fn info_label_uses_full_trimmed_info_and_meta_style() {
        let document = parse("```rust linenos extra=1\ncode\n```\n");
        let (id, block) = code_blocks(&document)[0];
        let mut screen = Screen::new(2, 40, ContentArea::new(0, 40)).unwrap();
        assert_eq!(
            render_code_block(&document, id, block, &mut screen, 0),
            Ok(2)
        );
        assert_eq!(row_text(&screen, 0).trim_end(), "[rust linenos extra=1]");
        assert_eq!(row_text(&screen, 1).trim_end(), "  code");
        for col in 0..22 {
            assert_eq!(screen.cell(0, col).unwrap().style(), StyleRole::META);
        }
        let meta = MONOKAI.resolve(StyleRole::META);
        assert_eq!(meta.background, None);
        assert_ne!(screen.cell(0, 0).unwrap().style(), StyleRole::CODE);
        assert_eq!(
            screen.line_source(0),
            Some(LineSource::new(block.source, id))
        );
        let lines = code_lines(&document, block).unwrap().unwrap();
        assert_eq!(
            screen.line_source(1),
            Some(LineSource::new(lines[0].source, id))
        );

        assert_eq!(
            format_info_label("  rust   linenos  extra  "),
            Some("[rust   linenos  extra]".to_owned())
        );
        assert_eq!(format_info_label(" \t "), None);
    }

    #[test]
    fn absent_and_whitespace_only_info_emit_no_label() {
        for source in ["```\ncode\n```\n", "```   \ncode\n```\n"] {
            let document = parse(source);
            let (id, block) = code_blocks(&document)[0];
            let mut screen = Screen::new(1, 20, ContentArea::new(0, 20)).unwrap();
            assert_eq!(
                render_code_block(&document, id, block, &mut screen, 0),
                Ok(1),
                "{source:?}"
            );
            assert_eq!(row_text(&screen, 0).trim_end(), "  code");
            assert_eq!(screen.cell(0, 0).unwrap().style(), StyleRole::CODE);
        }
    }

    #[test]
    fn info_only_empty_fence_emits_one_label_line() {
        let document = parse("```rust\n```\n");
        let (id, block) = code_blocks(&document)[0];
        let mut screen = Screen::new(1, 20, ContentArea::new(0, 20)).unwrap();
        assert_eq!(
            render_code_block(&document, id, block, &mut screen, 0),
            Ok(1)
        );
        assert_eq!(row_text(&screen, 0).trim_end(), "[rust]");
        assert_eq!(screen.cell(0, 0).unwrap().style(), StyleRole::META);
        assert_eq!(
            screen.line_source(0),
            Some(LineSource::new(block.source, id))
        );
    }

    #[test]
    fn labels_clip_whole_graphemes_at_1_20_80_120() {
        let info = "label-".to_owned() + &"x".repeat(120) + "-中文-end";
        let source = format!("```{info}\n```\n");
        let document = parse(&source);
        let (id, block) = code_blocks(&document)[0];
        let label = format!("[{info}]");
        for width in [1_u16, 20, 80, 120] {
            let mut screen = Screen::new(1, width, ContentArea::new(0, width)).unwrap();
            assert_eq!(
                render_code_block(&document, id, block, &mut screen, 0),
                Ok(1)
            );
            let fit = fit_prefix(&label, usize::from(width));
            assert_eq!(
                row_text(&screen, 0),
                format!("{:<width$}", &label[..fit.end], width = usize::from(width))
            );
            assert_eq!(screen.cell(0, 0).unwrap().text(), "[");
            assert_eq!(screen.cell(0, 0).unwrap().style(), StyleRole::META);
        }

        let wide = parse("```中文\n```\n");
        let (id, block) = code_blocks(&wide)[0];
        let mut screen = Screen::new(1, 4, ContentArea::new(0, 4)).unwrap();
        assert_eq!(render_code_block(&wide, id, block, &mut screen, 0), Ok(1));
        assert_eq!(screen.cell(0, 0).unwrap().text(), "[");
        assert_eq!(screen.cell(0, 1).unwrap().text(), "中");
        assert!(screen.cell(0, 2).unwrap().is_continuation());
        assert_eq!(screen.cell(0, 3).unwrap().text(), " ");

        let mut zero = Screen::new(1, 1, ContentArea::new(0, 0)).unwrap();
        assert_eq!(render_code_block(&wide, id, block, &mut zero, 0), Ok(0));
    }

    #[test]
    fn content_offset_and_non_code_rejection_are_explicit() {
        let document = parse("```\nabcdefgh\n```\n");
        let (id, block) = code_blocks(&document)[0];
        let mut screen = Screen::new(1, 14, ContentArea::new(5, 8)).unwrap();
        assert_eq!(
            render_code_block(&document, id, block, &mut screen, 0),
            Ok(1)
        );
        assert_eq!(screen.cell(0, 5).unwrap().text(), " ");
        assert_eq!(screen.cell(0, 6).unwrap().text(), " ");
        assert_eq!(screen.cell(0, 5).unwrap().style(), StyleRole::CODE);
        assert_eq!(screen.cell(0, 7).unwrap().text(), "a");
        assert_eq!(screen.cell(0, 12).unwrap().text(), "f");
        for col in 0..5 {
            assert_eq!(screen.cell(0, col).unwrap().text(), " ");
        }
        for col in 13..14 {
            assert_eq!(screen.cell(0, col).unwrap().text(), " ");
        }

        let paragraph = parse("plain paragraph\n");
        assert_eq!(
            render_code_block(
                &paragraph,
                BlockId(0),
                &paragraph.blocks()[0],
                &mut screen,
                0
            ),
            Err(CodeError::NotCode)
        );
    }

    #[test]
    fn code_fixture_has_no_overflow_at_all_target_widths() {
        let document = parse(include_str!("../../tests/fixtures/spec/code.md"));
        for width in [1_u16, 2, 20, 80, 120] {
            let mut screen = Screen::new(400, width, ContentArea::new(0, width)).unwrap();
            let mut row = 0_u16;
            for (id, block) in code_blocks(&document) {
                let lines = render_code_block(&document, id, block, &mut screen, row).unwrap();
                assert!(u32::from(row) + u32::from(lines) <= 400, "width {width}");
                let code_lines = code_lines(&document, block).unwrap().unwrap_or_default();
                let has_label = code_label(&document, block).unwrap().is_some();
                for (offset, line) in (row..row + lines).enumerate() {
                    let expected = if has_label && offset == 0 {
                        block.source
                    } else {
                        let index = offset - usize::from(has_label);
                        code_lines[index].source
                    };
                    assert_eq!(
                        screen.line_source(line),
                        Some(LineSource::new(expected, id)),
                        "width {width} row {line}"
                    );
                }
                row += lines;
            }
            assert!(row > 0, "width {width}");
        }
    }
}
