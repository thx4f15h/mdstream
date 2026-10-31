use std::collections::BTreeSet;

use mdtui3::*;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn tiny() -> WorkLimit {
    WorkLimit {
        bytes: 6,
        queries: 1,
        objects: 1,
        ..Default::default()
    }
}

fn build(doc: &Document, limit: WorkLimit) -> (HeadingAnchors, WorkLimit) {
    let mut index = HeadingAnchors::new(doc);
    let mut total = WorkLimit::default();
    for _ in 0..(doc.source().len() + 1) * 30 {
        let mut budget = Budget::new(limit);
        let status = index.advance(doc, &mut budget).unwrap();
        let used = budget.used();
        assert!(used.bytes <= limit.bytes);
        assert!(used.queries <= limit.queries);
        assert!(used.objects <= limit.objects);
        assert_eq!((used.tiles, used.recovery, used.dependencies), (0, 0, 0));
        total.bytes += used.bytes;
        total.objects += used.objects;
        total.queries += used.queries;
        if status == Completion::EndOfDocument {
            assert!(index.is_complete());
            assert_eq!(index.indexed_headings(), doc.headings().entries().len());
            let mut empty = Budget::new(WorkLimit::default());
            assert_eq!(index.advance(doc, &mut empty), Ok(status));
            assert_eq!(empty.used(), WorkLimit::default());
            return (index, total);
        }
        assert_eq!(status, Completion::BudgetExhausted);
        assert_ne!(used, WorkLimit::default());
    }
    panic!("index made no progress");
}

fn slugs(doc: &Document, index: &HeadingAnchors) -> Vec<String> {
    (0..index.indexed_headings())
        .map(|id| {
            index
                .get(doc, HeadingId(id))
                .unwrap()
                .unwrap()
                .slug
                .to_string()
        })
        .collect()
}

// Independent, deliberately eager test model. Production must not flatten
// all titles or restart full-string collision checks from suffix 1 each time.
fn oracle(doc: &Document) -> Vec<String> {
    let mut used = BTreeSet::new();
    let mut result = Vec::new();
    for heading in doc.headings().entries() {
        let text: String = doc
            .display_runs_in(heading.content)
            .unwrap()
            .filter(|run| !run.style.contains(Style::HIDDEN))
            .map(|run| run.text)
            .collect();
        let mut base = String::new();
        for ch in text.chars() {
            match ch {
                '-' | '_' => base.push(ch),
                ch if ch.is_alphanumeric() => base.extend(ch.to_lowercase()),
                ch if ch.is_whitespace() => base.push('-'),
                _ => {}
            }
        }
        if base.is_empty() {
            base = "section".into();
        }
        let mut slug = base.clone();
        let mut n = 0;
        while !used.insert(slug.clone()) {
            n += 1;
            slug = format!("{base}-{n}");
        }
        result.push(slug);
    }
    result
}

fn lookup(
    doc: &Document,
    index: &HeadingAnchors,
    fragment: &str,
    limit: WorkLimit,
) -> Result<FragmentStep, LayoutError> {
    let mut task = index.fragment(fragment)?;
    for _ in 0..fragment.len() + 3 {
        let mut budget = Budget::new(limit);
        match task.advance(doc, &mut budget)? {
            FragmentStep::BudgetExhausted => assert_ne!(budget.used(), WorkLimit::default()),
            result => {
                let mut empty = Budget::new(WorkLimit::default());
                assert_eq!(task.advance(doc, &mut empty), Ok(result));
                assert_eq!(empty.used(), WorkLimit::default());
                return Ok(result);
            }
        }
    }
    panic!("fragment made no progress");
}

fn heading(doc: &Document, id: usize) -> FragmentStep {
    FragmentStep::Resolved(AnchorTarget {
        heading: Some(HeadingId(id)),
        source: doc.headings().entries()[id].source.start,
        provisional: !doc.is_finished(),
    })
}

#[test]
fn display_dialect_unicode_empty_and_global_suffix_collisions() {
    let doc = parse(concat!(
        "# A**B**&amp;C [Label](/secret)\n",
        "body-only\n\n",
        "# ` A  B `\n",
        "# 中文 É İ e\u{301} 𐐀 🙂\n",
        "# !!!\n# section\n# A\n# A-1\n# A\n# A-1\n# A\n#\n",
        "# &fjlig; &#x41;_Z\n",
        "```\n# code-only\n```\n> # quote-only\n- # list-only\n"
    ));
    let expected = [
        "abc-label",
        "a--b",
        "中文-é-i\u{307}-e-𐐨-",
        "section",
        "section-1",
        "a",
        "a-1",
        "a-2",
        "a-1-1",
        "a-3",
        "section-2",
        "fj-a_z",
        "quote-only",
        "list-only",
    ];
    for limit in [tiny(), WorkLimit::unlimited()] {
        let (index, _) = build(&doc, limit);
        assert_eq!(slugs(&doc, &index), expected);
        assert_eq!(slugs(&doc, &index), oracle(&doc));
        for (id, slug) in expected.iter().enumerate() {
            assert_eq!(
                lookup(&doc, &index, &format!("#{slug}"), tiny()),
                Ok(heading(&doc, id))
            );
        }
        for name in ["body-only", "secret", "code-only"] {
            assert_eq!(
                lookup(&doc, &index, &format!("#{name}"), tiny()),
                Ok(FragmentStep::NotFound)
            );
        }
    }
}

