use mdstream::*;

#[path = "support/snapshot.rs"]
mod support;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn codes(doc: &Document) -> Vec<&Block> {
    doc.blocks()
        .iter()
        .filter(|b| matches!(b.kind, BlockKind::Code { .. }))
        .collect()
}

fn visible(doc: &Document, span: Span) -> String {
    doc.display_runs_in(span)
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect()
}

fn tight(doc: &Document) {
    for i in 0..doc.container_count() {
        if let Some(spacing) = doc.container(ContainerId(i)).unwrap().list_tightness {
            assert_eq!(spacing, ListTightness::Tight);
        }
    }
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
fn omitted_list_indent_keeps_one_literal_code_block() {
    for (source, expected) in [
        ("- a\n- ```\n  b\n\n\n  ```\n- c\n", "b\n\n\n"),
        (
            "- ```\n \n  *中🙂* &amp;\n\n  ```\n- next\n",
            "\n*中🙂* &amp;\n\n",
        ),
        (
            "> - - ~~~\n> \n>     raw\n>   \n>     ~~~\n> - next\n",
            "\nraw\n\n",
        ),
        ("- ```\n\n", "\n"),
        ("- ```\n ", ""),
    ] {
        let doc = parse(source);
        assert_eq!(codes(&doc).len(), 1, "{source:?}");
        assert_eq!(visible(&doc, codes(&doc)[0].source), expected, "{source:?}");
        tight(&doc);
        check_reads(&doc);
    }
}

#[test]
fn blank_indent_matrix_preserves_only_surplus_columns() {
    let mut cases = 0;
    for marker in ["-", "+", "*", "1.", "9)"] {
        for outer in ["", "> ", "> > "] {
            for depth in 1..=3 {
                let items = format!("{marker} ").repeat(depth);
                let width = (marker.len() + 1) * depth;
                for gap in ["", " ", "  ", "   ", "\t", " \t", "\t ", "        "] {
                    for fence in ["```", "~~~"] {
                        for eol in ["\n", "\r\n"] {
                            let prefix = format!("{outer}{}", " ".repeat(width));
                            let source = format!(
                                "{outer}{items}{fence}{eol}{outer}{gap}{eol}{prefix}raw{eol}{prefix}{fence}{eol}"
                            );
                            // Independent physical tab-stop model: remove the total
                            // item width, retaining a split tab as spaces and later bytes verbatim.
                            let mut column = outer.len();
                            let target = column + width;
                            let mut remaining = String::new();
                            for ch in gap.chars() {
                                let next = column + if ch == '\t' { 4 - column % 4 } else { 1 };
                                if column >= target {
                                    remaining.push(ch);
                                } else if next > target {
                                    remaining.push_str(&" ".repeat(next - target));
                                }
                                column = next;
                            }
                            let doc = parse(&source);
                            assert_eq!(codes(&doc).len(), 1, "{source:?}");
                            assert_eq!(
                                visible(&doc, codes(&doc)[0].source),
                                format!("{remaining}\nraw\n"),
                                "{source:?}"
                            );
                            tight(&doc);
                            check_reads(&doc);
                            cases += 1;
                        }
                    }
                }
            }
        }
    }
    assert_eq!(cases, 1440);
}

#[test]
fn missing_quotes_nonblank_lines_math_and_external_spacing_keep_boundaries() {
    for (head, boundary) in [
        ("- > ```\n  > raw\n", "\n"),
        ("> - ```\n>   raw\n", "\n"),
        ("> - > - ```\n>   >   raw\n", ">\n"),
        ("- ```\n  raw\n", " x\n"),
        ("- ```\n  raw\n", "- next\n"),
        ("- ```\n  raw\n", "\u{c}\n"),
    ] {
        let doc = parse(&format!("{head}{boundary}"));
        assert_eq!(codes(&doc)[0].source.end, head.len());
    }
    let doc = parse("- $$\n  raw\n\n  $$\n");
    assert_eq!(doc.blocks()[0].source.end, "- $$\n  raw\n".len());
    let doc = parse("- ```\n\n  ```\n\n- next\n");
    assert_eq!(
        doc.container(ContainerId(0)).unwrap().list_tightness,
        Some(ListTightness::Loose)
    );
    let source = format!("- ```\n{}\n  ```\n", " ".repeat(1025));
    // Existing bounded classifier accepts this fully matched list prefix;
    // long whitespace bodies continue streaming inside the fence.
    let doc = parse(&source);
    assert_eq!(codes(&doc).len(), 1);
    assert_eq!(
        visible(&doc, codes(&doc)[0].source),
        format!("{}\n", " ".repeat(1023))
    );
}

#[test]
fn all_splits_prefixes_and_eof_keep_code_and_mapping() {
    for source in [
        "- a\n- ```\n  b\n\n\n  ```\n- c\n",
        "> - - ~~~\r\n> \t\r\n>     中🙂\r\n>     ~~~\r\n",
        "- ```\n \n  **literal**\n ",
        "- > ```\n  > x\n\n- next\n",
        "- ```\n\n  ```\n\n- next\n",
    ] {
        let whole = parse(source);
        for at in (0..=source.len()).filter(|&at| source.is_char_boundary(at)) {
            let mut doc = Document::new();
            doc.append(&source[..at]).unwrap();
            doc.append(&source[at..]).unwrap();
            doc.finish();
            assert_eq!(support::snapshot(&doc), support::snapshot(&whole));
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
            for c in prefix.chars() {
                eof.append(c.encode_utf8(&mut [0; 4])).unwrap();
            }
            eof.finish();
            assert_eq!(support::snapshot(&eof), support::snapshot(&parse(prefix)));
            check_reads(&eof);
        }
    }
}

#[test]
fn code_identity_and_fold_survive_blanks_without_loosen_notifications() {
    let mut doc = Document::new();
    doc.append("- ```\n  x\n").unwrap();
    let scope = doc.blocks()[0].container;
    let mut view = FoldView::new(&doc, 1).unwrap();
    view.set_collapsed(&doc, FoldId::Code(BlockId(0)), true)
        .unwrap();
    for source in ["\n", " \n", "  raw\n", "  ```\n", "- next\n"] {
        let update = doc.append(source).unwrap();
        assert!(update.loosened_lists.is_empty());
        view.apply(&doc, &update).unwrap();
        assert!(view.is_collapsed(FoldId::Code(BlockId(0))));
        assert_eq!(doc.blocks()[0].container, scope);
        assert_eq!(codes(&doc).len(), 1);
    }
    assert_eq!(visible(&doc, doc.blocks()[0].source), "x\n\n\nraw\n");
    tight(&doc);
}

#[test]
fn deep_list_blanks_and_long_code_do_not_rescan_ancestors_or_history() {
    let depth = 480;
    let mut doc = Document::new();
    doc.append(&format!("{}```\n", "- ".repeat(depth))).unwrap();
    let stats = doc.container_arena_stats();
    let before = doc.work();
    for _ in 0..1000 {
        let update = doc.append("\n").unwrap();
        assert!(update.loosened_lists.is_empty());
        assert!(update.closed_containers.is_empty());
    }
    assert!(doc.work().container_steps - before.container_steps < 10_000);
    assert!(doc.work().container_bytes - before.container_bytes < 10_000);
    assert!(doc.work().list_steps - before.list_steps < 10_000);
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert_eq!(doc.container_arena_stats(), stats);
    doc.append(&format!(
        "{}{}",
        " ".repeat(depth * 2),
        "x".repeat(1_000_000)
    ))
    .unwrap();
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("x").unwrap();
    }
    assert_eq!(doc.work().container_steps, before.container_steps);
    assert_eq!(doc.work().container_bytes, before.container_bytes);
    assert_eq!(doc.work().list_steps, before.list_steps);
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    doc.append("\n\n").unwrap();
    doc.finish();
    assert_eq!(codes(&doc).len(), 1);
    assert_eq!(visible(&doc, codes(&doc)[0].source).len(), 1_002_002);
    tight(&doc);
}
