//! Offline scoring of CommonMark 0.31.2 Inlines (327), excluding its subsections.
//! The adapter reads public paragraph/display semantics, never Markdown syntax.
use mdstream::{BlockKind, Budget, DisplayStep, Document, Span, Style, WorkLimit};

#[path = "support/snapshot.rs"]
mod support;
use support::snapshot;

include!("fixtures/commonmark_inlines.rs");

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

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
        let mut active_code = false;
        for run in doc.display_runs_in(block.source).unwrap() {
            if run.style.contains(Style::HIDDEN) {
                continue;
            }
            for unsupported in [Style::EMPHASIS, Style::STRONG, Style::STRIKE, Style::MATH] {
                assert!(!run.style.contains(unsupported), "unscored inline style");
            }
            assert!(run.link.is_none());
            let code = run.style.contains(Style::CODE);
            if code != active_code {
                html.push_str(if code { "<code>" } else { "</code>" });
                active_code = code;
            }
            // Omit only a borrowed physical paragraph terminator. A decoded
            // entity or a code-space replacement ending in LF is still content.
            let payload = if run.source.end == block.source.end
                && run.replacement.is_none()
                && !code
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
                if ch == '\n' && run.style.contains(Style::HARD_BREAK) && !code {
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
        if active_code {
            html.push_str("</code>");
        }
        html.push_str("</p>\n");
    }
    html
}

fn check_case(number: usize, markdown: &str, expected: &str, doc: &Document) {
    assert_eq!(project(doc), expected, "example {number}");
    assert_eq!(doc.source(), markdown);
    let mut end = 0;
    for run in doc.display_runs_in(Span::new(0, markdown.len())).unwrap() {
        assert_eq!(run.source.start, end);
        assert!(run.source.start < run.source.end);
        // The official example has no replacements, including hidden syntax.
        assert_eq!(run.replacement, None);
        assert_eq!(run.text, &markdown[run.source.range()]);
        end = run.source.end;
    }
    assert_eq!(end, markdown.len());
}

#[test]
fn pinned_commonmark_inlines_report() {
    assert_eq!(INLINE_CASES.len(), 1);
    let mut counts = [0; 3];
    for &(number, markdown, expected) in INLINE_CASES {
        assert_eq!(number, 327);
        check_case(number, markdown, expected, &parse(markdown));
        counts[0] += 1;
        eprintln!("example {number}: supported — matches unchanged upstream HTML");
    }
    assert_eq!(counts, [1, 0, 0]);
    eprintln!(
        "CommonMark 0.31.2 Inlines (327): 1 supported, 0 deliberate difference, 0 unsupported; subsections not scored by this test"
    );
}

#[test]
fn pinned_commonmark_inlines_every_split_prefix_and_eof() {
    let mut splits = 0;
    let mut prefixes = 0;
    for &(number, markdown, expected) in INLINE_CASES {
        let whole = parse(markdown);
        for split in (0..=markdown.len()).filter(|&i| markdown.is_char_boundary(i)) {
            let mut doc = Document::new();
            doc.append(&markdown[..split]).unwrap();
            doc.append(&markdown[split..]).unwrap();
            doc.finish();
            assert_eq!(snapshot(&doc), snapshot(&whole), "split {split}");
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
            assert_eq!(snapshot(&finished), snapshot(&prefix), "EOF {end}");
            prefixes += 1;
        }
        doc.finish();
        check_case(number, markdown, expected, &doc);
    }
    assert_eq!((splits, prefixes), (9, 8));
}

#[test]
fn inlines_mapping_slices_and_six_byte_budget() {
    for &(_, markdown, _) in INLINE_CASES {
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
        assert_eq!(
            cursor
                .next(&doc, &mut Budget::new(WorkLimit::default()))
                .unwrap(),
            DisplayStep::Complete
        );
    }
}

#[test]
fn inlines_projection_preserves_code_literal_suffixes_and_whitespace() {
    for (source, expected) in [
        ("", ""),
        ("`hi`lo`", "<p><code>hi</code>lo`</p>\n"),
        ("`中`尾`\n", "<p><code>中</code>尾`</p>\n"),
        ("`hi`lo``\n", "<p><code>hi</code>lo``</p>\n"),
        ("`hi`lo`x`\n", "<p><code>hi</code>lo<code>x</code></p>\n"),
        ("`hi`\n\nlo`\n", "<p><code>hi</code></p>\n<p>lo`</p>\n"),
        ("a  b\tc\n", "<p>a  b\tc</p>\n"),
        ("a\nb\n", "<p>a\nb</p>\n"),
        ("a  \nb\n", "<p>a<br />\nb</p>\n"),
        ("&#10;\n", "<p>\n</p>\n"),
        ("` a `\n", "<p><code>a</code></p>\n"),
        ("`a\nb`\n", "<p><code>a b</code></p>\n"),
        ("`<&\"`\n", "<p><code>&lt;&amp;&quot;</code></p>\n"),
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
    let baseline = project(&parse("`hi`lo`\n"));
    for changed in [
        "hi lo`\n",
        "`bye`lo`\n",
        "`hi`lo\n",
        "`hi` lo`\n",
        "`hi`\n\nlo`\n",
    ] {
        assert_ne!(project(&parse(changed)), baseline);
    }
}

#[test]
fn inlines_projection_rejects_unscored_structure_and_styles() {
    for source in [
        "# h\n",
        "> q\n",
        "- a\n",
        "***\n",
        "    code\n",
        "~~~\nx\n~~~\n",
        "a|b\n--|--\n",
        "*em*\n",
        "**strong**\n",
        "~~strike~~\n",
        "$math$\n",
        "[a](/x)\n",
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
