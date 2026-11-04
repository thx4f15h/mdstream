//! Offline scoring of the complete CommonMark 0.31.2 Thematic breaks section.
//! This narrow adapter reads public blocks, containers, headings and display
//! runs. It never reparses Markdown or receives the example number.
use mdstream::{
    BlockId, BlockKind, ContainerId, ContainerKind, Document, ListMarker, ListTightness, Span,
    Style,
};

#[path = "support/snapshot.rs"]
mod support;
use support::snapshot;

include!("fixtures/commonmark_thematic_breaks.rs");

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

fn inline(doc: &Document, source: Span, code: bool) -> String {
    let mut chars = Vec::new();
    for run in doc.display_runs_in(source).unwrap() {
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
    // Only the physical prose/heading terminator is structural. Keep code
    // LFs, internal breaks and all visible spaces/tabs; never trim content.
    if !code && chars.last().is_some_and(|(ch, _)| *ch == '\n') {
        let (_, style) = chars.pop().unwrap();
        assert!(!style.contains(Style::HARD_BREAK));
        assert!(!style.contains(Style::SOFT_BREAK));
    }
    let mut html = String::new();
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
    html
}

fn close_container(doc: &Document, id: ContainerId, html: &mut String) {
    html.push_str(match doc.container(id).unwrap().kind {
        ContainerKind::List { .. } => "</ul>\n",
        ContainerKind::Item { .. } => "</li>\n",
        other => panic!("container outside thematic break projection: {other:?}"),
    });
}

fn project(doc: &Document) -> String {
    assert!(doc.links().is_empty());
    assert!(doc.maths().is_empty());
    assert!(doc.tables().is_empty());
    let mut html = String::new();
    let mut stack = Vec::new();
    let mut seen = vec![false; doc.container_count()];
    let mut items = vec![false; doc.container_count()];
    let mut headings = doc.headings().entries().iter();
    for (index, block) in doc.blocks().iter().enumerate() {
        let mut path = Vec::new();
        let mut parent = block.container;
        while let Some(id) = parent {
            path.push(id);
            parent = doc.container(id).unwrap().parent;
        }
        path.reverse();
        let shared = stack.iter().zip(&path).take_while(|(a, b)| a == b).count();
        while stack.len() > shared {
            close_container(doc, stack.pop().unwrap(), &mut html);
        }
        for &id in &path[shared..] {
            let node = doc.container(id).unwrap();
            assert!(!seen[id.0], "reopened sealed container");
            seen[id.0] = true;
            // Score tight bullet lists, including nested lists. Do not flatten
            // ancestry or infer thematic breaks from their source spelling.
            match node.kind {
                ContainerKind::List {
                    marker: ListMarker::Bullet(_),
                } => {
                    if let Some(parent) = node.parent {
                        assert!(matches!(
                            doc.container(parent).unwrap().kind,
                            ContainerKind::Item { .. }
                        ));
                        html.push('\n');
                    }
                    assert_eq!(node.list_tightness, Some(ListTightness::Tight));
                    html.push_str("<ul>\n");
                }
                ContainerKind::Item {
                    number: None,
                    task: None,
                } => {
                    assert!(matches!(
                        doc.container(node.parent.unwrap()).unwrap().kind,
                        ContainerKind::List {
                            marker: ListMarker::Bullet(_)
                        }
                    ));
                    html.push_str("<li>");
                }
                other => panic!("container outside thematic break projection: {other:?}"),
            }
            stack.push(id);
        }
        if let Some(id) = block.container {
            assert!(matches!(
                doc.container(id).unwrap().kind,
                ContainerKind::Item { .. }
            ));
            assert!(!items[id.0], "unscored multi-leaf item");
            items[id.0] = true;
            assert!(matches!(
                block.kind,
                BlockKind::ListItem { .. } | BlockKind::ThematicBreak
            ));
        }
        let (open, close, code) = match block.kind {
            BlockKind::Paragraph => ("<p>".to_owned(), "</p>\n".to_owned(), false),
            BlockKind::ListItem {
                number: None,
                task: None,
                ..
            } => {
                assert!(block.container.is_some());
                (String::new(), String::new(), false)
            }
            BlockKind::Heading { level } => {
                let heading = headings.next().expect("missing indexed heading");
                assert_eq!(heading.block, BlockId(index));
                assert_eq!(heading.source, block.source);
                assert_eq!(heading.level, level);
                assert_eq!(heading.container, None);
                (format!("<h{level}>"), format!("</h{level}>\n"), false)
            }
            BlockKind::Code { info: None } => {
                ("<pre><code>".to_owned(), "</code></pre>\n".to_owned(), true)
            }
            BlockKind::Blank => continue,
            BlockKind::ThematicBreak => {
                if block.container.is_some() {
                    html.push('\n');
                }
                html.push_str("<hr />\n");
                continue;
            }
            ref other => panic!("block outside thematic break projection: {other:?}"),
        };
        html.push_str(&open);
        html.push_str(&inline(doc, block.source, code));
        html.push_str(&close);
    }
    while let Some(id) = stack.pop() {
        close_container(doc, id, &mut html);
    }
    assert!(seen.into_iter().all(|seen| seen), "unscored container");
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

// Observations are kept separate from the verbatim upstream oracle. A change
// in either direction must trigger review, never automatic reclassification.
fn assessment(number: usize) -> Assessment {
    match number {
        43..=48 | 50..=61 => Assessment::Supported,
        49 => Assessment::DeliberateDifference {
            reason: "indented code interrupts paragraphs in the documented dialect (also Paragraphs 223)",
            observed: "<p>Foo</p>\n<pre><code>***\n</code></pre>\n",
        },
        _ => panic!("unclassified upstream example {number}"),
    }
}

fn check_case(number: usize, markdown: &str, expected: &str, doc: &Document) {
    let actual = project(doc);
    match assessment(number) {
        Assessment::Supported => assert_eq!(actual, expected, "example {number}"),
        Assessment::DeliberateDifference { observed, .. } => {
            assert_ne!(
                actual, expected,
                "example {number} now matches upstream; review classification"
            );
            assert_eq!(
                actual, observed,
                "example {number} changed; upstream remains {expected:?}"
            );
        }
    }
    assert_eq!(doc.source(), markdown);
    // These inputs need no replacement text. Hidden punctuation and container
    // prefixes remain addressable, with every display span borrowing raw source.
    for run in doc.display_runs_in(Span::new(0, markdown.len())).unwrap() {
        assert_eq!(run.replacement, None);
        assert_eq!(run.text, &markdown[run.source.start..run.source.end]);
    }
}

#[test]
fn pinned_commonmark_thematic_breaks_report() {
    assert_eq!(THEMATIC_BREAK_CASES.len(), 19);
    let mut counts = [0; 3];
    for (index, &(number, markdown, expected)) in THEMATIC_BREAK_CASES.iter().enumerate() {
        assert_eq!(number, 43 + index);
        check_case(number, markdown, expected, &parse(markdown));
        let (category, label, reason) = match assessment(number) {
            Assessment::Supported => (0, "supported", "matches upstream"),
            Assessment::DeliberateDifference { reason, .. } => (1, "deliberate difference", reason),
        };
        counts[category] += 1;
        eprintln!("example {number}: {label} — {reason}");
    }
    assert_eq!(counts, [18, 1, 0]);
    eprintln!(
        "CommonMark 0.31.2 Thematic breaks (43–61): 18 supported, 1 deliberate difference, 0 unsupported; other sections not scored by this test"
    );
}

#[test]
fn pinned_commonmark_thematic_breaks_every_split_and_scalar_prefix() {
    let mut splits = 0;
    let mut prefixes = 0;
    for &(number, markdown, expected) in THEMATIC_BREAK_CASES {
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
            assert_eq!(doc.pending(), prefix.pending());
            prefixes += 1;
        }
        doc.finish();
        check_case(number, markdown, expected, &doc);
        assert_eq!(snapshot(&doc), snapshot(&whole));
    }
    assert_eq!((splits, prefixes), (290, 271));
    eprintln!(
        "19 examples: 290 single splits and 271 scalar prefixes preserve classified semantics and source metadata"
    );
}

#[test]
fn thematic_breaks_projection_preserves_structure_styles_and_whitespace() {
    for (source, expected) in [
        ("***\n", "<hr />\n"),
        ("**\n", "<p>**</p>\n"),
        ("a\n***\nb\n", "<p>a</p>\n<hr />\n<p>b</p>\n"),
        ("a\n---\nb\n", "<h2>a</h2>\n<p>b</p>\n"),
        ("a\n\n---\n", "<p>a</p>\n<hr />\n"),
        ("    ***  \n", "<pre><code>***  \n</code></pre>\n"),
        (" *-*\n", "<p><em>-</em></p>\n"),
        ("a  b\n", "<p>a  b</p>\n"),
        ("a\tb\n", "<p>a\tb</p>\n"),
        ("a &#32;\n", "<p>a  </p>\n"),
        ("a\nb\n", "<p>a\nb</p>\n"),
        ("a  \nb\n", "<p>a<br />\nb</p>\n"),
        ("<&\"\n", "<p>&lt;&amp;&quot;</p>\n"),
        ("- a\n- b\n", "<ul>\n<li>a</li>\n<li>b</li>\n</ul>\n"),
        (
            "- a\n***\n- b\n",
            "<ul>\n<li>a</li>\n</ul>\n<hr />\n<ul>\n<li>b</li>\n</ul>\n",
        ),
        (
            "- - a\n",
            "<ul>\n<li>\n<ul>\n<li>a</li>\n</ul>\n</li>\n</ul>\n",
        ),
    ] {
        assert_eq!(project(&parse(source)), expected, "input {source:?}");
    }
}

#[test]
fn thematic_breaks_projection_rejects_unscored_structure_and_styles() {
    for source in [
        "> quote\n",
        "1. ordered\n",
        "- [x] task\n",
        "- a\n\n- b\n",
        "- a\n  continuation\n",
        "- # heading\n",
        "```rust\ncode\n```\n",
        "a | b\n--|--\n",
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
