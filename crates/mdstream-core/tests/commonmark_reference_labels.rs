//! Offline CommonMark 0.31.2 Links reference label matching subset (539–552).
//! This is 14 examples, not the complete Links section (482–571).
use mdstream::{
    BlockKind, Budget, Completion, DisplayStep, Document, LinkId, LinkKind, LinkTarget, Span,
    Style, WorkLimit,
};
#[path = "support/snapshot.rs"]
mod support;
use support::snapshot;
include!("fixtures/commonmark_reference_labels.rs");
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

// Only output encoding: never decode Markdown or alter the core target.
// This subset needs spaces, double quotes, backslashes and UTF-8 encoded.
fn url(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        assert!(
            !ch.is_ascii_control() && !matches!(ch, '<' | '>' | '`'),
            "unscored URI encoding"
        );
        if !ch.is_ascii() || matches!(ch, ' ' | '"' | '\\') {
            const HEX: &[u8] = b"0123456789ABCDEF";
            for byte in ch.encode_utf8(&mut [0; 4]).bytes() {
                out.push('%');
                out.push(char::from(HEX[usize::from(byte >> 4)]));
                out.push(char::from(HEX[usize::from(byte & 15)]));
            }
        } else {
            out.push_str(&escape(ch.encode_utf8(&mut [0; 4])));
        }
    }
    out
}

fn text(doc: &Document, span: Span, terminator: bool) -> String {
    let mut out = String::new();
    for run in doc.display_runs_in(span).unwrap() {
        if run.style.contains(Style::HIDDEN) {
            continue;
        }
        for flag in [
            Style::STRIKE,
            Style::MATH,
            Style::EMPHASIS,
            Style::STRONG,
            Style::CODE,
        ] {
            assert!(!run.style.contains(flag), "unscored inline style");
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
            if ch == '\n' && run.style.contains(Style::HARD_BREAK) {
                out.push_str("<br />");
            }
            out.push_str(&escape(ch.encode_utf8(&mut [0; 4])));
        }
    }
    out
}

// The projector receives only the document, never example IDs or expected HTML.
// Empty labels are emitted from public object envelopes, not inferred from text.
fn project(doc: &Document) -> String {
    assert_eq!(doc.container_count(), 0);
    assert!(doc.headings().entries().is_empty());
    assert!(doc.tables().is_empty() && doc.maths().is_empty() && doc.footnotes().is_empty());
    let metadata = metadata_snapshot(doc);
    let mut objects: Vec<_> = metadata
        .iter()
        .enumerate()
        .filter_map(|(i, data)| {
            let info = doc.link_info(LinkId(i)).unwrap();
            assert_eq!(info.kind, LinkKind::Link);
            match doc.links()[i] {
                LinkTarget::Direct(span) => assert_ne!(span, info.label, "unscored autolink"),
                LinkTarget::Reference(_) => {}
                _ => panic!("unscored autolink"),
            }
            data.as_ref().map(|data| (info, data))
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
            html.push_str(&text(doc, Span::new(position, info.source.start), false));
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
            html.push_str("<a href=\"");
            html.push_str(&url(destination));
            html.push('"');
            if let Some(title) = title {
                html.push_str(" title=\"");
                html.push_str(&escape(title));
                html.push('"');
            }
            html.push('>');
            html.push_str(&text(doc, info.label, false));
            html.push_str("</a>");
            position = info.source.end;
        }
        html.push_str(&text(doc, Span::new(position, block.source.end), true));
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
        539 | 544 | 549..=552 => Assessment::Supported,
        540 => Assessment::DeliberateDifference {
            reason: "reference labels use Unicode lowercase, not full Unicode case folding",
            observed: "<p>ẞ</p>\n",
        },
        541 => Assessment::Unsupported {
            reason: "reference definition labels cannot span physical lines",
            observed: "<p>Foo\nbar: /url</p>\n<p>Baz</p>\n",
        },
        542 => Assessment::DeliberateDifference {
            reason: "unresolved shortcut references retain hidden brackets before a spaced reference",
            observed: "<p>foo <a href=\"/url\" title=\"title\">bar</a></p>\n",
        },
        543 => Assessment::DeliberateDifference {
            reason: "unresolved shortcut references retain hidden brackets before a next-line reference",
            observed: "<p>foo\n<a href=\"/url\" title=\"title\">bar</a></p>\n",
        },
        545 => Assessment::DeliberateDifference {
            reason: "raw reference-label spelling is preserved; unresolved full references hide their syntax",
            observed: "<p>bar</p>\n",
        },
        546 => Assessment::DeliberateDifference {
            reason: "invalid definition remains content; unresolved shortcut reference still hides brackets",
            observed: "<p>foo[ref[]</p>\n<p>[ref[]: /uri</p>\n",
        },
        547 => Assessment::DeliberateDifference {
            reason: "single nested candidate and unresolved-reference hidden-bracket fallback",
            observed: "<p>foo[refbar]</p>\n<p>[refbar]: /uri</p>\n",
        },
        548 => Assessment::DeliberateDifference {
            reason: "single nested candidate and unresolved-reference hidden-bracket fallback",
            observed: "<p>[[foo]]</p>\n<p>[[foo]]: /url</p>\n",
        },
        _ => panic!("unclassified upstream example {number}"),
    }
}

