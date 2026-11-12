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
fn new_prose_matches_independent_column_model() {
    let mut cases = 0;
    for prefix in ["", " ", "  ", "   ", "> ", ">> ", "- ", "1. "] {
        for whitespace in ["", " ", "  ", "   ", "\t", " \t", "\t ", "\t\t"] {
            let separator = prefix.len() + 1;
            let mut column = separator + 4 - separator % 4;
            for b in whitespace.bytes() {
                column += if b == b'\t' { 4 - column % 4 } else { 1 };
            }
            let indent = column - separator - 1;
            for body in [
                "中🙂 *a* &amp;",
                "[x](u) ![y](v)",
                "-bad",
                "####### text",
                "[^bad] body",
                "`code`",
                "$math$",
                "a\tb",
            ] {
                for ending in ["\n", "\r\n", ""] {
                    let source = format!("{prefix}>\t{whitespace}{body}{ending}");
                    let doc = parse(&source);
                    // This unit covers <=3 columns; larger indentation keeps
                    // the existing code path, whose literal tabs are distinct.
                    if indent <= 3 {
                        let model = parse(&format!(
                            "{prefix}>{}{body}{ending}",
                            " ".repeat(indent + 1)
                        ));
                        assert_eq!(visible(&doc), visible(&model), "{source:?}");
                        assert_eq!(doc.container_count(), model.container_count());
                        for i in 0..doc.container_count() {
                            let a = doc.container(ContainerId(i)).unwrap();
                            let b = model.container(ContainerId(i)).unwrap();
                            assert_eq!(a.kind, b.kind);
                            assert_eq!(a.parent, b.parent);
                        }
                        assert_eq!(doc.blocks().len(), model.blocks().len());
                        for (a, b) in doc.blocks().iter().zip(model.blocks()) {
                            assert_eq!(a.kind, b.kind);
                            assert_eq!(a.container, b.container);
                        }
                        assert_eq!(doc.links().len(), model.links().len());
                        for i in 0..doc.links().len() {
                            assert_eq!(
                                doc.link_destination(LinkId(i)),
                                model.link_destination(LinkId(i))
                            );
                        }
                    }
                    if indent >= 4 {
                        assert!(
                            doc.blocks()
                                .iter()
                                .any(|b| matches!(b.kind, BlockKind::Code { .. })),
                            "{source:?}"
                        );
                        assert!(
                            visible(&doc).ends_with(&format!(
                                "{body}{}",
                                if ending.is_empty() { "" } else { "\n" }
                            )),
                            "{source:?}"
                        );
                    }
                    assert_eq!(doc.source(), source);
                    cases += 1;
                }
            }
        }
    }
    assert_eq!(cases, 1536);
}

#[test]
fn blank_fence_table_and_window_boundaries_are_explicit() {
    for (source, expected) in [
        (">\ttext\n", "  text\n"),
        (">\t text\n", "   text\n"),
        (">\t  text\n", "text\n"),
        (">\t", "  "),
        (">\t \n", "   \n"),
        ("> ```\n>\ttext\n> ```\n", "  text\n"),
        ("> $$\n>\ttext\n> $$\n", "  text\n"),
    ] {
        assert_eq!(visible(&parse(source)), expected, "{source:?}");
    }
    for source in [">\ta | b\n> - | -\n", ">\ta | b\n> nope\n", ">\ta\n> ===\n"] {
        let model = source.replacen(">\t", ">   ", 1);
        let doc = parse(source);
        let model = parse(&model);
        assert_eq!(visible(&doc), visible(&model), "{source:?}");
        assert_eq!(doc.tables().len(), model.tables().len());
        assert_eq!(
            doc.headings().entries().len(),
            model.headings().entries().len()
        );
    }
    for count in [1020, 1021, 1022, 1023] {
        let source = format!("{}>\ttext\n", ">".repeat(count));
        let doc = parse(&source);
        let tab = source.find('\t').unwrap();
        let text: String = doc
            .display_runs_in(Span::new(tab, tab + 1))
            .unwrap()
            .filter(|r| !r.style.contains(Style::HIDDEN))
            .map(|r| r.text)
            .collect();
        let separator = count + 1;
        let residue = 4 - separator % 4 - 1;
        let expected = " ".repeat(residue);
        assert_eq!(text, expected, "count {count}");
    }
}

