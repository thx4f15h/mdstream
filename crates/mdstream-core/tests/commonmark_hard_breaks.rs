//! Offline semantic scoring of the complete CommonMark 0.31.2 Hard line breaks section.
//! This narrow adapter reads public blocks/display runs, never reparses Markdown
//! or receives an example number. Only structural blanks and one terminal LF
//! in prose/headings are omitted; indentation, trailing spaces and code LFs survive.
use mdstream::{Block, BlockId, BlockKind, Document, Heading, Span, Style};

include!("fixtures/commonmark_hard_breaks.rs");

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
        let (open, close, code) = match block.kind {
            BlockKind::Heading { level } => {
                let heading = headings.next().expect("missing indexed heading");
                assert_eq!(heading.block, BlockId(index));
                assert_eq!(heading.source, block.source);
                assert_eq!(heading.level, level);
                assert_eq!(heading.container, None);
                (format!("<h{level}>"), format!("</h{level}>\n"), false)
            }
            BlockKind::Paragraph => ("<p>".to_owned(), "</p>\n".to_owned(), false),
            BlockKind::Code { info: None } => {
                ("<pre><code>".to_owned(), "</code></pre>\n".to_owned(), true)
            }
            BlockKind::Blank => continue,
            ref other => panic!("block outside hard break projection: {other:?}"),
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

            chars.extend(run.text.chars().map(|ch| (ch, run.style)));
        }
        // Remove only the physical block terminator, never a space or an
        // internal line break. Code terminators remain content.
        if !code && chars.last().is_some_and(|(ch, _)| *ch == '\n') {
            let (_, style) = chars.pop().unwrap();
            assert!(!style.contains(Style::HARD_BREAK));
            assert!(!style.contains(Style::SOFT_BREAK));
        }
        html.push_str(&open);
        let mut active_tag = None;
        for (ch, style) in chars {
            let emphasis = style.contains(Style::EMPHASIS);
            let code_span = !code && style.contains(Style::CODE);
            // This chapter never nests emphasis and code. Fail on expansion
            // rather than inventing a general inline nesting renderer.
            assert!(!emphasis || !code_span, "unscored nested inline styles");
            let next_tag = if emphasis {
                Some("em")
            } else if code_span {
                Some("code")
            } else {
                None
            };
            if active_tag != next_tag {
                if let Some(tag) = active_tag {
                    html.push_str(&format!("</{tag}>"));
                }
                if let Some(tag) = next_tag {
                    html.push_str(&format!("<{tag}>"));
                }
                active_tag = next_tag;
            }
            if ch == '\n' && style.contains(Style::HARD_BREAK) && !code && !code_span {
                html.push_str("<br />");
            }
            escape(ch, &mut html);
        }
        if let Some(tag) = active_tag {
            html.push_str(&format!("</{tag}>"));
        }
        html.push_str(&close);
    }
    assert!(headings.next().is_none(), "extra indexed heading");
    html
}

enum Assessment {
    Supported,
    DeliberateDifference {
        reason: &'static str,
        observed: &'static str,
    },
    Unsupported {
        reason: &'static str,
        observed: &'static str,
    },
}

// Current observations are separate from the verbatim upstream oracle.
// Improvements and regressions both require explicit reclassification.
fn assessment(number: usize) -> Assessment {
    match number {
        633..=635 | 638..=641 | 644..=647 => Assessment::Supported,
        636 => Assessment::DeliberateDifference {
            reason: "documented indented-code interruption dialect ends the paragraph before the hard break",
            observed: "<p>foo</p>\n<pre><code> bar\n</code></pre>\n",
        },
        637 => Assessment::DeliberateDifference {
            reason: "documented indented-code interruption dialect leaves a literal paragraph-final backslash",
            observed: "<p>foo\\</p>\n<pre><code> bar\n</code></pre>\n",
        },
        642 | 643 => Assessment::Unsupported {
            reason: "raw HTML is outside project requirements; its contents are parsed as ordinary inline text",
            observed: "<p>&lt;a href=&quot;foo<br />\nbar&quot;&gt;</p>\n",
        },
        _ => panic!("unclassified upstream example {number}"),
    }
}

