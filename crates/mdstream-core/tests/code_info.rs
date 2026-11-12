use mdstream::*;

#[path = "support/snapshot.rs"]
mod support;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn infos(doc: &Document) -> Vec<String> {
    (0..doc.blocks().len())
        .filter_map(|index| {
            let mut cursor = doc.code_info(BlockId(index))?;
            let mut text = String::new();
            let mut budget = Budget::new(WorkLimit::unlimited());
            while let Some(chunk) = cursor.advance(&mut budget).unwrap() {
                text.push_str(chunk.text);
            }
            Some(text)
        })
        .collect()
}

#[test]
fn ascii_escape_model_keeps_raw_spans_and_borrowed_output() {
    // Independent CommonMark punctuation ranges, not the implementation's predicate.
    let mut cases = 0;
    for byte in 0u8..=127 {
        if matches!(byte, b'\n' | b'\r') {
            continue;
        }
        let ch = char::from(byte);
        let punctuation = matches!(byte, 0x21..=0x2f | 0x3a..=0x40 | 0x5b..=0x60 | 0x7b..=0x7e);
        for slashes in 0..=5 {
            for prefix in ["", "> ", "- ", "> - "] {
                let raw = format!("a{}{ch}z", "\\".repeat(slashes));
                let source = format!("{prefix}~~~ {raw}\n");
                let doc = parse(&source);
                let BlockKind::Code { info: Some(span) } = doc.blocks()[0].kind else {
                    panic!("missing info: {source:?}");
                };
                assert_eq!(doc.text(span), Some(raw.as_str()));
                let mut cursor = doc.code_info(BlockId(0)).unwrap();
                let mut budget = Budget::new(WorkLimit::unlimited());
                let mut end = span.start;
                let mut decoded = String::new();
                let mut chunks = 0;
                while let Some(chunk) = cursor.advance(&mut budget).unwrap() {
                    assert_eq!(chunk.source.start, end);
                    let original = doc.text(chunk.source).unwrap();
                    let expected = if original.len() == 2 && original.starts_with('\\') {
                        &original[1..]
                    } else {
                        original
                    };
                    assert_eq!(chunk.text, expected);
                    assert_eq!(chunk.text.as_ptr(), expected.as_ptr());
                    assert_eq!(chunk.text.chars().count(), 1);
                    end = chunk.source.end;
                    decoded.push_str(chunk.text);
                    chunks += 1;
                }
                let expected = format!(
                    "a{}{ch}z",
                    "\\".repeat(slashes / 2 + usize::from(slashes % 2 == 1 && !punctuation))
                );
                assert_eq!(decoded, expected, "{source:?}");
                assert_eq!(end, span.end);
                assert_eq!(cursor.source_position(), span.end);
                let probe = usize::from(byte == b'&' && slashes % 2 == 0);
                assert_eq!(budget.used().bytes, span.len() + 2 * probe);
                assert_eq!(budget.used().queries, chunks + probe);
                assert_eq!(budget.used().objects, chunks);
                cases += 1;
            }
        }
    }
    assert_eq!(cases, 3024);
}

