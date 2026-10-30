use mdtui3::*;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

// Tests may flatten each title; production search must never do so.
fn oracle(doc: &Document, query: &str, max_level: u8) -> Vec<HeadingMatch> {
    let mut hits = Vec::new();
    for (id, heading) in doc.headings().entries().iter().enumerate() {
        if heading.level > max_level {
            continue;
        }
        let mut text = Vec::new();
        let mut sources = Vec::new();
        for run in doc
            .display_runs_in(heading.content)
            .unwrap()
            .filter(|run| !run.style.contains(Style::HIDDEN))
        {
            text.extend_from_slice(run.text.as_bytes());
            for i in 0..run.text.len() {
                sources.push(
                    run.replacement
                        .unwrap_or(Span::new(run.source.start + i, run.source.start + i + 1)),
                );
            }
        }
        for (i, bytes) in text.windows(query.len()).enumerate() {
            if bytes == query.as_bytes() {
                hits.push(HeadingMatch {
                    heading: HeadingId(id),
                    display: i..i + query.len(),
                    source: Span::new(sources[i].start, sources[i + query.len() - 1].end),
                });
            }
        }
    }
    hits
}

fn tiny() -> WorkLimit {
    WorkLimit {
        bytes: 6,
        queries: 1,
        objects: 1,
        ..Default::default()
    }
}

fn search(
    doc: &Document,
    query: &str,
    max_level: u8,
    limit: WorkLimit,
) -> (Vec<HeadingMatch>, WorkLimit) {
    let mut task = HeadingSearch::new(doc, query, max_level).unwrap();
    let mut hits = Vec::new();
    let mut total = WorkLimit::default();
    for _ in 0..(doc.source().len() + 1) * 12 {
        let mut budget = Budget::new(limit);
        let completion = task.advance(doc, &mut budget, &mut hits).unwrap();
        let used = budget.used();
        assert!(used.bytes <= limit.bytes);
        assert!(used.queries <= limit.queries);
        assert!(used.objects <= limit.objects);
        assert_eq!((used.tiles, used.recovery, used.dependencies), (0, 0, 0));
        total.bytes += used.bytes;
        total.queries += used.queries;
        total.objects += used.objects;
        if completion == Completion::EndOfDocument {
            assert_eq!(task.current_heading(), None);
            assert_eq!(task.source_position(), None);
            let count = hits.len();
            let mut empty = Budget::new(WorkLimit::default());
            assert_eq!(
                task.advance(doc, &mut empty, &mut hits).unwrap(),
                Completion::EndOfDocument
            );
            assert_eq!(hits.len(), count);
            assert_eq!(empty.used(), WorkLimit::default());
            assert!(hits.iter().all(|hit| doc.text(hit.source).is_some()));
            return (hits, total);
        }
        assert_eq!(completion, Completion::BudgetExhausted);
        assert_ne!(used, WorkLimit::default(), "task made no progress");
    }
    panic!("heading search did not finish");
}

#[test]
fn searches_display_names_with_absolute_mapping_and_level_filtering() {
    let doc = parse(concat!(
        "# a**b**&amp;c [label](/secret)\n",
        "body-only ab&c\n\n",
        "### ` ab&c `\n",
        "## &fjlig; &#x1f642; 中文\n",
        "a**b**&amp;c\n===\n",
        "```\n# code-only\n```\n",
        "> # quote-only\n- # list-only\n"
    ));
    for max_level in [0, 1, 2, 3, 6, u8::MAX] {
        for query in ["ab&c", "label", "f", "j", "fj", "🙂", "中文"] {
            let hits = search(&doc, query, max_level, tiny()).0;
            assert_eq!(hits, oracle(&doc, query, max_level));
        }
    }
    let hits = search(&doc, "ab&c", 6, tiny()).0;
    assert_eq!(hits.len(), 3);
    assert_eq!(hits[0].heading, HeadingId(0));
    assert_eq!(hits[0].display, 0..4);
    assert_eq!(doc.text(hits[0].source), Some("a**b**&amp;c"));
    assert_eq!(hits[1].heading, HeadingId(1));
    assert_eq!(hits[2].heading, HeadingId(3));
    let f = search(&doc, "f", 6, tiny()).0.remove(0);
    let j = search(&doc, "j", 6, tiny()).0.remove(0);
    assert_eq!(f.source, j.source);
    assert_eq!(f.display, 0..1);
    assert_eq!(j.display, 1..2);
    for absent in ["body-only", "secret", "**", "amp", "code-only"] {
        assert!(search(&doc, absent, 6, tiny()).0.is_empty(), "{absent}");
    }
    for present in ["quote-only", "list-only"] {
        assert_eq!(search(&doc, present, 6, tiny()).0.len(), 1);
    }
    assert!(search(&doc, "AB&C", 6, tiny()).0.is_empty());
}

