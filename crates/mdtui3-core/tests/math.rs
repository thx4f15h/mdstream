use mdtui3::*;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn contents(doc: &Document) -> Vec<(MathKind, &str)> {
    doc.maths()
        .iter()
        .map(|math| (math.kind, doc.text(math.content).unwrap()))
        .collect()
}

fn visible(doc: &Document) -> String {
    doc.display_runs_in(Span::new(0, doc.source().len()))
        .unwrap()
        .filter(|run| !run.style.contains(Style::HIDDEN))
        .map(|run| run.text)
        .collect()
}

#[test]
fn exact_runs_raw_content_and_explicit_dialect() {
    for (source, expected) in [
        ("$x$", vec![(MathKind::Inline, "x")]),
        ("$$中🙂 + x_1$$", vec![(MathKind::Display, "中🙂 + x_1")]),
        ("$ x $", vec![(MathKind::Inline, " x ")]),
        ("$5 and $10", vec![(MathKind::Inline, "5 and ")]),
        ("$$a$b$$", vec![(MathKind::Display, "a$b")]),
        ("$a$$b$", vec![(MathKind::Inline, "a$$b")]),
        ("$a$$$b$", vec![(MathKind::Inline, "a$$$b")]),
        ("$$$a$$$", vec![]),
        ("$$$$", vec![]),
        (r"\(x\) \[x\]", vec![]),
        (
            "$x$ $$y$$ $z$",
            vec![
                (MathKind::Inline, "x"),
                (MathKind::Display, "y"),
                (MathKind::Inline, "z"),
            ],
        ),
    ] {
        let doc = parse(source);
        assert_eq!(contents(&doc), expected, "{source:?}");
        let mut previous = 0;
        for math in doc.maths() {
            assert!(math.source.start >= previous);
            let width = if math.kind == MathKind::Inline { 1 } else { 2 };
            assert_eq!(
                math.content,
                Span::new(math.source.start + width, math.source.end - width)
            );
            assert!(
                doc.runs_in(math.content)
                    .unwrap()
                    .all(|run| run.style.contains(Style::MATH))
            );
            previous = math.source.end;
        }
    }
    let doc = parse(r"$**bold** `code` &amp; [x](u) \alpha_1$");
    assert!(doc.links().is_empty());
    assert_eq!(visible(&doc), r"**bold** `code` &amp; [x](u) \alpha_1");
    for run in doc.runs_in(doc.math(MathId(0)).unwrap().content).unwrap() {
        assert_eq!(run.style, Style::MATH);
    }
}

#[test]
fn escapes_are_local_and_opaque_body_preserves_backslashes() {
    for (source, expected) in [
        (r"\$x\$", vec![]),
        (r"\\$x$", vec![(MathKind::Inline, "x")]),
        (r"$a\$b$", vec![(MathKind::Inline, r"a\$b")]),
        (r"$a\\$", vec![(MathKind::Inline, r"a\\")]),
        // Escaping consumes one byte, including only the first dollar of a run.
        (r"$$a\$$b$$", vec![(MathKind::Display, r"a\$$b")]),
        (r"$a\$$", vec![(MathKind::Inline, r"a\$")]),
    ] {
        assert_eq!(contents(&parse(source)), expected, "{source}");
    }
}

#[test]
fn existing_code_links_entities_and_outer_styles_compose() {
    let doc = parse(
        "`$code$` [label](/$target$ '$title$') <https://x/$auto$> &$no$; **$bold$** [$label$](u)\n\n```\n$block$\n```\n\n    $indent$\n",
    );
    assert_eq!(
        contents(&doc),
        vec![
            (MathKind::Inline, "no"),
            (MathKind::Inline, "bold"),
            (MathKind::Inline, "label")
        ]
    );
    let bold = doc.math(MathId(1)).unwrap();
    assert!(
        doc.runs_in(bold.content)
            .unwrap()
            .all(|run| run.style.contains(Style::STRONG))
    );
    let label = doc.math(MathId(2)).unwrap();
    assert!(
        doc.runs_in(label.content)
            .unwrap()
            .all(|run| run.link.is_some())
    );
    assert_eq!(doc.link_destination(LinkId(0)), Some("/$target$"));
    assert_eq!(doc.link_title(LinkId(0)), Some("$title$"));
    // A failed link suffix consumes its already-read dollars without replay.
    assert!(parse("[x](bad$inside$ target)\n").maths().is_empty());
}

