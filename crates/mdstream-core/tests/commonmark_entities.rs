//! Offline scoring of CommonMark 0.31.2 Entity and numeric character references (25–41).
//! Project public semantics only; no Markdown reparsing, trimming, or example IDs.
use mdstream::{
    BlockId, BlockKind, Budget, ContainerKind, DisplayStep, Document, LinkId, LinkKind, LinkTarget,
    ListMarker, ListTightness, Span, Style, WorkLimit,
};

#[path = "support/snapshot.rs"]
mod support;
use support::snapshot;

#[path = "support/link_metadata.rs"]
mod link_metadata;

include!("fixtures/commonmark_entities.rs");

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
    assert!(doc.headings().entries().is_empty());
    assert!(doc.maths().is_empty());
    assert!(doc.tables().is_empty());
    assert!(doc.footnotes().is_empty());
    for index in 0..doc.links().len() {
        let id = LinkId(index);
        assert_eq!(doc.link_info(id).unwrap().kind, LinkKind::Link);
        assert!(!matches!(doc.links()[index], LinkTarget::Email(_)));
        if doc.link_destination(id).is_none() {
            assert!(matches!(doc.links()[index], LinkTarget::Reference(_)));
        }
    }
    let mut html = String::new();
    // Unresolved references display their public label with hidden brackets;
    // only resolved public destinations become anchors.
    let mut seen: Vec<_> = (0..doc.links().len())
        .map(|index| doc.link_destination(LinkId(index)).is_none())
        .collect();
    let mut list = None;
    let mut seen_containers = vec![false; doc.container_count()];
    for (index, block) in doc.blocks().iter().enumerate() {
        let item_list = block.container.map(|id| {
            let item = doc.container(id).unwrap();
            assert!(matches!(
                item.kind,
                ContainerKind::Item {
                    number: None,
                    task: None
                }
            ));
            assert!(!seen_containers[id.0], "unscored multi-leaf item");
            seen_containers[id.0] = true;
            let parent = item.parent.unwrap();
            let node = doc.container(parent).unwrap();
            assert_eq!(node.parent, None, "unscored nested list");
            assert!(matches!(
                node.kind,
                ContainerKind::List {
                    marker: ListMarker::Bullet(_)
                }
            ));
            assert_eq!(node.list_tightness, Some(ListTightness::Tight));
            assert!(matches!(
                block.kind,
                BlockKind::ListItem {
                    number: None,
                    task: None,
                    ..
                }
            ));
            parent
        });
        if list != item_list {
            if list.is_some() {
                html.push_str("</ul>\n");
            }
            if let Some(id) = item_list {
                assert!(!seen_containers[id.0], "reopened list");
                seen_containers[id.0] = true;
                html.push_str("<ul>\n");
            }
            list = item_list;
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
            BlockKind::ListItem {
                number: None,
                task: None,
                ..
            } => {
                assert!(block.container.is_some());
                ("<li>".to_owned(), "</li>\n", false)
            }
            BlockKind::Paragraph => ("<p>".to_owned(), "</p>\n", false),
            BlockKind::Code { info } => {
                let mut open = "<pre><code".to_owned();
                if info.is_some() {
                    // Consume the core decoder; the projection does no decoding.
                    let mut cursor = doc.code_info(BlockId(index)).unwrap();
                    let mut budget = Budget::new(WorkLimit::unlimited());
                    let mut decoded = String::new();
                    while let Some(chunk) = cursor.advance(&mut budget).unwrap() {
                        decoded.push_str(chunk.text);
                    }
                    open.push_str(" class=\"language-");
                    for ch in decoded.split_ascii_whitespace().next().unwrap().chars() {
                        escape(ch, &mut open);
                    }
                    open.push('"');
                }
                open.push('>');
                (open, "</code></pre>\n", true)
            }
            ref other => panic!("block outside entity projection: {other:?}"),
        };
        let mut chars = Vec::new();
        for run in doc.display_runs_in(block.source).unwrap() {
            if run.style.contains(Style::HIDDEN) {
                continue;
            }
            for unsupported in [Style::STRONG, Style::STRIKE, Style::MATH] {
                assert!(!run.style.contains(unsupported), "unscored inline style");
            }
            assert!(!literal || (run.style.contains(Style::CODE) && run.link.is_none()));
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
        // Only the physical paragraph terminator is structural. All other
        // whitespace is preserved by this adapter.
        if !literal
            && chars.last().is_some_and(|(ch, _, _, source, replacement)| {
                *ch == '\n'
                    && replacement.is_none()
                    && source.end == block.source.end
                    && doc.text(*source).unwrap().ends_with('\n')
            })
        {
            let (_, style, link, _, _) = chars.pop().unwrap();
            assert!(!style.contains(Style::HARD_BREAK));
            assert!(!style.contains(Style::SOFT_BREAK));
            assert!(!style.contains(Style::CODE));
            assert_eq!(link, None);
        }
        html.push_str(&open);
        let mut active_code = false;
        let mut active_link = None;
        let mut active_em = false;
        for (ch, style, link, _, _) in chars {
            let code = style.contains(Style::CODE) && !literal;
            let em = style.contains(Style::EMPHASIS);
            assert!(!em || (!code && link.is_none()), "unscored nested emphasis");
            if active_em && !em {
                html.push_str("</em>");
            }
            assert!(!code || link.is_none(), "unscored code inside a link");
            if active_code && !code {
                html.push_str("</code>");
            }
            if active_link != link {
                if active_link.is_some() {
                    html.push_str("</a>");
                }
                if let Some(id) = link {
                    assert!(!seen[id.0], "split link label");
                    seen[id.0] = true;
                    html.push_str(&link_metadata::opening_tag(
                        doc,
                        id,
                        link_metadata::SIX_BYTES,
                    ));
                }
            }
            if code && !active_code {
                html.push_str("<code>");
            }
            if em && !active_em {
                html.push_str("<em>");
            }
            active_em = em;
            active_code = code;
            active_link = link;
            if ch == '\n' && style.contains(Style::HARD_BREAK) && !code && !literal {
                html.push_str("<br />");
            }
            escape(ch, &mut html);
        }
        if active_code {
            html.push_str("</code>");
        }
        if active_link.is_some() {
            html.push_str("</a>");
        }
        if active_em {
            html.push_str("</em>");
        }
        html.push_str(close);
    }
    if list.is_some() {
        html.push_str("</ul>\n");
    }
    assert!(
        seen_containers.into_iter().all(|seen| seen),
        "unscored container"
    );
    assert!(seen.into_iter().all(|seen| seen), "unscored link");
    html
}

