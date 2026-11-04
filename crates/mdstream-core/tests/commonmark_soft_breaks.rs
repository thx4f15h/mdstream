//! Offline scoring of the complete CommonMark 0.31.2 Soft line breaks section.
//! Project only public paragraph/display semantics; never trim whitespace,
//! reparse Markdown or use example numbers to alter the projection.
use mdstream::{Block, BlockKind, Document, Span, Style};

include!("fixtures/commonmark_soft_breaks.rs");

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn project(doc: &Document) -> String {
    assert_eq!(doc.container_count(), 0);
    assert!(doc.headings().entries().is_empty());
    assert!(doc.links().is_empty());
    assert!(doc.maths().is_empty());
    let mut html = String::new();
    for block in doc.blocks() {
        match block.kind {
            BlockKind::Blank => continue,
            BlockKind::Paragraph => {}
            ref other => panic!("block outside soft break projection: {other:?}"),
        }
        let mut chars = Vec::new();
        for run in doc.display_runs_in(block.source).unwrap() {
            if run.style.contains(Style::HIDDEN) {
                continue;
            }
            assert!(run.link.is_none());
            for unsupported in [
                Style::EMPHASIS,
                Style::STRONG,
                Style::STRIKE,
                Style::CODE,
                Style::MATH,
            ] {
                assert!(!run.style.contains(unsupported), "unscored inline style");
            }
            chars.extend(run.text.chars().map(|ch| (ch, run.style)));
        }
        // Only the physical paragraph terminator is omitted, not internal LFs
        // or any spaces. A terminator must not carry an inline break marker.
        if chars.last().is_some_and(|(ch, _)| *ch == '\n') {
            let (_, style) = chars.pop().unwrap();
            assert!(!style.contains(Style::HARD_BREAK));
            assert!(!style.contains(Style::SOFT_BREAK));
        }
        html.push_str("<p>");
        for (ch, style) in chars {
            if ch == '\n' && style.contains(Style::HARD_BREAK) {
                html.push_str("<br />");
            }
            match ch {
                '&' => html.push_str("&amp;"),
                '<' => html.push_str("&lt;"),
                '>' => html.push_str("&gt;"),
                '"' => html.push_str("&quot;"),
                _ => html.push(ch),
            }
        }
        html.push_str("</p>\n");
    }
    html
}

fn check_case(number: usize, markdown: &str, expected: &str, doc: &Document) {
    assert_eq!(
        project(doc),
        expected,
        "example {number}, input {markdown:?}"
    );
    // HTML alone cannot distinguish a correctly marked soft break from a
    // literal LF. Pin the semantic marker and its original source separately.
    let breaks: Vec<_> = doc
        .display_runs_in(Span::new(0, doc.source().len()))
        .unwrap()
        .filter(|run| !run.style.contains(Style::HIDDEN))
        .filter(|run| run.style.contains(Style::SOFT_BREAK))
        .map(|run| {
            assert!(!run.style.contains(Style::HARD_BREAK));
            (run.source, run.replacement, run.text.to_owned())
        })
        .collect();
    let newline = markdown.find('\n').unwrap();
    assert_eq!(
        breaks,
        [(Span::new(newline, newline + 1), None, "\n".to_owned())]
    );
}

#[test]
fn pinned_commonmark_soft_breaks_report() {
    assert_eq!(SOFT_BREAK_CASES.len(), 2);
    for (index, &(number, markdown, expected)) in SOFT_BREAK_CASES.iter().enumerate() {
        assert_eq!(number, 648 + index);
        check_case(number, markdown, expected, &parse(markdown));
        eprintln!("example {number}: supported — matches upstream and soft break source mapping");
    }
    eprintln!(
        "CommonMark 0.31.2 Soft line breaks (648–649): 2 supported, 0 deliberate difference, 0 unsupported; other sections not scored by this test"
    );
}

#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    blocks: Vec<Block>,
    display: Vec<(Span, Option<Span>, String, Style)>,
    pending: Span,
}

fn snapshot(doc: &Document) -> Snapshot {
    assert_eq!(doc.container_count(), 0);
    assert!(doc.headings().entries().is_empty());
    assert!(doc.links().is_empty());
    assert!(doc.maths().is_empty());
    Snapshot {
        blocks: doc.blocks().to_vec(),
        display: doc
            .display_runs_in(Span::new(0, doc.source().len()))
            .unwrap()
            .map(|run| (run.source, run.replacement, run.text.to_owned(), run.style))
            .collect(),
        pending: doc.pending(),
    }
}

#[test]
fn pinned_commonmark_soft_breaks_every_split_and_scalar_prefix() {
    let mut splits = 0;
    let mut prefixes = 0;
    for &(number, markdown, expected) in SOFT_BREAK_CASES {
        let whole = parse(markdown);
        for split in (0..=markdown.len()).filter(|&split| markdown.is_char_boundary(split)) {
            let mut doc = Document::new();
            doc.append(&markdown[..split]).unwrap();
            doc.append(&markdown[split..]).unwrap();
            doc.finish();
            assert_eq!(
                snapshot(&doc),
                snapshot(&whole),
                "example {number}, split {split}"
            );
            check_case(number, markdown, expected, &doc);
            splits += 1;
        }
        let mut doc = Document::new();
        for (offset, ch) in markdown.char_indices() {
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            let mut prefix = Document::new();
            prefix.append(&markdown[..offset + ch.len_utf8()]).unwrap();
            assert_eq!(
                snapshot(&doc),
                snapshot(&prefix),
                "example {number}, prefix {offset}"
            );
            prefixes += 1;
        }
        doc.finish();
        check_case(number, markdown, expected, &doc);
        assert_eq!(
            snapshot(&doc),
            snapshot(&whole),
            "example {number}, scalar stream"
        );
    }
    assert_eq!((splits, prefixes), (20, 18));
    eprintln!(
        "2 examples: 20 single splits and 18 scalar prefixes preserve semantics and source metadata"
    );
}

#[test]
fn soft_breaks_projection_preserves_whitespace_and_break_distinctions() {
    for (source, expected) in [
        ("a\nb\n", "<p>a\nb</p>\n"),
        ("a\n\nb\n", "<p>a</p>\n<p>b</p>\n"),
        ("a  \nb\n", "<p>a<br />\nb</p>\n"),
        ("a\\\nb\n", "<p>a<br />\nb</p>\n"),
        ("a  b\n", "<p>a  b</p>\n"),
        ("a\tb\n", "<p>a\tb</p>\n"),
        ("a &#32;\n", "<p>a  </p>\n"),
        ("a &#32;\nb\n", "<p>a  \nb</p>\n"),
        ("<&\"\n", "<p>&lt;&amp;&quot;</p>\n"),
    ] {
        assert_eq!(project(&parse(source)), expected, "input {source:?}");
    }
}

#[test]
fn soft_breaks_projection_rejects_unscored_structure_and_styles() {
    for source in [
        "# heading\n",
        "> quote\n",
        "- item\n",
        "---\n",
        "    code\n",
        "*emphasis*\n",
        "**strong**\n",
        "~~strike~~\n",
        "`code`\n",
        "[link](url)\n",
        "$math$\n",
    ] {
        assert!(
            std::panic::catch_unwind(|| project(&parse(source))).is_err(),
            "accepted unscored input {source:?}"
        );
    }
}
