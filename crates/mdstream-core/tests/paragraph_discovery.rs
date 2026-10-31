use mdstream::*;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}
fn frame(n: usize) -> Budget {
    Budget::new(WorkLimit {
        queries: n,
        objects: n,
        ..Default::default()
    })
}
fn discover(doc: &Document, range: Span, min: usize) -> (Vec<ParagraphFoldCandidate>, usize) {
    let mut task = ParagraphFoldCandidates::new(doc, range, min).unwrap();
    let mut all = Vec::new();
    let mut out = Vec::new();
    let mut steps = 0;
    loop {
        let mut budget = frame(1);
        let status = task.advance(doc, &mut budget, &mut out).unwrap();
        steps += budget.used().queries;
        assert_eq!(budget.used().queries, budget.used().objects);
        assert_eq!(budget.used().bytes, 0);
        all.extend_from_slice(&out);
        if status == Completion::Covered {
            break;
        }
        assert_eq!(status, Completion::BudgetExhausted);
        assert!(steps <= doc.blocks().len() + 2);
    }
    assert_eq!(
        task.advance(doc, &mut frame(0), &mut out),
        Ok(Completion::Covered)
    );
    assert!(out.is_empty());
    (all, steps)
}
fn all(doc: &Document) -> Vec<ParagraphFoldCandidate> {
    discover(doc, Span::new(0, doc.source().len()), 0).0
}
fn registration(
    doc: &Document,
    view: &FoldView,
    c: ParagraphFoldCandidate,
    preview: usize,
) -> SourceFoldRegistration {
    if c.container.is_some() {
        SourceFoldRegistration::container_paragraph(doc, view, c.block, preview)
    } else if c.open {
        SourceFoldRegistration::open_paragraph(doc, view, c.block, preview)
    } else {
        SourceFoldRegistration::paragraph(doc, view, c.block, preview)
    }
    .unwrap()
}
fn register(doc: &Document, view: &mut FoldView, c: ParagraphFoldCandidate) -> FoldId {
    let mut task = registration(doc, view, c, 0);
    for _ in 0..5000 {
        if let Some(id) = task.advance(doc, view, &mut frame(1)).unwrap().id {
            return id;
        }
    }
    panic!("registration stalled")
}

#[test]
fn mixed_paragraphs_have_exact_logical_ranges_and_registerable_first_leaves() {
    let source = "top 中🙂\ncontinued\n\n# heading\n> first\n> second\n>\n> third\n\n- item\n  continued\n- sibling\n\n```\ncode\n```\n\nend\n";
    let doc = parse(source);
    let expected = [
        ("top", "\n\n#"),
        ("> first", ">\n> third"),
        ("> third", "\n\n-"),
        ("- item", "- sibling"),
        ("- sibling", "\n\n```"),
        ("end\n", ""),
    ]
    .map(|(start, end)| {
        let start = source.find(start).unwrap();
        let end = if end.is_empty() {
            source.len()
        } else {
            let at = start + source[start..].find(end).unwrap();
            at + usize::from(end.starts_with('\n'))
        };
        Span::new(start, end)
    });
    let candidates = all(&doc);
    assert_eq!(
        candidates.iter().map(|c| c.source).collect::<Vec<_>>(),
        expected
    );
    let mut view = FoldView::new(&doc, 1).unwrap();
    for c in candidates {
        assert!(!c.open);
        assert_eq!(c.classified_end, c.source.end);
        assert_eq!(c.block, doc.block_at(c.source.start).unwrap());
        assert_eq!(c.container, doc.block(c.block).unwrap().container);
        let id = register(&doc, &mut view, c);
        assert_eq!(
            view.fold_candidate(&doc, id).unwrap().unwrap().source,
            c.source
        );
    }
}

