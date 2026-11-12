use mdstream::*;

#[path = "support/snapshot.rs"]
mod support;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn rules(doc: &Document) -> Vec<&Block> {
    doc.blocks()
        .iter()
        .filter(|b| b.kind == BlockKind::ThematicBreak)
        .collect()
}

fn visible(doc: &Document) -> String {
    doc.display_runs_in(Span::new(0, doc.source().len()))
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect()
}

#[test]
fn new_quote_rule_uses_physical_columns_and_keeps_scope() {
    let mut cases = 0;
    for prefix in ["", " ", "  ", "   ", "> ", ">> ", "- ", "1. "] {
        for whitespace in ["", " ", "  ", "   ", "\t", " \t", "\t ", "\t\t"] {
            let column = prefix.len() + 1;
            let mut end = column + 4 - column % 4;
            for byte in whitespace.bytes() {
                end += if byte == b'\t' { 4 - end % 4 } else { 1 };
            }
            let indent = end - (column + 1);
            for marker in ['*', '-', '_'] {
                for count in [2, 3, 7] {
                    for ending in ["\n", "\r\n", ""] {
                        let payload = vec![marker.to_string(); count].join(" \t");
                        let source = format!("{prefix}>\t{whitespace}{payload}{ending}");
                        let doc = parse(&source);
                        let found = rules(&doc);
                        assert_eq!(
                            found.len(),
                            usize::from(indent <= 3 && count >= 3),
                            "{source:?}, indent {indent}"
                        );
                        if let Some(rule) = found.first() {
                            assert!(matches!(
                                doc.container(rule.container.unwrap()).unwrap().kind,
                                ContainerKind::Quote
                            ));
                            assert_eq!(rule.source, Span::new(0, source.len()));
                            assert_eq!(rule.state, SyntaxState::Sealed);
                            assert!(doc.headings().entries().is_empty());
                        }
                        assert_eq!(doc.source(), source);
                        cases += 1;
                    }
                }
            }
        }
    }
    assert_eq!(cases, 1728);
}

#[test]
fn precedence_failure_and_window_keep_dialect() {
    for (source, count, headings) in [
        (">\t ---\n", 1, 0),
        (">\t * * *\n", 1, 0),
        ("- >\t - - -\n", 1, 0),
        ("- seed\n\t>\t _ _ _\n", 1, 0), // Partial tab in the retained Item path.
        ("> seed\n> >\t ---\n", 1, 0),   // New scope has no Setext paragraph.
        ("> seed\n>\t ---\n", 0, 1),     // Existing scope retains Setext priority.
        ("> a | b\n> >\t - - -\n", 1, 0),
        (">\t **\n", 0, 0),
        (">\t *-*\n", 0, 0),
        (">\t *** x\n", 0, 0),
        (">\t \\***\n", 0, 0),
        (">\t  ***\n", 0, 0),
        (">\t ***\r", 0, 0),
        ("> ```\n>\t ***\n> ```\n", 0, 0),
        ("> $$\n>\t ***\n> $$\n", 0, 0),
        ("> a | b\n>\t --- | ---\n", 0, 0),
    ] {
        let doc = parse(source);
        assert_eq!(rules(&doc).len(), count, "{source:?}");
        assert_eq!(doc.headings().entries().len(), headings, "{source:?}");
    }
    for (source, expected) in [
        (">\t $$\n", ""), // A confirmed math delimiter is hidden.
        (">\t ordinary\n", "   ordinary\n"),
        (">\t --x\n", "   --x\n"),
    ] {
        assert_eq!(visible(&parse(source)), expected, "{source:?}");
    }
    for depth in 1014..=1024 {
        for ending in ["\n", "\r\n", ""] {
            let source = format!("{}\t***{ending}", ">".repeat(depth));
            assert_eq!(rules(&parse(&source)).len(), 1, "{source:?}");
        }
    }
    let mut doc = Document::new();
    doc.append(">\t ***").unwrap();
    assert!(rules(&doc).is_empty());
    doc.append(" x\n").unwrap();
    assert!(rules(&doc).is_empty());
    let mut doc = Document::new();
    doc.append(">\t ***").unwrap();
    doc.append("\n").unwrap();
    assert_eq!(rules(&doc).len(), 1);
    let doc = parse("- >\t - - -\n  > tail\n- next\n");
    assert_eq!(doc.container_count(), 4);
    assert_eq!(rules(&doc)[0].container, doc.blocks()[1].container);
}

