use mdstream::*;

#[path = "support/snapshot.rs"]
mod support;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn payload(doc: &Document) -> String {
    let definition = doc.footnote(FootnoteId(0)).unwrap().definition.unwrap();
    let mut cursor = doc.footnote_payload(FootnoteId(0)).unwrap();
    let mut text = String::new();
    let mut charged = 0;
    while cursor.source_position() < definition.content.end {
        let at = cursor.source_position();
        assert_eq!(
            cursor.advance(&mut Budget::new(WorkLimit::default())),
            Err(Completion::BudgetExhausted)
        );
        assert_eq!(cursor.source_position(), at);
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 6,
            ..Default::default()
        });
        let chunk = cursor.advance(&mut budget).unwrap().unwrap();
        assert_eq!(chunk.source.start, at);
        assert_eq!(
            budget.used().bytes,
            chunk.source.len().max(chunk.text.len())
        );
        assert!(chunk.source.len() <= 6);
        assert_eq!(budget.used().queries, 1);
        assert_eq!(budget.used().objects, 1);
        if !chunk.text.is_empty() && doc.text(chunk.source) != Some(chunk.text) {
            assert_eq!(doc.text(chunk.source), Some("\t"));
            assert!((1..=3).contains(&chunk.text.len()));
            assert!(chunk.text.bytes().all(|b| b == b' '));
        }
        text.push_str(chunk.text);
        charged += chunk.source.len();
    }
    assert_eq!(charged, definition.content.len());
    text
}

#[test]
fn complete_path_column_model() {
    let mut cases = 0;
    for (opener, prefix) in [
        ("", ""),
        ("> ", "> "),
        (">> ", ">> "),
        (">>> ", ">>> "),
        ("- ", "  "),
        ("1. ", "   "),
        ("> - ", ">   "),
        ("- - ", "    "),
        ("10. ", "\t"),
    ] {
        for len in 0..=5 {
            for bits in 0..1usize << len {
                let whitespace: String = (0..len)
                    .map(|i| if bits & (1 << i) == 0 { ' ' } else { '\t' })
                    .collect();
                let mut column = 0;
                for byte in prefix.bytes() {
                    column += if byte == b'\t' { 4 - column % 4 } else { 1 };
                }
                let target = column + 4;
                let mut consumed = None;
                for (at, byte) in whitespace.bytes().enumerate() {
                    column += if byte == b'\t' { 4 - column % 4 } else { 1 };
                    if column >= target {
                        consumed = Some((at + 1, column - target));
                        break;
                    }
                }
                for ending in ["\n", "\r\n", ""] {
                    let source = format!(
                        "{opener}[^n]: first\n{prefix}{whitespace}中🙂 **raw** &amp;{ending}"
                    );
                    let doc = parse(&source);
                    let expected = consumed.map_or_else(
                        || "first".to_owned(),
                        |(at, spaces)| {
                            format!(
                                "first\n{}{}中🙂 **raw** &amp;",
                                " ".repeat(spaces),
                                &whitespace[at..]
                            )
                        },
                    );
                    assert_eq!(payload(&doc), expected, "{source:?}");
                    let definition = doc.footnote(FootnoteId(0)).unwrap().definition.unwrap();
                    assert_eq!(definition.block, BlockId(0));
                    assert_eq!(definition.state, SyntaxState::Sealed);
                    assert_eq!(doc.source(), source);
                    cases += 1;
                }
            }
        }
    }
    assert_eq!(cases, 1701);
}

#[test]
fn all_splits_prefixes_and_eof_agree() {
    for source in [
        "[^n]: first\r\n\t中🙂\r\n\r\n \tthird\r\ntail\n",
        "> [^n]: first\n> \t  second\n> \t\tovershoot\n",
        "- [^n]: first\n  \t  second\n\tpartial path\n",
        ">>> [^n]: first\n>>> \t\tmore\n>>> tail\n",
        "1. [^n]: first\n   \t   second",
        "[^n]: first\n\t\n\tthird\n[^n]: duplicate\n\tignored\n",
    ] {
        let expected = parse(source);
        for split in (0..=source.len()).filter(|&at| source.is_char_boundary(at)) {
            let mut doc = Document::new();
            doc.append(&source[..split]).unwrap();
            doc.append(&source[split..]).unwrap();
            doc.finish();
            assert_eq!(support::snapshot(&doc), support::snapshot(&expected));
            assert_eq!(payload(&doc), payload(&expected));
        }
        let mut doc = Document::new();
        for (at, ch) in source.char_indices() {
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            let text = &source[..at + ch.len_utf8()];
            let mut whole = Document::new();
            whole.append(text).unwrap();
            assert_eq!(support::snapshot(&doc), support::snapshot(&whole));
            assert_eq!(doc.footnotes(), whole.footnotes());
            assert_eq!(doc.pending(), whole.pending());
            let mut eof = Document::new();
            for ch in text.chars() {
                eof.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            }
            eof.finish();
            assert_eq!(support::snapshot(&eof), support::snapshot(&parse(text)));
            if eof
                .footnote(FootnoteId(0))
                .and_then(|n| n.definition)
                .is_some()
            {
                assert_eq!(payload(&eof), payload(&parse(text)));
            }
        }
    }
}

