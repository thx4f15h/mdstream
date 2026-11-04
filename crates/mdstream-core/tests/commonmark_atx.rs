//! Narrow, offline semantic comparison for the entire CommonMark ATX section.
//! This adapter is not a library HTML renderer. It scores block kind/order,
//! heading level, visible text (including spaces), and emphasis in examples
//! 62–79. Blank source blocks and one terminal physical LF in a paragraph or
//! heading are structural; code LFs are content. No other whitespace is trimmed.
use mdstream::{BlockKind, Document, Style};

include!("fixtures/commonmark_atx.rs");

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
    let mut headings = doc.headings().entries().iter();
    for block in doc.blocks() {
        let (open, close, code) = match block.kind {
            BlockKind::Paragraph => ("<p>".to_owned(), "</p>\n".to_owned(), false),
            BlockKind::Heading { level } => {
                let heading = headings.next().expect("missing indexed heading");
                assert_eq!(heading.level, level);
                assert_eq!(heading.source, block.source);
                (format!("<h{level}>"), format!("</h{level}>\n"), false)
            }
            BlockKind::Code { info: None } => {
                ("<pre><code>".to_owned(), "</code></pre>\n".to_owned(), true)
            }
            BlockKind::Blank => continue,
            BlockKind::ThematicBreak => {
                html.push_str("<hr />\n");
                continue;
            }
            ref other => panic!("block outside ATX projection: {other:?}"),
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
        // internal line break. In particular, empty headings must survive.
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
    assert!(headings.next().is_none(), "extra indexed heading");
    html
}

#[derive(Clone, Copy)]
enum Assessment {
    Supported,
    DeliberateDifference {
        reason: &'static str,
        observed: &'static str,
    },
}

// These are CURRENT observations, not replacements for the upstream oracle.
// A changed observation (including a newly passing case) requires review and
// reclassification; neither mismatches nor improvements silently change scores.
fn assessment(number: usize) -> Assessment {
    match number {
        62..=69 | 71..=79 => Assessment::Supported,
        70 => Assessment::DeliberateDifference {
            reason: "indented code interrupts paragraphs in the existing dialect (README: container code blocks)",
            observed: "<p>foo</p>\n<pre><code># bar\n</code></pre>\n",
        },
        _ => panic!("unclassified upstream example {number}"),
    }
}

fn check_case(number: usize, markdown: &str, expected: &str, actual: &str) {
    match assessment(number) {
        Assessment::Supported => {
            assert_eq!(actual, expected, "example {number}, input {markdown:?}");
        }
        Assessment::DeliberateDifference { observed, .. } => {
            assert_ne!(
                actual, expected,
                "example {number} now agrees with upstream; review its classification"
            );
            assert_eq!(
                actual, observed,
                "example {number} changed; input {markdown:?}; upstream remains {expected:?}"
            );
        }
    }
}

#[test]
fn pinned_commonmark_atx_report() {
    assert_eq!(ATX_CASES.len(), 18);
    let mut counts = [0; 3];
    for (index, &(number, markdown, expected)) in ATX_CASES.iter().enumerate() {
        assert_eq!(number, 62 + index);
        let actual = project(&parse(markdown));
        check_case(number, markdown, expected, &actual);
        let (category, label, reason) = match assessment(number) {
            Assessment::Supported => (0, "supported", "matches upstream"),
            Assessment::DeliberateDifference { reason, .. } => (1, "deliberate difference", reason),
        };
        counts[category] += 1;
        eprintln!("example {number}: {label} — {reason}");
    }
    assert_eq!(counts, [17, 1, 0]);
    eprintln!(
        "CommonMark 0.31.2 ATX headings (62–79): {} supported, {} deliberate difference, {} unsupported; other sections not scored by this test",
        counts[0], counts[1], counts[2]
    );
}

#[test]
fn pinned_commonmark_atx_every_split_and_scalar_stream_matches_classified_oracle() {
    for &(number, markdown, expected) in ATX_CASES {
        for split in (0..=markdown.len()).filter(|&split| markdown.is_char_boundary(split)) {
            let mut doc = Document::new();
            doc.append(&markdown[..split]).unwrap();
            doc.append(&markdown[split..]).unwrap();
            doc.finish();
            check_case(number, markdown, expected, &project(&doc));
        }
        let mut doc = Document::new();
        for ch in markdown.chars() {
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
        }
        doc.finish();
        check_case(number, markdown, expected, &project(&doc));
    }
}

#[test]
fn atx_projection_preserves_text_styles_block_boundaries_and_empty_headings() {
    // Sensitivity checks for the adapter itself: it must not erase precisely
    // the semantics being scored (notably a whitespace or emphasis defect).
    assert_eq!(project(&parse("#\n\n##\n")), "<h1></h1>\n<h2></h2>\n");
    assert_eq!(project(&parse("text  \n")), "<p>text</p>\n");
    assert_eq!(project(&parse("text &#32;\n")), "<p>text  </p>\n");
    assert_eq!(project(&parse("a\nb\n")), "<p>a\nb</p>\n");
    assert_eq!(project(&parse("a\n\nb\n")), "<p>a</p>\n<p>b</p>\n");
    assert_eq!(
        project(&parse("*a* & < \\\"\n")),
        "<p><em>a</em> &amp; &lt; &quot;</p>\n"
    );
    assert_ne!(project(&parse("# *a*\n")), project(&parse("# a\n")));
    assert_ne!(project(&parse("# a\n")), project(&parse("## a\n")));
    assert_ne!(project(&parse("# a  b\n")), project(&parse("# a b\n")));
    assert_eq!(project(&parse("# a \n")), project(&parse("# a\n")));
    assert_eq!(
        project(&parse("    a  \n")),
        "<pre><code>a  \n</code></pre>\n"
    );
}
