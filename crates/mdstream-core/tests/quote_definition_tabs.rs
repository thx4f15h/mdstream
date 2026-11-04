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
fn new_quote_definition_matches_column_model() {
    let mut cases = 0;
    for prefix in ["", " ", "  ", "   ", "> ", ">> ", "- ", "1. "] {
        for whitespace in ["", " ", "  ", "   ", "\t", " \t", "\t ", "\t\t"] {
            let separator = prefix.len() + 1;
            let mut column = separator + 4 - separator % 4;
            for b in whitespace.bytes() {
                column += if b == b'\t' { 4 - column % 4 } else { 1 };
            }
            let indent = column - separator - 1;
            for target in ["/raw&amp;", "<a b>", "/raw\\*"] {
                for ending in ["\n", "\r\n", ""] {
                    let source =
                        format!("{prefix}>\t{whitespace}[中 KEY]: {target} 't&amp;\\*'{ending}");
                    let doc = parse(&source);
                    let definitions: Vec<_> = doc
                        .blocks()
                        .iter()
                        .filter(|b| matches!(b.kind, BlockKind::Definition { .. }))
                        .collect();
                    assert_eq!(definitions.len(), usize::from(indent <= 3), "{source:?}");
                    if let Some(block) = definitions.first() {
                        assert!(matches!(
                            doc.container(block.container.unwrap()).unwrap().kind,
                            ContainerKind::Quote
                        ));
                        assert_eq!(block.state, SyntaxState::Sealed);
                        assert!(visible(&doc, block.source).is_empty());
                        let reference = doc
                            .references()
                            .iter()
                            .find(|r| r.destination.is_some())
                            .unwrap();
                        assert_eq!(reference.label, "中 key");
                        assert_eq!(
                            doc.text(reference.destination.unwrap()),
                            Some(target.trim_matches(['<', '>']))
                        );
                        assert_eq!(doc.text(reference.title.unwrap()), Some("t&amp;\\*"));
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
fn failed_probes_keep_separator_and_window_rules() {
    for body in [
        "[r]:",
        "[r] /no",
        "[]: /no",
        "[ ]: /no",
        "[\u{2003}]: /no",
        "[r]: <bad",
        "[r]: /a 'bad",
        "[^f] body",
        "ordinary",
    ] {
        let source = format!(">\t {body}\n");
        let doc = parse(&source);
        assert!(
            doc.references().iter().all(|r| r.destination.is_none()),
            "{source:?}"
        );
        assert!(
            doc.footnotes().iter().all(|r| r.definition.is_none()),
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
        ">\t  [r]: /no\n",
        "\t[r]: /no\n",
        "- > seed\n\t[r]: /no\n",
        "> ```\n>\t [r]: /no\n> ```\n",
        "> $$\n>\t [r]: /no\n> $$\n",
    ] {
        assert!(
            parse(source)
                .references()
                .iter()
                .all(|r| r.destination.is_none()),
            "{source:?}"
        );
    }
    for source in ["- seed\n\t>\t[r]: /yes\n", "> seed\n> >\t[r]: /yes\n"] {
        assert_eq!(
            parse(source)
                .references()
                .iter()
                .filter(|r| r.destination.is_some())
                .count(),
            1,
            "{source:?}"
        );
    }
    assert!(
        parse("> a | b\n>\t [r]: /no\n")
            .references()
            .iter()
            .all(|r| r.destination.is_none())
    );
    let mut pending = Document::new();
    pending.append(">\t [r]: /a").unwrap();
    assert!(pending.blocks().is_empty());
    pending.append(" 'bad\n").unwrap();
    assert!(pending.references().iter().all(|r| r.destination.is_none()));
    for size in [
        BLOCK_LOOKAHEAD - 1,
        BLOCK_LOOKAHEAD,
        BLOCK_LOOKAHEAD + 1,
        2048,
    ] {
        for ending in ["\n", "\r\n", ""] {
            let base = ">\t [r]: /a";
            let source = format!(
                "{base}{}{ending}",
                " ".repeat(size - base.len() - ending.len())
            );
            let accepted =
                size < BLOCK_LOOKAHEAD || (size == BLOCK_LOOKAHEAD && !ending.is_empty());
            assert_eq!(
                parse(&source)
                    .references()
                    .iter()
                    .any(|r| r.destination.is_some()),
                accepted,
                "size {size}, ending {ending:?}"
            );
        }
    }
    let source = format!(">\t [{}]: /no\n", "x".repeat(MAX_LABEL_BYTES + 1));
    assert!(
        parse(&source)
            .references()
            .iter()
            .all(|r| r.destination.is_none())
    );
}

#[test]
fn all_splits_prefixes_and_eof_agree() {
    for source in [
        ">\t [中]: <a b> '🙂'\r\n> [中]\n",
        "[中]\n>\t [中]: /a '🙂'\n>\t [中]: /ignored\n",
        "- seed\n\t>\t[r]: /yes\n",
        "> [中]\r\n>\t [中]: <a b> '🙂'\r\n> tail\n",
        "- [r]\n\t[r]: /url\n- next\n",
        "> seed\n>\t[r]: /url\n",
        "- - seed\n\t[r]: /url\n",
        ">\t [r]: /url",
        ">\t [ ]: /bad\n> after\n",
        ">\t [r]: /bad 'open\n> tail\n",
        "- > seed\n\t[r]: /no\n",
        "> a | b\n>\t [r]: /yes\n",
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
fn late_definition_updates_references_and_budgeted_mapping() {
    let source = "[中] 🙂 &amp;\r\n>\t [中]: /raw&amp; 't\\*'\r\n";
    let mut doc = Document::new();
    let boundary = source.find(">\t ").unwrap();
    doc.append(&source[..boundary]).unwrap();
    let links = doc.links().to_vec();
    let update = doc.append(&source[boundary..]).unwrap();
    assert_eq!(update.references_changed, [SymbolId(0)]);
    assert!(update.new_links.is_empty());
    assert!(doc.links() == links);
    assert_eq!(doc.link_destination(LinkId(0)), Some("/raw&amp;"));
    assert_eq!(doc.link_title(LinkId(0)), Some("t\\*"));
    assert!(
        update
            .changes
            .iter()
            .any(|c| c.kind == ChangeKind::Reference)
    );
    let update = doc.append(">\t [中]: /ignored\n").unwrap();
    assert!(update.references_changed.is_empty());
    assert_eq!(doc.link_destination(LinkId(0)), Some("/raw&amp;"));
    doc.finish();
    let span = Span::new(0, doc.source().len());
    let expected = "中 🙂 &\n";
    assert_eq!(visible(&doc, span), expected);
    assert_eq!(
        doc.runs_in(span)
            .unwrap()
            .map(|r| r.text)
            .collect::<String>(),
        doc.source()
    );
    let tiled: String = (0..doc.tile_count())
        .flat_map(|i| doc.display_runs(TileId(i)).unwrap())
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    let scalars: String = doc
        .source()
        .char_indices()
        .flat_map(|(at, ch)| {
            doc.display_runs_in(Span::new(at, at + ch.len_utf8()))
                .unwrap()
        })
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    assert_eq!(tiled, expected);
    assert_eq!(scalars, expected);
    let mut cursor = doc.display_cursor(span).unwrap();
    let mut output = String::new();
    for _ in 0..doc.source().len() * 2 {
        let at = cursor.source_position();
        if at < span.end {
            assert_eq!(
                cursor
                    .next(&doc, &mut Budget::new(WorkLimit::default()))
                    .unwrap(),
                DisplayStep::BudgetExhausted
            );
            assert_eq!(cursor.source_position(), at);
        }
        let mut budget = Budget::new(WorkLimit {
            bytes: 6,
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        match cursor.next(&doc, &mut budget).unwrap() {
            DisplayStep::Run(run) => {
                assert_eq!(run.source.start, at);
                assert_eq!(cursor.source_position(), run.source.end);
                if !run.style.contains(Style::HIDDEN) {
                    output.push_str(run.text);
                }
            }
            DisplayStep::Complete => break,
            DisplayStep::BudgetExhausted => panic!("one scalar fits"),
        }
        assert!(budget.used().bytes <= 6);
    }
    assert_eq!(output, expected);
    assert_eq!(cursor.source_position(), span.end);
}

#[test]
fn deep_long_input_and_old_references_have_bounded_work() {
    let prefix = ">".repeat(989);
    let mut doc = Document::new();
    doc.append(&format!("{prefix} {}", "x".repeat(1_000_000)))
        .unwrap();
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("x").unwrap();
    }
    assert_eq!(doc.work().inline_bytes - before.inline_bytes, 1000);
    assert_eq!(doc.work().classified_bytes, before.classified_bytes);
    assert_eq!(doc.work().container_steps, before.container_steps);
    assert_eq!(doc.work().container_bytes, before.container_bytes);
    doc.append("\n").unwrap();
    let before = doc.work();
    let update = doc.append(&format!("{prefix}>\t [r]: /yes\n")).unwrap();
    assert_eq!(update.references_changed, [SymbolId(0)]);
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert_eq!(doc.work().label_bytes - before.label_bytes, 1);
    assert!(doc.work().classified_bytes - before.classified_bytes < 6 * BLOCK_LOOKAHEAD as u64);
    assert!(doc.work().container_steps - before.container_steps < 6000);
    assert!(doc.work().tree_visits - before.tree_visits < 1000);

    let mut doc = Document::new();
    doc.append(&format!(">\t [r]: /{}", "x".repeat(1_000_000)))
        .unwrap();
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("x").unwrap();
    }
    assert_eq!(doc.work().inline_bytes - before.inline_bytes, 1000);
    assert_eq!(doc.work().classified_bytes, before.classified_bytes);
    assert_eq!(doc.work().container_steps, before.container_steps);
    assert_eq!(doc.work().container_bytes, before.container_bytes);
    assert_eq!(doc.work().label_bytes, before.label_bytes);
    doc.finish();
    assert!(doc.references().iter().all(|r| r.destination.is_none()));

    let mut doc = Document::new();
    doc.append(&format!("{}\n\n", "[r] ".repeat(20_000)))
        .unwrap();
    let links = doc.links().to_vec();
    let before = doc.work();
    let update = doc.append(">\t [r]: /late\n").unwrap();
    assert!(doc.links() == links);
    assert_eq!(update.references_changed, [SymbolId(0)]);
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert_eq!(doc.work().label_bytes - before.label_bytes, 1);
    assert!(doc.work().tree_visits - before.tree_visits < 2000);
    assert!(update.changes.len() < 12);
    assert_eq!(doc.link_destination(LinkId(19_999)), Some("/late"));
    for i in 0..4096 {
        doc.append(&format!("\n>\t [r{i}]: /url\n")).unwrap();
    }
    doc.finish();
    assert_eq!(doc.references().len(), 4097);
    assert_eq!(doc.container_count(), 4097);
    assert!(doc.work().classified_bytes < 8 * doc.source().len() as u64);
}
