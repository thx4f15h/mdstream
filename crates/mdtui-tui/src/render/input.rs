use mdtui3::{BlockId, DisplayRun, Document, LinkId, Span, Style};

use crate::screen::StyleRole;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Text,
    SoftBreak,
    HardBreak,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputToken {
    pub kind: TokenKind,
    pub text: String,
    pub source: Span,
    pub replacement: Option<Span>,
    pub block: Option<BlockId>,
    pub link: Option<LinkId>,
    pub style: StyleRole,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DisplayInput {
    tokens: Vec<InputToken>,
}

impl DisplayInput {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn from_tokens(tokens: Vec<InputToken>) -> Self {
        Self { tokens }
    }

    pub fn from_document(document: &Document, source: Span) -> Option<Self> {
        let links = crate::render::objects::SharedLinks::new(document);
        let mut objects =
            crate::render::objects::ObjectWindow::new(document, source, links.clone())?;
        let mut budget = mdtui3::Budget::new(mdtui3::WorkLimit::unlimited());
        if !objects.advance(document, &mut budget).ok()? {
            return None;
        }
        let mut cursor = document.display_cursor(source)?;
        let mut input = Self::new();
        loop {
            match links
                .with_resolver(|resolver| {
                    cursor.next_with_resolver(document, resolver, &mut budget)
                })
                .ok()?
            {
                mdtui3::DisplayStep::Run(mut run) => {
                    input.tokens.extend(image_tokens(&objects, &run));
                    let presentation = run
                        .link
                        .map(|id| objects.presentation(id))
                        .unwrap_or_default();
                    run.link = presentation.hit;
                    let literal_image = objects.is_literal_image(run.source);
                    if input.push_run(run) {
                        let token = input.tokens.last_mut()?;
                        if presentation.image.is_some() {
                            token.style = token.style.with(StyleRole::IMAGE);
                        } else if presentation.disposition
                            == Some(mdtui3::LinkDisposition::Unresolved)
                            || literal_image
                        {
                            token.style = token.style.with(StyleRole::LITERAL_LINK);
                        }
                    }
                }
                mdtui3::DisplayStep::Complete => break,
                mdtui3::DisplayStep::BudgetExhausted => return None,
            }
        }
        if !objects.finish_empty(&mut budget, |label| {
            let at = input
                .tokens
                .partition_point(|token| token.source.end <= label.start);
            input
                .tokens
                .get(at)
                .is_some_and(|token| token.source.start < label.end && !token.text.is_empty())
        }) {
            return None;
        }
        let fallback = objects
            .empty_fallbacks
            .iter()
            .map(|object| InputToken {
                kind: TokenKind::Text,
                text: "[image]".to_owned(),
                source: object.source,
                replacement: Some(object.source),
                block: document.block_at(object.source.start),
                link: Some(object.hit),
                style: StyleRole::LINK.with(StyleRole::IMAGE),
            })
            .collect();
        input.tokens = merge_source(input.tokens, fallback, |token| token.source.start);
        Some(input)
    }

    pub(crate) fn clipped(&self, source: Span) -> Self {
        let start = self
            .tokens
            .partition_point(|token| token.source.end <= source.start);
        let mut tokens = Vec::new();
        for token in self.tokens[start..]
            .iter()
            .take_while(|token| token.source.start < source.end)
        {
            if token.replacement.is_some() || token.source.len() != token.text.len() {
                if token.source.start >= source.start {
                    tokens.push(token.clone());
                }
                continue;
            }
            let lo = token.source.start.max(source.start);
            let hi = token.source.end.min(source.end);
            if let Some(text) = token
                .text
                .get(lo - token.source.start..hi - token.source.start)
            {
                let mut clipped = token.clone();
                clipped.source = Span::new(lo, hi);
                clipped.text = text.to_owned();
                tokens.push(clipped);
            }
        }
        Self::from_tokens(tokens)
    }

    pub fn push_run(&mut self, run: DisplayRun<'_>) -> bool {
        if run.style.contains(Style::HIDDEN) {
            return false;
        }
        let kind = token_kind(run.style);
        self.tokens.push(InputToken {
            kind,
            text: run.text.to_owned(),
            source: run.source,
            replacement: run.replacement,
            block: run.block,
            link: run.link,
            style: style_role(run.style, run.link.is_some()),
        });
        true
    }

    #[must_use]
    pub fn tokens(&self) -> &[InputToken] {
        &self.tokens
    }

    #[must_use]
    pub fn into_tokens(self) -> Vec<InputToken> {
        self.tokens
    }

    #[must_use]
    pub fn visible_text(&self) -> String {
        self.tokens
            .iter()
            .map(|token| token.text.as_str())
            .collect()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.tokens.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }
}

pub(crate) fn token_kind(style: Style) -> TokenKind {
    if style.contains(Style::HARD_BREAK) {
        TokenKind::HardBreak
    } else if style.contains(Style::SOFT_BREAK) {
        TokenKind::SoftBreak
    } else {
        TokenKind::Text
    }
}

pub(crate) fn style_role(style: Style, linked: bool) -> StyleRole {
    let mut role = StyleRole::DEFAULT;
    if style.contains(Style::EMPHASIS) {
        role = role.with(StyleRole::EMPHASIS);
    }
    if style.contains(Style::STRONG) {
        role = role.with(StyleRole::STRONG);
    }
    if style.contains(Style::STRIKE) {
        role = role.with(StyleRole::STRIKE);
    }
    if style.contains(Style::CODE) {
        role = role.with(StyleRole::CODE);
    }
    if style.contains(Style::MATH) {
        role = role.with(StyleRole::MATH);
    }
    if linked {
        role = role.with(StyleRole::LINK);
    }
    role
}

/// Prepared empty-alt controls, never raw candidate hits or descendant images.
pub(crate) fn image_tokens<'a>(
    objects: &'a crate::render::objects::ObjectWindow,
    run: &'a DisplayRun<'_>,
) -> impl Iterator<Item = InputToken> + 'a {
    let literals = if run.style.contains(Style::HIDDEN) {
        objects.literal_syntax(run.source)
    } else {
        Vec::new()
    };
    let start = objects
        .placeholders
        .partition_point(|object| object.source.start < run.source.start);
    let placeholders = objects.placeholders[start..]
        .iter()
        .take_while(move |object| object.source.start < run.source.end)
        .map(move |object| InputToken {
            kind: TokenKind::Text,
            text: "[image]".to_owned(),
            source: object.source,
            replacement: Some(object.source),
            block: run.block,
            link: Some(object.hit),
            style: StyleRole::LINK.with(StyleRole::IMAGE),
        });
    let mut literal_tokens = literals
        .into_iter()
        .map(move |syntax| InputToken {
            kind: TokenKind::Text,
            text: if run.replacement.is_some() {
                run.text.to_owned()
            } else {
                run.text
                    [syntax.source.start - run.source.start..syntax.source.end - run.source.start]
                    .to_owned()
            },
            source: syntax.source,
            replacement: run.replacement,
            block: run.block,
            link: syntax.hit,
            style: style_role(run.style, syntax.hit.is_some())
                .with(StyleRole::LITERAL_LINK)
                .with(if syntax.image {
                    StyleRole::IMAGE
                } else {
                    StyleRole::DEFAULT
                }),
        })
        .peekable();
    let mut placeholders = placeholders.peekable();
    std::iter::from_fn(move || match (literal_tokens.peek(), placeholders.peek()) {
        (Some(a), Some(b)) if a.source.start <= b.source.start => literal_tokens.next(),
        (Some(_), Some(_)) | (None, Some(_)) => placeholders.next(),
        (Some(_), None) => literal_tokens.next(),
        (None, None) => None,
    })
}