#[test]
fn identity_mapping_and_shared_budget_preserve_virtual_spaces() {
    let mut doc = Document::new();
    let update = doc.append(">\t*中🙂* &amp;\n").unwrap();
    assert_eq!(update.new_containers.len(), 1);
    let scope = doc.blocks()[0].container;
    let update = doc.append("> tail\n").unwrap();
    assert!(update.new_containers.is_empty());
    assert_eq!(doc.blocks()[1].container, scope);
    let update = doc.finish();
    assert_eq!(update.closed_containers.len(), 1);
    assert_eq!(visible(&doc), "  中🙂 &\ntail\n");
    let tab = doc.source().find('\t').unwrap();
    let runs: Vec<_> = doc
        .display_runs_in(Span::new(tab, tab + 1))
        .unwrap()
        .collect();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].text, "  ");
    assert_eq!(runs[0].replacement, Some(Span::new(tab, tab + 1)));
    assert!(!runs[0].style.contains(Style::HIDDEN));
    assert_eq!(
        doc.runs_in(Span::new(0, doc.source().len()))
            .unwrap()
            .map(|r| r.text)
            .collect::<String>(),
        doc.source()
    );
    let scalar: String = doc
        .source()
        .char_indices()
        .flat_map(|(at, ch)| {
            doc.display_runs_in(Span::new(at, at + ch.len_utf8()))
                .unwrap()
        })
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    assert_eq!(scalar, visible(&doc));
    let tiled: String = (0..doc.tile_count())
        .flat_map(|i| doc.display_runs(TileId(i)).unwrap())
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    assert_eq!(tiled, visible(&doc));
    assert_budget(&doc);
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
fn all_splits_prefixes_and_eof_agree() {
    for source in [
        ">\t 中🙂 *a* &amp;\r\n> tail\r\n",
        ">\t [x](u) ![y](v)\n",
        ">\t *a\n> b*\n",
        ">\t `a\n> b`\n",
        ">\t [a\n> b](u)\n",
        ">\t $a\n> b$\n",
        "- seed\n\t>\t text\n",
        "> seed\n> >\t text\n",
        ">\t a | b\n> - | -\n",
        ">\t a | b\n> no delimiter\n",
        ">\t a\n> ===\n",
        ">\t -bad\n",
        ">\t [^bad] body\n",
        ">\t ####### text\n",
        ">\t  text\n",
        ">\t",
        ">\t ",
        ">\t text",
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
fn deep_chains_and_long_streaming_body_have_bounded_work() {
    for depth in [1, 16, 64, 128] {
        let source = format!("{}body\n", ">\t> ".repeat(depth));
        let doc = parse(&source);
        assert_eq!(doc.container_count(), depth * 2);
        assert_eq!(visible(&doc), "body\n");
        assert!(doc.work().container_bytes < 50 * source.len() as u64);
        assert!(doc.work().classified_bytes < 8 * source.len() as u64);
        assert!(doc.work().container_steps < 8 * source.len() as u64);
    }
    let prefix = ">".repeat(988);
    let mut doc = Document::new();
    doc.append(&format!("{prefix} seed\n")).unwrap();
    let before = doc.work();
    doc.append(&format!("{prefix}>\t {}", "x".repeat(1_000_000)))
        .unwrap();
    assert_eq!(doc.container_count(), 989);
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
    assert!(visible(&doc).ends_with(&"x".repeat(1_001_000)));
}

#[test]
fn long_header_confirmation_fallback_and_eof_consume_residue_once() {
    for prefix in ["", "> ", ">> ", "   "] {
        let separator = prefix.len() + 1;
        let residue = 4 - separator % 4 - 1;
        for ending in [
            "\n> - | -\n",
            " | b\n> - | -\n",
            " | b\n> nope\n",
            " | b",
            "\n",
            "",
        ] {
            // Prefix each subsequent line with the same explicit ancestors.
            let suffix = ending.replace("\n>", &format!("\n{prefix}>"));
            for body in [
                format!("a | {}", "x".repeat(1100)),
                format!("*{}*", "x".repeat(1100)),
            ] {
                let source = format!("{prefix}>\t{body}{suffix}");
                let model = parse(&format!(
                    "{prefix}>{}{body}{suffix}",
                    " ".repeat(residue + 1)
                ));
                let whole = parse(&source);
                assert_budget(&whole);
                assert_eq!(visible(&whole), visible(&model), "{prefix:?}, {ending:?}");
                assert_eq!(whole.tables().len(), model.tables().len());
                let expected = support::snapshot(&whole);
                for split in (0..=source.len()).filter(|&at| source.is_char_boundary(at)) {
                    let mut doc = Document::new();
                    doc.append(&source[..split]).unwrap();
                    doc.append(&source[split..]).unwrap();
                    doc.finish();
                    assert_eq!(support::snapshot(&doc), expected, "split {split}");
                }
                let mut doc = Document::new();
                for (at, ch) in source.char_indices() {
                    doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
                    // Check each prefix around deferred classification/pipe/EOF,
                    // plus regular checkpoints in the streaming body.
                    if at < 12
                        || at % 127 == 0
                        || (1018..1030).contains(&at)
                        || at + 16 >= source.len()
                    {
                        let mut bulk = Document::new();
                        bulk.append(&source[..at + ch.len_utf8()]).unwrap();
                        assert_eq!(support::snapshot(&doc), support::snapshot(&bulk));
                        assert_eq!(doc.pending(), bulk.pending());
                    }
                }
                doc.finish();
                assert_eq!(support::snapshot(&doc), expected);
            }
        }
    }
}
