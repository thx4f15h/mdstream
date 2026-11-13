use mdtui3::{Block, BlockKind, Budget, Document, FootnoteTarget, Span, WorkLimit};

use crate::render::input::{DisplayInput, InputToken, TokenKind};
use crate::screen::StyleRole;

const HTML_BLOCK_TAGS: &[&str] = &[
    "address",
    "article",
    "aside",
    "base",
    "basefont",
    "blockquote",
    "body",
    "caption",
    "center",
    "col",
    "colgroup",
    "dd",
    "details",
    "dialog",
    "dir",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "frame",
    "frameset",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hr",
    "html",
    "iframe",
    "legend",
    "li",
    "link",
    "main",
    "menu",
    "menuitem",
    "nav",
    "noframes",
    "ol",
    "optgroup",
    "option",
    "p",
    "param",
    "pre",
    "script",
    "search",
    "section",
    "style",
    "summary",
    "table",
    "tbody",
    "td",
    "textarea",
    "tfoot",
    "th",
    "thead",
    "title",
    "tr",
    "track",
    "ul",
];

#[must_use]
pub fn is_hidden_block(block: &Block) -> bool {
    matches!(
        block.kind,
        BlockKind::Definition { .. } | BlockKind::FootnoteDefinition { .. }
    )
}

#[must_use]
pub fn is_html_block(document: &Document, block: &Block) -> bool {
    if block.kind != BlockKind::Paragraph {
        return false;
    }
    DisplayInput::from_document(document, block.source)
        .is_some_and(|input| html_block_start(&input.visible_text()))
}

/// Return current-block display tokens after removing invisible links, resolved
/// footnote markers, and non-code inline HTML. The query never extends beyond
/// `source`; HIDDEN runs are already omitted by [`DisplayInput`].
#[must_use]
pub fn visible_input(document: &Document, source: Span) -> Option<DisplayInput> {
    let input = DisplayInput::from_document(document, source)?;
    visible_prepared_input(document, source, input)
}

pub(crate) fn visible_prepared_input(
    document: &Document,
    source: Span,
    input: DisplayInput,
) -> Option<DisplayInput> {
    let tokens = input.into_tokens();
    if tokens.iter().all(|token| token.text.is_indexed()) {
        return Some(DisplayInput::from_tokens(tokens));
    }
    let mut hidden = Vec::new();

    let operations = source.len().saturating_add(2);
    let mut budget = Budget::new(WorkLimit {
        queries: operations,
        objects: operations,
        dependencies: operations,
        ..WorkLimit::default()
    });
    let mut references = document.footnote_references_in(source)?;
    while let Some((id, reference)) = references.advance(&mut budget).ok()? {
        let resolution = document.resolve_footnote(id, &mut budget).ok()??;
        if matches!(resolution.target, FootnoteTarget::Defined(_))
            && let Some(span) = clip(reference.source, source)
        {
            hidden.push(span);
        }
    }

    Some(visible_tokens(source, tokens, hidden))
}

pub(crate) fn visible_tokens(
    source: Span,
    tokens: Vec<InputToken>,
    mut hidden: Vec<Span>,
) -> DisplayInput {
    for token in &tokens {
        if token.kind == TokenKind::Text
            && !token.style.contains(StyleRole::CODE)
            && is_mapping_safe(token)
        {
            for (start, end) in
                inline_html_ranges(token.text.contiguous().expect("mapping-safe source token"))
            {
                hidden.push(Span::new(
                    token.source.start + start,
                    token.source.start + end,
                ));
            }
        }
    }

    let hidden = merge_spans(hidden, source);
    let mut visible = Vec::with_capacity(tokens.len());
    for token in tokens {
        visible.extend(visibly_split_token(token, &hidden));
    }
    DisplayInput::from_tokens(visible)
}

fn clip(span: Span, limit: Span) -> Option<Span> {
    let start = span.start.max(limit.start);
    let end = span.end.min(limit.end);
    (start < end).then_some(Span::new(start, end))
}

fn merge_spans(spans: Vec<Span>, limit: Span) -> Vec<Span> {
    let mut spans: Vec<_> = spans
        .into_iter()
        .filter_map(|span| clip(span, limit))
        .collect();
    spans.sort_unstable_by_key(|span| (span.start, span.end));
    let mut merged: Vec<Span> = Vec::with_capacity(spans.len());
    for span in spans {
        if let Some(last) = merged.last_mut()
            && span.start <= last.end
        {
            last.end = last.end.max(span.end);
        } else {
            merged.push(span);
        }
    }
    merged
}

fn is_mapping_safe(token: &InputToken) -> bool {
    !token.text.is_indexed()
        && token.replacement.is_none()
        && token.source.len() == token.text.len()
}

