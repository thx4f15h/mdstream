//! Offline scoring of CommonMark 0.31.2 List items (253–300).
//! The narrow projection preserves whitespace, block boundaries, ordered starts,
//! heading levels, code literals and the tight/loose list structure exercised here. It is not an HTML
//! renderer. It neither reparses Markdown nor receives the example number.
use mdstream::{
    BlockId, BlockKind, Budget, ContainerId, ContainerKind, DisplayStep, Document, ListMarker,
    ListTightness, Span, Style, WorkLimit,
};

#[path = "support/snapshot.rs"]
mod support;
use support::snapshot;

include!("fixtures/commonmark_list_items.rs");

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
    if !code
        && doc.text(Span::new(source.end.saturating_sub(1), source.end)) == Some("\n")
        && chars.last().is_some_and(|(ch, _)| *ch == '\n')
    {
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
    matches!(
        kind,
        BlockKind::Paragraph | BlockKind::ListItem { .. } | BlockKind::Quote { .. }
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

fn tight_item(doc: &Document, id: ContainerId) -> bool {
    let item = doc.container(id).unwrap();
    assert!(matches!(item.kind, ContainerKind::Item { .. }));
    doc.container(item.parent.unwrap()).unwrap().list_tightness == Some(ListTightness::Tight)
}

fn block_boundary(html: &mut String) {
    if !html.is_empty() && !html.ends_with('\n') {
        html.push('\n');
    }
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
        ContainerKind::Quote => "</blockquote>\n",
    });
}

fn project(doc: &Document) -> String {
    assert!(doc.links().is_empty());
    assert!(doc.maths().is_empty());
    assert!(doc.tables().is_empty());
    assert!(doc.footnotes().is_empty());
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
        for (depth, &id) in path.iter().enumerate().skip(shared) {
            let node = doc.container(id).unwrap();
            assert!(!seen[id.0], "reopened sealed container");
            seen[id.0] = true;
            // Only published container metadata determines grouping and start.
            match node.kind {
                ContainerKind::List { marker } => {
                    block_boundary(&mut html);
                    assert!(node.list_tightness.is_some());
                    match marker {
                        ListMarker::Bullet(_) => html.push_str("<ul>\n"),
                        ListMarker::Ordered(_) => {
                            let first = doc.container(path[depth + 1]).unwrap();
                            assert_eq!(first.parent, Some(id));
                            let ContainerKind::Item {
                                number: Some(number),
                                ..
                            } = first.kind
                            else {
                                panic!("ordered list without numbered first item");
                            };
                            if number == 1 {
                                html.push_str("<ol>\n");
                            } else {
                                html.push_str(&format!("<ol start=\"{number}\">\n"));
                            }
                        }
                    }
                }
                ContainerKind::Item { number, task: None } => {
                    let list = doc.container(node.parent.unwrap()).unwrap();
                    match list.kind {
                        ContainerKind::List {
                            marker: ListMarker::Bullet(_),
                        } => assert_eq!(number, None),
                        ContainerKind::List {
                            marker: ListMarker::Ordered(_),
                        } => assert!(number.is_some(), "missing ordered item number"),
                        other => panic!("invalid item parent: {other:?}"),
                    }
                    html.push_str("<li>");
                }
                ContainerKind::Quote => {
                    block_boundary(&mut html);
                    html.push_str("<blockquote>\n");
                }
                other => panic!("container outside list projection: {other:?}"),
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
                        && tight_item(doc, id)
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
                assert!(
                    block.container.is_none_or(|id| matches!(
                        doc.container(id).unwrap().kind,
                        ContainerKind::Quote | ContainerKind::Item { .. }
                    )),
                    "unscored container code"
                );
                ("<pre><code>".to_owned(), "</code></pre>\n".to_owned(), true)
            }
            BlockKind::Blank | BlockKind::Definition { .. } => {
                index += 1;
                continue;
            }
            ref other => panic!("block outside list projection: {other:?}"),
        };
        if !open.is_empty() {
            block_boundary(&mut html);
        }
        html.push_str(&open);
        html.push_str(&inline(doc, source, code));
        html.push_str(&close);
        index += 1;
    }
    while let Some(id) = stack.pop() {
        close_container(doc, id, &mut html);
    }
    assert!(headings.next().is_none(), "unscored heading");
    assert!(seen.into_iter().all(|seen| seen), "unscored container");
    html
}