#[test]
fn failures_blanks_raw_mapping_and_first_definition_win() {
    for (source, expected) in [
        ("[^n]: first\n \t\tsecond\n", "first\n\tsecond"),
        ("> [^n]: first\n> \t  second\n", "first\nsecond"),
        ("> [^n]: first\n> \t\tsecond\n", "first\n  second"),
        ("> [^n]: first\n\tsecond\n", "first"),
        ("- [^n]: first\n\t  second\n", "first\nsecond"),
        ("[^n]: first\n\t\n\tsecond\n", "first\n\nsecond"),
        (
            "[^n]: first\n\t**raw** &amp;\n[^n]: ignored\n\tlater\n",
            "first\n**raw** &amp;",
        ),
        ("[^n]: first\n   second\n", "first"),
        ("[^n]: first\n    second\n", "first\nsecond"),
    ] {
        let doc = parse(source);
        assert_eq!(payload(&doc), expected, "{source:?}");
        let definition = doc.footnote(FootnoteId(0)).unwrap().definition.unwrap();
        let mut raw = doc.footnote_content(FootnoteId(0)).unwrap();
        let mut text = String::new();
        while let Some(chunk) = raw
            .advance(&mut Budget::new(WorkLimit::unlimited()))
            .unwrap()
        {
            text.push_str(chunk.text);
        }
        assert_eq!(Some(text.as_str()), doc.text(definition.content));
    }
}

#[test]
fn deep_long_continuation_does_not_rescan_body_or_ancestors() {
    let prefix = format!("{} ", ">".repeat(991));
    let mut doc = Document::new();
    doc.append(&format!(
        "{prefix}[^n]: first\n{prefix}\t{}",
        "x".repeat(1_000_000)
    ))
    .unwrap();
    let block = doc
        .footnote(FootnoteId(0))
        .unwrap()
        .definition
        .unwrap()
        .block;
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("x").unwrap();
    }
    assert_eq!(doc.work().classified_bytes, before.classified_bytes);
    assert_eq!(doc.work().container_steps, before.container_steps);
    assert_eq!(doc.work().container_bytes, before.container_bytes);
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert_eq!(doc.work().footnote_steps - before.footnote_steps, 1000);
    doc.finish();
    assert_eq!(
        doc.footnote(FootnoteId(0))
            .unwrap()
            .definition
            .unwrap()
            .block,
        block
    );
    assert_eq!(payload(&doc), format!("first\n{}", "x".repeat(1_001_000)));
    assert!(doc.work().container_bytes < 12 * doc.source().len() as u64);
}

#[test]
fn continuation_growth_keeps_old_references_and_bounded_notifications() {
    let mut doc = Document::new();
    doc.append(&format!("{}\n\n[^n]: first\n", "[^n] ".repeat(20_000)))
        .unwrap();
    let references = doc.footnote_references().to_vec();
    let before = doc.work();
    for _ in 0..4096 {
        let update = doc.append("\tmore\n").unwrap();
        assert!(update.new_footnotes.is_empty());
        assert!(update.new_footnote_references.is_empty());
        assert_eq!(update.footnotes_changed, [FootnoteId(0)]);
        assert!(update.changes.len() < 12);
    }
    assert_eq!(doc.footnote_references(), references);
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert!(doc.work().footnote_steps - before.footnote_steps < 5 * 4096);
    assert!(doc.work().container_bytes - before.container_bytes < 20 * 4096);
    doc.finish();
    assert_eq!(payload(&doc), format!("first{}", "\nmore".repeat(4096)));
}