#[test]
fn all_splits_prefixes_and_eof_agree() {
    for source in [
        ">\t * * *\r\n> 中🙂\n",
        "- seed\n\t- - -\n  tail\n- next\n",
        "> seed\n>\t___\n",
        ">\t ***",
        "- >\t - - -\n  > 中🙂\n- next\n",
        "> seed\n> >\t ___\n",
        "- seed\n\t>\t ___\n",
        ">\t *** x\n> after\n",
        "> seed\n>\t ---\n",
        "- > seed\n\t***\n",
        ">\t **\n",
    ] {
        let whole = support::snapshot(&parse(source));
        for split in (0..=source.len()).filter(|&at| source.is_char_boundary(at)) {
            let mut doc = Document::new();
            doc.append(&source[..split]).unwrap();
            doc.append(&source[split..]).unwrap();
            doc.finish();
            assert_eq!(support::snapshot(&doc), whole, "split {split}: {source:?}");
        }
        let mut doc = Document::new();
        for (at, ch) in source.char_indices() {
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            let text = &source[..at + ch.len_utf8()];
            let mut prefix = Document::new();
            prefix.append(text).unwrap();
            assert_eq!(support::snapshot(&doc), support::snapshot(&prefix));
            assert_eq!(doc.pending(), prefix.pending());
            let mut eof = Document::new();
            for ch in text.chars() {
                eof.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            }
            eof.finish();
            assert_eq!(support::snapshot(&eof), support::snapshot(&parse(text)));
        }
    }
}

#[test]
fn mapping_tiles_slices_and_six_byte_budget_preserve_raw_rule() {
    let source = format!("{}\n>\t * * *\r\n", "x".repeat(254));
    let doc = parse(&source);
    assert_eq!(rules(&doc).len(), 1);
    let tab = source.find('\t').unwrap();
    assert_eq!(tab, 256);
    let expected = format!("{}\n * * *\n", "x".repeat(254));
    assert_eq!(visible(&doc), expected);
    let mut covered = 0;
    for run in doc.display_runs_in(Span::new(0, source.len())).unwrap() {
        assert_eq!(run.source.start, covered);
        assert_eq!(run.text, &source[run.source.range()]);
        assert!(run.replacement.is_none());
        if run.source.start <= tab && tab < run.source.end {
            assert!(run.style.contains(Style::HIDDEN));
        }
        covered = run.source.end;
    }
    assert_eq!(covered, source.len());
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
    let mut cursor = doc.display_cursor(Span::new(0, source.len())).unwrap();
    let mut output = String::new();
    while cursor.source_position() < source.len() {
        let position = cursor.source_position();
        assert_eq!(
            cursor
                .next(&doc, &mut Budget::new(WorkLimit::default()))
                .unwrap(),
            DisplayStep::BudgetExhausted
        );
        assert_eq!(cursor.source_position(), position);
        let mut budget = Budget::new(WorkLimit {
            bytes: 6,
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        let DisplayStep::Run(run) = cursor.next(&doc, &mut budget).unwrap() else {
            panic!("must advance")
        };
        assert!(budget.used().bytes <= 6);
        if !run.style.contains(Style::HIDDEN) {
            output.push_str(run.text);
        }
    }
    assert_eq!(output, expected);
}

#[test]
fn deep_long_failure_and_many_rules_have_bounded_work() {
    let prefix = format!("{}\t ", ">".repeat(989));
    let mut doc = Document::new();
    doc.append(&format!("{prefix}{}", "*".repeat(1_000_000)))
        .unwrap();
    assert!(rules(&doc).is_empty());
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("*").unwrap();
    }
    let after = doc.work();
    assert_eq!(after.input_bytes - before.input_bytes, 1000);
    assert_eq!(after.classified_bytes, before.classified_bytes);
    assert_eq!(after.container_bytes, before.container_bytes);
    assert_eq!(after.container_steps, before.container_steps);
    assert!(after.tree_visits - before.tree_visits < 100_000);
    doc.append(&format!("\n{prefix}* * *\n")).unwrap();
    doc.finish();
    assert_eq!(rules(&doc).len(), 2);
    assert_eq!(doc.container_count(), 989);
    assert_eq!(rules(&doc)[0].container, doc.blocks()[0].container);
    assert!(doc.work().classified_bytes < 30 * doc.source().len() as u64);
    let source = ">\t * * *\n\n".repeat(4096);
    let doc = parse(&source);
    assert_eq!(rules(&doc).len(), 4096);
    assert_eq!(visible(&doc), " * * *\n\n".repeat(4096));
    assert!(doc.work().classified_bytes < 10 * source.len() as u64);
    assert!(doc.work().container_steps < 8 * source.len() as u64);
}
