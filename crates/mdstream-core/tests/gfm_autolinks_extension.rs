//! Offline scoring of complete GFM 0.29-gfm Autolinks (extension) section (622–635).
//! The adapter reads public paragraph/display semantics, never Markdown syntax.
use mdstream::{BlockKind, Budget, DisplayStep, Document, Span, Style, WorkLimit};

#[path = "support/snapshot.rs"]
mod support;
use support::snapshot;

include!("fixtures/gfm_autolinks_extension.rs");

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
        for run in doc.display_runs_in(block.source).unwrap() {
            if run.style.contains(Style::HIDDEN) {
                continue;
            }
            for unsupported in [
                Style::CODE,
                Style::STRONG,
                Style::EMPHASIS,
                Style::MATH,
                Style::STRIKE,
            ] {
                assert!(!run.style.contains(unsupported), "unscored inline style");
            }
            assert!(run.link.is_none());
            // Omit only the borrowed physical paragraph terminator. Decoded
            // entities ending in LF remain content; never trim visible text.
            let payload = if run.source.end == block.source.end
                && run.replacement.is_none()
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
        html.push_str("</p>\n");
    }
    html
}

// Independent frozen observations of the documented no-bare-autolink dialect.
// Never passed to project(), which only reads the public semantic model.
const OBSERVED: &[(usize, &str)] = &[
    (622, "<p>www.commonmark.org</p>\n"),
    (
        623,
        "<p>Visit www.commonmark.org/help for more information.</p>\n",
    ),
    (
        624,
        "<p>Visit www.commonmark.org.</p>\n<p>Visit www.commonmark.org/a.b.</p>\n",
    ),
    (
        625,
        "<p>www.google.com/search?q=Markup+(business)</p>\n<p>www.google.com/search?q=Markup+(business)))</p>\n<p>(www.google.com/search?q=Markup+(business))</p>\n<p>(www.google.com/search?q=Markup+(business)</p>\n",
    ),
    (626, "<p>www.google.com/search?q=(business))+ok</p>\n"),
    (
        627,
        "<p>www.google.com/search?q=commonmark&amp;hl=en</p>\n<p>www.google.com/search?q=commonmark&amp;hl;</p>\n",
    ),
    (628, "<p>www.commonmark.org/he&lt;lp</p>\n"),
    (
        629,
        "<p>http://commonmark.org</p>\n<p>(Visit https://encrypted.google.com/search?q=Markup+(business))</p>\n",
    ),
    (630, "<p>foo@bar.baz</p>\n"),
    (
        631,
        "<p>hello@mail+xyz.example isn't valid, but hello+xyz@mail.example is.</p>\n",
    ),
    (
        632,
        "<p>a.b-c_d@a.b</p>\n<p>a.b-c_d@a.b.</p>\n<p>a.b-c_d@a.b-</p>\n<p>a.b-c_d@a.b_</p>\n",
    ),
    (
        633,
        "<p>mailto:foo@bar.baz</p>\n<p>mailto:a.b-c_d@a.b</p>\n<p>mailto:a.b-c_d@a.b.</p>\n<p>mailto:a.b-c_d@a.b/</p>\n<p>mailto:a.b-c_d@a.b-</p>\n<p>mailto:a.b-c_d@a.b_</p>\n<p>xmpp:foo@bar.baz</p>\n<p>xmpp:foo@bar.baz.</p>\n",
    ),
    (
        634,
        "<p>xmpp:foo@bar.baz/txt</p>\n<p>xmpp:foo@bar.baz/txt@bin</p>\n<p>xmpp:foo@bar.baz/txt@bin.com</p>\n",
    ),
    (635, "<p>xmpp:foo@bar.baz/txt/bin</p>\n"),
];

fn check_case(number: usize, markdown: &str, expected: &str, doc: &Document) {
    let actual = project(doc);
    assert_ne!(actual, expected, "review newly matching example {number}");
    assert_eq!(actual, OBSERVED.iter().find(|c| c.0 == number).unwrap().1);
    assert!(doc.links().is_empty());
    assert_eq!(doc.source(), markdown);
    let mut end = 0;
    for run in doc.display_runs_in(Span::new(0, markdown.len())).unwrap() {
        assert_eq!(run.source.start, end);
        assert!(run.source.start < run.source.end);
        assert_eq!(run.replacement, None);
        assert_eq!(run.text, &markdown[run.source.range()]);
        assert!(run.link.is_none());
        end = run.source.end;
    }
    assert_eq!(end, markdown.len());
}

