//! Offline scoring of CommonMark 0.31.2 Autolinks (594–612).
//! The adapter reads public paragraph/display semantics, never Markdown syntax.
use mdstream::{
    BlockKind, Budget, Completion, DisplayStep, Document, LinkId, LinkKind, LinkTarget, Span,
    Style, WorkLimit,
};

#[path = "support/snapshot.rs"]
mod support;
use support::snapshot;

include!("fixtures/commonmark_autolinks_official.rs");

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

const SIX_BYTES: WorkLimit = WorkLimit {
    bytes: 6,
    queries: 1,
    objects: 1,
    tiles: 0,
    recovery: 0,
    dependencies: 0,
};

fn escape(ch: char, out: &mut String) {
    match ch {
        '&' => out.push_str("&amp;"),
        '<' => out.push_str("&lt;"),
        '>' => out.push_str("&gt;"),
        '"' => out.push_str("&quot;"),
        _ => out.push(ch),
    }
}

// HTML output encoding only. Never decode Markdown or normalize the oracle.
fn opening_tag(doc: &Document, id: LinkId) -> String {
    let mut out = String::from("<a href=\"");
    for ch in doc.link_url(id).unwrap().chars() {
        assert!(!ch.is_ascii_whitespace() && !ch.is_ascii_control());
        assert!(
            !matches!(ch, '`' | '<' | '>' | '"'),
            "unscored URI encoding"
        );
        if !ch.is_ascii() || matches!(ch, '\\' | '[' | ']') {
            use std::fmt::Write;
            for b in ch.encode_utf8(&mut [0; 4]).bytes() {
                write!(&mut out, "%{b:02X}").unwrap();
            }
        } else {
            escape(ch, &mut out);
        }
    }
    out.push_str("\">");
    out
}

fn project(doc: &Document) -> String {
    assert_eq!(doc.container_count(), 0);
    assert!(doc.headings().entries().is_empty() && doc.references().is_empty());
    assert!(doc.tables().is_empty() && doc.maths().is_empty() && doc.footnotes().is_empty());
    for (i, target) in doc.links().iter().enumerate() {
        let info = doc.link_info(LinkId(i)).unwrap();
        assert_eq!(info.kind, LinkKind::Link);
        assert!(matches!(target, LinkTarget::Direct(s) | LinkTarget::Email(s) if *s == info.label));
        assert_eq!(info.title, None);
        if info.source != info.label {
            assert_eq!(info.source.start + 1, info.label.start);
            assert_eq!(info.source.end - 1, info.label.end);
        }
    }
    let mut html = String::new();
    let mut seen = vec![false; doc.links().len()];
    for block in doc.blocks() {
        match block.kind {
            BlockKind::Blank => continue,
            BlockKind::Paragraph => {}
            ref other => panic!("unscored block {other:?}"),
        }
        html.push_str("<p>");
        let mut active = None;
        for run in doc.display_runs_in(block.source).unwrap() {
            if run.style.contains(Style::HIDDEN) {
                continue;
            }
            for unsupported in [
                Style::EMPHASIS,
                Style::STRONG,
                Style::STRIKE,
                Style::MATH,
                Style::CODE,
            ] {
                assert!(!run.style.contains(unsupported), "unscored inline style");
            }
            if active != run.link {
                if active.is_some() {
                    html.push_str("</a>");
                }
                if let Some(id) = run.link {
                    assert!(!seen[id.0], "split link label");
                    seen[id.0] = true;
                    html.push_str(&opening_tag(doc, id));
                }
                active = run.link;
            }
            // Strip only a borrowed physical paragraph terminator.
            let payload = if run.source.end == block.source.end
                && run.replacement.is_none()
                && run.text.ends_with('\n')
                && doc.text(run.source).unwrap().ends_with('\n')
            {
                assert!(run.link.is_none());
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
                escape(ch, &mut html);
            }
        }
        if active.is_some() {
            html.push_str("</a>");
        }
        html.push_str("</p>\n");
    }
    assert!(seen.into_iter().all(|v| v), "unscored link");
    html
}

// Independent observations of the selected dialect; never passed to project().
const FAILED_EMAIL: &str = "<p>&lt;foo\\+@bar.example.com&gt;</p>\n";
const GFM_OBSERVED: &[(usize, &str)] = &[
    (
        608,
        "<p>&lt; <a href=\"https://foo.bar\">https://foo.bar</a> &gt;</p>\n",
    ),
    (
        611,
        "<p><a href=\"https://example.com\">https://example.com</a></p>\n",
    ),
    (
        612,
        "<p><a href=\"mailto:foo@bar.example.com\">foo@bar.example.com</a></p>\n",
    ),
];

