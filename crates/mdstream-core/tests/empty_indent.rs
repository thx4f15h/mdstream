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
fn empty_marker_continuations_use_one_separator_column() {
    let mut cases = 0;
    for (outer, continuation) in [("", ""), ("> ", "> "), ("- ", "  "), ("> - > ", ">   > ")] {
        for indent in 0..=3 {
            for (marker, number) in [
                ("-", None),
                ("+", None),
                ("*", None),
                ("0.", Some(0)),
                ("009)", Some(9)),
                ("123456789.", Some(123456789)),
            ] {
                for gap in ["", " ", "  ", "    ", "     ", "\t", " \t", "\t  \t"] {
                    for end in ["\n", "\r\n"] {
                        let width = indent + marker.len() + 1;
                        let head = format!("{outer}{}{marker}{gap}{end}", " ".repeat(indent));
                        let source = format!("{head}{continuation}{}中🙂\n", " ".repeat(width));
                        let doc = parse(&source);
                        assert_eq!(doc.blocks().len(), 2, "{source:?}");
                        assert_eq!(doc.blocks()[0].kind, BlockKind::Blank);
                        let scope = doc.blocks()[0].container;
                        assert_eq!(doc.blocks()[1].container, scope, "{source:?}");
                        assert_eq!(
                            visible(&doc, doc.blocks()[1].source),
                            "中🙂\n",
                            "{source:?}"
                        );
                        let item = doc.container(scope.unwrap()).unwrap();
                        assert_eq!(item.kind, ContainerKind::Item { number, task: None });
                        assert_eq!(
                            doc.container(item.parent.unwrap()).unwrap().list_tightness,
                            Some(ListTightness::Tight)
                        );
                        check_reads(&doc);
                        let short = parse(&format!(
                            "{head}{continuation}{}text\n",
                            " ".repeat(width - 1)
                        ));
                        assert_ne!(
                            short.blocks()[1].container,
                            short.blocks()[0].container,
                            "short: {source:?}"
                        );
                        cases += 1;
                    }
                }
            }
        }
    }
    assert_eq!(cases, 1536);
}

#[test]
fn normalized_empty_items_keep_code_and_partial_tab_mapping() {
    for gap in ["", "   ", "\t", " \t\t"] {
        for marker in ["-", "1.", "009)"] {
            let width = marker.len() + 1;
            let source = format!(
                "{marker}{gap}\n{}```\n{}raw\n{}```\n",
                " ".repeat(width),
                " ".repeat(width),
                " ".repeat(width)
            );
            let doc = parse(&source);
            assert_eq!(doc.blocks().len(), 2);
            assert_eq!(doc.blocks()[0].container, doc.blocks()[1].container);
            assert!(matches!(doc.blocks()[1].kind, BlockKind::Code { .. }));
            assert_eq!(visible(&doc, doc.blocks()[1].source), "raw\n");
            check_reads(&doc);
            let source = format!("{marker}{gap}\n{}raw\n", " ".repeat(width + 4));
            let doc = parse(&source);
            assert_eq!(doc.blocks()[0].container, doc.blocks()[1].container);
            assert!(matches!(doc.blocks()[1].kind, BlockKind::Code { .. }));
            assert_eq!(visible(&doc, doc.blocks()[1].source), "raw\n");
            check_reads(&doc);
        }
    }
    for source in [
        "-\n\t中🙂\n",
        "-   \n\t中🙂\n",
        "> -\n>\t中🙂\n",
        "- # parent\n\t-\t\n\t  中🙂\n",
        ">\t-\t\n>\t  中🙂\n",
    ] {
        let doc = parse(source);
        let count = doc.blocks().len();
        assert_eq!(doc.blocks()[count - 2].kind, BlockKind::Blank, "{source:?}");
        assert_eq!(
            doc.blocks()[count - 1].container,
            doc.blocks()[count - 2].container,
            "{source:?}"
        );
        check_reads(&doc);
    }
    let doc = parse("-\n\t中🙂\n");
    assert_eq!(visible(&doc, doc.blocks()[1].source), "  中🙂\n");
    let replacements: Vec<_> = doc
        .display_runs_in(doc.blocks()[1].source)
        .unwrap()
        .filter(|r| r.replacement.is_some())
        .collect();
    assert_eq!(replacements.len(), 1);
    assert_eq!(replacements[0].source, Span::new(2, 3));
    assert_eq!(replacements[0].text, "  ");
}