#[test]
fn resets_partial_matches_at_every_heading_and_keeps_overlaps_and_duplicate_ids() {
    let doc = parse("# aa\n#\n# a\n# aaa\n# aaaa\n# aaa\n");
    let hits = search(&doc, "aaa", 6, tiny()).0;
    assert_eq!(
        hits.iter().map(|hit| hit.heading).collect::<Vec<_>>(),
        [HeadingId(3), HeadingId(4), HeadingId(4), HeadingId(5)]
    );
    assert_eq!(hits[1].display, 0..3);
    assert_eq!(hits[2].display, 1..4);
    assert_eq!(hits, oracle(&doc, "aaa", 6));
    assert!(search(&parse("# ab\n# cd\n"), "bc", 6, tiny()).0.is_empty());

    let query = format!("{}&中{}", "x".repeat(200), "y".repeat(300));
    let title = format!("{}&amp;中{}", "x".repeat(TILE_BYTES), "y".repeat(600));
    let doc = parse(&format!("# {title}\n## short\n# {title}\n"));
    let expected = oracle(&doc, &query, 6);
    assert_eq!(expected.len(), 2);
    for limit in [tiny(), WorkLimit::unlimited()] {
        assert_eq!(search(&doc, &query, 6, limit).0, expected);
    }
}

#[test]
fn shares_budget_for_selection_reads_and_pending_match_emission() {
    let doc = parse("# 🙂\n");
    let mut task = HeadingSearch::new(&doc, "🙂", 6).unwrap();
    let mut hits = Vec::new();
    for limit in [
        WorkLimit::default(),
        WorkLimit {
            queries: 0,
            ..WorkLimit::unlimited()
        },
        WorkLimit {
            objects: 0,
            ..WorkLimit::unlimited()
        },
    ] {
        let mut budget = Budget::new(limit);
        assert_eq!(
            task.advance(&doc, &mut budget, &mut hits).unwrap(),
            Completion::BudgetExhausted
        );
        assert_eq!(budget.used(), WorkLimit::default());
        assert_eq!(task.current_heading(), None);
    }
    let mut budget = Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        ..Default::default()
    });
    task.advance(&doc, &mut budget, &mut hits).unwrap();
    assert_eq!(task.current_heading(), Some(HeadingId(0)));
    assert_eq!(task.source_position(), Some(2));
    assert_eq!(budget.used().bytes, 0);
    let mut too_small = Budget::new(WorkLimit {
        bytes: 5,
        ..WorkLimit::unlimited()
    });
    task.advance(&doc, &mut too_small, &mut hits).unwrap();
    assert_eq!(too_small.used(), WorkLimit::default());
    assert_eq!(task.source_position(), Some(2));

    let mut read = Budget::new(tiny());
    task.advance(&doc, &mut read, &mut hits).unwrap();
    assert_eq!(task.source_position(), Some(6));
    assert!(hits.is_empty());
    let mut compare = Budget::new(WorkLimit {
        bytes: 4,
        ..Default::default()
    });
    assert_eq!(
        task.advance(&doc, &mut compare, &mut hits).unwrap(),
        Completion::BudgetExhausted
    );
    assert_eq!(compare.used().bytes, 3);
    assert!(hits.is_empty());
    task.advance(&doc, &mut compare, &mut hits).unwrap();
    assert_eq!(compare.used().bytes, 3);
    let mut emit = Budget::new(WorkLimit {
        bytes: 1,
        objects: 1,
        ..Default::default()
    });
    assert_eq!(
        task.advance(&doc, &mut emit, &mut hits).unwrap(),
        Completion::EndOfDocument
    );
    assert_eq!(emit.used().queries, 0);
    assert_eq!(hits, oracle(&doc, "🙂", 6));
}

