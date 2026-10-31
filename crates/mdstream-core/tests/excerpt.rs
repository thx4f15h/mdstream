use mdstream::*;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn limits(source_bytes: usize, display_bytes: usize, lines: usize) -> ExcerptLimits {
    ExcerptLimits {
        source_bytes,
        display_bytes,
        lines,
    }
}

fn frame(bytes: usize) -> Budget {
    Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        bytes,
        ..Default::default()
    })
}

fn collect(
    doc: &Document,
    source: Span,
    limits: ExcerptLimits,
    bytes: usize,
) -> (String, ExcerptEnd, usize) {
    let mut task = DisplayExcerpt::new(doc, source, limits).unwrap();
    let mut output = String::new();
    let mut reads = 0;
    for _ in 0..source.len().min(limits.source_bytes).saturating_mul(4) + 20 {
        let mut budget = frame(bytes);
        loop {
            match task.next(doc, &mut budget).unwrap() {
                ExcerptStep::Run(run) => {
                    assert!(!run.text.is_empty());
                    assert!(!run.style.contains(Style::HIDDEN));
                    if run.replacement.is_none() {
                        assert_eq!(doc.text(run.source), Some(run.text));
                    }
                    output.push_str(run.text);
                }
                ExcerptStep::BudgetExhausted => break,
                ExcerptStep::Complete(end) => {
                    reads += budget.used().bytes;
                    let mut zero = Budget::new(WorkLimit::default());
                    assert_eq!(
                        task.next(doc, &mut zero).unwrap(),
                        ExcerptStep::Complete(end)
                    );
                    assert_eq!(zero.used(), WorkLimit::default());
                    assert!(output.len() <= limits.display_bytes);
                    assert!(output.bytes().filter(|&b| b == b'\n').count() <= limits.lines);
                    assert!(
                        task.source_position() <= source.start.saturating_add(limits.source_bytes)
                    );
                    return (output, end, reads);
                }
            }
        }
        reads += budget.used().bytes;
    }
    panic!("excerpt failed to progress");
}

#[test]
fn code_preview_limits_logical_lines_and_preserves_fold_state() {
    for source in [
        "```rs\n中🙂 first\nsecond\nthird\n```\n",
        "    中🙂 first\n    second\n    third\n",
    ] {
        let doc = parse(source);
        let id = FoldId::Code(BlockId(0));
        let candidate = doc.fold_candidate(id).unwrap();
        let mut view = FoldView::new(&doc, 1).unwrap();
        view.set_collapsed(&doc, id, true).unwrap();
        let before = (
            view.version(),
            view.heights().total_rows(),
            view.heights().mutation_visits(),
        );
        for bytes in [6, 17, 258] {
            let (text, end, _) = collect(&doc, candidate.hidden, limits(4096, 4096, 2), bytes);
            assert_eq!(text, "中🙂 first\nsecond\n");
            assert_eq!(end, ExcerptEnd::LineLimit);
        }
        assert!(view.is_collapsed(id));
        assert_eq!(
            before,
            (
                view.version(),
                view.heights().total_rows(),
                view.heights().mutation_visits()
            )
        );
    }
}

#[test]
fn display_bytes_do_not_split_scalars_and_replacements_keep_full_mapping() {
    let doc = parse("**中🙂** &NotEqualTilde; [label](/hidden)\nrest");
    let source = Span::new(0, doc.source().len());
    for (cap, expected) in [
        (1, ""),
        (2, ""),
        (3, "中"),
        (6, "中"),
        (7, "中🙂"),
        (8, "中🙂 "),
        (10, "中🙂 "),
        (11, "中🙂 ≂"),
    ] {
        for bytes in [6, 258] {
            let (text, end, _) = collect(&doc, source, limits(4096, cap, 10), bytes);
            assert_eq!(text, expected, "cap={cap}");
            assert_eq!(end, ExcerptEnd::DisplayLimit);
        }
    }
    let start = doc.source().find('&').unwrap();
    let mut task =
        DisplayExcerpt::new(&doc, Span::new(start, source.end), limits(1, 3, 10)).unwrap();
    let mut budget = frame(6);
    let ExcerptStep::Run(run) = task.next(&doc, &mut budget).unwrap() else {
        panic!()
    };
    assert_eq!(run.text, "≂");
    assert_eq!(
        run.replacement,
        Some(Span::new(start, start + "&NotEqualTilde;".len()))
    );
    assert_eq!(
        task.next(&doc, &mut budget).unwrap(),
        ExcerptStep::Complete(ExcerptEnd::DisplayLimit)
    );
    // The source cap can cut a replacement without losing its full mapping.
    assert_eq!(
        collect(&doc, Span::new(start, source.end), limits(1, 10, 10), 6).0,
        "≂\u{338}"
    );
}

