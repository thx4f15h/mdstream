//! Offline scoring of complete GFM 0.29-gfm Strikethrough section (491–493).
//! The adapter reads public paragraph/display semantics, never Markdown syntax.
use mdstream::{BlockKind, Budget, DisplayStep, Document, Span, Style, WorkLimit};

#[path = "support/snapshot.rs"]
mod support;
use support::snapshot;

include!("fixtures/gfm_strikethrough.rs");

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

// Project the published single strikethrough bit, not an inferred delimiter tree.
// Same-style nesting depth is not exposed by this API; never reconstruct it
// from source markers or use the example number/expected HTML in this adapter.
fn project(doc: &Document) -> String {
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
        let mut active_strike = false;
        for run in doc.display_runs_in(block.source).unwrap() {
            if run.style.contains(Style::HIDDEN) {
                continue;
            }
            for unsupported in [Style::CODE, Style::STRONG, Style::EMPHASIS, Style::MATH] {
                assert!(!run.style.contains(unsupported), "unscored inline style");
            }
            assert!(run.link.is_none());
            let strike = run.style.contains(Style::STRIKE);
            if strike != active_strike {
                html.push_str(if strike { "<del>" } else { "</del>" });
                active_strike = strike;
            }
            // Omit only the borrowed physical paragraph terminator. Decoded
            // entities ending in LF remain content; never trim visible text.
            let payload = if run.source.end == block.source.end
                && run.replacement.is_none()
                && !strike
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
        if active_strike {
            html.push_str("</del>");
        }
        html.push_str("</p>\n");
    }
    html
}

// The existing dialect recognizes exactly two tildes. This observation is
// independent of the fixed upstream HTML; changed behavior requires review.
fn check_case(number: usize, markdown: &str, expected: &str, doc: &Document) {
    let actual = project(doc);
    match number {
        491 => {
            assert_ne!(actual, expected, "review single-tilde classification");
            assert_eq!(actual, "<p><del>Hi</del> Hello, ~there~ world!</p>\n");
        }
        492 | 493 => assert_eq!(actual, expected, "example {number}"),
        _ => panic!("unclassified example {number}"),
    }
    assert_eq!(doc.source(), markdown);
    let mut end = 0;
    for run in doc.display_runs_in(Span::new(0, markdown.len())).unwrap() {
        assert_eq!(run.source.start, end);
        assert!(run.source.start < run.source.end);
        assert_eq!(run.replacement, None);
        assert_eq!(run.text, &markdown[run.source.range()]);
        end = run.source.end;
    }
    assert_eq!(end, markdown.len());
}

#[test]
fn pinned_gfm_strikethrough_report() {
    assert_eq!(STRIKE_CASES.len(), 3);
    let mut counts = [0; 3];
    for (index, &(number, markdown, expected)) in STRIKE_CASES.iter().enumerate() {
        assert_eq!(number, 491 + index);
        check_case(number, markdown, expected, &parse(markdown));
        let category = usize::from(number == 491);
        counts[category] += 1;
        eprintln!("example {number}: category {category}");
    }
    assert_eq!(counts, [2, 1, 0]);
    eprintln!(
        "GFM 0.29-gfm Strikethrough 491–493: 2 supported, 1 deliberate difference (single tilde), 0 unsupported"
    );
}

