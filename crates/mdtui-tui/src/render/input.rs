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

    pub fn from_document(document: &Document, source: Span) -> Option<Self> {
        let mut input = Self::new();
        for run in document.display_runs_in(source)? {
            input.push_run(run);
        }
        Some(input)
    }

    pub fn push_run(&mut self, run: DisplayRun<'_>) -> bool {
        if run.style.contains(Style::HIDDEN) {
            return false;
        }
        let kind = if run.style.contains(Style::HARD_BREAK) {
            TokenKind::HardBreak
        } else if run.style.contains(Style::SOFT_BREAK) {
            TokenKind::SoftBreak
        } else {
            TokenKind::Text
        };
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

fn style_role(style: Style, linked: bool) -> StyleRole {
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