#[test]
fn windows_and_thresholds_match_independent_source_envelopes() {
    let mut source = String::new();
    let mut expected = Vec::new();
    for i in 0..400 {
        source.push_str("# separator\n");
        let start = source.len();
        let body = "中🙂x".repeat(i % 9 + 1);
        match i % 4 {
            0 => source.push_str(&format!("{body}\nmore\n")),
            1 => source.push_str(&format!("> {body}\n> more\n")),
            2 => source.push_str(&format!("- {body}\n  more\n")),
            _ => source.push_str(&format!("> - {body}\r\n>   more\r\n")),
        }
        expected.push(Span::new(start, source.len()));
        source.push('\n');
    }
    let doc = parse(&source);
    assert_eq!(
        all(&doc).iter().map(|c| c.source).collect::<Vec<_>>(),
        expected
    );
    let mut seed = 78u64;
    for _ in 0..1600 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let start = seed as usize % (source.len() + 1);
        let end = (start + (seed >> 32) as usize % 150).min(source.len());
        let min = (seed >> 24) as usize % 100;
        let got = discover(&doc, Span::new(start, end), min).0;
        let want: Vec<_> = expected
            .iter()
            .copied()
            .filter(|s| start < end && s.start < end && start < s.end && s.len() >= min)
            .collect();
        assert_eq!(
            got.iter().map(|c| c.source).collect::<Vec<_>>(),
            want,
            "{start}..{end}, min={min}"
        );
    }
    // Half-open windows at every byte, including inside a scalar and in gaps.
    let doc = parse("> 中🙂\n> tail\n>\n");
    for at in 0..doc.source().len() {
        let got = discover(&doc, Span::new(at, at + 1), 0).0;
        assert_eq!(got.len(), usize::from(at < 17));
        if let Some(c) = got.first() {
            assert_eq!(c.source, Span::new(0, 17));
        }
    }
}

#[test]
fn budget_is_atomic_and_nonparagraph_leaves_cannot_bypass_it() {
    let doc = parse(&format!("{}tail\n", "# heading\n".repeat(4096)));
    let mut task = ParagraphFoldCandidates::new(&doc, Span::new(0, doc.source().len()), 0).unwrap();
    let mut out = all(&doc);
    for limit in [
        WorkLimit::default(),
        WorkLimit {
            queries: 1,
            ..Default::default()
        },
        WorkLimit {
            objects: 1,
            ..Default::default()
        },
    ] {
        let mut budget = Budget::new(limit);
        assert_eq!(
            task.advance(&doc, &mut budget, &mut out),
            Ok(Completion::BudgetExhausted)
        );
        assert!(out.is_empty());
        assert_eq!(budget.used(), WorkLimit::default());
    }
    for _ in 0..4096 {
        assert_eq!(
            task.advance(&doc, &mut frame(1), &mut out),
            Ok(Completion::BudgetExhausted)
        );
        assert!(out.is_empty());
    }
    assert_eq!(
        task.advance(&doc, &mut frame(1), &mut out),
        Ok(Completion::Covered)
    );
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].block, BlockId(4096));
}

#[test]
fn pending_tail_is_provisional_and_does_not_meet_the_length_threshold() {
    for (initial, pending) in [
        ("abc\n", "中🙂"),
        ("> abc\n> def\n", "> 中🙂"),
        ("- abc\n  def\n", "  中🙂"),
    ] {
        let mut doc = Document::new();
        doc.append(initial).unwrap();
        doc.append(pending).unwrap();
        let window = Span::new(initial.len(), doc.source().len());
        let c = discover(&doc, window, initial.len()).0[0];
        assert_eq!(c.source, Span::new(0, doc.source().len()));
        assert_eq!(c.classified_end, initial.len());
        assert!(c.open);
        assert!(discover(&doc, window, initial.len() + 1).0.is_empty());
        let mut view = FoldView::new(&doc, 1).unwrap();
        register(&doc, &mut view, c);
        let update = doc.finish();
        view.apply(&doc, &update).unwrap();
        let c = all(&doc)[0];
        assert!(!c.open);
        assert_eq!(c.classified_end, doc.source().len());
    }
    let mut doc = Document::new();
    doc.append("unclassified").unwrap();
    assert!(all(&doc).is_empty());
}

#[test]
fn late_setext_tables_and_scope_boundaries_refresh_candidates_and_registered_ids() {
    for (initial, tail, end) in [
        ("one\ntwo\n", "===\n", None),
        ("a|b\n", "-|-\n", None),
        ("lead\na|b\n", "-|-\n", Some(5)),
        ("> one\n> two\n", "> ===\n", None),
        ("> a|b\n", "> -|-\n", None),
        ("> lead\n> a|b\n", "> -|-\n", Some(7)),
        ("> one\n", "> > nested\n", Some(6)),
        ("- one\n", "  two\n- sibling\n", Some(12)),
        ("> lead\n> a|b\n", "> nope\n>\n", Some(20)),
    ] {
        let mut doc = Document::new();
        doc.append(initial).unwrap();
        let c = all(&doc)[0];
        let mut view = FoldView::new(&doc, 1).unwrap();
        let id = register(&doc, &mut view, c);
        view.set_collapsed(&doc, id, true).unwrap();
        let update = doc.append(tail).unwrap();
        view.apply(&doc, &update).unwrap();
        let found = all(&doc).into_iter().find(|p| p.block == c.block);
        assert_eq!(found.map(|p| p.source.end), end, "{initial:?} + {tail:?}");
        assert_eq!(
            view.fold_candidate(&doc, id).unwrap().map(|c| c.source.end),
            end
        );
        if let Some(p) = found {
            assert!(!p.open);
        }
    }
}

