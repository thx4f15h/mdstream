use mdstream::*;

#[path = "support/snapshot.rs"]
mod support;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn lists(doc: &Document) -> Vec<ListTightness> {
    (0..doc.container_count())
        .filter_map(|i| doc.container(ContainerId(i)).unwrap().list_tightness)
        .collect()
}

fn check_reads(doc: &Document) {
    let span = Span::new(0, doc.source().len());
    let raw: String = doc.runs_in(span).unwrap().map(|r| r.text).collect();
    assert_eq!(raw, doc.source());
    let expected: String = doc
        .display_runs_in(span)
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    let mut output = String::new();
    let mut cursor = doc.display_cursor(span).unwrap();
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
                if let Some(replacement) = run.replacement {
                    assert_eq!(replacement, run.source);
                    assert_eq!(doc.text(replacement), Some("\t"));
                    let line_start = doc.source()[..replacement.start]
                        .rfind('\n')
                        .map_or(0, |i| i + 1);
                    let column = replacement.start - line_start;
                    assert_eq!(run.text, " ".repeat(3 - column % 4));
                } else {
                    assert_eq!(run.text, &doc.source()[run.source.range()]);
                }
                if !run.style.contains(Style::HIDDEN) {
                    output.push_str(run.text);
                }
            }
            DisplayStep::Complete => break,
            DisplayStep::BudgetExhausted => panic!("six bytes must make progress"),
        }
        assert!(budget.used().bytes <= 6);
        assert!(budget.used().queries <= 1 && budget.used().objects <= 1);
    }
    assert_eq!(output, expected);
}

#[test]
fn quote_tail_does_not_separate_parent_items_or_blocks() {
    for source in [
        "* a\n  > b\n  >\n* c\n",
        "- > a\n  >\n- b\n",
        "- > a\n  >\n  b\n",
        "- > a\n  >\n  # b\n",
        "- > a\n  >\n  > > b\n  > >\n- c\n",
        "- > a\n  >\n",
        "- > a\n  >",
    ] {
        let doc = parse(source);
        assert_eq!(lists(&doc), [ListTightness::Tight], "{source:?}");
        check_reads(&doc);
    }
}

#[test]
fn quote_boundary_stops_nested_list_tail_but_external_blanks_still_separate() {
    for (source, expected) in [
        (
            "- > - inner\n  >\n- next\n",
            vec![ListTightness::Tight, ListTightness::Tight],
        ),
        (
            "- > - inner\n  >\n  > - next\n- outer\n",
            vec![ListTightness::Tight, ListTightness::Loose],
        ),
        (
            "- - > inner\n    >\n  - next\n- outer\n",
            vec![ListTightness::Tight, ListTightness::Tight],
        ),
        ("- > a\n  >\n\n- b\n", vec![ListTightness::Loose]),
        ("- > a\n  >\n  \n  b\n", vec![ListTightness::Loose]),
        (
            "- > - inner\n\n- next\n",
            vec![ListTightness::Loose, ListTightness::Tight],
        ),
        (
            "- - inner\n\n- next\n",
            vec![ListTightness::Loose, ListTightness::Tight],
        ),
        ("> - a\n>\n> - b\n", vec![ListTightness::Loose]),
    ] {
        assert_eq!(lists(&parse(source)), expected, "{source:?}");
    }
}

#[test]
fn quote_tail_matrix_has_scope_local_spacing() {
    let mut cases = 0;
    for (marker, indent) in [
        ("-", "  "),
        ("+", "  "),
        ("*", "  "),
        ("9.", "   "),
        ("1)", "   "),
    ] {
        for depth in 1..=4 {
            let quotes = ">".repeat(depth);
            for gap in ["", " ", "  ", "\t"] {
                for eol in ["\n", "\r\n"] {
                    for (tail, expected) in [
                        (format!("{marker} next{eol}"), ListTightness::Tight),
                        (format!("{indent}# next{eol}"), ListTightness::Tight),
                        (format!("{eol}{marker} next{eol}"), ListTightness::Loose),
                        (String::new(), ListTightness::Tight),
                    ] {
                        let source =
                            format!("{marker} {quotes} 中🙂{eol}{indent}{quotes}{gap}{eol}{tail}");
                        let doc = parse(&source);
                        assert_eq!(lists(&doc), [expected], "{source:?}");
                        assert_eq!(doc.blocks()[1].kind, BlockKind::Blank);
                        check_reads(&doc);
                        cases += 1;
                    }
                }
            }
        }
    }
    assert_eq!(cases, 640);
}

