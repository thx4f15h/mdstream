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
fn complete_blank_keeps_quote_after_split_item_indent() {
    let mut doc = Document::new();
    doc.append("- > [old](u)\n").unwrap();
    let scope = doc.blocks()[0].container;
    let update = doc.append("\t> \n").unwrap();
    assert!(update.closed_containers.is_empty());
    assert_eq!(doc.container_count(), 3);
    assert_eq!(doc.blocks()[1].kind, BlockKind::Blank);
    assert_eq!(doc.blocks()[1].container, scope);
    doc.append("\t> new\n").unwrap();
    doc.finish();
    assert_eq!(doc.container_count(), 3);
    assert_eq!(doc.blocks()[2].container, scope);
    assert_eq!(doc.link_destination(LinkId(0)), Some("u"));
    assert_eq!(visible(&doc), "old\n\nnew\n");
}

#[test]
fn blanks_match_independent_physical_column_model() {
    let mut cases = 0;
    for outer in ["", "> ", ">> ", "   > "] {
        for marker in ["- ", "1. "] {
            for depth in 1..=6 {
                let target = outer.len() + marker.len() * depth;
                for leading in 0..4 {
                    let mut col = outer.len() + leading;
                    let mut prefix = format!("{outer}{}", " ".repeat(leading));
                    while col < target {
                        prefix.push('\t');
                        col = (col / 4 + 1) * 4;
                    }
                    let expanded = format!("{outer}{}", " ".repeat(col - outer.len()));
                    // Quotes behind the split tab, including another item,
                    // expose path retention that an unprefixed blank cannot.
                    for inner in ["", "> ", "> - "] {
                        let continuation = inner.replace("- ", "  ");
                        for tail in ["", " ", "   "] {
                            let opening = format!("{outer}{}{inner}a\n", marker.repeat(depth));
                            let source = format!(
                                "{opening}{prefix}{continuation}{tail}\n{prefix}{continuation}b\n"
                            );
                            let model_source = format!(
                                "{opening}{expanded}{continuation}{tail}\n{expanded}{continuation}b\n"
                            );
                            let doc = parse(&source);
                            let model = parse(&model_source);
                            assert_eq!(visible(&doc), visible(&model), "{source:?}");
                            assert_eq!(doc.blocks().len(), model.blocks().len(), "{source:?}");
                            for (a, b) in doc.blocks().iter().zip(model.blocks()) {
                                assert_eq!(
                                    (&a.kind, a.container),
                                    (&b.kind, b.container),
                                    "{source:?}"
                                );
                            }
                            assert_eq!(
                                doc.container_count(),
                                model.container_count(),
                                "{source:?}"
                            );
                            for i in 0..doc.container_count() {
                                let a = doc.container(ContainerId(i)).unwrap();
                                let b = model.container(ContainerId(i)).unwrap();
                                assert_eq!(
                                    (a.kind, a.parent, a.depth, a.list_tightness),
                                    (b.kind, b.parent, b.depth, b.list_tightness),
                                    "{source:?}"
                                );
                            }
                            assert!(doc.work().container_bytes < 25 * source.len() as u64);
                            assert!(doc.work().classified_bytes < 12 * source.len() as u64);
                            cases += 1;
                        }
                    }
                }
            }
        }
    }
    assert_eq!(cases, 1728);
}

#[test]
fn blank_scope_controls_spacing_and_table_fallback() {
    // A blank inside a quote does not separate the containing item's content.
    let mut quoted = Document::new();
    quoted.append("- > a\n").unwrap();
    let scope = quoted.blocks()[0].container;
    let update = quoted.append("\t>\t\r\n").unwrap();
    assert!(update.closed_containers.is_empty());
    assert!(update.loosened_lists.is_empty());
    assert_eq!(quoted.blocks()[1].container, scope);
    assert_eq!(quoted.blocks()[1].kind, BlockKind::Blank);
    quoted.append("\t> b\n").unwrap();
    assert_eq!(
        quoted.container(ContainerId(0)).unwrap().list_tightness,
        Some(ListTightness::Tight)
    );

    // A direct item blank separates its paragraphs and emits one loosen event.
    let mut item = Document::new();
    item.append("- a|b\n").unwrap();
    let scope = item.blocks()[0].container;
    let blank = item.append("\t \n").unwrap();
    assert!(blank.closed_containers.is_empty());
    assert!(blank.loosened_lists.is_empty());
    assert!(item.tables().is_empty());
    assert_eq!(item.blocks()[1].container, scope);
    let update = item.append("\tc|d\n\t-|-\n").unwrap();
    assert_eq!(update.loosened_lists, [ContainerId(0)]);
    assert_eq!(item.tables().len(), 1);
    assert_eq!(
        item.text(item.tables()[0].cell(0, 0).unwrap().source),
        Some("c")
    );
    assert_eq!(item.blocks()[item.tables()[0].block.0].container, scope);
    assert_eq!(
        item.container(ContainerId(0)).unwrap().list_tightness,
        Some(ListTightness::Loose)
    );
}

