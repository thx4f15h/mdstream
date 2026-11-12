use mdstream::*;

#[path = "support/snapshot.rs"]
mod support;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn visible(doc: &Document, span: Span) -> String {
    doc.display_runs_in(span)
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect()
}

#[test]
fn underline_column_model_preserves_title_identity_and_scope() {
    let mut cases = 0;
    for (before, prefix, residual) in [
        ("seed\n", "", 0),
        ("> seed\n", ">", 0),
        ("> seed\n", "> ", 0),
        (">> seed\n", ">> ", 0),
        ("> > seed\n", "> > ", 0),
        (">>> seed\n", ">>> ", 0),
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
            // A quote consumes one optional separator column, even when it
            // is the first column of a tab. Other prefixes already include it.
            let separator = usize::from(prefix == ">" && !whitespace.is_empty());
            let indent = residual + column - start - separator;
            for (marker, level) in [('=', 1), ('-', 2)] {
                for count in [1, 3, 9] {
                    for ending in ["\n", "\r\n", ""] {
                        let source = format!(
                            "{before}{prefix}{whitespace}{} \t{ending}",
                            marker.to_string().repeat(count)
                        );
                        let doc = parse(&source);
                        let headings = doc.headings().entries();
                        assert_eq!(headings.len(), usize::from(indent <= 3), "{source:?}");
                        if let Some(heading) = headings.first() {
                            assert_eq!(heading.block, BlockId(0));
                            assert_eq!(heading.level, level);
                            assert_eq!(heading.container, doc.blocks()[0].container);
                            assert_eq!(heading.source, Span::new(0, source.len()));
                            assert_eq!(visible(&doc, heading.content), "seed");
                            assert_eq!(visible(&doc, heading.source), "seed\n");
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
fn precedence_failures_and_complete_shared_window() {
    for (source, headings, rules) in [
        ("> seed\n> \t---\n", 1, 0),
        ("- seed\n\t-\n", 1, 0),
        ("> seed\n>\t===\n", 1, 0),
        ("> seed\n> \t- - -\n", 0, 1),
        ("> \t---\n", 0, 1),
        ("> \t===\n", 0, 0),
        ("> seed\n>\n> \t===\n", 0, 0),
        ("> seed\n> \t==x\n", 0, 0),
        ("> seed\n> \t=-=\n", 0, 0),
        ("> seed\n> \t= =\n", 0, 0),
        ("> seed\n> \t\\===\n", 0, 0),
        ("> seed\n> \t\t===\n", 0, 0),
        ("seed\n\t===\n", 0, 0),
        ("- > seed\n\t===\n", 0, 0),
        ("> seed\n===\n", 0, 0),
        ("> ```\n> seed\n> \t===\n> ```\n", 0, 0),
        ("> $$\n> seed\n> \t===\n> $$\n", 0, 0),
        ("> a | b\n> \t--- | ---\n", 0, 0),
        ("> a | b\n> \t===\n", 1, 0),
        ("- a | b\n\t===\n", 1, 0),
        ("- # heading\n\t===\n", 1, 0),
        ("- seed\n\n\t===\n", 0, 0),
        ("- seed\n-\n", 0, 0),
    ] {
        let doc = parse(source);
        assert_eq!(doc.headings().entries().len(), headings, "{source:?}");
        assert_eq!(
            doc.blocks()
                .iter()
                .filter(|b| b.kind == BlockKind::ThematicBreak)
                .count(),
            rules,
            "{source:?}"
        );
    }
    for depth in 1016..=1020 {
        let prefix = ">".repeat(depth);
        let underline = format!("{prefix} \t===\n");
        let source = format!("{prefix} seed\n{underline}");
        assert_eq!(
            parse(&source).headings().entries().len(),
            usize::from((depth + 1) % 4 != 0),
            "depth {depth}"
        );
    }
}

#[test]
fn all_splits_prefixes_and_eof_agree() {
    for source in [
        "> 中🙂 *first\r\n> last*\n> \t===\r\n> tail\n",
        "- seed\n\t-\n  tail\n- next\n",
        "> seed\n>\t===\n",
        "- - seed\n\t===\n",
        "> seed\n> \t---",
        "> seed\n> \t==x\n> after\n",
        "- > seed\n\t===\n",
        "> a | b\n> \t===\n",
        "> a | b\n>\t===\n",
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
fn late_promotion_keeps_leaves_and_reports_heading() {
    let mut doc = Document::new();
    doc.append("> 中 *first\n> last*\n").unwrap();
    let blocks = doc.blocks().to_vec();
    let mut view = FoldView::<()>::new(&doc, 1).unwrap();
    view.set_defaults(FoldDefaults {
        heading_levels: [true; 6],
        ..Default::default()
    });
    let pending = doc.append("> \t==").unwrap();
    view.apply(&doc, &pending).unwrap();
    assert!(doc.headings().entries().is_empty());
    let update = doc.append("=\n").unwrap();
    assert_eq!(update.new_headings, 0..1);
    assert_eq!(update.new_blocks, 2..2);
    assert_eq!(doc.blocks().len(), blocks.len());
    assert_eq!(doc.blocks()[1].source.start, blocks[1].source.start);
    assert_eq!(doc.blocks()[1].source.end, doc.source().len());
    assert_eq!(doc.blocks()[1].kind, blocks[1].kind);
    assert_eq!(doc.blocks()[1].container, blocks[1].container);
    let heading = doc.headings().entries()[0];
    assert_eq!(heading.block, BlockId(0));
    assert_eq!(heading.container, blocks[0].container);
    assert_eq!(visible(&doc, heading.content), "中 first\nlast");
    view.apply(&doc, &update).unwrap();
    assert!(view.is_collapsed(FoldId::Heading(HeadingId(0))));
}

#[test]
fn deep_million_byte_promotion_and_long_failure_have_bounded_work() {
    let prefix = ">".repeat(998);
    let mut doc = Document::new();
    doc.append(&format!("{prefix} {}", "x".repeat(1_000_000)))
        .unwrap();
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("x").unwrap();
    }
    let after = doc.work();
    assert_eq!(after.inline_bytes - before.inline_bytes, 1000);
    assert_eq!(after.classified_bytes, before.classified_bytes);
    assert_eq!(after.container_steps, before.container_steps);
    assert_eq!(after.heading_steps, before.heading_steps);
    doc.append("\n").unwrap();
    let before = doc.work();
    let update = doc.append(&format!("{prefix} \t===\n")).unwrap();
    assert_eq!(update.new_headings, 0..1);
    assert_eq!(doc.headings().entries()[0].content.len(), 1_001_000);
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert_eq!(doc.work().label_bytes, before.label_bytes);
    assert!(doc.work().classified_bytes - before.classified_bytes < 6 * BLOCK_LOOKAHEAD as u64);
    assert!(doc.work().tree_visits - before.tree_visits < 1000);
    assert!(doc.work().container_steps - before.container_steps < 6_000);

    let mut doc = Document::new();
    doc.append(&format!("> seed\n> \t{}", "=".repeat(1_000_000)))
        .unwrap();
    assert!(doc.headings().entries().is_empty());
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("=").unwrap();
    }
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert_eq!(doc.work().classified_bytes, before.classified_bytes);
    assert_eq!(doc.work().container_steps, before.container_steps);
    doc.append("\n> \t===\n").unwrap();
    assert_eq!(doc.headings().entries().len(), 1);
    assert!(doc.work().inline_bytes <= doc.source().len() as u64);
}

#[test]
fn tab_underline_mapping_search_and_slug_share_six_byte_budget() {
    let source = ">   **中**\r\n> 🙂 &amp; ` a ` &#32; \t\r\n> \t===\n";
    let doc = parse(source);
    let span = Span::new(0, source.len());
    let expected = "中\n🙂 & a  \n";
    assert_eq!(visible(&doc, span), expected);
    assert_eq!(doc.source(), source);
    assert_eq!(
        doc.runs_in(span)
            .unwrap()
            .map(|r| r.text)
            .collect::<String>(),
        source
    );
    let tiled: String = (0..doc.tile_count())
        .flat_map(|i| doc.display_runs(TileId(i)).unwrap())
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    let scalars: String = source
        .char_indices()
        .flat_map(|(at, ch)| {
            doc.display_runs_in(Span::new(at, at + ch.len_utf8()))
                .unwrap()
        })
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    assert_eq!(tiled, expected);
    assert_eq!(scalars, expected);
    let mut cursor = doc.display_cursor(span).unwrap();
    let mut output = String::new();
    for _ in 0..source.len() * 2 {
        let at = cursor.source_position();
        if at < span.end {
            assert_eq!(
                cursor
                    .next(&doc, &mut Budget::new(WorkLimit::default()))
                    .unwrap(),
                DisplayStep::BudgetExhausted
            );
            assert_eq!(cursor.source_position(), at);
        }
        let mut budget = tiny();
        match cursor.next(&doc, &mut budget).unwrap() {
            DisplayStep::Run(run) => {
                if !run.style.contains(Style::HIDDEN) {
                    output.push_str(run.text);
                }
            }
            DisplayStep::Complete => break,
            DisplayStep::BudgetExhausted => panic!("one scalar fits"),
        }
        assert!(budget.used().bytes <= 6);
    }
    assert_eq!(output, expected);
    assert_eq!(cursor.source_position(), span.end);
    let mut search = HeadingSearch::new(&doc, "a  ", 6).unwrap();
    let mut anchors = HeadingAnchors::new(&doc);
    let mut hits = Vec::new();
    for _ in 0..500 {
        let mut budget = tiny();
        let a = anchors.advance(&doc, &mut budget).unwrap();
        let s = search.advance(&doc, &mut budget, &mut hits).unwrap();
        assert!(
            budget.used().bytes <= 6 && budget.used().queries <= 1 && budget.used().objects <= 1
        );
        if a == Completion::EndOfDocument && s == Completion::EndOfDocument {
            break;
        }
    }
    assert!(anchors.is_complete());
    assert_eq!(hits.len(), 1);
    assert_eq!(doc.text(hits[0].source), Some("a ` &#32;"));
    assert_eq!(
        anchors
            .get(&doc, HeadingId(0))
            .unwrap()
            .unwrap()
            .slug
            .to_string(),
        "中---a--"
    );
}

fn tiny() -> Budget {
    Budget::new(WorkLimit {
        bytes: 6,
        queries: 1,
        objects: 1,
        ..Default::default()
    })
}
