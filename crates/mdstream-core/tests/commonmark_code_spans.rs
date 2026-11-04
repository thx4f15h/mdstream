//! Offline scoring of CommonMark 0.31.2 Code spans (328–349).
//! Project public semantics only; no Markdown reparsing, trimming, or example IDs.
use mdstream::{BlockKind, Document, LinkId, LinkKind, LinkTarget, Span, Style};

#[path = "support/snapshot.rs"]
mod support;
use support::snapshot;

include!("fixtures/commonmark_code_spans.rs");

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
    // This chapter only needs direct URI autolinks. Their label and target
    // share the published span; references, images and titled links are rejected.
    for (index, target) in doc.links().iter().enumerate() {
        let id = LinkId(index);
        let info = doc.link_info(id).unwrap();
        assert_eq!(info.kind, LinkKind::Link);
        assert_eq!(*target, LinkTarget::Direct(info.label));
        assert_eq!(info.title, None);
        assert_eq!(info.source.start + 1, info.label.start);
        assert_eq!(info.source.end - 1, info.label.end);
    }
    let mut html = String::new();
    let mut seen = vec![false; doc.links().len()];
    for block in doc.blocks() {
        match block.kind {
            BlockKind::Blank => continue,
            BlockKind::Paragraph => {}
            ref other => panic!("block outside code span projection: {other:?}"),
        }
        let mut chars = Vec::new();
        for run in doc.display_runs_in(block.source).unwrap() {
            if run.style.contains(Style::HIDDEN) {
                continue;
            }
            for unsupported in [Style::EMPHASIS, Style::STRONG, Style::STRIKE, Style::MATH] {
                assert!(!run.style.contains(unsupported), "unscored inline style");
            }
            chars.extend(run.text.chars().map(|ch| (ch, run.style, run.link)));
        }
        // Only the physical paragraph terminator is structural. All other
        // whitespace is preserved by this adapter.
        if chars.last().is_some_and(|(ch, _, _)| *ch == '\n') {
            let (_, style, link) = chars.pop().unwrap();
            assert!(!style.contains(Style::HARD_BREAK));
            assert!(!style.contains(Style::SOFT_BREAK));
            assert!(!style.contains(Style::CODE));
            assert_eq!(link, None);
        }
        html.push_str("<p>");
        let mut active_code = false;
        let mut active_link = None;
        for (ch, style, link) in chars {
            let code = style.contains(Style::CODE);
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
                    html.push_str("<a href=\"");
                    for ch in doc.link_url(id).unwrap().chars() {
                        // Output-layer URI encoding; the core retains the raw
                        // backtick. No other URL encoding is claimed here.
                        assert!(ch.is_ascii() && !ch.is_ascii_whitespace());
                        if ch == '`' {
                            html.push_str("%60");
                        } else {
                            escape(ch, &mut html);
                        }
                    }
                    html.push_str("\">");
                }
            }
            if code && !active_code {
                html.push_str("<code>");
            }
            active_code = code;
            active_link = link;
            if ch == '\n' && style.contains(Style::HARD_BREAK) && !code {
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
        html.push_str("</p>\n");
    }
    assert!(seen.into_iter().all(|seen| seen), "unscored link");
    html
}

enum Assessment {
    Supported,
    Unsupported {
        reason: &'static str,
        observed: &'static str,
    },
}

// Keep observations separate from the unchanged upstream oracle. Both fixes
// and regressions require review instead of automatic reclassification.
fn assessment(number: usize) -> Assessment {
    match number {
        328..=343 | 345..=349 => Assessment::Supported,
        344 => Assessment::Unsupported {
            reason: "raw HTML is outside project requirements; its contents retain inline parsing",
            observed: "<p>&lt;a href=&quot;<code>&quot;&gt;</code></p>\n",
        },
        _ => panic!("unclassified upstream example {number}"),
    }
}

