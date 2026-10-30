use mdtui3::*;

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
fn unlimited() -> Budget {
    Budget::new(WorkLimit::unlimited())
}
fn h(i: usize) -> FoldId {
    FoldId::Heading(HeadingId(i))
}

#[test]
fn confirmation_preserves_physical_leaf_ids_and_publishes_one_heading() {
    let mut doc = Document::new();
    doc.append("> - [x] first\r\n>   second\nthird\r\n")
        .unwrap();
    let blocks = doc.blocks().to_vec();
    let before = doc.work();
    let update = doc.append(">   ===\r\n").unwrap();
    assert_eq!(update.new_blocks, 3..3);
    assert_eq!(update.new_headings, 0..1);
    assert_eq!(update.changed_blocks, 0..3);
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    let title = doc.headings().entries()[0];
    assert_eq!(title.block, BlockId(0));
    assert_eq!(title.container, Some(ContainerId(2)));
    assert_eq!(title.source, Span::new(0, doc.source().len()));
    assert_eq!(visible(&doc, title.content), "first\nsecond\nthird");
    assert_eq!(doc.blocks()[0].source, blocks[0].source);
    assert_eq!(doc.blocks()[0].kind, BlockKind::Heading { level: 1 });
    assert_eq!(doc.blocks()[1], blocks[1]);
    assert_eq!(doc.blocks()[2].source.start, blocks[2].source.start);
    assert_eq!(doc.blocks()[2].source.end, title.source.end);
    assert_eq!(doc.blocks()[2].kind, blocks[2].kind);
    for (i, block) in doc.blocks().iter().enumerate() {
        assert_eq!(doc.block_at(block.source.start), Some(BlockId(i)));
        assert_eq!(block.state, SyntaxState::Sealed);
        if i > 0 {
            assert_eq!(doc.blocks()[i - 1].source.end, block.source.start);
        }
    }
    assert!(matches!(
        doc.container(ContainerId(2)).unwrap().kind,
        ContainerKind::Item {
            task: Some(TaskMarker { checked: true, .. }),
            ..
        }
    ));
    let update = doc.append(">   body\n>   next\n>   ---\n").unwrap();
    assert_eq!(update.new_headings, 1..2);
    assert_eq!(doc.headings().entries()[0], title);
    assert_eq!(doc.headings().parent(HeadingId(1)), Some(HeadingId(0)));
    assert!(doc.finish().new_headings.is_empty());
    assert!(doc.finish().changed_blocks.is_empty());
}

#[test]
fn confirmed_inline_objects_and_source_maps_survive_promotion() {
    let mut doc = Document::new();
    doc.append("> **中\r\n> 🙂** [a\n> b](u) ![c\n> d](img) ` e\n> f ` $g\n> h$ &amp;\\\n> last\n")
        .unwrap();
    let links: Vec<_> = (0..doc.links().len())
        .map(|i| *doc.link_info(LinkId(i)).unwrap())
        .collect();
    let maths = doc.maths().to_vec();
    let end = doc.source().len() - 1;
    let expected = visible(&doc, Span::new(2, end));
    let before = doc.work();
    let update = doc.append("> ---\n").unwrap();
    assert!(update.new_links.is_empty());
    assert!(update.new_maths.is_empty());
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert_eq!(visible(&doc, doc.headings().entries()[0].content), expected);
    assert_eq!(doc.maths(), maths);
    for (i, old) in links.iter().enumerate() {
        assert_eq!(doc.link_info(LinkId(i)), Some(old));
    }
    assert_eq!(doc.link_destination(LinkId(0)), Some("u"));
    assert_eq!(doc.link_destination(LinkId(1)), Some("img"));
    for (span, style) in [
        (Span::new(4, 7), Style::STRONG),
        (
            Span::new(
                doc.source().find("\\\n").unwrap() + 1,
                doc.source().find("\\\n").unwrap() + 2,
            ),
            Style::HARD_BREAK,
        ),
    ] {
        assert!(doc.runs_in(span).unwrap().any(|r| r.style.contains(style)));
    }
}

