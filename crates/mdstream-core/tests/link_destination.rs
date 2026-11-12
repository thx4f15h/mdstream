use mdstream::*;

#[path = "support/snapshot.rs"]
mod support;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

type DestinationRead = (String, Vec<(Span, String)>, usize);

fn read(doc: &Document, id: LinkId, bytes: usize) -> DestinationRead {
    let mut cursor = doc.decoded_link_destination(id).unwrap();
    let start = cursor.source_position();
    let mut output = String::new();
    let mut chunks = Vec::new();
    let mut used = 0;
    let mut end = start;
    for _ in 0..20_000 {
        let mut budget = Budget::new(WorkLimit {
            bytes,
            queries: 1,
            objects: 1,
            ..WorkLimit::default()
        });
        match cursor.advance(&mut budget) {
            Ok(Some(chunk)) => {
                assert_eq!(chunk.source.start, end);
                assert!(doc.text(chunk.source).is_some());
                end = chunk.source.end;
                output.push_str(chunk.text);
                chunks.push((chunk.source, chunk.text.to_owned()));
            }
            Ok(None) => {
                assert_eq!(end - start, doc.link_destination(id).unwrap().len());
                assert!(used <= 4 * (end - start));
                return (output, chunks, used);
            }
            Err(Completion::BudgetExhausted) => {
                assert_ne!(budget.used(), WorkLimit::default(), "no progress");
            }
            Err(other) => panic!("unexpected {other:?}"),
        }
        assert_eq!(cursor.source_position(), end);
        assert!(budget.used().bytes <= bytes);
        used += budget.used().bytes;
    }
    panic!("cursor did not finish");
}

const CASES: &[(&str, &str)] = &[
    ("&amp;", "&"),
    ("&ouml;", "ö"),
    ("&nGt;", "≫\u{20d2}"),
    ("&fjlig;", "fj"),
    ("&CounterClockwiseContourIntegral;", "∳"),
    ("&#0;", "�"),
    ("&#xD800;", "�"),
    ("&#1114112;", "�"),
    ("&#X1F642;", "🙂"),
    ("&#128;", "\u{80}"),
    ("&#0000042;", "*"),
    ("&#00000042;", "&#00000042;"),
    ("&#x00002a;", "*"),
    ("&#x000002a;", "&#x000002a;"),
    ("&#10;&#9;&#32;", "\n\t "),
    ("&AMP;&Amp;", "&&Amp;"),
    ("&madeup;", "&madeup;"),
    ("&amp", "&amp"),
    ("&#;&#x;&#-1;", "&#;&#x;&#-1;"),
    ("&a中;", "&a中;"),
    ("\\&amp;", "&amp;"),
    ("\\\\&amp;", "\\&"),
    ("&amp;amp;", "&amp;"),
    ("&#92;*", "\\*"),
    ("&amp;#42;", "&#42;"),
    ("&&amp;", "&&"),
    (
        "&aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa&amp;",
        "&aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa&",
    ),
    ("\\\"\\'\\(\\)\\*\\_", "\"'()*_"),
    ("\\q\\中🙂", "\\q\\中🙂"),
    ("  \t*hi*  ", "  \t*hi*  "),
];

