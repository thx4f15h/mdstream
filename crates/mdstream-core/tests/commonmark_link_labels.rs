//! Offline scoring of CommonMark 0.31.2 Links label/nesting subset (512–538).
//! Only public source ranges, kinds, display runs and decoded metadata are used.
use mdstream::{
    BlockKind, Budget, Completion, DisplayStep, Document, LinkId, LinkKind, LinkTarget, Span,
    Style, WorkLimit,
};

#[path = "support/snapshot.rs"]
mod support;
use support::snapshot;

include!("fixtures/commonmark_link_labels.rs");

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
// This subset also needs square brackets in autolink targets percent-encoded.
fn url(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        assert!(
            !ch.is_ascii_control() && !matches!(ch, '<' | '>' | '`'),
            "unscored URI encoding"
        );
        if !ch.is_ascii() || matches!(ch, ' ' | '"' | '\\' | '[' | ']') {
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

// The subset has em outside strong/code. Public style flags do not carry
// arbitrary delimiter ancestry: reject other/ambiguous nesting, never reparse it.
const STYLES: [(Style, &str); 3] = [
    (Style::EMPHASIS, "em"),
    (Style::STRONG, "strong"),
    (Style::CODE, "code"),
];

#[derive(Default)]
struct Text {
    html: String,
    active: Vec<&'static str>,
}

impl Text {
    fn transition(&mut self, tags: Vec<&'static str>) {
        let common = self
            .active
            .iter()
            .zip(&tags)
            .take_while(|(a, b)| a == b)
            .count();
        assert!(
            tags.len() - common <= 1,
            "unscored ambiguous or reversed style nesting"
        );
        for tag in self.active[common..].iter().rev() {
            self.html.push_str(&format!("</{tag}>"));
        }
        for tag in &tags[common..] {
            self.html.push_str(&format!("<{tag}>"));
        }
        self.active = tags;
    }

    fn write(
        &mut self,
        doc: &Document,
        span: Span,
        alt: bool,
        terminator: bool,
        inherited: &[&str],
    ) {
        for run in doc.display_runs_in(span).unwrap() {
            if run.style.contains(Style::HIDDEN) {
                continue;
            }
            assert!(!run.style.contains(Style::STRIKE) && !run.style.contains(Style::MATH));
            let tags = STYLES
                .iter()
                .filter_map(|&(flag, tag)| {
                    (!alt && run.style.contains(flag) && !inherited.contains(&tag)).then_some(tag)
                })
                .collect();
            self.transition(tags);
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
                    self.html.push_str("<br />");
                }
                self.html.push_str(&escape(ch.encode_utf8(&mut [0; 4])));
            }
        }
    }

    fn finish(mut self) -> String {
        self.transition(Vec::new());
        self.html
    }
}

fn label(doc: &Document, span: Span, alt: bool, inherited: &[&str]) -> String {
    let mut text = Text::default();
    text.write(doc, span, alt, false, inherited);
    text.finish()
}

// Only styles covering every byte of the public object envelope can wrap that
// object. Crossing styles remain inside each text region; no delimiter is moved,
// restored or inferred. Hidden syntax participates in this containment check.
fn enclosing_styles(doc: &Document, span: Span) -> Vec<&'static str> {
    STYLES
        .iter()
        .filter_map(|&(flag, tag)| {
            doc.display_runs_in(span)
                .unwrap()
                .all(|run| run.style.contains(flag))
                .then_some(tag)
        })
        .collect()
}

