use std::fmt;

use mdtui3::{Block, BlockId, BlockKind, Budget, ContainerKind, Document, Span, WorkLimit};

use crate::render::heading::compose_inline_input;
use crate::render::input::{DisplayInput, InputToken, TokenKind};
use crate::render::text::{
    GraphemeKind, TextCursor, TextError, display_width, fit_prefix, graphemes,
};
use crate::render::visibility::{is_html_block, visible_input};
use crate::screen::{Cell, LineSource, Screen, ScreenError, StyleRole};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListVisual {
    pub marker: String,
    pub marker_style: StyleRole,
    pub body_style: StyleRole,
    pub indent: usize,
    pub marker_width: usize,
    pub marker_indent_width: usize,
    pub body_indent: usize,
    pub continuation_indent: usize,
    pub number: Option<u32>,
    pub task_checked: Option<bool>,
}

impl ListVisual {
    #[must_use]
    pub const fn marker_and_space_width(&self) -> usize {
        self.marker_width + 1
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListRender {
    pub visual: ListVisual,
    pub written_width: usize,
}

impl ListRender {
    #[must_use]
    pub const fn marker_complete(&self) -> bool {
        self.written_width == self.visual.marker_and_space_width()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListError {
    NotListItem,
    MissingContainer,
    InvalidContainer,
    MissingContent,
    Traversal,
    RowOverflow,
    Text(TextError),
    Screen(ScreenError),
}

impl fmt::Display for ListError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotListItem => formatter.write_str("block is not a list item"),
            Self::MissingContainer => formatter.write_str("list item has no container ancestry"),
            Self::InvalidContainer => {
                formatter.write_str("list item container ancestry is invalid")
            }
            Self::MissingContent => formatter.write_str("list item source is not a valid range"),
            Self::Traversal => formatter.write_str("list item ancestry exceeds its budget"),
            Self::RowOverflow => formatter.write_str("list item output exceeds terminal rows"),
            Self::Text(error) => error.fmt(formatter),
            Self::Screen(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ListError {}

impl From<TextError> for ListError {
    fn from(error: TextError) -> Self {
        Self::Text(error)
    }
}

impl From<ScreenError> for ListError {
    fn from(error: ScreenError) -> Self {
        Self::Screen(error)
    }
}

pub fn list_visual(document: &Document, block: &Block) -> Result<Option<ListVisual>, ListError> {
    let input = visible_input(document, block.source).ok_or(ListError::MissingContent)?;
    let body_visible = input
        .tokens()
        .iter()
        .any(|token| token.kind == TokenKind::Text && !token.text.is_ascii_whitespace());
    if !body_visible {
        return Ok(None);
    }
    Ok(Some(build_list_visual(document, block)?))
}

pub(crate) fn build_list_visual(
    document: &Document,
    block: &Block,
) -> Result<ListVisual, ListError> {
    let indent = ancestor_indent(document, block)?;
    build_list_visual_with_indent(block, indent)
}

pub(crate) fn build_list_visual_with_indent(
    block: &Block,
    indent: usize,
) -> Result<ListVisual, ListError> {
    let BlockKind::ListItem { number, task, .. } = block.kind else {
        return Err(ListError::NotListItem);
    };
    let task_checked = task.map(|task| task.checked);
    let marker = match task_checked {
        Some(true) => "✅".to_owned(),
        Some(false) => "⬜".to_owned(),
        None => number.map_or_else(|| "•".to_owned(), |number| format!("{number}.")),
    };
    let marker_width = display_width(&marker);
    let marker_indent_width = if task_checked.is_some() {
        1
    } else {
        marker_width
    };
    let body_indent = indent.saturating_add(marker_width).saturating_add(1);
    let continuation_indent = indent.saturating_add(marker_indent_width).saturating_add(1);

    Ok(ListVisual {
        marker,
        marker_style: if task_checked == Some(true) {
            StyleRole::TASK_CHECKED
        } else {
            StyleRole::LIST_MARKER
        },
        body_style: if task_checked == Some(true) {
            StyleRole::STRIKE
        } else {
            StyleRole::DEFAULT
        },
        indent,
        marker_width,
        marker_indent_width,
        body_indent,
        continuation_indent,
        number,
        task_checked,
    })
}

pub fn render_list_marker(
    document: &Document,
    block: &Block,
    screen: &mut Screen,
    row: u16,
) -> Result<Option<ListRender>, ListError> {
    let Some(visual) = list_visual(document, block)? else {
        return Ok(None);
    };
    Ok(Some(write_list_marker(&visual, screen, row)?))
}

fn write_list_marker(
    visual: &ListVisual,
    screen: &mut Screen,
    row: u16,
) -> Result<ListRender, ListError> {
    let content = screen.content_area();
    let available = usize::from(content.width);
    write_indent(
        screen,
        row,
        content.x,
        visual.indent.min(available),
        visual.marker_style,
    )?;
    let written_width = if visual.indent >= available {
        0
    } else {
        let start = content
            .x
            .saturating_add(u16::try_from(visual.indent).unwrap_or(u16::MAX));
        let marker = format!("{} ", visual.marker);
        let fit = fit_prefix(&marker, available - visual.indent);
        let mut cursor = TextCursor::new(start);
        for grapheme in graphemes(&marker[..fit.end]) {
            cursor.place(screen, row, grapheme, visual.marker_style, None, None)?;
        }
        fit.cells
    };

    Ok(ListRender {
        visual: visual.clone(),
        written_width,
    })
}

pub fn render_list_item(
    document: &Document,
    block_id: BlockId,
    block: &Block,
    screen: &mut Screen,
    start_row: u16,
) -> Result<u16, ListError> {
    if !matches!(block.kind, BlockKind::ListItem { .. }) {
        return Err(ListError::NotListItem);
    }
    if screen.content_area().width == 0 {
        return Ok(0);
    }
    let input = item_input(document, block_id, block)?;
    render_list_input(document, block_id, block, &input, screen, start_row)
}

pub(crate) fn render_list_input(
    document: &Document,
    block_id: BlockId,
    block: &Block,
    input: &DisplayInput,
    screen: &mut Screen,
    start_row: u16,
) -> Result<u16, ListError> {
    let visual = build_list_visual(document, block)?;
    render_list_input_with_visual(block_id, block, input, visual, screen, start_row)
}

pub(crate) fn render_list_input_with_visual(
    block_id: BlockId,
    block: &Block,
    input: &DisplayInput,
    visual: ListVisual,
    screen: &mut Screen,
    start_row: u16,
) -> Result<u16, ListError> {
    let input = compose_inline_input(input);
    let words = words_from_glyphs(input.glyphs());
    if words.is_empty() {
        return Ok(0);
    }
    let marker = write_list_marker(&visual, screen, start_row)?;

    let source = LineSource::new(block.source, block_id);
    screen.set_line_source(start_row, source)?;
    let content = screen.content_area();
    let first_offset = marker
        .visual
        .body_indent
        .max(marker.visual.indent.saturating_add(marker.written_width));
    let first_capacity = usize::from(content.width).saturating_sub(first_offset);
    let first_column = if first_offset < usize::from(content.width) {
        content.x + u16::try_from(first_offset).expect("body indent fits content width")
    } else {
        content.right()
    };
    let mut writer = BodyWriter {
        screen,
        block_id,
        source: block.source,
        start_row,
        line: 0,
        row: start_row,
        cursor: TextCursor::new(first_column),
        content_x: content.x,
        content_width: content.width,
        continuation_indent: marker.visual.continuation_indent,
        body_style: marker.visual.body_style,
        capacity: first_capacity,
        used: 0,
        has_content: false,
        last_class: None,
    };
    writer.place_words(&words)?;
    writer.line.checked_add(1).ok_or(ListError::RowOverflow)
}

pub(crate) fn item_input(
    document: &Document,
    block_id: BlockId,
    block: &Block,
) -> Result<DisplayInput, ListError> {
    let container = block.container.ok_or(ListError::MissingContainer)?;
    let span = document
        .container(container)
        .ok_or(ListError::MissingContainer)?
        .source;
    let blocks = document.blocks();
    let start = blocks.partition_point(|candidate| candidate.source.end <= span.start);
    let end = blocks.partition_point(|candidate| candidate.source.start < span.end);
    let mut tokens = Vec::new();
    for (index, candidate) in blocks[start..end].iter().enumerate() {
        if candidate.container != Some(container)
            || (index + start != block_id.0 && candidate.kind != BlockKind::Paragraph)
            || is_html_block(document, candidate)
        {
            continue;
        }
        let input = visible_input(document, candidate.source).ok_or(ListError::MissingContent)?;
        if !tokens.is_empty() {
            tokens.push(InputToken {
                kind: TokenKind::Text,
                text: " ".into(),
                source: Span::new(candidate.source.start, candidate.source.start),
                replacement: None,
                block: Some(block_id),
                link: None,
                style: StyleRole::DEFAULT,
            });
        }
        tokens.extend(input.into_tokens());
    }
    Ok(DisplayInput::from_tokens(tokens))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListWordClass {
    Cjk,
    Punctuation,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ListWord {
    glyphs: Vec<super::input::DisplayGlyph>,
    width: usize,
    class: ListWordClass,
    separated: bool,
    preserved_boundary: bool,
}
fn words_from_glyphs(glyphs: Vec<super::input::DisplayGlyph>) -> Vec<ListWord> {
    let mut words = Vec::<ListWord>::new();
    let mut current = Vec::new();
    let mut separated = false;
    let mut pending_space = false;
    let mut prior_preserved = false;
    let mut current_preserved = false;
    let finish = |words: &mut Vec<ListWord>,
                  current: &mut Vec<super::input::DisplayGlyph>,
                  separated: bool,
                  preserved_boundary: bool| {
        if !current.is_empty() {
            let glyphs = std::mem::take(current);
            let width = glyphs.iter().map(|g| usize::from(g.width)).sum();
            words.push(ListWord {
                glyphs,
                width,
                class: ListWordClass::Other,
                separated,
                preserved_boundary,
            });
        }
    };
    for glyph in glyphs {
        if glyph.kind == GraphemeKind::Control {
            continue;
        }
        let preserved =
            glyph.style.contains(StyleRole::IMAGE) || glyph.style.contains(StyleRole::LITERAL_LINK);
        let boundary = preserved || prior_preserved;
        prior_preserved = preserved;
        if glyph.text.is_whitespace() {
            finish(&mut words, &mut current, separated, current_preserved);
            separated = false;
            if !words.is_empty() {
                pending_space = true;
            }
            continue;
        }
        if glyph.width == 0 {
            if current.is_empty() {
                if let Some(last) = words.last_mut() {
                    last.glyphs.push(glyph)
                } else {
                    current.push(glyph)
                }
            } else {
                current.push(glyph)
            }
            continue;
        }
        let first = glyph.text.first_char().unwrap();
        let class = if is_cjk(first) {
            ListWordClass::Cjk
        } else if is_punctuation(first) {
            ListWordClass::Punctuation
        } else {
            ListWordClass::Other
        };
        if class == ListWordClass::Other {
            if current.is_empty() {
                separated = pending_space && !words.is_empty();
                current_preserved = boundary;
            }
            current.push(glyph);
            pending_space = false;
        } else {
            finish(&mut words, &mut current, separated, current_preserved);
            separated = false;
            let width = usize::from(glyph.width);
            words.push(ListWord {
                glyphs: vec![glyph],
                width,
                class,
                separated: pending_space && !words.is_empty(),
                preserved_boundary: boundary,
            });
            pending_space = false;
        }
    }
    finish(&mut words, &mut current, separated, current_preserved);
    words
}
#[cfg(test)]
fn list_words(text: &str) -> Vec<ListWord> {
    let input = DisplayInput::from_tokens(vec![InputToken {
        kind: TokenKind::Text,
        text: text.into(),
        source: Span::new(0, text.len()),
        replacement: None,
        block: None,
        link: None,
        style: StyleRole::DEFAULT,
    }]);
    words_from_glyphs(input.glyphs())
}

fn is_cjk(character: char) -> bool {
    matches!(
        character as u32,
        0x1100..=0x11FF
            | 0x3040..=0x30FF
            | 0x3100..=0x312F
            | 0x3130..=0x318F
            | 0x31F0..=0x31FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xA960..=0xA97F
            | 0xAC00..=0xD7AF
            | 0xD7B0..=0xD7FF
            | 0xF900..=0xFAFF
            | 0xFF66..=0xFF9D
            | 0x20000..=0x2FA1F
    )
}

fn is_punctuation(character: char) -> bool {
    character.is_ascii_punctuation()
        || matches!(
            character as u32,
            0x2000..=0x206F
                | 0x2E00..=0x2E7F
                | 0x3000..=0x303F
                | 0xFE10..=0xFE1F
                | 0xFE30..=0xFE4F
                | 0xFE50..=0xFE6F
                | 0xFF01..=0xFF0F
                | 0xFF1A..=0xFF20
                | 0xFF3B..=0xFF40
                | 0xFF5B..=0xFF65
                | 0xFFE0..=0xFFE6
        )
}

fn needs_separator(previous: ListWordClass, current: ListWordClass) -> bool {
    !matches!(
        (previous, current),
        (ListWordClass::Cjk, ListWordClass::Cjk)
            | (ListWordClass::Cjk, ListWordClass::Punctuation)
            | (ListWordClass::Punctuation, ListWordClass::Cjk)
            | (ListWordClass::Punctuation, ListWordClass::Punctuation)
    )
}

struct BodyWriter<'a> {
    screen: &'a mut Screen,
    block_id: BlockId,
    source: Span,
    start_row: u16,
    line: u16,
    row: u16,
    cursor: TextCursor,
    content_x: u16,
    content_width: u16,
    continuation_indent: usize,
    body_style: StyleRole,
    capacity: usize,
    used: usize,
    has_content: bool,
    last_class: Option<ListWordClass>,
}

impl BodyWriter<'_> {
    fn place_words(&mut self, words: &[ListWord]) -> Result<(), ListError> {
        for word in words {
            self.place_word(word)?;
        }
        Ok(())
    }

    fn place_word(&mut self, word: &ListWord) -> Result<(), ListError> {
        if word.width == 0 {
            self.write_glyphs(&word.glyphs, false)?;
            self.has_content = true;
            self.last_class = Some(word.class);
            return Ok(());
        }
        let separator = self.has_content
            && (word.separated
                || !word.preserved_boundary
                    && self
                        .last_class
                        .is_some_and(|last| needs_separator(last, word.class)));
        if word.width + usize::from(separator) <= self.capacity.saturating_sub(self.used) {
            if separator {
                let space = graphemes(" ").next().unwrap();
                self.cursor
                    .place(self.screen, self.row, space, self.body_style, None, None)?;
            }
            self.write_glyphs(&word.glyphs, false)?;
            self.used += word.width + usize::from(separator);
            self.has_content = true;
            self.last_class = Some(word.class);
            return Ok(());
        }
        if self.has_content {
            if !self.start_continuation()? {
                return Ok(());
            }
        } else if self.capacity == 0 && (self.line != 0 || !self.start_continuation()?) {
            return Ok(());
        }
        if word.width <= self.capacity {
            self.write_glyphs(&word.glyphs, false)?;
            self.used += word.width;
            self.has_content = true;
        } else {
            self.write_glyphs(&word.glyphs, true)?;
        }
        self.last_class = Some(word.class);
        Ok(())
    }
    fn write_glyphs(
        &mut self,
        glyphs: &[super::input::DisplayGlyph],
        hard: bool,
    ) -> Result<(), ListError> {
        for glyph in glyphs {
            let width = usize::from(glyph.width);
            if hard && width > 0 && self.used + width > self.capacity {
                if self.has_content && !self.start_continuation()? {
                    return Ok(());
                }
                if self.capacity == 0 {
                    return Ok(());
                }
                if width > self.capacity {
                    continue;
                }
            }
            if let Err(error) = self.cursor.place_payload(
                self.screen,
                self.row,
                &glyph.text,
                glyph.width,
                glyph.kind,
                self.body_style,
                glyph.link,
                Some(glyph.source),
            ) && error != TextError::NoBaseForZeroWidth
            {
                return Err(error.into());
            }
            if hard {
                self.used += width;
                self.has_content |= width > 0;
            }
        }
        Ok(())
    }

    fn start_continuation(&mut self) -> Result<bool, ListError> {
        let capacity = usize::from(self.content_width).saturating_sub(self.continuation_indent);
        if capacity == 0 {
            return Ok(false);
        }
        let line = self.line.checked_add(1).ok_or(ListError::RowOverflow)?;
        let row = self
            .start_row
            .checked_add(line)
            .ok_or(ListError::RowOverflow)?;
        let indent = self
            .continuation_indent
            .min(usize::from(self.content_width));
        write_indent(self.screen, row, self.content_x, indent, self.body_style)?;
        let column = self.content_x
            + u16::try_from(self.continuation_indent)
                .expect("continuation indent fits content width");
        self.screen
            .set_line_source(row, LineSource::new(self.source, self.block_id))?;
        self.line = line;
        self.row = row;
        self.cursor = TextCursor::new(column);
        self.capacity = capacity;
        self.used = 0;
        self.has_content = false;
        self.last_class = None;
        Ok(true)
    }
}

fn write_indent(
    screen: &mut Screen,
    row: u16,
    start: u16,
    count: usize,
    style: StyleRole,
) -> Result<(), ListError> {
    for offset in 0..count {
        let column = start
            .checked_add(u16::try_from(offset).map_err(|_| ListError::RowOverflow)?)
            .ok_or(ListError::RowOverflow)?;
        screen.write_cell(row, column, Cell::new(" ", style, 1, None, None)?)?;
    }
    Ok(())
}

fn ancestor_indent(document: &Document, block: &Block) -> Result<usize, ListError> {
    let container = block.container.ok_or(ListError::MissingContainer)?;
    let bound = block.source.len().saturating_add(2);
    let mut budget = Budget::new(WorkLimit {
        queries: bound,
        objects: bound,
        ..WorkLimit::default()
    });
    let mut ancestors = document
        .container_ancestors(container)
        .ok_or(ListError::MissingContainer)?;
    let mut first = true;
    let mut indent = 0_usize;
    while let Some((_, container)) = ancestors
        .advance(&mut budget)
        .map_err(|_| ListError::Traversal)?
    {
        if first {
            if !matches!(container.kind, ContainerKind::Item { .. }) {
                return Err(ListError::InvalidContainer);
            }
            first = false;
            continue;
        }
        if let ContainerKind::Item { number, task } = container.kind {
            let width = if task.is_some() {
                1
            } else {
                number.map_or(1, |number| decimal_digits(number) + 1)
            };
            indent = indent.saturating_add(width).saturating_add(1);
        }
    }
    if first {
        Err(ListError::InvalidContainer)
    } else {
        Ok(indent)
    }
}

fn decimal_digits(number: u32) -> usize {
    if number == 0 {
        1
    } else {
        number.ilog10() as usize + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen::{Cell, ContentArea};

    fn parse(source: &str) -> Document {
        let mut document = Document::new();
        document.append(source).unwrap();
        document.finish();
        document
    }

    fn list_items(document: &Document) -> Vec<&Block> {
        document
            .blocks()
            .iter()
            .filter(|block| matches!(block.kind, BlockKind::ListItem { .. }))
            .collect()
    }

    fn list_item_blocks(document: &Document) -> Vec<(BlockId, &Block)> {
        document
            .blocks()
            .iter()
            .enumerate()
            .filter(|(_, block)| matches!(block.kind, BlockKind::ListItem { .. }))
            .map(|(id, block)| (BlockId(id), block))
            .collect()
    }

    fn row_text(screen: &Screen, row: u16) -> String {
        let content = screen.content_area();
        let mut text = String::new();
        for col in content.x..content.right() {
            text.push_str(&screen.cell(row, col).unwrap().text().collect_text());
        }
        text
    }

    fn visuals(document: &Document) -> Vec<ListVisual> {
        list_items(document)
            .into_iter()
            .map(|block| {
                list_visual(document, block)
                    .unwrap()
                    .expect("visible list item")
            })
            .collect()
    }

    #[test]
    fn nested_bullet_ordered_and_task_formulas_match_reference() {
        let document = parse(concat!(
            "- bullet parent\n",
            "  - child bullet\n",
            "    1. child ordered\n",
            "       1. deep ordered\n",
            "          - [ ] deep task\n",
        ));
        let visuals = visuals(&document);
        assert_eq!(visuals.len(), 5);
        let expected = [
            ("•", 0, 1, 1, 2, 2),
            ("•", 2, 1, 1, 4, 4),
            ("1.", 4, 2, 2, 7, 7),
            ("1.", 7, 2, 2, 10, 10),
            ("⬜", 10, 2, 1, 13, 12),
        ];
        for (visual, (marker, indent, width, indent_width, body, continuation)) in
            visuals.iter().zip(expected)
        {
            assert_eq!(visual.marker, marker);
            assert_eq!(visual.indent, indent);
            assert_eq!(visual.marker_width, width);
            assert_eq!(visual.marker_indent_width, indent_width);
            assert_eq!(visual.body_indent, body);
            assert_eq!(visual.continuation_indent, continuation);
        }
    }

    #[test]
    fn ordered_markers_preserve_source_start_numbers() {
        for (number, expected_marker, expected_width) in
            [(0_u32, "0.", 2), (10, "10.", 3), (99, "99.", 3)]
        {
            let document = parse(&format!("{number}. item\n"));
            let visuals = visuals(&document);
            assert_eq!(visuals.len(), 1);
            assert_eq!(visuals[0].marker, expected_marker);
            assert_eq!(visuals[0].number, Some(number));
            assert_eq!(visuals[0].marker_width, expected_width);
            assert_eq!(visuals[0].marker_indent_width, expected_width);
            assert_eq!(visuals[0].body_indent, expected_width + 1);
            assert_eq!(visuals[0].marker_style, StyleRole::LIST_MARKER);
            assert_eq!(visuals[0].body_style, StyleRole::DEFAULT);
        }
    }

    #[test]
    fn parent_marker_width_controls_child_indent_while_task_uses_one_cell() {
        let cases = [
            ("- parent\n  - child\n", 2),
            ("1. parent\n   - child\n", 3),
            ("10. parent\n    - child\n", 4),
            ("99. parent\n    - child\n", 4),
            ("- [ ] parent\n  - child\n", 2),
        ];
        for (source, expected_child_indent) in cases {
            let document = parse(source);
            let visuals = visuals(&document);
            assert_eq!(visuals.len(), 2, "{source:?}");
            assert_eq!(visuals[1].indent, expected_child_indent, "{source:?}");
        }
    }

    #[test]
    fn task_markers_expose_marker_and_completed_body_styles() {
        let document = parse("- [ ] open\n- [x] done\n- [X] uppercase\n");
        let visuals = visuals(&document);
        assert_eq!(visuals.len(), 3);
        assert_eq!(visuals[0].marker, "⬜");
        assert_eq!(visuals[0].marker_style, StyleRole::LIST_MARKER);
        assert_eq!(visuals[0].body_style, StyleRole::DEFAULT);
        assert_eq!(visuals[0].task_checked, Some(false));
        for visual in &visuals[1..] {
            assert_eq!(visual.marker, "✅");
            assert_eq!(visual.marker_style, StyleRole::TASK_CHECKED);
            assert_ne!(visual.marker_style, StyleRole::CODE);
            assert_eq!(visual.body_style, StyleRole::STRIKE);
            assert_eq!(visual.task_checked, Some(true));
        }
        for visual in &visuals {
            assert_eq!(visual.marker_width, 2);
            assert_eq!(visual.marker_indent_width, 1);
            assert_eq!(visual.body_indent, 3);
            assert_eq!(visual.continuation_indent, 2);
        }
    }

    #[test]
    fn empty_items_skip_markers_while_empty_alt_images_keep_them() {
        let document = parse("-\n- [ ]\n- ![](image.png)\n- [^note]\n\n[^note]: body\n");
        let items = list_items(&document);
        assert_eq!(items.len(), 3); // The first empty item is a Blank leaf.
        let mut screen = Screen::new(3, 20, ContentArea::new(0, 20)).unwrap();
        for (row, item) in items.into_iter().enumerate() {
            let visible = row == 1;
            assert_eq!(list_visual(&document, item).unwrap().is_some(), visible);
            assert_eq!(
                render_list_marker(&document, item, &mut screen, row as u16)
                    .unwrap()
                    .is_some(),
                visible
            );
            assert!(screen.line_source(row as u16).is_none());
        }
    }

    #[test]
    fn marker_rendering_clips_and_preserves_other_columns_without_line_source() {
        let document = parse("- item\n");
        let item = list_items(&document)[0];
        let mut screen = Screen::new(1, 14, ContentArea::new(5, 8)).unwrap();
        screen
            .write_cell(
                0,
                7,
                Cell::new("B", StyleRole::DEFAULT, 1, None, None).unwrap(),
            )
            .unwrap();
        screen
            .write_cell(
                0,
                12,
                Cell::new("Z", StyleRole::DEFAULT, 1, None, None).unwrap(),
            )
            .unwrap();

        let rendered = render_list_marker(&document, item, &mut screen, 0)
            .unwrap()
            .expect("visible item");
        assert_eq!(rendered.written_width, 2);
        assert!(rendered.marker_complete());
        assert_eq!(screen.cell(0, 5).unwrap().text(), "•");
        assert_eq!(screen.cell(0, 5).unwrap().style(), StyleRole::LIST_MARKER);
        assert_eq!(screen.cell(0, 6).unwrap().text(), " ");
        assert_eq!(screen.cell(0, 6).unwrap().style(), StyleRole::LIST_MARKER);
        assert_eq!(screen.cell(0, 7).unwrap().text(), "B");
        assert_eq!(screen.cell(0, 12).unwrap().text(), "Z");
        assert!(screen.line_source(0).is_none());
        for col in 0..5 {
            assert_eq!(screen.cell(0, col).unwrap().text(), " ");
        }
        for col in 13..14 {
            assert_eq!(screen.cell(0, col).unwrap().text(), " ");
        }

        let mut narrow = Screen::new(1, 1, ContentArea::new(0, 1)).unwrap();
        let narrow_render = render_list_marker(&document, item, &mut narrow, 0)
            .unwrap()
            .unwrap();
        assert_eq!(narrow_render.written_width, 1);
        assert!(!narrow_render.marker_complete());
        assert_eq!(narrow.cell(0, 0).unwrap().text(), "•");

        let nested_document = parse("- parent\n  - child\n");
        let child = list_items(&nested_document)[1];
        let mut nested = Screen::new(1, 10, ContentArea::new(2, 8)).unwrap();
        let nested_render = render_list_marker(&nested_document, child, &mut nested, 0)
            .unwrap()
            .unwrap();
        assert_eq!(nested_render.visual.indent, 2);
        assert_eq!(nested_render.written_width, 2);
        assert_eq!(nested.cell(0, 4).unwrap().text(), "•");
        assert_eq!(nested.cell(0, 5).unwrap().text(), " ");
        assert_eq!(nested.cell(0, 2).unwrap().text(), " ");
        assert_eq!(nested.cell(0, 3).unwrap().text(), " ");
    }

    #[test]
    fn task_marker_cells_share_style_and_narrow_task_is_not_split() {
        let document = parse("- [ ] open\n- [x] done\n");
        let items = list_items(&document);
        let mut screen = Screen::new(2, 10, ContentArea::new(0, 10)).unwrap();
        for row in 0..2 {
            screen
                .write_cell(
                    row,
                    3,
                    Cell::new("B", StyleRole::DEFAULT, 1, None, None).unwrap(),
                )
                .unwrap();
        }
        let open = render_list_marker(&document, items[0], &mut screen, 0)
            .unwrap()
            .unwrap();
        let done = render_list_marker(&document, items[1], &mut screen, 1)
            .unwrap()
            .unwrap();
        assert_eq!(open.written_width, 3);
        assert_eq!(done.written_width, 3);
        assert_eq!(screen.cell(0, 0).unwrap().text(), "⬜");
        assert!(screen.cell(0, 1).unwrap().is_continuation());
        assert_eq!(screen.cell(0, 2).unwrap().style(), StyleRole::LIST_MARKER);
        assert_eq!(screen.cell(0, 3).unwrap().text(), "B");
        assert_eq!(screen.cell(1, 0).unwrap().text(), "✅");
        assert!(screen.cell(1, 1).unwrap().is_continuation());
        assert_eq!(screen.cell(1, 0).unwrap().style(), StyleRole::TASK_CHECKED);
        assert_eq!(screen.cell(1, 1).unwrap().style(), StyleRole::TASK_CHECKED);
        assert_eq!(screen.cell(1, 2).unwrap().style(), StyleRole::TASK_CHECKED);
        assert_eq!(screen.cell(1, 3).unwrap().text(), "B");

        let mut narrow = Screen::new(1, 1, ContentArea::new(0, 1)).unwrap();
        let narrow_render = render_list_marker(&document, items[1], &mut narrow, 0)
            .unwrap()
            .unwrap();
        assert_eq!(narrow_render.written_width, 0);
        assert_eq!(narrow.cell(0, 0).unwrap().text(), " ");
    }

    #[test]
    fn list_fixture_items_compute_and_clip_without_overflow() {
        let document = parse(include_str!("../../tests/fixtures/spec/lists.md"));
        let mut screen = Screen::new(200, 40, ContentArea::new(0, 40)).unwrap();
        let mut row = 0;
        for item in list_items(&document) {
            if let Some(rendered) = render_list_marker(&document, item, &mut screen, row).unwrap() {
                assert!(rendered.written_width <= 40);
                assert!(rendered.visual.body_indent >= rendered.visual.indent);
                row += 1;
            }
            assert!(row < 200);
        }
        assert!(row > 0);
    }

    #[test]
    fn item_composition_keeps_image_identity_and_reorders_emphasis() {
        let document = parse("- base **B** *I* `c` [l](u) ![i](i) ~~s~~\n");
        let (id, block) = list_item_blocks(&document)[0];
        let mut screen = Screen::new(1, 40, ContentArea::new(0, 40)).unwrap();
        assert_eq!(
            render_list_item(&document, id, block, &mut screen, 0),
            Ok(1)
        );
        assert_eq!(row_text(&screen, 0).trim_end(), "• base c i sIB");
        for col in 2..14 {
            assert_eq!(screen.cell(0, col).unwrap().style(), StyleRole::DEFAULT);
            assert_eq!(
                screen.cell(0, col).unwrap().link_id(),
                (col == 9).then_some(mdtui3::LinkId(1))
            );
        }
        assert_eq!(
            screen.line_source(0),
            Some(LineSource::new(block.source, id))
        );
    }

    #[test]
    fn ordered_item_renders_marker_and_body_in_distinct_cells() {
        let document = parse("10. ordered body\n");
        let (id, block) = list_item_blocks(&document)[0];
        let mut screen = Screen::new(1, 24, ContentArea::new(0, 24)).unwrap();
        assert_eq!(
            render_list_item(&document, id, block, &mut screen, 0),
            Ok(1)
        );
        assert_eq!(row_text(&screen, 0).trim_end(), "10. ordered body");
        for col in 0..3 {
            assert_eq!(screen.cell(0, col).unwrap().style(), StyleRole::LIST_MARKER);
        }
        assert_eq!(screen.cell(0, 4).unwrap().style(), StyleRole::DEFAULT);
        assert_eq!(
            screen.line_source(0),
            Some(LineSource::new(block.source, id))
        );
    }

    #[test]
    fn word_wrap_collapses_whitespace_and_drops_separator_at_breaks() {
        let document = parse("- one   two\tthree\n");
        let (id, block) = list_item_blocks(&document)[0];
        let mut screen = Screen::new(4, 10, ContentArea::new(0, 10)).unwrap();
        assert_eq!(
            render_list_item(&document, id, block, &mut screen, 0),
            Ok(2)
        );
        assert_eq!(row_text(&screen, 0).trim_end(), "• one two");
        assert_eq!(row_text(&screen, 1).trim_end(), "  three");
        assert!(!row_text(&screen, 1).starts_with("   "));
        for row in 0..2 {
            assert_eq!(
                screen.line_source(row),
                Some(LineSource::new(block.source, id))
            );
        }

        let punctuation = parse("- hello, world\n");
        let (id, block) = list_item_blocks(&punctuation)[0];
        let mut screen = Screen::new(1, 20, ContentArea::new(0, 20)).unwrap();
        render_list_item(&punctuation, id, block, &mut screen, 0).unwrap();
        assert_eq!(row_text(&screen, 0).trim_end(), "• hello , world");

        let narrow = parse("- hello, world\n");
        let (id, block) = list_item_blocks(&narrow)[0];
        let mut screen = Screen::new(2, 9, ContentArea::new(0, 9)).unwrap();
        assert_eq!(render_list_item(&narrow, id, block, &mut screen, 0), Ok(2));
        assert_eq!(row_text(&screen, 0).trim_end(), "• hello ,");
        assert_eq!(row_text(&screen, 1).trim_end(), "  world");
    }

    #[test]
    fn cjk_classification_and_spacing_rules_are_explicit() {
        for character in ['中', 'あ', 'ア', '한'] {
            assert!(is_cjk(character), "{character:?}");
        }
        for character in ['，', '。', '!', '—'] {
            assert!(is_punctuation(character), "{character:?}");
        }
        assert!(!is_punctuation('Ａ'));
        assert!(!is_punctuation('１'));
        assert!(!is_cjk('A'));
        assert!(!is_punctuation('中'));

        let classes: Vec<_> = list_words("hello,世界，abc!")
            .iter()
            .map(|word| word.class)
            .collect();
        assert_eq!(
            classes,
            [
                ListWordClass::Other,
                ListWordClass::Punctuation,
                ListWordClass::Cjk,
                ListWordClass::Cjk,
                ListWordClass::Punctuation,
                ListWordClass::Other,
                ListWordClass::Punctuation,
            ]
        );
        assert!(!needs_separator(ListWordClass::Cjk, ListWordClass::Cjk));
        assert!(!needs_separator(
            ListWordClass::Cjk,
            ListWordClass::Punctuation
        ));
        assert!(needs_separator(
            ListWordClass::Other,
            ListWordClass::Punctuation
        ));
        assert!(needs_separator(
            ListWordClass::Punctuation,
            ListWordClass::Other
        ));

        let document = parse("- 你好，世界 あア한 中文abc\n");
        let (id, block) = list_item_blocks(&document)[0];
        let mut screen = Screen::new(1, 40, ContentArea::new(0, 40)).unwrap();
        render_list_item(&document, id, block, &mut screen, 0).unwrap();
        assert_eq!(
            row_text(&screen, 0).trim_end(),
            "• 你好，世界 あア한 中文 abc"
        );
    }

    #[test]
    fn long_words_hard_break_and_wide_graphemes_stay_whole() {
        let document = parse("- abcdefgh\n");
        let (id, block) = list_item_blocks(&document)[0];
        let mut screen = Screen::new(4, 5, ContentArea::new(0, 5)).unwrap();
        assert_eq!(
            render_list_item(&document, id, block, &mut screen, 0),
            Ok(3)
        );
        assert_eq!(row_text(&screen, 0), "• abc");
        assert_eq!(row_text(&screen, 1), "  def");
        assert_eq!(row_text(&screen, 2).trim_end(), "  gh");

        let wide = parse("- 中文\n");
        let (id, block) = list_item_blocks(&wide)[0];
        let mut screen = Screen::new(4, 4, ContentArea::new(0, 4)).unwrap();
        assert_eq!(render_list_item(&wide, id, block, &mut screen, 0), Ok(2));
        assert_eq!(row_text(&screen, 0), "• 中");
        assert_eq!(row_text(&screen, 1).trim_end(), "  文");
        assert!(screen.cell(0, 2).unwrap().width() == 2);
        assert!(screen.cell(0, 3).unwrap().is_continuation());

        let wide_narrow = parse("- 中\n");
        let (id, block) = list_item_blocks(&wide_narrow)[0];
        let mut screen = Screen::new(1, 3, ContentArea::new(0, 3)).unwrap();
        assert_eq!(
            render_list_item(&wide_narrow, id, block, &mut screen, 0),
            Ok(1)
        );
        assert_eq!(row_text(&screen, 0).trim_end(), "•");
        assert!(screen.cell(0, 2).unwrap().text() == " ");
    }

    #[test]
    fn completed_task_body_and_continuation_indent_use_strike() {
        let document = parse("- [x] one two three\n");
        let (id, block) = list_item_blocks(&document)[0];
        let mut screen = Screen::new(4, 7, ContentArea::new(0, 7)).unwrap();
        assert_eq!(
            render_list_item(&document, id, block, &mut screen, 0),
            Ok(3)
        );
        assert_eq!(row_text(&screen, 0).trim_end(), "✅ one");
        assert_eq!(row_text(&screen, 1).trim_end(), "  two");
        assert_eq!(row_text(&screen, 2).trim_end(), "  three");
        for col in 0..3 {
            assert_eq!(
                screen.cell(0, col).unwrap().style(),
                StyleRole::TASK_CHECKED
            );
        }
        for col in 3..6 {
            assert_eq!(screen.cell(0, col).unwrap().style(), StyleRole::STRIKE);
        }
        for col in 0..5 {
            assert_eq!(screen.cell(1, col).unwrap().style(), StyleRole::STRIKE);
        }
        for col in 0..7 {
            assert_eq!(screen.cell(2, col).unwrap().style(), StyleRole::STRIKE);
        }
        for row in 1..3 {
            assert_eq!(
                screen.line_source(row),
                Some(LineSource::new(block.source, id))
            );
        }

        let unchecked = parse("- [ ] one two\n");
        let (id, block) = list_item_blocks(&unchecked)[0];
        let mut screen = Screen::new(3, 7, ContentArea::new(0, 7)).unwrap();
        assert_eq!(
            render_list_item(&unchecked, id, block, &mut screen, 0),
            Ok(2)
        );
        assert_eq!(screen.cell(1, 0).unwrap().style(), StyleRole::DEFAULT);
        assert_eq!(screen.cell(1, 1).unwrap().style(), StyleRole::DEFAULT);
        assert_eq!(screen.cell(1, 2).unwrap().style(), StyleRole::DEFAULT);
    }

    #[test]
    fn first_line_indent_spaces_use_marker_style() {
        let document = parse("- parent text\n  - child text\n");
        let items = list_item_blocks(&document);
        let (id, child) = items[1];
        let mut screen = Screen::new(1, 30, ContentArea::new(0, 30)).unwrap();
        render_list_item(&document, id, child, &mut screen, 0).unwrap();
        assert_eq!(screen.cell(0, 0).unwrap().style(), StyleRole::LIST_MARKER);
        assert_eq!(screen.cell(0, 1).unwrap().style(), StyleRole::LIST_MARKER);
        assert_eq!(screen.cell(0, 2).unwrap().text(), "•");
        assert_eq!(screen.cell(0, 3).unwrap().style(), StyleRole::LIST_MARKER);
        assert_eq!(screen.cell(0, 4).unwrap().style(), StyleRole::DEFAULT);
    }

    #[test]
    fn empty_link_and_footnote_items_skip_rows_while_image_keeps_placeholder() {
        let document = parse("- [drop](u)\n- ![](image.png)\n- [^note]\n\n[^note]: body\n");
        let mut screen = Screen::new(3, 20, ContentArea::new(0, 20)).unwrap();
        for (index, (id, block)) in list_item_blocks(&document).into_iter().enumerate() {
            screen.clear_back();
            let image = index == 1;
            assert_eq!(
                render_list_item(&document, id, block, &mut screen, 0),
                Ok(u16::from(image))
            );
            assert_eq!(
                row_text(&screen, 0).trim(),
                if image { "• [image]" } else { "" }
            );
            assert_eq!(screen.line_source(0).is_some(), image);
        }
    }

    #[test]
    fn multiline_item_joins_continuation_blocks_with_one_space() {
        let document = parse("- first line\n  second line\n");
        let (id, block) = list_item_blocks(&document)[0];
        let mut screen = Screen::new(1, 40, ContentArea::new(0, 40)).unwrap();
        assert_eq!(
            render_list_item(&document, id, block, &mut screen, 0),
            Ok(1)
        );
        assert_eq!(row_text(&screen, 0).trim_end(), "• first line second line");
    }

    #[test]
    fn degenerate_widths_and_content_offsets_never_write_outside() {
        let document = parse("- x\n");
        let (id, block) = list_item_blocks(&document)[0];
        let mut zero = Screen::new(1, 1, ContentArea::new(0, 0)).unwrap();
        assert_eq!(render_list_item(&document, id, block, &mut zero, 0), Ok(0));
        assert!(zero.line_source(0).is_none());

        let mut one = Screen::new(1, 1, ContentArea::new(0, 1)).unwrap();
        assert_eq!(render_list_item(&document, id, block, &mut one, 0), Ok(1));
        assert_eq!(one.cell(0, 0).unwrap().text(), "•");
        assert!(one.line_source(0).is_some());

        let mut offset = Screen::new(1, 14, ContentArea::new(5, 8)).unwrap();
        assert_eq!(
            render_list_item(&document, id, block, &mut offset, 0),
            Ok(1)
        );
        assert_eq!(offset.cell(0, 5).unwrap().text(), "•");
        assert_eq!(offset.cell(0, 6).unwrap().text(), " ");
        assert_eq!(offset.cell(0, 7).unwrap().text(), "x");
        for col in 0..5 {
            assert_eq!(offset.cell(0, col).unwrap().text(), " ");
        }
        for col in 12..14 {
            assert_eq!(offset.cell(0, col).unwrap().text(), " ");
        }

        let task = parse("- [x] x\n");
        let (id, block) = list_item_blocks(&task)[0];
        let mut narrow_task = Screen::new(1, 1, ContentArea::new(0, 1)).unwrap();
        assert_eq!(
            render_list_item(&task, id, block, &mut narrow_task, 0),
            Ok(1)
        );
        assert_eq!(narrow_task.cell(0, 0).unwrap().text(), " ");

        let mut full_marker = Screen::new(2, 3, ContentArea::new(0, 3)).unwrap();
        assert_eq!(
            render_list_item(&task, id, block, &mut full_marker, 0),
            Ok(2)
        );
        assert_eq!(full_marker.cell(0, 0).unwrap().text(), "✅");
        assert_eq!(full_marker.cell(1, 0).unwrap().style(), StyleRole::STRIKE);
        assert_eq!(full_marker.cell(1, 1).unwrap().style(), StyleRole::STRIKE);
        assert_eq!(full_marker.cell(1, 2).unwrap().text(), "x");
        assert_eq!(full_marker.cell(1, 2).unwrap().style(), StyleRole::STRIKE);
    }

    #[test]
    fn list_fixture_items_wrap_without_overflow_at_20_80_120() {
        let document = parse(include_str!("../../tests/fixtures/spec/lists.md"));
        for width in [20_u16, 80, 120] {
            let mut screen = Screen::new(600, width, ContentArea::new(0, width)).unwrap();
            let mut row = 0_u16;
            for (id, block) in list_item_blocks(&document) {
                let lines = render_list_item(&document, id, block, &mut screen, row).unwrap();
                assert!(u32::from(row) + u32::from(lines) <= 600, "width {width}");
                for line in row..row + lines {
                    assert_eq!(
                        screen.line_source(line),
                        Some(LineSource::new(block.source, id)),
                        "width {width} row {line}"
                    );
                }
                row += lines;
            }
            assert!(row > 0, "width {width}");
        }
    }
}