#[test]
fn paragraph_newlines_and_structural_boundaries_are_explicit() {
    for (source, expected) in [
        ("$$a\r\nb$$\n", vec![(MathKind::Display, "a\r\nb")]),
        ("$a\nb$", vec![(MathKind::Inline, "a\nb")]),
        ("$a\n\nb$\n", vec![]),
        ("$a\n# b$\n", vec![]),
        ("> $a\n> b$\n", vec![(MathKind::Inline, "a\n> b")]),
        ("> - $inner$\n", vec![(MathKind::Inline, "inner")]),
        ("# $$heading$$\n", vec![(MathKind::Display, "heading")]),
        ("$title$\n---\n", vec![(MathKind::Inline, "title")]),
        ("$a\n---\nb$\n", vec![]),
    ] {
        assert_eq!(contents(&parse(source)), expected, "{source:?}");
    }
    let doc = parse("| $h$ | h2 |\n| -- | -- |\n| $$x$$ | $a |\n| b$ | $c\\|d$ |\n");
    assert_eq!(
        contents(&doc),
        vec![
            (MathKind::Inline, "h"),
            (MathKind::Display, "x"),
            (MathKind::Inline, r"c\|d")
        ]
    );
    assert_eq!(doc.tables().len(), 1);
}

#[test]
fn failed_and_unclosed_candidates_remain_literal_without_replay() {
    for tail in [
        "$**not strong** &amp; `code`",
        "$$[not](link)$",
        "$",
        "$$",
        "$\\",
    ] {
        let source = format!("{} {tail}", "p".repeat(BLOCK_LOOKAHEAD));
        let mut doc = Document::new();
        doc.append(&source).unwrap();
        assert!(doc.maths().is_empty());
        let before = doc.work();
        doc.finish();
        assert_eq!(doc.work().inline_bytes, before.inline_bytes);
        assert!(doc.maths().is_empty());
        assert!(doc.links().is_empty());
        assert_eq!(visible(&doc), source);
        assert!(doc.finish().new_maths.is_empty());
    }
    // Once a paragraph ends, the next paragraph has a fresh candidate state.
    let doc = parse("$failed\n\n$works$\n");
    assert_eq!(contents(&doc), vec![(MathKind::Inline, "works")]);
}

#[test]
fn maximal_runs_wait_for_lookahead_and_updates_report_confirmation_once() {
    let mut doc = Document::new();
    doc.append(&"p".repeat(BLOCK_LOOKAHEAD)).unwrap();
    doc.append("$x$").unwrap();
    assert!(doc.maths().is_empty());
    doc.append("$").unwrap(); // That potential closer is now a mismatched run.
    doc.append("y").unwrap();
    assert!(doc.maths().is_empty());
    let update = doc.append("$ ").unwrap();
    assert_eq!(update.new_maths, 0..1);
    assert_eq!(contents(&doc), vec![(MathKind::Inline, "x$$y")]);
    let saved = *doc.math(MathId(0)).unwrap();
    assert_eq!(doc.append("").unwrap().new_maths, 1..1);
    let second = doc.append("$$z$$").unwrap();
    assert_eq!(second.new_maths, 1..1);
    let eof = doc.finish();
    assert_eq!(eof.new_maths, 1..2);
    assert_eq!(*doc.math(MathId(0)).unwrap(), saved);
    assert_eq!(doc.finish().new_maths, 2..2);
    assert!(doc.append("more").is_err());

    let mut doc = Document::new();
    let mut batch = Update::default();
    let mut scratch = Update::default();
    for chunk in ["$a$\n", "$$b$$\n", "text $c$"] {
        doc.append_into(chunk, &mut scratch).unwrap();
        batch.merge(scratch.clone()).unwrap();
    }
    doc.finish_into(&mut scratch);
    batch.merge(scratch).unwrap();
    assert_eq!(batch.new_maths, 0..3);
    assert_eq!(doc.maths().len(), 3);
}

#[test]
fn late_confirmation_invalidates_old_tiles_and_layout_even_without_style_sizing() {
    let mut doc = Document::new();
    let first = doc.append(&format!("${}", "x".repeat(10_000))).unwrap();
    let old = doc.tile_key(TileId(0)).unwrap();
    let mut heights = HeightIndex::default();
    heights.apply(&doc, &first).unwrap();
    let ticket = heights.ticket(TileId(0)).unwrap();
    let before = doc.work();
    let update = doc.append("$ ").unwrap();
    assert_eq!(doc.work().inline_bytes - before.inline_bytes, 2);
    assert_eq!(doc.work().math_steps - before.math_steps, 1);
    assert!(doc.work().tree_visits - before.tree_visits < 1000);
    assert_ne!(doc.tile_key(TileId(0)).unwrap(), old);
    assert!(update.dirty.iter().any(|span| span.start == 0));
    assert!(
        update
            .changes
            .iter()
            .any(|c| c.kind == ChangeKind::Structure && c.source.start == 0)
    );
    heights
        .apply_with_policy(
            &doc,
            &update,
            LayoutPolicy {
                style_affects_size: false,
                ..LayoutPolicy::default()
            },
        )
        .unwrap();
    assert!(heights.measure(ticket, 3).is_err());
}