enum Assessment {
    Supported,
    Deliberate {
        reason: &'static str,
        observed: &'static str,
    },
    Unsupported {
        reason: &'static str,
        observed: &'static str,
    },
}

// Independent observations never replace the verbatim upstream oracle.
fn assessment(number: usize) -> Assessment {
    match number {
        25..=30 | 32..=40 => Assessment::Supported,
        31 => Assessment::Unsupported {
            reason: "raw HTML is outside project requirements",
            observed: "<p>&lt;a href=&quot;öö.html&quot;&gt;</p>\n",
        },
        41 => Assessment::Deliberate {
            reason: "failed link suffix retains the shortcut reference with hidden brackets",
            observed: "<p>a(url &quot;tit&quot;)</p>\n",
        },
        _ => panic!("unclassified upstream example {number}"),
    }
}

fn check_case(number: usize, markdown: &str, expected: &str, doc: &Document) {
    let actual = project(doc);
    link_metadata::snapshot(doc);
    match assessment(number) {
        Assessment::Supported => assert_eq!(actual, expected, "example {number}"),
        Assessment::Deliberate { observed, .. } | Assessment::Unsupported { observed, .. } => {
            assert_ne!(
                actual, expected,
                "example {number} now matches; review classification"
            );
            assert_eq!(
                actual, observed,
                "example {number} changed; upstream {expected:?}"
            );
        }
    }
    assert_eq!(doc.source(), markdown);
    let mut end = 0;
    for run in doc.display_runs_in(Span::new(0, markdown.len())).unwrap() {
        assert_eq!(run.source.start, end);
        assert!(run.source.start < run.source.end);
        assert!(markdown.is_char_boundary(run.source.start));
        assert!(markdown.is_char_boundary(run.source.end));
        if let Some(original) = run.replacement {
            assert!(original.start <= run.source.start && run.source.end <= original.end);
            assert!(markdown.is_char_boundary(original.start));
            assert!(markdown.is_char_boundary(original.end));
            assert_eq!(original, run.source);
            assert!(!run.style.contains(Style::CODE));
            let raw = &markdown[original.start..original.end];
            let expected = match raw {
                "&nbsp;" => "\u{a0}",
                "&amp;" => "&",
                "&copy;" => "©",
                "&AElig;" => "Æ",
                "&Dcaron;" => "Ď",
                "&frac34;" => "¾",
                "&HilbertSpace;" => "ℋ",
                "&DifferentialD;" => "ⅆ",
                "&ClockwiseContourIntegral;" => "∲",
                "&ngE;" => "≧\u{338}",
                "&#35;" => "#",
                "&#1234;" => "Ӓ",
                "&#992;" => "Ϡ",
                "&#0;" => "�",
                "&#X22;" | "&quot;" => "\"",
                "&#XD06;" => "ആ",
                "&#xcab;" => "ಫ",
                "&ouml;" => "ö",
                "&#42;" => "*",
                "&#10;" => "\n",
                "&#9;" => "\t",
                other => panic!("unexpected entity replacement {other:?}"),
            };
            assert_eq!(run.text, expected, "replacement {raw}");
        } else {
            assert_eq!(run.text, &markdown[run.source.start..run.source.end]);
        }
        end = run.source.end;
    }
    assert_eq!(end, markdown.len());
}

