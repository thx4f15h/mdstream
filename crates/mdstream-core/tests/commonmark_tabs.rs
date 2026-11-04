//! Offline scoring of CommonMark 0.31.2 Tabs (1–11).
//! The narrow projection preserves whitespace, block boundaries, heading levels,
//! code literals and the tight/loose list structure exercised here. It is not an HTML
//! renderer. It neither reparses Markdown nor receives the example number.
use mdstream::{
    BlockId, BlockKind, Budget, ContainerId, ContainerKind, DisplayStep, Document, ListMarker,
    ListTightness, Span, Style, WorkLimit,
};

#[path = "support/snapshot.rs"]
mod support;
use support::snapshot;

include!("fixtures/commonmark_tabs.rs");

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
        for &id in &path[shared..] {
            let node = doc.container(id).unwrap();
            assert!(!seen[id.0], "reopened sealed container");
            seen[id.0] = true;
            // Preserve list nesting and published tight/loose metadata.
            // Only bullet lists and ordered lists starting at 1 are projected.
            match node.kind {
                ContainerKind::List { marker } => {
                    if let Some(parent) = node.parent
                        && matches!(
                            doc.container(parent).unwrap().kind,
                            ContainerKind::Item { .. }
                        )
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
                    if !tight_item(doc, id)
                        || (block.container == Some(id)
                            && matches!(block.kind, BlockKind::Code { .. }))
                    {
                        html.push('\n');
                    }
                }
                ContainerKind::Quote => html.push_str("<blockquote>\n"),
                other => panic!("container outside tab projection: {other:?}"),
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
                assert!(block.container.is_none(), "unscored container heading");
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
            ref other => panic!("block outside tab projection: {other:?}"),
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

fn check_mapping(markdown: &str, source: Span, text: &str, replacement: Option<Span>) {
    if let Some(mapped) = replacement {
        assert_eq!(mapped, source);
        assert_eq!(&markdown[source.range()], "\t");
        assert!((1..=3).contains(&text.len()));
        assert!(text.bytes().all(|byte| byte == b' '));
    } else {
        assert_eq!(text, &markdown[source.range()]);
    }
}

fn check_case(number: usize, markdown: &str, expected: &str, doc: &Document) {
    let actual = project(doc);
    assert_eq!(actual, expected, "example {number}");
    assert_eq!(doc.source(), markdown);
    // Hidden prefix bytes remain addressable. Partial code tabs use the core's
    // published replacement mapping; other runs borrow exact source bytes.
    let mut end = 0;
    for run in doc.display_runs_in(Span::new(0, markdown.len())).unwrap() {
        assert_eq!(run.source.start, end);
        assert!(run.source.start < run.source.end);
        assert!(markdown.is_char_boundary(run.source.start));
        assert!(markdown.is_char_boundary(run.source.end));
        end = run.source.end;
        check_mapping(markdown, run.source, run.text, run.replacement);
    }
    assert_eq!(end, markdown.len());
}

#[test]
fn pinned_commonmark_tabs_report() {
    assert_eq!(TAB_CASES.len(), 11);
    for (index, &(number, markdown, expected)) in TAB_CASES.iter().enumerate() {
        assert_eq!(number, 1 + index);
        check_case(number, markdown, expected, &parse(markdown));
        eprintln!("example {number}: supported — matches upstream");
    }
    eprintln!(
        "CommonMark 0.31.2 Tabs (1–11): 11 supported, 0 deliberate differences, 0 unsupported; other sections not scored by this test"
    );
}

#[test]
fn pinned_commonmark_tabs_every_split_and_scalar_prefix() {
    let mut splits = 0;
    let mut prefixes = 0;
    for &(number, markdown, expected) in TAB_CASES {
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
    assert_eq!((splits, prefixes), (148, 137));
    eprintln!(
        "11 examples: 148 single splits and 137 scalar prefixes preserve classified semantics and source metadata"
    );
}

#[test]
fn tabs_projection_preserves_literals_whitespace_and_structure() {
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
        ("a&#10;", "<p>a\n</p>\n"),
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
        (
            "-\t\tfoo\n",
            "<ul>\n<li>\n<pre><code>  foo\n</code></pre>\n</li>\n</ul>\n",
        ),
        (
            "-\t\t foo\n",
            "<ul>\n<li>\n<pre><code>   foo\n</code></pre>\n</li>\n</ul>\n",
        ),
        ("-\tfoo\n", "<ul>\n<li>foo</li>\n</ul>\n"),
        (
            "- a\n\n      code\n",
            "<ul>\n<li>\n<p>a</p>\n<pre><code>code\n</code></pre>\n</li>\n</ul>\n",
        ),
        (
            "- a\n\n\t\tcode\n",
            "<ul>\n<li>\n<p>a</p>\n<pre><code>  code\n</code></pre>\n</li>\n</ul>\n",
        ),
        ("> quote\n", "<blockquote>\n<p>quote</p>\n</blockquote>\n"),
        (
            ">\t\tfoo\n",
            "<blockquote>\n<pre><code>  foo\n</code></pre>\n</blockquote>\n",
        ),
        (
            ">     a \tb\n",
            "<blockquote>\n<pre><code>a \tb\n</code></pre>\n</blockquote>\n",
        ),
        ("> a\n> b\n", "<blockquote>\n<p>a\nb</p>\n</blockquote>\n"),
        (
            "> > a\n",
            "<blockquote>\n<blockquote>\n<p>a</p>\n</blockquote>\n</blockquote>\n",
        ),
        (
            "> - a\n",
            "<blockquote>\n<ul>\n<li>a</li>\n</ul>\n</blockquote>\n",
        ),
        ("    中\t🙂\n", "<pre><code>中\t🙂\n</code></pre>\n"),
        (
            "  - foo\n\n\tbar\n",
            "<ul>\n<li>\n<p>foo</p>\n<p>bar</p>\n</li>\n</ul>\n",
        ),
    ] {
        assert_eq!(project(&parse(source)), expected, "input {source:?}");
    }
}

#[test]
fn tabs_projection_rejects_unscored_structure_and_styles() {
    for source in [
        "> # heading\n",
        "2. ordered\n",
        "- [x] task\n",
        "- # heading\n",
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

#[test]
fn tab_samples_preserve_mapping_when_sliced_and_read_with_six_byte_budgets() {
    let limit = WorkLimit {
        bytes: 6,
        queries: 1,
        objects: 1,
        ..Default::default()
    };
    for &(_, markdown, _) in TAB_CASES {
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
