use mdstream::*;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

// An intentionally whole-document oracle used only by tests.
fn oracle(doc: &Document, query: &str) -> Vec<DisplayMatch> {
    let mut text = Vec::new();
    let mut sources = Vec::new();
    for run in doc
        .display_runs_in(Span::new(0, doc.source().len()))
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
    {
        text.extend_from_slice(run.text.as_bytes());
        for i in 0..run.text.len() {
            sources.push(
                run.replacement
                    .unwrap_or(Span::new(run.source.start + i, run.source.start + i + 1)),
            );
        }
    }
    text.windows(query.len())
        .enumerate()
        .filter(|(_, bytes)| *bytes == query.as_bytes())
        .map(|(i, _)| DisplayMatch {
            display: i..i + query.len(),
            source: Span::new(sources[i].start, sources[i + query.len() - 1].end),
        })
        .collect()
}

fn search(doc: &Document, query: &str, limit: WorkLimit) -> (Vec<DisplayMatch>, WorkLimit) {
    let mut task = DisplaySearch::new(doc, query).unwrap();
    let mut found = Vec::new();
    let mut total = WorkLimit::default();
    for _ in 0..(doc.source().len() + 1) * 12 {
        let mut budget = Budget::new(limit);
        let completion = task.advance(doc, &mut budget, &mut found).unwrap();
        let used = budget.used();
        assert!(used.bytes <= limit.bytes);
        assert!(used.objects <= limit.objects);
        assert!(used.queries <= limit.queries);
        total.bytes += used.bytes;
        total.objects += used.objects;
        total.queries += used.queries;
        if completion == Completion::EndOfDocument {
            assert_eq!(task.source_position(), doc.source().len());
            let count = found.len();
            let mut empty = Budget::new(WorkLimit::default());
            assert_eq!(
                task.advance(doc, &mut empty, &mut found).unwrap(),
                Completion::EndOfDocument
            );
            assert_eq!(found.len(), count);
            assert_eq!(empty.used(), WorkLimit::default());
            for hit in &found {
                assert!(doc.text(hit.source).is_some());
            }
            return (found, total);
        }
        assert_eq!(completion, Completion::BudgetExhausted);
        assert_ne!(used, WorkLimit::default(), "search made no progress");
    }
    panic!("search did not finish");
}

fn tiny() -> WorkLimit {
    WorkLimit {
        bytes: 6,
        queries: 1,
        objects: 1,
        ..Default::default()
    }
}

#[test]
fn matches_decoded_text_and_maps_hidden_gaps_code_and_entity_parts() {
    let source =
        "a**b**&amp;c ` x\r\ny ` ![alt](private \"secret\") &NotEqualTilde; &fjlig; &#x1F642; 中文";
    let doc = parse(source);
    for (query, raw) in [
        ("ab&c", "a**b**&amp;c"),
        ("b&", "b**&amp;"),
        ("x y", "x\r\ny"),
        ("alt", "alt"),
        ("≂", "&NotEqualTilde;"),
        ("\u{338}", "&NotEqualTilde;"),
        ("f", "&fjlig;"),
        ("j", "&fjlig;"),
        ("fj", "&fjlig;"),
        ("🙂", "&#x1F642;"),
        ("中文", "中文"),
    ] {
        let hits = search(&doc, query, tiny()).0;
        assert_eq!(hits.len(), 1, "{query:?}");
        assert_eq!(doc.text(hits[0].source), Some(raw));
        assert_eq!(hits, oracle(&doc, query));
    }
    assert_eq!(search(&doc, "ab&c", tiny()).0[0].display, 0..4);
    for hidden in ["private", "secret", "amp", "**", "NotEqualTilde", "`"] {
        assert!(search(&doc, hidden, tiny()).0.is_empty(), "{hidden:?}");
    }
    let crlf = parse("`a\r\nb`");
    assert_eq!(
        search(&crlf, " ", tiny()).0,
        [DisplayMatch {
            display: 1..2,
            source: Span::new(2, 4),
        }]
    );
}