#[test]
fn pinned_commonmark_entities_report() {
    assert_eq!(ENTITY_CASES.len(), 17);
    let mut counts = [0; 3];
    for (index, &(number, markdown, expected)) in ENTITY_CASES.iter().enumerate() {
        assert_eq!(number, 25 + index);
        check_case(number, markdown, expected, &parse(markdown));
        let (category, label, reason) = match assessment(number) {
            Assessment::Supported => (0, "supported", "matches upstream"),
            Assessment::Deliberate { reason, .. } => (1, "deliberate difference", reason),
            Assessment::Unsupported { reason, .. } => (2, "unsupported", reason),
        };
        counts[category] += 1;
        eprintln!("example {number}: {label} — {reason}");
    }
    assert_eq!(counts, [15, 1, 1]);
    eprintln!(
        "CommonMark 0.31.2 Entity and numeric character references (25–41): 15 supported, 1 deliberate difference, 1 unsupported; other sections not scored by this test"
    );
}

#[test]
fn pinned_commonmark_entities_every_split_and_scalar_prefix() {
    let mut splits = 0;
    let mut prefixes = 0;
    for &(number, markdown, expected) in ENTITY_CASES {
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
            prefixes += 1;
        }
        doc.finish();
        check_case(number, markdown, expected, &doc);
        assert_eq!(snapshot(&doc), snapshot(&whole));
    }
    assert_eq!((splits, prefixes), (524, 507));
    eprintln!(
        "17 examples: {splits} UTF-8 single splits and {prefixes} scalar prefixes preserve classified semantics and source metadata"
    );
}