#[test]
fn shared_budget_preserves_probe_and_atomic_output() {
    let doc = parse("[x](<&nGt;\\*🙂>)\n");
    let mut cursor = doc.decoded_link_destination(LinkId(0)).unwrap();
    let start = cursor.source_position();
    for limit in [
        WorkLimit::default(),
        WorkLimit {
            queries: 0,
            ..WorkLimit::unlimited()
        },
        WorkLimit {
            objects: 0,
            ..WorkLimit::unlimited()
        },
    ] {
        let mut budget = Budget::new(limit);
        assert_eq!(
            cursor.advance(&mut budget),
            Err(Completion::BudgetExhausted)
        );
        assert_eq!(budget.used(), WorkLimit::default());
        assert_eq!(cursor.source_position(), start);
    }
    let mut scan = Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 5,
        ..WorkLimit::default()
    });
    assert_eq!(cursor.advance(&mut scan), Err(Completion::BudgetExhausted));
    assert_eq!(scan.used().bytes, 5);
    assert_eq!(scan.used().queries, 1);
    assert_eq!(scan.used().objects, 0);
    assert_eq!(cursor.source_position(), start);
    let mut small = Budget::new(WorkLimit {
        bytes: 5,
        ..WorkLimit::unlimited()
    });
    assert_eq!(cursor.advance(&mut small), Err(Completion::BudgetExhausted));
    assert_eq!(small.used(), WorkLimit::default());
    let mut shared = Budget::new(WorkLimit {
        queries: 2,
        objects: 2,
        bytes: 7,
        ..WorkLimit::default()
    });
    assert!(shared.charge(WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 1,
        ..WorkLimit::default()
    }));
    let chunk = cursor.advance(&mut shared).unwrap().unwrap();
    assert_eq!(chunk.text, "≫\u{20d2}");
    assert_eq!(chunk.source, Span::new(start, start + 5));
    assert_eq!(shared.remaining().bytes, 0);
    for (cost, expected) in [(2, "*"), (4, "🙂")] {
        let before = cursor.source_position();
        let mut small = Budget::new(WorkLimit {
            bytes: cost - 1,
            ..WorkLimit::unlimited()
        });
        assert_eq!(cursor.advance(&mut small), Err(Completion::BudgetExhausted));
        assert_eq!(small.used(), WorkLimit::default());
        assert_eq!(cursor.source_position(), before);
        let mut budget = Budget::new(WorkLimit {
            bytes: cost,
            queries: 1,
            objects: 1,
            ..WorkLimit::default()
        });
        assert_eq!(cursor.advance(&mut budget).unwrap().unwrap().text, expected);
        assert_eq!(budget.used().bytes, cost);
    }
    assert_eq!(
        cursor.advance(&mut Budget::new(WorkLimit::default())),
        Ok(None)
    );
}

#[test]
fn explicit_destinations_model_with_images_references_and_containers() {
    let mut cases = 0;
    for &(raw, expected) in CASES {
        for prefix in ["", "> ", "- ", "> - "] {
            for image in ["", "!"] {
                for reference in [false, true] {
                    for ending in ["\n", "\r\n", ""] {
                        let source = if reference {
                            format!("{prefix}{image}[label][r]\n\n[r]: <a{raw}z> \"title\"{ending}")
                        } else {
                            format!("{prefix}{image}[label](<a{raw}z> \"title\"){ending}")
                        };
                        let doc = parse(&source);
                        assert_eq!(doc.links().len(), 1, "{source:?}");
                        let raw = format!("a{raw}z");
                        assert_eq!(doc.link_destination(LinkId(0)), Some(raw.as_str()));
                        let before = support::snapshot(&doc);
                        let (text, chunks, _) = read(&doc, LinkId(0), 6);
                        assert_eq!(text, format!("a{expected}z"), "{source:?}");
                        assert_eq!(read(&doc, LinkId(0), usize::MAX).0, text);
                        let start = source.find(&format!("<{raw}>")).unwrap() + 1;
                        assert_eq!(chunks.first().unwrap().0.start, start);
                        assert_eq!(chunks.last().unwrap().0.end, start + raw.len());
                        assert_eq!(doc.link_url(LinkId(0)).unwrap(), raw);
                        assert_eq!(doc.link_title(LinkId(0)), Some("title"));
                        assert_eq!(
                            doc.link_info(LinkId(0)).unwrap().kind,
                            if image.is_empty() {
                                LinkKind::Link
                            } else {
                                LinkKind::Image
                            }
                        );
                        assert_eq!(support::snapshot(&doc), before);
                        cases += 1;
                    }
                }
            }
        }
    }
    assert_eq!(cases, 1440);
}