// No example IDs or oracle strings enter the projection. It does not construct
// missing outer objects, decode Markdown, restore hidden brackets or trim text.
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
        let mut text = Text::default();
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
            text.write(
                doc,
                Span::new(position, info.source.start),
                false,
                false,
                &[],
            );
            let inherited = enclosing_styles(doc, info.source);
            text.transition(inherited.clone());
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
            text.html
                .push_str(if image { "<img src=\"" } else { "<a href=\"" });
            text.html.push_str(&url(destination));
            text.html.push('"');
            if image {
                text.html.push_str(" alt=\"");
                text.html
                    .push_str(&label(doc, info.label, true, &inherited));
                text.html.push('"');
            }
            if let Some(title) = title {
                text.html.push_str(" title=\"");
                text.html.push_str(&escape(title));
                text.html.push('"');
            }
            if image {
                text.html.push_str(" />");
            } else {
                text.html.push('>');
                text.html
                    .push_str(&label(doc, info.label, false, &inherited));
                text.html.push_str("</a>");
            }
            position = info.source.end;
        }
        text.write(doc, Span::new(position, block.source.end), false, true, &[]);
        html.push_str(&text.finish());
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

// Frozen observations are reviewed separately from the verbatim upstream oracle.
fn assessment(number: usize) -> Assessment {
    match number {
        514..=516 | 518..=519 | 522 | 525..=527 | 529..=530 | 532..=533 | 537..=538 => {
            Assessment::Supported
        }
        512 => Assessment::Unsupported {
            reason: "nested unescaped brackets replace the outer link candidate",
            observed: "<p>[link [foo bar]](/uri)</p>\n",
        },
        513 => Assessment::DeliberateDifference {
            reason: "unresolved shortcut references keep the hidden-bracket dialect",
            observed: "<p>link bar](/uri)</p>\n",
        },
        517 => Assessment::Unsupported {
            reason: "an inner image replaces the outer link candidate",
            observed: "<p>[<img src=\"moon.jpg\" alt=\"moon\" />](/uri)</p>\n",
        },
        520 => Assessment::Unsupported {
            reason: "the inner link replaces the outer image candidate",
            observed: "<p>![[<a href=\"uri1\">foo</a>](uri2)](uri3)</p>\n",
        },
        521 => Assessment::Unsupported {
            reason: "emphasis crosses the direct link label boundary",
            observed: "<p><a href=\"/uri\"><em>foo</em></a></p>\n",
        },
        523 => Assessment::DeliberateDifference {
            reason: "unresolved shortcut references keep hidden brackets even inside emphasis",
            observed: "<p><em>foo bar</em> baz</p>\n",
        },
        524 => Assessment::DeliberateDifference {
            reason: "raw HTML is outside the project dialect and stays literal",
            observed: "<p>[foo &lt;bar attr=&quot;](baz)&quot;&gt;</p>\n",
        },
        528 => Assessment::Unsupported {
            reason: "nested brackets replace the outer full reference candidate",
            observed: "<p>[link [foo bar]]<a href=\"/uri\">ref</a></p>\n",
        },
        531 => Assessment::Unsupported {
            reason: "an inner image replaces the outer full reference candidate",
            observed: "<p>[<img src=\"moon.jpg\" alt=\"moon\" />]<a href=\"/uri\">ref</a></p>\n",
        },
        534 => Assessment::Unsupported {
            reason: "emphasis crosses the full reference label opener",
            observed: "<p><a href=\"/uri\"><em>foo</em></a></p>\n",
        },
        535 => Assessment::Unsupported {
            reason: "emphasis crosses the full reference label closer",
            observed: "<p><a href=\"/uri\">foo <em>bar</em></a></p>\n",
        },
        536 => Assessment::DeliberateDifference {
            reason: "raw HTML is literal and does not suppress reference recognition",
            observed: "<p>[foo &lt;bar attr=&quot;]<a href=\"/uri\">ref</a>&quot;&gt;</p>\n",
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
fn pinned_commonmark_link_labels_report() {
    assert_eq!(LINK_LABEL_CASES.len(), 27);
    let mut counts = [0; 3];
    for (index, &(number, markdown, expected)) in LINK_LABEL_CASES.iter().enumerate() {
        assert_eq!(number, 512 + index);
        check_case(number, markdown, expected, &parse(markdown));
        let (category, label, reason) = match assessment(number) {
            Assessment::Supported => (0, "supported", "matches upstream"),
            Assessment::DeliberateDifference { reason, .. } => (1, "deliberate difference", reason),
            Assessment::Unsupported { reason, .. } => (2, "unsupported", reason),
        };
        counts[category] += 1;
        eprintln!("example {number}: {label} — {reason}");
    }
    assert_eq!(counts, [15, 4, 8]);
    eprintln!(
        "CommonMark 0.31.2 Links subset (512–538): 15 supported, 4 deliberate differences, 8 unsupported; 482–511 and 539–571 outside this score"
    );
}

#[test]
fn pinned_commonmark_link_labels_every_split_and_scalar_prefix() {
    let mut splits = 0;
    let mut prefixes = 0;
    for &(number, markdown, expected) in LINK_LABEL_CASES {
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
    assert_eq!((splits, prefixes), (814, 787));
    eprintln!(
        "27 examples: 814 single splits and 787 scalar prefixes preserve classified semantics and source metadata"
    );
}

#[test]
fn link_label_samples_preserve_mapping_when_sliced_and_read_with_six_byte_budgets() {
    let limit = WorkLimit {
        bytes: 6,
        queries: 1,
        objects: 1,
        ..Default::default()
    };
    for &(_, markdown, _) in LINK_LABEL_CASES {
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
fn link_label_raw_spans_identity_and_shared_display_destination_title_budget() {
    let extra = "[中&amp;\\*](<a b%20中\\\\> 't&amp;\\\"') []() [](/x '')\n";
    for source in LINK_LABEL_CASES
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
fn link_label_projection_preserves_nested_styles_objects_and_literal_failures() {
    for (source, expected) in [
        ("", ""),
        ("a  b\tc\n", "<p>a  b\tc</p>\n"),
        ("a\nb\n", "<p>a\nb</p>\n"),
        ("a  \nb\n", "<p>a<br />\nb</p>\n"),
        ("&#10;\n", "<p>\n</p>\n"),
        (
            "[]() ![](/x '')\n",
            "<p><a href=\"\"></a> <img src=\"/x\" alt=\"\" title=\"\" /></p>\n",
        ),
        ("[a\nb](/x)\n", "<p><a href=\"/x\">a\nb</a></p>\n"),
        ("[  a  b  ](/x)\n", "<p><a href=\"/x\">  a  b  </a></p>\n"),
        (
            "[中&amp;\\*](<a b%20中\\\\> 't&amp;\\\"')\n",
            "<p><a href=\"a%20b%20%E4%B8%AD%5C\" title=\"t&amp;&quot;\">中&amp;*</a></p>\n",
        ),
        ("[a](/x\n", "<p>a(/x</p>\n"),
        ("[a](/x 't\n", "<p>a(/x 't</p>\n"),
        ("*[a](/x)*\n", "<p><em><a href=\"/x\">a</a></em></p>\n"),
        (
            "*before [a](/x) after*\n",
            "<p><em>before <a href=\"/x\">a</a> after</em></p>\n",
        ),
        (
            "**before [a](/x) after**\n",
            "<p><strong>before <a href=\"/x\">a</a> after</strong></p>\n",
        ),
        (
            "[*a **b** `c`*](/x)\n",
            "<p><a href=\"/x\"><em>a <strong>b</strong> <code>c</code></em></a></p>\n",
        ),
        (
            "*a [**b**](/x) c*\n",
            "<p><em>a <a href=\"/x\"><strong>b</strong></a> c</em></p>\n",
        ),
        (
            "*a ![**b**](/x) c*\n",
            "<p><em>a <img src=\"/x\" alt=\"b\" /> c</em></p>\n",
        ),
        (
            "[foo [bar](/x)](/y)\n",
            "<p>[foo <a href=\"/x\">bar</a>](/y)</p>\n",
        ),
        (
            "[![中](x)](/y)\n",
            "<p>[<img src=\"x\" alt=\"中\" />](/y)</p>\n",
        ),
        (
            "<https://e.test/?x=[中]&y=%5D>\n",
            "<p><a href=\"https://e.test/?x=%5B%E4%B8%AD%5D&amp;y=%5D\">https://e.test/?x=[中]&amp;y=%5D</a></p>\n",
        ),
        (
            "[x][r]\n\n[r]: /u 't'\n",
            "<p><a href=\"/u\" title=\"t\">x</a></p>\n",
        ),
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
    let baseline = project(&parse("*before [a](/x 't') after*\n"));
    for source in [
        "before [a](/x 't') after\n",
        "**before [a](/x 't') after**\n",
        "*before [b](/x 't') after*\n",
        "*before [a](/y 't') after*\n",
        "*before [a](/x 'u') after*\n",
        "*before [a](/x) after*\n",
        "*before [](/x 't') after*\n",
        "*before ![a](/x 't') after*\n",
        "before [*a*](/x 't') after\n",
        "*before [a](/x 't' after*\n",
    ] {
        assert_ne!(project(&parse(source)), baseline, "{source:?}");
    }
    assert_eq!(url("/%20[中]&\"\\ "), "/%20%5B%E4%B8%AD%5D&amp;%22%5C%20");
}

#[test]
fn link_label_projection_rejects_unscored_structure_and_style_nesting() {
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
        "**a *b* c**\n",
        "<a@b.org>\n",
        "[a](<a`b>)\n",
        "[a](<a\\<b>)\n",
    ] {
        assert!(
            std::panic::catch_unwind(|| project(&parse(source))).is_err(),
            "accepted {source:?}"
        );
    }
    assert!(
        std::panic::catch_unwind(|| {
            Text::default().transition(vec!["em", "strong"]);
        })
        .is_err()
    );
}

#[test]
fn late_definitions_resolve_existing_label_objects_without_rebuilding_outer_candidates() {
    for &(number, source, expected) in LINK_LABEL_CASES.iter().filter(|(n, _, _)| *n >= 527) {
        let (body, definitions) = source.split_once("\n\n").unwrap();
        let mut doc = Document::new();
        doc.append(body).unwrap();
        doc.append("\n\n").unwrap();
        let targets = doc.links().to_vec();
        let infos: Vec<_> = (0..targets.len())
            .map(|i| *doc.link_info(LinkId(i)).unwrap())
            .collect();
        for (i, target) in targets.iter().enumerate() {
            if matches!(target, LinkTarget::Reference(_)) {
                assert_eq!(doc.link_destination(LinkId(i)), None);
            }
        }
        let definition_start = doc.source().len();
        for ch in definitions.chars() {
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            assert_eq!(doc.links(), targets);
            for (i, info) in infos.iter().enumerate() {
                assert_eq!(doc.link_info(LinkId(i)), Some(info));
            }
        }
        check_case(number, source, expected, &doc);
        let resolved = metadata_snapshot(&doc);
        for (i, target) in targets.iter().enumerate() {
            if matches!(target, LinkTarget::Reference(_)) && resolved[i].is_some() {
                assert!(
                    doc.decoded_link_destination(LinkId(i))
                        .unwrap()
                        .source_position()
                        >= definition_start
                );
            }
        }
        // Repeated definitions cannot change the old object, label or destination.
        doc.append("[ref]: /ignored 'ignored'\n[bar]: /ignored\n")
            .unwrap();
        doc.finish();
        assert_eq!(doc.links(), targets);
        // Example 528 has an unresolved inner [bar], which the new definition
        // legitimately resolves; it still cannot recreate the missing outer link.
        for (i, old) in resolved.iter().enumerate() {
            assert_eq!(doc.link_info(LinkId(i)), Some(&infos[i]));
            if old.is_some() {
                assert_eq!(&metadata_snapshot(&doc)[i], old);
            }
        }
        if number == 528 {
            assert_eq!(doc.links().len(), 2);
            assert_eq!(doc.link_destination(LinkId(0)), Some("/ignored"));
            assert_eq!(doc.text(infos[0].label), Some("bar"));
            assert_eq!(doc.text(infos[1].label), Some("ref"));
        }
    }
}