#[test]
fn utf8_all_splits_and_live_prefixes_preserve_discovery_snapshots() {
    let long = "x".repeat(BLOCK_LOOKAHEAD + 9);
    let sources = [
        "top 中🙂\r\nmore\r\n\r\n> - one\n>   two\n>   ===\n\nend".to_owned(),
        "> one\n> a|b\n> -|-\n> x|y\n>\n- item\n  lazy\n\nend".to_owned(),
        "- one\n\t中🙂\n  two\n- next\n> quote\nlazy\n>\n".to_owned(),
        format!("> lead\n> {long}中|b\r\n> -|-\r\n>\r\nend"),
    ];
    for source in sources {
        let expected = all(&parse(&source));
        for split in (0..=source.len()).filter(|&at| source.is_char_boundary(at)) {
            let mut doc = Document::new();
            doc.append(&source[..split]).unwrap();
            let prefix = all(&doc);
            doc.append("").unwrap();
            assert_eq!(all(&doc), prefix);
            doc.append(&source[split..]).unwrap();
            doc.finish();
            assert_eq!(all(&doc), expected, "split {split}");
        }
        let mut streamed = Document::new();
        for (at, ch) in source.char_indices() {
            streamed.append(&ch.to_string()).unwrap();
            let end = at + ch.len_utf8();
            let mut batch = Document::new();
            batch.append(&source[..end]).unwrap();
            assert_eq!(all(&streamed), all(&batch), "prefix {end}");
            // Starting in the new bytes still finds the earlier first leaf.
            assert_eq!(
                discover(&streamed, Span::new(at, end), 0).0,
                discover(&batch, Span::new(at, end), 0).0
            );
        }
    }
}

#[test]
fn deep_million_byte_paragraph_and_many_leaves_take_one_discovery_step() {
    let mut doc = Document::new();
    doc.append(&format!("{}start\n", ">".repeat(1000))).unwrap();
    doc.append(&format!("{}{}", ">".repeat(1000), "x".repeat(1_000_000)))
        .unwrap();
    let work = doc.work();
    for range in [
        Span::new(0, doc.source().len()),
        Span::new(800_000, 800_001),
    ] {
        let (out, steps) = discover(&doc, range, 1_000_000);
        assert_eq!(steps, 1);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].block, BlockId(0));
        assert_eq!(out[0].container, Some(ContainerId(999)));
    }
    assert_eq!(doc.work(), work);
    let mut doc = Document::new();
    doc.append(&"> line\n".repeat(4096)).unwrap();
    for closed in [false, true] {
        if closed {
            doc.finish();
        }
        for min in [0, usize::MAX] {
            for start in [0, 7, 7 * 3000, doc.source().len() - 1] {
                let (out, steps) = discover(&doc, Span::new(start, doc.source().len()), min);
                assert_eq!(steps, 1);
                assert_eq!(out.len(), usize::from(min == 0));
                if let Some(c) = out.first() {
                    assert_eq!(c.block, BlockId(0));
                }
            }
        }
    }
    assert_eq!(doc.work().paragraph_closures, 1);
}

#[test]
fn historical_paragraphs_and_nested_lists_do_not_enlarge_window_work() {
    let mut doc = Document::new();
    doc.append(&"> old\n>\n".repeat(4096)).unwrap();
    let start = doc.source().len();
    doc.append("> new\n> continuation\n").unwrap();
    let (out, steps) = discover(&doc, Span::new(start + 10, doc.source().len()), 0);
    assert_eq!(steps, 1);
    assert_eq!(out[0].source.start, start);
    doc.finish();
    assert_eq!(
        discover(&doc, Span::new(start + 10, doc.source().len()), 0).1,
        1
    );
    let mut doc = Document::new();
    for depth in 0..250 {
        doc.append(&format!("{}- item\n", "  ".repeat(depth)))
            .unwrap();
    }
    let end = doc.source().len();
    let (out, steps) = discover(&doc, Span::new(end - 2, end), 0);
    assert_eq!(steps, 1);
    assert_eq!(out[0].block, BlockId(249));
    assert_eq!(all(&doc).len(), 250);
}