#[test]
fn quote_tail_is_stable_at_every_split_prefix_and_eof() {
    for source in [
        "* a\n  > b\n  >\n* c\n",
        "- > 中🙂\r\n  >\t\r\n- b",
        "- > - inner\n  >\n- next\n",
        "- > a\n  >\n\n- b\n",
        "- - > inner\n    >\n  - next\n",
    ] {
        let whole = parse(source);
        for split in (0..=source.len()).filter(|&at| source.is_char_boundary(at)) {
            let mut doc = Document::new();
            doc.append(&source[..split]).unwrap();
            doc.append(&source[split..]).unwrap();
            doc.finish();
            assert_eq!(support::snapshot(&doc), support::snapshot(&whole));
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
        doc.finish();
        assert_eq!(support::snapshot(&doc), support::snapshot(&whole));
        check_reads(&doc);
    }
}

#[test]
fn quote_close_notifications_keep_parent_identity_and_fold_state() {
    let mut doc = Document::new();
    doc.append("- > a\n  >\n").unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    view.set_collapsed(&doc, FoldId::Container(ContainerId(0)), true)
        .unwrap();
    let update = doc.append("- b\n").unwrap();
    assert!(update.loosened_lists.is_empty());
    assert_eq!(update.closed_containers, [ContainerId(2), ContainerId(1)]);
    assert_eq!(update.new_containers, 3..4);
    assert_eq!(
        doc.container(ContainerId(3)).unwrap().parent,
        Some(ContainerId(0))
    );
    view.apply(&doc, &update).unwrap();
    assert!(view.is_collapsed(FoldId::Container(ContainerId(0))));
    let update = doc.append("\n- c\n").unwrap();
    assert_eq!(update.loosened_lists, [ContainerId(0)]);
    view.apply(&doc, &update).unwrap();
    let update = doc.finish();
    assert!(update.loosened_lists.is_empty());
    view.apply(&doc, &update).unwrap();
}

#[test]
fn deep_quote_closure_and_long_growth_do_not_rescan_ancestors_or_body() {
    let quotes = ">".repeat(980);
    let mut doc = Document::new();
    doc.append(&format!("- {quotes} {}\n", "x".repeat(1_000_000)))
        .unwrap();
    let before = doc.work();
    doc.append(&format!("  {quotes}\n")).unwrap();
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    let before = doc.work();
    let update = doc.append("- next\n").unwrap();
    assert_eq!(update.closed_containers.len(), 981);
    assert!(update.loosened_lists.is_empty());
    assert!(doc.work().container_steps - before.container_steps < 1000);
    assert!(doc.work().list_steps - before.list_steps < 10);
    assert_eq!(lists(&doc), [ListTightness::Tight]);
    doc.append(&format!("  {}", "x".repeat(1_000_000))).unwrap();
    let before = doc.work();
    let stats = doc.container_arena_stats();
    for _ in 0..1000 {
        doc.append("x").unwrap();
    }
    assert_eq!(doc.work().container_steps, before.container_steps);
    assert_eq!(doc.work().container_bytes, before.container_bytes);
    assert_eq!(doc.work().list_steps, before.list_steps);
    assert_eq!(doc.work().inline_bytes - before.inline_bytes, 1000);
    assert_eq!(doc.container_arena_stats(), stats);
    doc.finish();
}
