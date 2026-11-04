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
                    assert_eq!(replacement, run.source);
                    assert_eq!(doc.text(replacement), Some("\t"));
                    let line_start = doc.source()[..replacement.start]
                        .rfind('\n')
                        .map_or(0, |i| i + 1);
                    let column = replacement.start - line_start;
                    assert_eq!(run.text, " ".repeat(3 - column % 4));
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
fn empty_quotes_publish_blank_leaves_with_original_scope_and_source() {
    let source = ">\n>  \n> \n>\t\n";
    let doc = parse(source);
    assert_eq!(doc.container_count(), 1);
    assert_eq!(doc.blocks().len(), 4);
    let mut start = 0;
    for (block, line) in doc.blocks().iter().zip(source.split_inclusive('\n')) {
        assert_eq!(block.kind, BlockKind::Blank);
        assert_eq!(block.container, Some(ContainerId(0)));
        assert_eq!(block.source, Span::new(start, start + line.len()));
        assert_eq!(block.state, SyntaxState::Sealed);
        start += line.len();
    }
    assert_eq!(
        doc.container(ContainerId(0)).unwrap().source,
        Span::new(0, source.len())
    );
    // Blank is a structural classification, not whitespace erasure. The
    // optional separator consumes one column; an existing partial-tab path
    // maps its residual columns to the original tab byte.
    assert_eq!(visible(&doc), "\n \n\n  \n");
    check_reads(&doc);
}

#[test]
fn blank_quote_matrix_preserves_nested_identity_and_physical_lines() {
    let mut cases = 0;
    for outer in ["", "- ", "1. ", "   "] {
        for depth in 1..=12 {
            for gap in ["", " ", "  ", "\t", " \t ", "    ", "\t\t", " \t\t "] {
                for end in ["\n", "\r\n", ""] {
                    let prefix = format!("{outer}{}", ">".repeat(depth));
                    let source = format!("{prefix}{gap}{end}");
                    let doc = parse(&source);
                    assert_eq!(doc.blocks().len(), 1, "{source:?}");
                    assert_eq!(doc.blocks()[0].kind, BlockKind::Blank, "{source:?}");
                    let extra = if outer == "- " || outer == "1. " {
                        2
                    } else {
                        0
                    };
                    assert_eq!(doc.container_count(), depth + extra);
                    let scope = ContainerId(depth + extra - 1);
                    assert_eq!(doc.blocks()[0].container, Some(scope));
                    assert_eq!(doc.container(scope).unwrap().kind, ContainerKind::Quote);
                    assert_eq!(doc.blocks()[0].source, Span::new(0, source.len()));
                    check_reads(&doc);
                    cases += 1;
                }
            }
        }
    }
    assert_eq!(cases, 1152);
}

#[test]
fn blanks_break_inline_lazy_and_setext_sequences_without_reopening_quotes() {
    for source in [
        "> *a\n>\n> b*\n",
        "> `a\n> \n> b`\n",
        "> [a\n>  \n> b](url)\n",
        "> $a\n>\t\n> b$\n",
    ] {
        let doc = parse(source);
        assert_eq!(doc.blocks()[1].kind, BlockKind::Blank);
        assert_eq!(doc.container_count(), 1);
        assert!(doc.links().is_empty());
        assert!(doc.maths().is_empty());
        for run in doc.display_runs_in(Span::new(0, source.len())).unwrap() {
            for style in [
                Style::EMPHASIS,
                Style::CODE,
                Style::MATH,
                Style::SOFT_BREAK,
                Style::HARD_BREAK,
            ] {
                assert!(!run.style.contains(style), "{source:?}: {run:?}");
            }
        }
        check_reads(&doc);
    }
    let doc = parse("> foo\n>\nbar\n");
    assert_eq!(doc.blocks()[1].kind, BlockKind::Blank);
    assert_eq!(doc.blocks()[2].container, None);
    let doc = parse("> foo\n>\n> ---\n");
    assert!(doc.headings().entries().is_empty());
    assert_eq!(doc.blocks()[2].kind, BlockKind::ThematicBreak);
    let doc = parse("> foo\n>\n> bar\n> baz\n");
    assert_eq!(doc.container_count(), 1);
    assert!(
        doc.display_runs_in(doc.blocks()[0].source)
            .unwrap()
            .all(|r| !r.style.contains(Style::SOFT_BREAK))
    );
    assert!(
        doc.display_runs_in(doc.blocks()[2].source)
            .unwrap()
            .any(|r| r.style.contains(Style::SOFT_BREAK))
    );
}