#[test]
fn clipped_link_text_keeps_style_and_absolute_source_mapping() {
    let doc = parse("before\n\n[**a&amp;b**](/target) after\n");
    let label = doc.link_info(LinkId(0)).unwrap().label;
    let mut task = DisplayExcerpt::new(&doc, label, limits(100, 2, 1)).unwrap();
    let mut budget = Budget::new(WorkLimit::unlimited());
    let mut text = String::new();
    loop {
        match task.next(&doc, &mut budget).unwrap() {
            ExcerptStep::Run(run) => {
                assert_eq!(run.link, Some(LinkId(0)));
                assert!(run.style.contains(Style::STRONG));
                assert!(run.source.start >= label.start);
                if run.text == "&" {
                    assert_eq!(doc.text(run.replacement.unwrap()), Some("&amp;"));
                }
                text.push_str(run.text);
            }
            ExcerptStep::Complete(end) => {
                assert_eq!(end, ExcerptEnd::DisplayLimit);
                break;
            }
            ExcerptStep::BudgetExhausted => panic!("unlimited budget exhausted"),
        }
    }
    assert_eq!(text, "a&");
}

#[test]
fn source_caps_zero_limits_empty_and_exact_ends() {
    let doc = parse("中🙂\nend");
    let source = Span::new(0, doc.source().len());
    for cap in 0..7 {
        let (text, end, _) = collect(&doc, source, limits(cap, 100, 10), 6);
        assert_eq!(text, if cap < 3 { "" } else { "中" });
        assert_eq!(end, ExcerptEnd::SourceLimit);
    }
    for (caps, expected) in [
        (limits(100, 0, 10), ExcerptEnd::DisplayLimit),
        (limits(100, 100, 0), ExcerptEnd::LineLimit),
        (limits(0, 0, 0), ExcerptEnd::SourceLimit),
    ] {
        let (text, end, reads) = collect(&doc, source, caps, 6);
        assert_eq!((text.as_str(), end, reads), ("", expected, 0));
    }
    assert_eq!(
        collect(&doc, Span::new(0, 0), limits(0, 0, 0), 6),
        (String::new(), ExcerptEnd::SourceEnd, 0)
    );
    assert_eq!(
        collect(&doc, Span::new(0, 8), limits(8, 8, 1), 258).1,
        ExcerptEnd::SourceEnd
    );
    let doc = parse("**hidden markers**");
    let (text, end, _) = collect(
        &doc,
        Span::new(0, doc.source().len()),
        limits(usize::MAX, usize::MAX, usize::MAX),
        6,
    );
    assert_eq!(
        (text.as_str(), end),
        ("hidden markers", ExcerptEnd::SourceEnd)
    );
}

#[test]
fn errors_and_exhausted_budget_do_not_change_task_or_spend_work() {
    let mut doc = parse("中x");
    for source in [
        Span::new(1, 3),
        Span::new(0, 1),
        Span::new(4, 3),
        Span::new(0, 5),
    ] {
        assert!(matches!(
            DisplayExcerpt::new(&doc, source, limits(0, 0, 0)),
            Err(LayoutError::InvalidRange)
        ));
    }
    let mut task = DisplayExcerpt::new(&doc, Span::new(0, 4), limits(4, 4, 1)).unwrap();
    for limit in [
        WorkLimit::default(),
        WorkLimit {
            queries: 1,
            bytes: 6,
            ..Default::default()
        },
        WorkLimit {
            objects: 1,
            bytes: 6,
            ..Default::default()
        },
        WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 4,
            ..Default::default()
        },
    ] {
        let mut budget = Budget::new(limit);
        assert_eq!(
            task.next(&doc, &mut budget).unwrap(),
            ExcerptStep::BudgetExhausted
        );
        assert_eq!(budget.used(), WorkLimit::default());
        assert_eq!(task.source_position(), 0);
    }
    let other = parse("中x");
    let mut budget = frame(6);
    assert_eq!(
        task.next(&other, &mut budget),
        Err(LayoutError::WrongDocument)
    );
    assert_eq!(budget.used(), WorkLimit::default());
    assert!(matches!(
        task.next(&doc, &mut budget),
        Ok(ExcerptStep::Run(_))
    ));
    assert_eq!(
        task.next(&doc, &mut budget).unwrap(),
        ExcerptStep::Complete(ExcerptEnd::SourceEnd)
    );
    assert_eq!(
        task.next(&other, &mut budget),
        Err(LayoutError::WrongDocument)
    );
    // Repeated EOF preserves a completed task.
    doc.finish();
    assert_eq!(
        task.next(&doc, &mut budget).unwrap(),
        ExcerptStep::Complete(ExcerptEnd::SourceEnd)
    );
}

