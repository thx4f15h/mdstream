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
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect()
}

#[test]
fn quote_code_columns_match_space_model_with_literal_payload() {
    let mut count = 0;
    for lead in ["", " ", "  ", "   ", "> ", "- ", "- > "] {
        for whitespace in ["\t\t", " \t\t", "\t  ", "\t   ", "    \t"] {
            let column = lead.len() + 1;
            let mut end = column;
            let mut consumed = 0;
            for byte in whitespace.bytes() {
                end += if byte == b'\t' { 4 - end % 4 } else { 1 };
                consumed += 1;
                if end - column >= 5 {
                    break;
                }
            }
            let tail = &whitespace[consumed..];
            if end - column < 5 {
                continue;
            }
            for body in [
                "中🙂\tend",
                "*raw* &amp;",
                "# title",
                "```rs",
                "-|-",
                "> nested",
            ] {
                for ending in ["\n", "\r\n", ""] {
                    let source = format!("{lead}>{whitespace}{body}{ending}");
                    let model = parse(&format!(
                        "{lead}>{}{tail}{body}{ending}",
                        " ".repeat(end - column)
                    ));
                    let doc = parse(&source);
                    assert_eq!(doc.blocks().len(), 1, "{source:?}");
                    assert_eq!(
                        doc.blocks()[0].kind,
                        BlockKind::Code { info: None },
                        "{source:?}"
                    );
                    assert_eq!(visible(&doc), visible(&model), "{source:?}");
                    assert_eq!(
                        visible(&doc),
                        format!(
                            "{}{tail}{body}{}",
                            " ".repeat(end - column - 5),
                            if ending.is_empty() { "" } else { "\n" }
                        )
                    );
                    let scope = doc.container(doc.blocks()[0].container.unwrap()).unwrap();
                    assert_eq!(scope.kind, ContainerKind::Quote);
                    assert!(doc.tables().is_empty());
                    assert!(doc.links().is_empty());
                    if !ending.is_empty() {
                        let continuation =
                            format!("{}>{whitespace}{body}{ending}", lead.replace("- ", "  "));
                        let continued = parse(&format!("{source}{continuation}"));
                        assert_eq!(continued.blocks().len(), 1, "{source:?}");
                        assert_eq!(continued.blocks()[0].kind, BlockKind::Code { info: None });
                        assert_eq!(visible(&continued), visible(&doc).repeat(2), "{source:?}");
                    }
                    count += 1;
                }
            }
        }
    }
    assert_eq!(count, 504);
    eprintln!("{count} quote indentation model cases");
}

#[test]
fn retained_quotes_merge_code_and_preserve_failed_paths_and_fences() {
    let doc = parse(">\t\tfoo\n>\t\tbar\n>     baz\n");
    assert_eq!(doc.blocks().len(), 1);
    assert_eq!(doc.blocks()[0].kind, BlockKind::Code { info: None });
    assert_eq!(visible(&doc), "  foo\n  bar\nbaz\n");
    for (source, expected) in [
        ("> old\n>\t\t*raw*\n", "old\n  *raw*\n"),
        (">\t# title\n", "title\n"),
        ("> \tfoo\n", "\tfoo\n"),
        (">\t\t\n", "  \t\n"),
        ("- old\n\t\t# literal\n", "old\n\t\t# literal\n"),
        ("> ```\n>\t\tfoo\n> ```\n", "  \tfoo\n"),
        ("> $$\n>\t\tfoo\n> $$\n", "  \tfoo\n"),
    ] {
        assert_eq!(visible(&parse(source)), expected, "{source:?}");
    }
    let doc = parse("- > old\n\t\tfoo\n");
    assert!(
        doc.blocks()
            .iter()
            .all(|b| !matches!(b.kind, BlockKind::Code { .. }))
    );
    for head in ["> a|b\n", "> a|b\n> -|-\n> x|y\n"] {
        let doc = parse(&format!("{head}>\t\t-|-\n"));
        assert_eq!(doc.tables().len(), usize::from(head.contains("-|-")));
        assert_eq!(
            doc.blocks().last().unwrap().kind,
            BlockKind::Code { info: None }
        );
        assert!(visible(&doc).ends_with("  -|-\n"));
    }
}

