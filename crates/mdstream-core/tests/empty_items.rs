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

fn check_reads(doc: &Document) {
    let span = Span::new(0, doc.source().len());
    let raw: String = doc.runs_in(span).unwrap().map(|r| r.text).collect();
    assert_eq!(raw, doc.source());
    let mut cursor = doc.display_cursor(span).unwrap();
    let mut shown = String::new();
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
                assert_eq!(run.replacement, None);
                assert_eq!(run.text, &doc.source()[run.source.range()]);
                if !run.style.contains(Style::HIDDEN) {
                    shown.push_str(run.text);
                }
            }
            DisplayStep::Complete => break,
            DisplayStep::BudgetExhausted => panic!("six bytes must make progress"),
        }
        assert!(budget.used().bytes <= 6);
        assert!(budget.used().queries <= 1 && budget.used().objects <= 1);
    }
    assert_eq!(shown, visible(doc));
    let sliced: String = doc
        .source()
        .char_indices()
        .flat_map(|(at, ch)| {
            doc.display_runs_in(Span::new(at, at + ch.len_utf8()))
                .unwrap()
        })
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    assert_eq!(sliced, shown);
}

#[test]
fn empty_items_publish_blank_leaves_with_container_metadata() {
    let source = "-\n- \n-\t\n-   \r\n";
    let doc = parse(source);
    assert_eq!(doc.container_count(), 5);
    assert_eq!(doc.blocks().len(), 4);
    let mut start = 0;
    for (index, (block, line)) in doc
        .blocks()
        .iter()
        .zip(source.split_inclusive('\n'))
        .enumerate()
    {
        assert_eq!(block.kind, BlockKind::Blank);
        assert_eq!(block.source, Span::new(start, start + line.len()));
        assert_eq!(block.state, SyntaxState::Sealed);
        let scope = ContainerId(index + 1);
        assert_eq!(block.container, Some(scope));
        let item = doc.container(scope).unwrap();
        assert_eq!(
            item.kind,
            ContainerKind::Item {
                number: None,
                task: None
            }
        );
        assert_eq!(item.parent, Some(ContainerId(0)));
        assert_eq!(item.source, block.source);
        start += line.len();
    }
    assert_eq!(
        doc.container(ContainerId(0)).unwrap().list_tightness,
        Some(ListTightness::Tight)
    );
    assert_eq!(visible(&doc), "\n\n\n\n");
    check_reads(&doc);
}

#[test]
fn empty_item_matrix_preserves_markers_numbers_scopes_and_line_endings() {
    let mut cases = 0;
    for outer in ["", "> ", "- ", "> - > "] {
        for indent in 0..=3 {
            for (marker, number) in [
                ("-", None),
                ("+", None),
                ("*", None),
                ("0.", Some(0)),
                ("009)", Some(9)),
                ("123456789.", Some(123456789)),
            ] {
                for gap in ["", " ", "  ", "\t", " \t", "   "] {
                    for end in ["\n", "\r\n", ""] {
                        let source = format!("{outer}{}{marker}{gap}{end}", " ".repeat(indent));
                        let doc = parse(&source);
                        assert_eq!(doc.blocks().len(), 1, "{source:?}");
                        let block = &doc.blocks()[0];
                        assert_eq!(block.kind, BlockKind::Blank, "{source:?}");
                        assert_eq!(block.source, Span::new(0, source.len()));
                        let item = doc.container(block.container.unwrap()).unwrap();
                        assert_eq!(item.kind, ContainerKind::Item { number, task: None });
                        let list = doc.container(item.parent.unwrap()).unwrap();
                        let marker = if number.is_some() {
                            ListMarker::Ordered(*marker.as_bytes().last().unwrap())
                        } else {
                            ListMarker::Bullet(marker.as_bytes()[0])
                        };
                        assert_eq!(list.kind, ContainerKind::List { marker });
                        assert_eq!(list.list_tightness, Some(ListTightness::Tight));
                        check_reads(&doc);
                        cases += 1;
                    }
                }
            }
        }
    }
    assert_eq!(cases, 1728);
}

