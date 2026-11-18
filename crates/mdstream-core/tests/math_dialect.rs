use mdstream::*;

fn budget() -> Budget {
    Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 6,
        ..WorkLimit::default()
    })
}

fn parsed(source: &str) -> Document {
    let mut document = Document::new();
    document.append(source).unwrap();
    document.finish();
    document
}

fn current(document: &Document) -> Vec<MathObjectMeta> {
    let mut window = document
        .math_window(Span::new(0, document.source().len()))
        .unwrap();
    let mut objects = Vec::new();
    loop {
        match window.advance(document, &mut budget()).unwrap() {
            MathWindowStep::Object(meta) => objects.push(meta),
            MathWindowStep::Complete => return objects,
            MathWindowStep::BudgetExhausted => panic!("positive metadata budget"),
        }
    }
}

#[test]
fn paren_math_has_exact_raw_ranges_and_existing_opaque_priorities() {
    for (source, expected) in [
        (r"\(x\)", vec!["x"]),
        (r"\(\)", vec![""]),
        (r"\\(x\\)", vec![]),
        (r"\\\(x\)", vec!["x"]),
        (r"\(a$ b$$ \[ c\)", vec![r"a$ b$$ \[ c"]),
        (r"$a\(b\)c$", vec![r"a\(b\)c"]),
        (r"\(a\\\)z", vec![r"a\\"]),
        ("\\(unclosed\n\nend\\)", vec![]),
        (
            r"`\(code\)` [\(label\)](/\(target\) '\(title\)') <https://x/\(auto\)>",
            vec!["label"],
        ),
    ] {
        let document = parsed(source);
        let objects = current(&document);
        assert_eq!(
            objects
                .iter()
                .map(|m| document.text(m.content).unwrap())
                .collect::<Vec<_>>(),
            expected,
            "{source:?}"
        );
        for meta in objects {
            assert_eq!(meta.state, SyntaxState::Sealed);
            assert!(meta.closed);
            if meta.delimiter == MathDelimiter::Paren {
                assert_eq!(meta.kind, MathKind::Inline);
                assert_eq!(
                    document.text(Span::new(meta.source.start, meta.content.start)),
                    Some(r"\(")
                );
                assert_eq!(
                    document.text(Span::new(meta.content.end, meta.source.end)),
                    Some(r"\)")
                );
            }
        }
        let expected = current(&document);
        for cut in 0..=source.len() {
            if !source.is_char_boundary(cut) {
                continue;
            }
            let mut chunked = Document::new();
            chunked.append(&source[..cut]).unwrap();
            chunked.append("").unwrap();
            chunked.append(&source[cut..]).unwrap();
            chunked.finish();
            let actual = current(&chunked);
            assert_eq!(
                actual
                    .iter()
                    .map(|m| (
                        m.id,
                        m.source,
                        m.content,
                        m.kind,
                        m.delimiter,
                        m.closed,
                        m.state
                    ))
                    .collect::<Vec<_>>(),
                expected
                    .iter()
                    .map(|m| (
                        m.id,
                        m.source,
                        m.content,
                        m.kind,
                        m.delimiter,
                        m.closed,
                        m.state
                    ))
                    .collect::<Vec<_>>(),
                "cut{cut} {source:?}"
            );
        }
    }
}