// Independent observations, never passed to the projection or used as upstream
// expectations. A change or a newly matching example requires a score review.
enum Assessment {
    Supported,
    DeliberateDifference {
        reason: &'static str,
        observed: &'static str,
    },
}

// The original observed strings below remain immutable historical evidence.
// These exact official sources now join indented chunks across internal blanks;
// the unchanged projection must match the verbatim upstream HTML.
fn assessment(number: usize) -> Assessment {
    let previous = legacy_assessment(number);
    if matches!(number, 264 | 289) {
        assert!(matches!(previous, Assessment::DeliberateDifference { .. }));
        Assessment::Supported
    } else {
        previous
    }
}

fn legacy_assessment(number: usize) -> Assessment {
    match number {
        253..=263 | 265..=288 | 290 | 292..=300 => Assessment::Supported,
        264 => Assessment::DeliberateDifference {
            reason: "blank lines terminate indented code in the documented dialect",
            observed: "<ul>\n<li>\n<p>Foo</p>\n<pre><code>bar\n</code></pre>\n<pre><code>baz\n</code></pre>\n</li>\n</ul>\n",
        },
        289 => Assessment::DeliberateDifference {
            reason: "blank lines terminate indented code in the documented dialect",
            observed: "<pre><code>1.  A paragraph\n    with two lines.\n</code></pre>\n<pre><code>        indented code\n</code></pre>\n<pre><code>    &gt; A block quote.\n</code></pre>\n",
        },
        291 => Assessment::DeliberateDifference {
            reason: "lazy prose retains leading whitespace in the documented container dialect",
            observed: "<ol>\n<li>A paragraph\n    with two lines.</li>\n</ol>\n",
        },
        _ => panic!("unclassified upstream example {number}"),
    }
}

fn check_mapping(markdown: &str, source: Span, text: &str, replacement: Option<Span>) {
    // This entire chapter uses exact borrowed runs, including hidden prefixes.
    assert_eq!(replacement, None);
    assert_eq!(text, &markdown[source.range()]);
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
                "example {number}; upstream is {expected:?}"
            );
        }
    }
    assert_eq!(doc.source(), markdown);
    let mut end = 0;
    for run in doc.display_runs_in(Span::new(0, markdown.len())).unwrap() {
        assert_eq!(run.source.start, end);
        assert!(run.source.start < run.source.end);
        end = run.source.end;
        check_mapping(markdown, run.source, run.text, run.replacement);
    }
    assert_eq!(end, markdown.len());
    let raw: String = doc
        .runs_in(Span::new(0, markdown.len()))
        .unwrap()
        .map(|r| r.text)
        .collect();
    assert_eq!(raw, markdown);
}

#[test]
fn pinned_commonmark_list_items_report() {
    assert_eq!(LIST_ITEM_CASES.len(), 48);
    let mut counts = [0; 3];
    for (index, &(number, markdown, expected)) in LIST_ITEM_CASES.iter().enumerate() {
        assert_eq!(number, 253 + index);
        check_case(number, markdown, expected, &parse(markdown));
        let (category, label, reason) = match assessment(number) {
            Assessment::Supported => (0, "supported", "matches upstream"),
            Assessment::DeliberateDifference { reason, .. } => (1, "deliberate difference", reason),
        };
        counts[category] += 1;
        eprintln!("example {number}: {label} — {reason}");
    }
    assert_eq!(counts, [47, 1, 0]);
    eprintln!(
        "CommonMark 0.31.2 List items (253–300): {} supported, {} deliberate differences, {} unsupported; other sections not scored by this test",
        counts[0], counts[1], counts[2]
    );
}