#[test]
fn budgeted_raw_reader_and_metadata_share_limits_and_resume_atomically() {
    let doc = parse("text $$中🙂\\alpha &amp;\r\nx$$ end $z$");
    let mut objects = doc.maths_in(Span::new(0, doc.source().len())).unwrap();
    let mut none = Budget::new(WorkLimit::default());
    assert_eq!(objects.advance(&mut none), Err(Completion::BudgetExhausted));
    assert_eq!(none.used(), WorkLimit::default());
    let mut frame = Budget::new(WorkLimit {
        queries: 2,
        objects: 2,
        bytes: 4,
        ..WorkLimit::default()
    });
    let (id, math) = objects.advance(&mut frame).unwrap().unwrap();
    assert_eq!(id, MathId(0));
    let mut reader = doc.math_content(id).unwrap();
    let first = reader.advance(&mut frame).unwrap().unwrap();
    assert_eq!(first.text, "中");
    assert_eq!(frame.used().bytes, 3);
    assert_eq!(frame.used().objects, 2);
    let position = reader.source_position();
    assert_eq!(reader.advance(&mut frame), Err(Completion::BudgetExhausted));
    assert_eq!(reader.source_position(), position);
    for limit in [
        WorkLimit {
            queries: 1,
            bytes: 4,
            ..WorkLimit::default()
        },
        WorkLimit {
            objects: 1,
            bytes: 4,
            ..WorkLimit::default()
        },
        WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 3,
            ..WorkLimit::default()
        },
    ] {
        let mut budget = Budget::new(limit);
        assert_eq!(
            reader.advance(&mut budget),
            Err(Completion::BudgetExhausted)
        );
        assert_eq!(reader.source_position(), position);
        assert_eq!(budget.used(), WorkLimit::default());
    }
    let mut out = first.text.to_owned();
    let mut bytes = first.text.len();
    loop {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 4,
            ..WorkLimit::default()
        });
        let Some(chunk) = reader.advance(&mut budget).unwrap() else {
            break;
        };
        assert_eq!(doc.text(chunk.source), Some(chunk.text));
        assert!(chunk.text.len() <= 4);
        out.push_str(chunk.text);
        bytes += budget.used().bytes;
    }
    assert_eq!(out, doc.text(math.content).unwrap());
    assert_eq!(bytes, math.content.len());
    assert_eq!(reader.advance(&mut none), Ok(None));
    assert!(doc.math_content(MathId(999)).is_none());
    assert!(doc.maths_in(Span::new(9, 1)).is_none());
    assert!(doc.maths_in(Span::new(0, usize::MAX)).is_none());
    assert!(
        doc.maths_in(Span::new(math.content.start + 1, math.content.end))
            .is_none()
    );
}

#[test]
fn viewport_objects_match_linear_intersection_model_with_bounded_steps() {
    let mut source = String::new();
    let mut expected = Vec::new();
    for i in 0..4096 {
        let start = source.len();
        let text = format!("$m{i}$");
        source.push_str(&text);
        expected.push(Span::new(start, source.len()));
        source.push(' ');
    }
    let doc = parse(&source);
    assert_eq!(
        doc.maths().iter().map(|m| m.source).collect::<Vec<_>>(),
        expected
    );
    for start in (0..source.len()).step_by(137) {
        let end = (start + 32).min(source.len());
        let want: Vec<_> = expected
            .iter()
            .enumerate()
            .filter(|(_, s)| s.start < end && start < s.end)
            .map(|(i, _)| MathId(i))
            .collect();
        let mut cursor = doc.maths_in(Span::new(start, end)).unwrap();
        let mut got = Vec::new();
        loop {
            let mut budget = Budget::new(WorkLimit {
                queries: 1,
                objects: 1,
                ..WorkLimit::default()
            });
            let Some((id, _)) = cursor.advance(&mut budget).unwrap() else {
                break;
            };
            assert_eq!(budget.used().bytes, 0);
            got.push(id);
            assert_eq!(
                cursor.advance(&mut budget),
                Err(Completion::BudgetExhausted)
            );
        }
        assert_eq!(got, want);
        assert_eq!(
            cursor.advance(&mut Budget::new(WorkLimit::default())),
            Ok(None)
        );
    }
    let mut empty = doc.maths_in(Span::new(1, 1)).unwrap();
    assert_eq!(
        empty.advance(&mut Budget::new(WorkLimit::default())),
        Ok(None)
    );
}