#[test]
fn boundaries_reset_sequences_and_underline_requires_explicit_exact_scope() {
    for barrier in [
        ">\n",
        "> # atx\n",
        ">     code\n",
        "> [r]: /u\n",
        "> [^n]: raw\n",
        "> ```\n> raw\n> ```\n",
        "> > # nested\n",
        "> a|b\n> -|-\n>\n",
    ] {
        let source = format!("> old\n> old two\n{barrier}> new\n> tail\n> ===\n");
        let doc = parse(&source);
        let last = doc.headings().entries().last().unwrap();
        assert_eq!(
            last.source.start,
            source.find("> new").unwrap(),
            "{barrier:?}"
        );
        assert_eq!(visible(&doc, last.content), "new\ntail");
    }
    for source in [
        "> a\n> b\n===\n",
        "> a\n> b\n---\n",
        "> a\n> b\n> > ===\n",
        "- a\n  b\n-\n",
        "> a\n> b\n>     ===\n",
    ] {
        assert!(parse(source).headings().entries().is_empty(), "{source:?}");
    }
    let doc = parse("- first\n  tail\n- second\n  next\n  -\n");
    assert_eq!(
        visible(&doc, doc.headings().entries()[0].content),
        "second\nnext"
    );
}

#[test]
fn invalid_and_overlong_underlines_remain_text_and_eof_confirms_only_complete_syntax() {
    for bad in [
        "==x".to_owned(),
        "=--".to_owned(),
        "=".repeat(BLOCK_LOOKAHEAD + 1),
    ] {
        let source = format!("> first\n> second\n> {bad}\n");
        let mut doc = Document::new();
        doc.append(&source).unwrap();
        assert!(doc.headings().entries().is_empty());
        let before = doc.work();
        doc.append("> ===\n").unwrap();
        assert_eq!(doc.work().inline_bytes, before.inline_bytes);
        assert_eq!(
            visible(&doc, doc.headings().entries()[0].content),
            format!("first\nsecond\n{bad}")
        );
    }
    let mut doc = Document::new();
    doc.append("> first\n> second\n> ===").unwrap();
    assert!(doc.headings().entries().is_empty());
    assert_eq!(doc.finish().new_headings, 0..1);
    assert_eq!(
        visible(&doc, doc.headings().entries()[0].content),
        "first\nsecond"
    );
    let doc = parse("> *open\n> still\n> ===\n> close*\n");
    assert!(
        doc.runs_in(Span::new(0, doc.source().len()))
            .unwrap()
            .all(|r| !r.style.contains(Style::EMPHASIS))
    );
}

#[test]
fn pending_table_headers_join_on_failure_and_reset_on_confirmation() {
    for body in [
        "*first\n> last*|b".to_owned(),
        format!("a\n> |{}|b", "x".repeat(2048)),
        format!("*first\n> {}*|b", "x".repeat(2048)),
        format!("`first\n> {}|b", "x".repeat(2048)),
        format!("$first\n> {}|b", "x".repeat(2048)),
        format!("[first\n> {}|b", "x".repeat(2048)),
    ] {
        let source = format!("> {body}\n");
        let plain = parse(&source);
        let expected = visible(&plain, Span::new(2, source.len() - 1));
        let doc = parse(&format!("{source}> ===\n"));
        assert!(doc.tables().is_empty());
        assert_eq!(doc.headings().entries()[0].block, BlockId(0));
        assert_eq!(visible(&doc, doc.headings().entries()[0].content), expected);
        assert!(doc.work().inline_bytes <= doc.source().len() as u64);
    }
    let doc = parse("> before\n> a|b\n> -|-\n> row|cell\n>\n> first\n> tail\n> ===\n");
    assert_eq!(doc.tables().len(), 1);
    assert_eq!(
        visible(&doc, doc.headings().entries()[0].content),
        "first\ntail"
    );
    let source = format!("> first\n> a|b\n> -|{}x\n> ===\n", "-".repeat(2048));
    let doc = parse(&source);
    assert!(doc.tables().is_empty());
    assert_eq!(doc.headings().entries()[0].source.start, 0);
    assert!(visible(&doc, doc.headings().entries()[0].content).ends_with('x'));
}