#[test]
fn fragment_decoding_is_once_case_sensitive_and_strict_utf8() {
    let doc = parse("# 中文\n# A B\n# A+B\n# 𐐀\n");
    let (index, _) = build(&doc, tiny());
    for fragment in ["#中文", "#%E4%B8%AD%E6%96%87", "#中%e6%96%87"] {
        assert_eq!(lookup(&doc, &index, fragment, tiny()), Ok(heading(&doc, 0)));
    }
    assert_eq!(lookup(&doc, &index, "#a%2Db", tiny()), Ok(heading(&doc, 1)));
    assert_eq!(
        lookup(&doc, &index, "#%F0%90%90%A8", tiny()),
        Ok(heading(&doc, 3))
    );
    assert_eq!(
        lookup(&doc, &index, "#", tiny()),
        Ok(FragmentStep::Resolved(AnchorTarget {
            heading: None,
            source: 0,
            provisional: false,
        }))
    );
    for fragment in ["#A-B", "#a+b", "#a%2Bb", "#a%252Db", "#a b", "#%00", "#a#b"] {
        assert_eq!(
            lookup(&doc, &index, fragment, tiny()),
            Ok(FragmentStep::NotFound)
        );
    }
    for fragment in [
        "",
        "a-b",
        "page#a-b",
        "#%",
        "#%2",
        "#%GG",
        "#%80",
        "#%C0%AF",
        "#%ED%A0%80",
        "#%F4%90%80%80",
        "#%F5%80%80%80",
        "#%E4%B8",
        "#%C2x",
        "#missing%FF",
        "#%E4中",
        "#%E4%B8%AD%",
    ] {
        assert_eq!(
            lookup(&doc, &index, fragment, tiny()),
            Err(LayoutError::InvalidRange),
            "{fragment}"
        );
    }
}

