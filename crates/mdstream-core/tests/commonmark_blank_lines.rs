//! Offline scoring of the complete CommonMark 0.31.2 Blank lines section.
//! The adapter uses public blocks, heading metadata and display runs only.
use mdstream::{BlockId, BlockKind, Document, Span, Style};

#[path = "support/snapshot.rs"]
mod support;
use support::snapshot;

include!("fixtures/commonmark_blank_lines.rs");

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn escape(ch: char, out: &mut String) {
    match ch {
        '&' => out.push_str("&amp;"),
        '<' => out.push_str("&lt;"),
        '>' => out.push_str("&gt;"),
        '"' => out.push_str("&quot;"),
        _ => out.push(ch),
    }
}

fn project(doc: &Document) -> String {
    // The selected upstream examples have no containers or link/math objects.
    // Reject additions outside the adapter's scope instead of silently ignoring them.
    assert_eq!(doc.container_count(), 0);
    assert!(doc.links().is_empty());
    assert!(doc.maths().is_empty());
    let mut html = String::new();
    let mut headings = doc.headings().entries().iter();
    for (index, block) in doc.blocks().iter().enumerate() {
        let (open, close) = match block.kind {
            BlockKind::Heading { level } => {
                let heading = headings.next().expect("missing indexed heading");
                assert_eq!(heading.block, BlockId(index));
                assert_eq!(heading.source, block.source);
                assert_eq!(heading.level, level);
                assert_eq!(heading.container, None);
                (format!("<h{level}>"), format!("</h{level}>\n"))
            }
            BlockKind::Paragraph => ("<p>".to_owned(), "</p>\n".to_owned()),
            BlockKind::Blank => continue,
            ref other => panic!("block outside blank line projection: {other:?}"),
        };
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
        // Remove only the physical block terminator, never a space or an
        // internal line break. No trim or Markdown parsing is performed.
        if chars.last().is_some_and(|(ch, _)| *ch == '\n') {
            let (_, style) = chars.pop().unwrap();
            assert!(!style.contains(Style::HARD_BREAK));
            assert!(!style.contains(Style::SOFT_BREAK));
        }
        html.push_str(&open);
        for (ch, style) in chars {
            if ch == '\n' && style.contains(Style::HARD_BREAK) {
                html.push_str("<br />");
            }
            escape(ch, &mut html);
        }
        html.push_str(&close);
    }
    assert!(headings.next().is_none(), "extra indexed heading");
    html
}

fn check_case(markdown: &str, expected: &str, doc: &Document) {
    assert_eq!(project(doc), expected, "input {markdown:?}");
    assert_eq!(doc.source(), markdown);
    // Blank source and hidden heading markers remain addressable. This case
    // has no replacements: every display span still borrows its exact source.
    for run in doc.display_runs_in(Span::new(0, markdown.len())).unwrap() {
        assert_eq!(run.replacement, None);
        assert_eq!(run.text, &markdown[run.source.start..run.source.end]);
    }
}

#[test]
fn pinned_commonmark_blank_lines_report() {
    assert_eq!(BLANK_LINE_CASES.len(), 1);
    let (number, markdown, expected) = BLANK_LINE_CASES[0];
    assert_eq!(number, 227);
    check_case(markdown, expected, &parse(markdown));
    eprintln!("example {number}: supported — matches upstream");
    eprintln!(
        "CommonMark 0.31.2 Blank lines (227): 1 supported, 0 deliberate difference, 0 unsupported; other sections not scored by this test"
    );
}

#[test]
fn pinned_commonmark_blank_lines_every_split_and_scalar_prefix() {
    let mut splits = 0;
    let mut prefixes = 0;
    for &(number, markdown, expected) in BLANK_LINE_CASES {
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
            assert_eq!(doc.pending(), whole.pending());
            check_case(markdown, expected, &doc);
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
            assert_eq!(doc.pending(), prefix.pending());
            prefixes += 1;
        }
        doc.finish();
        check_case(markdown, expected, &doc);
        assert_eq!(snapshot(&doc), snapshot(&whole));
    }
    assert_eq!((splits, prefixes), (23, 22));
    eprintln!(
        "1 example: 23 single splits and 22 scalar prefixes preserve semantics and source metadata"
    );
}

#[test]
fn blank_lines_projection_preserves_structure_whitespace_and_breaks() {
    for (source, expected) in [
        ("", ""),
        ("\n  \n\n", ""),
        ("a\nb\n", "<p>a\nb</p>\n"),
        ("a\n\nb\n", "<p>a</p>\n<p>b</p>\n"),
        ("a\n  \n\nb\n", "<p>a</p>\n<p>b</p>\n"),
        ("a  \nb\n", "<p>a<br />\nb</p>\n"),
        ("a  b\n", "<p>a  b</p>\n"),
        ("a\tb\n", "<p>a\tb</p>\n"),
        ("a &#32;\n", "<p>a  </p>\n"),
        ("\n## a  b\n\n", "<h2>a  b</h2>\n"),
        ("#\n\n", "<h1></h1>\n"),
        ("a\n\n# b\n\nc\n", "<p>a</p>\n<h1>b</h1>\n<p>c</p>\n"),
        ("<&\"\n", "<p>&lt;&amp;&quot;</p>\n"),
    ] {
        assert_eq!(project(&parse(source)), expected, "input {source:?}");
    }
}

#[test]
fn blank_lines_projection_rejects_unscored_structure_and_styles() {
    for source in [
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