#[test]
fn entity_projection_preserves_whitespace_styles_structure_and_raw_metadata() {
    for (source, expected) in [
        ("", ""),
        ("a  b\tc\n", "<p>a  b\tc</p>\n"),
        ("a\nb\n", "<p>a\nb</p>\n"),
        ("a  \nb\n", "<p>a<br />\nb</p>\n"),
        ("a &#32;\n", "<p>a  </p>\n"),
        ("&#10;", "<p>\n</p>\n"),
        ("x&#10;\n", "<p>x\n</p>\n"),
        ("&#9;x\n", "<p>\tx</p>\n"),
        ("&#35; x\n", "<p># x</p>\n"),
        ("&amp;amp; \\&amp;\n", "<p>&amp;amp; &amp;amp;</p>\n"),
        ("&ngE; &#x1F642;\n", "<p>≧\u{338} 🙂</p>\n"),
        ("&#42;x&#42; *y*\n", "<p>*x* <em>y</em></p>\n"),
        ("`&amp;` &amp;\n", "<p><code>&amp;amp;</code> &amp;</p>\n"),
        ("`  x  `\n", "<p><code> x </code></p>\n"),
        (
            "~~~\n&amp;\t \n\n~~~\n",
            "<pre><code>&amp;amp;\t \n\n</code></pre>\n",
        ),
        ("```\n", "<pre><code></code></pre>\n"),
        ("```\nx", "<pre><code>x</code></pre>\n"),
        ("    &amp;\n", "<pre><code>&amp;amp;\n</code></pre>\n"),
        (
            "&#42; a\n\n* a\n* b\n",
            "<p>* a</p>\n<ul>\n<li>a</li>\n<li>b</li>\n</ul>\n",
        ),
        ("[a&amp;b](url)\n", "<p><a href=\"url\">a&amp;b</a></p>\n"),
        (
            "[a](url 'a&\"b')\n",
            "<p><a href=\"url\" title=\"a&amp;&quot;b\">a</a></p>\n",
        ),
        (
            "a\n\n~~~\nx\n~~~\n\nb\n",
            "<p>a</p>\n<pre><code>x\n</code></pre>\n<p>b</p>\n",
        ),
        ("[missing]\n", "<p>missing</p>\n"),
        (
            "[a](url &quot;tit&quot;)\n",
            "<p>a(url &quot;tit&quot;)</p>\n",
        ),
        (
            "[a]\n\n[a]: url 'title'\n",
            "<p><a href=\"url\" title=\"title\">a</a></p>\n",
        ),
        (
            "[a](/f&ouml; 't&amp;')\n",
            "<p><a href=\"/f%C3%B6\" title=\"t&amp;\">a</a></p>\n",
        ),
        (
            "~~~ f&ouml; extra\nx\n~~~\n",
            "<pre><code class=\"language-fö\">x\n</code></pre>\n",
        ),
    ] {
        assert_eq!(project(&parse(source)), expected, "input {source:?}");
    }
    for source in [
        "[a](/f&ouml; 't&amp;')\n",
        "[a]\n\n[a]: /f&ouml; 't&amp;'\n",
    ] {
        let doc = parse(source);
        assert_eq!(doc.link_destination(LinkId(0)), Some("/f&ouml;"));
        assert_eq!(doc.link_url(LinkId(0)).unwrap(), "/f&ouml;");
        assert_eq!(doc.link_title(LinkId(0)), Some("t&amp;"));
    }
    let doc = parse("~~~ f&ouml; extra\nx\n~~~\n");
    let BlockKind::Code { info: Some(info) } = doc.blocks()[0].kind else {
        panic!("missing raw info");
    };
    assert_eq!(doc.text(info), Some("f&ouml; extra"));
    let doc = parse("[a](url &quot;tit&quot;)\n");
    assert_eq!(doc.links().len(), 1);
    assert!(matches!(doc.links()[0], LinkTarget::Reference(_)));
    assert_eq!(doc.link_destination(LinkId(0)), None);
    assert_eq!(doc.text(doc.link_info(LinkId(0)).unwrap().label), Some("a"));
}

