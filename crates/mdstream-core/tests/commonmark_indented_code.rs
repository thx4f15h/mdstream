//! Offline scoring of CommonMark 0.31.2 Indented code blocks (107–118).
//! The narrow projection preserves whitespace, block boundaries, heading levels,
//! code literals and the tight/loose list structure exercised here. It is not an HTML
//! renderer. It neither reparses Markdown nor receives the example number.
use mdstream::{
    BlockId, BlockKind, ContainerId, ContainerKind, Document, ListMarker, ListTightness, Span,
    Style,
};

#[path = "support/snapshot.rs"]
mod support;
use support::snapshot;

include!("fixtures/commonmark_indented_code.rs");

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
        for unsupported in [Style::EMPHASIS, Style::STRONG, Style::STRIKE, Style::MATH] {
            assert!(!run.style.contains(unsupported), "unscored inline style");
        }
        assert!(code || !run.style.contains(Style::CODE));
        chars.extend(run.text.chars().map(|ch| (ch, run.style)));
    }
    // Only one physical block terminator is structural. Never trim spaces,
    // tabs, internal breaks, or code LFs, even for a known unsupported example.
    if !code && chars.last().is_some_and(|(ch, _)| *ch == '\n') {
        let (_, style) = chars.pop().unwrap();
        assert!(!style.contains(Style::HARD_BREAK));
        assert!(!style.contains(Style::SOFT_BREAK));
    }
    let mut html = String::new();
    for (ch, style) in chars {
        if ch == '\n' && style.contains(Style::HARD_BREAK) && !code {
            html.push_str("<br />");
        }
        escape(ch, &mut html);
    }
    html
}

fn paragraph(kind: &BlockKind) -> bool {
    matches!(kind, BlockKind::Paragraph | BlockKind::ListItem { .. })
}

fn continued_line(doc: &Document, source: Span) -> bool {
    doc.display_runs_in(Span::new(source.end - 1, source.end))
        .unwrap()
        .any(|run| {
            run.text == "\n"
                && (run.style.contains(Style::SOFT_BREAK) || run.style.contains(Style::HARD_BREAK))
        })
}

fn tight_item(doc: &Document, id: ContainerId) -> bool {
    let item = doc.container(id).unwrap();
    assert!(matches!(item.kind, ContainerKind::Item { .. }));
    doc.container(item.parent.unwrap()).unwrap().list_tightness == Some(ListTightness::Tight)
}

fn close_container(doc: &Document, id: ContainerId, html: &mut String) {
    html.push_str(match doc.container(id).unwrap().kind {
        ContainerKind::List {
            marker: ListMarker::Bullet(_),
        } => "</ul>\n",
        ContainerKind::List {
            marker: ListMarker::Ordered(_),
        } => "</ol>\n",
        ContainerKind::Item { .. } => "</li>\n",
        other => panic!("container outside indented code projection: {other:?}"),
    });
}