pub(crate) fn merge_source<T>(left: Vec<T>, right: Vec<T>, source: impl Fn(&T) -> usize) -> Vec<T> {
    let mut left = left.into_iter().peekable();
    let mut right = right.into_iter().peekable();
    std::iter::from_fn(move || match (left.peek(), right.peek()) {
        (Some(a), Some(b)) if source(a) <= source(b) => left.next(),
        (Some(_), Some(_)) | (None, Some(_)) => right.next(),
        (Some(_), None) => left.next(),
        (None, None) => None,
    })
    .collect()
}

/// Borrowed pieces of one display stream. Grapheme boundaries are determined
/// after joining the bounded window, never independently at cursor/tile cuts.
pub(crate) struct GlyphInput<'a> {
    pub text: &'a str,
    pub source: Span,
    pub replacement: Option<Span>,
    pub block: Option<BlockId>,
    pub link: Option<LinkId>,
    pub style: StyleRole,
    pub kind: TokenKind,
}

pub(crate) struct DisplayGlyph {
    pub text: String,
    pub source: Span,
    pub block: Option<BlockId>,
    pub link: Option<LinkId>,
    pub style: StyleRole,
    pub width: u8,
    pub kind: crate::render::text::GraphemeKind,
}

pub(crate) fn display_glyphs<'a>(parts: impl Iterator<Item = GlyphInput<'a>>) -> Vec<DisplayGlyph> {
    let mut text = String::new();
    let mut spans = Vec::new();
    for part in parts {
        let value = match part.kind {
            TokenKind::HardBreak => continue,
            TokenKind::SoftBreak => " ",
            TokenKind::Text => part.text,
        };
        if value.is_empty() {
            continue;
        }
        let start = text.len();
        text.push_str(value);
        spans.push((start, text.len(), part));
    }
    let mut index = 0;
    let mut glyphs = Vec::new();
    for grapheme in crate::render::text::graphemes(&text) {
        while spans[index].1 <= grapheme.start {
            index += 1;
        }
        let (first_start, _, first) = &spans[index];
        let source_start = if first.replacement.is_none()
            && first.source.len() == first.text.len()
            && first.kind == TokenKind::Text
        {
            first.source.start + grapheme.start - first_start
        } else {
            first.source.start
        };
        let mut last_index = index;
        while spans[last_index].1 < grapheme.end {
            last_index += 1;
        }
        let (last_start, _, last) = &spans[last_index];
        let source_end = if last.replacement.is_none()
            && last.source.len() == last.text.len()
            && last.kind == TokenKind::Text
        {
            last.source.start + grapheme.end - last_start
        } else {
            last.source.end
        };
        glyphs.push(DisplayGlyph {
            text: grapheme.text.to_owned(),
            source: Span::new(source_start, source_end),
            block: first.block,
            link: first.link,
            style: first.style,
            width: grapheme.width,
            kind: grapheme.kind,
        });
        index = last_index;
    }
    glyphs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> Document {
        let mut document = Document::new();
        document.append(source).unwrap();
        document.finish();
        document
    }

    fn display_visible(document: &Document, span: Span) -> String {
        document
            .display_runs_in(span)
            .unwrap()
            .filter(|run| !run.style.contains(Style::HIDDEN))
            .map(|run| run.text)
            .collect()
    }

    fn token_with_text<'a>(input: &'a DisplayInput, text: &str) -> &'a InputToken {
        input
            .tokens()
            .iter()
            .find(|token| token.text == text)
            .unwrap_or_else(|| panic!("missing input token {text:?}"))
    }

    #[test]
    fn entities_and_code_whitespace_are_not_renormalized() {
        let source = "a &amp; b\n\n`a  \t b`\n";
        let document = parse(source);
        let span = Span::new(0, source.len());
        let input = DisplayInput::from_document(&document, span).unwrap();
        assert_eq!(input.visible_text(), "a & b\n\na  \t b\n");
        assert_eq!(input.visible_text(), display_visible(&document, span));
        assert!(!input.visible_text().contains('`'));

        let entity = token_with_text(&input, "&");
        assert!(entity.replacement.is_some());
        let code = token_with_text(&input, "a  \t b");
        assert!(code.style.contains(StyleRole::CODE));
    }

    #[test]
    fn soft_hard_and_crlf_breaks_become_distinct_tokens() {
        let source = "soft\nline\n\nhard  \nline\n\ncrlf\r\nline\n";
        let document = parse(source);
        let span = Span::new(0, source.len());
        let input = DisplayInput::from_document(&document, span).unwrap();
        assert_eq!(input.visible_text(), display_visible(&document, span));

        let breaks: Vec<_> = input
            .tokens()
            .iter()
            .filter(|token| token.kind != TokenKind::Text)
            .collect();
        assert_eq!(
            breaks
                .iter()
                .filter(|token| token.kind == TokenKind::SoftBreak)
                .count(),
            2
        );
        assert_eq!(
            breaks
                .iter()
                .filter(|token| token.kind == TokenKind::HardBreak)
                .count(),
            1
        );
        assert!(breaks.iter().all(|token| token.text == "\n"));

        let crlf_lf = source.find("crlf\r\n").unwrap() + "crlf\r".len();
        let crlf_break = breaks
            .iter()
            .find(|token| token.source.start <= crlf_lf && token.source.end > crlf_lf)
            .expect("CRLF soft break must map to its LF");
        assert_eq!(crlf_break.kind, TokenKind::SoftBreak);
        let hard_lf = source.find("hard  \n").unwrap() + "hard  ".len();
        let hard_break = breaks
            .iter()
            .find(|token| token.kind == TokenKind::HardBreak)
            .expect("hard break token must exist");
        assert!(hard_break.source.start <= hard_lf && hard_break.source.end > hard_lf);
        assert!(breaks.iter().all(|token| token.source.start < source.len()));
    }

    #[test]
    fn hidden_container_prefixes_do_not_leak_and_metadata_survives() {
        let source = "> - *hello* [link](https://example.com)\n";
        let document = parse(source);
        let span = Span::new(0, source.len());
        let input = DisplayInput::from_document(&document, span).unwrap();
        assert_eq!(input.visible_text(), display_visible(&document, span));
        assert!(input.visible_text().starts_with("hello"));
        assert!(!input.visible_text().contains("> "));
        assert!(!input.visible_text().contains("- "));

        let hello = token_with_text(&input, "hello");
        assert!(hello.style.contains(StyleRole::EMPHASIS));
        assert!(hello.block.is_some());
        assert!(hello.source.start < hello.source.end);
        let link = token_with_text(&input, "link");
        assert!(link.link.is_some());
        assert!(link.style.contains(StyleRole::LINK));
        assert!(link.block.is_some());
    }

    #[test]
    fn core_styles_map_to_tui_roles_including_math() {
        let source = "*e* **s** ~~x~~ `c` $m$ [l](u)\n";
        let document = parse(source);
        let span = Span::new(0, source.len());
        let input = DisplayInput::from_document(&document, span).unwrap();

        assert!(
            token_with_text(&input, "e")
                .style
                .contains(StyleRole::EMPHASIS)
        );
        assert!(
            token_with_text(&input, "s")
                .style
                .contains(StyleRole::STRONG)
        );
        assert!(
            token_with_text(&input, "x")
                .style
                .contains(StyleRole::STRIKE)
        );
        assert!(token_with_text(&input, "c").style.contains(StyleRole::CODE));
        assert!(token_with_text(&input, "m").style.contains(StyleRole::MATH));
        assert!(token_with_text(&input, "l").style.contains(StyleRole::LINK));
    }

    #[test]
    fn incremental_feed_matches_static_ingestion_and_preserves_order() {
        let source = format!("{}tail\n", "**a** &amp; `a  b` ".repeat(80));
        let document = parse(&source);
        let span = Span::new(0, source.len());
        let static_input = DisplayInput::from_document(&document, span).unwrap();
        let mut fed = DisplayInput::new();
        for run in document.display_runs_in(span).unwrap() {
            fed.push_run(run);
        }
        assert_eq!(fed, static_input);
        assert_eq!(fed.visible_text(), display_visible(&document, span));
        assert!(source.len() > 256);
        assert!(fed.len() > 1);
    }

    #[test]
    fn fixture_inline_and_paragraphs_ingest_end_to_end() {
        let inline = include_str!("../../tests/fixtures/spec/inline.md");
        let paragraphs = include_str!("../../tests/fixtures/spec/paragraphs.md");
        for source in [inline, paragraphs] {
            let document = parse(source);
            let span = Span::new(0, source.len());
            let input = DisplayInput::from_document(&document, span).unwrap();
            assert_eq!(input.visible_text(), display_visible(&document, span));
            assert!(!input.is_empty());
        }

        let inline_document = parse(inline);
        let inline_input =
            DisplayInput::from_document(&inline_document, Span::new(0, inline.len())).unwrap();
        assert!(inline_input.visible_text().contains("a & b"));
        assert!(!inline_input.visible_text().contains("&amp;"));

        let paragraph_document = parse(paragraphs);
        let paragraph_input =
            DisplayInput::from_document(&paragraph_document, Span::new(0, paragraphs.len()))
                .unwrap();
        let paragraph_text = paragraph_input.visible_text();
        assert!(paragraph_text.contains("中文"));
        assert!(paragraph_text.contains('🙂'));
        assert!(paragraph_text.contains('\t'));
        assert!(paragraph_text.contains('\u{a0}'));
    }
}
