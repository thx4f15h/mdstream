use mdstream::*;

#[path = "support/snapshot.rs"]
mod support;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn definitions(doc: &Document) -> usize {
    doc.blocks()
        .iter()
        .filter(|b| matches!(b.kind, BlockKind::Definition { .. }))
        .count()
}

fn visible(doc: &Document) -> String {
    doc.display_runs_in(Span::new(0, doc.source().len()))
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect()
}

#[test]
fn same_scope_definitions_continue_prose_without_resolving_references() {
    for (first, next) in [
        ("", ""),
        ("> ", "> "),
        ("- ", "  "),
        ("> - ", ">   "),
        ("- > ", "  > "),
        ("- ", "\t"),
        ("> ", ">\t"),
        ("- - ", "\t"),
    ] {
        for body in ["seed", "a|b", "*open", "`open", "[open"] {
            let source = format!("{first}{body}\n{next}[r]: /bad 'title'\n");
            let doc = parse(&source);
            assert_eq!(definitions(&doc), 0, "{source:?}");
            assert!(doc.references().iter().all(|r| r.destination.is_none()));
            assert!(visible(&doc).contains(": /bad 'title'"));
            assert_eq!(
                doc.blocks()[0].container,
                doc.blocks().last().unwrap().container
            );
            assert!(doc.tables().is_empty());
        }
    }
}

#[test]
fn independent_scope_and_blank_model_preserves_real_definitions() {
    let mut cases = 0;
    for depth in 0..8 {
        let parent = "> ".repeat(depth);
        for (first, next) in [("", ""), ("- ", "  "), ("12. ", "    ")] {
            for indent in 0..=3 {
                for end in ["\n", "\r\n", ""] {
                    for separated in [false, true] {
                        let separator = if separated {
                            format!("{parent}{next}\n")
                        } else {
                            String::new()
                        };
                        let source = format!(
                            "{parent}{first}seed\n{separator}{parent}{next}{}[r]: /url{end}",
                            " ".repeat(indent)
                        );
                        let doc = parse(&source);
                        assert_eq!(definitions(&doc), usize::from(separated), "{source:?}");
                        assert_eq!(
                            doc.references()
                                .iter()
                                .filter(|r| r.destination.is_some())
                                .count(),
                            usize::from(separated)
                        );
                        assert!(doc.work().classified_bytes <= 12 * source.len() as u64);
                        cases += 1;
                    }
                }
            }
        }
    }
    assert_eq!(cases, 576);
    for source in [
        "[r]: /url\n[s]: /second\n",
        "seed\n> [r]: /url\n",
        "seed\n- [r]: /url\n",
        "- seed\n- [r]: /url\n",
        "> seed\n[r]: /url\n",
        "# heading\n[r]: /url\n",
        "a|b\n-|-\n[r]: /url\n",
        "> a|b\n> -|-\n> [r]: /url\n",
    ] {
        assert!(definitions(&parse(source)) > 0, "{source:?}");
    }
    // Footnotes keep their distinct interruption dialect.
    assert!(
        parse("seed\n[^f]: note\n")
            .blocks()
            .iter()
            .any(|b| matches!(b.kind, BlockKind::FootnoteDefinition { .. }))
    );
    eprintln!("{cases} independent scope/blank/indent/ending models");
}

#[test]
fn every_split_prefix_and_eof_preserves_definition_eligibility() {
    let mut splits = 0;
    let mut prefixes = 0;
    for source in [
        "Foo\n[bar]: /baz\n\n[bar]\n",
        "- *中🙂\n\t[r]: /url*\n",
        "> a|b\r\n>\t[r]: /url 't'\r\n",
        "> seed\n>\n> [r]: /url\n",
        "- seed\n- [r]: /url\n",
        "- `open\n  [r]: /url`\n",
        "[r]: /first\nseed\n[r]: /second",
        "a|b\n[r]: /not | table\n",
        "seed\n[r]:\n/url\n",
        "seed\n[r]: /url",
    ] {
        let expected = support::snapshot(&parse(source));
        for at in (0..=source.len()).filter(|&at| source.is_char_boundary(at)) {
            let mut doc = Document::new();
            doc.append(&source[..at]).unwrap();
            doc.append(&source[at..]).unwrap();
            doc.finish();
            assert_eq!(support::snapshot(&doc), expected, "{source:?}, split {at}");
            splits += 1;
        }
        let mut doc = Document::new();
        for (at, ch) in source.char_indices() {
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            let source = &source[..at + ch.len_utf8()];
            let mut bulk = Document::new();
            bulk.append(source).unwrap();
            assert_eq!(support::snapshot(&doc), support::snapshot(&bulk));
            assert_eq!(doc.pending(), bulk.pending());
            let mut eof = Document::new();
            for ch in source.chars() {
                eof.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            }
            eof.finish();
            assert_eq!(support::snapshot(&eof), support::snapshot(&parse(source)));
            prefixes += 1;
        }
    }
    eprintln!("{splits} splits and {prefixes} prefixes plus EOF");
}

