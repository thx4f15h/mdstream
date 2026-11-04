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
fn partial_tab_opens_quote_without_replacing_ancestors() {
    let source = "- parent\n\t> child\n    > next\n- sibling\n";
    let doc = parse(source);
    assert_eq!(visible(&doc), "parent\nchild\nnext\nsibling\n");
    assert_eq!(doc.container_count(), 4);
    let parent = doc.blocks()[0].container.unwrap();
    let quote = doc.container(doc.blocks()[1].container.unwrap()).unwrap();
    assert_eq!(quote.kind, ContainerKind::Quote);
    assert_eq!(quote.parent, Some(parent));
    assert_eq!(quote.depth, 3);
    assert_eq!(
        quote.source,
        Span::new(9, source.find("- sibling").unwrap())
    );
    assert_eq!(doc.blocks()[1].container, doc.blocks()[2].container);
    assert_ne!(doc.blocks()[3].container, Some(parent));
    assert_eq!(
        doc.container(ContainerId(0)).unwrap().list_tightness,
        Some(ListTightness::Tight)
    );

    let mut doc = Document::new();
    doc.append("- parent\n\n").unwrap();
    let update = doc.append("\t> child\n").unwrap();
    assert_eq!(update.new_containers, 2..3);
    assert!(update.closed_containers.is_empty());
    assert_eq!(update.loosened_lists, vec![ContainerId(0)]);
    assert_eq!(
        doc.container(ContainerId(2)).unwrap().parent,
        Some(ContainerId(1))
    );
    assert_eq!(
        doc.container(ContainerId(0)).unwrap().list_tightness,
        Some(ListTightness::Loose)
    );
}

#[test]
fn failed_paths_priorities_and_quote_payload_keep_existing_dialect() {
    let doc = parse("> parent\n>\t> child\n");
    assert_eq!(visible(&doc), "parent\nchild\n");
    assert_eq!(doc.container_count(), 2);
    assert_eq!(
        doc.container(ContainerId(1)).unwrap().parent,
        Some(ContainerId(0))
    );
    for (source, expected) in [
        ("- parent\n\t\t> excess\n", "parent\n\t\t> excess\n"),
        (
            "- parent\n   - first\n\t> incomplete\n",
            "parent\nfirst\n\t> incomplete\n",
        ),
        ("> - parent\n\t> missing\n", "parent\n\t> missing\n"),
        ("- ```\n\t> literal\n  ```\n", "  > literal\n"),
        ("- $$\n\t> literal\n  $$\n", "  > literal\n"),
    ] {
        assert_eq!(visible(&parse(source)), expected, "{source:?}");
    }
    // Only ancestor indentation changes. The new quote's own separator,
    // nested markers and leaf rules are the existing explicit dialect.
    for prefix in [
        "- *open\n",
        "- a|b\n",
        "- a|b\n  -|-\n  c|d\n",
        "- # heading\n",
        "- parent\n\n",
    ] {
        for body in [
            "> *tail*",
            ">\ttext",
            ">\t> child",
            "> - - -",
            "> ===",
            "> [r]: /u",
            "> ```rs\n    > body\n    > ```",
            "> $$\n    > x\n    > $$",
            "> |a|b|\n    > |-|-|\n    > |c|d|",
        ] {
            let source = format!("{prefix}\t{body}\n");
            let expanded = format!("{prefix}    {body}\n");
            let doc = parse(&source);
            let model = parse(&expanded);
            assert_eq!(visible(&doc), visible(&model), "{source:?}");
            assert_eq!(doc.container_count(), model.container_count(), "{source:?}");
            assert_eq!(doc.tables().len(), model.tables().len(), "{source:?}");
            let quote = doc.container(ContainerId(2)).unwrap();
            assert_eq!(quote.kind, ContainerKind::Quote, "{source:?}");
            assert_eq!(quote.parent, Some(ContainerId(1)), "{source:?}");
        }
    }
    let doc = parse("- *open\n\t> tail*\n");
    assert_eq!(visible(&doc), "*open\ntail*\n");
    assert!(
        doc.display_runs_in(Span::new(0, doc.source().len()))
            .unwrap()
            .all(|run| !run.style.contains(Style::EMPHASIS))
    );
    let doc = parse("- parent\n\t>\t\tcode\n");
    assert_eq!(doc.blocks()[1].kind, BlockKind::Code { info: None });
    assert_eq!(visible(&doc), "parent\n  code\n");
    let tab = doc.source().rfind('\t').unwrap();
    let runs: Vec<_> = doc
        .display_runs_in(Span::new(tab, tab + 1))
        .unwrap()
        .collect();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].text, "  ");
    assert_eq!(runs[0].replacement, Some(Span::new(tab, tab + 1)));
}