fn check_case(number: usize, markdown: &str, expected: &str, actual: &str) {
    match assessment(number) {
        Assessment::Supported => {
            assert_eq!(actual, expected, "example {number}, input {markdown:?}")
        }
        Assessment::DeliberateDifference { observed, .. }
        | Assessment::Unsupported { observed, .. } => {
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
fn pinned_commonmark_hard_breaks_report() {
    assert_eq!(HARD_BREAK_CASES.len(), 15);
    let mut counts = [0; 3];
    for (index, &(number, markdown, expected)) in HARD_BREAK_CASES.iter().enumerate() {
        assert_eq!(number, 633 + index);
        check_case(number, markdown, expected, &project(&parse(markdown)));
        let (category, label, reason) = match assessment(number) {
            Assessment::Supported => (0, "supported", "matches upstream"),
            Assessment::DeliberateDifference { reason, .. } => (1, "deliberate difference", reason),
            Assessment::Unsupported { reason, .. } => (2, "unsupported", reason),
        };
        counts[category] += 1;
        eprintln!("example {number}: {label} — {reason}");
    }
    assert_eq!(counts, [11, 2, 2]);
    eprintln!(
        "CommonMark 0.31.2 Hard line breaks (633–647): {} supported, {} deliberate difference, {} unsupported; other sections not scored by this test",
        counts[0], counts[1], counts[2]
    );
}

#[test]
fn pinned_commonmark_hard_breaks_every_split_and_scalar_prefix() {
    let mut splits = 0;
    let mut prefixes = 0;
    for &(number, markdown, expected) in HARD_BREAK_CASES {
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
    assert_eq!((splits, prefixes), (199, 184));
    eprintln!(
        "15 examples: 199 single splits and 184 scalar prefixes preserve classified semantics and source metadata"
    );
}

#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    blocks: Vec<Block>,
    headings: Vec<Heading>,
    display: Vec<(Span, Option<Span>, String, Style)>,
    pending: Span,
}

fn snapshot(doc: &Document) -> Snapshot {
    assert_eq!(doc.container_count(), 0);
    assert!(doc.links().is_empty());
    assert!(doc.maths().is_empty());
    Snapshot {
        blocks: doc.blocks().to_vec(),
        headings: doc.headings().entries().to_vec(),
        display: doc
            .display_runs_in(Span::new(0, doc.source().len()))
            .unwrap()
            .map(|run| (run.source, run.replacement, run.text.to_owned(), run.style))
            .collect(),
        pending: doc.pending(),
    }
}

#[test]
fn hard_breaks_projection_preserves_breaks_styles_and_whitespace() {
    for (source, expected) in [
        ("a\nb\n", "<p>a\nb</p>\n"),
        ("a  \nb\n", "<p>a<br />\nb</p>\n"),
        ("a\\\nb\n", "<p>a<br />\nb</p>\n"),
        ("a  \n\nb\n", "<p>a</p>\n<p>b</p>\n"),
        ("a\\\n\nb\n", "<p>a\\</p>\n<p>b</p>\n"),
        ("a  b\n", "<p>a  b</p>\n"),
        ("a\tb\n", "<p>a\tb</p>\n"),
        ("*a  \nb*\n", "<p><em>a<br />\nb</em></p>\n"),
        ("`a  \nb`\n", "<p><code>a   b</code></p>\n"),
        ("`a\\\nb`\n", "<p><code>a\\ b</code></p>\n"),
        ("`a`*b*\n", "<p><code>a</code><em>b</em></p>\n"),
        ("*a*`b`\n", "<p><em>a</em><code>b</code></p>\n"),
        ("    a  \n", "<pre><code>a  \n</code></pre>\n"),
        ("### a\\\n", "<h3>a\\</h3>\n"),
        ("## a  \n", "<h2>a</h2>\n"),
        ("<a>\n", "<p>&lt;a&gt;</p>\n"),
        ("a &amp; &#32;\n", "<p>a &amp;  </p>\n"),
    ] {
        assert_eq!(project(&parse(source)), expected, "input {source:?}");
    }
}

#[test]
fn hard_breaks_projection_rejects_unscored_structure_and_styles() {
    for source in [
        "> quote\n",
        "- item\n",
        "---\n",
        "**strong**\n",
        "~~strike~~\n",
        "*a `b`*\n",
        "[link](url)\n",
        "$math$\n",
    ] {
        assert!(
            std::panic::catch_unwind(|| project(&parse(source))).is_err(),
            "accepted unscored input {source:?}"
        );
    }
}