#[test]
fn pinned_commonmark_list_items_every_split_and_scalar_prefix() {
    let mut splits = 0;
    let mut prefixes = 0;
    for &(number, markdown, expected) in LIST_ITEM_CASES {
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
            let prefix_source = &markdown[..offset + ch.len_utf8()];
            let mut finished = Document::new();
            finished.append(&markdown[..offset]).unwrap();
            finished
                .append(&markdown[offset..offset + ch.len_utf8()])
                .unwrap();
            finished.finish();
            assert_eq!(snapshot(&finished), snapshot(&parse(prefix_source)));
            prefixes += 1;
        }
        doc.finish();
        check_case(number, markdown, expected, &doc);
        assert_eq!(snapshot(&doc), snapshot(&whole));
    }
    assert_eq!((splits, prefixes), (1541, 1493));
    eprintln!(
        "48 examples: 1541 single splits and 1493 scalar prefixes preserve classified semantics and source metadata"
    );
}

#[test]
fn list_item_samples_preserve_mapping_when_sliced_and_read_with_six_byte_budgets() {
    let limit = WorkLimit {
        bytes: 6,
        queries: 1,
        objects: 1,
        ..Default::default()
    };
    for &(_, markdown, _) in LIST_ITEM_CASES {
        let doc = parse(markdown);
        let span = Span::new(0, markdown.len());
        let expected: String = doc
            .display_runs_in(span)
            .unwrap()
            .filter(|r| !r.style.contains(Style::HIDDEN))
            .map(|r| r.text)
            .collect();
        let mut sliced = String::new();
        for (start, ch) in markdown.char_indices() {
            for run in doc
                .display_runs_in(Span::new(start, start + ch.len_utf8()))
                .unwrap()
            {
                assert_eq!(run.source, Span::new(start, start + ch.len_utf8()));
                check_mapping(markdown, run.source, run.text, run.replacement);
                if !run.style.contains(Style::HIDDEN) {
                    sliced.push_str(run.text);
                }
            }
        }
        assert_eq!(sliced, expected);
        let mut cursor = doc.display_cursor(span).unwrap();
        assert_eq!(
            cursor
                .next(&doc, &mut Budget::new(WorkLimit::default()))
                .unwrap(),
            DisplayStep::BudgetExhausted
        );
        assert_eq!(cursor.source_position(), 0);
        let mut output = String::new();
        while cursor.source_position() < markdown.len() {
            let old = cursor.source_position();
            let mut budget = Budget::new(limit);
            let DisplayStep::Run(run) = cursor.next(&doc, &mut budget).unwrap() else {
                panic!("small-budget reader made no progress");
            };
            assert_eq!(run.source.start, old);
            check_mapping(markdown, run.source, run.text, run.replacement);
            assert!(cursor.source_position() > old);
            assert!(budget.used().bytes <= 6);
            assert!(budget.used().queries <= 1 && budget.used().objects <= 1);
            if !run.style.contains(Style::HIDDEN) {
                output.push_str(run.text);
            }
            if cursor.source_position() < markdown.len() {
                assert_eq!(
                    cursor.next(&doc, &mut budget).unwrap(),
                    DisplayStep::BudgetExhausted
                );
            }
        }
        assert_eq!(output, expected);
        assert_eq!(
            cursor
                .next(&doc, &mut Budget::new(WorkLimit::default()))
                .unwrap(),
            DisplayStep::Complete
        );
    }
}