#[test]
fn nested_quotes_match_independent_expanded_column_model() {
    let mut cases = 0;
    for outer in ["", "> "] {
        for initial in 0..=3 {
            for depth in 1..=4 {
                for ancestor in ["-", "1."] {
                    let mut opener = String::new();
                    let mut indent = initial;
                    let mut targets = Vec::new();
                    for _ in 0..depth {
                        opener
                            .push_str(&format!("{outer}{}{ancestor} parent\n", " ".repeat(indent)));
                        indent += ancestor.len() + 1;
                        targets.push(outer.len() + indent);
                    }
                    for ws in [
                        "\t", " \t", "  \t", "\t ", "\t  ", "\t\t", " \t\t ", "\t\t\t", "\t\t\t\t",
                    ] {
                        let mut col = outer.len();
                        let mut partial = false;
                        for byte in ws.bytes() {
                            let next = col + if byte == b'\t' { 4 - col % 4 } else { 1 };
                            partial |= targets.iter().any(|&target| col < target && target < next);
                            col = next;
                        }
                        if !partial || col < outer.len() + indent || col - outer.len() - indent > 3
                        {
                            continue;
                        }
                        for child in [
                            "> 中🙂",
                            "> > inner",
                            "> - [x] child",
                            "> # heading",
                            ">",
                            ">     code",
                            ">\t\tcode",
                            "> $$",
                            "> |a|b|",
                        ] {
                            for ending in ["\n", "\r\n", ""] {
                                let source = format!("{opener}{outer}{ws}{child}{ending}");
                                let expanded = format!(
                                    "{opener}{outer}{}{child}{ending}",
                                    " ".repeat(col - outer.len())
                                );
                                let actual = parse(&source);
                                let model = parse(&expanded);
                                assert_eq!(visible(&actual), visible(&model), "{source:?}");
                                assert_eq!(
                                    actual.blocks().len(),
                                    model.blocks().len(),
                                    "{source:?}"
                                );
                                for (a, b) in actual.blocks().iter().zip(model.blocks()) {
                                    let (mut a_kind, mut b_kind) = (a.kind.clone(), b.kind.clone());
                                    for (kind, doc) in
                                        [(&mut a_kind, &actual), (&mut b_kind, &model)]
                                    {
                                        if let BlockKind::Math { content, .. } = kind {
                                            assert_eq!(doc.text(*content), Some(""));
                                            *content = Span::new(0, 0);
                                        }
                                        if let BlockKind::ListItem {
                                            task: Some(task), ..
                                        } = kind
                                        {
                                            assert_eq!(doc.text(task.source), Some("[x]"));
                                            task.source = Span::new(0, 0);
                                        }
                                    }
                                    assert_eq!(
                                        (a_kind, a.container),
                                        (b_kind, b.container),
                                        "{source:?}"
                                    );
                                }
                                assert_eq!(
                                    actual.container_count(),
                                    model.container_count(),
                                    "{source:?}"
                                );
                                for i in 0..actual.container_count() {
                                    let mut a = actual.container(ContainerId(i)).unwrap();
                                    let mut b = model.container(ContainerId(i)).unwrap();
                                    for (node, doc) in [(&mut a, &actual), (&mut b, &model)] {
                                        if let ContainerKind::Item {
                                            task: Some(task), ..
                                        } = &mut node.kind
                                        {
                                            assert_eq!(doc.text(task.source), Some("[x]"));
                                            task.source = Span::new(0, 0);
                                        }
                                    }
                                    assert_eq!(
                                        (a.kind, a.parent, a.depth, a.list_tightness),
                                        (b.kind, b.parent, b.depth, b.list_tightness),
                                        "{source:?}"
                                    );
                                }
                                assert!(actual.work().container_steps < 12 * source.len() as u64);
                                assert!(actual.work().container_bytes < 24 * source.len() as u64);
                                cases += 1;
                            }
                        }
                    }
                }
            }
        }
    }
    assert_eq!(cases, 3051);
    eprintln!("{cases} independent expanded-column models");
}