#[test]
fn bracket_blocks_preserve_scope_columns_payload_and_family() {
    for source in [
        "\\[\nx\n\\]\n",
        "> \\[\r\n> x\r\n> \\]\r\n",
        ">\t\\[\n> x\n>\t\\]\n",
    ] {
        let document = parsed(source);
        let objects = current(&document);
        assert_eq!(objects.len(), 1, "{source:?}");
        let meta = objects[0];
        assert_eq!(meta.delimiter, MathDelimiter::Bracket);
        assert!(meta.closed);
        assert!(matches!(meta.id, MathObjectId::Block(_)));
        let mut cursor = document.math_payload_cursor(meta.id).unwrap();
        let mut payload = String::new();
        loop {
            match cursor.advance(&document, &mut budget()).unwrap() {
                MathPayloadStep::Chunk(chunk) => payload.push_str(chunk.text),
                MathPayloadStep::Complete => break,
                other => panic!("{other:?}"),
            }
        }
        assert!(payload == "x\n" || payload == "x\r\n");
    }
    let document = parsed("\\[\n$$\n\\[\n\\]\n");
    let meta = current(&document)[0];
    assert_eq!(document.text(meta.content), Some("$$\n\\[\n"));
    let document = parsed("> \\[\n> x\nexit\n");
    assert!(!current(&document)[0].closed);
    for marker in ["$$", r"\["] {
        for spaces in 0..=3 {
            let source = format!("- x\n\t{}{marker}\n", " ".repeat(spaces));
            let document = parsed(&source);
            assert_eq!(!current(&document).is_empty(), spaces <= 1, "{source:?}");
            assert_eq!(
                document
                    .blocks()
                    .iter()
                    .any(|b| matches!(b.kind, BlockKind::Code { .. })),
                spaces >= 2,
                "{source:?}"
            );
        }
    }
    for source in ["\\[x\\]\n", "\\[\r", "\\\\[\n", "    \\[\n"] {
        assert!(current(&parsed(source)).is_empty(), "{source:?}");
    }
}

