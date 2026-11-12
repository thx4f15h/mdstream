//! Offline scoring of CommonMark 0.31.2 Emphasis and strong emphasis subset (350–373).
//! The adapter reads public paragraph/display semantics, never Markdown syntax.
use mdstream::{BlockKind, Budget, DisplayStep, Document, Span, Style, WorkLimit};

#[path = "support/snapshot.rs"]
mod support;
use support::snapshot;

include!("fixtures/commonmark_emphasis_single.rs");

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

// Project the published single emphasis bit, not an inferred delimiter tree.
// This bit-only adapter does not consume the published depths; never reconstruct them
// from source markers or use the example number/expected HTML in this adapter.
fn legacy_project(doc: &Document) -> String {
    assert_eq!(doc.container_count(), 0);
    assert!(doc.headings().entries().is_empty());
    assert!(doc.links().is_empty() && doc.references().is_empty());
    assert!(doc.tables().is_empty() && doc.maths().is_empty() && doc.footnotes().is_empty());
    let mut html = String::new();
    for block in doc.blocks() {
        match block.kind {
            BlockKind::Blank => continue,
            BlockKind::Paragraph => {}
            ref other => panic!("unscored block {other:?}"),
        }
        html.push_str("<p>");
        let mut active_em = false;
        for run in doc.display_runs_in(block.source).unwrap() {
            if run.style.contains(Style::HIDDEN) {
                continue;
            }
            for unsupported in [Style::CODE, Style::STRONG, Style::STRIKE, Style::MATH] {
                assert!(!run.style.contains(unsupported), "unscored inline style");
            }
            assert!(run.link.is_none());
            let em = run.style.contains(Style::EMPHASIS);
            if em != active_em {
                html.push_str(if em { "<em>" } else { "</em>" });
                active_em = em;
            }
            // Omit only the borrowed physical paragraph terminator. Decoded
            // entities ending in LF remain content; never trim visible text.
            let payload = if run.source.end == block.source.end
                && run.replacement.is_none()
                && !em
                && run.text.ends_with('\n')
                && doc.text(run.source).unwrap().ends_with('\n')
            {
                assert!(!run.style.contains(Style::HARD_BREAK));
                assert!(!run.style.contains(Style::SOFT_BREAK));
                run.text.strip_suffix('\n').unwrap()
            } else {
                run.text
            };
            for ch in payload.chars() {
                if ch == '\n' && run.style.contains(Style::HARD_BREAK) {
                    html.push_str("<br />");
                }
                match ch {
                    '&' => html.push_str("&amp;"),
                    '<' => html.push_str("&lt;"),
                    '>' => html.push_str("&gt;"),
                    '"' => html.push_str("&quot;"),
                    _ => html.push(ch),
                }
            }
        }
        if active_em {
            html.push_str("</em>");
        }
        html.push_str("</p>\n");
    }
    html
}

#[path = "support/ordered_emphasis.rs"]
mod ordered_emphasis;

fn project(doc: &Document) -> String {
    ordered_emphasis::project(doc, Style::EMPHASIS, false)
}

#[allow(dead_code)] // Uniform ledger states remain available after this chapter closes its gaps.
enum Assessment {
    Supported,
    Unsupported {
        reason: &'static str,
        observed: &'static str,
    },
}

// Independent observations: changes require review, not automatic acceptance.
fn assessment(number: usize) -> Assessment {
    match number {
        350..=373 => Assessment::Supported,
        _ => panic!("unclassified example {number}"),
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
        // These official examples have no replacements, including hidden syntax.
        assert_eq!(run.replacement, None);
        assert_eq!(run.text, &markdown[run.source.range()]);
        end = run.source.end;
    }
    assert_eq!(end, markdown.len());
}