#[test]
fn shared_budget_exhaustion_is_atomic_and_resumes_utf8_and_escapes() {
    let doc = parse("~~~ \\+中🙂\\q\\&amp;\\\n");
    let BlockKind::Code { info: Some(span) } = doc.blocks()[0].kind else {
        panic!("missing info");
    };
    let mut cursor = doc.code_info(BlockId(0)).unwrap();
    let mut output = String::new();
    let mut total = 0;
    for cost in [2, 3, 4, 1, 1, 2, 1, 1, 1, 1, 1] {
        let before = cursor.source_position();
        for limit in [
            WorkLimit {
                queries: 0,
                ..WorkLimit::unlimited()
            },
            WorkLimit {
                objects: 0,
                ..WorkLimit::unlimited()
            },
            WorkLimit {
                bytes: cost - 1,
                ..WorkLimit::unlimited()
            },
        ] {
            let mut budget = Budget::new(limit);
            assert_eq!(
                cursor.advance(&mut budget),
                Err(Completion::BudgetExhausted)
            );
            assert_eq!(cursor.source_position(), before);
            assert_eq!(budget.used(), WorkLimit::default());
        }
        let mut budget = Budget::new(WorkLimit {
            queries: 2,
            objects: 2,
            bytes: cost + 1,
            ..WorkLimit::default()
        });
        // Simulate a preceding consumer in the same frame.
        assert!(budget.charge(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 1,
            ..WorkLimit::default()
        }));
        let chunk = cursor.advance(&mut budget).unwrap().unwrap();
        assert_eq!(chunk.source.len(), cost);
        assert_eq!(budget.remaining().bytes, 0);
        assert_eq!(budget.remaining().queries, 0);
        assert_eq!(budget.remaining().objects, 0);
        output.push_str(chunk.text);
        total += cost;
    }
    assert_eq!(output, "+中🙂\\q&amp;\\");
    assert_eq!(total, span.len());
    let mut empty = Budget::new(WorkLimit::default());
    assert_eq!(cursor.advance(&mut empty), Ok(None));
    assert_eq!(cursor.advance(&mut empty), Ok(None));
    assert_eq!(empty.used(), WorkLimit::default());
}

#[test]
fn info_spanning_tiles_resumes_with_six_byte_frames() {
    let raw = format!("{}\\+🙂中{}", "a".repeat(251), "\\!é".repeat(80));
    let expected = format!("{}+🙂中{}", "a".repeat(251), "!é".repeat(80));
    let doc = parse(&format!("~~~ {raw}\n"));
    let BlockKind::Code { info: Some(span) } = doc.blocks()[0].kind else {
        panic!("missing info");
    };
    let mut cursor = doc.code_info(BlockId(0)).unwrap();
    let mut output = String::new();
    let mut end = span.start;
    let mut crossed = false;
    loop {
        let mut frame = Budget::new(WorkLimit {
            bytes: 6,
            queries: 6,
            objects: 6,
            ..WorkLimit::default()
        });
        let before = cursor.source_position();
        loop {
            match cursor.advance(&mut frame) {
                Ok(Some(chunk)) => {
                    assert_eq!(chunk.source.start, end);
                    if chunk.source == Span::new(255, 257) {
                        crossed = true;
                        assert_eq!(chunk.text, "+");
                        assert_eq!(doc.text(chunk.source), Some("\\+"));
                    }
                    output.push_str(chunk.text);
                    end = chunk.source.end;
                }
                Err(Completion::BudgetExhausted) => break,
                Ok(None) => {
                    assert!(crossed);
                    assert_eq!(end, span.end);
                    assert_eq!(output, expected);
                    return;
                }
                Err(other) => panic!("unexpected completion: {other:?}"),
            }
        }
        assert!(cursor.source_position() > before);
        assert!(frame.used().bytes <= 6);
    }
}

#[test]
fn every_utf8_split_and_scalar_prefix_preserves_info_and_core_semantics() {
    for source in [
        "``` foo\\+bar other\nfoo\n```\n",
        "> - ~~~ a\\*中🙂\\\\+ &amp; \\é\r\n>   body\r\n>   ~~~\r\n",
        "~~~ \\` \\! \\ \\t \\中 \\",
        "``` bad\\`info\nplain\n",
    ] {
        let expected = parse(source);
        for at in (0..=source.len()).filter(|&at| source.is_char_boundary(at)) {
            let mut doc = Document::new();
            doc.append(&source[..at]).unwrap();
            doc.append(&source[at..]).unwrap();
            doc.finish();
            assert_eq!(support::snapshot(&doc), support::snapshot(&expected));
            assert_eq!(infos(&doc), infos(&expected));
        }
        let mut doc = Document::new();
        for (at, ch) in source.char_indices() {
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            let prefix = &source[..at + ch.len_utf8()];
            let mut whole = Document::new();
            whole.append(prefix).unwrap();
            assert_eq!(support::snapshot(&doc), support::snapshot(&whole));
            assert_eq!(infos(&doc), infos(&whole));
            let mut finished = Document::new();
            for ch in prefix.chars() {
                finished.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            }
            finished.finish();
            assert_eq!(infos(&finished), infos(&parse(prefix)));
        }
    }
}

