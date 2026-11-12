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
fn marker_delimiters_match_physical_columns() {
    let mut cases = 0;
    for outer in ["", "> ", ">> ", "   > "] {
        for marker in ["- ", "1. "] {
            for depth in 1..=6 {
                let opening = format!("{outer}{}a|b\n", marker.repeat(depth));
                let target = outer.len() + marker.len() * depth;
                for leading in 0..4 {
                    let mut col = outer.len() + leading;
                    let mut prefix = format!("{outer}{}", " ".repeat(leading));
                    while col < target {
                        prefix.push('\t');
                        col += 4 - col % 4;
                    }
                    let expanded = format!("{outer}{}", " ".repeat(col - outer.len()));
                    for (body, tables) in [
                        ("- | -", 1),
                        ("- | -:", 1),
                        ("- | :-", 1),
                        ("nope", 0),
                        ("wrong|three|cells", 0),
                        ("`x`|&amp;", 0),
                        ("- | x", 0),
                        ("- | - | -", 0),
                    ] {
                        let source = format!("{opening}{prefix}{body}\n");
                        let actual = parse(&source);
                        let model = parse(&format!("{opening}{expanded}{body}\n"));
                        assert_eq!(actual.tables().len(), tables, "{source:?}");
                        assert_eq!(actual.tables().len(), model.tables().len());
                        assert_eq!(visible(&actual), visible(&model), "{source:?}");
                        assert_eq!(actual.container_count(), model.container_count());
                        for i in 0..actual.container_count() {
                            let a = actual.container(ContainerId(i)).unwrap();
                            let b = model.container(ContainerId(i)).unwrap();
                            assert_eq!(
                                (a.kind, a.parent, a.depth, a.list_tightness),
                                (b.kind, b.parent, b.depth, b.list_tightness)
                            );
                        }
                        for (a, b) in actual.tables().iter().zip(model.tables()) {
                            assert_eq!(a.columns, b.columns);
                            assert_eq!(a.rows().len(), b.rows().len());
                        }
                        assert!(actual.work().container_bytes < 20 * source.len() as u64);
                        assert!(actual.work().classified_bytes < 12 * source.len() as u64);
                        cases += 1;
                    }
                }
            }
        }
    }
    assert_eq!(cases, 1536);
}

#[test]
fn rejected_header_then_marker_delimiter_preserves_identity() {
    let mut doc = Document::new();
    doc.append("- [old](u)|b\n\t*中🙂*|[a](v)|c\n").unwrap();
    let scope = doc.blocks()[0].container;
    let link = *doc.link_info(LinkId(0)).unwrap();
    let update = doc.append("\t- | -: | -\n").unwrap();
    assert_eq!(doc.tables().len(), 1);
    let table = &doc.tables()[0];
    assert_eq!(
        table
            .columns
            .iter()
            .map(|c| c.alignment)
            .collect::<Vec<_>>(),
        [Alignment::Default, Alignment::Right, Alignment::Default]
    );
    assert_eq!(doc.blocks()[table.block.0].container, scope);
    assert_eq!(doc.text(table.cell(0, 0).unwrap().source), Some("*中🙂*"));
    assert_eq!(
        *doc.link_info(LinkId(0)).unwrap(),
        LinkInfo {
            state: SyntaxState::Sealed,
            ..link
        }
    );
    assert_eq!(doc.link_destination(LinkId(0)), Some("u"));
    assert_eq!(update.new_tables, 0..1);
    assert!(update.closed_containers.is_empty());
    let block = table.block;
    doc.append("\td|e|f\n").unwrap();
    doc.finish();
    assert_eq!(doc.tables()[0].block, block);
    assert_eq!(doc.tables()[0].rows().len(), 2);
    assert!(visible(&doc).starts_with("old|b\n"));
    assert_eq!(doc.link_destination(LinkId(1)), Some("v"));
}

#[test]
fn marker_lines_are_chunk_and_eof_invariant() {
    for source in [
        "- *中🙂*|[a](u)\r\n\t- | -:\r\n\tc|d\r\n",
        "1. a|b\n\tnope\n",
        "> a|b\n>\t- | -",
        "- a|b\n\twrong|three|cells\n\t- | - | -\n",
        "- a|b\n\tnope",
        "- a|b\n\t",
        "- a|b\n\t`x`|&amp;\n\t- | -\n",
        "- a|b\n\t- | -\n",
        "- > a|b\n\t- | -\n",
        "- a|b\n\t  -|-\n",
        "- a|b\n\t- | x\n",
        "- a|b\n\t- | - | -\n",
        "- parent\n\t- a|b\n\t- | -\n",
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
            assert_eq!(support::snapshot(&eof), support::snapshot(&parse(prefix)));
        }
    }
}

