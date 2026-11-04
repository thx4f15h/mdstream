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
fn full_boundary_item_code_matches_independent_column_model() {
    let mut count = 0;
    for marker in ["- ", "-  ", "-   ", "1.  ", "123.    ", "-   -   "] {
        for whitespace in [
            "  \t\t",
            "   \t\t",
            "\t\t",
            " \t\t",
            "\t    ",
            "\t\t\t",
            "        \t",
        ] {
            // Consume whole source bytes up to the Item boundary; cases that
            // split an ancestor tab are covered by item_tab_code instead.
            let mut columns = 0;
            let mut boundary = false;
            let mut consumed = 0;
            let target = marker.len() + 4;
            for byte in whitespace.bytes() {
                columns += if byte == b'\t' { 4 - columns % 4 } else { 1 };
                boundary |= columns == marker.len();
                consumed += 1;
                if columns >= target {
                    break;
                }
            }
            if !boundary || columns < target {
                continue;
            }
            let tail = &whitespace[consumed..];
            for body in [
                "中🙂\tend",
                "*raw* &amp;",
                "# title",
                "```rs",
                "-|-",
                "> nested",
            ] {
                for ending in ["\n", "\r\n", ""] {
                    let source = format!("{marker}seed\n\n{whitespace}{body}{ending}");
                    let doc = parse(&source);
                    let expected = format!(
                        "seed\n\n{}{tail}{body}{}",
                        " ".repeat(columns - target),
                        if ending.is_empty() { "" } else { "\n" }
                    );
                    assert_eq!(visible(&doc), expected, "{source:?}");
                    let code = doc.blocks().last().unwrap();
                    assert_eq!(code.kind, BlockKind::Code { info: None }, "{source:?}");
                    assert_eq!(code.container, doc.blocks()[0].container);
                    assert_eq!(doc.source(), source);
                    assert!(doc.links().is_empty());
                    assert!(doc.tables().is_empty());
                    let item = doc.container(code.container.unwrap()).unwrap();
                    assert_eq!(
                        doc.container(item.parent.unwrap()).unwrap().list_tightness,
                        Some(ListTightness::Loose)
                    );
                    if !ending.is_empty() {
                        let continued = parse(&format!("{source}{whitespace}{body}{ending}"));
                        assert_eq!(continued.blocks().len(), doc.blocks().len());
                        assert_eq!(visible(&continued), format!("{expected}{}", &expected[6..]));
                    }
                    count += 1;
                }
            }
        }
    }
    assert_eq!(count, 414);
    eprintln!("{count} full-boundary item code models (two thirds also check continuation)");
}

#[test]
fn failed_paths_prose_blank_and_fences_keep_dialect() {
    for (source, expected) in [
        ("-   old\n\t\t*raw*\n", "old\n\traw\n"),
        ("-   old\n\n\tplain\n", "old\n\nplain\n"),
        ("-   old\n\n\t\t\n", "old\n\n\t\n"),
        ("-   > old\n\t\tfoo\n", "old\n\tfoo\n"),
        ("-   ```\n\t\tfoo\n\t```\n", "\tfoo\n"),
        ("-   $$\n\t\tfoo\n\t$$\n", "\tfoo\n"),
    ] {
        assert_eq!(visible(&parse(source)), expected, "{source:?}");
    }
    let doc = parse("> -   seed\n>\n>     \t\t中\n");
    assert_eq!(
        doc.blocks().last().unwrap().kind,
        BlockKind::Code { info: None }
    );
    assert_eq!(
        doc.blocks().last().unwrap().container,
        doc.blocks()[0].container
    );
    assert_eq!(visible(&doc), "seed\n\n  中\n");
}

#[test]
fn every_split_prefix_and_classification_window_preserve_state() {
    for source in [
        "-   seed\n\n\t\t中🙂\t*raw*\r\n\t\tend".to_owned(),
        "-   -   seed\n\n\t\t\tfirst\n\t\t\tsecond\n- sibling\n".to_owned(),
        "> -   seed\n>\n>     \t\tcode\n".to_owned(),
        "- old\n\t\t*raw*\n".to_owned(),
        format!("-   seed\n\n\t\t{}\r\n", "x".repeat(BLOCK_LOOKAHEAD + 9)),
        format!("-   seed\n\n\t\t{}", " ".repeat(BLOCK_LOOKAHEAD + 9)),
    ] {
        let expected = support::snapshot(&parse(&source));
        for split in (0..=source.len()).filter(|&i| source.is_char_boundary(i)) {
            let mut doc = Document::new();
            doc.append(&source[..split]).unwrap();
            doc.append(&source[split..]).unwrap();
            doc.finish();
            assert_eq!(
                support::snapshot(&doc),
                expected,
                "split {split}: {source:?}"
            );
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
    doc.append("-   seed\n\n\t\t中").unwrap();
    let key = doc.tile_key(TileId(0)).unwrap();
    let mut cursor = doc
        .display_cursor(Span::new(0, doc.source().len()))
        .unwrap();
    doc.append("\n").unwrap();
    assert_eq!(visible(&doc), "seed\n\n中\n");
    assert_ne!(key, doc.tile_key(TileId(0)).unwrap());
    assert_eq!(
        cursor.next(&doc, &mut Budget::new(WorkLimit::unlimited())),
        Err(LayoutError::StaleContinuation)
    );
}

#[test]
fn full_boundary_code_tab_has_exact_source_mapping_and_bounded_resume() {
    let source = format!(
        "-  seed\n\n       {}\n   \t\t中🙂\tend\r\n",
        "x".repeat(235)
    );
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
            assert_eq!(run.text, " ");
            assert!(run.style.contains(Style::CODE));
            assert!(!run.style.contains(Style::HIDDEN));
            replacements += 1;
        } else {
            assert_eq!(run.text, &source[run.source.range()]);
        }
    }
    assert_eq!((end, replacements), (source.len(), 1));
    let expected = format!("seed\n\n{}\n 中🙂\tend\n", "x".repeat(235));
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
    let opener = "-   ".repeat(250);
    let prefix = "\t".repeat(251);
    let mut doc = Document::new();
    doc.append(&format!(
        "{opener}seed\n\n{prefix}{}",
        "x".repeat(1_000_000)
    ))
    .unwrap();
    assert_eq!(
        doc.blocks().last().unwrap().kind,
        BlockKind::Code { info: None }
    );
    assert_eq!(
        doc.blocks().last().unwrap().container,
        doc.blocks()[0].container
    );
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
    doc.append("-   seed\n\n").unwrap();
    let before = doc.work();
    for _ in 0..4096 {
        doc.append("\t\ta\n").unwrap();
    }
    doc.finish();
    assert_eq!(
        doc.blocks()
            .iter()
            .filter(|b| matches!(b.kind, BlockKind::Code { .. }))
            .count(),
        1
    );
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert!(doc.work().classified_bytes < 4096 * 30);
    assert!(doc.work().container_bytes < 4096 * 80);
    assert!(doc.work().container_steps < 4096 * 10);
    assert_eq!(visible(&doc), format!("seed\n\n{}", "a\n".repeat(4096)));
}