fn check_case(number: usize, markdown: &str, expected: &str, doc: &Document) {
    let actual = project(doc);
    match number {
        606 => {
            assert_ne!(actual, expected, "review newly supported example 606");
            assert_eq!(actual, FAILED_EMAIL);
        }
        608 | 611 | 612 => {
            assert_ne!(actual, expected, "review GFM dialect example {number}");
            assert_eq!(
                actual,
                GFM_OBSERVED.iter().find(|c| c.0 == number).unwrap().1
            );
        }
        594..=605 | 607 | 609 | 610 => assert_eq!(actual, expected, "example {number}"),
        _ => panic!("unclassified example {number}"),
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
fn pinned_commonmark_autolinks_report() {
    assert_eq!(AUTOLINK_CASES.len(), 19);
    let mut counts = [0; 3];
    for (index, &(number, markdown, expected)) in AUTOLINK_CASES.iter().enumerate() {
        assert_eq!(number, 594 + index);
        check_case(number, markdown, expected, &parse(markdown));
        let category = usize::from(matches!(number, 606 | 608 | 611 | 612));
        counts[category] += 1;
        eprintln!(
            "example {number}: {}",
            if category == 0 {
                "supported"
            } else {
                "deliberate difference: opaque failure or GFM bare address"
            }
        );
    }
    assert_eq!(counts, [15, 4, 0]);
    eprintln!("CommonMark 0.31.2 Autolinks: 15 supported, 4 deliberate differences, 0 unsupported");
}

#[test]
fn pinned_commonmark_autolinks_every_split_prefix_and_eof() {
    let mut splits = 0;
    let mut prefixes = 0;
    for &(number, markdown, expected) in AUTOLINK_CASES {
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
    assert_eq!((splits, prefixes), (421, 402));
}

#[test]
fn autolinks_mapping_slices_and_six_byte_budget() {
    for &(_, markdown, _) in AUTOLINK_CASES {
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
fn autolinks_projection_preserves_urls_labels_and_failed_candidates() {
    for (source, expected) in [
        ("", ""),
        ("<ab:>\n", "<p><a href=\"ab:\">ab:</a></p>\n"),
        ("<a@b>\n", "<p><a href=\"mailto:a@b\">a@b</a></p>\n"),
        (
            "<ab:中%20\\[&amp;>\n",
            "<p><a href=\"ab:%E4%B8%AD%20%5C%5B&amp;amp;\">ab:中%20\\[&amp;amp;</a></p>\n",
        ),
        ("<ab:unfinished", "<p>&lt;ab:unfinished</p>\n"),
        ("<ab:a b>", "<p>&lt;ab:a b&gt;</p>\n"),
        ("<a\\+@b>\n", "<p>&lt;a\\+@b&gt;</p>\n"),
        (
            "<ab:x>  <a@b>\n",
            "<p><a href=\"ab:x\">ab:x</a>  <a href=\"mailto:a@b\">a@b</a></p>\n",
        ),
        ("a  b\tc\n", "<p>a  b\tc</p>\n"),
        ("&#10;\n", "<p>\n</p>\n"),
    ] {
        let whole = parse(source);
        assert_eq!(project(&whole), expected, "{source:?}");
        for split in (0..=source.len()).filter(|&i| source.is_char_boundary(i)) {
            let mut doc = Document::new();
            doc.append(&source[..split]).unwrap();
            doc.append(&source[split..]).unwrap();
            doc.finish();
            assert_eq!(snapshot(&doc), snapshot(&whole));
            assert_eq!(project(&doc), expected);
        }
    }
    let baseline = project(&parse("<ab:x>\n"));
    for changed in [
        "ab:x\n",
        "<ab:y>\n",
        "<ab:x\n",
        "<a@b>\n",
        "<ab:x> tail\n",
        "<ab:x>\n\nnext\n",
    ] {
        assert_ne!(project(&parse(changed)), baseline);
    }
}

#[test]
fn autolinks_projection_rejects_unscored_structure_styles_and_links() {
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
        "`code`\n",
        "[a](/x)\n",
        "![a](/x)\n",
        "[r]: /x\n",
        "[^f]: footnote\n",
        "<ab:`>\n",
    ] {
        assert!(
            std::panic::catch_unwind(|| project(&parse(source))).is_err(),
            "accepted {source:?}"
        );
    }
}

#[test]
fn autolink_display_and_destination_share_six_byte_budget() {
    for source in AUTOLINK_CASES
        .iter()
        .map(|c| c.1)
        .chain(["<ab:中%20\\[&amp;> <a@b>\n"])
    {
        let doc = parse(source);
        for i in 0..doc.links().len() {
            let id = LinkId(i);
            let info = doc.link_info(id).unwrap();
            let mut display = doc.display_cursor(info.label).unwrap();
            let mut destination = doc.decoded_link_destination(id).unwrap();
            let mut zero = Budget::new(WorkLimit::default());
            assert_eq!(
                destination.advance(&mut zero),
                Err(Completion::BudgetExhausted)
            );
            assert_eq!(destination.source_position(), info.label.start);
            let mut shown = String::new();
            let mut target = String::new();
            let mut done = false;
            let mut frames = 0;
            while !done || display.source_position() < info.label.end {
                frames += 1;
                assert!(frames <= 2 * info.label.len() + 2);
                let mut budget = Budget::new(SIX_BYTES);
                match display.next(&doc, &mut budget).unwrap() {
                    DisplayStep::Run(run) => {
                        assert_eq!(run.link, Some(id));
                        assert_eq!(run.replacement, None);
                        assert_eq!(run.text, doc.text(run.source).unwrap());
                        shown.push_str(run.text);
                    }
                    DisplayStep::Complete => {}
                    other => panic!("display did not progress {other:?}"),
                }
                let old = destination.source_position();
                match destination.advance(&mut budget) {
                    Ok(Some(chunk)) => {
                        assert_eq!(chunk.source.start, old);
                        assert_eq!(chunk.text, doc.text(chunk.source).unwrap());
                        target.push_str(chunk.text);
                    }
                    Ok(None) => done = true,
                    Err(Completion::BudgetExhausted) => {
                        assert_eq!(destination.source_position(), old)
                    }
                    Err(other) => panic!("unexpected {other:?}"),
                }
                assert!(
                    budget.used().bytes <= 6
                        && budget.used().queries <= 1
                        && budget.used().objects <= 1
                );
            }
            assert_eq!(destination.source_position(), info.label.end);
            assert_eq!(shown, doc.text(info.label).unwrap());
            assert_eq!(target, shown);
            assert_eq!(doc.link_destination(id), Some(target.as_str()));
            let expected_url = if matches!(doc.links()[i], LinkTarget::Email(_)) {
                format!("mailto:{target}")
            } else {
                target
            };
            assert_eq!(doc.link_url(id).unwrap(), expected_url);
        }
    }
}
