use mdstream::*;

#[path = "support/snapshot.rs"]
mod support;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn visible(doc: &Document, span: Span) -> String {
    doc.display_runs_in(span)
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect()
}

#[test]
fn new_quote_footnote_matches_column_model() {
    let mut cases = 0;
    for prefix in ["", " ", "  ", "   ", "> ", ">> ", "- ", "1. "] {
        for whitespace in ["", " ", "  ", "   ", "\t", " \t", "\t ", "\t\t"] {
            let separator = prefix.len() + 1;
            let mut column = separator + 4 - separator % 4;
            for b in whitespace.bytes() {
                column += if b == b'\t' { 4 - column % 4 } else { 1 };
            }
            let indent = column - separator - 1;
            for body in ["", "中🙂 &amp; **raw**", "\t more"] {
                for ending in ["\n", "\r\n", ""] {
                    let source = format!("{prefix}>\t{whitespace}[^中_KEY]: {body}{ending}");
                    let doc = parse(&source);
                    let definitions: Vec<_> = doc
                        .blocks()
                        .iter()
                        .filter(|b| matches!(b.kind, BlockKind::FootnoteDefinition { .. }))
                        .collect();
                    assert_eq!(definitions.len(), usize::from(indent <= 3), "{source:?}");
                    if let Some(block) = definitions.first() {
                        assert!(matches!(
                            doc.container(block.container.unwrap()).unwrap().kind,
                            ContainerKind::Quote
                        ));
                        assert_eq!(block.state, SyntaxState::Sealed);
                        assert!(visible(&doc, block.source).is_empty());
                        let note = doc
                            .footnotes()
                            .iter()
                            .find(|n| n.definition.is_some())
                            .unwrap();
                        assert_eq!(note.label, "中_KEY");
                        let definition = note.definition.unwrap();
                        assert_eq!(doc.text(definition.content), Some(body));
                        assert_eq!(definition.block, doc.block_at(block.source.start).unwrap());
                        assert!(doc.footnote_references().is_empty());
                    }
                    assert_eq!(doc.source(), source);
                    cases += 1;
                }
            }
        }
    }
    assert_eq!(cases, 576);
}