#[test]
fn absent_info_failed_candidates_entities_and_literal_body_keep_their_dialect() {
    for source in [
        "",
        "# title\n",
        "    code\n",
        "```\n",
        "~~~ \t\n",
        "$$\nx\n$$\n",
        "``` bad\\`info\n",
    ] {
        let doc = parse(source);
        assert!(doc.code_info(BlockId(0)).is_none(), "{source:?}");
        assert!(doc.code_info(BlockId(usize::MAX)).is_none());
    }
    let doc = parse("~~~ &amp; \\&amp; \\é \\中 \\🙂 \\*x\\*\n\\* &amp;\n~~~\n");
    assert_eq!(infos(&doc), vec!["& &amp; \\é \\中 \\🙂 *x*"]);
    let body: String = doc
        .display_runs_in(Span::new(0, doc.source().len()))
        .unwrap()
        .filter(|run| !run.style.contains(Style::HIDDEN))
        .map(|run| run.text)
        .collect();
    assert_eq!(body, "\\* &amp;\n");
    assert!(doc.links().is_empty());
    assert!(doc.maths().is_empty());
    assert!(doc.footnotes().is_empty());
}

#[test]
fn classification_boundary_and_large_history_do_not_expand_read_or_append_work() {
    // The complete raw info crosses the old window; decoding starts only
    // after LF/EOF confirms it.
    let prefix = format!("~~~ {}\\", "a".repeat(BLOCK_LOOKAHEAD - 5));
    assert_eq!(prefix.len(), BLOCK_LOOKAHEAD);
    let mut doc = Document::new();
    doc.append(&prefix).unwrap();
    let expected = format!("{}\\", "a".repeat(BLOCK_LOOKAHEAD - 5));
    assert!(infos(&doc).is_empty());
    doc.append("+").unwrap();
    doc.append(&"z".repeat(1_000_000)).unwrap();
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("z").unwrap();
    }
    let after = doc.work();
    assert_eq!(after.classified_bytes, before.classified_bytes);
    assert_eq!(after.inline_bytes, before.inline_bytes);
    assert_eq!(after.container_steps, before.container_steps);
    assert_eq!(
        after.fence_candidate_bytes - before.fence_candidate_bytes,
        1000
    );
    // The provisional raw opener creates no annotation nodes.
    assert!(after.tree_visits - before.tree_visits <= 64 * 1000);
    doc.finish();
    assert_eq!(
        infos(&doc),
        vec![format!(
            "{}+{}",
            expected.trim_end_matches('\\'),
            "z".repeat(1_001_000)
        )]
    );

    let mut doc = Document::new();
    for _ in 0..4096 {
        doc.append("~~~ x\\+y\nbody\n~~~\n").unwrap();
    }
    let deepest = ">".repeat(990);
    doc.append(&format!("{deepest} ~~~ x\\*y\n{deepest} "))
        .unwrap();
    doc.append(&"z".repeat(1_000_000)).unwrap();
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
    let mut cursor = doc.code_info(id).unwrap();
    let mut budget = Budget::new(WorkLimit {
        queries: 3,
        objects: 3,
        bytes: 4,
        ..WorkLimit::default()
    });
    let mut info = String::new();
    while let Some(chunk) = cursor.advance(&mut budget).unwrap() {
        info.push_str(chunk.text);
    }
    assert_eq!(info, "x*y");
    assert_eq!(budget.remaining().bytes, 0);
    assert_eq!(doc.work(), after);
    assert!(std::mem::size_of::<CodeInfo<'_>>() <= 12 * std::mem::size_of::<usize>());
}
