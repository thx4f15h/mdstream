use mdstream::*;

#[path = "support/snapshot.rs"]
mod support;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn read(doc: &Document, id: BlockId, bytes: usize) -> (String, Vec<(Span, String)>, usize) {
    let mut cursor = doc.code_info(id).unwrap();
    let mut output = String::new();
    let mut chunks = Vec::new();
    let mut used = 0;
    for _ in 0..10000 {
        let mut budget = Budget::new(WorkLimit {
            bytes,
            queries: 1,
            objects: 1,
            ..WorkLimit::default()
        });
        match cursor.advance(&mut budget) {
            Ok(Some(chunk)) => {
                output.push_str(chunk.text);
                chunks.push((chunk.source, chunk.text.to_owned()));
            }
            Ok(None) => return (output, chunks, used),
            Err(Completion::BudgetExhausted) => {
                assert_ne!(budget.used(), WorkLimit::default(), "no progress");
            }
            Err(other) => panic!("unexpected {other:?}"),
        }
        assert!(budget.used().bytes <= bytes);
        used += budget.used().bytes;
    }
    panic!("cursor did not finish");
}

const CASES: &[(&str, &str)] = &[
    ("&amp;", "&"),
    ("&ouml;", "ö"),
    ("&ngE;", "≧\u{338}"),
    ("&nGt;", "≫\u{20d2}"),
    ("&fjlig;", "fj"),
    ("&CounterClockwiseContourIntegral;", "∳"),
    ("&#0;", "�"),
    ("&#xD800;", "�"),
    ("&#1114112;", "�"),
    ("&#x10ffff;", "\u{10ffff}"),
    ("&#X1F642;", "🙂"),
    ("&#128;", "\u{80}"),
    ("&#0000042;", "*"),
    ("&#00000042;", "&#00000042;"),
    ("&#x00002a;", "*"),
    ("&#x000002a;", "&#x000002a;"),
    ("&#10;", "\n"),
    ("&#9;", "\t"),
    ("&#32;", " "),
    ("&AMP;", "&"),
    ("&Amp;", "&Amp;"),
    ("&madeup;", "&madeup;"),
    ("&amp", "&amp"),
    ("&#;", "&#;"),
    ("&#x;", "&#x;"),
    ("&#-1;", "&#-1;"),
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
];

#[test]
fn independent_entity_model_preserves_mapping_and_resumes_six_byte_frames() {
    let mut cases = 0;
    for &(raw, expected) in CASES {
        for prefix in ["", "> ", "- ", "> - "] {
            for ending in ["\n", "\r\n", ""] {
                let source = format!("{prefix}~~~ a{raw}z{ending}");
                let doc = parse(&source);
                let BlockKind::Code { info: Some(span) } = doc.blocks()[0].kind else {
                    panic!("missing info: {source:?}");
                };
                assert_eq!(doc.text(span), Some(format!("a{raw}z").as_str()));
                let (output, chunks, used) = read(&doc, BlockId(0), 6);
                assert_eq!(output, format!("a{expected}z"), "{source:?}");
                assert_eq!(read(&doc, BlockId(0), usize::MAX).0, output);
                let mut end = span.start;
                for (mapped, text) in chunks {
                    assert_eq!(mapped.start, end);
                    assert!(source.is_char_boundary(mapped.start));
                    assert!(source.is_char_boundary(mapped.end));
                    assert!(!text.is_empty());
                    end = mapped.end;
                }
                assert_eq!(end, span.end);
                assert!(used <= 4 * span.len(), "linear charged work");
                if raw.starts_with('&')
                    && raw.find(';') == Some(raw.len() - 1)
                    && !expected.contains('&')
                    && expected != raw
                {
                    let (_, chunks, _) = read(&doc, BlockId(0), 6);
                    assert_eq!(
                        chunks[1],
                        (Span::new(span.start + 1, span.end - 1), expected.into())
                    );
                }
                cases += 1;
            }
        }
    }
    assert_eq!(cases, CASES.len() * 12);
}

#[test]
fn shared_budget_pauses_probe_and_atomic_entity_output() {
    let doc = parse("~~~ &nGt;X\n");
    let mut cursor = doc.code_info(BlockId(0)).unwrap();
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
    // A query starts the probe, five bytes scan it, the emitted six UTF-8
    // bytes need another query/object and are never partially returned.
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
    assert_eq!(shared.remaining().queries, 0);
    let mut budget = Budget::new(WorkLimit::unlimited());
    assert_eq!(cursor.advance(&mut budget).unwrap().unwrap().text, "X");
    let mut empty = Budget::new(WorkLimit::default());
    assert_eq!(cursor.advance(&mut empty), Ok(None));
}