#[test]
fn overlapping_matches_cross_runs_tiles_and_large_queries() {
    for (source, query) in [
        ("a**ba**ba".to_owned(), "aba".to_owned()),
        ("abababacababaca".to_owned(), "ababaca".to_owned()),
        ("🙂**中**🙂中🙂".to_owned(), "🙂中🙂".to_owned()),
        (
            format!("{}&amp;中{}", "x".repeat(TILE_BYTES - 1), "y".repeat(600)),
            format!("{}&中{}", "x".repeat(200), "y".repeat(300)),
        ),
        ("a".repeat(600), "a".repeat(300)),
    ] {
        let doc = parse(&source);
        let expected = oracle(&doc, &query);
        assert!(!expected.is_empty());
        for limit in [tiny(), WorkLimit::unlimited()] {
            assert_eq!(search(&doc, &query, limit).0, expected);
        }
    }
}

#[test]
fn text_stream_keeps_literal_contexts_and_adds_no_layout_separators() {
    for (source, query, count) in [
        ("&amp; `&amp;` \\&amp;", "amp", 2),
        ("foo\nbar", "foo\nbar", 1),
        ("foo\nbar", "foo bar", 0),
        ("x &NewLine;y", "x \ny", 1),
        ("![alt](hidden)", "alt", 1),
        ("A &AMP; é", "a", 0),
        ("é", "e\u{301}", 0),
    ] {
        let doc = parse(source);
        let found = search(&doc, query, tiny()).0;
        assert_eq!(found.len(), count, "{source:?}, {query:?}");
        assert_eq!(found, oracle(&doc, query));
    }
    // Table borders have no text in the display stream; renderers may draw
    // their own separators, but the search must not invent them.
    let doc = parse("|a|b|\n|---|---|\n|c|d|\n");
    let found = search(&doc, "ab", tiny()).0;
    assert_eq!(found.len(), 1);
    assert_eq!(doc.text(found[0].source), Some("a|b"));
    assert_eq!(found, oracle(&doc, "ab"));
}

#[test]
fn shared_budget_pauses_before_emission_without_losing_buffered_bytes() {
    let doc = parse("aaa");
    let mut task = DisplaySearch::new(&doc, "a").unwrap();
    let mut out = Vec::new();
    // Reading consumes the only object unit, leaving a pending match.
    let mut budget = Budget::new(WorkLimit {
        bytes: 20,
        queries: 2,
        objects: 1,
        ..Default::default()
    });
    assert_eq!(
        task.advance(&doc, &mut budget, &mut out).unwrap(),
        Completion::BudgetExhausted
    );
    assert_eq!(task.source_position(), 3);
    assert_eq!(task.display_position(), 0);
    assert!(out.is_empty());
    let used = budget.used();
    assert_eq!(
        task.advance(&doc, &mut budget, &mut out).unwrap(),
        Completion::BudgetExhausted
    );
    assert_eq!(budget.used(), used);
    for position in 1..=3 {
        let mut budget = Budget::new(WorkLimit {
            bytes: 1,
            objects: 1,
            ..Default::default()
        });
        assert_eq!(
            task.advance(&doc, &mut budget, &mut out).unwrap(),
            if position == 3 {
                Completion::EndOfDocument
            } else {
                Completion::BudgetExhausted
            }
        );
        assert_eq!(task.display_position(), position);
        assert_eq!(out.len(), position);
        assert_eq!(budget.used().bytes, 1);
        assert_eq!(budget.used().objects, 1);
        assert_eq!(budget.used().queries, 0);
    }
    assert_eq!(out, oracle(&doc, "a"));
}

