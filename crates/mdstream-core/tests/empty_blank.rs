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
fn empty_items_stop_before_content_after_a_blank() {
    let mut cases = 0;
    for (outer, continuation) in [("", ""), ("> ", "> "), ("- ", "  "), ("> - > ", ">   > ")] {
        for indent in 0..=3 {
            for marker in ["-", "+", "*", "0.", "009)", "123456789."] {
                for gap in ["", " ", "    ", "\t"] {
                    for blank in ["", " ", "\t", "      "] {
                        let width = indent + marker.len() + 1;
                        let head = format!("{outer}{}{marker}{gap}\n", " ".repeat(indent));
                        let source = format!(
                            "{head}{continuation}{blank}\n{continuation}{}中🙂\n",
                            " ".repeat(width)
                        );
                        let doc = parse(&source);
                        let empty = doc.blocks()[0].container.unwrap();
                        let list = doc.container(empty).unwrap().parent.unwrap();
                        let parent = doc.container(list).unwrap().parent;
                        assert_eq!(doc.blocks()[0].kind, BlockKind::Blank);
                        assert_eq!(doc.blocks()[2].container, parent, "{source:?}");
                        assert_eq!(
                            doc.container(empty).unwrap().source.end,
                            head.len() + continuation.len() + blank.len() + 1
                        );
                        assert_eq!(
                            doc.container(list).unwrap().list_tightness,
                            Some(ListTightness::Tight)
                        );
                        check_reads(&doc);
                        cases += 1;
                    }
                }
            }
        }
    }
    assert_eq!(cases, 1536);
}

#[test]
fn empty_blank_siblings_keep_list_and_notify_once() {
    let mut doc = Document::new();
    doc.append("-\n").unwrap();
    let update = doc.append("\n").unwrap();
    assert!(update.closed_containers.is_empty());
    assert!(update.loosened_lists.is_empty());
    let update = doc.append("- next\n").unwrap();
    assert_eq!(update.closed_containers, [ContainerId(1)]);
    assert_eq!(update.loosened_lists, [ContainerId(0)]);
    assert_eq!(update.new_containers, 2..3);
    doc.append("-\n\n").unwrap();
    let update = doc.append("  outside\n").unwrap();
    assert_eq!(update.closed_containers, [ContainerId(3), ContainerId(0)]);
    assert!(update.loosened_lists.is_empty());
    assert_eq!(doc.blocks().last().unwrap().container, None);
    doc.finish();
    check_reads(&doc);
}

#[test]
fn empty_blank_partial_tabs_children_tasks_and_window() {
    for source in [
        "-\n\n\tcode\n",
        "> -\n>\n>\ttext\n",
        "- # parent\n\t-\n\n\t  text\n",
        ">\t-\n>\n>\t  text\n",
    ] {
        let doc = parse(source);
        let count = doc.blocks().len();
        assert_ne!(
            doc.blocks()[count - 1].container,
            doc.blocks()[count - 3].container,
            "{source:?}"
        );
        check_reads(&doc);
    }
    for first in ["- text", "- [ ]", "- [x]", "- >", "- -"] {
        let doc = parse(&format!("{first}\n\n  text\n"));
        assert_eq!(
            doc.blocks().last().unwrap().container,
            Some(ContainerId(1)),
            "{first}"
        );
        check_reads(&doc);
    }
    for source in [
        format!("-{}\n\n  text\n", " ".repeat(BLOCK_LOOKAHEAD)),
        format!("-\n{}\n  text\n", " ".repeat(BLOCK_LOOKAHEAD)),
    ] {
        let doc = parse(&source);
        assert_eq!(doc.blocks().last().unwrap().container, Some(ContainerId(1)));
        check_reads(&doc);
    }
}

#[test]
fn empty_blank_all_splits_prefixes_and_eof() {
    for source in [
        "-\n\n  中🙂\n",
        "-\r\n \r\n\tcode",
        "> -\n>\n>\t中🙂\n",
        "-\n\n- sibling\n-\n\n  outside",
        "- # parent\n\t-\n\n\t  text\n",
        ">\t-\n>\n>\t  text\n",
        "- [x]\n\n  task\n",
        "- -\n\n  text\n",
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
fn empty_blank_does_not_rescan_retained_ancestors_or_old_text() {
    let prefix = ">".repeat(980);
    let mut doc = Document::new();
    doc.append(&format!("{prefix} {}\n\n", "x".repeat(1_000_000)))
        .unwrap();
    doc.append(&format!("{prefix} - -\n")).unwrap();
    for _ in 0..1000 {
        let before = doc.work();
        doc.append(&format!("{prefix}\n")).unwrap();
        let after = doc.work();
        assert!(after.container_steps - before.container_steps < 4000);
        assert!(after.container_bytes - before.container_bytes < 10_000);
        assert!(after.classified_bytes - before.classified_bytes < 4000);
        assert_eq!(after.inline_bytes, before.inline_bytes);
    }
    doc.append(&format!("{prefix}     body\n")).unwrap();
    let item = doc.blocks().last().unwrap().container.unwrap();
    assert_eq!(doc.container(item).unwrap().depth, 982);
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

#[test]
fn empty_blank_reclassifies_in_parent_scope_and_keeps_eof_ranges() {
    for (source, text, code, depth) in [
        ("-\n\n  text\n", "text\n", false, 0),
        ("-\n\n    code\n", "code\n", true, 0),
        ("-\n\n\tcode\n", "code\n", true, 0),
        ("> -\n>\n>   text\n", "  text\n", false, 1),
        ("- outer\n  -\n\n    text\n", "  text\n", false, 2),
        ("- # parent\n\t-\n\n\t  code\n", "code\n", true, 2),
    ] {
        let doc = parse(source);
        let block = doc.blocks().last().unwrap();
        assert_eq!(
            matches!(block.kind, BlockKind::Code { .. }),
            code,
            "{source:?}"
        );
        assert_eq!(
            block
                .container
                .map_or(0, |id| doc.container(id).unwrap().depth),
            depth,
            "{source:?}"
        );
        assert_eq!(visible(&doc, block.source), text, "{source:?}");
        check_reads(&doc);
    }
    let mut doc = Document::new();
    doc.append("-\n\n  # title\n").unwrap();
    assert_eq!(doc.headings().entries().len(), 1);
    assert_eq!(doc.blocks().last().unwrap().container, None);
    doc.finish();
    for source in ["-\n", "-\n\n", "-\n \r\n\t", "-\n\n  text"] {
        let doc = parse(source);
        let empty = doc.container(ContainerId(1)).unwrap();
        assert_eq!(empty.state, SyntaxState::Sealed);
        assert_eq!(
            empty.source.end,
            if source.ends_with("text") {
                3
            } else {
                source.len()
            }
        );
        assert_eq!(
            doc.container(ContainerId(0)).unwrap().list_tightness,
            Some(ListTightness::Tight)
        );
        check_reads(&doc);
    }
    // Both marker and separator need complete bounded classification. The
    // maximum complete LF line is accepted; a committed long line isn't retried.
    for source in [
        format!("-{}\n\n  text\n", " ".repeat(BLOCK_LOOKAHEAD - 2)),
        format!("-\n{}\n  text\n", " ".repeat(BLOCK_LOOKAHEAD - 1)),
    ] {
        let doc = parse(&source);
        assert_eq!(doc.blocks().last().unwrap().container, None);
        check_reads(&doc);
    }
}