fn check_case(number: usize, markdown: &str, expected: &str, doc: &Document) {
    let actual = project(doc);
    match assessment(number) {
        Assessment::Supported => assert_eq!(actual, expected, "example {number}"),
        Assessment::Unsupported { observed, .. } => {
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
fn pinned_commonmark_code_spans_report() {
    assert_eq!(CODE_SPAN_CASES.len(), 22);
    let mut counts = [0; 3];
    for (index, &(number, markdown, expected)) in CODE_SPAN_CASES.iter().enumerate() {
        assert_eq!(number, 328 + index);
        check_case(number, markdown, expected, &parse(markdown));
        let (category, label, reason) = match assessment(number) {
            Assessment::Supported => (0, "supported", "matches upstream"),
            Assessment::Unsupported { reason, .. } => (2, "unsupported", reason),
        };
        counts[category] += 1;
        eprintln!("example {number}: {label} — {reason}");
    }
    assert_eq!(counts, [21, 0, 1]);
    eprintln!(
        "CommonMark 0.31.2 Code spans (328–349): 21 supported, 0 deliberate difference, 1 unsupported; other sections not scored by this test"
    );
}

#[test]
fn pinned_commonmark_code_spans_every_split_and_scalar_prefix() {
    let mut splits = 0;
    let mut prefixes = 0;
    for &(number, markdown, expected) in CODE_SPAN_CASES {
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
    assert_eq!((splits, prefixes), (299, 277));
    eprintln!(
        "22 examples: 299 UTF-8 single splits and 277 scalar prefixes preserve classified semantics and source metadata"
    );
}

#[test]
fn code_span_projection_preserves_whitespace_styles_links_and_blocks() {
    for (source, expected) in [
        ("", ""),
        ("a  b\n", "<p>a  b</p>\n"),
        ("a\tb\n", "<p>a\tb</p>\n"),
        ("a &#32;\n", "<p>a  </p>\n"),
        ("a\nb\n", "<p>a\nb</p>\n"),
        ("a  \nb\n", "<p>a<br />\nb</p>\n"),
        ("a\n\nb\n", "<p>a</p>\n<p>b</p>\n"),
        ("`a`\n", "<p><code>a</code></p>\n"),
        ("` a `\n", "<p><code>a</code></p>\n"),
        ("`  a  `\n", "<p><code> a </code></p>\n"),
        ("`  `\n", "<p><code>  </code></p>\n"),
        ("`a\tb`\n", "<p><code>a\tb</code></p>\n"),
        ("`a\nb`\n", "<p><code>a b</code></p>\n"),
        ("`a  \nb`\n", "<p><code>a   b</code></p>\n"),
        (
            "`*a* &amp; $b$`\n",
            "<p><code>*a* &amp;amp; $b$</code></p>\n",
        ),
        (
            "`α` and `β`\n",
            "<p><code>α</code> and <code>β</code></p>\n",
        ),
        (
            "<https://a.test/`x>\n",
            "<p><a href=\"https://a.test/%60x\">https://a.test/`x</a></p>\n",
        ),
        (
            "`<https://a.test>`\n",
            "<p><code>&lt;https://a.test&gt;</code></p>\n",
        ),
        (
            "<ab:x> <ab:y>\n",
            "<p><a href=\"ab:x\">ab:x</a> <a href=\"ab:y\">ab:y</a></p>\n",
        ),
    ] {
        assert_eq!(project(&parse(source)), expected, "input {source:?}");
    }
    let doc = parse("<https://a.test/`x>\n");
    assert_eq!(doc.link_url(LinkId(0)).unwrap(), "https://a.test/`x");
    assert_eq!(
        doc.text(doc.link_info(LinkId(0)).unwrap().label),
        Some("https://a.test/`x")
    );
}

#[test]
fn code_span_projection_rejects_unscored_structure_and_styles() {
    for source in [
        "# heading\n",
        "> quote\n",
        "- list\n",
        "***\n",
        "    code\n",
        "```rust\ncode\n```\n",
        "a|b\n--|--\n",
        "*em*\n",
        "**strong**\n",
        "~~strike~~\n",
        "$math$\n",
        "[link](url)\n",
        "![image](url)\n",
        "[ref][id]\n",
        "<a@example.com>\n",
    ] {
        assert!(
            std::panic::catch_unwind(|| project(&parse(source))).is_err(),
            "accepted unscored input {source:?}"
        );
    }
}
