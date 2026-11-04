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

fn assert_budget(doc: &Document) {
    let mut cursor = doc
        .display_cursor(Span::new(0, doc.source().len()))
        .unwrap();
    let mut output = String::new();
    loop {
        let at = cursor.source_position();
        let mut empty = Budget::new(WorkLimit::default());
        if at < doc.source().len() {
            assert!(matches!(
                cursor.next(doc, &mut empty).unwrap(),
                DisplayStep::BudgetExhausted
            ));
            assert_eq!(cursor.source_position(), at);
        }
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 6,
            ..Default::default()
        });
        match cursor.next(doc, &mut budget).unwrap() {
            DisplayStep::Run(run) => {
                assert_eq!(run.source.start, at);
                if !run.style.contains(Style::HIDDEN) {
                    output.push_str(run.text);
                }
            }
            DisplayStep::Complete => break,
            DisplayStep::BudgetExhausted => panic!("one scalar fits"),
        }
        assert!(budget.used().bytes <= 6);
    }
    assert_eq!(output, visible(doc));
}

#[test]
fn new_paragraph_after_blank_or_leaf_keeps_partial_tab_columns() {
    for (source, expected) in [
        ("- seed\n\n\t中🙂 *a* &amp;\n", "seed\n\n  中🙂 a &\n"),
        ("1. seed\n\n\ttext\n", "seed\n\n text\n"),
        ("- # title\n\ttext\n", "title\n  text\n"),
        ("- ```\n  code\n  ```\n\ttext\n", "code\n  text\n"),
        ("- [r]: /url\n\t[r]\n", "  r\n"),
        ("- - - seed\n\n\t\ttext\n", "seed\n\n  text\n"),
        ("> # title\n>\ttext\n", "title\n  text\n"),
    ] {
        let doc = parse(source);
        assert_eq!(visible(&doc), expected, "{source:?}");
        assert_eq!(
            doc.blocks().first().unwrap().container,
            doc.blocks().last().unwrap().container
        );
        assert_eq!(doc.source(), source);
        assert_budget(&doc);
    }
}

#[test]
fn new_paragraphs_match_independent_physical_column_model() {
    let mut cases = 0;
    for parent in ["", "> ", ">> ", "   > "] {
        for depth in 1..=8 {
            for leaf in ["seed\n", "# title\n", "[r]: /url\n"] {
                let opener = format!("{parent}{}{leaf}", "- ".repeat(depth));
                // A blank with full ancestry ends prose without closing its scope.
                let blank = format!("{parent}{}\n", " ".repeat(depth * 2));
                for leading in 0..4 {
                    let target = parent.len() + depth * 2;
                    let mut column = parent.len() + leading;
                    let mut prefix = format!("{parent}{}", " ".repeat(leading));
                    while column < target {
                        prefix.push('\t');
                        column += 4 - column % 4;
                    }
                    for body in ["中🙂 *a* &amp;", "[r] ![i](v)", "-bad", "a\tb"] {
                        let source = format!("{opener}{blank}{prefix}{body}\r\n");
                        let model_source = format!(
                            "{opener}{blank}{parent}{}{body}\r\n",
                            " ".repeat(column - parent.len())
                        );
                        let doc = parse(&source);
                        let model = parse(&model_source);
                        assert_eq!(visible(&doc), visible(&model), "{source:?}");
                        assert_eq!(doc.container_count(), model.container_count());
                        for i in 0..doc.container_count() {
                            let a = doc.container(ContainerId(i)).unwrap();
                            let b = model.container(ContainerId(i)).unwrap();
                            assert_eq!(
                                (a.kind, a.parent, a.list_tightness),
                                (b.kind, b.parent, b.list_tightness)
                            );
                        }
                        assert_eq!(doc.blocks().len(), model.blocks().len());
                        for (a, b) in doc.blocks().iter().zip(model.blocks()) {
                            assert_eq!((&a.kind, a.container), (&b.kind, b.container));
                        }
                        assert_eq!(doc.links().len(), model.links().len());
                        assert!(doc.work().container_bytes < 16 * source.len() as u64);
                        assert!(doc.work().container_steps < 4 * source.len() as u64);
                        cases += 1;
                    }
                }
            }
        }
    }
    assert_eq!(cases, 1536);
}

#[test]
fn mapping_identity_and_budget_cover_a_new_paragraph() {
    let mut doc = Document::new();
    doc.append("- seed\n\n").unwrap();
    let scope = doc.blocks()[0].container;
    let update = doc.append("\t*中🙂* &amp;\n").unwrap();
    assert!(update.new_containers.is_empty());
    assert_eq!(doc.blocks().last().unwrap().container, scope);
    doc.finish();
    let tab = doc.source().find('\t').unwrap();
    let runs: Vec<_> = doc
        .display_runs_in(Span::new(tab, tab + 1))
        .unwrap()
        .collect();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].text, "  ");
    assert_eq!(runs[0].replacement, Some(Span::new(tab, tab + 1)));
    assert!(!runs[0].style.contains(Style::HIDDEN));
    let raw: String = doc
        .runs_in(Span::new(0, doc.source().len()))
        .unwrap()
        .map(|r| r.text)
        .collect();
    assert_eq!(raw, doc.source());
    let tiled: String = (0..doc.tile_count())
        .flat_map(|i| doc.display_runs(TileId(i)).unwrap())
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    let scalars: String = doc
        .source()
        .char_indices()
        .flat_map(|(at, ch)| {
            doc.display_runs_in(Span::new(at, at + ch.len_utf8()))
                .unwrap()
        })
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    assert_eq!(tiled, visible(&doc));
    assert_eq!(scalars, visible(&doc));
    assert_budget(&doc);
}