#[test]
fn empty_item_siblings_notifications_and_unextended_boundaries() {
    let mut doc = Document::new();
    doc.append("- first\n").unwrap();
    let update = doc.append("-   \n").unwrap();
    assert_eq!(update.new_containers, 2..3);
    assert_eq!(update.closed_containers, vec![ContainerId(1)]);
    let update = doc.append("  next\n").unwrap();
    assert!(update.new_containers.is_empty());
    assert!(update.closed_containers.is_empty());
    assert!(update.loosened_lists.is_empty());
    assert_eq!(doc.blocks()[2].container, Some(ContainerId(2)));
    doc.append("\n").unwrap();
    let update = doc.append("- [x] task\n").unwrap();
    assert_eq!(update.loosened_lists, vec![ContainerId(0)]);
    assert!(matches!(
        doc.blocks()[4].kind,
        BlockKind::ListItem { task: Some(_), .. }
    ));
    doc.finish();
    check_reads(&doc);
    // A subsequent blank prevents the empty item from acquiring its first content.
    let doc = parse("-\n\n  next\n");
    assert_eq!(doc.blocks()[2].container, None);
    assert_eq!(visible(&doc, doc.blocks()[2].source), "next\n");
    // Nonempty marker spacing, task content, and paragraph interruption keep their dialect.
    for source in ["-   first\n    next\n", "-   [ ]\n    next\n"] {
        let doc = parse(source);
        assert_eq!(doc.blocks()[0].container, doc.blocks()[1].container);
        assert_eq!(visible(&doc, doc.blocks()[1].source), "next\n");
        check_reads(&doc);
    }
    assert_eq!(parse("text\n*   \n").container_count(), 0);
}

#[test]
fn empty_indent_window_and_eof_are_bounded() {
    for ending in ["", "\n", "\r\n"] {
        let source = format!("-{}{ending}", " ".repeat(BLOCK_LOOKAHEAD - 3));
        let doc = parse(&source);
        assert_eq!(doc.blocks()[0].kind, BlockKind::Blank);
        check_reads(&doc);
    }
    let short = format!("-{}\n  text\n", " ".repeat(BLOCK_LOOKAHEAD - 2));
    let doc = parse(&short);
    assert_eq!(doc.blocks()[0].kind, BlockKind::Blank);
    assert_eq!(doc.blocks()[0].container, doc.blocks()[1].container);
    assert_eq!(visible(&doc, doc.blocks()[1].source), "text\n");
    for tail in ["", "text"] {
        let source = format!("-{}{tail}\n  next\n", " ".repeat(BLOCK_LOOKAHEAD));
        let doc = parse(&source);
        assert_eq!(
            matches!(doc.blocks()[0].kind, BlockKind::Blank),
            tail.is_empty()
        );
        for at in [1, BLOCK_LOOKAHEAD - 1, BLOCK_LOOKAHEAD, BLOCK_LOOKAHEAD + 1] {
            let mut split = Document::new();
            split.append(&source[..at]).unwrap();
            split.append(&source[at..]).unwrap();
            split.finish();
            assert_eq!(support::snapshot(&split), support::snapshot(&doc));
        }
        check_reads(&doc);
    }
}

#[test]
fn empty_indent_all_splits_prefixes_and_eof_are_consistent() {
    for source in [
        "-\n  中🙂\n-   \n  next",
        "> 009)\t \r\n>      中🙂\n",
        "-\n\t中🙂\n",
        "- # parent\n\t-\t\n\t  中🙂\n",
        ">\t-\t\n>\t  中🙂\n",
        "-\n  ```\n  raw\n  ```\n-\n      code\n",
        "-\n\n  next\n- [x] task\n",
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
            check_reads(&doc);
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
fn empty_indent_does_not_rescan_ancestors_or_old_text() {
    let prefix = ">".repeat(980);
    let mut doc = Document::new();
    doc.append(&format!("{prefix} - {}\n", "x".repeat(1_000_000)))
        .unwrap();
    for _ in 0..100 {
        let before = doc.work();
        doc.append(&format!("{prefix} -\t\n{prefix}   body\n"))
            .unwrap();
        let after = doc.work();
        assert!(after.container_steps - before.container_steps < 8_000);
        assert!(after.container_bytes - before.container_bytes < 30_000);
        assert!(after.classified_bytes - before.classified_bytes < 8_000);
        assert!(after.inline_bytes - before.inline_bytes < 20);
        let count = doc.blocks().len();
        assert_eq!(
            doc.blocks()[count - 1].container,
            doc.blocks()[count - 2].container
        );
        assert_eq!(visible(&doc, doc.blocks()[count - 1].source), "body\n");
    }
    assert_eq!(doc.container_count(), 1082);
    doc.append(&format!("{prefix}   {}", "x".repeat(1_000_000)))
        .unwrap();
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("x").unwrap();
    }
    assert_eq!(doc.work().container_steps, before.container_steps);
    assert_eq!(doc.work().container_bytes, before.container_bytes);
    assert_eq!(doc.work().classified_bytes, before.classified_bytes);
    assert_eq!(doc.work().inline_bytes - before.inline_bytes, 1000);
    doc.finish();
}
