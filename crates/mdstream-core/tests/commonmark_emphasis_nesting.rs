//! Offline scoring of CommonMark 0.31.2 Emphasis and strong emphasis subset (406–416).
//! The adapter reads public paragraph/display semantics, never Markdown syntax.
use mdstream::{BlockKind, Budget, DisplayStep, Document, Span, Style, WorkLimit};

#[path = "support/snapshot.rs"]
mod support;
use support::snapshot;

include!("fixtures/commonmark_emphasis_nesting.rs");

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

// Infer order only from public style transitions: an outer style must already
// be visible when an inner style begins. Simultaneous starts and crossing
// endings are ambiguous and rejected. Same-style depth is never reconstructed.
const STYLES: [(Style, &str); 2] = [(Style::EMPHASIS, "em"), (Style::STRONG, "strong")];

fn transition(html: &mut String, active: &mut Vec<usize>, style: Style) {
    let retained: Vec<_> = active
        .iter()
        .copied()
        .filter(|&i| style.contains(STYLES[i].0))
        .collect();
    assert_eq!(
        &active[..retained.len()],
        retained,
        "crossing style boundary"
    );
    let added: Vec<_> = (0..STYLES.len())
        .filter(|i| style.contains(STYLES[*i].0) && !retained.contains(i))
        .collect();
    assert!(added.len() <= 1, "ambiguous simultaneous style starts");
    for &i in active[retained.len()..].iter().rev() {
        html.push_str(&format!("</{}>", STYLES[i].1));
    }
    for &i in &added {
        html.push_str(&format!("<{}>", STYLES[i].1));
    }
    *active = retained;
    active.extend(added);
}

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
        let mut active = Vec::new();
        for run in doc.display_runs_in(block.source).unwrap() {
            if run.style.contains(Style::HIDDEN) {
                continue;
            }
            for unsupported in [Style::CODE, Style::STRIKE, Style::MATH] {
                assert!(!run.style.contains(unsupported), "unscored inline style");
            }
            assert!(run.link.is_none());
            transition(&mut html, &mut active, run.style);
            // Omit only the borrowed physical paragraph terminator. Decoded
            // entities ending in LF remain content; never trim visible text.
            let payload = if run.source.end == block.source.end
                && run.replacement.is_none()
                && active.is_empty()
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
        for &i in active.iter().rev() {
            html.push_str(&format!("</{}>", STYLES[i].1));
        }
        html.push_str("</p>\n");
    }
    html
}

#[path = "support/partial_emphasis.rs"]
mod partial_emphasis;

#[path = "support/ordered_emphasis.rs"]
mod ordered_emphasis;

fn project(doc: &Document) -> String {
    ordered_emphasis::project(doc, Style::EMPHASIS.union(Style::STRONG), false)
}

#[allow(dead_code)] // Uniform ledger states remain available after this chapter closes its gaps.
enum Assessment {
    Rejected {
        reason: &'static str,
    },
    Supported,
    Unsupported {
        reason: &'static str,
        observed: &'static str,
    },
}

// Independent reviewed observations; never generated from the parser or oracle.
// No example in this subset is classified as a deliberate dialect difference.
fn assessment(number: usize) -> Assessment {
    match number {
        406..=416 => Assessment::Supported,
        _ => panic!("unclassified example {number}"),
    }
}

