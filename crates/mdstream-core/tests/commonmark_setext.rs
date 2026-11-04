//! Offline scoring of the complete CommonMark 0.31.2 Setext section (80–106).
//! The narrow projection preserves whitespace, block boundaries, heading levels,
//! emphasis, and the quote/list structure exercised here. It is not an HTML
//! renderer. It neither reparses Markdown nor receives the example number.
use mdstream::{
    Block, BlockId, BlockKind, Container, ContainerId, ContainerKind, Document, Heading,
    ListMarker, ListTightness, Span, Style,
};

include!("fixtures/commonmark_setext.rs");

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
    // Only one physical block terminator is structural. Never trim spaces,
    // tabs, internal breaks, or code LFs, even for a known unsupported example.
    if !code && chars.last().is_some_and(|(ch, _)| *ch == '\n') {
        chars.pop();
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

fn paragraph(kind: &BlockKind) -> bool {
    matches!(
        kind,
        BlockKind::Paragraph | BlockKind::Quote { .. } | BlockKind::ListItem { .. }
    )
}

fn continued_line(doc: &Document, source: Span) -> bool {
    doc.display_runs_in(Span::new(source.end - 1, source.end))
        .unwrap()
        .any(|run| {
            run.text == "\n"
                && (run.style.contains(Style::SOFT_BREAK) || run.style.contains(Style::HARD_BREAK))
        })
}

fn close_container(doc: &Document, id: ContainerId, html: &mut String) {
    html.push_str(match doc.container(id).unwrap().kind {
        ContainerKind::Quote => "</blockquote>\n",
        ContainerKind::List { .. } => "</ul>\n",
        ContainerKind::Item { .. } => "</li>\n",
    });
}

fn project(doc: &Document) -> String {
    assert!(doc.links().is_empty());
    assert!(doc.maths().is_empty());
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
            // These samples contain root quotes and root bullet lists with
            // plain, tight items. Fail loudly on unscored container semantics.
            match node.kind {
                ContainerKind::Quote => {
                    assert!(node.parent.is_none());
                    html.push_str("<blockquote>\n");
                }
                ContainerKind::List {
                    marker: ListMarker::Bullet(_),
                } => {
                    assert!(node.parent.is_none());
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
                other => panic!("container outside Setext projection: {other:?}"),
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
                let tight_item = block.container.is_some_and(|id| {
                    matches!(doc.container(id).unwrap().kind, ContainerKind::Item { .. })
                });
                if tight_item {
                    (String::new(), String::new(), false)
                } else {
                    ("<p>".to_owned(), "</p>\n".to_owned(), false)
                }
            }
            BlockKind::Heading { level } => {
                let heading = headings.next().expect("missing indexed heading");
                assert_eq!(heading.block, BlockId(index));
                assert_eq!(heading.level, level);
                assert_eq!(heading.source, block.source);
                assert_eq!(heading.container, block.container);
                (format!("<h{level}>"), format!("</h{level}>\n"), false)
            }
            BlockKind::Code { info: None } => {
                ("<pre><code>".to_owned(), "</code></pre>\n".to_owned(), true)
            }
            BlockKind::Blank => {
                index += 1;
                continue;
            }
            BlockKind::ThematicBreak => {
                html.push_str("<hr />\n");
                index += 1;
                continue;
            }
            ref other => panic!("block outside Setext projection: {other:?}"),
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

// Current observations are separate from the verbatim upstream oracle. Both a
// regression and an improvement require review, never automatic reclassification.
fn assessment(number: usize) -> Assessment {
    match number {
        80..=84 | 86 | 88..=106 => Assessment::Supported,
        85 => Assessment::DeliberateDifference {
            reason: "blank lines terminate indented code in the documented dialect (README: container code blocks)",
            observed: "<pre><code>Foo\n---\n</code></pre>\n<pre><code>Foo\n</code></pre>\n<hr />\n",
        },
        87 => Assessment::DeliberateDifference {
            reason: "indented code interrupts paragraphs in the documented dialect (also ATX example 70)",
            observed: "<p>Foo</p>\n<pre><code>---\n</code></pre>\n",
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
                "example {number} now matches upstream; review classification"
            );
            assert_eq!(
                actual, observed,
                "example {number} changed; input {markdown:?}; upstream remains {expected:?}"
            );
        }
    }
}

#[test]
fn pinned_commonmark_setext_report() {
    assert_eq!(SETEXT_CASES.len(), 27);
    let mut counts = [0; 3];
    for (index, &(number, markdown, expected)) in SETEXT_CASES.iter().enumerate() {
        assert_eq!(number, 80 + index);
        let actual = project(&parse(markdown));
        check_case(number, markdown, expected, &actual);
        let (category, label, reason) = match assessment(number) {
            Assessment::Supported => (0, "supported", "matches upstream"),
            Assessment::DeliberateDifference { reason, .. } => (1, "deliberate difference", reason),
        };
        counts[category] += 1;
        eprintln!("example {number}: {label} — {reason}");
    }
    assert_eq!(counts, [25, 2, 0]);
    eprintln!(
        "CommonMark 0.31.2 Setext headings (80–106): {} supported, {} deliberate differences, {} unsupported; other sections not scored by this test",
        counts[0], counts[1], counts[2]
    );
}

#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    blocks: Vec<Block>,
    headings: Vec<Heading>,
    containers: Vec<Container>,
    display: Vec<(Span, Option<Span>, String, Style)>,
    pending: Span,
}

fn snapshot(doc: &Document) -> Snapshot {
    Snapshot {
        blocks: doc.blocks().to_vec(),
        headings: doc.headings().entries().to_vec(),
        containers: (0..doc.container_count())
            .map(|i| doc.container(ContainerId(i)).unwrap())
            .collect(),
        display: doc
            .display_runs_in(Span::new(0, doc.source().len()))
            .unwrap()
            .map(|r| (r.source, r.replacement, r.text.to_owned(), r.style))
            .collect(),
        pending: doc.pending(),
    }
}

#[test]
fn pinned_commonmark_setext_every_split_and_scalar_prefix() {
    let mut splits = 0;
    for &(number, markdown, expected) in SETEXT_CASES {
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
        }
        doc.finish();
        check_case(number, markdown, expected, &project(&doc));
        assert_eq!(
            snapshot(&doc),
            snapshot(&whole),
            "example {number}, scalar stream"
        );
    }
    assert_eq!(splits, 533);
    eprintln!(
        "27 examples: 533 single splits and 506 scalar prefixes preserve classified semantics and source metadata"
    );
}