#[test]
fn rejects_wrong_or_stale_documents_before_touching_budget_or_results() {
    let mut doc = Document::new();
    doc.append("# aaa\n").unwrap();
    let mut pending = HeadingSearch::new(&doc, "a", 6).unwrap();
    let mut hits = Vec::new();
    pending
        .advance(&doc, &mut Budget::new(tiny()), &mut hits)
        .unwrap();
    assert_eq!(pending.revision(), doc.revision());
    let position = pending.source_position();
    let other = parse("# aaa\n");
    let mut budget = Budget::new(WorkLimit::unlimited());
    assert_eq!(
        pending.advance(&other, &mut budget, &mut hits),
        Err(LayoutError::WrongDocument)
    );
    assert_eq!(budget.used(), WorkLimit::default());
    assert_eq!(pending.source_position(), position);

    let mut completed = HeadingSearch::new(&doc, "a", 6).unwrap();
    completed.advance(&doc, &mut budget, &mut hits).unwrap();
    let before = hits.clone();
    doc.append("").unwrap();
    assert_eq!(
        completed.advance(&doc, &mut budget, &mut hits).unwrap(),
        Completion::EndOfDocument
    );
    doc.append("## aaa\n").unwrap();
    for task in [&mut pending, &mut completed] {
        let mut budget = Budget::new(WorkLimit::unlimited());
        assert_eq!(
            task.advance(&doc, &mut budget, &mut hits),
            Err(LayoutError::StaleContinuation)
        );
        assert_eq!(budget.used(), WorkLimit::default());
        assert_eq!(hits, before);
    }
    let mut unfinished = HeadingSearch::new(&doc, "a", 6).unwrap();
    doc.finish();
    let mut empty = Budget::new(WorkLimit::default());
    assert_eq!(
        unfinished.advance(&doc, &mut empty, &mut hits),
        Err(LayoutError::StaleContinuation)
    );
    let mut finished = HeadingSearch::new(&doc, "a", 6).unwrap();
    doc.finish();
    assert_eq!(
        finished
            .advance(&doc, &mut budget, &mut Vec::new())
            .unwrap(),
        Completion::EndOfDocument
    );
    assert_eq!(search(&doc, "a", 6, tiny()).0.len(), 6);

    let empty = Document::new();
    assert!(matches!(
        HeadingSearch::new(&empty, "", 6),
        Err(LayoutError::InvalidRange)
    ));
    assert!(search(&empty, "a", 6, WorkLimit::default()).0.is_empty());
    assert!(search(&doc, "a", 0, WorkLimit::default()).0.is_empty());
    assert!(search(&parse("#\n##\n"), "a", 6, tiny()).0.is_empty());
}

#[test]
fn live_prefixes_late_setext_and_chunk_splits_match_title_oracle() {
    let prefix = "x".repeat(BLOCK_LOOKAHEAD);
    for start in [prefix.clone(), format!("# {prefix}")] {
        let suffix = "&amp; **a** ` b ` 中\n===\n# a&amp;中\n";
        let mut doc = Document::new();
        doc.append(&start).unwrap();
        for c in suffix.chars() {
            let mut old = HeadingSearch::new(&doc, "a", 6).unwrap();
            doc.append(c.encode_utf8(&mut [0; 4])).unwrap();
            let mut empty = Budget::new(WorkLimit::default());
            assert_eq!(
                old.advance(&doc, &mut empty, &mut Vec::new()),
                Err(LayoutError::StaleContinuation)
            );
            for query in ["x&", "&", "a b", "中", "a&中"] {
                assert_eq!(search(&doc, query, 6, tiny()).0, oracle(&doc, query, 6));
            }
        }
    }
    let source = "# a**b**&amp;中\n\nbody\n\n` x `&fjlig;\n===\n## a**b**&amp;中\n";
    let expected = parse(source);
    for split in source.char_indices().map(|(i, _)| i).chain([source.len()]) {
        let mut doc = Document::new();
        doc.append(&source[..split]).unwrap();
        doc.append(&source[split..]).unwrap();
        doc.finish();
        for query in ["ab&中", "xfj", "f", "body"] {
            assert_eq!(
                search(&doc, query, 6, tiny()).0,
                oracle(&expected, query, 6)
            );
        }
    }
}

#[test]
fn skips_giant_bodies_and_filtered_headings_with_constant_charged_work() {
    let mut previous = None;
    for (body, excluded) in [(8, 1), (1_000_000, 10_000)] {
        let doc = parse(&format!(
            "# needle\n\n{}\n\n{}# needle\n",
            "x".repeat(body),
            "### needle\n".repeat(excluded)
        ));
        let before = doc.work();
        let (hits, used) = search(&doc, "needle", 1, tiny());
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[1].heading, HeadingId(excluded + 1));
        assert_eq!(doc.work(), before);
        assert!(used.bytes < 64);
        if let Some(previous) = previous {
            assert_eq!(used, previous);
        }
        previous = Some(used);
    }
    let doc = parse(&"###### needle\n".repeat(10_000));
    let (hits, used) = search(&doc, "needle", 1, tiny());
    assert!(hits.is_empty());
    assert_eq!(
        used,
        WorkLimit {
            queries: 1,
            objects: 1,
            ..Default::default()
        }
    );
}