fn check_case(number: usize, markdown: &str, expected: &str, doc: &Document) {
    match assessment(number) {
        Assessment::Supported => assert_eq!(project(doc), expected, "example {number}"),
        Assessment::Rejected { .. } => {
            assert!(std::panic::catch_unwind(|| legacy_project(doc)).is_err());
            partial_emphasis::check(number, doc);
        }
        Assessment::Unsupported { observed, .. } => {
            let actual = project(doc);
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
fn pinned_commonmark_emphasis_nesting_report() {
    assert_eq!(EMPHASIS_CASES.len(), 11);
    let mut counts = [0; 3];
    for (index, &(number, markdown, expected)) in EMPHASIS_CASES.iter().enumerate() {
        assert_eq!(number, 406 + index);
        check_case(number, markdown, expected, &parse(markdown));
        let (category, reason) = match assessment(number) {
            Assessment::Supported => (0, "matches upstream"),
            Assessment::Unsupported { reason, .. } | Assessment::Rejected { reason } => (2, reason),
        };
        counts[category] += 1;
        eprintln!("example {number}: category {category} — {reason}");
    }
    assert_eq!(counts, [11, 0, 0]);
    eprintln!(
        "CommonMark 0.31.2 Emphasis subset 406–416: 11 supported, 0 deliberate differences, 0 unsupported; 417–481 not scored"
    );
}

#[test]
fn pinned_commonmark_emphasis_nesting_every_split_prefix_and_eof() {
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
    assert_eq!((splits, prefixes), (171, 160));
}

#[test]
fn emphasis_nesting_mapping_slices_and_six_byte_budget() {
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
fn nesting_projection_preserves_visible_runs_and_rejects_ambiguous_order() {
    for (source, expected) in [
        ("", ""),
        (
            "_中 **😀** 尾_\n",
            "<p><em>中 <strong>😀</strong> 尾</em></p>\n",
        ),
        ("*a**b*\n", "<p><em>a**b</em></p>\n"),
        ("*a  b\tc*\n", "<p><em>a  b\tc</em></p>\n"),
        ("*a\nb*\n", "<p><em>a\nb</em></p>\n"),
        ("*a  \nb*\n", "<p><em>a<br />\nb</em></p>\n"),
        ("*a\n\nb*\n", "<p>*a</p>\n<p>b*</p>\n"),
        ("*&lt;&amp;&quot;*\n", "<p><em>&lt;&amp;&quot;</em></p>\n"),
        ("&#10;\n", "<p>\n</p>\n"),
    ] {
        assert_eq!(project(&parse(source)), expected);
    }
    let baseline = project(&parse("*a **b** c*\n"));
    for changed in [
        "a **b** c\n",
        "*a b c*\n",
        "*a **x** c*\n",
        "*a **b**  c*\n",
    ] {
        assert_ne!(project(&parse(changed)), baseline);
    }
    let doc = parse("*a **b** c*\n");
    let both = doc
        .display_runs_in(Span::new(0, doc.source().len()))
        .unwrap()
        .find(|r| r.style.contains(Style::EMPHASIS) && r.style.contains(Style::STRONG))
        .unwrap()
        .style;
    assert!(
        std::panic::catch_unwind(|| transition(&mut String::new(), &mut Vec::new(), both)).is_err()
    );
    assert!(
        std::panic::catch_unwind(|| transition(&mut String::new(), &mut vec![0, 1], Style::STRONG))
            .is_err()
    );
    for source in [
        "# h\n",
        "> q\n",
        "- a\n",
        "***\n",
        "    code\n",
        "~~~\nx\n~~~\n",
        "a|b\n--|--\n",
        "`code`\n",
        "~~s~~\n",
        "$m$\n",
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
fn same_style_depth_is_not_invented_by_projection() {
    let nested = parse("_foo _bar_ baz_\n");
    let flat = parse("_foo bar baz_\n");
    assert_eq!(legacy_project(&nested), legacy_project(&flat));
    assert_ne!(project(&nested), project(&flat));
    let mut shown = String::new();
    for run in nested
        .display_runs_in(Span::new(0, nested.source().len()))
        .unwrap()
    {
        assert!(!run.style.contains(Style::STRONG));
        if !run.style.contains(Style::HIDDEN) && run.text != "\n" {
            assert!(run.style.contains(Style::EMPHASIS));
            shown.push_str(run.text);
        }
    }
    assert_eq!(shown, "foo bar baz");
    // The oracle preserves two levels; equal visible style bits do not prove
    // HTML equivalence. Keep this a scored gap, never synthesize an inner tag.
    let &(_, _, official) = EMPHASIS_CASES.iter().find(|c| c.0 == 407).unwrap();
    assert_eq!(project(&nested), official);
}

#[test]
fn nested_late_closers_preserve_identity_and_literal_partial_runs() {
    let body = "中😀x ".repeat(300);
    for (open, inner, inner_close, close, expected) in [
        (
            "_",
            "__",
            "__",
            "_",
            format!("<em>{body}<strong>bar</strong> tail</em>"),
        ),
        (
            "_",
            "_",
            "_",
            "_",
            format!("<em>{body}<em>bar</em> tail</em>"),
        ),
        (
            "*",
            "**",
            "**",
            "*",
            format!("<em>{body}<strong>bar</strong> tail</em>"),
        ),
        ("***", "", "**", "*", format!("***{body}bar** tail*")),
    ] {
        let prefix = format!("{open}{body}{inner}bar");
        let suffix = format!("{inner_close} tail{close}!\n");
        let mut doc = Document::new();
        doc.append(&prefix).unwrap();
        let block = doc.block_at(0).unwrap();
        let mut current = prefix.clone();
        for ch in suffix.chars() {
            current.push(ch);
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            let mut whole = Document::new();
            whole.append(&current).unwrap();
            assert_eq!(snapshot(&doc), snapshot(&whole));
            assert_eq!(doc.pending(), whole.pending());
            assert_eq!(doc.block_at(0).unwrap(), block);
        }
        doc.finish();
        assert_eq!(snapshot(&doc), snapshot(&parse(&current)));
        assert_eq!(doc.block_at(0).unwrap(), block);
        if open == "***" {
            assert!(std::panic::catch_unwind(|| legacy_project(&doc)).is_err());
            let at = doc.source().find('中').unwrap();
            let run = doc.runs_in(Span::new(at, at + 3)).unwrap().next().unwrap();
            assert_eq!((run.emphasis_depth, run.strong_depth), (1, 1));
            assert_eq!(
                project(&doc),
                format!("<p><em><strong>{body}bar</strong> tail</em>!</p>\n")
            );
        } else {
            assert_eq!(project(&doc), format!("<p>{expected}!</p>\n"));
        }
    }
}
