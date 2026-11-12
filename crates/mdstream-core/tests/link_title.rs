use mdstream::*;

#[path = "support/snapshot.rs"]
mod support;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

type TitleRead = (String, Vec<(Span, String)>, usize);

fn read(doc: &Document, id: LinkId, bytes: usize) -> TitleRead {
    let mut cursor = doc.decoded_link_title(id).unwrap();
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
                assert_eq!(end - start, doc.link_title(id).unwrap().len());
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
fn independent_model_direct_reference_images_and_containers() {
    let mut cases = 0;
    for &(raw, expected) in CASES {
        for prefix in ["", "> ", "- ", "> - "] {
            for image in ["", "!"] {
                for reference in [false, true] {
                    for ending in ["\n", "\r\n", ""] {
                        let source = if reference {
                            format!("{prefix}{image}[label][r]\n\n[r]: /url \"a{raw}z\"{ending}")
                        } else {
                            format!("{prefix}{image}[label](/url \"a{raw}z\"){ending}")
                        };
                        let doc = parse(&source);
                        assert_eq!(doc.links().len(), 1, "{source:?}");
                        let id = LinkId(0);
                        assert_eq!(
                            doc.link_title(id),
                            Some(format!("a{raw}z").as_str()),
                            "{source:?}"
                        );
                        let before = support::snapshot(&doc);
                        let (text, chunks, _) = read(&doc, id, 6);
                        assert_eq!(text, format!("a{expected}z"), "{source:?}");
                        assert_eq!(read(&doc, id, usize::MAX).0, text);
                        let start = source.find(&format!("\"a{raw}z\"")).unwrap() + 1;
                        assert_eq!(chunks.first().unwrap().0.start, start);
                        assert_eq!(chunks.last().unwrap().0.end, start + raw.len() + 2);
                        assert_eq!(doc.link_destination(id), Some("/url"));
                        assert_eq!(
                            doc.link_info(id).unwrap().kind,
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
fn ascii_escape_model_and_plain_chunks_borrow_source() {
    let mut cases = 0;
    for byte in 0u8..=127 {
        if matches!(byte, b'\r' | b'\n') {
            continue;
        }
        for slashes in 0..=5 {
            if byte == b'"' && slashes % 2 == 0 {
                continue;
            }
            let ch = char::from(byte);
            let punctuation = matches!(byte, 0x21..=0x2f | 0x3a..=0x40 | 0x5b..=0x60 | 0x7b..=0x7e);
            let raw = format!("a{}{ch}z", "\\".repeat(slashes));
            let doc = parse(&format!("[x](/u \"{raw}\")\n"));
            assert_eq!(doc.link_title(LinkId(0)), Some(raw.as_str()));
            let expected = format!(
                "a{}{ch}z",
                "\\".repeat(slashes / 2 + usize::from(slashes % 2 == 1 && !punctuation))
            );
            assert_eq!(read(&doc, LinkId(0), 6).0, expected);
            let mut cursor = doc.decoded_link_title(LinkId(0)).unwrap();
            let mut budget = Budget::new(WorkLimit::unlimited());
            while let Some(chunk) = cursor.advance(&mut budget).unwrap() {
                let raw = doc.text(chunk.source).unwrap();
                let borrowed = if raw.len() == 2 && raw.starts_with('\\') {
                    &raw[1..]
                } else {
                    raw
                };
                assert_eq!(chunk.text, borrowed);
                assert_eq!(chunk.text.as_ptr(), borrowed.as_ptr());
            }
            cases += 1;
        }
    }
    assert_eq!(cases, 753);
}

#[test]
fn late_definition_empty_absent_and_first_definition_wins() {
    let mut doc = Document::new();
    doc.append("[a][r] ![b][r] [r][] [r]\n\n").unwrap();
    let links = doc.links().to_vec();
    assert_eq!(links.len(), 4);
    let LinkTarget::Reference(symbol) = links[0] else {
        panic!("expected reference");
    };
    for i in 0..4 {
        assert!(doc.decoded_link_title(LinkId(i)).is_none());
    }
    let update = doc.append("> [r]: /first \"&amp;\\*\"\n").unwrap();
    assert_eq!(update.references_changed, [symbol]);
    assert_eq!(doc.links(), links);
    let expected = read(&doc, LinkId(0), 6);
    assert_eq!(expected.0, "&*");
    for i in 1..4 {
        assert_eq!(read(&doc, LinkId(i), 6), expected);
    }
    let update = doc.append("\n[r]: /second \"wrong\"\n").unwrap();
    assert!(update.references_changed.is_empty());
    assert_eq!(read(&doc, LinkId(0), 6), expected);

    let doc = parse(
        "[a](/u \"\") [b](/u) <https://example.org> <a@example.org> [missing]\n\n[n]: /u\n\n[n]\n",
    );
    let mut empty = doc.decoded_link_title(LinkId(0)).unwrap();
    let mut zero = Budget::new(WorkLimit::default());
    assert_eq!(empty.advance(&mut zero), Ok(None));
    assert_eq!(empty.advance(&mut zero), Ok(None));
    assert_eq!(zero.used(), WorkLimit::default());
    for i in 1..doc.links().len() {
        assert!(doc.decoded_link_title(LinkId(i)).is_none());
    }
    assert!(doc.decoded_link_title(LinkId(usize::MAX)).is_none());
    assert_eq!(doc.link_url(LinkId(3)).unwrap(), "mailto:a@example.org");

    let doc = parse("[r]\n\n[r]: /first\n[r]: /second \"ignored\"\n");
    assert!(doc.decoded_link_title(LinkId(0)).is_none());
    assert_eq!(doc.link_destination(LinkId(0)), Some("/first"));

    let mut doc = Document::new();
    doc.append("[a](/u \"&amp;").unwrap();
    assert!(titles(&doc).iter().all(Option::is_none));
    doc.append("\")").unwrap();
    // Short physical lines remain unclassified until LF/CRLF/EOF.
    assert!(titles(&doc).iter().all(Option::is_none));
    doc.append("\n").unwrap();
    assert_eq!(read(&doc, LinkId(0), 6).0, "&");
}

#[test]
fn shared_budget_preserves_probe_and_atomic_output() {
    let doc = parse("[x](/u \"&nGt;\\*🙂\")\n");
    let mut cursor = doc.decoded_link_title(LinkId(0)).unwrap();
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

fn titles(doc: &Document) -> Vec<Option<TitleRead>> {
    (0..doc.links().len())
        .map(|i| {
            doc.decoded_link_title(LinkId(i))
                .map(|_| read(doc, LinkId(i), 6))
        })
        .collect()
}

#[test]
fn every_utf8_split_and_prefix_including_pending_and_failed_titles() {
    for source in [
        "> - [中](/u \"&nGt;\\*🙂\")\r\n",
        "![a][r] [r][]\n\n> [r]: /u 'a&amp;\\+z'\n",
        "[a](/u (&#10;&quot;\\)))\n",
        "[a](/u \"unfinished &amp;",
        "[a](/u \"bad\nline\") [x]\n",
    ] {
        let expected = parse(source);
        for at in (0..=source.len()).filter(|&at| source.is_char_boundary(at)) {
            let mut doc = Document::new();
            doc.append(&source[..at]).unwrap();
            doc.append(&source[at..]).unwrap();
            doc.finish();
            assert_eq!(support::snapshot(&doc), support::snapshot(&expected));
            assert_eq!(titles(&doc), titles(&expected));
        }
        let mut doc = Document::new();
        for (at, ch) in source.char_indices() {
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            let prefix = &source[..at + ch.len_utf8()];
            let mut whole = Document::new();
            whole.append(prefix).unwrap();
            assert_eq!(support::snapshot(&doc), support::snapshot(&whole));
            assert_eq!(titles(&doc), titles(&whole));
            let finished = parse(prefix);
            let mut chunked = Document::new();
            for ch in prefix.chars() {
                chunked.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            }
            chunked.finish();
            assert_eq!(support::snapshot(&chunked), support::snapshot(&finished));
            assert_eq!(titles(&chunked), titles(&finished));
        }
    }
}

#[test]
fn title_boundaries_mapping_and_original_url_are_preserved() {
    for (open, close) in [('"', '"'), ('\'', '\''), ('(', ')')] {
        let raw = format!("{}&amp", "a".repeat(MAX_LINK_TITLE_BYTES - 4));
        let doc = parse(&format!("[a](/url&amp; {open}{raw}{close})\n"));
        assert_eq!(read(&doc, LinkId(0), 6).0, raw);
        assert_eq!(doc.link_url(LinkId(0)).unwrap(), "/url&amp;");
        let doc = parse(&format!("[a](/u {open}{raw}z{close})\n"));
        assert!(titles(&doc).iter().all(Option::is_none));
    }
    let source = format!(
        "[a](/u \"{}&CounterClockwiseContourIntegral;\\*\")\n",
        "a".repeat(241)
    );
    let doc = parse(&source);
    let (_, chunks, _) = read(&doc, LinkId(0), 6);
    let start = source.find('&').unwrap();
    assert_eq!(chunks[241], (Span::new(start, start + 33), "∳".into()));
    assert!(doc.tiles_overlapping(chunks[241].0).len() > 1);
    let doc = parse("[a](/u \"&#34;&#41;&#10;&#9;\")\n");
    assert_eq!(read(&doc, LinkId(0), 6).0, "\")\n\t");
    assert_eq!(doc.links().len(), 1);

    // Definition recognition retains its title data limit, not a line window.
    let doc = parse(&format!(
        "[r]\n\n[r]: /u \"{}&amp;\"\n",
        "a".repeat(BLOCK_LOOKAHEAD)
    ));
    assert!(doc.decoded_link_title(LinkId(0)).is_some());
    assert_eq!(doc.link_destination(LinkId(0)), Some("/u"));
}

#[test]
fn long_body_deep_containers_and_old_references_do_not_add_read_work() {
    let prefix = ">".repeat(980);
    let mut doc = Document::new();
    doc.append(&format!("{prefix} [a](/u \"&nGt;\\*\") "))
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
    let update = doc.append("> [r]: /u \"&nGt;\\*\"\n").unwrap();
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
    assert!(std::mem::size_of::<LinkTitle<'_>>() <= 12 * std::mem::size_of::<usize>());
}
