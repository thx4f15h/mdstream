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

fn quotes(doc: &Document) -> usize {
    (0..doc.container_count())
        .filter(|&i| {
            matches!(
                doc.container(ContainerId(i)).unwrap().kind,
                ContainerKind::Quote
            )
        })
        .count()
}

#[test]
fn new_nested_quote_matches_independent_column_model() {
    let mut cases = 0;
    for prefix in ["", " ", "  ", "   ", "> ", ">> ", "- ", "1. "] {
        for whitespace in ["", " ", "  ", "   ", "\t", " \t", "\t ", "\t\t"] {
            let separator = prefix.len() + 1;
            let mut column = separator + 4 - separator % 4;
            for b in whitespace.bytes() {
                column += if b == b'\t' { 4 - column % 4 } else { 1 };
            }
            let indent = column - separator - 1;
            for body in ["", "中🙂", "*a* &amp;", "# title", "- item", "```"] {
                for ending in ["\n", "\r\n", ""] {
                    let source = format!("{prefix}>\t{whitespace}> {body}{ending}");
                    let doc = parse(&source);
                    assert_eq!(
                        quotes(&doc),
                        prefix.matches('>').count() + 1 + usize::from(indent <= 3),
                        "{source:?}"
                    );
                    if indent <= 3 {
                        let model = parse(&format!(
                            "{prefix}>{}> {body}{ending}",
                            " ".repeat(indent + 1)
                        ));
                        assert_eq!(visible(&doc), visible(&model), "{source:?}");
                        assert_eq!(doc.container_count(), model.container_count());
                        assert_eq!(doc.blocks().len(), model.blocks().len());
                        for (a, b) in doc.blocks().iter().zip(model.blocks()) {
                            assert_eq!(a.kind, b.kind, "{source:?}");
                            assert_eq!(a.container, b.container);
                        }
                    }
                    assert_eq!(doc.source(), source);
                    cases += 1;
                }
            }
        }
    }
    assert_eq!(cases, 1152);
}

#[test]
fn all_splits_prefixes_and_eof_agree() {
    for source in [
        ">\t > 中🙂 *a* &amp;\r\n> > tail\r\n",
        ">\t >\t> # head\n> > > next\n",
        "- seed\n\t>\t > body\n",
        "> seed\n> >\t > body\n",
        "> a | b\n> >\t > body\n",
        ">\t > - [x] task\n> >   next\n",
        ">\t > ```\n> > raw\n> > ```\n",
        ">\t > $$\n> > x\n> > $$\n",
        ">\t > [^n]: body\n> >     next\n",
        ">\t > [r]: /url\n> > [r]\n",
        ">\t >\t> ",
        ">\t  > code\n",
        ">\t >",
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
fn failures_window_and_literal_contexts_keep_their_dialect() {
    assert_eq!(quotes(&parse(">\t >x\n")), 2);
    let ordinary = parse(">\tordinary\n");
    assert_eq!(visible(&ordinary), "  ordinary\n");
    for source in [
        "> ```\n>\t> body\n> ```\n",
        "> $$\n>\t> body\n> $$\n",
        ">\t  > body\n",
    ] {
        assert_eq!(quotes(&parse(source)), 1, "{source:?}");
    }
    for depth in 1016..=1024 {
        let source = format!("{}>\t > body\n", ">".repeat(depth));
        let doc = parse(&source);
        let indent = 4 - (depth + 1) % 4; // Tab residue plus one real space.
        let expected = (depth + 1).min(BLOCK_LOOKAHEAD)
            + usize::from(depth + 4 <= BLOCK_LOOKAHEAD && indent <= 3);
        assert_eq!(quotes(&doc), expected, "depth {depth}");
        if depth == 1021 {
            // The next marker is the first byte outside the window. It must
            // not commit even the outer quote's speculative tab separator.
            let tab = source.find('\t').unwrap();
            assert!(
                doc.display_runs_in(Span::new(tab, tab + 1))
                    .unwrap()
                    .all(|r| !r.style.contains(Style::HIDDEN))
            );
        }
    }
}

#[test]
fn identity_source_mapping_and_shared_budget_survive_continuation() {
    let mut doc = Document::new();
    let update = doc.append(">\t > *中🙂* &amp;\n").unwrap();
    assert_eq!(update.new_containers.len(), 2);
    let inner = doc.blocks()[0].container.unwrap();
    assert_eq!(doc.container(inner).unwrap().parent, Some(ContainerId(0)));
    let update = doc.append("> > tail\n").unwrap();
    assert!(update.new_containers.is_empty());
    assert_eq!(doc.blocks()[1].container, Some(inner));
    let update = doc.finish();
    assert_eq!(update.closed_containers.len(), 2);
    assert_eq!(visible(&doc), "中🙂 &\ntail\n");
    let tab = doc.source().find('\t').unwrap();
    assert!(
        doc.display_runs_in(Span::new(tab, tab + 1))
            .unwrap()
            .all(|r| r.style.contains(Style::HIDDEN))
    );
    assert_eq!(
        doc.runs_in(Span::new(0, doc.source().len()))
            .unwrap()
            .map(|r| r.text)
            .collect::<String>(),
        doc.source()
    );
    let mut cursor = doc
        .display_cursor(Span::new(0, doc.source().len()))
        .unwrap();
    let mut output = String::new();
    loop {
        let at = cursor.source_position();
        let mut empty = Budget::new(WorkLimit::default());
        if at < doc.source().len() {
            assert!(matches!(
                cursor.next(&doc, &mut empty).unwrap(),
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
        match cursor.next(&doc, &mut budget).unwrap() {
            DisplayStep::Run(run) => {
                assert!(run.source.start >= at);
                if !run.style.contains(Style::HIDDEN) {
                    output.push_str(run.text);
                }
            }
            DisplayStep::Complete => break,
            DisplayStep::BudgetExhausted => panic!("one scalar fits"),
        }
        assert!(budget.used().bytes <= 6);
    }
    assert_eq!(output, visible(&doc));
}

#[test]
fn deep_chains_and_long_streaming_body_have_bounded_work() {
    for depth in [1, 16, 128, 330] {
        let source = format!("{}> body\n", ">\t ".repeat(depth));
        let doc = parse(&source);
        assert_eq!(quotes(&doc), depth + 1);
        assert_eq!(visible(&doc), "body\n");
        assert!(doc.work().container_bytes < 40 * source.len() as u64);
        assert!(doc.work().classified_bytes < 8 * source.len() as u64);
        assert!(doc.work().container_steps < 8 * source.len() as u64);
    }
    let prefix = ">".repeat(988);
    let mut doc = Document::new();
    doc.append(&format!("{prefix} seed\n")).unwrap();
    let before = doc.work();
    doc.append(&format!("{prefix}>\t > {}", "x".repeat(1_000_000)))
        .unwrap();
    assert_eq!(quotes(&doc), 990);
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