#[test]
fn empty_items_preserve_spacing_and_end_inline_sequences() {
    for (source, tightness) in [
        ("-\n  first\n", ListTightness::Tight),
        ("-\n\n  first\n", ListTightness::Tight),
        ("-\n- second\n", ListTightness::Tight),
        ("-\n\n- second\n", ListTightness::Loose),
        ("- a\n\n-\n", ListTightness::Loose),
    ] {
        let doc = parse(source);
        assert!(doc.blocks().iter().any(|b| b.kind == BlockKind::Blank));
        assert_eq!(
            doc.container(ContainerId(0)).unwrap().list_tightness,
            Some(tightness),
            "{source:?}"
        );
        check_reads(&doc);
    }
    for source in [
        "- *a\n-\n  b*\n",
        "- `a\n-\n  b`\n",
        "- [a\n-\n  b](url)\n",
        "- $a\n-\n  b$\n",
    ] {
        let doc = parse(source);
        assert_eq!(doc.blocks()[1].kind, BlockKind::Blank);
        assert!(doc.links().is_empty());
        assert!(doc.maths().is_empty());
        for run in doc.display_runs_in(Span::new(0, source.len())).unwrap() {
            for style in [
                Style::EMPHASIS,
                Style::CODE,
                Style::MATH,
                Style::SOFT_BREAK,
                Style::HARD_BREAK,
            ] {
                assert!(!run.style.contains(style), "{source:?}: {run:?}");
            }
        }
    }
    let doc = parse("-\n  ---\n");
    assert!(doc.headings().entries().is_empty());
    assert_eq!(doc.blocks()[1].kind, BlockKind::ThematicBreak);
    let doc = parse("- a | b\n-\n  --|--\n");
    assert!(doc.tables().is_empty());
    assert_eq!(doc.blocks()[1].kind, BlockKind::Blank);
}

#[test]
fn task_items_nonempty_leaves_code_and_window_boundaries_keep_their_dialect() {
    for source in ["- [ ]\n", "- [x] \r\n", "> 1. [X]\t"] {
        let doc = parse(source);
        let block = &doc.blocks()[0];
        let BlockKind::ListItem {
            task: Some(task), ..
        } = block.kind
        else {
            panic!("{source:?}")
        };
        let item = doc.container(block.container.unwrap()).unwrap();
        assert!(matches!(item.kind, ContainerKind::Item { task: Some(t), .. } if t == task));
        assert_eq!(
            doc.text(task.source),
            Some(if task.checked {
                if source.contains('X') { "[X]" } else { "[x]" }
            } else {
                "[ ]"
            })
        );
        check_reads(&doc);
    }
    for source in ["- text\n", "1. text\n", "- [invalid]\n"] {
        assert!(matches!(
            parse(source).blocks()[0].kind,
            BlockKind::ListItem { .. }
        ));
    }
    for source in ["- ```\n  -\n  ```\n", "- $$\n  -\n  $$\n"] {
        let doc = parse(source);
        assert_eq!(doc.blocks().len(), 1);
        assert!(!matches!(doc.blocks()[0].kind, BlockKind::Blank));
    }
    for end in ["", "\n"] {
        let short = parse(&format!("-{}{end}", " ".repeat(BLOCK_LOOKAHEAD - 2)));
        assert_eq!(short.blocks()[0].kind, BlockKind::Blank);
        let long = parse(&format!("-{}{end}", " ".repeat(BLOCK_LOOKAHEAD)));
        assert!(!matches!(long.blocks()[0].kind, BlockKind::Blank));
    }
}

#[test]
fn empty_items_are_stable_at_every_split_prefix_and_eof() {
    for source in [
        "-\n- \r\n-\t",
        "> 009)\n>    中🙂\n",
        "- *中\n-\n  文*\n",
        "-\n\n- b\n",
        "- [x]\n- [ ]\n-\n",
        "- a | b\n-\n  --|--\n",
    ] {
        let whole = parse(source);
        for split in (0..=source.len()).filter(|&at| source.is_char_boundary(at)) {
            let mut doc = Document::new();
            doc.append(&source[..split]).unwrap();
            doc.append(&source[split..]).unwrap();
            doc.finish();
            assert_eq!(
                support::snapshot(&doc),
                support::snapshot(&whole),
                "{source:?} split {split}"
            );
        }
        let mut doc = Document::new();
        for (offset, ch) in source.char_indices() {
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            let prefix = &source[..offset + ch.len_utf8()];
            let mut bulk = Document::new();
            bulk.append(prefix).unwrap();
            assert_eq!(support::snapshot(&doc), support::snapshot(&bulk));
            assert_eq!(doc.pending(), bulk.pending());
            let mut eof = Document::new();
            for ch in prefix.chars() {
                eof.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            }
            eof.finish();
            assert_eq!(support::snapshot(&eof), support::snapshot(&parse(prefix)));
        }
        check_reads(&whole);
    }
}

#[test]
fn empty_items_do_not_rescan_historical_text_or_ancestors() {
    let prefix = ">".repeat(980);
    let mut doc = Document::new();
    doc.append(&format!("{prefix} - {}\n", "x".repeat(1_000_000)))
        .unwrap();
    let before = doc.work();
    for _ in 0..100 {
        doc.append(&format!("{prefix} -\n")).unwrap();
    }
    assert_eq!(doc.container_count(), 1082);
    assert!(doc.blocks()[1..].iter().all(|b| b.kind == BlockKind::Blank));
    assert!(doc.work().container_steps - before.container_steps < 200_000);
    assert!(doc.work().container_bytes - before.container_bytes < 800_000);
    assert!(doc.work().classified_bytes - before.classified_bytes < 400_000);
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    doc.append(&format!("{prefix} - {}", "x".repeat(1_000_000)))
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
