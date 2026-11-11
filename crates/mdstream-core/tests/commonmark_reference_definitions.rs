//! Offline scoring of CommonMark 0.31.2 Link reference definitions (192–218).
//! Public semantics only: no Markdown reparsing, bracket repair, or example IDs
//! in the projection. Output URI/HTML encoding uses the existing metadata adapter.
use mdstream::{
    BlockId, BlockKind, Budget, ContainerKind, DisplayStep, Document, LinkId, LinkKind, LinkTarget,
    Span, Style, WorkLimit,
};

#[path = "support/snapshot.rs"]
mod support;
use support::snapshot;
#[path = "support/link_metadata.rs"]
mod link_metadata;

include!("fixtures/commonmark_reference_definitions.rs");

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
    assert!(doc.maths().is_empty());
    assert!(doc.tables().is_empty());
    assert!(doc.footnotes().is_empty());
    let mut seen_links = Vec::new();
    for (index, target) in doc.links().iter().enumerate() {
        let id = LinkId(index);
        assert_eq!(doc.link_info(id).unwrap().kind, LinkKind::Link);
        assert!(matches!(target, LinkTarget::Reference(_)));
        // Unresolved references keep the public hidden-bracket dialect.
        seen_links.push(doc.link_destination(id).is_none());
    }
    let mut html = String::new();
    let mut quote = None;
    let mut seen_quotes = vec![false; doc.container_count()];
    let mut headings = doc.headings().entries().iter();
    for (index, block) in doc.blocks().iter().enumerate() {
        if quote != block.container {
            if quote.is_some() {
                html.push_str("</blockquote>\n");
            }
            if let Some(id) = block.container {
                let node = doc.container(id).unwrap();
                assert_eq!(node.kind, ContainerKind::Quote);
                assert_eq!(node.parent, None, "unscored nested container");
                assert!(!seen_quotes[id.0], "reopened quote");
                seen_quotes[id.0] = true;
                html.push_str("<blockquote>\n");
            }
            quote = block.container;
        }
        let (open, close, literal) = match block.kind {
            BlockKind::Blank => continue,
            BlockKind::Definition { .. } => {
                assert!(
                    doc.display_runs_in(block.source)
                        .unwrap()
                        .all(|r| r.style.contains(Style::HIDDEN))
                );
                continue;
            }
            BlockKind::Paragraph | BlockKind::Quote { .. } => {
                ("<p>".to_owned(), "</p>\n".to_owned(), false)
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
            ref other => panic!("block outside definition projection: {other:?}"),
        };
        let mut chars = Vec::new();
        for run in doc.display_runs_in(block.source).unwrap() {
            if run.style.contains(Style::HIDDEN) {
                continue;
            }
            for unsupported in [Style::EMPHASIS, Style::STRONG, Style::STRIKE, Style::MATH] {
                assert!(!run.style.contains(unsupported), "unscored inline style");
            }
            assert_eq!(run.style.contains(Style::CODE), literal);
            assert!(!literal || run.link.is_none());
            chars.extend(run.text.chars().map(|ch| {
                (
                    ch,
                    run.style,
                    run.link.filter(|id| doc.link_destination(*id).is_some()),
                    run.source,
                    run.replacement,
                )
            }));
        }
        // Remove only a physical block terminator, never payload whitespace
        // or an entity-produced LF. Code terminators remain literal content.
        if !literal
            && chars.last().is_some_and(|(ch, _, _, source, replacement)| {
                *ch == '\n'
                    && replacement.is_none()
                    && (source.end == block.source.end
                        || matches!(block.kind, BlockKind::Heading { .. }))
                    && doc.text(*source).unwrap().ends_with('\n')
            })
        {
            let (_, style, link, _, _) = chars.pop().unwrap();
            assert!(!style.contains(Style::HARD_BREAK));
            assert!(!style.contains(Style::SOFT_BREAK));
            assert_eq!(link, None);
        }
        html.push_str(&open);
        let mut active_link = None;
        for (ch, style, link, _, _) in chars {
            if link != active_link {
                if active_link.is_some() {
                    html.push_str("</a>");
                }
                if let Some(id) = link {
                    assert!(!seen_links[id.0], "split label");
                    seen_links[id.0] = true;
                    html.push_str(&link_metadata::opening_tag(
                        doc,
                        id,
                        link_metadata::SIX_BYTES,
                    ));
                }
                active_link = link;
            }
            if ch == '\n' && style.contains(Style::HARD_BREAK) && !literal {
                html.push_str("<br />");
            }
            escape(ch, &mut html);
        }
        if active_link.is_some() {
            html.push_str("</a>");
        }
        html.push_str(&close);
    }
    if quote.is_some() {
        html.push_str("</blockquote>\n");
    }
    assert!(
        seen_quotes.into_iter().all(|seen| seen),
        "unscored container"
    );
    assert!(seen_links.into_iter().all(|seen| seen), "unscored link");
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

// Frozen observations are separate from the unchanged upstream oracle.
// A changed result must be reviewed, even if it now matches upstream.
fn assessment(number: usize) -> Assessment {
    match number {
        192..=195 | 198 | 200 | 202..=208 | 210 | 214..=218 => Assessment::Supported,
        196 => Assessment::Unsupported {
            reason: "multi-line reference definitions are not recognized",
            observed: "<p>foo: /url '\ntitle\nline1\nline2\n'</p>\n<p>foo</p>\n",
        },
        197 => Assessment::DeliberateDifference {
            reason: "unresolved references retain public labels with hidden brackets",
            observed: "<p>foo: /url 'title</p>\n<p>with blank line'</p>\n<p>foo</p>\n",
        },
        199 => Assessment::DeliberateDifference {
            reason: "unresolved references retain public labels with hidden brackets",
            observed: "<p>foo:</p>\n<p>foo</p>\n",
        },
        201 => Assessment::DeliberateDifference {
            reason: "unresolved references retain public labels with hidden brackets",
            observed: "<p>foo: &lt;bar&gt;(baz)</p>\n<p>foo</p>\n",
        },
        209 => Assessment::DeliberateDifference {
            reason: "unresolved references retain public labels with hidden brackets",
            observed: "<p>foo: /url &quot;title&quot; ok</p>\n",
        },
        211 => Assessment::DeliberateDifference {
            reason: "unresolved references retain public labels with hidden brackets",
            observed: "<pre><code>[foo]: /url &quot;title&quot;\n</code></pre>\n<p>foo</p>\n",
        },
        212 => Assessment::DeliberateDifference {
            reason: "unresolved references retain public labels with hidden brackets",
            observed: "<pre><code>[foo]: /url\n</code></pre>\n<p>foo</p>\n",
        },
        213 => Assessment::DeliberateDifference {
            reason: "definitions cannot interrupt prose; unresolved references still hide brackets",
            observed: "<p>Foo\nbar: /baz</p>\n<p>bar</p>\n",
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
    link_metadata::snapshot(doc);
    match assessment(number) {
        Assessment::Supported => assert_eq!(actual, expected, "example {number}"),
        Assessment::DeliberateDifference { observed, .. }
        | Assessment::Unsupported { observed, .. } => {
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
fn next_line_definition_official_titles_match_unchanged_html() {
    for (number, destination, title) in [(193, "/url", "the title"), (195, "my url", "title")] {
        let &(_, markdown, html) = REFERENCE_DEFINITION_CASES
            .iter()
            .find(|c| c.0 == number)
            .unwrap();
        let doc = parse(markdown);
        assert_eq!(doc.links().len(), 1);
        assert_eq!(doc.link_destination(LinkId(0)), Some(destination));
        assert_eq!(doc.link_title(LinkId(0)), Some(title));
        assert_eq!(
            doc.references()[0].title.and_then(|s| doc.text(s)),
            Some(title)
        );
        assert_eq!(
            doc.text(doc.references()[0].destination.unwrap()),
            Some(destination)
        );
        let text: String = doc
            .display_runs_in(Span::new(0, markdown.len()))
            .unwrap()
            .filter(|r| !r.style.contains(Style::HIDDEN))
            .map(|r| r.text)
            .collect();
        assert!(!text.contains(title));
        assert_eq!(project(&doc), html);
    }
    let &(_, markdown, html) = REFERENCE_DEFINITION_CASES
        .iter()
        .find(|c| c.0 == 198)
        .unwrap();
    check_case(198, markdown, html, &parse(markdown));
    let &(_, markdown, html) = REFERENCE_DEFINITION_CASES
        .iter()
        .find(|c| c.0 == 217)
        .unwrap();
    let doc = parse(markdown);
    assert_eq!(project(&doc), html);
    assert_eq!(doc.link_title(LinkId(1)), Some("bar"));
    assert_eq!(doc.link_destination(LinkId(2)), Some("/baz-url"));
}

#[test]
fn pinned_commonmark_reference_definitions_report() {
    assert_eq!(REFERENCE_DEFINITION_CASES.len(), 27);
    let mut counts = [0; 3];
    for (index, &(number, markdown, expected)) in REFERENCE_DEFINITION_CASES.iter().enumerate() {
        assert_eq!(number, 192 + index);
        check_case(number, markdown, expected, &parse(markdown));
        let (category, label, reason) = match assessment(number) {
            Assessment::Supported => (0, "supported", "matches upstream"),
            Assessment::DeliberateDifference { reason, .. } => (1, "deliberate difference", reason),
            Assessment::Unsupported { reason, .. } => (2, "unsupported", reason),
        };
        counts[category] += 1;
        eprintln!("example {number}: {label} — {reason}");
    }
    assert_eq!(counts, [19, 7, 1]);
    eprintln!(
        "CommonMark 0.31.2 Link reference definitions (192–218): 19 supported, 7 deliberate differences, 1 unsupported; other sections not scored by this test"
    );
}

#[test]
fn pinned_commonmark_reference_definitions_every_split_and_scalar_prefix() {
    let mut splits = 0;
    let mut prefixes = 0;
    for &(number, markdown, expected) in REFERENCE_DEFINITION_CASES {
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
            assert_eq!(
                link_metadata::snapshot(&doc),
                link_metadata::snapshot(&prefix)
            );
            let prefix_source = &markdown[..offset + ch.len_utf8()];
            let mut finished = Document::new();
            finished.append(&markdown[..offset]).unwrap();
            finished
                .append(&markdown[offset..offset + ch.len_utf8()])
                .unwrap();
            finished.finish();
            let whole_prefix = parse(prefix_source);
            assert_eq!(snapshot(&finished), snapshot(&whole_prefix));
            assert_eq!(
                link_metadata::snapshot(&finished),
                link_metadata::snapshot(&whole_prefix)
            );
            prefixes += 1;
        }
        doc.finish();
        check_case(number, markdown, expected, &doc);
        assert_eq!(snapshot(&doc), snapshot(&whole));
    }
    assert_eq!((splits, prefixes), (833, 806));
    eprintln!(
        "27 examples: 833 single splits and 806 scalar prefixes preserve classified semantics and source metadata"
    );
}

#[test]
fn definition_samples_preserve_mapping_when_sliced_and_read_with_six_byte_budgets() {
    let limit = WorkLimit {
        bytes: 6,
        queries: 1,
        objects: 1,
        ..Default::default()
    };
    for &(_, markdown, _) in REFERENCE_DEFINITION_CASES {
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
fn definition_projection_preserves_whitespace_structure_and_metadata() {
    for (source, expected) in [
        ("", ""),
        ("a  b\tc\n", "<p>a  b\tc</p>\n"),
        ("a\nb\n", "<p>a\nb</p>\n"),
        ("a  \nb\n", "<p>a<br />\nb</p>\n"),
        ("a &#32;\n", "<p>a  </p>\n"),
        ("&#10;\n", "<p>\n</p>\n"),
        ("# h\n", "<h1>h</h1>\n"),
        ("h\n===\n", "<h1>h</h1>\n"),
        ("a\nb\n===\n", "<h1>a\nb</h1>\n"),
        ("h&#10;\n===\n", "<h1>h\n</h1>\n"),
        ("~~~\na  \n\n~~~\n", "<pre><code>a  \n\n</code></pre>\n"),
        ("~~~\nx", "<pre><code>x</code></pre>\n"),
        ("[missing]\n", "<p>missing</p>\n"),
        ("[a]: url\n", ""),
        ("[a]\n\n[a]: <>\n", "<p><a href=\"\">a</a></p>\n"),
        ("# [a]\n[a]: url\n", "<h1><a href=\"url\">a</a></h1>\n"),
        (
            "[a]\n\n> [a]: url\n",
            "<p><a href=\"url\">a</a></p>\n<blockquote>\n</blockquote>\n",
        ),
        (
            "[a]: /f&ouml; 't&amp;'\n\n[a]\n",
            "<p><a href=\"/f%C3%B6\" title=\"t&amp;\">a</a></p>\n",
        ),
        (
            "[a]: /a%20b 'a\\\"b\\q'\n\n[a]\n",
            "<p><a href=\"/a%20b\" title=\"a&quot;b\\q\">a</a></p>\n",
        ),
    ] {
        let doc = parse(source);
        assert_eq!(project(&doc), expected, "{source:?}");
        link_metadata::snapshot(&doc);
    }
}

#[test]
fn definition_projection_keeps_first_definition_late_resolution_and_raw_spans() {
    let mut doc = Document::new();
    doc.append("[Foo]\n\n[foo]\n\n").unwrap();
    assert_eq!(doc.links().len(), 2);
    let targets = doc.links().to_vec();
    let infos: Vec<_> = (0..2).map(|i| *doc.link_info(LinkId(i)).unwrap()).collect();
    assert_eq!(project(&doc), "<p>Foo</p>\n<p>foo</p>\n");
    doc.append("[FOO]: /f&ouml; 't&amp;'\n").unwrap();
    let revision = doc.references()[0].revision;
    for (i, info) in infos.iter().enumerate() {
        let id = LinkId(i);
        assert_eq!(doc.link_info(id), Some(info));
        assert_eq!(doc.link_destination(id), Some("/f&ouml;"));
        assert_eq!(doc.link_title(id), Some("t&amp;"));
    }
    doc.append("[foo]: /ignored 'ignored'\n").unwrap();
    doc.finish();
    assert_eq!(doc.links(), targets);
    assert_eq!(doc.references()[0].revision, revision);
    assert_eq!(
        project(&doc),
        "<p><a href=\"/f%C3%B6\" title=\"t&amp;\">Foo</a></p>\n<p><a href=\"/f%C3%B6\" title=\"t&amp;\">foo</a></p>\n"
    );
    link_metadata::snapshot(&doc);
}

#[test]
fn definition_projection_rejects_unscored_structure_and_styles() {
    for source in [
        "- a\n",
        "> > a\n",
        "***\n",
        "~~~rust\nx\n~~~\n",
        "a|b\n--|--\n",
        "*em*\n",
        "**strong**\n",
        "~~strike~~\n",
        "`code`\n",
        "$math$\n",
        "[^f]: footnote\n",
        "[a](url)\n",
        "![a]\n",
        "<a@b.org>\n",
    ] {
        assert!(
            std::panic::catch_unwind(|| project(&parse(source))).is_err(),
            "accepted unscored input {source:?}"
        );
    }
}

#[test]
fn multiline_label_official_208_matches_original_html() {
    let &(_, markdown, html) = REFERENCE_DEFINITION_CASES
        .iter()
        .find(|c| c.0 == 208)
        .unwrap();
    let doc = parse(markdown);
    assert_eq!(project(&doc), html);
    assert_eq!(doc.links().len(), 0);
    assert_eq!(doc.blocks().len(), 2);
    assert!(matches!(doc.blocks()[0].kind, BlockKind::Definition { .. }));
    assert_eq!(
        doc.blocks()[0].source,
        Span::new(0, markdown.find("bar").unwrap())
    );
}