#[test]
fn partial_index_late_targets_revision_and_identity_contract() {
    let mut doc = Document::new();
    doc.append("# First\n").unwrap();
    let mut index = HeadingAnchors::new(&doc);
    assert_eq!(index.revision(), doc.revision());
    assert_eq!(index.get(&doc, HeadingId(0)), Ok(None));
    assert_eq!(
        index.get(&doc, HeadingId(1)),
        Err(LayoutError::InvalidRange)
    );
    assert_eq!(
        lookup(&doc, &index, "#first", tiny()),
        Ok(FragmentStep::IndexIncomplete)
    );
    index
        .advance(&doc, &mut Budget::new(WorkLimit::unlimited()))
        .unwrap();
    assert_eq!(lookup(&doc, &index, "#first", tiny()), Ok(heading(&doc, 0)));
    assert!(
        index
            .get(&doc, HeadingId(0))
            .unwrap()
            .unwrap()
            .target
            .provisional
    );
    assert_eq!(
        lookup(&doc, &index, "#later", tiny()),
        Ok(FragmentStep::Unresolved)
    );
    doc.append("").unwrap();
    assert_eq!(
        index.advance(&doc, &mut Budget::new(WorkLimit::default())),
        Ok(Completion::EndOfDocument)
    );
    let other = parse("# First\n");
    let mut budget = Budget::new(WorkLimit::unlimited());
    assert_eq!(
        index.advance(&other, &mut budget),
        Err(LayoutError::WrongDocument)
    );
    assert_eq!(
        index.get(&other, HeadingId(0)),
        Err(LayoutError::WrongDocument)
    );
    assert_eq!(
        index
            .fragment("#first")
            .unwrap()
            .advance(&other, &mut budget),
        Err(LayoutError::WrongDocument)
    );
    assert_eq!(budget.used(), WorkLimit::default());
    doc.append("# Later\n").unwrap();
    assert_eq!(
        index.advance(&doc, &mut budget),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(
        index.get(&doc, HeadingId(0)),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(
        index.fragment("#first").unwrap().advance(&doc, &mut budget),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(budget.used(), WorkLimit::default());
    let (mut index, _) = build(&doc, tiny());
    assert_eq!(lookup(&doc, &index, "#later", tiny()), Ok(heading(&doc, 1)));
    doc.finish();
    assert_eq!(
        index.advance(&doc, &mut budget),
        Err(LayoutError::StaleContinuation)
    );
    let (mut index, _) = build(&doc, tiny());
    doc.finish();
    assert_eq!(
        index.advance(&doc, &mut budget),
        Ok(Completion::EndOfDocument)
    );
    assert!(
        !index
            .get(&doc, HeadingId(1))
            .unwrap()
            .unwrap()
            .target
            .provisional
    );
    assert_eq!(
        lookup(&doc, &index, "#absent", tiny()),
        Ok(FragmentStep::NotFound)
    );
    let empty = parse("");
    let (index, work) = build(&empty, tiny());
    assert_eq!(work, WorkLimit::default());
    assert!(matches!(
        lookup(&empty, &index, "#", tiny()),
        Ok(FragmentStep::Resolved(_))
    ));
}

#[test]
fn budgets_are_shared_and_fragment_triplets_are_atomic() {
    let doc = parse("# 中\n# 中\n");
    let mut index = HeadingAnchors::new(&doc);
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
            index.advance(&doc, &mut budget),
            Ok(Completion::BudgetExhausted)
        );
        assert_eq!(budget.used(), WorkLimit::default());
        assert_eq!(index.indexed_headings(), 0);
    }
    let mut setup_only = Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        ..Default::default()
    });
    assert_eq!(
        index.advance(&doc, &mut setup_only),
        Ok(Completion::BudgetExhausted)
    );
    assert_eq!(setup_only.used().bytes, 0);
    assert_eq!(setup_only.used().queries, 1);
    loop {
        let mut budget = Budget::new(tiny());
        if index.advance(&doc, &mut budget).unwrap() == Completion::EndOfDocument {
            break;
        }
        let used = budget.used();
        assert_eq!(
            index.advance(&doc, &mut budget),
            Ok(Completion::BudgetExhausted)
        );
        assert_eq!(budget.used(), used);
    }
    let mut task = index.fragment("#%E4%B8%AD-1").unwrap();
    for limit in [
        WorkLimit::default(),
        WorkLimit {
            bytes: 2,
            ..WorkLimit::unlimited()
        },
        WorkLimit {
            objects: 0,
            ..WorkLimit::unlimited()
        },
    ] {
        let mut budget = Budget::new(limit);
        assert_eq!(
            task.advance(&doc, &mut budget),
            Ok(FragmentStep::BudgetExhausted)
        );
        assert_eq!(budget.used(), WorkLimit::default());
    }
    let mut no_query = Budget::new(WorkLimit {
        queries: 0,
        ..WorkLimit::unlimited()
    });
    assert_eq!(
        task.advance(&doc, &mut no_query),
        Ok(FragmentStep::BudgetExhausted)
    );
    assert_eq!(no_query.used().bytes, 11);
    assert_eq!(
        task.advance(&doc, &mut Budget::new(tiny())),
        Ok(heading(&doc, 1))
    );
}

#[test]
fn all_utf8_chunk_cuts_and_streaming_prefixes_match_snapshot_model() {
    let source =
        "# A&fjlig; **B**\n# A-1\n# A\n# A\n中文 &amp; X\n===\n# [Late][id]\n\n[id]: /target\n";
    for cut in (0..=source.len()).filter(|&i| source.is_char_boundary(i)) {
        let mut doc = Document::new();
        doc.append(&source[..cut]).unwrap();
        let (index, _) = build(&doc, tiny());
        assert_eq!(slugs(&doc, &index), oracle(&doc), "prefix {cut}");
        doc.append(&source[cut..]).unwrap();
        doc.finish();
        let (index, _) = build(&doc, tiny());
        assert_eq!(slugs(&doc, &index), oracle(&doc), "cut {cut}");
    }
    let source = format!(
        "# {} [Label][id]\n\n[id]: /url\n",
        "A".repeat(BLOCK_LOOKAHEAD + 20)
    );
    let mut doc = Document::new();
    for (i, ch) in source.char_indices() {
        doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
        if i >= BLOCK_LOOKAHEAD {
            let (index, _) = build(&doc, WorkLimit::unlimited());
            assert_eq!(slugs(&doc, &index), oracle(&doc), "long prefix {i}");
        }
    }
    doc.finish();
    let (index, _) = build(&doc, tiny());
    assert_eq!(slugs(&doc, &index), oracle(&doc));
}

