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
fn new_and_sibling_items_keep_code_indent_scope_and_literal_payload() {
    for source in [
        "-\t\tfoo\n",
        "-\t\t[x] *foo*\n",
        "- old\n-\t\tfoo\n",
        "- old\n  - nested\n-\t\tfoo\n",
        "> -\t\tfoo\n",
        "- -\t\tfoo\n",
    ] {
        let doc = parse(source);
        let code = doc.blocks().last().unwrap();
        assert_eq!(code.kind, BlockKind::Code { info: None }, "{source:?}");
        let item = doc.container(code.container.unwrap()).unwrap();
        assert!(matches!(item.kind, ContainerKind::Item { task: None, .. }));
        assert!(doc.links().is_empty());
        assert!(doc.tables().is_empty());
        assert!(
            visible(&doc).ends_with(if source.contains("[x]") {
                "  [x] *foo*\n"
            } else if source.starts_with("- -") || source.starts_with(">") {
                "foo\n"
            } else {
                "  foo\n"
            }),
            "{source:?}: {:?}",
            visible(&doc)
        );
    }
    let doc = parse("- old\n  - nested\n-\t\tfoo\n");
    let first = doc.container(doc.blocks()[0].container.unwrap()).unwrap();
    let nested = doc.container(doc.blocks()[1].container.unwrap()).unwrap();
    let sibling = doc.container(doc.blocks()[2].container.unwrap()).unwrap();
    assert_eq!(first.parent, sibling.parent);
    assert_ne!(first.parent, nested.parent);
    assert_eq!(nested.source.end, 17);
    assert_eq!(
        doc.container(first.parent.unwrap()).unwrap().list_tightness,
        Some(ListTightness::Tight)
    );
    let doc = parse("-\t\tfoo\n      bar\n- next\n");
    assert_eq!(visible(&doc), "  foo\nbar\nnext\n");
    assert_eq!(doc.blocks().len(), 2);
    assert_ne!(doc.blocks()[0].container, doc.blocks()[1].container);
}

#[test]
fn marker_code_matches_independent_column_model() {
    let mut count = 0;
    for outer in ["", "> ", "- "] {
        for marker in ["-", "+", "*", "1.", "123)", "   -"] {
            for whitespace in [
                "\t\t", " \t\t", "\t    ", "\t\t\t", "  \t\t\t", "    \t", "     \t",
            ] {
                let start = outer.len() + marker.len();
                let target = start + 5;
                let mut columns = start;
                let mut consumed = 0;
                for byte in whitespace.bytes() {
                    columns += if byte == b'\t' { 4 - columns % 4 } else { 1 };
                    consumed += 1;
                    if columns >= target {
                        break;
                    }
                }
                if columns < target {
                    continue;
                }
                for body in [
                    "中🙂\tend",
                    "[x] *raw* &amp;",
                    "```rs",
                    "> quote",
                    "-|-",
                    "# title",
                ] {
                    for ending in ["\n", "\r\n", ""] {
                        let source = format!("{outer}{marker}{whitespace}{body}{ending}");
                        let doc = parse(&source);
                        let expected = format!(
                            "{}{}{body}{}",
                            " ".repeat(columns - target),
                            &whitespace[consumed..],
                            if ending.is_empty() { "" } else { "\n" }
                        );
                        assert_eq!(visible(&doc), expected, "{source:?}");
                        assert_eq!(doc.blocks().len(), 1, "{source:?}");
                        assert_eq!(
                            doc.blocks()[0].kind,
                            BlockKind::Code { info: None },
                            "{source:?}"
                        );
                        assert_eq!(
                            doc.container(doc.blocks()[0].container.unwrap())
                                .unwrap()
                                .kind,
                            ContainerKind::Item {
                                number: if marker == "1." {
                                    Some(1)
                                } else if marker == "123)" {
                                    Some(123)
                                } else {
                                    None
                                },
                                task: None
                            }
                        );
                        if !ending.is_empty() {
                            let prefix = format!(
                                "{}{}",
                                if outer == "- " { "  " } else { outer },
                                " ".repeat(marker.len() + 5)
                            );
                            let continued = parse(&format!("{source}{prefix}end{ending}"));
                            assert_eq!(continued.blocks().len(), 1, "{source:?}");
                            assert_eq!(
                                visible(&continued),
                                format!("{expected}end\n"),
                                "{source:?}"
                            );
                        }
                        count += 1;
                    }
                }
            }
        }
    }
    assert_eq!(count, 2268);
    eprintln!("{count} marker code column models; two thirds also check continuation");
}