#[test]
fn every_split_prefix_and_window_preserve_state_and_ids() {
    for source in [
        "> parent\n>\t> child\n".to_owned(),
        "- parent\n   - first\n\t> incomplete\n".to_owned(),
        " - foo\n   - bar\n\t > 中🙂\r\n     > next\n- root\n".to_owned(),
        "> - foo\n>\t> - [x] bar\n".to_owned(),
        "- foo\n\t>\t\tcode\n".to_owned(),
        "- foo\n\t>\n\t> - - -\n\t> |a|b|\n".to_owned(),
        "- foo\n\t> ```\n\t> body\n\t> ```\n".to_owned(),
        "- foo\n\t> $$\n\t> x\n\t> $$\n".to_owned(),
        format!("- foo\n\t> {}\r\n", "x".repeat(BLOCK_LOOKAHEAD + 9)),
        format!("- foo\n\t{}> late\n", " ".repeat(BLOCK_LOOKAHEAD + 9)),
        "- foo\n\t> eof".to_owned(),
    ] {
        let expected = support::snapshot(&parse(&source));
        for split in (0..=source.len()).filter(|&i| source.is_char_boundary(i)) {
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
            let mut prefix = Document::new();
            prefix.append(&source[..at + ch.len_utf8()]).unwrap();
            assert_eq!(doc.pending(), prefix.pending());
            assert_eq!(support::snapshot(&doc), support::snapshot(&prefix));
        }
        doc.finish();
        assert_eq!(support::snapshot(&doc), expected);
    }
    let mut doc = Document::new();
    doc.append(" - foo\n   - bar\n").unwrap();
    let parents: Vec<_> = doc.blocks().iter().map(|b| b.container).collect();
    let mut reader = doc
        .display_cursor(Span::new(0, doc.source().len()))
        .unwrap();
    let update = doc.append("\t > baz\n").unwrap();
    assert_eq!(update.new_containers, 4..5);
    assert!(update.closed_containers.is_empty());
    assert!(update.loosened_lists.is_empty());
    assert_eq!(
        reader.next(&doc, &mut Budget::new(WorkLimit::unlimited())),
        Err(LayoutError::StaleContinuation)
    );
    for (i, id) in parents.into_iter().enumerate() {
        assert_eq!(doc.blocks()[i].container, id);
    }
    let source = format!("- parent\n\t{}> late\n", " ".repeat(BLOCK_LOOKAHEAD + 9));
    let doc = parse(&source);
    assert_eq!(doc.container_count(), 2);
    assert!(visible(&doc).contains("> late"));
}

#[test]
fn hidden_partial_prefix_preserves_source_slices_tiles_and_budget() {
    let source = format!("{}\n\n- foo\n\t> 中🙂\tend\r\n", "x".repeat(248));
    assert_eq!(source.find('\t'), Some(256));
    let doc = parse(&source);
    let expected = format!("{}\n\nfoo\n中🙂\tend\n", "x".repeat(248));
    assert_eq!(visible(&doc), expected);
    let mut end = 0;
    for run in doc.display_runs_in(Span::new(0, source.len())).unwrap() {
        assert_eq!(run.source.start, end);
        end = run.source.end;
        assert_eq!(run.text, &source[run.source.range()]);
        assert!(run.replacement.is_none());
        if run.source.start <= 256 && 256 < run.source.end {
            assert!(run.style.contains(Style::HIDDEN));
        }
    }
    assert_eq!(end, source.len());
    let sliced: String = source
        .char_indices()
        .flat_map(|(at, ch)| {
            doc.display_runs_in(Span::new(at, at + ch.len_utf8()))
                .unwrap()
        })
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    assert_eq!(sliced, expected);
    let tiled: String = (0..doc.tile_count())
        .flat_map(|i| doc.display_runs(TileId(i)).unwrap())
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    assert_eq!(tiled, expected);
    let mut reader = doc.display_cursor(Span::new(0, source.len())).unwrap();
    let mut output = String::new();
    while reader.source_position() < source.len() {
        let old = reader.source_position();
        assert_eq!(
            reader
                .next(&doc, &mut Budget::new(WorkLimit::default()))
                .unwrap(),
            DisplayStep::BudgetExhausted
        );
        assert_eq!(reader.source_position(), old);
        let mut budget = Budget::new(WorkLimit {
            bytes: 6,
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        let DisplayStep::Run(run) = reader.next(&doc, &mut budget).unwrap() else {
            panic!("no progress")
        };
        assert!(budget.used().bytes <= 6);
        assert_eq!(run.source.start, old);
        assert!(reader.source_position() > old);
        if !run.style.contains(Style::HIDDEN) {
            output.push_str(run.text);
        }
    }
    assert_eq!(output, expected);
}

#[test]
fn deep_million_byte_body_and_many_new_quotes_do_not_rescan_history() {
    let mut doc = Document::new();
    doc.append(&format!(
        "{}parent\n{}> {}",
        "- ".repeat(499),
        "\t".repeat(250),
        "x".repeat(1_000_000)
    ))
    .unwrap();
    let last = doc.blocks().last().unwrap();
    assert_eq!(doc.container(last.container.unwrap()).unwrap().depth, 999);
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("x").unwrap();
    }
    assert_eq!(doc.work().classified_bytes, before.classified_bytes);
    assert_eq!(doc.work().container_bytes, before.container_bytes);
    assert_eq!(doc.work().container_steps, before.container_steps);
    assert!(doc.work().inline_bytes - before.inline_bytes <= 1001);
    doc.append("\n").unwrap();
    doc.finish();
    let mut doc = Document::new();
    for _ in 0..4096 {
        doc.append("- parent\n\t> child\n").unwrap();
    }
    doc.finish();
    assert_eq!(doc.blocks().len(), 8192);
    assert_eq!(doc.container_count(), 4096 * 2 + 1);
    assert!(doc.work().classified_bytes < 4096 * 60);
    assert!(doc.work().container_bytes < 4096 * 150);
    assert!(doc.work().container_steps < 4096 * 25);
    assert_eq!(visible(&doc), "parent\nchild\n".repeat(4096));
}