#[test]
fn every_split_prefix_and_long_classification_window_preserve_state() {
    for source in [
        ">\t\t中🙂\t*raw*\r\n>\t\tend".to_owned(),
        "- >\t\tfirst\n  >\t\tsecond\n- sibling\n".to_owned(),
        "> a|b\n>\t\t-|-\n".to_owned(),
        format!(">\t\t{}\r\n", "x".repeat(BLOCK_LOOKAHEAD + 9)),
        format!(">\t\t{}", " ".repeat(BLOCK_LOOKAHEAD + 9)),
    ] {
        let expected = support::snapshot(&parse(&source));
        for split in (0..=source.len()).filter(|&i| source.is_char_boundary(i)) {
            let mut doc = Document::new();
            doc.append(&source[..split]).unwrap();
            doc.append(&source[split..]).unwrap();
            doc.finish();
            assert_eq!(support::snapshot(&doc), expected, "split {split}");
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
    doc.append(">\t\t中").unwrap();
    let key = doc.tile_key(TileId(0)).unwrap();
    let mut cursor = doc
        .display_cursor(Span::new(0, doc.source().len()))
        .unwrap();
    doc.append("\n").unwrap();
    assert_eq!(visible(&doc), "  中\n");
    assert_ne!(key, doc.tile_key(TileId(0)).unwrap());
    assert_eq!(
        cursor.next(&doc, &mut Budget::new(WorkLimit::unlimited())),
        Err(LayoutError::StaleContinuation)
    );
}

#[test]
fn partial_code_tab_has_exact_source_mapping_and_bounded_resume() {
    let source = format!(">     {}\n>\t\t中🙂\tend\r\n", "x".repeat(247));
    let doc = parse(&source);
    let tab = source.find("\t\t").unwrap() + 1;
    assert_eq!(tab, 256);
    let mut end = 0;
    let mut replacements = 0;
    for run in doc.display_runs_in(Span::new(0, source.len())).unwrap() {
        assert_eq!(run.source.start, end);
        end = run.source.end;
        if let Some(mapped) = run.replacement {
            assert_eq!(mapped, Span::new(tab, tab + 1));
            assert_eq!(run.source, mapped);
            assert_eq!(doc.text(mapped), Some("\t"));
            assert_eq!(run.text, "  ");
            assert!(run.style.contains(Style::CODE));
            assert!(!run.style.contains(Style::HIDDEN));
            replacements += 1;
        } else {
            assert_eq!(run.text, &source[run.source.range()]);
        }
    }
    assert_eq!((end, replacements), (source.len(), 1));
    let expected = format!("{}\n  中🙂\tend\n", "x".repeat(247));
    assert_eq!(visible(&doc), expected);
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
        let old = cursor.source_position();
        assert_eq!(
            cursor
                .next(&doc, &mut Budget::new(WorkLimit::default()))
                .unwrap(),
            DisplayStep::BudgetExhausted
        );
        assert_eq!(cursor.source_position(), old);
        let mut budget = Budget::new(WorkLimit {
            bytes: 6,
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        let DisplayStep::Run(run) = cursor.next(&doc, &mut budget).unwrap() else {
            panic!("no progress")
        };
        assert_eq!(run.source.start, old);
        assert!(budget.used().bytes <= 6);
        assert!(cursor.source_position() > old);
        if !run.style.contains(Style::HIDDEN) {
            output.push_str(run.text);
        }
    }
    assert_eq!(output, expected);
}

#[test]
fn deep_million_byte_code_and_many_lines_do_not_rescan_history() {
    let prefix = ">".repeat(1000);
    let mut doc = Document::new();
    doc.append(&format!("{prefix}>\t\t{}", "x".repeat(1_000_000)))
        .unwrap();
    assert_eq!(doc.blocks()[0].kind, BlockKind::Code { info: None });
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("x").unwrap();
    }
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert_eq!(doc.work().classified_bytes, before.classified_bytes);
    assert_eq!(doc.work().container_bytes, before.container_bytes);
    assert_eq!(doc.work().container_steps, before.container_steps);
    assert_eq!(doc.work().tree_visits, before.tree_visits);
    doc.append("\n").unwrap();
    doc.finish();
    let mut doc = Document::new();
    for _ in 0..4096 {
        doc.append(">\t\ta\n").unwrap();
    }
    doc.finish();
    assert_eq!(doc.blocks().len(), 1);
    assert_eq!(doc.work().inline_bytes, 0);
    assert!(doc.work().classified_bytes < 4096 * 30);
    assert!(doc.work().container_bytes < 4096 * 80);
    assert!(doc.work().container_steps < 4096 * 10);
    assert_eq!(visible(&doc), "  a\n".repeat(4096));
}