#[test]
fn pinned_gfm_autolinks_extension_report() {
    assert_eq!(BARE_CASES.len(), 14);
    assert_eq!(OBSERVED.len(), 14);
    let mut counts = [0; 3];
    for (index, &(number, markdown, expected)) in BARE_CASES.iter().enumerate() {
        assert_eq!(number, 622 + index);
        assert_eq!(OBSERVED[index].0, number);
        check_case(number, markdown, expected, &parse(markdown));
        counts[1] += 1;
        eprintln!("example {number}: deliberate difference (bare addresses are not linked)");
    }
    assert_eq!(counts, [0, 14, 0]);
    eprintln!(
        "GFM Autolinks extension 622–635: 0 supported, 14 deliberate differences, 0 unsupported"
    );
}

#[test]
fn pinned_gfm_autolinks_extension_every_split_prefix_and_eof() {
    let mut splits = 0;
    let mut prefixes = 0;
    for &(number, markdown, expected) in BARE_CASES {
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
    assert_eq!((splits, prefixes), (945, 931));
}

#[test]
fn autolinks_extension_mapping_slices_and_six_byte_budget() {
    for &(_, markdown, _) in BARE_CASES {
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
fn bare_projection_preserves_visible_payload_and_rejects_unscored_semantics() {
    for (source, expected) in [
        ("", ""),
        ("www.example.com/中😀\n", "<p>www.example.com/中😀</p>\n"),
        ("a  b\tc\n", "<p>a  b\tc</p>\n"),
        ("a\nb\n", "<p>a\nb</p>\n"),
        ("a  \nb\n", "<p>a<br />\nb</p>\n"),
        ("a\n\nb\n", "<p>a</p>\n<p>b</p>\n"),
        ("www.x.test/?a&amp;b\n", "<p>www.x.test/?a&amp;b</p>\n"),
        ("&#10;\n", "<p>\n</p>\n"),
        ("www.x.test/a\\*b\n", "<p>www.x.test/a*b</p>\n"),
    ] {
        assert_eq!(project(&parse(source)), expected, "{source:?}");
    }
    let baseline = project(&parse("www.x.test/?a&b\n"));
    for source in [
        "www.y.test/?a&b\n",
        "www.x.test/?a&c\n",
        "www.x.test/?a  &b\n",
    ] {
        assert_ne!(project(&parse(source)), baseline);
    }
    for source in [
        "# h\n",
        "> q\n",
        "- item\n",
        "***\n",
        "    code\n",
        "```\nx\n```\n",
        "a|b\n--|--\n",
        "`code`\n",
        "*em*\n",
        "**strong**\n",
        "~~strike~~\n",
        "$math$\n",
        "[a](/x)\n",
        "![a](/x)\n",
        "<https://x.test>\n",
        "<a@b.test>\n",
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
fn angle_links_stay_distinct_from_bare_addresses_and_keep_source_mapping() {
    for (address, url) in [
        (
            "https://example.com/a?b=c&d=e",
            "https://example.com/a?b=c&d=e",
        ),
        ("foo@bar.baz", "mailto:foo@bar.baz"),
        ("mailto:foo@bar.baz", "mailto:foo@bar.baz"),
        ("xmpp:foo@bar.baz/txt", "xmpp:foo@bar.baz/txt"),
    ] {
        let bare = parse(address);
        assert!(bare.links().is_empty());
        assert_eq!(
            project(&bare),
            format!("<p>{}</p>\n", address.replace('&', "&amp;"))
        );
        let source = format!("<{address}>\n");
        let linked = parse(&source);
        assert_eq!(linked.links().len(), 1);
        let id = mdstream::LinkId(0);
        assert_eq!(linked.link_url(id).unwrap(), url);
        let info = linked.link_info(id).unwrap();
        assert_eq!(info.label, Span::new(1, 1 + address.len()));
        assert_eq!(linked.text(info.label), Some(address));
        let label: String = linked
            .display_runs_in(info.label)
            .unwrap()
            .map(|r| {
                assert_eq!(r.link, Some(id));
                assert_eq!(r.replacement, None);
                r.text
            })
            .collect();
        assert_eq!(label, address);
    }
}

#[test]
fn long_bare_address_append_keeps_block_identity_and_no_link_object() {
    let mut doc = Document::new();
    doc.append("www.example.com/").unwrap();
    doc.append(&"中😀x".repeat(300)).unwrap();
    let block = doc.block_at(0).unwrap().0;
    for ch in "/tail?q=(value)&key=1).\n".chars() {
        doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
        assert_eq!(doc.block_at(0).unwrap().0, block);
        assert!(doc.links().is_empty());
    }
    doc.finish();
    assert_eq!(doc.block_at(0).unwrap().0, block);
    assert_eq!(snapshot(&doc), snapshot(&parse(doc.source())));
    assert!(project(&doc).ends_with("/tail?q=(value)&amp;key=1).</p>\n"));
}