#[test]
fn each_budget_dimension_and_empty_input_have_explicit_behavior() {
    let doc = parse("🙂");
    let mut task = DisplaySearch::new(&doc, "🙂").unwrap();
    for limit in [
        WorkLimit::default(),
        WorkLimit {
            bytes: 5,
            ..WorkLimit::unlimited()
        },
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
            task.advance(&doc, &mut budget, &mut Vec::new()).unwrap(),
            Completion::BudgetExhausted
        );
        assert_eq!(task.source_position(), 0);
        assert_eq!(task.display_position(), 0);
        assert_eq!(budget.used(), WorkLimit::default());
    }
    assert_eq!(search(&doc, "🙂", tiny()).0.len(), 1);
    let empty = Document::new();
    assert!(DisplaySearch::new(&empty, "").is_err());
    assert!(search(&empty, "a", WorkLimit::default()).0.is_empty());
    assert!(
        search(&parse("[hidden]: /url\n"), "hidden", tiny())
            .0
            .is_empty()
    );
}

#[test]
fn revision_and_identity_are_checked_even_with_pending_text_or_completed_search() {
    let mut doc = Document::new();
    doc.append("aaa").unwrap();
    let mut task = DisplaySearch::new(&doc, "a").unwrap();
    assert_eq!(task.revision(), doc.revision());
    let mut out = Vec::new();
    task.advance(
        &doc,
        &mut Budget::new(WorkLimit {
            bytes: 5,
            queries: 1,
            objects: 1,
            ..Default::default()
        }),
        &mut out,
    )
    .unwrap();
    assert_eq!(task.source_position(), 3);
    assert_eq!(task.display_position(), 0);
    let other = parse("aaa");
    let mut budget = Budget::new(WorkLimit::unlimited());
    assert_eq!(
        task.advance(&other, &mut budget, &mut out),
        Err(LayoutError::WrongDocument)
    );
    assert_eq!(budget.used(), WorkLimit::default());
    doc.append("b").unwrap();
    assert_eq!(
        task.advance(&doc, &mut budget, &mut out),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(budget.used(), WorkLimit::default());
    assert!(out.is_empty());
    assert_eq!(task.display_position(), 0);

    let mut completed = DisplaySearch::new(&doc, "a").unwrap();
    completed.advance(&doc, &mut budget, &mut out).unwrap();
    assert_eq!(out.len(), 3);
    doc.append("").unwrap();
    assert_eq!(
        completed.advance(&doc, &mut budget, &mut out).unwrap(),
        Completion::EndOfDocument
    );
    doc.finish();
    let before = out.clone();
    let mut empty = Budget::new(WorkLimit::default());
    assert_eq!(
        completed.advance(&doc, &mut empty, &mut out),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(out, before);
    let mut finished = DisplaySearch::new(&doc, "a").unwrap();
    doc.finish();
    assert_eq!(
        finished
            .advance(&doc, &mut budget, &mut Vec::new())
            .unwrap(),
        Completion::EndOfDocument
    );
}

#[test]
fn late_semantic_changes_require_explicit_restart() {
    let prefix = "x".repeat(BLOCK_LOOKAHEAD);
    for (pending, suffix, query, raw) in [
        ("&amp", ";", "&", "&amp;"),
        ("` a ", "` ", "a", "a"),
        ("**word", "** ", "word", "word"),
        ("[label]", "(/target) ", "label", "label"),
    ] {
        let mut doc = Document::new();
        doc.append(&format!("{prefix}{pending}")).unwrap();
        let mut task = DisplaySearch::new(&doc, query).unwrap();
        let mut old = Vec::new();
        task.advance(&doc, &mut Budget::new(WorkLimit::unlimited()), &mut old)
            .unwrap();
        doc.append(suffix).unwrap();
        let mut budget = Budget::new(WorkLimit::unlimited());
        assert_eq!(
            task.advance(&doc, &mut budget, &mut old),
            Err(LayoutError::StaleContinuation)
        );
        assert_eq!(budget.used(), WorkLimit::default());
        let fresh = search(&doc, query, tiny()).0;
        assert_eq!(fresh, oracle(&doc, query));
        assert_eq!(fresh.len(), 1);
        assert_eq!(doc.text(fresh[0].source), Some(raw));
    }
}

#[test]
fn each_live_prefix_and_chunk_split_matches_the_display_oracle() {
    let prefix = "x".repeat(BLOCK_LOOKAHEAD);
    let suffix = "&amp; **a** ` a\r\n中 ` &NotEqualTilde; [a](hidden)\n===\n";
    let source = format!("{prefix}{suffix}");
    let queries = ["&", "a a 中", "≂\u{338}", "a", "hidden", "**", "x&"];
    let mut doc = Document::new();
    doc.append(&prefix).unwrap();
    for c in suffix.chars() {
        doc.append(c.encode_utf8(&mut [0; 4])).unwrap();
        for query in queries {
            assert_eq!(search(&doc, query, tiny()).0, oracle(&doc, query));
        }
    }
    let whole = parse(&source);
    for split in suffix.char_indices().map(|(i, _)| prefix.len() + i) {
        let mut doc = Document::new();
        doc.append(&source[..split]).unwrap();
        doc.append(&source[split..]).unwrap();
        doc.finish();
        for query in queries {
            assert_eq!(search(&doc, query, tiny()).0, oracle(&whole, query));
        }
    }
}

#[test]
fn deterministic_generated_queries_and_budget_slices_match_naive_search() {
    let atoms = [
        "a",
        "ba",
        "**a**",
        "&amp;",
        "&fjlig;",
        "&NotEqualTilde;",
        "🙂",
        "中",
        "` a\r\nb `",
        "[a](hidden)",
        "\n\n",
    ];
    let mut seed = 0x747e_826b_2f17_3219_u64;
    let mut random = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed as usize
    };
    for _ in 0..80 {
        let source: String = (0..30).map(|_| atoms[random() % atoms.len()]).collect();
        let doc = parse(&source);
        let text: String = doc
            .display_runs_in(Span::new(0, source.len()))
            .unwrap()
            .filter(|r| !r.style.contains(Style::HIDDEN))
            .map(|r| r.text)
            .collect();
        let boundaries: Vec<_> = text
            .char_indices()
            .map(|(i, _)| i)
            .chain([text.len()])
            .collect();
        for _ in 0..8 {
            let from = random() % (boundaries.len() - 1);
            let to = from + 1 + random() % (boundaries.len() - from - 1);
            let query = &text[boundaries[from]..boundaries[to]];
            let limit = WorkLimit {
                bytes: 6 + random() % 31,
                queries: 1 + random() % 4,
                objects: 1 + random() % 4,
                ..Default::default()
            };
            assert_eq!(
                search(&doc, query, limit).0,
                oracle(&doc, query),
                "seed={seed:#x}",
                seed = 0x747e_826b_2f17_3219_u64
            );
        }
    }
}

#[test]
fn failure_fallback_and_large_hidden_regions_have_bounded_work() {
    let mut previous = 0;
    for size in [4096, 8192, 16384] {
        let doc = parse(&"a".repeat(size));
        let query = format!("{}b", "a".repeat(2000));
        let (hits, used) = search(&doc, &query, tiny());
        assert!(hits.is_empty());
        assert!(used.bytes <= size * 5 + 6, "{} for {size}", used.bytes);
        assert!(used.queries <= size);
        if previous > 0 {
            assert!(used.bytes <= previous * 3);
        }
        previous = used.bytes;
    }
    for size in [4096, 1_000_000] {
        let doc = parse(&format!("```{}\nbody\n```\n", "x".repeat(size)));
        let before = doc.work();
        let mut task = DisplaySearch::new(&doc, "body").unwrap();
        let mut budget = Budget::new(WorkLimit {
            bytes: 32,
            queries: 2,
            objects: 2,
            ..Default::default()
        });
        let mut out = Vec::new();
        assert_eq!(
            task.advance(&doc, &mut budget, &mut out).unwrap(),
            Completion::BudgetExhausted
        );
        assert!(task.source_position() <= 32);
        assert!(out.is_empty());
        assert_eq!(doc.work(), before);
        assert!(budget.used().bytes <= 32);
        assert!(budget.used().queries <= 2);
    }
}