#[test]
fn all_splits_prefixes_and_eof_agree() {
    for source in [
        ">\t [^中]: **🙂** &amp;\r\n>     next\r\n> tail\n",
        "[^中]\n>\t [^中]: body\n",
        "- seed\n\t>\t[^r]: body\n",
        "> seed\n> >\t[^r]: body\n",
        "> [^中]\r\n>\t [^中]: **🙂** &amp;\r\n>     next\r\n> tail\n",
        "- [^r]\n\t[^r]: body\n      next\n- next\n",
        "> seed\n>\t[^r]: body\n",
        "- - seed\n\t[^r]: body\n",
        ">\t [^r]: body",
        ">\t [^bad label]: body\n> after\n",
        "- > seed\n\t[^r]: body\n",
        "> a | b\n>\t [^r]: body\n",
    ] {
        let whole = support::snapshot(&parse(source));
        for split in (0..=source.len()).filter(|&at| source.is_char_boundary(at)) {
            let mut doc = Document::new();
            doc.append(&source[..split]).unwrap();
            doc.append(&source[split..]).unwrap();
            doc.finish();
            assert_eq!(support::snapshot(&doc), whole, "split {split}: {source:?}");
        }
        let mut doc = Document::new();
        for (at, ch) in source.char_indices() {
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            let text = &source[..at + ch.len_utf8()];
            let mut prefix = Document::new();
            prefix.append(text).unwrap();
            assert_eq!(support::snapshot(&doc), support::snapshot(&prefix));
            assert_eq!(doc.pending(), prefix.pending());
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
fn failures_window_and_four_column_continuations() {
    for body in [
        "[^]:",
        "[^bad label] body",
        "[^n] body",
        "ordinary",
        "[^n",
        "[^n]",
    ] {
        let source = format!(">\t {body}\n");
        let doc = parse(&source);
        assert!(
            doc.footnotes().iter().all(|n| n.definition.is_none()),
            "{source:?}"
        );
        assert!(
            doc.display_runs_in(Span::new(1, 2))
                .unwrap()
                .all(|r| !r.style.contains(Style::HIDDEN)),
            "{source:?}"
        );
    }
    for source in [
        ">\t  [^n]: body\n",
        "\t[^n]: body\n",
        "- > seed\n\t[^n]: body\n",
        "> ```\n>\t [^n]: body\n> ```\n",
        "> $$\n>\t [^n]: body\n> $$\n",
    ] {
        assert_eq!(
            parse(source)
                .footnotes()
                .iter()
                .filter(|n| n.definition.is_some())
                .count(),
            usize::from(source == "- > seed\n\t[^n]: body\n"),
            "{source:?}"
        );
    }
    for source in [
        "- seed\n\t>\t[^n]: body\n",
        "> seed\n> >\t[^n]: body\n",
        "> a | b\n> >\t[^n]: body\n",
    ] {
        assert_eq!(
            parse(source)
                .footnotes()
                .iter()
                .filter(|n| n.definition.is_some())
                .count(),
            1,
            "{source:?}"
        );
    }
    for continuation in [">\tsecond", ">     second", ">\t\tsecond"] {
        let source = format!(">\t [^n]:body\n{continuation}\n");
        let doc = parse(&source);
        let mut reader = doc.footnote_payload(FootnoteId(0)).unwrap();
        let mut payload = String::new();
        while let Some(chunk) = reader
            .advance(&mut Budget::new(WorkLimit::unlimited()))
            .unwrap()
        {
            payload.push_str(chunk.text);
        }
        assert_eq!(
            payload,
            match continuation {
                ">\tsecond" => "body",
                ">     second" => "body\nsecond",
                _ => "body\n  second",
            }
        );
    }
    // Only the label and colon need fit; body consumption is streaming.
    for depth in 1008..=1020 {
        let opener = format!("{}>\t[^n]:", ">".repeat(depth));
        for ending in ["\n", "\r\n", ""] {
            let source = format!("{opener} body{ending}");
            assert_eq!(
                parse(&source)
                    .footnotes()
                    .iter()
                    .filter(|n| n.definition.is_some())
                    .count(),
                1,
                "depth {depth}, {ending:?}"
            );
        }
    }
    for length in [MAX_FOOTNOTE_LABEL_BYTES, MAX_FOOTNOTE_LABEL_BYTES + 1] {
        let source = format!(">\t [^{}]: body\n", "x".repeat(length));
        assert_eq!(
            parse(&source)
                .footnotes()
                .iter()
                .filter(|n| n.definition.is_some())
                .count(),
            usize::from(length == MAX_FOOTNOTE_LABEL_BYTES)
        );
    }
}

#[test]
fn late_definition_preserves_identity_mapping_and_budget() {
    let mut doc = Document::new();
    doc.append("[^中] 🙂\r\n").unwrap();
    let references = doc.footnote_references().to_vec();
    let number = doc.footnote(FootnoteId(0)).unwrap().number;
    let update = doc.append(">\t [^中]: **raw** &amp;\r\n").unwrap();
    assert_eq!(update.footnotes_changed, [FootnoteId(0)]);
    assert!(update.new_footnote_references.is_empty());
    assert!(doc.footnote_references() == references);
    assert_eq!(doc.footnote(FootnoteId(0)).unwrap().number, number);
    assert_eq!(number, Some(1));
    let block = doc
        .footnote(FootnoteId(0))
        .unwrap()
        .definition
        .unwrap()
        .block;
    doc.append(">     中🙂\r\n> \r\n>     third\r\n> tail\n")
        .unwrap();
    let definition = doc.footnote(FootnoteId(0)).unwrap().definition.unwrap();
    assert_eq!(definition.block, block);
    assert_eq!(definition.state, SyntaxState::Sealed);
    let before = doc.footnote(FootnoteId(0)).unwrap().clone();
    let update = doc.append(">\t [^中]: ignored\n").unwrap();
    assert!(update.footnotes_changed.is_empty());
    assert_eq!(doc.footnote(FootnoteId(0)), Some(&before));
    doc.finish();
    assert_eq!(
        visible(&doc, Span::new(0, doc.source().len())),
        "[^中] 🙂\ntail\n"
    );
    let mut cursor = doc.footnote_payload(FootnoteId(0)).unwrap();
    let mut output = String::new();
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
        assert!(chunk.source.len() <= 6);
        assert_eq!(budget.used().bytes, chunk.source.len());
        assert_eq!(budget.used().queries, 1);
        assert_eq!(budget.used().objects, 1);
        if !chunk.text.is_empty() {
            assert_eq!(doc.text(chunk.source), Some(chunk.text));
        }
        output.push_str(chunk.text);
        charged += budget.used().bytes;
    }
    assert_eq!(output, "**raw** &amp;\r\n中🙂\r\n\r\nthird");
    assert_eq!(charged, definition.content.len());
    assert_eq!(
        doc.runs_in(Span::new(0, doc.source().len()))
            .unwrap()
            .map(|r| r.text)
            .collect::<String>(),
        doc.source()
    );
    let mut cursor = doc
        .display_cursor(Span::new(0, doc.source().len()))
        .unwrap();
    let mut display = String::new();
    loop {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 6,
            ..Default::default()
        });
        match cursor.next(&doc, &mut budget).unwrap() {
            DisplayStep::Run(run) => {
                if !run.style.contains(Style::HIDDEN) {
                    display.push_str(run.text);
                }
            }
            DisplayStep::Complete => break,
            DisplayStep::BudgetExhausted => panic!("one scalar fits"),
        }
        assert!(budget.used().bytes <= 6);
    }
    assert_eq!(display, "[^中] 🙂\ntail\n");
}