#[test]
fn million_byte_candidates_and_adversarial_runs_have_linear_work() {
    let mut doc = Document::new();
    doc.append("$$").unwrap();
    doc.append(&"x".repeat(1_000_000)).unwrap();
    assert!(doc.maths().is_empty());
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("x").unwrap();
    }
    assert_eq!(doc.work().inline_bytes - before.inline_bytes, 1000);
    assert_eq!(doc.work().math_steps, before.math_steps);
    assert_eq!(doc.work().classified_bytes, before.classified_bytes);
    let before = doc.work();
    doc.append("$$ ").unwrap();
    assert_eq!(doc.maths().len(), 1);
    assert_eq!(doc.work().inline_bytes - before.inline_bytes, 3);
    assert_eq!(doc.work().math_steps - before.math_steps, 1);
    assert!(doc.work().tree_visits - before.tree_visits < 1000);
    let mut reader = doc.math_content(MathId(0)).unwrap();
    let mut budget = Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 16,
        ..WorkLimit::default()
    });
    assert_eq!(reader.advance(&mut budget).unwrap().unwrap().text.len(), 16);
    assert_eq!(budget.used().bytes, 16);

    for n in [1024, 4096, 16_384] {
        for body in ["$$x".repeat(n), "$".repeat(n), r"\$".repeat(n)] {
            let source = format!("$a{body}");
            let mut doc = Document::new();
            for c in source.chars() {
                doc.append(c.encode_utf8(&mut [0; 4])).unwrap();
            }
            let before = doc.work();
            doc.finish();
            assert!(doc.maths().is_empty());
            assert_eq!(doc.work().inline_bytes, source.len() as u64);
            assert_eq!(doc.work().inline_bytes, before.inline_bytes);
            assert!(doc.work().math_steps <= (n + 2) as u64);
            assert!(doc.annotation_node_count() < 32);
        }
    }
}

#[test]
fn folded_math_body_is_never_consumed_by_hidden_layout() {
    let mut doc = Document::new();
    doc.append(&format!("# title\n$ {} $\n", "x".repeat(1_000_000)))
        .unwrap();
    doc.finish();
    let mut view = FoldView::new(&doc, 1).unwrap();
    view.set_collapsed(&doc, FoldId::Heading(HeadingId(0)), true)
        .unwrap();
    // Metadata can be inspected without reading the enormous hidden content.
    let math = doc.math(MathId(0)).unwrap();
    let mut cursor = doc.maths_in(math.source).unwrap();
    let mut budget = Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        ..WorkLimit::default()
    });
    assert_eq!(cursor.advance(&mut budget).unwrap().unwrap().0, MathId(0));
    assert_eq!(budget.used().bytes, 0);
    #[derive(Default)]
    struct Sink {
        bytes: usize,
        placeholders: usize,
    }
    impl LayoutSink for Sink {
        fn begin_tile(&mut self, _: TileId) {}
        fn text(&mut self, run: RenderRun<'_>) {
            self.bytes += run.text.len();
        }
        fn object(&mut self, _: LinkId, _: &LinkInfo) {}
        fn end_tile(&mut self, _: TileId) -> u64 {
            1
        }
    }
    impl FoldLayoutSink for Sink {
        fn placeholder(&mut self, _: FoldPlaceholder) {
            self.placeholders += 1;
        }
    }
    let mut pass = LayoutPass::folded(&doc, &view, TileId(0), 10).unwrap();
    let mut sink = Sink::default();
    let mut budget = Budget::new(WorkLimit {
        queries: 32,
        objects: 32,
        tiles: 32,
        bytes: 32,
        ..WorkLimit::default()
    });
    assert_eq!(
        pass.advance_folded(&doc, &mut view, &mut budget, &mut sink)
            .unwrap(),
        Completion::EndOfDocument
    );
    assert_eq!(sink.placeholders, 1);
    assert!(sink.bytes < 16);
    assert!(budget.used().bytes < 16);
}