#[test]
fn pinned_commonmark_emphasis_single_report() {
    assert_eq!(EMPHASIS_CASES.len(), 24);
    let mut counts = [0; 3];
    for (index, &(number, markdown, expected)) in EMPHASIS_CASES.iter().enumerate() {
        assert_eq!(number, 350 + index);
        check_case(number, markdown, expected, &parse(markdown));
        let (category, reason) = match assessment(number) {
            Assessment::Supported => (0, "matches upstream"),
            Assessment::Unsupported { reason, .. } => (2, reason),
        };
        counts[category] += 1;
        eprintln!("example {number}: category {category} — {reason}");
    }
    assert_eq!(counts, [24, 0, 0]);
    eprintln!(
        "CommonMark 0.31.2 Emphasis subset 350–373: 24 supported, 0 deliberate differences, 0 unsupported; 374–481 not scored"
    );
}

#[test]
fn pinned_commonmark_emphasis_single_every_split_prefix_and_eof() {
    let mut splits = 0;
    let mut prefixes = 0;
    for &(number, markdown, expected) in EMPHASIS_CASES {
        let whole = parse(markdown);
        for split in (0..=markdown.len()).filter(|&i| markdown.is_char_boundary(i)) {
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
            let end = offset + ch.len_utf8();
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            let mut prefix = Document::new();
            prefix.append(&markdown[..end]).unwrap();
            assert_eq!(snapshot(&doc), snapshot(&prefix), "prefix {end}");
            assert_eq!(doc.pending(), prefix.pending());
            let mut finished = Document::new();
            for c in markdown[..end].chars() {
                finished.append(c.encode_utf8(&mut [0; 4])).unwrap();
            }
            prefix.finish();
            finished.finish();
            assert_eq!(
                snapshot(&finished),
                snapshot(&prefix),
                "example {number}, EOF {end}"
            );
            assert_eq!(finished.pending(), prefix.pending());
            prefixes += 1;
        }
        doc.finish();
        check_case(number, markdown, expected, &doc);
    }
    assert_eq!((splits, prefixes), (286, 262));
}

#[test]
fn emphasis_single_mapping_slices_and_six_byte_budget() {
    for &(_, markdown, _) in EMPHASIS_CASES {
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
                assert_eq!(run.replacement, None);
                assert_eq!(run.text, &markdown[run.source.range()]);
                if !run.style.contains(Style::HIDDEN) {
                    sliced.push_str(run.text);
                }
            }
        }
        assert_eq!(sliced, expected);
        let expected_styles: Vec<_> = doc
            .display_runs_in(span)
            .unwrap()
            .flat_map(|r| {
                r.text
                    .char_indices()
                    .map(move |(i, _)| (r.source.start + i, r.style))
            })
            .collect();
        let mut actual_styles = Vec::new();
        let mut cursor = doc.display_cursor(span).unwrap();
        let mut peer = doc.display_cursor(span).unwrap();
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
            let mut budget = Budget::new(WorkLimit {
                bytes: 6,
                queries: 1,
                objects: 1,
                ..Default::default()
            });
            let DisplayStep::Run(run) = cursor.next(&doc, &mut budget).unwrap() else {
                panic!("no progress");
            };
            assert_eq!(run.source.start, old);
            assert_eq!(run.replacement, None);
            assert_eq!(run.text, &markdown[run.source.range()]);
            assert!(cursor.source_position() > old);
            actual_styles.extend(
                run.text
                    .char_indices()
                    .map(|(i, _)| (run.source.start + i, run.style)),
            );
            let used = budget.used();
            assert_eq!(
                peer.next(&doc, &mut budget).unwrap(),
                DisplayStep::BudgetExhausted
            );
            assert_eq!(peer.source_position(), 0);
            assert_eq!(budget.used(), used);
            assert!(
                budget.used().bytes <= 6
                    && budget.used().queries <= 1
                    && budget.used().objects <= 1
            );
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
        assert_eq!(actual_styles, expected_styles);
        assert_eq!(
            cursor
                .next(&doc, &mut Budget::new(WorkLimit::default()))
                .unwrap(),
            DisplayStep::Complete
        );
    }
}

