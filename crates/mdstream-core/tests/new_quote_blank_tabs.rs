use mdstream::*;

#[path = "support/snapshot.rs"]
mod support;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn visible(doc: &Document) -> String {
    doc.display_runs_in(Span::new(0, doc.source().len()))
        .unwrap()
        .filter(|run| !run.style.contains(Style::HIDDEN))
        .map(|run| run.text)
        .collect()
}

#[test]
fn new_quote_blank_consumes_one_separator_column() {
    let doc = parse(">\t\n");
    assert_eq!(doc.blocks()[0].kind, BlockKind::Blank);
    assert_eq!(doc.blocks()[0].container, Some(ContainerId(0)));
    assert_eq!(visible(&doc), "  \n");
    let tab: Vec<_> = doc.display_runs_in(Span::new(1, 2)).unwrap().collect();
    assert_eq!(tab.len(), 1);
    assert_eq!(tab[0].source, Span::new(1, 2));
    assert_eq!(tab[0].replacement, Some(Span::new(1, 2)));
    assert_eq!(tab[0].text, "  ");
}

#[test]
fn blank_separator_matches_independent_column_model() {
    let mut cases = 0;
    for outer in ["", " ", "  ", "   ", "> ", ">> ", "- ", "1. "] {
        for depth in 0..8 {
            let prefix = format!("{outer}{}", ">".repeat(depth));
            let col = prefix.len() + 1;
            let expanded = " ".repeat(4 - col % 4);
            for tail in ["", " ", "   ", "\t", " \t ", "\t\t", "        "] {
                for end in ["\n", "\r\n", ""] {
                    let source = format!("{prefix}>\t{tail}{end}");
                    let doc = parse(&source);
                    let model = parse(&format!("{prefix}>{expanded}{tail}{end}"));
                    assert_eq!(visible(&doc), visible(&model), "{source:?}");
                    assert_eq!(doc.blocks()[0].kind, BlockKind::Blank);
                    assert_eq!(doc.blocks()[0].container, model.blocks()[0].container);
                    assert_eq!(doc.container_count(), model.container_count());
                    for i in 0..doc.container_count() {
                        let a = doc.container(ContainerId(i)).unwrap();
                        let b = model.container(ContainerId(i)).unwrap();
                        assert_eq!(
                            (a.kind, a.parent, a.depth, a.list_tightness),
                            (b.kind, b.parent, b.depth, b.list_tightness)
                        );
                    }
                    assert!(doc.work().container_bytes <= 30 * source.len() as u64);
                    cases += 1;
                }
            }
        }
    }
    assert_eq!(cases, 1344);
}

#[test]
fn new_blank_keeps_identity_spacing_and_header_boundaries() {
    let mut doc = Document::new();
    doc.append("- [old](u)\n").unwrap();
    let old = doc.block_at(0).unwrap();
    let update = doc.append("  > >\t\r\n").unwrap();
    assert!(update.closed_containers.is_empty());
    assert!(update.loosened_lists.is_empty());
    let quote = doc.blocks()[1].container;
    assert_eq!(doc.blocks()[1].kind, BlockKind::Blank);
    doc.append("  > > new\n").unwrap();
    assert_eq!(doc.blocks()[2].container, quote);
    assert_eq!(doc.block_at(0).unwrap(), old);
    assert_eq!(doc.link_destination(LinkId(0)), Some("u"));
    assert_eq!(
        doc.container(ContainerId(0)).unwrap().list_tightness,
        Some(ListTightness::Tight)
    );
    doc.finish();

    let doc = parse("a|b\n>\t\n> c|d\n> -|-\n");
    assert_eq!(doc.tables().len(), 1);
    assert_eq!(doc.blocks()[1].kind, BlockKind::Blank);
    assert_eq!(
        doc.text(doc.tables()[0].cell(0, 0).unwrap().source),
        Some("c")
    );
    assert_eq!(
        doc.blocks()[doc.tables()[0].block.0].container,
        doc.blocks()[1].container
    );
}

#[test]
fn blank_separator_is_chunk_prefix_and_eof_invariant() {
    for source in [
        ">\t\n> 中🙂 &amp;\n",
        "- >\t\r\n  > [x](u)\n",
        ">\t>\t\t \r\n> > b\n",
        "a|b\n>\t\n> c|d\n> -|-\n",
        ">\t ",
        ">\t\r",
        "  >\t\n",
        "   >\t\t\n",
        "> ```\n>\t\n> ```\n",
        "> $$\n>\t\n> $$\n",
    ] {
        let expected = support::snapshot(&parse(source));
        for split in (0..=source.len()).filter(|&at| source.is_char_boundary(at)) {
            let mut doc = Document::new();
            doc.append(&source[..split]).unwrap();
            doc.append(&source[split..]).unwrap();
            doc.finish();
            assert_eq!(
                support::snapshot(&doc),
                expected,
                "split {split}: {source:?}"
            );
        }
        let mut doc = Document::new();
        for (at, ch) in source.char_indices() {
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            let prefix = &source[..at + ch.len_utf8()];
            let mut bulk = Document::new();
            bulk.append(prefix).unwrap();
            assert_eq!(
                support::snapshot(&doc),
                support::snapshot(&bulk),
                "prefix {prefix:?}"
            );
            assert_eq!(doc.pending(), bulk.pending());
            let mut eof = Document::new();
            for ch in prefix.chars() {
                eof.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            }
            eof.finish();
            assert_eq!(
                support::snapshot(&eof),
                support::snapshot(&parse(prefix)),
                "EOF {prefix:?}"
            );
        }
    }
}