fn project(doc: &Document) -> String {
    assert!(doc.links().is_empty());
    assert!(doc.maths().is_empty());
    assert!(doc.tables().is_empty());
    let mut html = String::new();
    let mut stack = Vec::new();
    let mut seen = vec![false; doc.container_count()];
    let mut headings = doc.headings().entries().iter();
    let mut index = 0;
    while let Some(block) = doc.blocks().get(index) {
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
            // Preserve list nesting and published tight/loose metadata.
            // This chapter needs bullet lists and ordered lists starting at 1.
            match node.kind {
                ContainerKind::List { marker } => {
                    if let Some(parent) = node.parent
                        && tight_item(doc, parent)
                    {
                        html.push('\n');
                    }
                    assert!(node.list_tightness.is_some());
                    html.push_str(match marker {
                        ListMarker::Bullet(_) => "<ul>\n",
                        ListMarker::Ordered(_) => "<ol>\n",
                    });
                }
                ContainerKind::Item { number, task: None } => {
                    let list = doc.container(node.parent.unwrap()).unwrap();
                    match list.kind {
                        ContainerKind::List {
                            marker: ListMarker::Bullet(_),
                        } => assert_eq!(number, None),
                        ContainerKind::List {
                            marker: ListMarker::Ordered(_),
                        } => assert_eq!(number, Some(1), "unscored ordered item number"),
                        other => panic!("invalid item parent: {other:?}"),
                    }
                    html.push_str("<li>");
                    if !tight_item(doc, id) {
                        html.push('\n');
                    }
                }
                other => panic!("container outside indented code projection: {other:?}"),
            }
            stack.push(id);
        }
        let mut source = block.source;
        let (open, close, code) = match block.kind {
            ref kind if paragraph(kind) => {
                // Container prose has one leaf per physical line. Coalesce
                // only adjacent prose in the same scope with a published
                // continuation break; never infer a paragraph from raw text.
                if block.container.is_some() {
                    while let Some(next) = doc.blocks().get(index + 1) {
                        if next.container != block.container
                            || !paragraph(&next.kind)
                            || next.source.start != source.end
                            || !continued_line(doc, source)
                        {
                            break;
                        }
                        source.end = next.source.end;
                        index += 1;
                    }
                }
                let tight_item = block.container.is_some_and(|id| tight_item(doc, id));
                if tight_item {
                    (String::new(), String::new(), false)
                } else {
                    ("<p>".to_owned(), "</p>\n".to_owned(), false)
                }
            }
            BlockKind::Heading { level } => {
                assert!(block.container.is_none(), "unscored container heading");
                let heading = headings.next().expect("missing indexed heading");
                assert_eq!(heading.block, BlockId(index));
                assert_eq!(heading.level, level);
                assert_eq!(heading.source, block.source);
                assert_eq!(heading.container, block.container);
                (format!("<h{level}>"), format!("</h{level}>\n"), false)
            }
            BlockKind::Code { info: None } => {
                assert!(block.container.is_none(), "unscored container code");
                ("<pre><code>".to_owned(), "</code></pre>\n".to_owned(), true)
            }
            BlockKind::Blank => {
                index += 1;
                continue;
            }
            BlockKind::ThematicBreak => {
                assert!(block.container.is_none(), "unscored container rule");
                html.push_str("<hr />\n");
                index += 1;
                continue;
            }
            ref other => panic!("block outside indented code projection: {other:?}"),
        };
        html.push_str(&open);
        html.push_str(&inline(doc, source, code));
        html.push_str(&close);
        index += 1;
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

// Observations never replace the verbatim upstream oracle. Any change in
// either direction requires review instead of automatic reclassification.
// The original observed strings below remain immutable historical evidence.
// These exact official sources now join indented chunks across internal blanks;
// the unchanged projection must match the verbatim upstream HTML.
fn assessment(number: usize) -> Assessment {
    let previous = legacy_assessment(number);
    if matches!(number, 110..=112) {
        assert!(matches!(previous, Assessment::DeliberateDifference { .. }));
        Assessment::Supported
    } else {
        previous
    }
}

fn legacy_assessment(number: usize) -> Assessment {
    match number {
        107..=109 | 114..=118 => Assessment::Supported,
        110 => Assessment::DeliberateDifference {
            reason: "a blank line terminates indented code in the documented dialect",
            observed: "<pre><code>&lt;a/&gt;\n*hi*\n</code></pre>\n<pre><code>- one\n</code></pre>\n",
        },
        111 => Assessment::DeliberateDifference {
            reason: "blank lines split code chunks instead of remaining inside one block",
            observed: "<pre><code>chunk1\n</code></pre>\n<pre><code>chunk2\n</code></pre>\n<pre><code>chunk3\n</code></pre>\n",
        },
        112 => Assessment::DeliberateDifference {
            reason: "even a sufficiently indented blank line terminates code",
            observed: "<pre><code>chunk1\n</code></pre>\n<pre><code>  chunk2\n</code></pre>\n",
        },
        113 => Assessment::DeliberateDifference {
            reason: "indented code interrupts paragraphs in the documented dialect",
            observed: "<p>Foo</p>\n<pre><code>bar\n</code></pre>\n",
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
    // All chapter inputs use borrowed display text. Hidden indent/prefix bytes
    // remain addressable and code whitespace retains its exact source mapping.
    for run in doc.display_runs_in(Span::new(0, markdown.len())).unwrap() {
        assert_eq!(run.replacement, None);
        assert_eq!(run.text, &markdown[run.source.start..run.source.end]);
    }
}

#[test]
fn pinned_commonmark_indented_code_report() {
    assert_eq!(INDENTED_CODE_CASES.len(), 12);
    let mut counts = [0; 3];
    for (index, &(number, markdown, expected)) in INDENTED_CODE_CASES.iter().enumerate() {
        assert_eq!(number, 107 + index);
        check_case(number, markdown, expected, &parse(markdown));
        let (category, label, reason) = match assessment(number) {
            Assessment::Supported => (0, "supported", "matches upstream"),
            Assessment::DeliberateDifference { reason, .. } => (1, "deliberate difference", reason),
        };
        counts[category] += 1;
        eprintln!("example {number}: {label} — {reason}");
    }
    assert_eq!(counts, [11, 1, 0]);
    eprintln!(
        "CommonMark 0.31.2 Indented code blocks (107–118): 11 supported, 1 deliberate difference, 0 unsupported; other sections not scored by this test"
    );
}

#[test]
fn pinned_commonmark_indented_code_every_split_and_scalar_prefix() {
    let mut splits = 0;
    let mut prefixes = 0;
    for &(number, markdown, expected) in INDENTED_CODE_CASES {
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
    assert_eq!((splits, prefixes), (309, 297));
    eprintln!(
        "12 examples: 309 single splits and 297 scalar prefixes preserve classified semantics and source metadata"
    );
}

#[test]
fn indented_code_projection_preserves_literals_whitespace_and_structure() {
    for (source, expected) in [
        ("", ""),
        ("    a\n    b\n", "<pre><code>a\nb\n</code></pre>\n"),
        ("    a  \n", "<pre><code>a  \n</code></pre>\n"),
        ("     a\tb\n", "<pre><code> a\tb\n</code></pre>\n"),
        (
            "    a\n\n    b\n",
            "<pre><code>a\n</code></pre>\n<pre><code>b\n</code></pre>\n",
        ),
        ("    a", "<pre><code>a</code></pre>\n"),
        (
            "    *hi* `code` &amp; <a/> $x$\n",
            "<pre><code>*hi* `code` &amp;amp; &lt;a/&gt; $x$\n</code></pre>\n",
        ),
        ("a  b\n", "<p>a  b</p>\n"),
        ("a\tb\n", "<p>a\tb</p>\n"),
        ("a &#32;\n", "<p>a  </p>\n"),
        ("a\nb\n", "<p>a\nb</p>\n"),
        ("a  \nb\n", "<p>a<br />\nb</p>\n"),
        ("# a\n", "<h1>a</h1>\n"),
        ("a\n---\n", "<h2>a</h2>\n"),
        ("---\n", "<hr />\n"),
        ("- a\n- b\n", "<ul>\n<li>a</li>\n<li>b</li>\n</ul>\n"),
        (
            "- a\n\n- b\n",
            "<ul>\n<li>\n<p>a</p>\n</li>\n<li>\n<p>b</p>\n</li>\n</ul>\n",
        ),
        ("- a\n  b\n", "<ul>\n<li>a\nb</li>\n</ul>\n"),
        (
            "- a\n  - b\n",
            "<ul>\n<li>a\n<ul>\n<li>b</li>\n</ul>\n</li>\n</ul>\n",
        ),
        ("1. a\n", "<ol>\n<li>a</li>\n</ol>\n"),
    ] {
        if source == "    a\n\n    b\n" {
            assert_eq!(
                expected,
                "<pre><code>a\n</code></pre>\n<pre><code>b\n</code></pre>\n"
            );
            assert_eq!(
                project(&parse(source)),
                "<pre><code>a\n\nb\n</code></pre>\n"
            );
        } else {
            assert_eq!(project(&parse(source)), expected, "input {source:?}");
        }
    }
}

#[test]
fn indented_code_projection_rejects_unscored_structure_and_styles() {
    for source in [
        "> quote\n",
        "2. ordered\n",
        "- [x] task\n",
        "- # heading\n",
        "- a\n\n      code\n",
        "- ***\n",
        "```rust\ncode\n```\n",
        "a | b\n--|--\n",
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