#[test]
fn emphasis_projection_preserves_visible_markers_whitespace_and_style() {
    for (source, expected) in [
        ("", ""),
        ("*中😀*\n", "<p><em>中😀</em></p>\n"),
        ("_a_ *b*\n", "<p><em>a</em> <em>b</em></p>\n"),
        ("*a  b\tc*\n", "<p><em>a  b\tc</em></p>\n"),
        ("a\nb\n", "<p>a\nb</p>\n"),
        ("*a\nb*\n", "<p><em>a\nb</em></p>\n"),
        ("*a  \nb*\n", "<p><em>a<br />\nb</em></p>\n"),
        ("*a\n\nb*\n", "<p>*a</p>\n<p>b*</p>\n"),
        ("***x\n", "<p>***x</p>\n"),
        ("*a**\n", "<p><em>a</em>*</p>\n"),
        ("&#10;\n", "<p>\n</p>\n"),
        ("*&lt;&amp;&quot;*\n", "<p><em>&lt;&amp;&quot;</em></p>\n"),
        ("*a*尾\n", "<p><em>a</em>尾</p>\n"),
    ] {
        let whole = parse(source);
        assert_eq!(project(&whole), expected, "{source:?}");
        for split in (0..=source.len()).filter(|&i| source.is_char_boundary(i)) {
            let mut doc = Document::new();
            doc.append(&source[..split]).unwrap();
            doc.append(&source[split..]).unwrap();
            doc.finish();
            assert_eq!(snapshot(&doc), snapshot(&whole), "{source:?}, {split}");
            assert_eq!(project(&doc), expected);
        }
    }
    let baseline = project(&parse("*ab*\n"));
    for changed in [
        "ab\n",
        "*cd*\n",
        "*ab\n",
        "*a b*\n",
        "*a\nb*\n",
        "*a\n\nb*\n",
    ] {
        assert_ne!(project(&parse(changed)), baseline);
    }
}

#[test]
fn emphasis_projection_rejects_unscored_structure_and_styles() {
    for source in [
        "# h\n",
        "> q\n",
        "- a\n",
        "***\n",
        "    code\n",
        "~~~\nx\n~~~\n",
        "a|b\n--|--\n",
        "`code`\n",
        "**strong**\n",
        "~~strike~~\n",
        "$math$\n",
        "[a](/x)\n",
        "![a](/x)\n",
        "[r]: /x\n",
        "[^f]: footnote\n",
    ] {
        assert!(
            std::panic::catch_unwind(|| legacy_project(&parse(source))).is_err(),
            "accepted {source:?}"
        );
    }
}

#[test]
fn emphasis_late_close_updates_style_without_changing_block_identity() {
    // Exceed the bounded classification window so the opener/body have already
    // been published before the closer arrives. Partial UTF-8 is tested through
    // every legal split above; append itself accepts complete str values.
    for marker in ["*", "_"] {
        let body = "中😀x ".repeat(300);
        let source = format!("{marker}{body}尾");
        let mut doc = Document::new();
        doc.append(&source).unwrap();
        let id = doc.block_at(0).unwrap();
        assert_eq!(project(&doc), format!("<p>{source}</p>\n"));
        doc.append(marker).unwrap();
        doc.append("!\n").unwrap();
        assert_eq!(doc.block_at(0).unwrap(), id);
        assert_eq!(project(&doc), format!("<p><em>{body}尾</em>!</p>\n"));
        doc.finish();
        assert_eq!(doc.block_at(0).unwrap(), id);
        assert_eq!(
            snapshot(&doc),
            snapshot(&parse(&format!("{source}{marker}!\n")))
        );
        // EOF finalizes pending delimiters without reparsing unmatched text.
        let unmatched = parse(&source);
        assert_eq!(project(&unmatched), format!("<p>{source}</p>\n"));
    }
}