#[test]
fn giant_setext_search_is_deferred_and_kmp_failure_work_stays_linear() {
    for size in [4096, 8192, 1_000_000] {
        let mut doc = Document::new();
        doc.append(&format!("{}\n", "a".repeat(size))).unwrap();
        let mut old = HeadingSearch::new(&doc, "a", 6).unwrap();
        let before = doc.work();
        doc.append("===\n").unwrap();
        assert_eq!(doc.work().inline_bytes, before.inline_bytes);
        assert!(doc.work().tree_visits - before.tree_visits < 200);
        assert_eq!(
            old.advance(&doc, &mut Budget::new(tiny()), &mut Vec::new()),
            Err(LayoutError::StaleContinuation)
        );
        let query = format!("{}b", "a".repeat(2000));
        let mut task = HeadingSearch::new(&doc, &query, 6).unwrap();
        assert_eq!(task.source_position(), None);
        let mut budget = Budget::new(WorkLimit {
            bytes: 32,
            queries: 2,
            objects: 2,
            ..Default::default()
        });
        assert_eq!(
            task.advance(&doc, &mut budget, &mut Vec::new()).unwrap(),
            Completion::BudgetExhausted
        );
        assert!(task.source_position().unwrap() <= 30);
        let (hits, used) = search(
            &doc,
            &query,
            6,
            WorkLimit {
                bytes: 1024,
                queries: 8,
                objects: 8,
                ..Default::default()
            },
        );
        assert!(hits.is_empty());
        assert!(
            used.bytes < size * 4 + 16,
            "{} bytes for {size}",
            used.bytes
        );
        assert!(used.queries < size);
    }
}

#[test]
fn seeded_title_queries_and_budget_slices_match_naive_search() {
    let seed = 0x7aa1_c832_1893_4f20_u64;
    let mut state = seed;
    let mut random = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state as usize
    };
    let atoms = [
        "a",
        "ba",
        "**a**",
        "&amp;",
        "&fjlig;",
        "🙂",
        "中",
        "` a `",
        "[a](hidden)",
    ];
    for _ in 0..60 {
        let mut source = String::new();
        for _ in 0..20 {
            source.push_str(&"#".repeat(1 + random() % 6));
            source.push(' ');
            for _ in 0..8 {
                source.push_str(atoms[random() % atoms.len()]);
            }
            source.push_str("\n\nbody-only\n\n");
        }
        let doc = parse(&source);
        for _ in 0..8 {
            let heading = &doc.headings().entries()[random() % 20];
            let title: String = doc
                .display_runs_in(heading.content)
                .unwrap()
                .filter(|run| !run.style.contains(Style::HIDDEN))
                .map(|run| run.text)
                .collect();
            let boundaries: Vec<_> = title
                .char_indices()
                .map(|(i, _)| i)
                .chain([title.len()])
                .collect();
            let from = random() % (boundaries.len() - 1);
            let to = from + 1 + random() % (boundaries.len() - from - 1);
            let query = &title[boundaries[from]..boundaries[to]];
            let max_level = (random() % 8) as u8;
            let limit = WorkLimit {
                bytes: 6 + random() % 31,
                queries: 1 + random() % 4,
                objects: 1 + random() % 4,
                ..Default::default()
            };
            assert_eq!(
                search(&doc, query, max_level, limit).0,
                oracle(&doc, query, max_level),
                "seed={seed:#x}"
            );
        }
    }
}

#[test]
fn searching_hidden_names_preserves_folds_until_explicit_navigation() {
    let doc = parse("# Parent\n## a**b**&amp;c\nbody\n## Other\nbody\n");
    let mut view = FoldView::new(&doc, 1).unwrap();
    for id in 0..3 {
        view.set_collapsed(&doc, FoldId::Heading(HeadingId(id)), true)
            .unwrap();
    }
    let version = view.version();
    let hit = search(&doc, "ab&c", 6, tiny()).0.remove(0);
    assert_eq!(view.version(), version);
    let mut navigation = FoldNavigation::new(
        &doc,
        &view,
        &Navigator::new(),
        NavAction::Source(hit.source.start),
        FoldNavigationPolicy::Reveal,
        24,
    )
    .unwrap();
    for _ in 0..10 {
        let progress = navigation
            .advance(&doc, &mut view, &mut Budget::new(tiny()))
            .unwrap();
        if progress.completion == Completion::Covered {
            assert_eq!(progress.target.unwrap().source, Some(hit.source.start));
            assert!(!view.is_collapsed(FoldId::Heading(HeadingId(0))));
            assert!(view.is_collapsed(FoldId::Heading(HeadingId(1))));
            assert!(view.is_collapsed(FoldId::Heading(HeadingId(2))));
            return;
        }
    }
    panic!("search result navigation did not complete");
}