#[test]
fn promotion_invalidates_old_readers_and_measurements_and_merges_deltas() {
    let mut doc = Document::new();
    let mut batch = doc
        .append(&format!("> first\n> {}\n> last\n", "x".repeat(9000)))
        .unwrap();
    let blocks = doc.blocks().to_vec();
    let mut heights = HeightIndex::default();
    heights.apply(&doc, &batch).unwrap();
    let ticket = heights.ticket(TileId(0)).unwrap();
    let key = DependencyKey::capture(&doc, TileId(0), &mut unlimited())
        .unwrap()
        .unwrap();
    let mut search = HeadingSearch::new(&doc, "first", 6).unwrap();
    let mut anchors = HeadingAnchors::new(&doc);
    let mut candidates = doc
        .fold_candidates(Span::new(0, doc.source().len()))
        .unwrap();
    let update = doc.append("> ---\n").unwrap();
    for (i, old) in blocks.iter().enumerate() {
        if old != &doc.blocks()[i] {
            assert!(update.changed_blocks.contains(&i));
        }
    }
    assert!(
        update
            .changes
            .iter()
            .any(|c| c.source.start == 0 && c.source.end == doc.source().len())
    );
    heights.apply(&doc, &update).unwrap();
    assert!(heights.measure(ticket, 42).is_err());
    assert_eq!(
        key.validate(&doc, &mut unlimited()).unwrap(),
        CacheValidity::Changed
    );
    let mut empty = Budget::new(WorkLimit::default());
    assert_eq!(
        search.advance(&doc, &mut empty, &mut Vec::new()),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(
        anchors.advance(&doc, &mut empty),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(
        candidates.advance(&doc, &mut empty, &mut Vec::new()),
        Err(LayoutError::StaleContinuation)
    );
    batch.merge(update).unwrap();
    let update = doc.finish();
    heights.apply(&doc, &update).unwrap();
    batch.merge(update).unwrap();
    let mut rebuilt = HeightIndex::default();
    rebuilt.apply(&doc, &batch).unwrap();
    assert_eq!(heights.total_rows(), rebuilt.total_rows());
    assert_eq!(batch.new_headings, 0..1);
    assert_eq!(batch.changed_blocks, 0..3);
}

#[test]
fn fold_discovery_navigation_defaults_and_preview_use_the_full_title() {
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 1).unwrap();
    view.set_defaults(FoldDefaults {
        heading_levels: [true; 6],
        ..Default::default()
    });
    for chunk in [
        "> # old\n",
        "> first\n",
        "> second\n",
        "> ===\n",
        "> body\n",
        "> ## child\n> tail\n",
    ] {
        let update = doc.append(chunk).unwrap();
        view.apply(&doc, &update).unwrap();
    }
    let title = doc.headings().entries()[1];
    assert_eq!(
        doc.fold_candidate(h(0)).unwrap().source.end,
        title.source.start
    );
    assert!(view.is_collapsed(h(0)) && view.is_collapsed(h(1)));
    assert_eq!(
        doc.fold_candidate(h(1)).unwrap().hidden.start,
        title.source.end
    );
    for at in [
        title.source.start,
        title.content.start,
        doc.source().find("second").unwrap(),
    ] {
        assert_eq!(doc.headings().current(at), Some(HeadingId(1)));
        assert_eq!(
            view.hidden_at(&doc, at, &mut unlimited()).unwrap(),
            FoldLookup::Visible
        );
        assert_eq!(doc.fold_candidate_at(at).unwrap().id, h(1));
    }
    let mut cursor = doc
        .fold_candidates(doc.blocks()[title.block.0].source)
        .unwrap();
    let mut out = Vec::new();
    cursor.advance(&doc, &mut unlimited(), &mut out).unwrap();
    assert_eq!(out.iter().filter(|c| c.id == h(1)).count(), 1);
    let body = doc.source().find("> body").unwrap();
    assert!(
        matches!(view.hidden_at(&doc, body, &mut unlimited()).unwrap(), FoldLookup::Hidden(c) if c.id == h(1))
    );
    let mut nav = Navigator::new();
    assert_eq!(
        nav.navigate(&doc, view.heights(), NavAction::Heading(HeadingId(1)), 2)
            .unwrap()
            .source,
        Some(title.source.start)
    );
    assert_eq!(
        view.set_preview(&doc, Some(h(1)), &mut Budget::new(WorkLimit::default()))
            .unwrap()
            .completion,
        Completion::BudgetExhausted
    );
    view.set_preview(&doc, Some(h(1)), &mut unlimited())
        .unwrap();
    assert_eq!(
        view.hidden_at(&doc, body, &mut unlimited()).unwrap(),
        FoldLookup::Visible
    );
    view.set_preview(&doc, None, &mut unlimited()).unwrap();
    assert!(view.is_collapsed(h(1)));
    view.set_collapsed(&doc, FoldId::Container(ContainerId(0)), true)
        .unwrap();
    assert!(
        matches!(view.hidden_at(&doc, title.source.start, &mut unlimited()).unwrap(), FoldLookup::Hidden(c) if c.id == FoldId::Container(ContainerId(0)))
    );
}

#[test]
fn search_and_slug_read_multiline_titles_with_shared_budget_and_mapping() {
    let doc = parse("> **中**\r\n> 🙂 &amp; last\n> ---\n");
    let mut search = HeadingSearch::new(&doc, "中\n🙂", 6).unwrap();
    let mut anchors = HeadingAnchors::new(&doc);
    let mut hits = Vec::new();
    let mut frames = 0;
    loop {
        frames += 1;
        let mut budget = Budget::new(WorkLimit {
            bytes: 6,
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        let a = anchors.advance(&doc, &mut budget).unwrap();
        let s = search.advance(&doc, &mut budget, &mut hits).unwrap();
        assert!(
            budget.used().bytes <= 6 && budget.used().objects <= 1 && budget.used().queries <= 1
        );
        if a == Completion::EndOfDocument && s == Completion::EndOfDocument {
            break;
        }
        assert!(frames < 200);
    }
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].heading, HeadingId(0));
    assert_eq!(
        anchors
            .get(&doc, HeadingId(0))
            .unwrap()
            .unwrap()
            .slug
            .to_string(),
        "中---last"
    );
}

#[test]
fn deep_million_byte_continuation_and_confirmation_never_rescan_old_text() {
    for (first, next) in [
        (">".repeat(1000), ">".repeat(1000)),
        ("- ".repeat(250), " ".repeat(500)),
    ] {
        let mut doc = Document::new();
        doc.append(&format!("{first}first\n{next}{}", "x".repeat(1_000_000)))
            .unwrap();
        let before = doc.work();
        let arena = doc.annotation_arena_stats();
        for _ in 0..1000 {
            doc.append("x").unwrap();
        }
        assert_eq!(doc.work().inline_bytes - before.inline_bytes, 1000);
        assert_eq!(doc.work().classified_bytes, before.classified_bytes);
        assert_eq!(doc.work().container_steps, before.container_steps);
        assert_eq!(doc.work().heading_steps, before.heading_steps);
        assert_eq!(
            doc.annotation_arena_stats().allocation_calls,
            arena.allocation_calls
        );
        doc.append(&format!("\n{next}last\n")).unwrap();
        let middle = doc.blocks()[1].clone();
        let before = doc.work();
        let update = doc.append(&format!("{next}=\n")).unwrap();
        assert_eq!(update.new_headings, 0..1);
        assert_eq!(update.changed_blocks.start, 0);
        assert_eq!(doc.work().inline_bytes, before.inline_bytes);
        assert_eq!(doc.work().heading_steps - before.heading_steps, 2);
        assert!(doc.work().tree_visits - before.tree_visits < 1000);
        assert_eq!(doc.blocks()[1], middle);
        assert_eq!(doc.headings().entries()[0].source.start, 0);
        assert!(doc.headings().entries()[0].content.len() > 1_001_000);
    }
}

#[test]
fn thousands_of_lazy_leaves_confirm_without_rewriting_intermediate_blocks() {
    let mut doc = Document::new();
    let prefix = ">".repeat(1000);
    doc.append(&format!("{prefix}first\n")).unwrap();
    let before = doc.work();
    for _ in 0..4096 {
        doc.append("x\n").unwrap();
    }
    assert_eq!(doc.work().container_steps - before.container_steps, 4096);
    let blocks = doc.blocks().to_vec();
    let before = doc.work();
    doc.append(&format!("{prefix}=\n")).unwrap();
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert_eq!(doc.work().heading_steps - before.heading_steps, 2);
    assert!(doc.work().tree_visits - before.tree_visits < 1000);
    assert_eq!(&doc.blocks()[1..4096], &blocks[1..4096]);
    assert_eq!(doc.blocks().len(), blocks.len());
    assert_eq!(doc.headings().entries()[0].block, BlockId(0));
}

#[test]
fn generated_titles_match_prefix_free_display_and_independent_scope_ranges() {
    let tokens = [
        "**a", "b**", "`c", "d`", "$e", "f$", "[g", "h](u)", "&amp;", "🙂",
    ];
    for i in 0..400 {
        let depth = i % 8 + 1;
        let prefix = ">".repeat(depth);
        let a = tokens[i % 10];
        let b = tokens[i / 10 % 10];
        let body = format!("{a}\r\n{b}\n");
        let plain = parse(&format!("{body}===\n"));
        let source = format!(
            "# root\n{prefix}# prior\n{prefix}{a}\r\n{prefix}{b}\n{prefix}===\n{prefix}body\n{prefix}> # nested\n{prefix}# peer\n"
        );
        let doc = parse(&source);
        let headings = doc.headings();
        let title = headings.entries()[2];
        assert_eq!(
            visible(&doc, title.content),
            visible(&plain, plain.headings().entries()[0].content),
            "{i}"
        );
        let start = source.find(&format!("{prefix}{a}\r\n")).unwrap();
        let end = source.find(&format!("{prefix}body")).unwrap();
        let peer = source.find(&format!("{prefix}# peer")).unwrap();
        assert_eq!(title.source, Span::new(start, end));
        assert_eq!(title.container, Some(ContainerId(depth - 1)));
        assert_eq!(
            headings.section(HeadingId(1), source.len()).unwrap().end,
            start
        );
        assert_eq!(
            headings.section(HeadingId(2), source.len()),
            Some(Span::new(start, peer))
        );
        assert_eq!(headings.next_sibling(HeadingId(2)), Some(HeadingId(4)));
        assert_eq!(headings.parent(HeadingId(2)), None);
        let fold = doc.fold_candidate(h(2)).unwrap();
        assert_eq!(fold.hidden, Span::new(end, peer));
        assert_eq!(fold.parent, Some(FoldId::Container(ContainerId(depth - 1))));
    }
}

#[test]
fn many_historical_headings_do_not_add_confirmation_or_fold_refresh_work() {
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 1).unwrap();
    view.set_defaults(FoldDefaults {
        heading_levels: [true; 6],
        ..Default::default()
    });
    let update = doc.append(&"> a\n> b\n> =\n".repeat(4096)).unwrap();
    view.apply(&doc, &update).unwrap();
    let update = doc.append("> next\n> line\n").unwrap();
    view.apply(&doc, &update).unwrap();
    let before = doc.work();
    let visits = view.range_visits();
    let defaults = view.default_visits();
    let update = doc.append("> =\n").unwrap();
    view.apply(&doc, &update).unwrap();
    assert_eq!(update.new_headings, 4096..4097);
    assert_eq!(update.changed_blocks, 8192..8194);
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert_eq!(doc.work().heading_steps - before.heading_steps, 2);
    assert!(doc.work().tree_visits - before.tree_visits < 1000);
    assert!(view.range_visits() - visits <= 8);
    assert_eq!(view.default_visits() - defaults, 1);
    assert!(view.is_collapsed(h(0)) && view.is_collapsed(h(4096)));
    assert_eq!(
        doc.fold_candidate(h(4095)).unwrap().source.end,
        doc.headings().entries()[4096].source.start
    );
}