#[test]
fn every_utf8_split_and_prefix_keeps_core_snapshot_and_decoded_info() {
    for source in [
        "~~~ f&ouml;&ouml;\\&amp;&#x1f642;&ngE;\nbody &amp;\n~~~\n",
        "> - ~~~ &CounterClockwiseContourIntegral; &bad; 中\r\n>   x\r\n>   ~~~\r\n",
        "~~~ &#xD800; &amp",
    ] {
        let expected = parse(source);
        for at in (0..=source.len()).filter(|&at| source.is_char_boundary(at)) {
            let mut doc = Document::new();
            doc.append(&source[..at]).unwrap();
            doc.append(&source[at..]).unwrap();
            doc.finish();
            assert_eq!(support::snapshot(&doc), support::snapshot(&expected));
            assert_eq!(
                read(&doc, BlockId(0), 6).0,
                read(&expected, BlockId(0), 6).0
            );
        }
        let mut doc = Document::new();
        for (at, ch) in source.char_indices() {
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            let mut whole = Document::new();
            whole.append(&source[..at + ch.len_utf8()]).unwrap();
            assert_eq!(support::snapshot(&doc), support::snapshot(&whole));
            if doc.code_info(BlockId(0)).is_some() {
                assert_eq!(read(&doc, BlockId(0), 6).0, read(&whole, BlockId(0), 6).0);
            }
            let finished = parse(&source[..at + ch.len_utf8()]);
            let mut chunked = Document::new();
            for ch in source[..at + ch.len_utf8()].chars() {
                chunked.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            }
            chunked.finish();
            assert_eq!(support::snapshot(&chunked), support::snapshot(&finished));
            if finished.code_info(BlockId(0)).is_some() {
                assert_eq!(
                    read(&chunked, BlockId(0), 6).0,
                    read(&finished, BlockId(0), 6).0
                );
            }
        }
    }
}

#[test]
fn committed_info_boundary_and_literal_body_are_unchanged() {
    let source = format!(
        "~~~ {}&CounterClockwiseContourIntegral;z\n",
        "a".repeat(249)
    );
    let doc = parse(&source);
    let (_, chunks, _) = read(&doc, BlockId(0), 6);
    assert_eq!(chunks[249], (Span::new(253, 286), "∳".into()));
    for suffix in ["&amp", "&#x1f", "&CounterClockwiseContourIntegral"] {
        let prefix = format!(
            "~~~ {}{suffix}",
            "a".repeat(BLOCK_LOOKAHEAD - 4 - suffix.len())
        );
        let mut doc = Document::new();
        doc.append(&prefix).unwrap();
        let before = read(&doc, BlockId(0), 6).0;
        doc.append(";\n&amp; &#42; \\*\n~~~\n").unwrap();
        doc.finish();
        assert_eq!(read(&doc, BlockId(0), 6).0, before);
        assert!(before.ends_with(suffix));
        let body: String = doc
            .display_runs_in(Span::new(0, doc.source().len()))
            .unwrap()
            .filter(|r| !r.style.contains(Style::HIDDEN))
            .map(|r| r.text)
            .collect();
        assert_eq!(body, "&amp; &#42; \\*\n");
    }
}

#[test]
fn deep_large_body_and_history_do_not_add_info_or_append_work() {
    let mut doc = Document::new();
    for _ in 0..4096 {
        doc.append("~~~ &amp;\nbody\n~~~\n").unwrap();
    }
    let prefix = ">".repeat(980);
    doc.append(&format!("{prefix} ~~~ &ngE;&#x1f642;\n{prefix} "))
        .unwrap();
    doc.append(&"x".repeat(1_000_000)).unwrap();
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("z").unwrap();
    }
    let after = doc.work();
    assert_eq!(after.classified_bytes, before.classified_bytes);
    assert_eq!(after.inline_bytes, before.inline_bytes);
    assert_eq!(after.container_steps, before.container_steps);
    assert_eq!(after.tree_visits, before.tree_visits);
    let id = BlockId(doc.blocks().len() - 1);
    let (text, _, used) = read(&doc, id, 6);
    assert_eq!(text, "≧\u{338}🙂");
    assert_eq!(used, "&ngE;&#x1f642;".len() + text.len());
    assert_eq!(doc.work(), after);
}

#[test]
fn complete_named_table_and_numeric_scalar_model() {
    // The pinned table is the exhaustive data oracle, not the decoder; the
    // independent cases above also cover lookup failures and non-recursion.
    mod names {
        include!("../src/entity_names.rs");
    }
    for &(name, expected) in names::NAMES {
        assert!(expected.len() <= 6);
        let doc = parse(&format!("~~~ &{name};\n"));
        let (output, chunks, used) = read(&doc, BlockId(0), 6);
        assert_eq!(output, expected, "{name}");
        assert_eq!(chunks.len(), 1);
        assert_eq!(used, name.len() + 2 + expected.len());
    }
    for value in (0..=0x11ffffu32).step_by(997).chain([
        0, 9, 10, 13, 128, 0xd7ff, 0xd800, 0xdfff, 0xe000, 0x10ffff, 0x110000, 9999999,
    ]) {
        let expected = char::from_u32(value)
            .filter(|ch| *ch != '\0')
            .unwrap_or('�')
            .to_string();
        for body in [
            format!("#{value}"),
            format!("#x{value:x}"),
            format!("#X{value:X}"),
        ] {
            let doc = parse(&format!("~~~ &{body};\n"));
            assert_eq!(read(&doc, BlockId(0), 6).0, expected, "{body}");
        }
    }
}