#[test]
fn ascii_escape_model_and_literal_autolinks() {
    let mut cases = 0;
    for byte in 0u8..=127 {
        if matches!(byte, b'\r' | b'\n') {
            continue;
        }
        for slashes in 0..=5 {
            if matches!(byte, b'<' | b'>') && slashes % 2 == 0 {
                continue;
            }
            let ch = char::from(byte);
            let punctuation = matches!(byte, 0x21..=0x2f | 0x3a..=0x40 | 0x5b..=0x60 | 0x7b..=0x7e);
            let raw = format!("a{}{ch}z", "\\".repeat(slashes));
            let doc = parse(&format!("[x](<{raw}>)\n"));
            assert_eq!(doc.link_destination(LinkId(0)), Some(raw.as_str()));
            let expected = format!(
                "a{}{ch}z",
                "\\".repeat(slashes / 2 + usize::from(slashes % 2 == 1 && !punctuation))
            );
            assert_eq!(read(&doc, LinkId(0), 6).0, expected);
            cases += 1;
        }
    }
    assert_eq!(cases, 750);
    for raw in ["https://e.org/&amp;\\*🙂", "ab:&#42;\\+", "a&b@example.org"] {
        let doc = parse(&format!("<{raw}>\n"));
        assert_eq!(doc.links().len(), 1);
        let (text, chunks, used) = read(&doc, LinkId(0), 6);
        assert_eq!(text, raw);
        assert_eq!(used, raw.len());
        assert_eq!(chunks.len(), raw.chars().count());
        let mut cursor = doc.decoded_link_destination(LinkId(0)).unwrap();
        let mut budget = Budget::new(WorkLimit::unlimited());
        while let Some(chunk) = cursor.advance(&mut budget).unwrap() {
            assert_eq!(
                chunk.text.as_ptr(),
                doc.text(chunk.source).unwrap().as_ptr()
            );
        }
        let explicit = parse(&format!("[x](<{raw}>)\n"));
        if raw.contains('&') && raw.contains(';') {
            assert_ne!(read(&explicit, LinkId(0), 6).0, raw);
        }
    }
    let doc = parse("<a&b@example.org>\n");
    assert_eq!(read(&doc, LinkId(0), 6).0, "a&b@example.org");
    assert_eq!(doc.link_url(LinkId(0)).unwrap(), "mailto:a&b@example.org");
}

#[test]
fn late_winning_definition_empty_missing_and_stable_identity() {
    let mut doc = Document::new();
    doc.append("[r] ![r][] [a][r] ![b][r]\n\n").unwrap();
    let links = doc.links().to_vec();
    let LinkTarget::Reference(symbol) = links[0] else {
        panic!("reference");
    };
    assert_eq!(links.len(), 4);
    for i in 0..4 {
        assert!(doc.decoded_link_destination(LinkId(i)).is_none());
    }
    let update = doc.append("> [r]: <&amp;\\*> \"title\"\n").unwrap();
    assert_eq!(update.references_changed, [symbol]);
    assert_eq!(doc.links(), links);
    let expected = read(&doc, LinkId(0), 6);
    assert_eq!(expected.0, "&*");
    for i in 1..4 {
        assert_eq!(read(&doc, LinkId(i), 6), expected);
    }
    let update = doc.append("\n[r]: /wrong\n").unwrap();
    assert!(update.references_changed.is_empty());
    assert_eq!(read(&doc, LinkId(0), 6), expected);
    let doc = parse("[a]() ![b](<>) [r]\n\n[r]: <>\n");
    assert_eq!(doc.links().len(), 3);
    for i in 0..3 {
        let mut cursor = doc.decoded_link_destination(LinkId(i)).unwrap();
        let mut budget = Budget::new(WorkLimit::default());
        assert_eq!(cursor.advance(&mut budget), Ok(None));
        assert_eq!(cursor.advance(&mut budget), Ok(None));
        assert_eq!(budget.used(), WorkLimit::default());
    }
    assert!(doc.decoded_link_destination(LinkId(usize::MAX)).is_none());
    let doc = parse("[r]\n\n[r]: <>\n[r]: /wrong\n");
    assert_eq!(read(&doc, LinkId(0), 6).0, "");
}

fn destinations(doc: &Document) -> Vec<Option<DestinationRead>> {
    (0..doc.links().len())
        .map(|i| {
            doc.decoded_link_destination(LinkId(i))
                .map(|_| read(doc, LinkId(i), 6))
        })
        .collect()
}

#[test]
fn every_utf8_split_and_prefix_pending_failed_and_autolinks() {
    for source in [
        "> - [中](<&nGt;\\*🙂> \"&amp;\")\r\n",
        "![a][r] [r][]\n\n> [r]: /a&amp;\\+z\n",
        "[a](/a(b)\\(c\\)) [b](<&#10;&quot;&#41;>)\n",
        "[a](<unfinished&amp;",
        "[a](bad\nline) [x]\n",
        "<https://e.org/&amp;\\*🙂> <a&b@example.org>\n",
    ] {
        let expected = parse(source);
        for at in (0..=source.len()).filter(|&at| source.is_char_boundary(at)) {
            let mut doc = Document::new();
            doc.append(&source[..at]).unwrap();
            doc.append(&source[at..]).unwrap();
            doc.finish();
            assert_eq!(support::snapshot(&doc), support::snapshot(&expected));
            assert_eq!(destinations(&doc), destinations(&expected));
        }
        let mut doc = Document::new();
        for (at, ch) in source.char_indices() {
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            let prefix = &source[..at + ch.len_utf8()];
            let mut whole = Document::new();
            whole.append(prefix).unwrap();
            assert_eq!(support::snapshot(&doc), support::snapshot(&whole));
            assert_eq!(destinations(&doc), destinations(&whole));
            let finished = parse(prefix);
            let mut chunked = Document::new();
            for ch in prefix.chars() {
                chunked.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            }
            chunked.finish();
            assert_eq!(support::snapshot(&chunked), support::snapshot(&finished));
            assert_eq!(destinations(&chunked), destinations(&finished));
        }
    }
}

