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

fn check_reads(doc: &Document) {
    let span = Span::new(0, doc.source().len());
    assert_eq!(
        doc.runs_in(span)
            .unwrap()
            .map(|r| r.text)
            .collect::<String>(),
        doc.source()
    );
    let mut cursor = doc.display_cursor(span).unwrap();
    let mut output = String::new();
    loop {
        let at = cursor.source_position();
        if at < span.end {
            assert_eq!(
                cursor
                    .next(doc, &mut Budget::new(WorkLimit::default()))
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
        match cursor.next(doc, &mut budget).unwrap() {
            DisplayStep::Run(run) => {
                assert_eq!(run.source.start, at);
                assert!(doc.source().is_char_boundary(run.source.end));
                if run.replacement.is_none() {
                    assert_eq!(run.text, &doc.source()[run.source.range()]);
                }
                if !run.style.contains(Style::HIDDEN) {
                    output.push_str(run.text);
                }
            }
            DisplayStep::Complete => break,
            DisplayStep::BudgetExhausted => panic!("six bytes must progress"),
        }
        assert!(budget.used().bytes <= 6);
        assert!(budget.used().queries <= 1 && budget.used().objects <= 1);
    }
    assert_eq!(output, visible(doc, span));
}

#[test]
fn changing_indent_retains_list_and_updates_each_item_width() {
    let source = "- a\n - b\n  - c\n   - d\n  - e\n - f\n- g\n";
    let doc = parse(source);
    assert_eq!(doc.container_count(), 8);
    for (i, indent) in [0, 1, 2, 3, 2, 1, 0].into_iter().enumerate() {
        assert_eq!(doc.blocks()[i].container, Some(ContainerId(i + 1)));
        assert!(
            matches!(doc.blocks()[i].kind, BlockKind::ListItem { indent: n, .. } if n == indent)
        );
        assert_eq!(
            doc.container(ContainerId(i + 1)).unwrap().parent,
            Some(ContainerId(0))
        );
    }
    assert_eq!(
        doc.container(ContainerId(0)).unwrap().list_tightness,
        Some(ListTightness::Tight)
    );
    let doc = parse("- a\n - b\n   continuation\n  - c\n    continuation\n");
    assert_eq!(doc.container_count(), 4);
    assert_eq!(doc.blocks()[2].container, Some(ContainerId(2)));
    assert_eq!(doc.blocks()[4].container, Some(ContainerId(3)));
    check_reads(&doc);
}

#[test]
fn independent_indent_matrix_distinguishes_siblings_from_children() {
    let mut cases = 0;
    for outer in ["", "> ", "> > ", "- parent\n  "] {
        let continuation = if outer.starts_with('-') { "  " } else { outer };
        let base = usize::from(outer.starts_with('-')) * 2 + outer.matches('>').count();
        for (first, second) in [
            ("-", "-"),
            ("+", "+"),
            ("*", "*"),
            ("1.", "23."),
            ("1)", "2)"),
        ] {
            for initial in 0..=3 {
                for next in 0..=3 {
                    for gap in ["", "\n"] {
                        for eol in ["\n", "\r\n", ""] {
                            let blank = if gap.is_empty() {
                                String::new()
                            } else {
                                format!("{continuation}\n")
                            };
                            let source = format!(
                                "{outer}{}{first} first\n{blank}{continuation}{}{second} 中🙂{eol}",
                                " ".repeat(initial),
                                " ".repeat(next)
                            );
                            let doc = parse(&source);
                            let sibling = next < initial + first.len() + 1;
                            // A nested non-1 marker remains prose without a separator.
                            // Same-list siblings still accept arbitrary numbers.
                            let prose =
                                !sibling && gap.is_empty() && second.as_bytes()[0].is_ascii_digit();
                            let list = ContainerId(base);
                            assert_eq!(
                                doc.container_count(),
                                base + if prose {
                                    2
                                } else if sibling {
                                    3
                                } else {
                                    4
                                },
                                "{source:?}"
                            );
                            let last = doc
                                .container(ContainerId(doc.container_count() - 1))
                                .unwrap();
                            assert_eq!(
                                last.parent,
                                Some(if sibling || prose {
                                    list
                                } else {
                                    ContainerId(base + 2)
                                }),
                                "{source:?}"
                            );
                            assert_eq!(
                                doc.container(list).unwrap().list_tightness,
                                Some(if gap.is_empty() {
                                    ListTightness::Tight
                                } else {
                                    ListTightness::Loose
                                }),
                                "{source:?}"
                            );
                            if prose {
                                assert_eq!(
                                    doc.blocks().last().unwrap().container,
                                    Some(ContainerId(base + 1))
                                );
                                assert!(
                                    visible(&doc, Span::new(0, doc.source().len()))
                                        .contains(second)
                                );
                            }
                            check_reads(&doc);
                            cases += 1;
                        }
                    }
                }
            }
        }
    }
    assert_eq!(cases, 1920);
}

#[test]
fn partial_tabs_markers_rules_and_four_column_boundaries() {
    for (source, count, parent) in [
        ("- parent\n   - first\n\t- second\n", 5, 2),
        ("- parent\n     - first\n\t- second\n", 5, 2),
        ("> - first\n>\t- second\n", 5, 3),
        (">  - first\n>\t- second\n", 4, 1),
        ("- first\n  - child\n", 4, 2),
        (" - first\n+ second\n", 4, 2),
        ("  1. first\n 2) second\n", 4, 2),
    ] {
        let doc = parse(source);
        assert_eq!(doc.container_count(), count, "{source:?}");
        assert_eq!(
            doc.container(ContainerId(count - 1)).unwrap().parent,
            Some(ContainerId(parent)),
            "{source:?}"
        );
        check_reads(&doc);
    }
    let doc = parse(" - first\n  * * *\n");
    assert!(matches!(
        doc.blocks().last().unwrap().kind,
        BlockKind::ThematicBreak
    ));
    let doc = parse("1. a\n\n  2. b\n\n    3. c\n");
    assert_eq!(doc.container_count(), 3);
    assert!(matches!(
        doc.blocks().last().unwrap().kind,
        BlockKind::Code { .. }
    ));
    assert_eq!(doc.blocks().last().unwrap().container, None);
    let doc = parse("- a\n - b\n  - c\n   - d\n    - e\n");
    assert_eq!(doc.container_count(), 5);
    assert_eq!(
        visible(&doc, Span::new(0, doc.source().len())),
        "a\nb\nc\nd\n    - e\n"
    );
}

#[test]
fn all_splits_prefixes_eof_and_budget_keep_identity() {
    for source in [
        "- a\n - b\n  - c\n   - d\n  - e\n - f\n- g\n",
        "1. a\n\n  2. b\n\n   3. c\n",
        "1. a\n\n  2. b\n\n    3. c\n",
        "- parent\n   - first\n\t- [x] 中🙂\r\n",
        "> - first\n>\t- second\n",
        " - first\n  - ```\n    raw\n    ```\n - next",
    ] {
        let whole = parse(source);
        for at in (0..=source.len()).filter(|&at| source.is_char_boundary(at)) {
            let mut doc = Document::new();
            doc.append(&source[..at]).unwrap();
            doc.append(&source[at..]).unwrap();
            doc.finish();
            assert_eq!(
                support::snapshot(&doc),
                support::snapshot(&whole),
                "{source:?} at {at}"
            );
        }
        let mut streamed = Document::new();
        for (at, ch) in source.char_indices() {
            streamed.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            let prefix = &source[..at + ch.len_utf8()];
            let mut bulk = Document::new();
            bulk.append(prefix).unwrap();
            assert_eq!(support::snapshot(&streamed), support::snapshot(&bulk));
            assert_eq!(streamed.pending(), bulk.pending());
            let mut eof = Document::new();
            for ch in prefix.chars() {
                eof.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            }
            eof.finish();
            assert_eq!(support::snapshot(&eof), support::snapshot(&parse(prefix)));
            check_reads(&eof);
        }
    }
}

#[test]
fn sibling_closes_descendants_and_notifies_looseness_once() {
    let mut doc = Document::new();
    let head = " - first\n   - child\n\n";
    doc.append(head).unwrap();
    let update = doc.append("  - [x] second\n").unwrap();
    assert_eq!(update.new_containers, 4..5);
    assert_eq!(update.loosened_lists, vec![ContainerId(0)]);
    assert!(!update.closed_containers.contains(&ContainerId(0)));
    for i in 1..4 {
        assert!(update.closed_containers.contains(&ContainerId(i)));
        assert_eq!(
            doc.container(ContainerId(i)).unwrap().source.end,
            head.len()
        );
    }
    assert!(matches!(
        doc.container(ContainerId(4)).unwrap().kind,
        ContainerKind::Item { task: Some(_), .. }
    ));
    assert!(doc.append(" - third\n").unwrap().loosened_lists.is_empty());
    assert_eq!(
        doc.container(ContainerId(0)).unwrap().list_tightness,
        Some(ListTightness::Loose)
    );
    doc.finish();
    check_reads(&doc);
}

#[test]
fn deep_siblings_and_long_append_do_not_revisit_history() {
    let depth = 480;
    let mut doc = Document::new();
    doc.append(&format!("{}seed\n", "- ".repeat(depth)))
        .unwrap();
    let prefix = " ".repeat((depth - 1) * 2);
    for i in 0..1000 {
        let indent = [1, 2, 3, 2, 1, 0][i % 6];
        let line = format!("{prefix}{}- next\n", " ".repeat(indent));
        let before = doc.work();
        doc.append(&line).unwrap();
        let after = doc.work();
        assert!(after.container_steps - before.container_steps < 4 * depth as u64);
        assert!(after.container_bytes - before.container_bytes < 12 * line.len() as u64);
    }
    assert_eq!(doc.container_count(), depth * 2 + 1000);
    // The last sibling has two columns of marker indent and a four-column body indent.
    doc.append(&format!("{prefix}    {}", "x".repeat(1_000_000)))
        .unwrap();
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("x").unwrap();
    }
    assert_eq!(doc.work().container_steps, before.container_steps);
    assert_eq!(doc.work().container_bytes, before.container_bytes);
    assert_eq!(doc.work().list_steps, before.list_steps);
    doc.finish();
    assert_eq!(doc.container_count(), depth * 2 + 1000);
}
