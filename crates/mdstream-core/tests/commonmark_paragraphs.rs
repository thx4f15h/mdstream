//! Offline semantic scoring of the complete CommonMark 0.31.2 Paragraphs section.
//! This narrow adapter reads public blocks/display runs, never reparses Markdown
//! or receives an example number. Only structural blanks and one terminal LF
//! in a paragraph are omitted; indentation, trailing spaces and code LFs survive.
use mdstream::{Block, BlockKind, Document, Span, Style};

include!("fixtures/commonmark_paragraphs.rs");

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
    // The selected upstream examples have no containers or inline objects.
    // Reject additions outside the adapter's scope instead of silently ignoring them.
    assert_eq!(doc.container_count(), 0);
    assert!(doc.links().is_empty());
    assert!(doc.maths().is_empty());
    let mut html = String::new();
    assert!(doc.headings().entries().is_empty());
    for block in doc.blocks() {
        let (open, close, code) = match block.kind {
            BlockKind::Paragraph => ("<p>".to_owned(), "</p>\n".to_owned(), false),
            BlockKind::Code { info: None } => {
                ("<pre><code>".to_owned(), "</code></pre>\n".to_owned(), true)
            }
            BlockKind::Blank => continue,
            ref other => panic!("block outside paragraph projection: {other:?}"),
        };
        let mut chars = Vec::new();
        for run in doc.display_runs_in(block.source).unwrap() {
            if run.style.contains(Style::HIDDEN) {
                continue;
            }
            assert!(run.link.is_none());
            for unsupported in [Style::STRONG, Style::STRIKE, Style::MATH] {
                assert!(!run.style.contains(unsupported), "unscored inline style");
            }
            assert!(code || !run.style.contains(Style::CODE));
            chars.extend(run.text.chars().map(|ch| (ch, run.style)));
        }
        // Remove only the physical block terminator, never a space or an
        // internal line break. Code terminators remain content.
        if !code && chars.last().is_some_and(|(ch, _)| *ch == '\n') {
            chars.pop();
        }
        html.push_str(&open);
        let mut emphasis = false;
        for (ch, style) in chars {
            let next = style.contains(Style::EMPHASIS);
            if next != emphasis {
                html.push_str(if next { "<em>" } else { "</em>" });
                emphasis = next;
            }
            if ch == '\n' && style.contains(Style::HARD_BREAK) && !code {
                html.push_str("<br />");
            }
            escape(ch, &mut html);
        }
        if emphasis {
            html.push_str("</em>");
        }
        html.push_str(&close);
    }
    html
}

enum Assessment {
    Supported,
    DeliberateDifference {
        reason: &'static str,
        observed: &'static str,
    },
}

// Current observations are separate from the verbatim upstream oracle.
// Improvements and regressions both require explicit reclassification.
fn assessment(number: usize) -> Assessment {
    match number {
        219..=222 | 224..=226 => Assessment::Supported,
        223 => Assessment::DeliberateDifference {
            reason: "indented code interrupts paragraphs in the documented dialect (README: container code blocks)",
            observed: "<p>aaa</p>\n<pre><code>         bbb\n                                   ccc\n</code></pre>\n",
        },
        _ => panic!("unclassified upstream example {number}"),
    }
}

fn check_case(number: usize, markdown: &str, expected: &str, actual: &str) {
    match assessment(number) {
        Assessment::Supported => {
            assert_eq!(actual, expected, "example {number}, input {markdown:?}")
        }
        Assessment::DeliberateDifference { observed, .. } => {
            assert_ne!(
                actual, expected,
                "example {number} now matches upstream; review classification"
            );
            assert_eq!(
                actual, observed,
                "example {number} changed; input {markdown:?}; upstream {expected:?}"
            );
        }
    }
}

#[test]
fn pinned_commonmark_paragraphs_report() {
    assert_eq!(PARAGRAPH_CASES.len(), 8);
    let mut counts = [0; 3];
    for (index, &(number, markdown, expected)) in PARAGRAPH_CASES.iter().enumerate() {
        assert_eq!(number, 219 + index);
        check_case(number, markdown, expected, &project(&parse(markdown)));
        let (category, label, reason) = match assessment(number) {
            Assessment::Supported => (0, "supported", "matches upstream"),
            Assessment::DeliberateDifference { reason, .. } => (1, "deliberate difference", reason),
        };
        counts[category] += 1;
        eprintln!("example {number}: {label} — {reason}");
    }
    assert_eq!(counts, [7, 1, 0]);
    eprintln!(
        "CommonMark 0.31.2 Paragraphs (219–226): {} supported, {} deliberate difference, {} unsupported; other sections not scored by this test",
        counts[0], counts[1], counts[2]
    );
}

#[test]
fn pinned_commonmark_paragraphs_every_split_and_scalar_prefix() {
    let mut splits = 0;
    let mut prefixes = 0;
    for &(number, markdown, expected) in PARAGRAPH_CASES {
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
            check_case(number, markdown, expected, &project(&doc));
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
        check_case(number, markdown, expected, &project(&doc));
        assert_eq!(
            snapshot(&doc),
            snapshot(&whole),
            "example {number}, scalar stream"
        );
    }
    assert_eq!((splits, prefixes), (160, 152));
    eprintln!(
        "8 examples: 160 single splits and 152 scalar prefixes preserve classified semantics and source metadata"
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
fn paragraphs_projection_preserves_whitespace_breaks_and_block_boundaries() {
    assert_eq!(project(&parse("a\nb\n")), "<p>a\nb</p>\n");
    assert_eq!(project(&parse("a\n\nb\n")), "<p>a</p>\n<p>b</p>\n");
    assert_eq!(project(&parse("  a\t\n")), "<p>a</p>\n");
    assert_eq!(project(&parse("a\tb\n")), "<p>a\tb</p>\n");
    assert_eq!(project(&parse("a &#32;\n")), "<p>a  </p>\n");
    assert_eq!(project(&parse("a  b\n")), "<p>a  b</p>\n");
    assert_eq!(project(&parse("a  \nb\n")), "<p>a<br />\nb</p>\n");
    assert_eq!(
        project(&parse("    a  \n")),
        "<pre><code>a  \n</code></pre>\n"
    );
    // Exact same-scope blank continuation; historical split HTML is retained.
    let actual = project(&parse("    a\n\n    b\n"));
    assert_ne!(
        actual,
        "<pre><code>a\n</code></pre>\n<pre><code>b\n</code></pre>\n"
    );
    assert_eq!(actual, "<pre><code>a\n\nb\n</code></pre>\n");
    assert_eq!(
        project(&parse("*a* & <\n")),
        "<p><em>a</em> &amp; &lt;</p>\n"
    );
}

#[test]
fn paragraphs_projection_rejects_unscored_structure_and_styles() {
    for source in [
        "# heading\n",
        "> quote\n",
        "- item\n",
        "---\n",
        "**strong**\n",
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