#[test]
fn discovery_and_registration_share_budget_without_walking_ancestors_early() {
    let doc = parse(&format!("{}paragraph\n", ">".repeat(1000)));
    let mut budget = frame(1);
    let mut out = Vec::new();
    let mut discovery =
        ParagraphFoldCandidates::new(&doc, Span::new(0, doc.source().len()), 0).unwrap();
    assert_eq!(
        discovery.advance(&doc, &mut budget, &mut out),
        Ok(Completion::Covered)
    );
    let mut view = FoldView::new(&doc, 1).unwrap();
    let mut task = registration(&doc, &view, out[0], 0);
    assert_eq!(
        task.advance(&doc, &mut view, &mut budget)
            .unwrap()
            .completion,
        Completion::BudgetExhausted
    );
    assert_eq!(view.source_fold_count(), 0);
    let mut steps = 0;
    loop {
        steps += 1;
        let progress = task.advance(&doc, &mut view, &mut frame(1)).unwrap();
        if progress.id.is_some() {
            assert_eq!(progress.visited, 2000);
            assert_eq!(steps, 2005);
            break;
        }
        assert_eq!(view.source_fold_count(), 0);
        assert!(steps <= 2005);
    }
}

#[test]
fn view_registration_toggles_preview_and_resize_preserve_discovery_cursor() {
    let doc = parse("one\n\n> two\n> more\n>\n> three\n");
    let expected = all(&doc);
    let mut task = ParagraphFoldCandidates::new(&doc, Span::new(0, doc.source().len()), 0).unwrap();
    let mut out = Vec::new();
    task.advance(&doc, &mut frame(1), &mut out).unwrap();
    let mut found = out.clone();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let id = register(&doc, &mut view, out[0]);
    view.set_collapsed(&doc, id, true).unwrap();
    view.set_preview(&doc, Some(id), &mut frame(1)).unwrap();
    view.reset(2).unwrap();
    loop {
        let status = task.advance(&doc, &mut frame(1), &mut out).unwrap();
        found.extend_from_slice(&out);
        if status == Completion::Covered {
            break;
        }
    }
    assert_eq!(found, expected);
    assert_eq!(all(&doc), expected); // Includes the registered/hidden paragraph.
    assert!(SourceFoldRegistration::paragraph(&doc, &view, expected[0].block, 0).is_err());
    let mut other = FoldView::new(&doc, 1).unwrap();
    assert_ne!(id, register(&doc, &mut other, expected[0]));
    assert_eq!(all(&doc), expected);
}

#[test]
fn stale_wrong_document_and_invalid_windows_fail_without_consuming_budget_or_output() {
    let mut doc = Document::new();
    doc.append("one\n").unwrap();
    for range in [
        Span::new(2, 1),
        Span::new(0, 5),
        Span::new(usize::MAX, usize::MAX),
    ] {
        assert!(matches!(
            ParagraphFoldCandidates::new(&doc, range, 0),
            Err(LayoutError::InvalidRange)
        ));
    }
    let mut task = ParagraphFoldCandidates::new(&doc, Span::new(0, 4), 0).unwrap();
    let mut out = all(&doc);
    let original = out.clone();
    let mut budget = frame(10);
    let other = parse("one\n");
    assert_eq!(
        task.advance(&other, &mut budget, &mut out),
        Err(LayoutError::WrongDocument)
    );
    doc.append("").unwrap();
    assert_eq!(
        task.advance(&doc, &mut frame(1), &mut Vec::new()),
        Ok(Completion::Covered)
    );
    doc.append("more\n").unwrap();
    assert_eq!(
        task.advance(&doc, &mut budget, &mut out),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(budget.used(), WorkLimit::default());
    assert_eq!(out, original);
    let mut task = ParagraphFoldCandidates::new(&doc, Span::new(0, 0), 0).unwrap();
    doc.finish();
    assert_eq!(
        task.advance(&doc, &mut budget, &mut out),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(discover(&doc, Span::new(0, 0), 0), (Vec::new(), 0));
    assert_eq!(
        discover(&Document::new(), Span::new(0, 0), 0),
        (Vec::new(), 0)
    );
}

#[test]
fn merged_updates_reject_old_discovery_registration_and_measurements() {
    let mut doc = Document::new();
    doc.append("> one\n").unwrap();
    let c = all(&doc)[0];
    let mut view = FoldView::new(&doc, 1).unwrap();
    let mut task = ParagraphFoldCandidates::new(&doc, c.source, 0).unwrap();
    let id = register(&doc, &mut view, c);
    view.set_collapsed(&doc, id, true).unwrap();
    let mut other_view = FoldView::new(&doc, 1).unwrap();
    let mut registration = registration(&doc, &other_view, c, 0);
    let ticket = view.heights().ticket(TileId(0)).unwrap();
    let mut update = doc.append("> two\n").unwrap();
    update
        .merge(doc.append("> ===\n>\n> next\n").unwrap())
        .unwrap();
    view.apply(&doc, &update).unwrap();
    view.apply(&doc, &update).unwrap();
    other_view.apply(&doc, &update).unwrap();
    assert!(view.measure(ticket, 5).is_err());
    assert!(view.fold_candidate(&doc, id).unwrap().is_none());
    assert_eq!(
        task.advance(&doc, &mut frame(1), &mut Vec::new()),
        Err(LayoutError::StaleContinuation)
    );
    // The document revision is checked before any candidate can be committed.
    assert_eq!(
        registration.advance(&doc, &mut other_view, &mut frame(1)),
        Err(LayoutError::StaleContinuation)
    );
    let found = all(&doc);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].source.start, doc.source().find("> next").unwrap());
    register(&doc, &mut view, found[0]);
}