#[test]
fn entity_projection_rejects_unscored_structure_styles_and_urls() {
    for source in [
        "> quote\n",
        "# heading\n",
        "***\n",
        "a|b\n--|--\n",
        "**strong**\n",
        "~~strike~~\n",
        "$math$\n",
        "![image](url)\n",
        "<foo@example.com>\n",
        "[*em*](url)\n",
        "[`code`](url)\n",
        "*`code`*\n",
        "1. ordered\n",
        "- [ ] task\n",
        "- outer\n  - nested\n",
        "- a\n\n- b\n",
        "- a\n  b\n",
        "- ```\n  x\n  ```\n",
        "[a](/x&#9;y)\n",
        "[a](/x&#10;y)\n",
        "[a](/x&quot;y)\n",
        "[^a]\n\n[^a]: footnote\n",
    ] {
        assert!(
            std::panic::catch_unwind(|| project(&parse(source))).is_err(),
            "accepted unscored input {source:?}"
        );
    }
    for (source, expected) in [
        (
            "[a](<two words>)\n",
            "<p><a href=\"two%20words\">a</a></p>\n",
        ),
        ("[a](/x&#32;y)\n", "<p><a href=\"/x%20y\">a</a></p>\n"),
    ] {
        assert_eq!(project(&parse(source)), expected);
    }
}

#[test]
fn entity_samples_preserve_mapping_when_sliced_and_read_with_six_byte_budgets() {
    let limit = WorkLimit {
        bytes: 6,
        queries: 1,
        objects: 1,
        ..Default::default()
    };
    for &(_, markdown, _) in ENTITY_CASES {
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
                if let Some(original) = run.replacement {
                    assert!(original.start <= start && run.source.end <= original.end);
                    assert!(doc.text(original).unwrap().starts_with('&'));
                    if original.start < start {
                        assert_eq!(run.text, "");
                    }
                } else {
                    assert_eq!(run.text, &markdown[run.source.range()]);
                }
                if !run.style.contains(Style::HIDDEN) {
                    sliced.push_str(run.text);
                }
            }
        }
        assert_eq!(sliced, expected);
        let mut cursor = doc.display_cursor(span).unwrap();
        let mut output = String::new();
        while cursor.source_position() < markdown.len() {
            let old = cursor.source_position();
            let mut budget = Budget::new(limit);
            let DisplayStep::Run(run) = cursor.next(&doc, &mut budget).unwrap() else {
                panic!("small-budget reader made no progress");
            };
            assert_eq!(run.source.start, old);
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
fn entity_metadata_projection_encodes_core_output_once() {
    for (source, expected) in [
        (
            "[a](/f&ouml; 't&amp;')\n",
            "<p><a href=\"/f%C3%B6\" title=\"t&amp;\">a</a></p>\n",
        ),
        (
            "[a]\n\n[a]: /f&ouml; 't&amp;'\n",
            "<p><a href=\"/f%C3%B6\" title=\"t&amp;\">a</a></p>\n",
        ),
        (
            "[a](/中🙂 '  &ngE; &#10;&#9;  ')\n",
            "<p><a href=\"/%E4%B8%AD%F0%9F%99%82\" title=\"  ≧\u{338} \n\t  \">a</a></p>\n",
        ),
        (
            "[a](/&amp;ouml; 'x\\&amp;')\n",
            "<p><a href=\"/&amp;ouml;\" title=\"x&amp;amp;\">a</a></p>\n",
        ),
        (
            "[a](/&#92;* '&quot;')\n",
            "<p><a href=\"/%5C*\" title=\"&quot;\">a</a></p>\n",
        ),
        (
            "[a](/&CounterClockwiseContourIntegral; '&nGt;')\n",
            "<p><a href=\"/%E2%88%B3\" title=\"≫\u{20d2}\">a</a></p>\n",
        ),
    ] {
        let whole = parse(source);
        assert_eq!(project(&whole), expected, "{source:?}");
        for split in (0..=source.len()).filter(|&n| source.is_char_boundary(n)) {
            let mut doc = Document::new();
            doc.append(&source[..split]).unwrap();
            doc.append(&source[split..]).unwrap();
            doc.finish();
            assert_eq!(project(&doc), expected, "{source:?} split {split}");
        }
    }
}