#[test]
fn pinned_gfm_strikethrough_every_split_prefix_and_eof() {
    let mut splits = 0;
    let mut prefixes = 0;
    for &(number, markdown, expected) in STRIKE_CASES {
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
    assert_eq!((splits, prefixes), (91, 88));
}

#[test]
fn strikethrough_mapping_slices_and_six_byte_budget() {
    for &(_, markdown, _) in STRIKE_CASES {
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
fn strike_projection_preserves_payload_and_rejects_unscored_structures() {
    for (source, expected) in [
        ("", ""),
        ("~~中😀~~\n", "<p><del>中😀</del></p>\n"),
        ("~one~\n", "<p>~one~</p>\n"),
        ("x ~~~three~~~\n", "<p>x ~~~three~~~</p>\n"),
        ("~~a  b\tc~~\n", "<p><del>a  b\tc</del></p>\n"),
        ("~~a\nb~~\n", "<p><del>a\nb</del></p>\n"),
        ("~~a  \nb~~\n", "<p><del>a<br />\nb</del></p>\n"),
        ("~~a\n\nb~~\n", "<p>~~a</p>\n<p>b~~</p>\n"),
        (
            "~~&lt;&amp;&quot;~~\n",
            "<p><del>&lt;&amp;&quot;</del></p>\n",
        ),
        ("&#10;\n", "<p>\n</p>\n"),
        ("~~a", "<p>~~a</p>\n"),
        ("~~a ~~\n", "<p>~~a ~~</p>\n"),
    ] {
        assert_eq!(project(&parse(source)), expected, "{source:?}");
    }
    let baseline = project(&parse("~~a b~~\n"));
    for changed in ["a b\n", "~~a x~~\n", "~~a  b~~\n", "~a b~\n"] {
        assert_ne!(project(&parse(changed)), baseline);
    }
    for source in [
        "# h\n",
        "> q\n",
        "- a\n",
        "***\n",
        "    code\n",
        "~~~\nx\n~~~\n",
        "a|b\n--|--\n",
        "`code`\n",
        "*a*\n",
        "**a**\n",
        "$m$\n",
        "[label](/url)\n",
        "![a](/x)\n",
        "[r]: /x\n",
        "[^f]: footnote\n",
    ] {
        assert!(
            std::panic::catch_unwind(|| project(&parse(source))).is_err(),
            "accepted {source:?}"
        );
    }
}

#[test]
fn escaped_tilde_and_double_delimiter_mapping_remain_distinct() {
    let source = "~~\\~~~\n";
    let doc = parse(source);
    assert_eq!(project(&doc), "<p><del>~</del></p>\n");
    for offset in 0..source.len() {
        let run = doc
            .display_runs_in(Span::new(offset, offset + 1))
            .unwrap()
            .next()
            .unwrap();
        assert_eq!(run.source, Span::new(offset, offset + 1));
        assert_eq!(run.replacement, None);
        assert_eq!(run.text, &source[offset..offset + 1]);
        assert_eq!(
            run.style.contains(Style::HIDDEN),
            matches!(offset, 0..=2 | 4..=5)
        );
        if offset == 3 {
            assert!(run.style.contains(Style::STRIKE));
        }
    }
    assert_eq!(project(&parse("~~a\\~~\n")), "<p>~~a~~</p>\n");
    assert_eq!(project(&parse("\\~~a~~\n")), "<p>~~a~~</p>\n");
}

#[test]
fn late_strike_closure_preserves_block_identity_beyond_classification_window() {
    let body = "中😀x ".repeat(300);
    for (open, tail, content) in [
        ("~~", "~~", format!("<del>{body}end</del>")),
        ("~", "~", format!("~{body}end~")),
        ("~~", "~", format!("~~{body}end~")),
        ("~~", "~~~", format!("~~{body}end~~~")),
        ("~~~", "~~~", format!("~~~{body}end~~~")),
        ("~~", "\\~~~", format!("<del>{body}end~</del>")),
    ] {
        // A prose prefix prevents a triple tilde from becoming a code fence.
        let mut current = format!("prefix {open}{body}end");
        let mut doc = Document::new();
        doc.append(&current).unwrap();
        let block = doc.block_at(0).unwrap();
        for ch in format!("{tail}!\n").chars() {
            current.push(ch);
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            let mut whole = Document::new();
            whole.append(&current).unwrap();
            assert_eq!(snapshot(&doc), snapshot(&whole));
            assert_eq!(doc.pending(), whole.pending());
            assert_eq!(doc.block_at(0), Some(block));
        }
        doc.finish();
        assert_eq!(snapshot(&doc), snapshot(&parse(&current)));
        assert_eq!(doc.block_at(0), Some(block));
        assert_eq!(project(&doc), format!("<p>prefix {content}!</p>\n"));
    }
}