#[test]
fn blank_paths_are_chunk_prefix_and_eof_invariant() {
    for source in [
        "- > *中🙂*\r\n\t>\t\r\n\t> [x](u)\r\n",
        "- a|b\n\t \n\tc|d\n\t-|-\n",
        "- > - a\n\t>\t \n\t>   b\n",
        "- > a\n\t> ",
        "- > a\n\t>\r",
        "- > a\n\t\n\tb\n",
        "- > -\n\t>\n\t>   b\n",
        "- ```\n\t \n\t```\n",
        "- $$\n\t \n\t$$\n",
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
fn blank_mapping_slices_tiles_and_shared_budget() {
    let source = format!("- > {}中🙂\r\n\t>\t \r\n\t> &amp;\n", "a".repeat(240));
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
fn deep_blank_and_long_append_do_not_revisit_retained_path() {
    let outer = ">".repeat(970);
    let mut doc = Document::new();
    doc.append(&format!("{outer}- > [old](u)\n")).unwrap();
    let scope = doc.blocks()[0].container;
    let update = doc.append(&format!("{outer}\t> \n")).unwrap();
    assert!(update.closed_containers.is_empty());
    assert_eq!(doc.blocks()[1].kind, BlockKind::Blank);
    assert_eq!(doc.blocks()[1].container, scope);
    doc.append(&format!("{outer}\t> {}", "a".repeat(2048)))
        .unwrap();
    let before = doc.work();
    for _ in 0..1000 {
        doc.append(&"x".repeat(1000)).unwrap();
        assert!(doc.pending().len() <= BLOCK_LOOKAHEAD + 3);
    }
    let after = doc.work();
    assert_eq!(after.container_steps, before.container_steps);
    assert_eq!(after.container_bytes, before.container_bytes);
    assert_eq!(after.classified_bytes, before.classified_bytes);
    assert_eq!(after.inline_bytes - before.inline_bytes, 1_000_000);
    doc.append("\n").unwrap();
    doc.finish();
    assert_eq!(doc.container_count(), 973);
    assert_eq!(doc.blocks()[2].container, scope);
    assert_eq!(doc.link_destination(LinkId(0)), Some("u"));
    assert!(doc.work().container_bytes < 30 * BLOCK_LOOKAHEAD as u64);

    let mut doc = Document::new();
    doc.append("- > old\n").unwrap();
    for _ in 0..4096 {
        doc.append("\t> \n\t> next\n").unwrap();
    }
    let before = doc.work();
    let update = doc.append("\t> \n").unwrap();
    assert!(update.closed_containers.is_empty());
    assert_eq!(doc.container_count(), 3);
    assert!(doc.work().container_steps - before.container_steps <= 8);
    assert!(doc.work().container_bytes - before.container_bytes <= 40);
}

#[test]
fn incomplete_missing_and_out_of_window_paths_keep_their_boundaries() {
    let mut doc = Document::new();
    doc.append("- > a\n\t> ").unwrap();
    assert_eq!(doc.blocks().len(), 1);
    assert_eq!(doc.text(doc.pending()), Some("\t> "));
    doc.finish();
    assert_eq!(doc.blocks()[1].kind, BlockKind::Blank);
    assert_eq!(doc.blocks()[1].container, doc.blocks()[0].container);

    let mut doc = Document::new();
    doc.append("- > a\n").unwrap();
    let update = doc.append("\t\n").unwrap();
    assert_eq!(update.closed_containers, [ContainerId(2)]);
    assert_eq!(doc.blocks()[1].container, Some(ContainerId(1)));
    assert_eq!(visible(&doc), "a\n\t\n");

    // Both the initial quote and its complete continuation now consume one
    // separator column and map the residual columns to the original tab.
    assert_eq!(visible(&parse(">\t\n>\t\n")), "  \n  \n");
    for ending in ["", "\n"] {
        let source = format!("- > a\n\t> {}{ending}", " ".repeat(BLOCK_LOOKAHEAD + 10));
        let doc = parse(&source);
        assert!(!matches!(doc.blocks()[1].kind, BlockKind::Blank));
        let mut streamed = Document::new();
        for ch in source.chars() {
            streamed.append(ch.encode_utf8(&mut [0; 4])).unwrap();
        }
        streamed.finish();
        assert_eq!(support::snapshot(&streamed), support::snapshot(&doc));
    }
}
