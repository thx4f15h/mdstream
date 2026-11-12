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

fn assert_model(source: &str, expanded: &str) {
    let actual = parse(source);
    let model = parse(expanded);
    assert_eq!(visible(&actual), visible(&model), "{source:?}");
    assert_eq!(actual.blocks().len(), model.blocks().len(), "{source:?}");
    for (a, b) in actual.blocks().iter().zip(model.blocks()) {
        let (mut a_kind, mut b_kind) = (a.kind.clone(), b.kind.clone());
        for (kind, doc) in [(&mut a_kind, &actual), (&mut b_kind, &model)] {
            if let BlockKind::ListItem {
                task: Some(task), ..
            } = kind
            {
                assert_eq!(doc.text(task.source), Some("[x]"));
                task.source = Span::new(0, 0);
            }
        }
        assert_eq!((a_kind, a.container), (b_kind, b.container), "{source:?}");
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
}

#[test]
fn different_markers_match_independent_expanded_column_model() {
    let mut cases = 0;
    for outer in ["", "> ", "> > "] {
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
                            partial |= targets.iter().any(|&t| col < t && t < next);
                            col = next;
                        }
                        if !partial || col < outer.len() + indent || col - outer.len() - indent > 3
                        {
                            continue;
                        }
                        let spaces = " ".repeat(col - outer.len());
                        for (first, sibling) in [
                            ("-", "+"),
                            ("+", "*"),
                            ("*", "-"),
                            ("-", "23."),
                            ("1.", "4)"),
                            ("2)", "+"),
                        ] {
                            for body in ["中🙂", "[x] task", "- nested", "> quote", "\t\tcode"] {
                                for ending in ["\n", "\r\n", ""] {
                                    let prefix = format!("{opener}{outer}{spaces}{first} first\n");
                                    assert_model(
                                        &format!("{prefix}{outer}{ws}{sibling} {body}{ending}"),
                                        &format!("{prefix}{outer}{spaces}{sibling} {body}{ending}"),
                                    );
                                    cases += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(cases > 10_000);
    eprintln!("{cases} independent expanded-column sibling models");
}

#[test]
fn mixed_markers_every_split_prefix_eof_and_window() {
    for source in [
        "- parent\n\t- first\n\t+ [x] 中🙂\r\n\t- eof".to_owned(),
        "> - parent\n>   - first\n>\t+ second\n".to_owned(),
        "- parent\n\t- first\n      > child\n\n\t+ second\n".to_owned(),
        "- parent\n\t- first\n\t1)\t\tcode\n".to_owned(),
        "- parent\n\t- first\n\t- - -\n\t+ changed\n".to_owned(),
        format!(
            "- parent\n\t- first\n\t+ {}\r\n",
            "x".repeat(BLOCK_LOOKAHEAD + 9)
        ),
        format!(
            "- parent\n\t- first\n\t{}- late\n",
            " ".repeat(BLOCK_LOOKAHEAD + 9)
        ),
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
            let text = &source[..at + ch.len_utf8()];
            prefix.finish();
            assert_eq!(support::snapshot(&prefix), support::snapshot(&parse(text)));
            let mut eof = Document::new();
            eof.append(&source[..at]).unwrap();
            eof.append(&source[at..at + ch.len_utf8()]).unwrap();
            eof.finish();
            assert_eq!(support::snapshot(&eof), support::snapshot(&prefix));
        }
        doc.finish();
        assert_eq!(support::snapshot(&doc), expected);
    }
    let mut doc = Document::new();
    doc.append("- parent\n\t- first\n").unwrap();
    let mut reader = doc
        .display_cursor(Span::new(0, doc.source().len()))
        .unwrap();
    doc.append("\t+ second\n").unwrap();
    assert_eq!(
        reader.next(&doc, &mut Budget::new(WorkLimit::unlimited())),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(doc.blocks()[0].container, Some(ContainerId(1)));
    assert_eq!(doc.blocks()[1].container, Some(ContainerId(3)));
    assert_eq!(doc.blocks()[2].container, Some(ContainerId(5)));
}

#[test]
fn changed_list_prefix_preserves_source_slices_tiles_and_shared_budget() {
    let source = format!(
        "{}\n\n- foo\n    - first\n\t+ 中🙂\tend\r\n",
        "x".repeat(236)
    );
    assert_eq!(source.find('\t'), Some(256));
    let doc = parse(&source);
    let expected = format!("{}\n\nfoo\nfirst\n中🙂\tend\n", "x".repeat(236));
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
    let mut peer = doc.display_cursor(Span::new(0, source.len())).unwrap();
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
        let peer_pos = peer.source_position();
        assert_eq!(
            peer.next(&doc, &mut budget).unwrap(),
            DisplayStep::BudgetExhausted
        );
        assert_eq!(peer.source_position(), peer_pos);
        assert_eq!(budget.used().queries, 1);
        assert_eq!(run.source.start, old);
        assert!(reader.source_position() > old);
        if !run.style.contains(Style::HIDDEN) {
            output.push_str(run.text);
        }
    }
    assert_eq!(output, expected);
}

#[test]
fn deep_million_byte_body_and_many_list_changes_do_not_rescan_history() {
    let mut doc = Document::new();
    doc.append(&format!(
        "{}parent\n{}- first\n{}+ {}",
        "- ".repeat(499),
        "\t".repeat(250),
        "\t".repeat(250),
        "x".repeat(1_000_000)
    ))
    .unwrap();
    let last = doc.blocks().last().unwrap();
    assert_eq!(doc.container(last.container.unwrap()).unwrap().depth, 1000);
    assert_eq!(doc.container_count(), 1002);
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
    doc.append("- parent\n\t- first\n").unwrap();
    for i in 0..4096 {
        doc.append(if i % 2 == 0 {
            "\t+ sibling\n"
        } else {
            "\t- sibling\n"
        })
        .unwrap();
    }
    doc.finish();
    assert_eq!(doc.blocks().len(), 4098);
    assert_eq!(doc.container_count(), 8196);
    assert!(doc.work().classified_bytes < 4096 * 60);
    assert!(doc.work().container_bytes < 4096 * 150);
    assert!(doc.work().container_steps < 4096 * 25);
    assert_eq!(
        visible(&doc),
        format!("parent\nfirst\n{}", "sibling\n".repeat(4096))
    );
}

#[test]
fn replacement_seals_only_old_list_subtree_and_preserves_ancestor_identity() {
    for (first, next, kind, number) in [
        ("-", "+", ListMarker::Bullet(b'+'), None),
        ("+", "*", ListMarker::Bullet(b'*'), None),
        ("-", "23.", ListMarker::Ordered(b'.'), Some(23)),
        ("1.", "2)", ListMarker::Ordered(b')'), Some(2)),
        ("1)", "-", ListMarker::Bullet(b'-'), None),
    ] {
        for descendant in ["", "       - child\n", "       > child\n"] {
            let opening = format!("- parent\n\t{first} first [label](/url)\n{descendant}");
            let mut doc = Document::new();
            doc.append(&opening).unwrap();
            let count = doc.container_count();
            let blocks = doc.blocks().to_vec();
            let link = *doc.link_info(LinkId(0)).unwrap();
            let update = doc.append(&format!("\t{next} [x] 中🙂\n")).unwrap();
            assert_eq!(update.new_containers, count..count + 2);
            assert!(update.loosened_lists.is_empty());
            assert_eq!(update.closed_containers.len(), count - 2);
            for id in 2..count {
                assert!(update.closed_containers.contains(&ContainerId(id)));
                let node = doc.container(ContainerId(id)).unwrap();
                assert_eq!(node.source.end, opening.len());
                assert_eq!(node.state, SyntaxState::Sealed);
            }
            for id in 0..2 {
                let ancestor = doc.container(ContainerId(id)).unwrap();
                assert_eq!(ancestor.state, SyntaxState::Growing);
                assert_eq!(ancestor.source.end, doc.source().len());
            }
            let list = doc.container(ContainerId(count)).unwrap();
            assert_eq!(list.kind, ContainerKind::List { marker: kind });
            assert_eq!(list.parent, Some(ContainerId(1)));
            assert_eq!(list.depth, 3);
            assert_eq!(list.list_tightness, Some(ListTightness::Tight));
            let item = doc.container(ContainerId(count + 1)).unwrap();
            assert_eq!(item.parent, Some(ContainerId(count)));
            assert_eq!(item.depth, 4);
            let ContainerKind::Item {
                number: actual,
                task: Some(task),
            } = item.kind
            else {
                panic!("missing new task item")
            };
            assert_eq!(actual, number);
            assert!(task.checked);
            assert_eq!(doc.text(task.source), Some("[x]"));
            assert_eq!(&doc.blocks()[..blocks.len()], blocks);
            assert_eq!(doc.link_info(LinkId(0)), Some(&link));
            assert_eq!(doc.link_destination(LinkId(0)), Some("/url"));
            doc.finish();
            assert!(visible(&doc).ends_with("中🙂\n"));
        }
    }
}

#[test]
fn blank_boundaries_priority_and_failed_prefixes_keep_their_dialect() {
    for (first, next) in [("-", "+"), ("+", "*"), ("1.", "2)"), ("2)", "-")] {
        for descendant in ["", "       - child\n", "       > child\n"] {
            for blank in ["", "\n", "\n\n", "      \n"] {
                let opening = format!("- parent\n    {first} first\n{descendant}{blank}");
                for body in ["next", "[x] task", "# title", "- nested", "> quote", "a|b"] {
                    assert_model(
                        &format!("{opening}\t{next} {body}\n"),
                        &format!("{opening}    {next} {body}\n"),
                    );
                }
            }
        }
    }
    // Empty different markers retain lazy prose with partial-tab spaces.
    // Malformed markers are lazy prose; thematic breaks and quotes keep fallback.
    for (tail, expected) in [
        ("\t+\n", "parent\nfirst\n  +\n"),
        ("\t+ \n", "parent\nfirst\n  +\n"),
        ("\t* * *\n", "parent\nfirst\n* * *\n"),
        ("\t+closed\n", "parent\nfirst\n  +closed\n"),
        ("\t> quote\n", "parent\nfirst\nquote\n"),
    ] {
        assert_eq!(
            visible(&parse(&format!("- parent\n\t- first\n{tail}"))),
            expected
        );
    }
    for marker in ["```", "$$"] {
        let source = format!("- parent\n\t- {marker}\n\t  + literal\n      {marker}\n");
        assert_eq!(visible(&parse(&source)), "parent\n+ literal\n");
    }
    for source in [
        "> - parent\n>\t- first\n\t+ outside\n",
        "- parent\n  > quote\n\t+ outside\n",
        "- parent\n\t- first\n + root\n",
    ] {
        let doc = parse(source);
        let last = doc.blocks().last().unwrap();
        assert_ne!(last.container, Some(ContainerId(3)));
        assert_eq!(
            doc.container(ContainerId(2)).unwrap().state,
            SyntaxState::Sealed
        );
    }
}