#[test]
fn streaming_prefixes_late_setext_and_finish_require_explicit_recreation() {
    let mut doc = Document::new();
    for chunk in [
        "# A\n",
        "first &am",
        "p;\n",
        "\nlong title\n",
        "---\n",
        "```rs\n",
        "body",
        "\n```\n",
    ] {
        let mut old =
            DisplayExcerpt::new(&doc, Span::new(0, doc.source().len()), limits(1000, 100, 3))
                .unwrap();
        doc.append("").unwrap();
        assert_eq!(doc.revision(), old.revision());
        doc.append(chunk).unwrap();
        let mut budget = frame(6);
        assert_eq!(
            old.next(&doc, &mut budget),
            Err(LayoutError::StaleContinuation)
        );
        assert_eq!(budget.used(), WorkLimit::default());
        let range = Span::new(0, doc.source().len());
        let small = collect(&doc, range, limits(1000, 100, 3), 6);
        let large = collect(&doc, range, limits(1000, 100, 3), 258);
        assert_eq!((small.0, small.1), (large.0, large.1));
    }
    let mut old = DisplayExcerpt::new(&doc, Span::new(0, 0), limits(0, 0, 0)).unwrap();
    assert_eq!(
        old.next(&doc, &mut frame(6)).unwrap(),
        ExcerptStep::Complete(ExcerptEnd::SourceEnd)
    );
    doc.finish();
    let mut budget = frame(6);
    assert_eq!(
        old.next(&doc, &mut budget),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(budget.used(), WorkLimit::default());
}

#[test]
fn million_byte_first_line_and_hidden_syntax_have_bounded_work() {
    for count in [1000, 1_000_000] {
        for source in [
            format!("```\n{}\ntail\n```\n", "x".repeat(count)),
            format!("[link]({})\ntail", "x".repeat(count)),
        ] {
            let doc = parse(&source);
            let range = Span::new(0, source.len());
            let (text, end, reads) = collect(&doc, range, limits(64, 16, 2), 258);
            assert!(text.len() <= 16);
            assert!(matches!(
                end,
                ExcerptEnd::SourceLimit | ExcerptEnd::DisplayLimit
            ));
            assert!(reads < 1500, "read {reads} for {count} bytes");
        }
        let doc = parse(&format!("```{}\nbody\n```\n", "x".repeat(count)));
        let (text, end, reads) = collect(
            &doc,
            Span::new(0, doc.source().len()),
            limits(64, 100, 10),
            6,
        );
        assert!(text.is_empty());
        assert_eq!(end, ExcerptEnd::SourceLimit);
        assert!(reads < 1500);
    }
}

#[test]
fn prefix_model_covers_entities_crlf_styles_tiles_and_all_chunk_boundaries() {
    let source = format!(
        "# H\n**中🙂** &NewLine; [x&amp;y](/url)  \n` a\r\nb `\n\n```rs\n{}\r\ntail\n```\n",
        "a🙂 ".repeat(80)
    );
    for split in source.char_indices().map(|(i, _)| i).chain([source.len()]) {
        let mut doc = Document::new();
        doc.append(&source[..split]).unwrap();
        doc.append(&source[split..]).unwrap();
        doc.finish();
        for source_bytes in [0, 1, 17, 64, 257, source.len()] {
            let mut end = source_bytes;
            while !source.is_char_boundary(end) {
                end -= 1;
            }
            let visible: String = doc
                .display_runs_in(Span::new(0, end))
                .unwrap()
                .filter(|run| !run.style.contains(Style::HIDDEN))
                .map(|run| run.text)
                .collect();
            for (display_bytes, lines) in [(0, 0), (5, 1), (33, 2), (1000, 20)] {
                let mut expected = String::new();
                for line in visible.split_inclusive('\n').take(lines) {
                    expected.push_str(line);
                }
                let mut end = display_bytes.min(expected.len());
                while !expected.is_char_boundary(end) {
                    end -= 1;
                }
                expected.truncate(end);
                for bytes in [6, 258] {
                    let actual = collect(
                        &doc,
                        Span::new(0, source.len()),
                        limits(source_bytes, display_bytes, lines),
                        bytes,
                    );
                    assert_eq!(
                        actual.0, expected,
                        "split={split}, source={source_bytes}, display={display_bytes}, lines={lines}, budget={bytes}"
                    );
                }
            }
        }
    }
}