#[test]
fn splits_prefixes_eof_and_table_boundaries_are_stable() {
    for source in [
        "- seed\n\n\t中🙂 *a* &amp;\r\n",
        "- # title\n\t[a\n  b](u)\n",
        "- seed\n\n\t` a\n  b `\n",
        "- seed\n\n\t$a\n  b$\n",
        "- seed\n\n\ta | b\n  - | -\n",
        "- seed\n\n\ta | b\n  nope\n",
        "- a|b\n\t-|-\n",
        "- parent\n\t- first\n\t- a|b\n\t  -|-\n",
        "- seed\n\n\ta\n  ===\n",
        "- seed\n\n\t",
        "- > seed\n\n\ttail\n",
        "- seed\n\n\t  code\n",
        "- seed\n\n\t\tcode\n",
        "- seed\n\n\ttext",
    ] {
        let whole = parse(source);
        // Complete partial-tab paths admit headers and ordinary delimiters.
        assert_eq!(
            whole.tables().len(),
            usize::from(matches!(
                source,
                "- seed\n\n\ta | b\n  - | -\n"
                    | "- a|b\n\t-|-\n"
                    | "- parent\n\t- first\n\t- a|b\n\t  -|-\n"
            )),
            "{source:?}"
        );
        let expected = support::snapshot(&whole);
        for split in (0..=source.len()).filter(|&at| source.is_char_boundary(at)) {
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
            let text = &source[..at + ch.len_utf8()];
            let mut bulk = Document::new();
            bulk.append(text).unwrap();
            assert_eq!(support::snapshot(&doc), support::snapshot(&bulk));
            assert_eq!(doc.pending(), bulk.pending());
            let mut eof = Document::new();
            for ch in text.chars() {
                eof.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            }
            eof.finish();
            assert_eq!(support::snapshot(&eof), support::snapshot(&parse(text)));
        }
        assert_budget(&whole);
    }
    for (source, expected) in [
        ("- seed\n\n\t\n", "seed\n\n  \n"),
        ("- > seed\n\n\ttail\n", "seed\n\n  tail\n"),
        ("- > seed\n\ttail\n", "seed\n  tail\n"),
        ("- seed\n\n\t  code\n", "seed\n\ncode\n"),
    ] {
        assert_eq!(visible(&parse(source)), expected, "{source:?}");
    }
}

#[test]
fn deep_new_paragraph_and_long_pipe_text_do_not_rescan() {
    let prefix = ">".repeat(980);
    let mut doc = Document::new();
    doc.append(&format!("{prefix}- seed\n{prefix}\n")).unwrap();
    let before = doc.work();
    doc.append(&format!("{prefix}\t{}", "x".repeat(1_000_000)))
        .unwrap();
    assert_eq!(doc.container_count(), 982);
    assert!(doc.work().classified_bytes - before.classified_bytes < 8 * BLOCK_LOOKAHEAD as u64);
    assert!(doc.work().container_steps - before.container_steps < 6000);
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("x").unwrap();
    }
    assert_eq!(doc.work().classified_bytes, before.classified_bytes);
    assert_eq!(doc.work().container_steps, before.container_steps);
    assert_eq!(doc.work().container_bytes, before.container_bytes);
    assert_eq!(doc.work().inline_bytes - before.inline_bytes, 1000);
    doc.finish();
    assert!(visible(&doc).ends_with(&format!(" {}", "x".repeat(1_001_000))));
    for body in [
        format!("a | {}", "x".repeat(1100)),
        format!("{} | b", "x".repeat(1100)),
    ] {
        for ending in ["", "\n  - | -\n", "\n  nope\n"] {
            let source = format!("- seed\n\n\t{body}{ending}");
            let expected = parse(&source);
            let confirmed = ending == "\n  - | -\n";
            assert_eq!(expected.tables().len(), usize::from(confirmed));
            if !confirmed {
                assert!(visible(&expected).starts_with(&format!("seed\n\n  {body}")));
            }
            assert_budget(&expected);
            for split in 0..=source.len() {
                let mut doc = Document::new();
                doc.append(&source[..split]).unwrap();
                doc.append(&source[split..]).unwrap();
                doc.finish();
                assert_eq!(support::snapshot(&doc), support::snapshot(&expected));
            }
        }
    }
}

#[test]
fn classification_window_requires_nonblank_payload_before_committing() {
    for depth in [1000, 1020, 1021, 1022, 1023] {
        let prefix = ">".repeat(depth);
        let source = format!("{prefix} seed\n{prefix}\n{prefix}\ttext\n");
        let doc = parse(&source);
        let tab = source.find('\t').unwrap();
        let shown: String = doc
            .display_runs_in(Span::new(tab, tab + 1))
            .unwrap()
            .filter(|r| !r.style.contains(Style::HIDDEN))
            .map(|r| r.text)
            .collect();
        let residue = 4 - depth % 4 - 1;
        let expected = if depth + 1 < BLOCK_LOOKAHEAD || residue == 0 {
            " ".repeat(residue)
        } else {
            "\t".to_owned()
        };
        assert_eq!(shown, expected, "depth {depth}");
    }
}
