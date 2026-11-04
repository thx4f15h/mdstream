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
fn partial_tab_sibling_retains_list_and_closes_old_item_descendants() {
    for descendant in ["", "      - child\n", "      > child\n"] {
        let opener = format!("- parent\n\t- first\n{descendant}");
        let mut doc = Document::new();
        doc.append(&opener).unwrap();
        let old_count = doc.container_count();
        let update = doc.append("\t- [x] 中🙂\n").unwrap();
        assert_eq!(update.new_containers, old_count..old_count + 1);
        assert!(update.closed_containers.contains(&ContainerId(3)));
        assert!(!update.closed_containers.contains(&ContainerId(2)));
        let new = doc.container(ContainerId(old_count)).unwrap();
        assert_eq!(new.parent, Some(ContainerId(2)));
        assert_eq!(new.depth, 4);
        assert!(matches!(
            new.kind,
            ContainerKind::Item { task: Some(_), .. }
        ));
        assert_eq!(
            doc.container(ContainerId(3)).unwrap().source.end,
            opener.len()
        );
        for i in 4..old_count {
            assert_eq!(
                doc.container(ContainerId(i)).unwrap().state,
                SyntaxState::Sealed
            );
        }
        doc.finish();
        assert_eq!(
            visible(&doc),
            format!(
                "parent\nfirst\n{}中🙂\n",
                if descendant.is_empty() { "" } else { "child\n" }
            )
        );
    }
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
fn siblings_match_independent_expanded_column_model() {
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
                            ("-", "-"),
                            ("+", "+"),
                            ("*", "*"),
                            ("1.", "23."),
                            ("2)", "4)"),
                        ] {
                            for body in ["中🙂", "[x] task", "- nested", "> quote", "\t\tcode", ""]
                            {
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
fn blank_spacing_priorities_and_failed_probes_are_atomic() {
    for tail in [
        "\t-\n",
        "\t- - child\n",
        "\t- a|b\n      -|-\n",
        "\t- # heading\n",
    ] {
        assert_model(
            &format!("- parent\n\t- first\n{tail}"),
            &format!("- parent\n    - first\n{}", tail.replace('\t', "    ")),
        );
    }
    for descendant in ["", "      - child\n", "      > child\n"] {
        let prefix = format!("- parent\n\t- first\n{descendant}\n");
        let mut doc = Document::new();
        doc.append(&prefix).unwrap();
        let update = doc.append("\t- second\n").unwrap();
        assert_eq!(update.loosened_lists, vec![ContainerId(2)]);
        assert_eq!(
            doc.container(ContainerId(0)).unwrap().list_tightness,
            Some(ListTightness::Tight)
        );
        assert_eq!(
            doc.container(ContainerId(2)).unwrap().list_tightness,
            Some(ListTightness::Loose)
        );
    }
    // A shifted same-marker sibling is accepted below the old body indent.
    // Rule priority and fence/math literal payloads keep their prior dialect.
    for (tail, expected) in [
        ("\t- - -\n", "parent\nfirst\n\t- - -\n"),
        ("\t+ changed\n", "parent\nfirst\nchanged\n"),
        ("\t-closed\n", "parent\nfirst\n  -closed\n"),
        ("\t - shifted\n", "parent\nfirst\nshifted\n"),
        ("\t> quote\n", "parent\nfirst\n\t> quote\n"),
    ] {
        assert_eq!(
            visible(&parse(&format!("- parent\n\t- first\n{tail}"))),
            expected
        );
    }
    assert_model(
        "- parent\n\t- a|b\n\t- | -\n",
        "- parent\n    - a|b\n    - | -\n",
    );
    assert_eq!(
        visible(&parse("- parent\n\t- first\n\t- a|b\n\t  -|-\n")),
        "parent\nfirst\nab\n"
    );
    for marker in ["```", "$$"] {
        let source = format!("- parent\n\t- {marker}\n\t  - literal\n      {marker}\n");
        assert_eq!(visible(&parse(&source)), "parent\n- literal\n");
    }
}

#[test]
fn every_split_prefix_and_window_preserve_state_and_ids() {
    for source in [
        "- parent\n\t- first\n\t- [x] 中🙂\r\n\t- eof".to_owned(),
        "> - parent\n>   - first\n>\t- second\n".to_owned(),
        "- parent\n\t- first\n      > child\n\n\t- second\n".to_owned(),
        "- parent\n\t- first\n\t-\t\tcode\n".to_owned(),
        "- parent\n\t- first\n\t- - -\n\t+ changed\n".to_owned(),
        format!(
            "- parent\n\t- first\n\t- {}\r\n",
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
        }
        doc.finish();
        assert_eq!(support::snapshot(&doc), expected);
    }
    let mut doc = Document::new();
    doc.append("- parent\n\t- first\n").unwrap();
    let mut reader = doc
        .display_cursor(Span::new(0, doc.source().len()))
        .unwrap();
    doc.append("\t- second\n").unwrap();
    assert_eq!(
        reader.next(&doc, &mut Budget::new(WorkLimit::unlimited())),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(doc.blocks()[0].container, Some(ContainerId(1)));
    assert_eq!(doc.blocks()[1].container, Some(ContainerId(3)));
    assert_eq!(doc.blocks()[2].container, Some(ContainerId(4)));
}

#[test]
fn hidden_partial_prefix_preserves_source_slices_tiles_and_budget() {
    let source = format!(
        "{}\n\n- foo\n    - first\n\t- 中🙂\tend\r\n",
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
fn deep_million_byte_body_and_many_siblings_do_not_rescan_history() {
    let mut doc = Document::new();
    doc.append(&format!(
        "{}parent\n{}- first\n{}- {}",
        "- ".repeat(499),
        "\t".repeat(250),
        "\t".repeat(250),
        "x".repeat(1_000_000)
    ))
    .unwrap();
    let last = doc.blocks().last().unwrap();
    assert_eq!(doc.container(last.container.unwrap()).unwrap().depth, 1000);
    assert_eq!(doc.container_count(), 1001);
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
    for _ in 0..4096 {
        doc.append("\t- sibling\n").unwrap();
    }
    doc.finish();
    assert_eq!(doc.blocks().len(), 4098);
    assert_eq!(doc.container_count(), 4100);
    assert!(doc.work().classified_bytes < 4096 * 60);
    assert!(doc.work().container_bytes < 4096 * 150);
    assert!(doc.work().container_steps < 4096 * 25);
    assert_eq!(
        visible(&doc),
        format!("parent\nfirst\n{}", "sibling\n".repeat(4096))
    );
}