#[test]
fn insufficient_blank_ascii_and_partial_path_keep_existing_dialect() {
    for (source, expected) in [
        ("-\tfoo\n", "foo\n"),
        ("- \tfoo\n", "foo\n"),
        ("-     foo\n", "foo\n"),
        ("-\t\t\n", "\n"),
        ("- old\n\t\t*raw*\n", "old\n\t\traw\n"),
        (" - foo\n   - bar\n\t - baz\n", "foo\nbar\nbaz\n"),
        ("- ```\n  -\t\tfoo\n  ```\n", "-\t\tfoo\n"),
        ("- $$\n  -\t\tfoo\n  $$\n", "-\t\tfoo\n"),
    ] {
        assert_eq!(visible(&parse(source)), expected, "{source:?}");
    }
}

#[test]
fn every_split_prefix_and_classification_window_preserve_state() {
    for source in [
        "-\t\t中🙂\t*raw*\r\n      end".to_owned(),
        "- old\n  - nested\n-\t\tcode\n- sibling\n".to_owned(),
        "> -\t\t[x] raw\n>       end\n".to_owned(),
        "-\t\t\n-\tfoo\n".to_owned(),
        format!("-\t\t{}\r\n", "x".repeat(BLOCK_LOOKAHEAD + 9)),
        format!("-\t\t{}", " ".repeat(BLOCK_LOOKAHEAD + 9)),
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
    doc.append("-\t\t中").unwrap();
    let key = doc.tile_key(TileId(0)).unwrap();
    let mut cursor = doc
        .display_cursor(Span::new(0, doc.source().len()))
        .unwrap();
    doc.append("\n").unwrap();
    assert_ne!(key, doc.tile_key(TileId(0)).unwrap());
    assert_eq!(
        cursor.next(&doc, &mut Budget::new(WorkLimit::unlimited())),
        Err(LayoutError::StaleContinuation)
    );
}

#[test]
fn partial_code_tab_has_exact_source_mapping_and_bounded_resume() {
    let source = format!("{}\n\n-\t\t中🙂\tend\r\n", "x".repeat(252));
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
    let expected = format!("{}\n\n  中🙂\tend\n", "x".repeat(252));
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
fn deep_million_byte_code_and_many_siblings_do_not_rescan_history() {
    let opener = "- ".repeat(499);
    let mut doc = Document::new();
    doc.append(&format!("{opener}-\t\t{}", "x".repeat(1_000_000)))
        .unwrap();
    assert_eq!(
        doc.blocks().last().unwrap().kind,
        BlockKind::Code { info: None }
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
    for _ in 0..4096 {
        doc.append("-\t\ta\n").unwrap();
    }
    doc.finish();
    assert_eq!(doc.blocks().len(), 4096);
    assert!(
        doc.blocks()
            .iter()
            .all(|b| b.kind == BlockKind::Code { info: None })
    );
    assert_eq!(doc.work().inline_bytes, 0);
    assert!(doc.work().classified_bytes < 4096 * 30);
    assert!(doc.work().container_bytes < 4096 * 100);
    assert!(doc.work().container_steps < 4096 * 10);
    assert_eq!(visible(&doc), "  a\n".repeat(4096));
}