#[test]
fn bounds_bare_targets_mapping_and_url_encoding_remain_explicit() {
    for angle in [false, true] {
        let raw = format!("{}&amp", "a".repeat(MAX_DESTINATION_BYTES - 4));
        let wrap = |s: &str| {
            if angle {
                format!("<{s}>")
            } else {
                s.to_owned()
            }
        };
        let doc = parse(&format!("[x]({})\n", wrap(&raw)));
        assert_eq!(read(&doc, LinkId(0), 6).0, raw);
        let doc = parse(&format!("[x]({})\n", wrap(&format!("{raw}z"))));
        assert!(destinations(&doc).iter().all(Option::is_none));
    }
    for (raw, expected) in [
        ("/a(b)\\(c\\)", "/a(b)(c)"),
        ("/a\\+?x=1&amp;y=2", "/a+?x=1&y=2"),
        ("&#32;&#10;&#9;&#41;&#60;", " \n\t)<"),
        ("/%20/%2F/中", "/%20/%2F/中"),
        ("javascript&#58;alert(1)", "javascript:alert(1)"),
    ] {
        let doc = parse(&format!("[x]({raw})\n"));
        assert_eq!(read(&doc, LinkId(0), 6).0, expected);
        assert_eq!(doc.link_url(LinkId(0)).unwrap(), raw);
    }
    let source = format!(
        "[x](<{}&CounterClockwiseContourIntegral;>)\n",
        "a".repeat(241)
    );
    let doc = parse(&source);
    let (_, chunks, _) = read(&doc, LinkId(0), 6);
    let start = source.find('&').unwrap();
    assert_eq!(chunks[241], (Span::new(start, start + 33), "∳".into()));
    assert!(doc.tiles_overlapping(chunks[241].0).len() > 1);
    let doc = parse(&format!(
        "[r]\n\n[r]: /{}&amp;\n",
        "a".repeat(BLOCK_LOOKAHEAD)
    ));
    assert!(doc.decoded_link_destination(LinkId(0)).is_some());
}

#[test]
fn deep_long_body_and_old_references_do_not_add_read_work() {
    let mut doc = Document::new();
    doc.append(&format!("{} [a](<&nGt;\\*>) ", ">".repeat(980)))
        .unwrap();
    doc.append(&"x".repeat(1_000_000)).unwrap();
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("z").unwrap();
    }
    let after = doc.work();
    assert_eq!(after.inline_bytes - before.inline_bytes, 1000);
    assert_eq!(after.classified_bytes, before.classified_bytes);
    assert_eq!(after.container_steps, before.container_steps);
    let output = read(&doc, LinkId(0), 6);
    assert_eq!(output.0, "≫\u{20d2}*");
    assert_eq!(output.2, 13);
    assert_eq!(doc.work(), after);
    let mut doc = Document::new();
    doc.append(&format!("{}\n\n", "[r] ".repeat(20_000)))
        .unwrap();
    let links = doc.links().to_vec();
    let before = doc.work();
    let update = doc.append("> [r]: <&nGt;\\*>\n").unwrap();
    assert_eq!(update.references_changed, [SymbolId(0)]);
    assert!(update.changes.len() < 12);
    assert_eq!(doc.links(), links);
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert_eq!(doc.work().label_bytes - before.label_bytes, 1);
    assert!(doc.work().tree_visits - before.tree_visits < 2000);
    let after = doc.work();
    for id in [LinkId(0), LinkId(10_000), LinkId(19_999)] {
        let result = read(&doc, id, 6);
        assert_eq!(result.0, output.0);
        assert_eq!(result.2, output.2);
    }
    assert_eq!(doc.work(), after);
    assert!(std::mem::size_of::<LinkDestination<'_>>() <= 12 * std::mem::size_of::<usize>());
}