fn visibly_split_token(token: InputToken, hidden: &[Span]) -> Vec<InputToken> {
    // Indexed clusters already consumed canonical conditional footnote facts;
    // their gapped source envelope must not be dropped by a second span filter.
    if token.text.is_indexed() {
        return vec![token];
    }
    let first = hidden.partition_point(|span| span.end <= token.source.start);
    if first == hidden.len() || hidden[first].start >= token.source.end {
        return vec![token];
    }
    if !is_mapping_safe(&token) {
        return Vec::new();
    }

    let mut output = Vec::new();
    let mut cursor = token.source.start;
    for span in &hidden[first..] {
        if span.start >= token.source.end {
            break;
        }
        let start = span.start.max(token.source.start);
        let end = span.end.min(token.source.end);
        if cursor < start {
            output.push(token_fragment(&token, cursor, start));
        }
        cursor = cursor.max(end);
    }
    if cursor < token.source.end {
        output.push(token_fragment(&token, cursor, token.source.end));
    }
    output
}

fn token_fragment(token: &InputToken, start: usize, end: usize) -> InputToken {
    let relative_start = start - token.source.start;
    let relative_end = end - token.source.start;
    let mut fragment = token.clone();
    fragment.text = token.text.contiguous().expect("mapping-safe source token")
        [relative_start..relative_end]
        .into();
    fragment.source = Span::new(start, end);
    fragment.replacement = None;
    fragment
}

fn html_block_start(text: &str) -> bool {
    let trimmed = text.trim_start_matches(|character: char| character.is_ascii_whitespace());
    if trimmed.starts_with("<!--")
        || trimmed.starts_with("<![CDATA[")
        || trimmed.starts_with("<?")
        || starts_with_ignore_ascii_case(trimmed, "<!doctype")
    {
        return true;
    }
    let Some(mut rest) = trimmed.strip_prefix('<') else {
        return false;
    };
    rest = rest.strip_prefix('/').unwrap_or(rest);
    let end = rest
        .find(|character: char| !character.is_ascii_alphanumeric())
        .unwrap_or(rest.len());
    if end == 0 {
        return false;
    }
    let name = &rest[..end];
    HTML_BLOCK_TAGS
        .iter()
        .any(|tag| tag.eq_ignore_ascii_case(name))
}

fn starts_with_ignore_ascii_case(text: &str, prefix: &str) -> bool {
    text.as_bytes()
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix.as_bytes()))
}