#[test]
fn marker_line_mapping_and_shared_budget() {
    for ending in ["\t- | -\n\t&amp;|`x`\n", "\tnope\n", "\t"] {
        let source = format!("- *中🙂*|[a](u)\n{ending}");
        let doc = parse(&source);
        let full = Span::new(0, source.len());
        assert_eq!(
            doc.runs_in(full)
                .unwrap()
                .map(|r| r.text)
                .collect::<String>(),
            source
        );
        let scalars: String = source
            .char_indices()
            .flat_map(|(at, ch)| {
                doc.display_runs_in(Span::new(at, at + ch.len_utf8()))
                    .unwrap()
            })
            .filter(|r| !r.style.contains(Style::HIDDEN))
            .map(|r| r.text)
            .collect();
        let tiled: String = (0..doc.tile_count())
            .flat_map(|i| doc.display_runs(TileId(i)).unwrap())
            .filter(|r| !r.style.contains(Style::HIDDEN))
            .map(|r| r.text)
            .collect();
        assert_eq!(scalars, visible(&doc));
        assert_eq!(tiled, scalars);
        let tab = source.find('\t').unwrap();
        let shown: String = doc
            .display_runs_in(Span::new(tab, tab + 1))
            .unwrap()
            .filter(|r| !r.style.contains(Style::HIDDEN))
            .map(|r| r.text)
            .collect();
        assert_eq!(shown, if ending.starts_with("\t-") { "" } else { "  " });
        let mut cursor = doc.display_cursor(full).unwrap();
        let mut second = doc.display_cursor(full).unwrap();
        let mut output = String::new();
        loop {
            let position = cursor.source_position();
            let mut empty = Budget::new(WorkLimit::default());
            if position < source.len() {
                assert!(matches!(
                    cursor.next(&doc, &mut empty).unwrap(),
                    DisplayStep::BudgetExhausted
                ));
                assert_eq!(cursor.source_position(), position);
            }
            let mut budget = Budget::new(WorkLimit {
                queries: 1,
                objects: 1,
                bytes: 6,
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
        assert_eq!(output, visible(&doc));
    }
}

#[test]
fn scope_code_boundaries_and_long_delimiter_extension() {
    for source in ["- a|b\n\t- | -", "> a|b\n>\t- | -\r\n"] {
        assert_eq!(parse(source).tables().len(), 1, "{source:?}");
    }
    for source in [
        format!("- a|b\n\t{}|-\n", "-".repeat(1100)),
        format!("- a|b\n\t- | {}\n", "-".repeat(1100)),
    ] {
        assert_eq!(parse(&source).tables().len(), 1, "{source:?}");
    }
    for source in [
        "- a|b\n- | -\n".to_owned(),
        "- > a|b\n\t- | -\n".to_owned(),
        "- a|b\n\t  -|-\n".to_owned(),
        "- a|b\n\t  - | -\n".to_owned(),
        "- parent\n\t- a|b\n\t- | -\n".to_owned(),
        "- text\n\t- | -\n".to_owned(),
    ] {
        assert!(parse(&source).tables().is_empty(), "{source:?}");
    }
}

#[test]
fn deep_marker_delimiter_does_not_rescan_history() {
    let outer = ">".repeat(980);
    let mut doc = Document::new();
    doc.append(&format!(
        "{outer}- a|b\n{outer}\t[a](u){}",
        "x".repeat(1_000_000)
    ))
    .unwrap();
    let scope = doc.blocks()[0].container;
    let link = *doc.link_info(LinkId(0)).unwrap();
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("x").unwrap();
    }
    assert_eq!(doc.work().container_steps, before.container_steps);
    assert_eq!(doc.work().container_bytes, before.container_bytes);
    assert_eq!(doc.work().classified_bytes, before.classified_bytes);
    assert_eq!(doc.work().inline_bytes - before.inline_bytes, 1000);
    let tail = format!("|c\n{outer}\t- | -\n");
    doc.append(&tail).unwrap();
    assert!(doc.work().container_bytes - before.container_bytes < 20 * tail.len() as u64);
    assert!(doc.work().classified_bytes - before.classified_bytes < 12 * tail.len() as u64);
    assert!(doc.work().container_steps - before.container_steps < 10 * tail.len() as u64);
    assert_eq!(doc.tables().len(), 1);
    assert_eq!(doc.blocks()[doc.tables()[0].block.0].container, scope);
    assert_eq!(
        *doc.link_info(LinkId(0)).unwrap(),
        LinkInfo {
            state: SyntaxState::Sealed,
            ..link
        }
    );
    assert_eq!(doc.container_count(), 982);
    doc.finish();
    assert!(doc.work().table_header_bytes <= 2 * doc.source().len() as u64);
    assert!(doc.work().inline_bytes <= 2 * doc.source().len() as u64);
}