#[test]
fn setext_projection_preserves_whitespace_styles_and_block_scope() {
    assert_eq!(project(&parse("a\nb\n===\n")), "<h1>a\nb</h1>\n");
    assert_eq!(project(&parse("  a\t\n===\n")), "<h1>a</h1>\n");
    assert_eq!(project(&parse("a  \n---\n")), "<h2>a</h2>\n");
    assert_eq!(
        project(&parse("*a* & < \\\"\n===\n")),
        "<h1><em>a</em> &amp; &lt; &quot;</h1>\n"
    );
    assert_ne!(project(&parse("*a*\n===\n")), project(&parse("a\n===\n")));
    assert_ne!(project(&parse("a\n===\n")), project(&parse("a\n---\n")));
    assert_eq!(
        project(&parse("> a\nb\n===\n")),
        "<blockquote>\n<p>a\nb\n===</p>\n</blockquote>\n"
    );
    assert_eq!(
        project(&parse("> a\n\n> b\n")),
        "<blockquote>\n<p>a</p>\n</blockquote>\n<blockquote>\n<p>b</p>\n</blockquote>\n"
    );
    assert_eq!(
        project(&parse("- a\n- b\n")),
        "<ul>\n<li>a</li>\n<li>b</li>\n</ul>\n"
    );
    assert_eq!(project(&parse("  a\t\n")), "<p>a</p>\n");
    assert_eq!(project(&parse("a\tb\n")), "<p>a\tb</p>\n");
    assert_eq!(project(&parse("a\n\nb\n")), "<p>a</p>\n<p>b</p>\n");
    assert_eq!(
        project(&parse("    a  \n\n    b\n")),
        "<pre><code>a  \n</code></pre>\n<pre><code>b\n</code></pre>\n"
    );
    assert_eq!(project(&parse("a  \nb\n")), "<p>a<br />\nb</p>\n");
}