#[test]
fn list_items_projection_preserves_numbering_grouping_and_nested_looseness() {
    for (source, expected) in [
        ("", ""),
        ("# A\n", "<h1>A</h1>\n"),
        ("### A\n", "<h3>A</h3>\n"),
        ("- # A\n", "<ul>\n<li>\n<h1>A</h1>\n</li>\n</ul>\n"),
        (
            "- A\n  ---\n  b\n",
            "<ul>\n<li>\n<h2>A</h2>\nb</li>\n</ul>\n",
        ),
        (
            "-     a\n",
            "<ul>\n<li>\n<pre><code>a\n</code></pre>\n</li>\n</ul>\n",
        ),
        (
            "-      a\n",
            "<ul>\n<li>\n<pre><code> a\n</code></pre>\n</li>\n</ul>\n",
        ),
        ("- a\n- b\n", "<ul>\n<li>a</li>\n<li>b</li>\n</ul>\n"),
        (
            "- a\n+ b\n",
            "<ul>\n<li>a</li>\n</ul>\n<ul>\n<li>b</li>\n</ul>\n",
        ),
        (
            "0. a\n9. b\n",
            "<ol start=\"0\">\n<li>a</li>\n<li>b</li>\n</ol>\n",
        ),
        (
            "7) a\n2) b\n",
            "<ol start=\"7\">\n<li>a</li>\n<li>b</li>\n</ol>\n",
        ),
        (
            "- a\n\n- b\n",
            "<ul>\n<li>\n<p>a</p>\n</li>\n<li>\n<p>b</p>\n</li>\n</ul>\n",
        ),
        (
            "- a\n  - b\n\n    c\n",
            "<ul>\n<li>a\n<ul>\n<li>\n<p>b</p>\n<p>c</p>\n</li>\n</ul>\n</li>\n</ul>\n",
        ),
        (
            "- a\n  > b\n",
            "<ul>\n<li>a\n<blockquote>\n<p>b</p>\n</blockquote>\n</li>\n</ul>\n",
        ),
        ("- a  b\n  c\n", "<ul>\n<li>a  b\nc</li>\n</ul>\n"),
        ("- a  \n  b\n", "<ul>\n<li>a<br />\nb</li>\n</ul>\n"),
        (
            "- ```\n  a  \n  ```\n",
            "<ul>\n<li>\n<pre><code>a  \n</code></pre>\n</li>\n</ul>\n",
        ),
        ("- &lt;&amp;\"\n", "<ul>\n<li>&lt;&amp;&quot;</li>\n</ul>\n"),
    ] {
        assert_eq!(project(&parse(source)), expected, "{source:?}");
    }
}

#[test]
fn list_items_projection_consumes_empty_items_and_hidden_definitions() {
    let doc = parse("* a\n*\n\n* c\n");
    assert_eq!(doc.blocks()[1].kind, BlockKind::Blank);
    let empty_item = doc.container(doc.blocks()[1].container.unwrap()).unwrap();
    assert!(matches!(
        empty_item.kind,
        ContainerKind::Item { task: None, .. }
    ));
    assert_eq!(
        project(&doc),
        "<ul>\n<li>\n<p>a</p>\n</li>\n<li></li>\n<li>\n<p>c</p>\n</li>\n</ul>\n"
    );
    assert_eq!(project(&parse("-\n")), "<ul>\n<li></li>\n</ul>\n");
    let doc = parse("- [ref]: /url\n");
    assert!(matches!(doc.blocks()[0].kind, BlockKind::Definition { .. }));
    assert_eq!(project(&doc), "<ul>\n<li></li>\n</ul>\n");
    // Unknown HTML remains escaped prose; no raw-source HTML interpretation.
    assert_eq!(project(&parse("<!-- -->\n")), "<p>&lt;!-- --&gt;</p>\n");
}

#[test]
fn list_items_projection_rejects_unscored_structure_and_styles() {
    for source in [
        "***\n",
        "- [x] task\n",
        "- ***\n",
        "```rust\ncode\n```\n",
        "a | b\n--|--\n",
        "- *emphasis*\n",
        "- **strong**\n",
        "- ~~strike~~\n",
        "- `code`\n",
        "- [link](url)\n",
        "- $math$\n",
        "- [^f]: footnote\n",
    ] {
        assert!(
            std::panic::catch_unwind(|| project(&parse(source))).is_err(),
            "accepted unscored input {source:?}"
        );
    }
}
