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
    let mut consumed = 0;
    while cursor.source_position() < definition.content.end {
        let at = cursor.source_position();
        let mut empty = Budget::new(WorkLimit::default());
        assert_eq!(cursor.advance(&mut empty), Err(Completion::BudgetExhausted));
        assert_eq!(cursor.source_position(), at);
        assert_eq!(empty.used(), WorkLimit::default());
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
        assert!(budget.used().bytes <= 6);
        assert_eq!(budget.used().queries, 1);
        assert_eq!(budget.used().objects, 1);
        if !chunk.text.is_empty() && doc.text(chunk.source) != Some(chunk.text) {
            assert_eq!(doc.text(chunk.source), Some("\t"));
            assert!((1..=3).contains(&chunk.text.len()));
            assert!(chunk.text.bytes().all(|b| b == b' '));
        }
        consumed += chunk.source.len();
        text.push_str(chunk.text);
    }
    assert_eq!(consumed, definition.content.len());
    text
}

#[test]
fn independent_column_model_includes_partial_endpoints() {
    let mut cases = 0;
    for (opener, prefix, path_column, physical_column) in [
        ("", "", 0, 0),
        ("- ", "\t", 2, 4),
        ("1. ", "\t", 3, 4),
        ("- - ", " \t", 4, 4),
        ("- - - ", "\t\t", 6, 8),
        ("> ", ">\t", 2, 4),
        ("> - ", ">\t", 4, 4),
        ("> - - ", ">\t\t", 6, 8),
        ("- > ", "\t> ", 6, 6),
    ] {
        for len in 0..=5 {
            for bits in 0..1usize << len {
                let whitespace: String = (0..len)
                    .map(|i| if bits & (1 << i) == 0 { ' ' } else { '\t' })
                    .collect();
                let mut column = physical_column;
                let mut consumed = None;
                for (at, byte) in whitespace.bytes().enumerate() {
                    column += if byte == b'\t' { 4 - column % 4 } else { 1 };
                    if column >= path_column + 4 {
                        consumed = Some((at + 1, column - path_column - 4));
                        break;
                    }
                }
                // On insufficient split indentation, retain the old quote
                // spelling: the optional tab remains part of the four columns.
                if consumed.is_none() && opener == "> " && column == 5 {
                    consumed = Some((whitespace.len(), 0));
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
fn splits_prefixes_blank_lines_and_eof_agree() {
    for source in [
        "- [^n]: first\r\n\t\t中🙂\r\n\t\t\r\n\t\tlast\r\n- sibling\n",
        "> [^n]: first\n>\t\tsecond\n>\t\n>\t\tthird\n",
        "1. [^n]: first\n\t\t中🙂 **raw**\n\t\t\tlast",
        "- > [^n]: first\n\t> \t\tmore\n\t     missing quote\n",
        "> - [^n]: first\n>\t\tsecond\n> - [^n]: duplicate\n>\t\tignored\n",
    ] {
        let expected = parse(source);
        for split in (0..=source.len()).filter(|&at| source.is_char_boundary(at)) {
            let mut doc = Document::new();
            doc.append(&source[..split]).unwrap();
            doc.append(&source[split..]).unwrap();
            doc.finish();
            assert_eq!(support::snapshot(&doc), support::snapshot(&expected));
        }
        let mut doc = Document::new();
        for (at, ch) in source.char_indices() {
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            let text = &source[..at + ch.len_utf8()];
            let mut whole = Document::new();
            whole.append(text).unwrap();
            assert_eq!(support::snapshot(&doc), support::snapshot(&whole));
            assert_eq!(doc.pending(), whole.pending());
            let mut eof = Document::new();
            for ch in text.chars() {
                eof.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            }
            eof.finish();
            assert_eq!(support::snapshot(&eof), support::snapshot(&parse(text)));
        }
    }
}

#[test]
fn mapped_tab_is_atomic_under_budget_and_raw_reader_is_unchanged() {
    for (opener, continuation, spaces) in [
        ("- ", "\t\t", 2),
        ("1. ", "\t\t", 1),
        (">", "> \t\t", 2),
        (">", ">\t \t", 2),
        ("   > ", "   > \t\t", 3),
    ] {
        let source = format!("{opener}[^n]: first\n{continuation}🙂\n");
        let doc = parse(&source);
        assert_eq!(payload(&doc), format!("first\n{}🙂", " ".repeat(spaces)));
        let tab = source.rfind('\t').unwrap();
        let mut cursor = doc.footnote_payload(FootnoteId(0)).unwrap();
        while cursor.source_position() < tab {
            cursor
                .advance(&mut Budget::new(WorkLimit::unlimited()))
                .unwrap()
                .unwrap();
        }
        assert_eq!(cursor.source_position(), tab);
        for limit in [
            WorkLimit {
                queries: 1,
                objects: 1,
                bytes: spaces - 1,
                ..Default::default()
            },
            WorkLimit {
                queries: 0,
                objects: 1,
                bytes: 6,
                ..Default::default()
            },
            WorkLimit {
                queries: 1,
                objects: 0,
                bytes: 6,
                ..Default::default()
            },
        ] {
            let mut budget = Budget::new(limit);
            assert_eq!(
                cursor.advance(&mut budget),
                Err(Completion::BudgetExhausted)
            );
            assert_eq!(budget.used(), WorkLimit::default());
            assert_eq!(cursor.source_position(), tab);
        }
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: spaces,
            ..Default::default()
        });
        let chunk = cursor.advance(&mut budget).unwrap().unwrap();
        assert_eq!(chunk.source, Span::new(tab, tab + 1));
        assert_eq!(chunk.text, " ".repeat(spaces));
        assert_eq!(budget.used().bytes, spaces);
        let definition = doc.footnote(FootnoteId(0)).unwrap().definition.unwrap();
        let mut raw = doc.footnote_content(FootnoteId(0)).unwrap();
        let mut text = String::new();
        while let Some(chunk) = raw
            .advance(&mut Budget::new(WorkLimit::unlimited()))
            .unwrap()
        {
            assert_eq!(doc.text(chunk.source), Some(chunk.text));
            text.push_str(chunk.text);
        }
        assert_eq!(Some(text.as_str()), doc.text(definition.content));
    }
    for (source, expected) in [
        ("- [^n]: first\n\t\t\n\t\tlast\n", "first\n  \n  last"),
        ("- [^n]: first\n\t short\n", "first"),
        ("- > [^n]: first\n\t\tmissing quote\n", "first"),
        (
            "- [^n]: first\n\t\t**raw** &amp; [^x]\n- [^n]: ignored\n\t\tignored\n",
            "first\n  **raw** &amp; [^x]",
        ),
    ] {
        assert_eq!(payload(&parse(source)), expected);
    }
}

#[test]
fn deep_path_long_body_and_many_lines_keep_linear_work() {
    let opener = format!("{}1. ", "- ".repeat(490));
    let prefix = "\t".repeat(247);
    let mut doc = Document::new();
    doc.append(&format!(
        "{opener}[^n]: first\n{prefix}{}",
        "x".repeat(1_000_000)
    ))
    .unwrap();
    let definition = doc.footnote(FootnoteId(0)).unwrap().definition.unwrap();
    let scope = doc.block(definition.block).unwrap().container;
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("x").unwrap();
    }
    assert_eq!(doc.work().classified_bytes, before.classified_bytes);
    assert_eq!(doc.work().container_steps, before.container_steps);
    assert_eq!(doc.work().container_bytes, before.container_bytes);
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert_eq!(doc.work().footnote_steps - before.footnote_steps, 1000);
    doc.append("\n").unwrap();
    let before = doc.work();
    for _ in 0..512 {
        doc.append(&format!("{prefix}more\n")).unwrap();
    }
    assert!(doc.work().container_steps - before.container_steps < 4 * 491 * 512);
    assert!(doc.work().container_bytes - before.container_bytes < 16 * prefix.len() as u64 * 512);
    doc.finish();
    let after = doc.footnote(FootnoteId(0)).unwrap().definition.unwrap();
    assert_eq!(after.block, definition.block);
    assert_eq!(doc.block(after.block).unwrap().container, scope);
    assert_eq!(
        payload(&doc),
        format!("first\n {}{}", "x".repeat(1_001_000), "\n more".repeat(512))
    );
}

#[test]
fn late_definition_growth_keeps_old_references_and_bounded_notifications() {
    let mut doc = Document::new();
    doc.append(&format!("{}\n\n- [^n]: first\n", "[^n] ".repeat(20_000)))
        .unwrap();
    let references = doc.footnote_references().to_vec();
    let before = doc.work();
    for _ in 0..4096 {
        let update = doc.append("\t\tmore\n").unwrap();
        assert!(update.new_footnotes.is_empty());
        assert!(update.new_footnote_references.is_empty());
        assert_eq!(update.footnotes_changed, [FootnoteId(0)]);
        assert!(update.changes.len() < 12);
    }
    assert_eq!(doc.footnote_references(), references);
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert!(doc.work().footnote_steps - before.footnote_steps < 5 * 4096);
    assert!(doc.work().container_bytes - before.container_bytes < 40 * 4096);
    doc.finish();
    assert_eq!(payload(&doc), format!("first{}", "\n  more".repeat(4096)));
}
