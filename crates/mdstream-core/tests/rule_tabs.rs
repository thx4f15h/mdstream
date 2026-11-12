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
fn rule_indentation_model_uses_columns_and_preserves_scope() {
    let mut cases = 0;
    for (before, prefix, residual) in [
        ("", ">", 0),
        ("", "> ", 0),
        ("", ">> ", 0),
        ("", "> > ", 0),
        ("", ">>> ", 0),
        ("- seed\n", "  ", 0),
        ("1. seed\n", "   ", 0),
        ("> - seed\n", ">   ", 0),
        ("- seed\n", "\t", 2),
        ("> seed\n", ">\t", 2),
        ("- - seed\n", "\t", 0),
    ] {
        for whitespace in ["", " ", "   ", "\t", " \t", "  \t", "\t ", "\t\t"] {
            let mut column = 0;
            for b in prefix.bytes() {
                column += if b == b'\t' { 4 - column % 4 } else { 1 };
            }
            let start = column;
            for b in whitespace.bytes() {
                column += if b == b'\t' { 4 - column % 4 } else { 1 };
            }
            // A new Quote consumes one column of its directly adjacent tab
            // as the optional separator before measuring rule indentation.
            let separator =
                usize::from(before.is_empty() && prefix == ">" && whitespace.starts_with('\t'));
            let indent = residual + column - start - separator;
            for marker in ['*', '-', '_'] {
                for count in [3, 4, 9] {
                    for ending in ["\n", "\r\n", ""] {
                        // Spaced hyphens cannot be a Setext underline.
                        let payload = vec![marker.to_string(); count].join(" \t");
                        let source = format!("{before}{prefix}{whitespace}{payload}{ending}");
                        let doc = parse(&source);
                        let found = rules(&doc);
                        assert_eq!(found.len(), usize::from(indent <= 3), "{source:?}");
                        if let Some(rule) = found.first() {
                            assert!(rule.container.is_some(), "{source:?}");
                            if !before.is_empty() {
                                assert_eq!(rule.container, doc.blocks()[0].container, "{source:?}");
                            }
                            assert_eq!(rule.source, Span::new(before.len(), source.len()));
                            assert_eq!(rule.state, SyntaxState::Sealed);
                        }
                        assert_eq!(doc.source(), source);
                        cases += 1;
                    }
                }
            }
        }
    }
    assert_eq!(cases, 2376);
}

#[test]
fn precedence_failures_and_shared_window() {
    for (source, count, headings) in [
        ("> \t***\n", 1, 0),
        (">\t___", 1, 0),
        ("> seed\n> \t---\n", 0, 1), // Setext takes priority, including raw tabs
        ("> seed\n> ---\n", 0, 1),
        ("- seed\n\t* * *\n", 1, 0),
        ("- seed\n\t- - -\n", 1, 0),
        ("> - seed\n> \t- - -\n", 1, 0),
        ("> \t**\n", 0, 0),
        ("> \t*-*\n", 0, 0),
        ("> \t*** x\n", 0, 0),
        ("> \t\\***\n", 0, 0),
        ("> \t\t***\n", 0, 0),
        ("\t***\n", 0, 0),
        ("- > seed\n\t***\n", 1, 0),
        ("> ```\n> \t***\n> ```\n", 0, 0),
        ("> $$\n> \t***\n> $$\n", 0, 0),
        ("> a | b\n> \t--- | ---\n", 0, 0),
    ] {
        let doc = parse(source);
        assert_eq!(rules(&doc).len(), count, "{source:?}");
        assert_eq!(doc.headings().entries().len(), headings, "{source:?}");
    }
    for depth in [1016, 1017, 1018, 1019, 1020] {
        let source = format!("{} \t***\n", ">".repeat(depth));
        assert_eq!(
            rules(&parse(&source)).len(),
            usize::from((depth + 1) % 4 != 0),
            "depth {depth}"
        );
    }
    let doc = parse("- seed\n\t* * *\n  tail\n- next\n");
    assert_eq!(doc.container_count(), 3);
    assert_eq!(rules(&doc)[0].container, doc.blocks()[0].container);
    assert_eq!(doc.blocks().last().unwrap().container, Some(ContainerId(2)));
}

#[test]
fn all_splits_prefixes_and_eof_agree() {
    for source in [
        "> \t* * *\r\n> 中🙂\n",
        "- seed\n\t- - -\n  tail\n- next\n",
        "> seed\n>\t___\n",
        "> \t***",
        "> \t*** x\n> after\n",
        "> seed\n> \t---\n",
        "- > seed\n\t***\n",
        "> \t**\n",
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
    let source = format!("> {}\n> \t* * *\r\n", "x".repeat(251));
    let doc = parse(&source);
    assert_eq!(rules(&doc).len(), 1);
    let tab = source.find('\t').unwrap();
    assert_eq!(tab, 256);
    let expected = format!("{}\n\t* * *\n", "x".repeat(251));
    assert_eq!(visible(&doc), expected);
    let mut covered = 0;
    for run in doc.display_runs_in(Span::new(0, source.len())).unwrap() {
        assert_eq!(run.source.start, covered);
        assert_eq!(run.text, &source[run.source.range()]);
        assert!(run.replacement.is_none());
        if run.source.start <= tab && tab < run.source.end {
            assert!(!run.style.contains(Style::HIDDEN));
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
    let prefix = format!("{} \t", ">".repeat(1000));
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
    assert_eq!(doc.container_count(), 1000);
    assert_eq!(rules(&doc)[0].container, doc.blocks()[0].container);
    assert!(doc.work().classified_bytes < 30 * doc.source().len() as u64);
    let source = "> \t* * *\n".repeat(4096);
    let doc = parse(&source);
    assert_eq!(rules(&doc).len(), 4096);
    assert_eq!(visible(&doc), "\t* * *\n".repeat(4096));
    assert!(doc.work().classified_bytes < 10 * source.len() as u64);
    assert!(doc.work().container_steps < 8 * source.len() as u64);
}