#[test]
fn blank_quotes_preserve_list_spacing_code_and_pending_header_boundaries() {
    for (source, tightness) in [
        ("- > a\n  >\n  > b\n", ListTightness::Tight),
        ("- > a\n  >\n- b\n", ListTightness::Tight),
        ("- > a\n  >\n", ListTightness::Tight),
    ] {
        let doc = parse(source);
        assert_eq!(doc.blocks()[1].kind, BlockKind::Blank);
        assert_eq!(
            doc.container(ContainerId(0)).unwrap().list_tightness,
            Some(tightness)
        );
    }
    // Plain marker-only items also publish blank leaves; their Item remains.
    assert_eq!(parse("-\n").blocks()[0].kind, BlockKind::Blank);
    for source in ["> ```\n>\n> ```\n", "> $$\n>\n> $$\n"] {
        let doc = parse(source);
        assert_eq!(doc.blocks().len(), 1);
        assert!(!matches!(doc.blocks()[0].kind, BlockKind::Blank));
    }
    let doc = parse("> a | b\n>\n> --|--\n");
    assert!(doc.tables().is_empty());
    assert_eq!(doc.blocks()[1].kind, BlockKind::Blank);
    let doc = parse(">     code\n>\n>     next\n");
    assert_eq!(doc.blocks().len(), 3);
    assert_eq!(doc.blocks()[1].kind, BlockKind::Blank);
    assert!(matches!(doc.blocks()[2].kind, BlockKind::Code { .. }));
}

#[test]
fn quote_blanks_are_stable_at_every_split_prefix_and_eof() {
    for source in [
        ">\n>  \r\n>\t",
        "> *中🙂\n> \n> 文*\n",
        "> > a\n> >\n> b\nroot\n",
        "- > a\n  >\n- b\n",
        "> a | b\n>\t\n> --|--\n",
        "> ```\n>\n> ```\n>\n",
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
fn blank_classification_is_bounded_and_does_not_rescan_old_text_or_ancestors() {
    // An incomplete classification window keeps its existing prose/code kind;
    // a later LF or finish cannot reinterpret a previously published long line.
    for end in ["", "\n"] {
        let short = parse(&format!(">{}{end}", " ".repeat(BLOCK_LOOKAHEAD - 2)));
        assert_eq!(short.blocks()[0].kind, BlockKind::Blank);
        let long = parse(&format!(">{}{end}", " ".repeat(BLOCK_LOOKAHEAD)));
        assert!(!matches!(long.blocks()[0].kind, BlockKind::Blank));
    }
    let prefix = ">".repeat(980);
    let mut doc = Document::new();
    doc.append(&format!("{prefix} {}\n", "x".repeat(1_000_000)))
        .unwrap();
    let before = doc.work();
    for _ in 0..100 {
        doc.append(&format!("{prefix}\n")).unwrap();
    }
    assert_eq!(doc.container_count(), 980);
    assert!(doc.blocks()[1..].iter().all(|b| b.kind == BlockKind::Blank));
    assert!(doc.work().container_steps - before.container_steps < 200_000);
    assert!(doc.work().container_bytes - before.container_bytes < 800_000);
    assert!(doc.work().classified_bytes - before.classified_bytes < 400_000);
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
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