#[test]
fn deep_long_input_and_old_references_have_bounded_work() {
    let prefix = ">".repeat(988);
    let mut doc = Document::new();
    doc.append(&format!("{prefix} seed\n")).unwrap();
    let before = doc.work();
    doc.append(&format!("{prefix}>\t [^n]: {}", "x".repeat(1_000_000)))
        .unwrap();
    let definition = doc.footnote(FootnoteId(0)).unwrap().definition.unwrap();
    assert_eq!(doc.container_count(), 989);
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert!(doc.work().classified_bytes - before.classified_bytes < 6 * BLOCK_LOOKAHEAD as u64);
    assert!(doc.work().container_steps - before.container_steps < 6000);
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("x").unwrap();
    }
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert_eq!(doc.work().classified_bytes, before.classified_bytes);
    assert_eq!(doc.work().container_steps, before.container_steps);
    assert_eq!(doc.work().container_bytes, before.container_bytes);
    assert_eq!(doc.work().footnote_steps - before.footnote_steps, 1000);
    doc.finish();
    let grown = doc.footnote(FootnoteId(0)).unwrap().definition.unwrap();
    assert_eq!(grown.block, definition.block);
    assert_eq!(grown.content.len(), 1_001_000);

    let mut doc = Document::new();
    doc.append(&format!("{}\n\n", "[^n] ".repeat(20_000)))
        .unwrap();
    let references = doc.footnote_references().to_vec();
    let before = doc.work();
    let update = doc.append(">\t [^n]: late\n").unwrap();
    assert!(doc.footnote_references() == references);
    assert_eq!(update.footnotes_changed, [FootnoteId(0)]);
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert!(doc.work().footnote_steps - before.footnote_steps < 10);
    assert!(doc.work().tree_visits - before.tree_visits < 2000);
    assert!(update.changes.len() < 12);
    assert!(matches!(
        doc.resolve_footnote(
            FootnoteReferenceId(19_999),
            &mut Budget::new(WorkLimit::unlimited())
        )
        .unwrap()
        .unwrap()
        .target,
        FootnoteTarget::Defined(_)
    ));
    for i in 0..4096 {
        doc.append(&format!("\n>\t [^n{i}]: body\n")).unwrap();
    }
    doc.finish();
    assert_eq!(doc.footnotes().len(), 4097);
    assert_eq!(doc.container_count(), 4097);
    assert!(doc.work().classified_bytes < 8 * doc.source().len() as u64);
}
