//! Offline scoring of the complete CommonMark 0.31.2 Textual content section.
//! Project only public paragraph/display semantics; never trim whitespace,
//! reparse Markdown or use example numbers to alter the projection.
use mdstream::{Block, BlockKind, Document, Span, Style, Utf8Document};

include!("fixtures/commonmark_textual_content.rs");

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
            ref other => panic!("block outside textual content projection: {other:?}"),
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
    assert_eq!(doc.source(), markdown);
    // These plain-text examples must retain byte-for-byte borrowed mapping,
    // including the multibyte Greek scalars and all five internal spaces.
    for run in doc.display_runs_in(Span::new(0, markdown.len())).unwrap() {
        assert!(!run.style.contains(Style::HIDDEN));
        assert!(!run.style.contains(Style::SOFT_BREAK));
        assert!(!run.style.contains(Style::HARD_BREAK));
        assert_eq!(run.replacement, None);
        assert_eq!(run.text, &markdown[run.source.start..run.source.end]);
    }
}

#[test]
fn pinned_commonmark_textual_content_report() {
    assert_eq!(TEXTUAL_CONTENT_CASES.len(), 3);
    for (index, &(number, markdown, expected)) in TEXTUAL_CONTENT_CASES.iter().enumerate() {
        assert_eq!(number, 650 + index);
        check_case(number, markdown, expected, &parse(markdown));
        eprintln!("example {number}: supported — matches upstream and verbatim source mapping");
    }
    eprintln!(
        "CommonMark 0.31.2 Textual content (650–652): 3 supported, 0 deliberate difference, 0 unsupported; other sections not scored by this test"
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
fn pinned_commonmark_textual_content_every_split_and_scalar_prefix() {
    let mut splits = 0;
    let mut prefixes = 0;
    for &(number, markdown, expected) in TEXTUAL_CONTENT_CASES {
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
    assert_eq!((splits, prefixes), (48, 45));
    eprintln!(
        "3 examples: 48 single splits and 45 scalar prefixes preserve semantics and source metadata"
    );
}

#[test]
fn textual_content_projection_preserves_whitespace_and_break_distinctions() {
    for (source, expected) in [
        ("a\nb\n", "<p>a\nb</p>\n"),
        ("a\n\nb\n", "<p>a</p>\n<p>b</p>\n"),
        ("a  \nb\n", "<p>a<br />\nb</p>\n"),
        ("a\\\nb\n", "<p>a<br />\nb</p>\n"),
        ("a  b\n", "<p>a  b</p>\n"),
        ("a\tb\n", "<p>a\tb</p>\n"),
        ("χρῆν e\u{301} é\n", "<p>χρῆν e\u{301} é</p>\n"),
        ("a     b\n", "<p>a     b</p>\n"),
        ("a &#32;\n", "<p>a  </p>\n"),
        ("a &#32;\nb\n", "<p>a  \nb</p>\n"),
        ("<&\"\n", "<p>&lt;&amp;&quot;</p>\n"),
    ] {
        assert_eq!(project(&parse(source)), expected, "input {source:?}");
    }
}

#[test]
fn textual_content_projection_rejects_unscored_structure_and_styles() {
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

#[test]
fn pinned_commonmark_textual_content_every_byte_split_and_prefix() {
    let mut splits = 0;
    let mut prefixes = 0;
    for &(number, markdown, expected) in TEXTUAL_CONTENT_CASES {
        let whole = parse(markdown);
        let bytes = markdown.as_bytes();
        for split in 0..=bytes.len() {
            let mut stream = Utf8Document::new();
            stream.append(&bytes[..split]).unwrap();
            stream.append(&bytes[split..]).unwrap();
            stream.finish();
            assert!(stream.pending_bytes().is_empty());
            assert_eq!(
                snapshot(stream.document()),
                snapshot(&whole),
                "example {number}, byte split {split}"
            );
            check_case(number, markdown, expected, stream.document());
            splits += 1;
        }
        let mut stream = Utf8Document::new();
        for end in 1..=bytes.len() {
            stream.append(&bytes[end - 1..end]).unwrap();
            let mut boundary = end;
            while !markdown.is_char_boundary(boundary) {
                boundary -= 1;
            }
            assert_eq!(stream.pending_bytes(), &bytes[boundary..end]);
            assert_eq!(stream.document().source(), &markdown[..boundary]);
            let mut prefix = Document::new();
            prefix.append(&markdown[..boundary]).unwrap();
            assert_eq!(
                snapshot(stream.document()),
                snapshot(&prefix),
                "example {number}, byte prefix {end}"
            );
            prefixes += 1;
        }
        stream.finish();
        check_case(number, markdown, expected, stream.document());
        assert_eq!(snapshot(stream.document()), snapshot(&whole));
    }
    assert_eq!((splits, prefixes), (53, 50));
    eprintln!("3 examples: 53 byte splits and 50 byte prefixes preserve UTF-8 and source metadata");
}