#[test]
fn late_real_definition_preserves_blocks_links_and_notifications() {
    let mut doc = Document::new();
    doc.append("- [r] *open\n").unwrap();
    let block = doc.blocks()[0].clone();
    let link = *doc.link_info(LinkId(0)).unwrap();
    let update = doc.append("\t[r]: /ignored*\n").unwrap();
    assert!(update.references_changed.is_empty());
    assert_eq!(definitions(&doc), 0);
    assert_eq!(doc.link_destination(LinkId(0)), None);
    assert_eq!(doc.link_info(LinkId(0)), Some(&link));
    assert_eq!(doc.blocks()[0], block);
    assert!(
        doc.display_runs_in(doc.blocks()[1].source)
            .unwrap()
            .any(|r| r.style.contains(Style::EMPHASIS) && r.text.contains("ignored"))
    );
    let update = doc.append("\n[r]: /real\n").unwrap();
    assert_eq!(update.references_changed.len(), 1);
    assert_eq!(definitions(&doc), 1);
    assert_eq!(doc.link_destination(LinkId(0)), Some("/real"));
    assert_eq!(doc.link_info(LinkId(0)), Some(&link));
    assert_eq!(doc.blocks()[0], block);
    doc.finish();
}

#[test]
fn continued_definition_text_keeps_source_mapping_and_shared_budget() {
    let source = format!("- {}\n\t[r]: /中🙂&amp; 't'\n", "x".repeat(260));
    let doc = parse(&source);
    let full = Span::new(0, source.len());
    let expected = visible(&doc);
    assert_eq!(definitions(&doc), 0);
    assert!(expected.contains("\n  r: /中🙂& 't'"));
    assert_eq!(
        doc.runs_in(full)
            .unwrap()
            .map(|r| r.text)
            .collect::<String>(),
        source
    );
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
    let mut cursor = doc.display_cursor(full).unwrap();
    let mut second = doc.display_cursor(full).unwrap();
    let mut output = String::new();
    while cursor.source_position() < full.end {
        let at = cursor.source_position();
        assert_eq!(
            cursor
                .next(&doc, &mut Budget::new(WorkLimit::default()))
                .unwrap(),
            DisplayStep::BudgetExhausted
        );
        assert_eq!(cursor.source_position(), at);
        let mut budget = Budget::new(WorkLimit {
            bytes: 6,
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        let DisplayStep::Run(run) = cursor.next(&doc, &mut budget).unwrap() else {
            panic!("one scalar fits")
        };
        assert_eq!(run.source.start, at);
        if let Some(replacement) = run.replacement {
            assert!(matches!(doc.text(replacement), Some("\t" | "&amp;")));
        } else {
            assert_eq!(run.text, doc.text(run.source).unwrap());
        }
        if !run.style.contains(Style::HIDDEN) {
            output.push_str(run.text);
        }
        assert_eq!(
            second.next(&doc, &mut budget).unwrap(),
            DisplayStep::BudgetExhausted
        );
        assert_eq!(second.source_position(), 0);
        assert!(budget.used().bytes <= 6);
    }
    assert_eq!(output, expected);
}

#[test]
fn deep_long_and_historical_paragraphs_do_not_get_rescanned() {
    for depth in [0, 100, 970] {
        let prefix = ">".repeat(depth);
        let prefix = if depth == 0 {
            prefix
        } else {
            format!("{prefix} ")
        };
        let mut doc = Document::new();
        doc.append(&format!("{prefix}{}", "a".repeat(2048)))
            .unwrap();
        let before = doc.work();
        for _ in 0..1000 {
            doc.append(&"a".repeat(1000)).unwrap();
        }
        assert_eq!(doc.work().inline_bytes - before.inline_bytes, 1_000_000);
        assert_eq!(doc.work().classified_bytes, before.classified_bytes);
        assert_eq!(doc.work().container_steps, before.container_steps);
        doc.append("\n").unwrap();
        let before = doc.work();
        doc.append(&format!("{prefix}[r]: /url\n")).unwrap();
        assert_eq!(definitions(&doc), 0);
        assert!(doc.work().classified_bytes - before.classified_bytes < 6 * BLOCK_LOOKAHEAD as u64);
        assert!(doc.work().container_steps - before.container_steps < 6000);
        assert!(doc.work().inline_bytes - before.inline_bytes < 32);
        doc.finish();
    }
    let mut doc = Document::new();
    doc.append("- seed\n").unwrap();
    for _ in 0..4096 {
        doc.append("  [r]: /ignored\n").unwrap();
    }
    let before = doc.work();
    doc.append("  [r]: /last\n").unwrap();
    assert_eq!(definitions(&doc), 0);
    assert_eq!(doc.container_count(), 2);
    assert!(doc.work().container_steps - before.container_steps <= 2);
    assert!(doc.work().classified_bytes - before.classified_bytes < 100);
    doc.finish();
}
