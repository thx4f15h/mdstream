//! Offline scoring of CommonMark 0.31.2 Images (572–593).
//! Only public source ranges, kinds, display runs and decoded metadata are used.
use mdstream::{
    BlockKind, Budget, Completion, DisplayStep, Document, LinkId, LinkKind, LinkTarget, Span,
    Style, WorkLimit,
};

#[path = "support/snapshot.rs"]
mod support;
use support::snapshot;

include!("fixtures/commonmark_images.rs");

const SIX_BYTES: WorkLimit = WorkLimit {
    bytes: 6,
    queries: 1,
    objects: 1,
    tiles: 0,
    recovery: 0,
    dependencies: 0,
};

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn escape(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

fn metadata(doc: &Document, id: LinkId, limit: WorkLimit) -> Option<(String, Option<String>)> {
    let mut destination = doc.decoded_link_destination(id)?;
    let mut budget = Budget::new(limit);
    // Both readers share the same budget. Probe exhaustion may leave the source
    // position unchanged, so bound attempts by source size, not emitted chunks.
    macro_rules! read {
        ($cursor:expr, $raw:expr) => {{
            let cursor = &mut $cursor;
            let raw = $raw;
            let start = cursor.source_position();
            let mut end = start;
            let mut output = String::new();
            let mut steps = 0;
            loop {
                steps += 1;
                assert!(steps <= 4 * raw.len() + 3, "metadata did not progress");
                match cursor.advance(&mut budget) {
                    Ok(Some(chunk)) => {
                        assert_eq!(chunk.source.start, end);
                        assert!(doc.text(chunk.source).is_some());
                        end = chunk.source.end;
                        output.push_str(chunk.text);
                    }
                    Ok(None) => break,
                    Err(Completion::BudgetExhausted) => {
                        assert_ne!(budget.used(), WorkLimit::default());
                        assert!(budget.used().bytes <= limit.bytes);
                        assert!(budget.used().queries <= limit.queries);
                        assert!(budget.used().objects <= limit.objects);
                        budget = Budget::new(limit);
                    }
                    Err(other) => panic!("unexpected {other:?}"),
                }
                assert_eq!(cursor.source_position(), end);
                assert!(budget.used().bytes <= limit.bytes);
                assert!(budget.used().queries <= limit.queries);
                assert!(budget.used().objects <= limit.objects);
            }
            assert!(budget.used().bytes <= limit.bytes);
            assert!(budget.used().queries <= limit.queries);
            assert!(budget.used().objects <= limit.objects);
            assert_eq!(end - start, raw.len());
            output
        }};
    }
    let destination = read!(destination, doc.link_destination(id).unwrap());
    let title = doc
        .decoded_link_title(id)
        .map(|mut cursor| read!(cursor, doc.link_title(id).unwrap()));
    Some((destination, title))
}

fn metadata_snapshot(doc: &Document) -> Vec<Option<(String, Option<String>)>> {
    (0..doc.links().len())
        .map(|i| {
            let tiny = metadata(doc, LinkId(i), SIX_BYTES);
            assert_eq!(tiny, metadata(doc, LinkId(i), WorkLimit::unlimited()));
            tiny
        })
        .collect()
}

// A deliberately narrow output encoder: the chapter has ASCII URI targets.
// Do not silently claim coverage of general URI serialization.
fn url(text: &str) -> String {
    assert!(
        text.bytes()
            .all(|b| b.is_ascii_graphic() && !b"<>\"`\\".contains(&b)),
        "unscored URI encoding"
    );
    escape(text)
}

fn text(doc: &Document, span: Span, alt: bool, terminator: bool) -> String {
    let mut out = String::new();
    let mut active = None;
    for run in doc.display_runs_in(span).unwrap() {
        if run.style.contains(Style::HIDDEN) {
            continue;
        }
        assert!(!run.style.contains(Style::STRIKE) && !run.style.contains(Style::MATH));
        let tags: Vec<_> = [
            (Style::EMPHASIS, "em"),
            (Style::STRONG, "strong"),
            (Style::CODE, "code"),
        ]
        .into_iter()
        .filter(|(flag, _)| run.style.contains(*flag))
        .collect();
        assert!(tags.len() <= 1, "unscored nested inline styles");
        let tag = (!alt).then(|| tags.first().map(|(_, tag)| *tag)).flatten();
        if tag != active {
            if let Some(old) = active {
                out.push_str(&format!("</{old}>"));
            }
            if let Some(new) = tag {
                out.push_str(&format!("<{new}>"));
            }
            active = tag;
        }
        let payload = if terminator
            && run.source.end == span.end
            && run.replacement.is_none()
            && run.text.ends_with('\n')
            && doc.text(run.source).unwrap().ends_with('\n')
        {
            run.text.strip_suffix('\n').unwrap()
        } else {
            run.text
        };
        for ch in payload.chars() {
            if !alt && ch == '\n' && run.style.contains(Style::HARD_BREAK) {
                out.push_str("<br />");
            }
            out.push_str(&escape(ch.encode_utf8(&mut [0; 4])));
        }
    }
    if let Some(tag) = active {
        out.push_str(&format!("</{tag}>"));
    }
    out
}

fn project(doc: &Document) -> String {
    assert_eq!(doc.container_count(), 0);
    assert!(doc.headings().entries().is_empty());
    assert!(doc.tables().is_empty() && doc.maths().is_empty() && doc.footnotes().is_empty());
    let metadata = metadata_snapshot(doc);
    let mut objects: Vec<_> = metadata
        .iter()
        .enumerate()
        .filter_map(|(i, data)| {
            assert!(!matches!(doc.links()[i], LinkTarget::Email(_)));
            data.as_ref()
                .map(|data| (doc.link_info(LinkId(i)).unwrap(), data))
        })
        .collect();
    objects.sort_by_key(|(info, _)| info.source.start);
    let mut objects = objects.into_iter().peekable();
    let mut html = String::new();
    for block in doc.blocks() {
        match block.kind {
            BlockKind::Blank => continue,
            BlockKind::Definition { .. } => {
                assert!(
                    doc.display_runs_in(block.source)
                        .unwrap()
                        .all(|r| r.style.contains(Style::HIDDEN))
                );
                continue;
            }
            BlockKind::Paragraph => {}
            ref other => panic!("unscored block {other:?}"),
        }
        html.push_str("<p>");
        let mut position = block.source.start;
        while objects
            .peek()
            .is_some_and(|(info, _)| info.source.start < block.source.end)
        {
            let (info, (destination, title)) = objects.next().unwrap();
            assert!(
                position <= info.source.start && info.source.end <= block.source.end,
                "overlapping or cross-block objects"
            );
            assert!(info.source.start <= info.label.start && info.label.end <= info.source.end);
            html.push_str(&text(
                doc,
                Span::new(position, info.source.start),
                false,
                false,
            ));
            // A source envelope is not permission to discard visible syntax.
            // Both syntax regions outside the public label must be hidden.
            for span in [
                Span::new(info.source.start, info.label.start),
                Span::new(info.label.end, info.source.end),
            ] {
                assert!(
                    doc.display_runs_in(span)
                        .unwrap()
                        .all(|run| run.style.contains(Style::HIDDEN)),
                    "visible syntax outside object label"
                );
            }
            let image = info.kind == LinkKind::Image;
            html.push_str(if image { "<img src=\"" } else { "<a href=\"" });
            html.push_str(&url(destination));
            html.push('"');
            if image {
                html.push_str(" alt=\"");
                html.push_str(&text(doc, info.label, true, false));
                html.push('"');
            }
            if let Some(title) = title {
                html.push_str(" title=\"");
                html.push_str(&escape(title));
                html.push('"');
            }
            if image {
                html.push_str(" />");
            } else {
                html.push('>');
                html.push_str(&text(doc, info.label, false, false));
                html.push_str("</a>");
            }
            position = info.source.end;
        }
        html.push_str(&text(
            doc,
            Span::new(position, block.source.end),
            false,
            true,
        ));
        html.push_str("</p>\n");
    }
    assert!(objects.next().is_none(), "unscored object");
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

// Frozen observations are separate from the verbatim upstream oracle.
fn assessment(number: usize) -> Assessment {
    match number {
        572..=573 | 576..=589 | 591..=593 => Assessment::Supported,
        574 => Assessment::Unsupported {
            reason: "a nested image replaces the outer bracket candidate",
            observed: "<p>![foo <img src=\"/url\" alt=\"bar\" />](/url2)</p>\n",
        },
        575 => Assessment::Unsupported {
            reason: "a nested link replaces the outer image bracket candidate",
            observed: "<p>![foo <a href=\"/url\">bar</a>](/url2)</p>\n",
        },
        590 => Assessment::DeliberateDifference {
            reason: "unresolved references keep the existing hidden-bracket dialect",
            observed: "<p>![foo]</p>\n<p>[foo]: /url &quot;title&quot;</p>\n",
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
    metadata_snapshot(doc);
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
fn pinned_commonmark_images_report() {
    assert_eq!(IMAGE_CASES.len(), 22);
    let mut counts = [0; 3];
    for (index, &(number, markdown, expected)) in IMAGE_CASES.iter().enumerate() {
        assert_eq!(number, 572 + index);
        check_case(number, markdown, expected, &parse(markdown));
        let (category, label, reason) = match assessment(number) {
            Assessment::Supported => (0, "supported", "matches upstream"),
            Assessment::DeliberateDifference { reason, .. } => (1, "deliberate difference", reason),
            Assessment::Unsupported { reason, .. } => (2, "unsupported", reason),
        };
        counts[category] += 1;
        eprintln!("example {number}: {label} — {reason}");
    }
    assert_eq!(counts, [19, 1, 2]);
    eprintln!(
        "CommonMark 0.31.2 Images (572–593): 19 supported, 1 deliberate difference, 2 unsupported; other sections not scored by this test"
    );
}

#[test]
fn pinned_commonmark_images_every_split_and_scalar_prefix() {
    let mut splits = 0;
    let mut prefixes = 0;
    for &(number, markdown, expected) in IMAGE_CASES {
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
            assert_eq!(metadata_snapshot(&doc), metadata_snapshot(&prefix));
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
                metadata_snapshot(&finished),
                metadata_snapshot(&whole_prefix)
            );
            prefixes += 1;
        }
        doc.finish();
        check_case(number, markdown, expected, &doc);
        assert_eq!(snapshot(&doc), snapshot(&whole));
    }
    assert_eq!((splits, prefixes), (723, 701));
    eprintln!(
        "22 examples: 723 single splits and 701 scalar prefixes preserve classified semantics and source metadata"
    );
}

#[test]
fn image_samples_preserve_mapping_when_sliced_and_read_with_six_byte_budgets() {
    let limit = WorkLimit {
        bytes: 6,
        queries: 1,
        objects: 1,
        ..Default::default()
    };
    for &(_, markdown, _) in IMAGE_CASES {
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
fn image_projection_preserves_empty_alt_styles_whitespace_and_metadata() {
    for (source, expected) in [
        ("", ""),
        ("a  b\tc\n", "<p>a  b\tc</p>\n"),
        ("a\nb\n", "<p>a\nb</p>\n"),
        ("a  \nb\n", "<p>a<br />\nb</p>\n"),
        ("&#10;\n", "<p>\n</p>\n"),
        (
            "![]() ![](/x)\n",
            "<p><img src=\"\" alt=\"\" /> <img src=\"/x\" alt=\"\" /></p>\n",
        ),
        (
            "![  a  b  ](/x)\n",
            "<p><img src=\"/x\" alt=\"  a  b  \" /></p>\n",
        ),
        (
            "![*em* **strong** `code`](/x)\n",
            "<p><img src=\"/x\" alt=\"em strong code\" /></p>\n",
        ),
        ("![a\nb](/x)\n", "<p><img src=\"/x\" alt=\"a\nb\" /></p>\n"),
        (
            "![中&amp;\\*](/a\\+b?x=1&amp;y=2 't&amp;\\\"')\n",
            "<p><img src=\"/a+b?x=1&amp;y=2\" alt=\"中&amp;*\" title=\"t&amp;&quot;\" /></p>\n",
        ),
        (
            "![a](/x '')\n",
            "<p><img src=\"/x\" alt=\"a\" title=\"\" /></p>\n",
        ),
        ("[a](/x)\n", "<p><a href=\"/x\">a</a></p>\n"),
        ("![missing]\n", "<p>missing</p>\n"),
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
    // Changing any object property must change the projection; it cannot fill
    // in absent objects or recover Markdown nesting from the input string.
    let baseline = project(&parse("![a](/x 't')\n"));
    for changed in [
        "[a](/x 't')\n",
        "![b](/x 't')\n",
        "![a](/y 't')\n",
        "![a](/x 'u')\n",
        "![a](/x)\n",
        "![](/x 't')\n",
    ] {
        assert_ne!(project(&parse(changed)), baseline);
    }
}

#[test]
fn image_late_definitions_preserve_ids_raw_spans_and_first_definition() {
    let mut doc = Document::new();
    doc.append("![Foo]\n\n![Foo][]\n\n![a][Foo]\n\n").unwrap();
    assert_eq!(doc.links().len(), 3);
    let links = doc.links().to_vec();
    let infos: Vec<_> = (0..3).map(|i| *doc.link_info(LinkId(i)).unwrap()).collect();
    assert_eq!(metadata_snapshot(&doc), vec![None; 3]);
    assert_eq!(project(&doc), "<p>Foo</p>\n<p>Foo</p>\n<p>a</p>\n");
    doc.append("[FOO]: /a&amp;b 't&amp;'\n").unwrap();
    let revision = doc.references()[0].revision;
    for (i, info) in infos.iter().enumerate() {
        let id = LinkId(i);
        assert_eq!(doc.link_info(id), Some(info));
        assert_eq!(info.kind, LinkKind::Image);
        assert_eq!(doc.link_destination(id), Some("/a&amp;b"));
        assert_eq!(doc.link_title(id), Some("t&amp;"));
        assert_eq!(
            metadata(&doc, id, SIX_BYTES),
            Some(("/a&b".to_owned(), Some("t&".to_owned())))
        );
    }
    let resolved = project(&doc);
    assert_eq!(
        resolved,
        "<p><img src=\"/a&amp;b\" alt=\"Foo\" title=\"t&amp;\" /></p>\n<p><img src=\"/a&amp;b\" alt=\"Foo\" title=\"t&amp;\" /></p>\n<p><img src=\"/a&amp;b\" alt=\"a\" title=\"t&amp;\" /></p>\n"
    );
    doc.append("[foo]: /ignored 'ignored'\n").unwrap();
    doc.finish();
    assert_eq!(doc.links(), links);
    assert_eq!(doc.references()[0].revision, revision);
    assert_eq!(project(&doc), resolved);
}

#[test]
fn image_projection_rejects_unscored_structure_and_uri_encoding() {
    for source in [
        "- a\n",
        "> a\n",
        "# h\n",
        "***\n",
        "~~~\nx\n~~~\n",
        "a|b\n--|--\n",
        "~~strike~~\n",
        "$math$\n",
        "[^f]: footnote\n",
        "<a@b.org>\n",
        "![a](<a b>)\n",
        "![a](/中)\n",
    ] {
        assert!(
            std::panic::catch_unwind(|| project(&parse(source))).is_err(),
            "accepted {source:?}"
        );
    }
}
