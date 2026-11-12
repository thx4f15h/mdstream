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
                if let Some(replacement) = run.replacement {
                    assert_eq!(doc.text(replacement), Some("\t"));
                    assert!(run.text.chars().all(|ch| ch == ' '));
                } else {
                    assert_eq!(run.text, &doc.source()[run.source.range()]);
                }
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
fn empty_markers_continue_root_and_explicit_scoped_paragraphs() {
    let mut cases = 0;
    for (first, prefix, containers) in [
        ("text\n", "", 0),
        ("> text\n", "> ", 1),
        ("- text\n", "  ", 2),
        ("> - text\n", ">   ", 3),
    ] {
        for marker in ["*", "+", "1.", "0)", "009.", "123456789)"] {
            for indent in 0..=3 {
                for gap in ["", " ", "  ", "\t", " \t"] {
                    for end in ["", "\n", "\r\n"] {
                        let payload = format!("{}{marker}{gap}{end}", " ".repeat(indent));
                        let source = format!("{first}{prefix}{payload}");
                        let doc = parse(&source);
                        assert_eq!(doc.container_count(), containers, "{source:?}");
                        // Root paragraph indentation is structural; scoped prose keeps it.
                        // Terminal raw spaces/tabs are hidden when the paragraph seals.
                        let shown_indent = if containers == 0 { 0 } else { indent };
                        let shown_end = if end.is_empty() { "" } else { "\n" };
                        assert_eq!(
                            visible(&doc),
                            format!("text\n{}{marker}{shown_end}", " ".repeat(shown_indent)),
                            "{source:?}"
                        );
                        assert!(doc.headings().entries().is_empty());
                        assert_eq!(
                            doc.blocks().last().unwrap().container,
                            doc.blocks()[0].container
                        );
                        check_reads(&doc);
                        cases += 1;
                    }
                }
            }
        }
    }
    assert_eq!(cases, 1440);
}

#[test]
fn empty_markers_keep_lazy_and_partial_tab_paths() {
    for (source, expected, count) in [
        ("> text\n*\n", "text\n*\n", 1),
        ("- text\n+\n", "text\n+\n", 2),
        ("> - text\n1.\n", "text\n1.\n", 3),
        ("> text\n>\t*\n", "text\n  *\n", 1),
        ("- text\n\t+\n", "text\n  +\n", 2),
        ("- > text\n\t1.\n", "text\n  1.\n", 3),
    ] {
        let doc = parse(source);
        assert_eq!(doc.container_count(), count, "{source:?}");
        assert_eq!(visible(&doc), expected, "{source:?}");
        assert_eq!(
            doc.blocks().last().unwrap().container,
            doc.blocks()[0].container
        );
        check_reads(&doc);
    }
}

#[test]
fn siblings_new_scopes_tasks_and_block_priorities_remain_distinct() {
    for (source, count) in [
        ("- text\n-\n", 3),
        ("1. text\n2.\n", 3),
        (">  - text\n>\t-\n", 4),
        ("text\n\n*\n", 2),
        ("# title\n*\n", 2),
        ("text\n> *\n", 3),
        ("text\n- [ ]\n", 2),
        ("text\n2. body\n", 0),
        ("text\n*    body\n", 2),
        ("text\n*     body\n", 2),
    ] {
        let doc = parse(source);
        assert_eq!(doc.container_count(), count, "{source:?}");
        check_reads(&doc);
    }
    for source in ["text\n-\n", "> text\n> -\n", "- text\n  -\n"] {
        let doc = parse(source);
        assert_eq!(doc.headings().entries().len(), 1, "{source:?}");
        assert_eq!(doc.headings().entries()[0].level, 2);
    }
    let doc = parse("text\n* * *\n");
    assert_eq!(doc.blocks()[1].kind, BlockKind::ThematicBreak);
    for (source, count) in [("text | col\n+\n", 0), ("> text | col\n> +\n", 1)] {
        let doc = parse(source);
        assert_eq!(doc.container_count(), count);
        assert!(doc.tables().is_empty());
        assert_eq!(visible(&doc), "text | col\n+\n");
    }
    let mut doc = Document::new();
    doc.append("- text\n").unwrap();
    let update = doc.append("  +\n").unwrap();
    assert!(update.new_containers.is_empty());
    assert!(update.closed_containers.is_empty());
    assert!(update.loosened_lists.is_empty());
    assert_eq!(
        doc.container(ContainerId(0)).unwrap().list_tightness,
        Some(ListTightness::Tight)
    );
    let update = doc.append("\n- next\n").unwrap();
    assert_eq!(update.new_containers, 2..3);
    assert_eq!(update.loosened_lists, vec![ContainerId(0)]);
}

#[test]
fn empty_marker_decisions_are_bounded_by_the_classification_window() {
    for ending in ["", "\n"] {
        let short = format!("text\n*{}{ending}", " ".repeat(BLOCK_LOOKAHEAD - 2));
        assert_eq!(parse(&short).container_count(), 0);
        // Whole-line emptiness retains paragraph priority at every length.
        for tail in ["", "body"] {
            let long = format!("text\n*{}{tail}{ending}", " ".repeat(BLOCK_LOOKAHEAD));
            let whole = parse(&long);
            assert_eq!(whole.container_count(), if tail.is_empty() { 0 } else { 2 });
            for split in [5, 6, BLOCK_LOOKAHEAD + 4, BLOCK_LOOKAHEAD + 5, long.len()] {
                let mut doc = Document::new();
                doc.append(&long[..split]).unwrap();
                doc.append(&long[split..]).unwrap();
                doc.finish();
                assert_eq!(support::snapshot(&doc), support::snapshot(&whole));
            }
            check_reads(&whole);
        }
    }
    let mut doc = Document::new();
    doc.append("text\n*  ").unwrap();
    assert_eq!(doc.container_count(), 0);
    doc.append("body\n").unwrap();
    assert_eq!(doc.container_count(), 2);
}

#[test]
fn empty_marker_splits_prefixes_and_eof_preserve_semantics() {
    for source in [
        "中🙂\n* \r\n\nfoo\n1.\n",
        "> - text\n>   +\t\n>   next\n",
        "> text\n>\t*\n",
        "- text\n\t+\n",
        "> text\n*\n",
        "- text\n-\n",
        "text\n*  body\n",
        "- text\n  -\n",
        "text | col\n+\n",
        "> text | col\n> +\n",
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
fn empty_marker_checks_do_not_revisit_old_text_or_ancestors() {
    let prefix = ">".repeat(980);
    let mut doc = Document::new();
    doc.append(&format!("{prefix} {}\n", "x".repeat(1_000_000)))
        .unwrap();
    let before = doc.work();
    for _ in 0..100 {
        let update = doc.append(&format!("{prefix} *\n")).unwrap();
        assert!(update.new_containers.is_empty());
        assert!(update.closed_containers.is_empty());
    }
    assert_eq!(doc.container_count(), 980);
    assert!(doc.work().container_steps - before.container_steps < 200_000);
    assert!(doc.work().container_bytes - before.container_bytes < 800_000);
    assert!(doc.work().classified_bytes - before.classified_bytes < 400_000);
    doc.append(&format!("{prefix} {}", "x".repeat(1_000_000)))
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