#[test]
fn seeded_duplicate_model_and_percent_encoded_round_trips() {
    let mut seed = 0x86a7_2394_u64;
    let pieces = [
        "A", "a", "a-1", "a-2", "a-1-1", "!", "section", "中 文", "É", "&fjlig;", "**Ab**",
        "` A `", "İ", "e\u{301}",
    ];
    for _ in 0..60 {
        let mut source = String::new();
        for _ in 0..40 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            source.push_str(&format!(
                "# {}\n",
                pieces[(seed >> 32) as usize % pieces.len()]
            ));
        }
        let doc = parse(&source);
        let (index, _) = build(&doc, tiny());
        let expected = oracle(&doc);
        assert_eq!(slugs(&doc, &index), expected);
        for (id, slug) in expected.iter().enumerate() {
            let encoded = format!(
                "#{}",
                slug.bytes()
                    .map(|b| format!("%{b:02X}"))
                    .collect::<String>()
            );
            assert_eq!(
                lookup(&doc, &index, &encoded, tiny()),
                Ok(heading(&doc, id))
            );
        }
    }
}

#[test]
fn million_byte_setext_is_lazy_and_long_duplicate_and_fragment_are_budgeted() {
    let title = "x".repeat(1_000_000);
    let mut doc = Document::new();
    doc.append(&title).unwrap();
    let before = doc.work();
    doc.append("\n===\n").unwrap();
    let after = doc.work();
    assert!(after.classified_bytes - before.classified_bytes < 100);
    assert!(after.inline_bytes - before.inline_bytes < 100);
    let mut index = HeadingAnchors::new(&doc);
    assert_eq!(doc.work(), after);
    let mut budget = Budget::new(WorkLimit {
        bytes: 64,
        queries: 16,
        objects: 64,
        ..Default::default()
    });
    assert_eq!(
        index.advance(&doc, &mut budget),
        Ok(Completion::BudgetExhausted)
    );
    assert!(budget.used().bytes <= 64);
    assert_eq!(index.indexed_headings(), 0);
    doc.append(&format!("# {title}\n")).unwrap();
    doc.finish();
    let (index, used) = build(
        &doc,
        WorkLimit {
            bytes: 4096,
            queries: 256,
            objects: 4096,
            ..Default::default()
        },
    );
    assert_eq!(
        index.get(&doc, HeadingId(0)).unwrap().unwrap().slug.base,
        title
    );
    let second = index.get(&doc, HeadingId(1)).unwrap().unwrap();
    assert_eq!(second.slug.base, title);
    assert_eq!(second.slug.suffix, Some(1));
    assert!(used.bytes < doc.source().len() * 5);
    let fragment = format!("#{title}-1");
    let mut task = index.fragment(&fragment).unwrap();
    let mut total = 0;
    loop {
        let mut budget = Budget::new(WorkLimit {
            bytes: 64,
            objects: 64,
            queries: 1,
            ..Default::default()
        });
        let result = task.advance(&doc, &mut budget).unwrap();
        total += budget.used().bytes;
        if result != FragmentStep::BudgetExhausted {
            assert_eq!(result, heading(&doc, 1));
            break;
        }
    }
    assert_eq!(total, fragment.len() - 1);
}

#[test]
fn body_size_does_not_affect_index_work_and_resolution_preserves_folds() {
    let small = parse("# Parent\n## Target\nbody\n## Other\n");
    let large = parse(&format!(
        "# Parent\n## Target\n{}\n## Other\n",
        "b".repeat(1_000_000)
    ));
    let (_, a) = build(&small, tiny());
    let (index, b) = build(&large, tiny());
    assert_eq!(a, b);
    let mut view = FoldView::new(&large, 1).unwrap();
    for id in 0..3 {
        view.set_collapsed(&large, FoldId::Heading(HeadingId(id)), true)
            .unwrap();
    }
    let version = view.version();
    let FragmentStep::Resolved(target) = lookup(&large, &index, "#target", tiny()).unwrap() else {
        panic!()
    };
    assert_eq!(view.version(), version);
    let mut navigation = FoldNavigation::new(
        &large,
        &view,
        &Navigator::new(),
        NavAction::Source(target.source),
        FoldNavigationPolicy::Reveal,
        24,
    )
    .unwrap();
    loop {
        let mut budget = Budget::new(tiny());
        let result = navigation.advance(&large, &mut view, &mut budget).unwrap();
        assert_eq!(budget.used().bytes, 0);
        if result.completion == Completion::Covered {
            assert_eq!(result.target.unwrap().source, Some(target.source));
            break;
        }
    }
    assert!(!view.is_collapsed(FoldId::Heading(HeadingId(0))));
    assert!(view.is_collapsed(FoldId::Heading(HeadingId(1))));
    assert!(view.is_collapsed(FoldId::Heading(HeadingId(2))));
}