#[test]
fn new_blank_mapping_slices_tiles_and_shared_budget() {
    let source = format!("{}中🙂\r\n>\t \r\n> &amp;\n", "a".repeat(240));
    let doc = parse(&source);
    let full = Span::new(0, source.len());
    assert_eq!(
        doc.runs_in(full)
            .unwrap()
            .map(|r| r.text)
            .collect::<String>(),
        source
    );
    let expected = visible(&doc);
    let scalars: String = source
        .char_indices()
        .flat_map(|(at, ch)| {
            doc.display_runs_in(Span::new(at, at + ch.len_utf8()))
                .unwrap()
        })
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    assert_eq!(scalars, expected);
    let tiles: String = (0..doc.tile_count())
        .flat_map(|i| doc.display_runs(TileId(i)).unwrap())
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    assert_eq!(tiles, expected);
    let tab = source.find(">\t ").unwrap() + 1;
    let mapped: Vec<_> = doc
        .display_runs_in(Span::new(tab, tab + 1))
        .unwrap()
        .collect();
    assert_eq!(mapped.len(), 1);
    assert_eq!(mapped[0].source, Span::new(tab, tab + 1));
    assert_eq!(mapped[0].text, "  ");
    assert!(mapped[0].replacement.is_some());
    let mut cursor = doc.display_cursor(full).unwrap();
    let mut second = doc.display_cursor(full).unwrap();
    let mut output = String::new();
    loop {
        let position = cursor.source_position();
        if position < full.end {
            let mut empty = Budget::new(WorkLimit::default());
            assert!(matches!(
                cursor.next(&doc, &mut empty).unwrap(),
                DisplayStep::BudgetExhausted
            ));
            assert_eq!(cursor.source_position(), position);
        }
        let mut budget = Budget::new(WorkLimit {
            bytes: 6,
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        match cursor.next(&doc, &mut budget).unwrap() {
            DisplayStep::Run(run) => {
                assert_eq!(run.source.start, position);
                if !run.style.contains(Style::HIDDEN) {
                    output.push_str(run.text);
                }
                assert!(matches!(
                    second.next(&doc, &mut budget).unwrap(),
                    DisplayStep::BudgetExhausted
                ));
                assert_eq!(second.source_position(), 0);
            }
            DisplayStep::Complete => break,
            DisplayStep::BudgetExhausted => panic!("one scalar fits"),
        }
        assert!(budget.used().bytes <= 6);
    }
    assert_eq!(output, expected);
}

#[test]
fn bounded_blank_probe_does_not_revisit_history_or_long_body() {
    for depth in 1..=100 {
        let source = format!("{}\n", ">\t".repeat(depth));
        let doc = parse(&source);
        assert_eq!(doc.container_count(), depth);
        assert_eq!(doc.blocks()[0].kind, BlockKind::Blank);
        assert_eq!(visible(&doc), "  \n");
        assert!(doc.work().container_bytes < 30 * source.len() as u64);
    }
    let prefix = ">".repeat(968);
    let mut doc = Document::new();
    doc.append(&format!("{prefix}>\t\n")).unwrap();
    assert_eq!(doc.container_count(), 969);
    assert_eq!(doc.blocks()[0].kind, BlockKind::Blank);
    assert!(doc.work().container_bytes < 30_000);
    doc.append(&format!("{prefix}> {}", "a".repeat(2048)))
        .unwrap();
    let before = doc.work();
    for _ in 0..1000 {
        doc.append(&"x".repeat(1000)).unwrap();
    }
    let after = doc.work();
    assert_eq!(after.container_bytes, before.container_bytes);
    assert_eq!(after.container_steps, before.container_steps);
    assert_eq!(after.classified_bytes, before.classified_bytes);
    assert_eq!(after.inline_bytes - before.inline_bytes, 1_000_000);
    doc.finish();

    let mut doc = Document::new();
    for _ in 0..4096 {
        doc.append(">\t\nroot\n").unwrap();
    }
    let before = doc.work();
    doc.append(">\t\n").unwrap();
    assert!(doc.work().container_bytes - before.container_bytes <= 40);
    assert!(doc.work().container_steps - before.container_steps <= 8);
    assert_eq!(doc.blocks().last().unwrap().kind, BlockKind::Blank);
}

#[test]
fn incomplete_and_long_blank_keep_existing_window_rules() {
    let mut doc = Document::new();
    doc.append(">\t ").unwrap();
    assert!(doc.blocks().is_empty());
    assert_eq!(doc.text(doc.pending()), Some(">\t "));
    doc.finish();
    assert_eq!(visible(&doc), "   ");
    for end in ["", "\n"] {
        let source = format!(">\t{}{end}", " ".repeat(BLOCK_LOOKAHEAD + 10));
        let doc = parse(&source);
        assert!(!matches!(doc.blocks()[0].kind, BlockKind::Blank));
        assert!(
            doc.display_runs_in(Span::new(1, 2))
                .unwrap()
                .all(|r| r.replacement.is_none())
        );
        let mut streamed = Document::new();
        for ch in source.chars() {
            streamed.append(ch.encode_utf8(&mut [0; 4])).unwrap();
        }
        streamed.finish();
        assert_eq!(support::snapshot(&streamed), support::snapshot(&doc));
    }
}