fn check_mapping(markdown: &str, source: Span, text: &str, replacement: Option<Span>) {
    // These selected examples use exact borrowed runs, including hidden prefixes.
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
fn pinned_commonmark_reference_labels_report() {
    assert_eq!(REFERENCE_LABEL_CASES.len(), 14);
    let mut counts = [0; 3];
    for (index, &(number, markdown, expected)) in REFERENCE_LABEL_CASES.iter().enumerate() {
        assert_eq!(number, 539 + index);
        check_case(number, markdown, expected, &parse(markdown));
        let (category, label, reason) = match assessment(number) {
            Assessment::Supported => (0, "supported", "matches upstream"),
            Assessment::DeliberateDifference { reason, .. } => (1, "deliberate difference", reason),
            Assessment::Unsupported { reason, .. } => (2, "unsupported", reason),
        };
        counts[category] += 1;
        eprintln!("example {number}: {label} — {reason}");
    }
    assert_eq!(counts, [6, 7, 1]);
    eprintln!(
        "CommonMark 0.31.2 Links subset (539–552): 6 supported, 7 deliberate differences, 1 unsupported; 482–538 and 553–571 outside this score"
    );
}

#[test]
fn pinned_commonmark_reference_labels_every_split_and_scalar_prefix() {
    let mut splits = 0;
    let mut prefixes = 0;
    for &(number, markdown, expected) in REFERENCE_LABEL_CASES {
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
    assert_eq!((splits, prefixes), (396, 382));
    eprintln!(
        "14 examples: 396 single splits and 382 scalar prefixes preserve classified semantics and source metadata"
    );
}

#[test]
fn reference_label_samples_preserve_mapping_when_sliced_and_read_with_six_byte_budgets() {
    let limit = WorkLimit {
        bytes: 6,
        queries: 1,
        objects: 1,
        ..Default::default()
    };
    for &(_, markdown, _) in REFERENCE_LABEL_CASES {
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
fn reference_label_projection_preserves_whitespace_matching_and_decoded_metadata() {
    for (source, expected) in [
        ("", ""),
        (
            "[x][ A  B ]\n\n[a b]: /u\n",
            "<p><a href=\"/u\">x</a></p>\n",
        ),
        ("[É]\n\n[é]: /u\n", "<p><a href=\"/u\">É</a></p>\n"),
        ("[SS]\n\n[ẞ]: /u\n", "<p>SS</p>\n"),
        (
            "[x][r] [r][] [r]\n\n[r]: /u 't'\n",
            "<p><a href=\"/u\" title=\"t\">x</a> <a href=\"/u\" title=\"t\">r</a> <a href=\"/u\" title=\"t\">r</a></p>\n",
        ),
        ("a  b\tc\n", "<p>a  b\tc</p>\n"),
        ("a\nb\n", "<p>a\nb</p>\n"),
        ("a  \nb\n", "<p>a<br />\nb</p>\n"),
        ("&#10;\n", "<p>\n</p>\n"),
        (
            "[]() [](/x '')\n",
            "<p><a href=\"\"></a> <a href=\"/x\" title=\"\"></a></p>\n",
        ),
        ("[  a  b  ](/x)\n", "<p><a href=\"/x\">  a  b  </a></p>\n"),
        ("[a\nb](/x)\n", "<p><a href=\"/x\">a\nb</a></p>\n"),
        (
            "[中&amp;\\*](<a b%20中\\\\> 't&amp;\\\"')\n",
            "<p><a href=\"a%20b%20%E4%B8%AD%5C\" title=\"t&amp;&quot;\">中&amp;*</a></p>\n",
        ),
        ("[a](/x\n", "<p>a(/x</p>\n"),
        ("[a](/x 't\n", "<p>a(/x 't</p>\n"),
        ("[a] (<x>)\n", "<p>a (&lt;x&gt;)</p>\n"),
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
    let baseline = project(&parse("[a](/x 't')\n"));
    for changed in [
        "[b](/x 't')\n",
        "[a](/y 't')\n",
        "[a](/x 'u')\n",
        "[a](/x)\n",
        "[](/x 't')\n",
        "[a](/x '')\n",
        "[a](/x 't'\n",
    ] {
        assert_ne!(project(&parse(changed)), baseline);
    }
    let baseline = project(&parse("[x][r]\n\n[r]: /u 't'\n"));
    for changed in [
        "[y][r]\n\n[r]: /u 't'\n",
        "[x][s]\n\n[r]: /u 't'\n",
        "[x][r]\n\n[r]: /v 't'\n",
        "[x][r]\n\n[r]: /u 'other'\n",
        "[x][r]\n\n[r]: /u\n",
        "[x][r]\n\n[r]: /first\n[r]: /u 't'\n",
    ] {
        assert_ne!(project(&parse(changed)), baseline);
    }
    // Output escaping must neither decode existing percent sequences nor make
    // non-ASCII whitespace into a title separator.
    assert_eq!(url("/%20%2f&中\"\\ "), "/%20%2f&amp;%E4%B8%AD%22%5C%20");
}

#[test]
fn reference_label_projection_rejects_out_of_subset_structure() {
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
        "![a](/x)\n",
        "<https://example.com>\n",
        "<a@b.org>\n",
        "[*a*](/x)\n",
        "[**a**](/x)\n",
        "[`a`](/x)\n",
        "[a](<a`b>)\n",
        "[a](<a\\<b>)\n",
    ] {
        assert!(
            std::panic::catch_unwind(|| project(&parse(source))).is_err(),
            "accepted {source:?}"
        );
    }
}

#[test]
fn reference_label_raw_spans_identity_and_shared_display_destination_title_budget() {
    let extra = "[中&amp;\\*](<a b%20中\\\\> 't&amp;\\\"') []() [](/x '')\n";
    for source in REFERENCE_LABEL_CASES
        .iter()
        .map(|(_, source, _)| *source)
        .chain([extra])
    {
        let mut doc = Document::new();
        doc.append(source).unwrap();
        doc.append("\n").unwrap();
        let infos: Vec<_> = (0..doc.links().len())
            .map(|i| *doc.link_info(LinkId(i)).unwrap())
            .collect();
        let targets = doc.links().to_vec();
        let before = metadata_snapshot(&doc);
        // Appending independent syntax cannot renumber or rewrite old objects.
        doc.append("later [new](/new 'title')\n").unwrap();
        doc.finish();
        assert_eq!(&doc.links()[..targets.len()], targets);
        assert_eq!(&metadata_snapshot(&doc)[..before.len()], before);
        for (i, info) in infos.iter().enumerate() {
            let id = LinkId(i);
            assert_eq!(doc.link_info(id), Some(info));
            let Some((expected_destination, expected_title)) = &before[i] else {
                continue;
            };
            let start = doc.decoded_link_destination(id).unwrap().source_position();
            let raw_span = Span::new(start, start + doc.link_destination(id).unwrap().len());
            assert_eq!(doc.link_destination(id), doc.text(raw_span));
            let mut display = doc.display_cursor(info.label).unwrap();
            let mut destination = doc.decoded_link_destination(id).unwrap();
            let mut title = doc.decoded_link_title(id);
            assert_eq!(destination.source_position(), raw_span.start);
            let title_start = title.as_ref().map(|cursor| cursor.source_position());
            let expected_display: String = doc
                .display_runs_in(info.label)
                .unwrap()
                .filter(|r| !r.style.contains(Style::HIDDEN))
                .map(|r| r.text)
                .collect();
            let mut shown = String::new();
            let mut decoded_destination = String::new();
            let mut decoded_title = String::new();
            let mut display_done = false;
            let mut destination_done = false;
            let mut title_done = title.is_none();
            let mut frames = 0;
            while !display_done || !destination_done || !title_done {
                frames += 1;
                assert!(
                    frames
                        <= 4 * (info.source.len()
                            + expected_destination.len()
                            + expected_title.as_ref().map_or(0, String::len))
                            + 8,
                    "shared readers stalled"
                );
                let mut budget = Budget::new(SIX_BYTES);
                if !display_done {
                    match display.next(&doc, &mut budget).unwrap() {
                        DisplayStep::Run(run) => {
                            assert!(
                                info.label.start <= run.source.start
                                    && run.source.end <= info.label.end
                            );
                            if !run.style.contains(Style::HIDDEN) {
                                shown.push_str(run.text);
                            }
                        }
                        DisplayStep::Complete => display_done = true,
                        other => panic!("fresh frame failed display: {other:?}"),
                    }
                }
                if !destination_done {
                    let old = destination.source_position();
                    match destination.advance(&mut budget) {
                        Ok(Some(chunk)) => {
                            assert_eq!(chunk.source.start, old);
                            assert!(doc.text(chunk.source).is_some());
                            decoded_destination.push_str(chunk.text);
                            let end = chunk.source.end;
                            assert_eq!(destination.source_position(), end);
                        }
                        Ok(None) => destination_done = true,
                        Err(Completion::BudgetExhausted) => {
                            assert_eq!(destination.source_position(), old)
                        }
                        Err(other) => panic!("unexpected {other:?}"),
                    }
                }
                if !title_done {
                    let cursor = title.as_mut().unwrap();
                    let old = cursor.source_position();
                    match cursor.advance(&mut budget) {
                        Ok(Some(chunk)) => {
                            assert_eq!(chunk.source.start, old);
                            assert!(doc.text(chunk.source).is_some());
                            decoded_title.push_str(chunk.text);
                            let end = chunk.source.end;
                            assert_eq!(cursor.source_position(), end);
                        }
                        Ok(None) => title_done = true,
                        Err(Completion::BudgetExhausted) => {
                            assert_eq!(cursor.source_position(), old)
                        }
                        Err(other) => panic!("unexpected {other:?}"),
                    }
                }
                assert!(
                    budget.used().bytes <= 6
                        && budget.used().queries <= 1
                        && budget.used().objects <= 1
                );
            }
            assert_eq!(display.source_position(), info.label.end);
            assert_eq!(destination.source_position(), raw_span.end);
            assert_eq!(shown, expected_display);
            assert_eq!(&decoded_destination, expected_destination);
            assert_eq!(
                title.as_ref().map(|_| &decoded_title),
                expected_title.as_ref()
            );
            if let Some(cursor) = title {
                let span = Span::new(title_start.unwrap(), cursor.source_position());
                assert_eq!(doc.text(span), doc.link_title(id));
            }
        }
    }
    let doc = parse(extra);
    assert_eq!(doc.link_destination(LinkId(0)), Some("a b%20中\\\\"));
    assert_eq!(doc.link_title(LinkId(0)), Some("t&amp;\\\""));
    assert_eq!(
        metadata(&doc, LinkId(0), SIX_BYTES),
        Some(("a b%20中\\".to_owned(), Some("t&\"".to_owned())))
    );
}

#[test]
fn late_definitions_preserve_normalized_symbols_identity_and_first_definition() {
    let source = "[one][ A\tB ] [two][a b] [É] [ẞ] [raw][r\\[] [plain][r!] [escaped][r\\!]\n\n";
    let mut doc = Document::new();
    doc.append(source).unwrap();
    assert_eq!(doc.links().len(), 7);
    let targets = doc.links().to_vec();
    let infos: Vec<_> = (0..7).map(|i| *doc.link_info(LinkId(i)).unwrap()).collect();
    let symbols: Vec<_> = targets
        .iter()
        .map(|target| match target {
            LinkTarget::Reference(symbol) => *symbol,
            _ => panic!("expected reference"),
        })
        .collect();
    assert_eq!(symbols[0], symbols[1]);
    assert_ne!(symbols[5], symbols[6]);
    for (i, label) in ["a b", "a b", "é", "ß", "r\\[", "r!", "r\\!"]
        .into_iter()
        .enumerate()
    {
        assert_eq!(doc.references()[symbols[i].0].label, label);
        assert_eq!(doc.link_destination(LinkId(i)), None);
    }
    let definition_start = doc.source().len();
    let definitions = "[a b]: /first&amp; 't\\*'\n[é]: /unicode\n[SS]: /casefold\n[r\\[]: /bracket\n[r!]: /plain\n[r\\!]: /escaped\n";
    // Observe stable objects while each late definition is still incomplete.
    for ch in definitions.chars() {
        doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
        assert_eq!(doc.links(), targets);
        for (i, info) in infos.iter().enumerate() {
            assert_eq!(doc.link_info(LinkId(i)), Some(info));
        }
    }
    let expected = [
        Some("/first&"),
        Some("/first&"),
        Some("/unicode"),
        None,
        Some("/bracket"),
        Some("/plain"),
        Some("/escaped"),
    ];
    let before = metadata_snapshot(&doc);
    for (i, destination) in expected.into_iter().enumerate() {
        assert_eq!(before[i].as_ref().map(|data| data.0.as_str()), destination);
        if destination.is_some() {
            let reference = &doc.references()[symbols[i].0];
            let span = reference.destination.unwrap();
            assert!(span.start >= definition_start);
            assert_eq!(doc.text(span), doc.link_destination(LinkId(i)));
            assert_eq!(
                doc.decoded_link_destination(LinkId(i))
                    .unwrap()
                    .source_position(),
                span.start
            );
        }
    }
    assert_eq!(before[0].as_ref().unwrap().1.as_deref(), Some("t*"));
    let winners: Vec<_> = symbols
        .iter()
        .map(|id| doc.references()[id.0].clone())
        .collect();
    doc.append("\n[A B]: /second 'ignored'\n[É]: /second\n[r\\[]: /second\n")
        .unwrap();
    doc.finish();
    assert_eq!(doc.links(), targets);
    assert_eq!(metadata_snapshot(&doc), before);
    for (i, symbol) in symbols.iter().enumerate() {
        assert_eq!(doc.references()[symbol.0], winners[i]);
        assert_eq!(doc.link_info(LinkId(i)), Some(&infos[i]));
    }
}