fn inline_html_ranges(text: &str) -> Vec<(usize, usize)> {
    let mut recognizer = super::html_shape::HtmlShape::default();
    let mut spans = Vec::new();
    for (at, ch) in text.char_indices() {
        if let Some(span) = recognizer.push(at, ch) {
            spans.push((span.start, span.end));
        }
    }
    if let Some(span) = recognizer.open(text.len()) {
        spans.push((span.start, span.end));
    }
    spans
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

    fn block(document: &Document, kind: BlockKind) -> (mdtui3::BlockId, &Block) {
        document
            .blocks()
            .iter()
            .enumerate()
            .find(|(_, block)| std::mem::discriminant(&block.kind) == std::mem::discriminant(&kind))
            .map(|(id, block)| (mdtui3::BlockId(id), block))
            .expect("matching block")
    }

    fn plain_text(document: &Document, span: Span) -> String {
        let text: String = visible_input(document, span)
            .unwrap()
            .tokens()
            .iter()
            .filter(|token| token.kind == TokenKind::Text)
            .map(|token| token.text.collect_text())
            .collect();
        text.trim_end_matches(['\r', '\n']).to_owned()
    }

    #[test]
    fn images_keep_labels_placeholders_and_surrounding_spaces() {
        let document = parse("a ![x](y) b\n");
        let (_, paragraph) = block(&document, BlockKind::Paragraph);
        assert_eq!(plain_text(&document, paragraph.source), "a x b");

        let standalone = parse("![x](y)\n");
        let (_, image) = block(&standalone, BlockKind::Paragraph);
        assert_eq!(plain_text(&standalone, image.source), "x");

        let empty = parse("![](y)\n");
        let (_, empty_image) = block(&empty, BlockKind::Paragraph);
        assert_eq!(plain_text(&empty, empty_image.source), "[image]");
    }

    #[test]
    fn resolved_footnote_markers_disappear_but_missing_ones_stay() {
        let source = concat!(
            "Defined[^one] again[^one], named[^two], undefined[^missing].\n\n",
            "[^one]: first\n",
            "[^two]: second\n",
        );
        let document = parse(source);
        let (_, paragraph) = block(&document, BlockKind::Paragraph);
        assert_eq!(
            plain_text(&document, paragraph.source),
            "Defined again, named, undefined[^missing]."
        );

        let undefined = parse("Missing[^none].\n");
        let (_, paragraph) = block(&undefined, BlockKind::Paragraph);
        assert_eq!(plain_text(&undefined, paragraph.source), "Missing[^none].");
    }

    #[test]
    fn definition_blocks_are_structurally_hidden() {
        let document = parse("[ref]: https://example.com\n\n[^note]: body\n");
        let (_, definition) = block(
            &document,
            BlockKind::Definition {
                symbol: mdtui3::SymbolId(0),
            },
        );
        assert!(is_hidden_block(definition));
        assert_eq!(
            DisplayInput::from_document(&document, definition.source)
                .unwrap()
                .visible_text(),
            ""
        );
        let (_, footnote) = block(
            &document,
            BlockKind::FootnoteDefinition {
                symbol: mdtui3::FootnoteId(0),
            },
        );
        assert!(is_hidden_block(footnote));
        assert_eq!(
            DisplayInput::from_document(&document, footnote.source)
                .unwrap()
                .visible_text(),
            ""
        );
    }

    #[test]
    fn inline_html_tags_and_comments_disappear_without_touching_text() {
        let document = parse(concat!(
            "a <b>h</b> c\n\n",
            "Attributes <span class=\"x\" data-y=\"z\">inside</span> remain.\n\n",
            "Self <br/> closing.\n\n",
            "Comment a <!-- hidden --> b.\n\n",
            "Unclosed <not a tag remains.\n",
        ));
        let paragraphs: Vec<_> = document
            .blocks()
            .iter()
            .filter(|block| block.kind == BlockKind::Paragraph)
            .collect();
        assert_eq!(plain_text(&document, paragraphs[0].source), "a h c");
        assert_eq!(
            plain_text(&document, paragraphs[1].source),
            "Attributes inside remain."
        );
        assert_eq!(
            plain_text(&document, paragraphs[2].source),
            "Self  closing."
        );
        assert_eq!(plain_text(&document, paragraphs[3].source), "Comment a  b.");
        assert_eq!(
            plain_text(&document, paragraphs[4].source),
            "Unclosed <not a tag remains."
        );
    }

    #[test]
    fn recognized_html_blocks_are_skipped_and_following_paragraph_is_not() {
        let source = "<div class=\"block\">\nbody\n</div>\n\nAfter.\n";
        let document = parse(source);
        let paragraphs: Vec<_> = document
            .blocks()
            .iter()
            .filter(|block| block.kind == BlockKind::Paragraph)
            .collect();
        assert!(is_html_block(&document, paragraphs[0]));
        assert!(!is_html_block(&document, paragraphs[1]));
        assert_eq!(plain_text(&document, paragraphs[1].source), "After.");

        let comment = parse("<!-- block comment -->\n\nAfter comment.\n");
        let paragraphs: Vec<_> = comment
            .blocks()
            .iter()
            .filter(|block| block.kind == BlockKind::Paragraph)
            .collect();
        assert!(is_html_block(&comment, paragraphs[0]));
        assert!(!is_html_block(&comment, paragraphs[1]));
        assert_eq!(plain_text(&comment, paragraphs[1].source), "After comment.");
    }

    #[test]
    fn image_styles_and_link_metadata_do_not_leak_to_neighbors() {
        let document = parse("**a** ![x](y) *b*\n");
        let (_, paragraph) = block(&document, BlockKind::Paragraph);
        let input = visible_input(&document, paragraph.source).unwrap();
        let tokens: Vec<_> = input
            .tokens()
            .iter()
            .filter(|token| token.kind == TokenKind::Text)
            .collect();
        assert_eq!(input.visible_text().trim_end(), "a x b");
        assert_eq!(
            tokens.iter().filter(|token| token.link.is_some()).count(),
            1
        );
        let a = tokens.iter().find(|token| token.text == "a").unwrap();
        let b = tokens.iter().find(|token| token.text == "b").unwrap();
        assert!(a.style.contains(StyleRole::STRONG));
        assert!(b.style.contains(StyleRole::EMPHASIS));
    }

    #[test]
    fn fixture_sources_clean_without_panicking_or_overflowing() {
        for source in [
            include_str!("../../tests/fixtures/spec/images.md"),
            include_str!("../../tests/fixtures/spec/footnotes.md"),
            include_str!("../../tests/fixtures/spec/html.md"),
        ] {
            let document = parse(source);
            for block in document.blocks() {
                if is_hidden_block(block) || is_html_block(&document, block) {
                    continue;
                }
                let input = visible_input(&document, block.source).unwrap();
                assert!(input.visible_text().len() <= block.source.len());
            }
        }
    }
}