#[test]
fn owned_payload_continues_balanced_and_partial_prefix_without_eof_replay() {
    let mut document = Document::new();
    let first = document.append("\\[\nx").unwrap();
    let meta = current(&document)[0];
    let mut cursor = document.math_payload_cursor(meta.id).unwrap();
    assert!(
        first
            .math_changes
            .iter()
            .any(|c| c.kind == MathChangeKind::Append)
    );
    let mut text = String::new();
    let mut bytes = 0;
    let mut previous = meta.version;
    for suffix in ["", "+y", "\\fra", "c{a}", "{b}", "\r", "\n", "\\]\n"] {
        let update = document.append(suffix).unwrap();
        let now = document.math_object(meta.id).unwrap();
        assert_eq!(now.epoch, meta.epoch);
        assert!(now.version >= previous);
        previous = now.version;
        assert!(update.math_changes.iter().all(|c| c.id == meta.id));
        loop {
            let mut frame = budget();
            match cursor.advance(&document, &mut frame).unwrap() {
                MathPayloadStep::Chunk(chunk) => {
                    text.push_str(chunk.text);
                    bytes += frame.used().bytes
                }
                MathPayloadStep::Pending | MathPayloadStep::Complete => break,
                MathPayloadStep::BudgetExhausted => panic!("positive input budget"),
            }
        }
    }
    assert_eq!(text, "x+y\\frac{a}{b}\r\n");
    assert_eq!(bytes, text.len());
    assert!(document.math_object(meta.id).unwrap().closed);
    assert_eq!(
        cursor.advance(&document, &mut budget()).unwrap(),
        MathPayloadStep::Complete
    );
    let mut stale = document
        .math_window(Span::new(0, document.source().len()))
        .unwrap();
    document.append("tail\n").unwrap();
    assert_eq!(
        stale.advance(&document, &mut budget()),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(
        cursor.advance(&document, &mut budget()).unwrap(),
        MathPayloadStep::Complete
    );
}

#[test]
fn long_marker_and_late_header_keep_current_namespace_and_effects() {
    let source = format!(
        "{}\\[{}\n{}x\n{}\\]{}\n",
        "> ".repeat(1100),
        " ".repeat(2000),
        "> ".repeat(1100),
        "> ".repeat(1100),
        " ".repeat(2000)
    );
    let mut document = Document::new();
    for ch in source.chars() {
        document.append(&ch.to_string()).unwrap();
    }
    document.finish();
    assert_eq!(current(&document).len(), 1);
    assert!(current(&document)[0].closed);
    let mut document = Document::new();
    document.append("\\(old\n").unwrap();
    let row = format!("close\\) {} *s* | \\(cell\\)\n", "a".repeat(2000));
    let mut update = document.append(&row).unwrap();
    let old = current(&document)
        .iter()
        .find(|m| m.source.start == 0)
        .copied()
        .unwrap();
    let mut cursor = document.math_payload_cursor(old.id).unwrap();
    update
        .merge(document.append("--- | ---\n").unwrap())
        .unwrap();
    assert!(document.math_object(old.id).is_none());
    assert_eq!(
        cursor.advance(&document, &mut budget()),
        Err(LayoutError::StaleContinuation)
    );
    assert!(
        update
            .math_changes
            .iter()
            .any(|c| c.kind == MathChangeKind::Withdraw && c.id == old.id)
    );
    assert!(
        current(&document)
            .iter()
            .any(|m| document.text(m.content) == Some("cell"))
    );
}

#[test]
fn body_owned_windows_and_payload_keep_their_namespace_and_host_eof() {
    let mut d = Document::new();
    d.append("[^a]: \\(x\\)\n\n    \\[\n    a\n").unwrap();
    let view = d.footnote_body(FootnoteId(0)).unwrap();
    let work = view.work();
    let mut window = view.math_window(view.range()).unwrap();
    assert!(matches!(
        window
            .advance(view, &mut Budget::new(WorkLimit::default()))
            .unwrap(),
        BodyMathWindowStep::BudgetExhausted
    ));
    assert_eq!(view.work(), work);
    let mut ids = Vec::new();
    loop {
        match window.advance(view, &mut budget()).unwrap() {
            BodyMathWindowStep::Object(id, object) => {
                assert_eq!(id.body(), view.key());
                assert_eq!(object.body, view.key());
                assert_eq!(id.local(), object.value.id);
                ids.push(id);
            }
            BodyMathWindowStep::Complete => break,
            BodyMathWindowStep::BudgetExhausted => panic!("positive budget"),
        }
    }
    assert_eq!(ids.len(), 2);
    let mut cursor = view.math_payload_cursor(ids[1]).unwrap().unwrap();
    let mut payload = String::new();
    loop {
        match cursor.advance(view, &mut budget()).unwrap() {
            BodyMathPayloadStep::Chunk(c) => {
                assert!(matches!(c.source, BodyAddress::Projected(s) if s.body == view.key()));
                payload.push_str(c.text);
            }
            BodyMathPayloadStep::Pending => break,
            _ => panic!("growing body must not receive EOF"),
        }
    }
    d.append("    +b\n    \\]\n\n[^b]: other\nend\n").unwrap();
    let view = d.footnote_body(FootnoteId(0)).unwrap();
    loop {
        match cursor.advance(view, &mut budget()).unwrap() {
            BodyMathPayloadStep::Chunk(c) => payload.push_str(c.text),
            BodyMathPayloadStep::Complete => break,
            _ => panic!("closed object"),
        }
    }
    assert_eq!(payload, "a\n+b\n");
    let other = d.footnote_body(FootnoteId(1)).unwrap();
    assert!(matches!(
        cursor.advance(other, &mut budget()),
        Err(LayoutError::InvalidRange)
    ));
    let foreign = parsed("[^a]: \\(x\\)\n");
    assert!(matches!(
        cursor.advance(foreign.footnote_body(FootnoteId(0)).unwrap(), &mut budget()),
        Err(LayoutError::WrongDocument)
    ));
    assert!(d.maths().is_empty());
    d.finish();
    let view = d.footnote_body(FootnoteId(0)).unwrap();
    assert!(matches!(
        cursor.advance(view, &mut budget()).unwrap(),
        BodyMathPayloadStep::Complete
    ));
}

#[test]
fn new_and_old_delimiters_have_linear_real_input_work_and_active_window_storage() {
    // These are native counters, independent of the accepted standalone model.
    for n in [1024, 16384, 131072, 1_000_000] {
        for (name, source) in [
            ("paren-late-close", format!("\\({}\\)", "x".repeat(n))),
            ("paren-mismatch", format!("\\({}", "\\]".repeat(n / 2))),
            ("slash-chain", format!("{}(x\\)", "\\".repeat(n))),
            (
                "mixed-runs",
                format!("\\({}\\)", "$a$$b$$$c$$$$".repeat(n / 13)),
            ),
            ("bracket-body", format!("\\[\n{}\n\\]\n", "x".repeat(n))),
        ] {
            let mut d = Document::new();
            let mut update = Update::default();
            for byte in source.as_bytes() {
                d.append_into(
                    std::str::from_utf8(std::slice::from_ref(byte)).unwrap(),
                    &mut update,
                )
                .unwrap();
            }
            d.finish();
            let w = d.work();
            let len = source.len() as u64;
            assert_eq!(w.input_bytes, len, "{name} {n}");
            assert!(
                w.inline_bytes <= len + w.header_replay_bytes,
                "{name} {n}: {w:?}"
            );
            assert!(w.classified_bytes <= 8 * len + 100, "{name} {n}: {w:?}");
            assert!(w.math_steps <= len + 10, "{name} {n}: {w:?}");
            assert!(w.math_object_steps <= 3 * len + 20, "{name} {n}: {w:?}");
            let objects = current(&d);
            let stats = d.math_object_stats();
            assert_eq!(stats.active, objects.len());
            assert!(stats.retained <= len as usize / 2 + 1);
            eprintln!(
                "native_math case={name} n={n} bytes={len} input={} classify={} inline={} math={} object={} active={} retained={} index_payload={}",
                w.input_bytes,
                w.classified_bytes,
                w.inline_bytes,
                w.math_steps,
                w.math_object_steps,
                stats.active,
                stats.retained,
                stats.index_payload_bytes
            );
        }
    }
}

#[test]
fn bracket_block_every_byte_cut_and_prefix_eof_follow_exact_scope() {
    for source in [
        "\\[\r\nx\r\n\\]\r\n",
        "> \\[\n> x\n> \\]\n",
        "- \\[\n\tx\n  \\]\n",
        "- > \\[\n\tpayload\n",
        "> \\[\n> x\nroot\n\\]\n",
        "\\[ \t\n\\] \t",
        "\\[\n\n\\]\n",
        "\\[\nbody\r",
    ] {
        let whole = parsed(source);
        let expected = current(&whole);
        let key = |m: &MathObjectMeta| (m.id, m.source, m.content, m.state, m.closed, m.delimiter);
        for split in 0..=source.len() {
            let mut d = Document::new();
            d.append(&source[..split]).unwrap();
            d.append(&source[split..]).unwrap();
            d.finish();
            assert_eq!(
                current(&d).iter().map(key).collect::<Vec<_>>(),
                expected.iter().map(key).collect::<Vec<_>>(),
                "split={split} {source:?}"
            );
        }
        let mut d = Document::new();
        for end in 0..=source.len() {
            if end > 0 {
                d.append(&source[end - 1..end]).unwrap();
            }
            let eof = parsed(&source[..end]);
            let live = current(&d);
            for m in live {
                assert_eq!(m.document, d.id());
                assert!(m.source.end <= end && m.content.end <= m.source.end);
            }
            if end == source.len() {
                d.finish();
                assert_eq!(
                    current(&d).iter().map(key).collect::<Vec<_>>(),
                    current(&eof).iter().map(key).collect::<Vec<_>>()
                );
            }
        }
    }
}

#[test]
fn owned_payload_accounts_metadata_even_when_the_scalar_has_no_byte_admission() {
    let d = parsed("\\(🙂\\)");
    let mut c = d.math_payload_cursor(current(&d)[0].id).unwrap();
    let before = c.source_position();
    for bytes in 0..4 {
        let mut b = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes,
            ..WorkLimit::default()
        });
        assert_eq!(
            c.advance(&d, &mut b).unwrap(),
            MathPayloadStep::BudgetExhausted
        );
        assert_eq!(b.used().queries, 1);
        assert_eq!(b.used().objects, 1);
        assert_eq!(b.used().bytes, 0);
        assert_eq!(c.source_position(), before);
    }
    assert!(matches!(
        c.advance(&d, &mut budget()).unwrap(),
        MathPayloadStep::Chunk(MathChunk { text: "🙂", .. })
    ));
}
