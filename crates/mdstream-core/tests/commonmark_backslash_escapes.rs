//! Offline scoring of CommonMark 0.31.2 Backslash escapes (12–24).
//! Project public semantics only; no Markdown reparsing, trimming, or example IDs.
use mdstream::{
    BlockId, BlockKind, Budget, Document, LinkId, LinkKind, LinkTarget, Span, Style, WorkLimit,
};

#[path = "support/snapshot.rs"]
mod support;
use support::snapshot;

#[path = "support/link_metadata.rs"]
mod link_metadata;

include!("fixtures/commonmark_backslash_escapes.rs");

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
    assert_eq!(doc.container_count(), 0);
    assert!(doc.headings().entries().is_empty());
    assert!(doc.maths().is_empty());
    assert!(doc.tables().is_empty());
    assert!(doc.footnotes().is_empty());
    for index in 0..doc.links().len() {
        let id = LinkId(index);
        assert_eq!(doc.link_info(id).unwrap().kind, LinkKind::Link);
        assert!(!matches!(doc.links()[index], LinkTarget::Email(_)));
        assert!(doc.link_destination(id).is_some(), "unresolved link");
    }
    let mut html = String::new();
    let mut seen = vec![false; doc.links().len()];
    for (index, block) in doc.blocks().iter().enumerate() {
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
            BlockKind::Paragraph => ("<p>".to_owned(), "</p>\n", false),
            BlockKind::Code { info } => {
                let mut open = "<pre><code".to_owned();
                if info.is_some() {
                    // Decode only through the public, budgeted core reader.
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
            ref other => panic!("block outside backslash projection: {other:?}"),
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
            chars.extend(run.text.chars().map(|ch| (ch, run.style, run.link)));
        }
        // Only the physical paragraph terminator is structural. All other
        // whitespace is preserved by this adapter.
        if !literal && chars.last().is_some_and(|(ch, _, _)| *ch == '\n') {
            let (_, style, link) = chars.pop().unwrap();
            assert!(!style.contains(Style::HARD_BREAK));
            assert!(!style.contains(Style::SOFT_BREAK));
            assert!(!style.contains(Style::CODE));
            assert_eq!(link, None);
        }
        html.push_str(&open);
        let mut active_code = false;
        let mut active_link = None;
        let mut active_em = false;
        for (ch, style, link) in chars {
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
        12..=13 | 15..=20 | 22..=24 => Assessment::Supported,
        14 => Assessment::Deliberate {
            reason: "unclosed code candidate does not replay other inline syntax",
            observed: "<p>*not emphasized*\n&lt;br/&gt; not a tag\n[not a link](/foo)\n`not code`\n1\\. not a list\n\\* not a list\n\\# not a heading\n\\[foo]: /url &quot;not a reference&quot;\n\\&amp;ouml; not a character entity</p>\n",
        },
        21 => Assessment::Unsupported {
            reason: "raw HTML is outside project requirements",
            observed: "<p>&lt;a href=&quot;/bar/)&quot;&gt;</p>\n",
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
            // All replacements in this chapter are code-space normalization.
            assert!(run.style.contains(Style::CODE));
            let raw = &markdown[original.start..original.end];
            match run.text {
                "" => assert!(raw == " " || raw == "\n"),
                " " => assert_eq!(raw, "\n"),
                other => panic!("unexpected code replacement {other:?}"),
            }
        } else {
            assert_eq!(run.text, &markdown[run.source.start..run.source.end]);
        }
        end = run.source.end;
    }
    assert_eq!(end, markdown.len());
}

#[test]
fn pinned_commonmark_backslash_escapes_report() {
    assert_eq!(BACKSLASH_CASES.len(), 13);
    let mut counts = [0; 3];
    for (index, &(number, markdown, expected)) in BACKSLASH_CASES.iter().enumerate() {
        assert_eq!(number, 12 + index);
        check_case(number, markdown, expected, &parse(markdown));
        let (category, label, reason) = match assessment(number) {
            Assessment::Supported => (0, "supported", "matches upstream"),
            Assessment::Deliberate { reason, .. } => (1, "deliberate difference", reason),
            Assessment::Unsupported { reason, .. } => (2, "unsupported", reason),
        };
        counts[category] += 1;
        eprintln!("example {number}: {label} — {reason}");
    }
    assert_eq!(counts, [11, 1, 1]);
    eprintln!(
        "CommonMark 0.31.2 Backslash escapes (12–24): 11 supported, 1 deliberate difference, 1 unsupported; other sections not scored by this test"
    );
}

#[test]
fn pinned_commonmark_backslash_escapes_every_split_and_scalar_prefix() {
    let mut splits = 0;
    let mut prefixes = 0;
    for &(number, markdown, expected) in BACKSLASH_CASES {
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
    assert_eq!((splits, prefixes), (448, 435));
    eprintln!(
        "13 examples: 448 UTF-8 single splits and 435 scalar prefixes preserve classified semantics and source metadata"
    );
}

#[test]
fn backslash_projection_preserves_escapes_whitespace_styles_and_metadata() {
    for (source, expected) in [
        ("", ""),
        ("a  b\tc\n", "<p>a  b\tc</p>\n"),
        ("a\nb\n", "<p>a\nb</p>\n"),
        ("a\\\nb\n", "<p>a<br />\nb</p>\n"),
        ("a\\", "<p>a\\</p>\n"),
        ("\\*x\\*\n", "<p>*x*</p>\n"),
        ("\\\\*x*\n", "<p>\\<em>x</em></p>\n"),
        (
            "*x* plain `y`\n",
            "<p><em>x</em> plain <code>y</code></p>\n",
        ),
        ("`  x  `\n", "<p><code> x </code></p>\n"),
        ("`a\nb`\n", "<p><code>a b</code></p>\n"),
        ("`\\* &amp;`\n", "<p><code>\\* &amp;amp;</code></p>\n"),
        ("\\&amp;\n", "<p>&amp;amp;</p>\n"),
        ("a &#32;\n", "<p>a  </p>\n"),
        ("\\中\\φ\\«\n", "<p>\\中\\φ\\«</p>\n"),
        (
            "~~~\n\\*\t  \n\n~~~\n",
            "<pre><code>\\*\t  \n\n</code></pre>\n",
        ),
        ("```\n", "<pre><code></code></pre>\n"),
        ("```\nx", "<pre><code>x</code></pre>\n"),
        ("    \\&amp;\n", "<pre><code>\\&amp;amp;\n</code></pre>\n"),
        (
            "[a](url) [b](two)\n",
            "<p><a href=\"url\">a</a> <a href=\"two\">b</a></p>\n",
        ),
        (
            "[a](url 'a&\"b')\n",
            "<p><a href=\"url\" title=\"a&amp;&quot;b\">a</a></p>\n",
        ),
        (
            "[a]\n\n[a]: url 'title'\n",
            "<p><a href=\"url\" title=\"title\">a</a></p>\n",
        ),
        (
            "~~~ a\\+b other\nx\n~~~\n",
            "<pre><code class=\"language-a+b\">x\n</code></pre>\n",
        ),
        (
            "a\n\n~~~\nx\n~~~\n\nb\n",
            "<p>a</p>\n<pre><code>x\n</code></pre>\n<p>b</p>\n",
        ),
    ] {
        assert_eq!(project(&parse(source)), expected, "input {source:?}");
    }
    // Encoding the HTML URL must not change the core's raw address or label.
    let doc = parse("<https://a.test?find=\\*>\n");
    let id = LinkId(0);
    assert_eq!(doc.link_destination(id), Some("https://a.test?find=\\*"));
    assert_eq!(doc.link_url(id).unwrap(), "https://a.test?find=\\*");
    assert_eq!(
        doc.text(doc.link_info(id).unwrap().label),
        Some("https://a.test?find=\\*")
    );
    for source in ["[a](/b\\* 't\\*')\n", "[a]\n\n[a]: /b\\* 't\\*'\n"] {
        let doc = parse(source);
        assert_eq!(doc.link_destination(id), Some("/b\\*"));
        assert_eq!(doc.link_title(id), Some("t\\*"));
    }
    let doc = parse("~~~ a\\+b other\nx\n~~~\n");
    let BlockKind::Code { info: Some(info) } = doc.blocks()[0].kind else {
        panic!("missing raw info");
    };
    assert_eq!(doc.text(info), Some("a\\+b other"));
}

#[test]
fn backslash_projection_rejects_unscored_structure_and_styles() {
    for source in [
        "> quote\n",
        "- list\n",
        "# heading\n",
        "***\n",
        "a|b\n--|--\n",
        "**strong**\n",
        "~~strike~~\n",
        "$math$\n",
        "![image](url)\n",
        "<foo@example.com>\n",
        "[missing]\n",
        "[*em*](url)\n",
        "[`code`](url)\n",
        "*`code`*\n",
        "[a](<two\twords>)\n",
        "<https://a.test/`x>\n",
        "[^a]\n\n[^a]: footnote\n",
    ] {
        assert!(
            std::panic::catch_unwind(|| project(&parse(source))).is_err(),
            "accepted unscored input {source:?}"
        );
    }
    assert_eq!(
        project(&parse("[a](<two words>)\n")),
        "<p><a href=\"two%20words\">a</a></p>\n"
    );
}

#[test]
fn backslash_metadata_projection_uses_core_decoding_and_preserves_literal_autolinks() {
    for (source, expected) in [
        (
            "[a](/b\\* 't\\*')\n",
            "<p><a href=\"/b*\" title=\"t*\">a</a></p>\n",
        ),
        (
            "[a]\n\n[a]: /b\\* 't\\*'\n",
            "<p><a href=\"/b*\" title=\"t*\">a</a></p>\n",
        ),
        ("[a](u '')\n", "<p><a href=\"u\" title=\"\">a</a></p>\n"),
        ("[a]()\n", "<p><a href=\"\">a</a></p>\n"),
        (
            "<https://a.test/\\*?x=&amp;>\n",
            "<p><a href=\"https://a.test/%5C*?x=&amp;amp;\">https://a.test/\\*?x=&amp;amp;</a></p>\n",
        ),
        (
            "[a](/%C3%B6?x=1&amp;y=2 'a&amp;quot;')\n",
            "<p><a href=\"/%C3%B6?x=1&amp;y=2\" title=\"a&amp;quot;\">a</a></p>\n",
        ),
    ] {
        assert_eq!(project(&parse(source)), expected, "{source:?}");
    }
}