#[test]
fn streaming_discovery_registration_matches_range_and_visibility_model() {
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 2).unwrap();
    let mut expected = Vec::new();
    let mut registered = Vec::new();
    for turn in 0..240 {
        let start = doc.source().len();
        let update = doc.append(&format!("> line {turn}\n")).unwrap();
        view.apply(&doc, &update).unwrap();
        let c = discover(&doc, Span::new(start, doc.source().len()), 0).0[0];
        let id = register(&doc, &mut view, c);
        view.set_collapsed(&doc, id, true).unwrap();
        let update = doc.append("> continuation 中🙂\n").unwrap();
        view.apply(&doc, &update).unwrap();
        let end = doc.source().len();
        assert_eq!(
            discover(&doc, Span::new(end - 1, end), 0).0[0].source,
            Span::new(start, end)
        );
        let promoted = turn % 3 == 0;
        let update = doc
            .append(if promoted { "> ===\n>\n" } else { ">\n" })
            .unwrap();
        view.apply(&doc, &update).unwrap();
        if promoted {
            assert!(view.fold_candidate(&doc, id).unwrap().is_none());
        } else {
            expected.push(Span::new(start, end));
            registered.push(id);
        }
        assert_eq!(
            all(&doc).iter().map(|c| c.source).collect::<Vec<_>>(),
            expected
        );
        if turn % 11 == 0 {
            view.reset(2).unwrap();
        }
        for (&range, &id) in expected.iter().zip(&registered) {
            let c = view.fold_candidate(&doc, id).unwrap().unwrap();
            assert_eq!(c.source, range);
            assert!(!c.open);
            assert!(view.is_collapsed(id));
            assert!(
                matches!(view.hidden_at(&doc, range.start, &mut frame(1)).unwrap(), FoldLookup::Hidden(p) if p.id == id)
            );
        }
    }
}

#[test]
fn repeated_new_byte_windows_find_the_same_identity_without_history_or_body_work() {
    let mut doc = Document::new();
    doc.append(&"> old\n>\n".repeat(4096)).unwrap();
    let start = doc.source().len();
    doc.append(&format!("> {}", "x".repeat(1_000_000))).unwrap();
    let first = all(&doc).last().unwrap().block;
    let mut view = FoldView::new(&doc, 1).unwrap();
    let c = discover(&doc, Span::new(start, doc.source().len()), 1_000_000).0[0];
    let id = register(&doc, &mut view, c);
    view.set_collapsed(&doc, id, true).unwrap();
    let work = doc.work();
    let visits = view.range_visits();
    for _ in 0..1000 {
        let at = doc.source().len();
        let update = doc.append("x").unwrap();
        view.apply(&doc, &update).unwrap();
        let (found, steps) = discover(&doc, Span::new(at, at + 1), 1_000_000);
        assert_eq!(steps, 1);
        assert_eq!(found[0].block, first);
        assert_eq!(found[0].source, Span::new(start, at + 1));
        assert!(found[0].open);
    }
    assert_eq!(view.range_visits() - visits, 1000);
    assert_eq!(doc.work().inline_bytes - work.inline_bytes, 1000);
    assert_eq!(doc.work().container_steps, work.container_steps);
    assert_eq!(doc.work().paragraph_closures, work.paragraph_closures);
}
