use mdstream::*;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn containers(doc: &Document) -> Vec<Container> {
    (0..doc.container_count())
        .map(|i| doc.container(ContainerId(i)).unwrap())
        .collect()
}

fn lists(doc: &Document) -> Vec<ListTightness> {
    containers(doc)
        .into_iter()
        .filter_map(|c| c.list_tightness)
        .collect()
}

#[test]
fn omitted_indent_retains_list_and_item_until_the_next_classified_line() {
    let mut doc = Document::new();
    doc.append("- first\n").unwrap();
    let update = doc.append("\n \r\n\n").unwrap();
    assert!(update.closed_containers.is_empty());
    assert!(update.loosened_lists.is_empty());
    assert_eq!(doc.container_count(), 2);
    assert_eq!(doc.container(ContainerId(1)).unwrap().source.end, 13);
    assert!(
        doc.blocks()
            .iter()
            .all(|b| b.container == Some(ContainerId(1)))
    );
    let update = doc.append("- second").unwrap();
    assert!(update.closed_containers.is_empty());
    assert_eq!(lists(&doc), [ListTightness::Tight]);
    let update = doc.finish();
    assert_eq!(update.loosened_lists, [ContainerId(0)]);
    assert_eq!(update.new_containers, 2..3);
    assert_eq!(
        update.closed_containers,
        [ContainerId(1), ContainerId(2), ContainerId(0)]
    );
    assert_eq!(lists(&doc), [ListTightness::Loose]);
    assert_eq!(doc.container(ContainerId(1)).unwrap().source.end, 13);
}

#[test]
fn nested_omitted_lists_resolve_only_the_resumed_direct_scope() {
    for (source, expected) in [
        (
            "- outer\n  - inner\n\n    more\n",
            vec![ListTightness::Tight, ListTightness::Loose],
        ),
        (
            "- outer\n  - inner\n\n  - sibling\n",
            vec![ListTightness::Tight, ListTightness::Loose],
        ),
        (
            "- outer\n  - inner\n\n- sibling\n",
            vec![ListTightness::Loose, ListTightness::Tight],
        ),
        (
            "- outer\n  - inner\n\n  # parent\n",
            vec![ListTightness::Loose, ListTightness::Tight],
        ),
        (
            "- outer\n  - inner\n\n",
            vec![ListTightness::Tight, ListTightness::Tight],
        ),
        ("> - first\n>\n> - second\n", vec![ListTightness::Loose]),
        (
            "> - first\n\n> - second\n",
            vec![ListTightness::Tight, ListTightness::Tight],
        ),
        (
            "- > - inner\n\n- sibling\n",
            vec![ListTightness::Loose, ListTightness::Tight],
        ),
    ] {
        assert_eq!(lists(&parse(source)), expected, "{source:?}");
    }
}

#[test]
fn trailing_blanks_stay_in_retained_ranges_without_confirming_loose() {
    for tail in ["", "plain\n", "# root\n", "+ other\n", "1. other\n"] {
        let first = "- first\n\n \r\n";
        let doc = parse(&format!("{first}{tail}"));
        let list = doc.container(ContainerId(0)).unwrap();
        assert_eq!(list.source, Span::new(0, first.len()));
        assert_eq!(list.state, SyntaxState::Sealed);
        assert_eq!(list.list_tightness, Some(ListTightness::Tight));
        assert_eq!(
            doc.container(ContainerId(1)).unwrap().source.end,
            first.len()
        );
    }
}

#[test]
fn marker_task_and_empty_item_variants_confirm_only_real_separation() {
    for (marker, sibling, indent) in [
        ("-", "-", "  "),
        ("+", "+", "  "),
        ("*", "*", "  "),
        ("9.", "20.", "   "),
        ("1)", "008)", "   "),
        ("- [x]", "- [ ]", "  "),
    ] {
        for gap in ["\n", " \n", "\r\n\n", "\t\n", "\x0c\n"] {
            for tail in [format!("{sibling} next\n"), format!("{indent}next\n")] {
                let source = format!("{marker} first\n{gap}{tail}");
                assert_eq!(lists(&parse(&source)), [ListTightness::Loose], "{source:?}");
            }
        }
    }
    for source in ["-\n\n text\n", "- [x]\n\n  first\n", "-\n\n", "- [ ]\n\n"] {
        assert_eq!(lists(&parse(source)), [ListTightness::Tight], "{source:?}");
    }
    for source in ["-\n\n-\n", "- [x]\n\n- [ ]", "- a\n\n-\n"] {
        assert_eq!(lists(&parse(source)), [ListTightness::Loose], "{source:?}");
    }
}

#[test]
fn detached_blanks_end_non_code_sequences_but_fence_blanks_remain_opaque() {
    for first in ["- $$\n  raw\n", "- a\n      code\n", "- [^n]: raw\n"] {
        let source = format!("{first}\n  next\n");
        let doc = parse(&source);
        assert_eq!(lists(&doc), [ListTightness::Loose], "{source:?}");
        let leaf = doc.block_at(first.len() - 1).unwrap();
        assert_eq!(doc.block(leaf).unwrap().source.end, first.len());
        assert_eq!(doc.blocks().last().unwrap().container, Some(ContainerId(1)));
        assert_eq!(doc.blocks().last().unwrap().kind, BlockKind::Paragraph);
    }
    for source in [
        "- ```\n  raw\n\n  next\n",
        "- ```\n\n  raw\n  ```\n- next\n",
        "- ```\n  \n  raw\n  ```\n- next\n",
        "- $$\n  \n  raw\n  $$\n- next\n",
        "- [^n]: raw\n  \n      more\n- next\n",
    ] {
        assert_eq!(lists(&parse(source)), [ListTightness::Tight], "{source:?}");
    }
    let doc = parse("- *open\n\n  close*\n  a|b\n\n  -|-\n  text\n\n  ===\n");
    assert!(doc.tables().is_empty());
    assert!(doc.headings().entries().is_empty());
    assert!(
        !doc.runs_in(Span::new(0, doc.source().len()))
            .unwrap()
            .any(|r| r.style.contains(Style::EMPHASIS))
    );
    assert_eq!(lists(&doc), [ListTightness::Loose]);
    // A blank ends lazy continuation; omitted nonblank text cannot resume it.
    let doc = parse("- *open\n\nclose*\n");
    assert_eq!(doc.blocks().last().unwrap().container, None);
    assert_eq!(lists(&doc), [ListTightness::Tight]);
}

#[test]
fn missing_quotes_close_once_while_the_outer_list_keeps_its_identity() {
    let first = format!("- {} - inner\n", ">".repeat(995));
    let mut doc = Document::new();
    doc.append(&first).unwrap();
    let count = doc.container_count();
    let before = doc.work();
    let update = doc.append("\n").unwrap();
    assert_eq!(update.closed_containers.len(), count - 2);
    assert_eq!(update.closed_containers.last(), Some(&ContainerId(2)));
    assert!(update.loosened_lists.is_empty());
    assert!(doc.work().container_steps - before.container_steps < count as u64 + 20);
    for i in 2..count {
        assert_eq!(
            doc.container(ContainerId(i)).unwrap().source.end,
            first.len()
        );
    }
    let before = doc.work();
    let update = doc.append(&"\n".repeat(4096)).unwrap();
    assert!(update.closed_containers.is_empty());
    assert_eq!(doc.work().container_steps - before.container_steps, 4096);
    let update = doc.append("- sibling\n").unwrap();
    assert_eq!(update.new_containers, count..count + 1);
    assert_eq!(update.loosened_lists, [ContainerId(0)]);
    assert_eq!(
        doc.container(ContainerId(count)).unwrap().parent,
        Some(ContainerId(0))
    );
}

#[test]
fn deep_blank_runs_and_million_byte_growth_never_walk_retained_ancestors() {
    let depth = 250;
    let mut doc = Document::new();
    doc.append(&format!("{}# title\n", "- ".repeat(depth)))
        .unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    for i in 0..doc.container_count() {
        view.set_collapsed(&doc, FoldId::Container(ContainerId(i)), true)
            .unwrap();
    }
    let stats = doc.container_arena_stats();
    let before = doc.work();
    let index = view.container_index_visits();
    for _ in 0..4096 {
        let update = doc.append("\n").unwrap();
        assert!(update.closed_containers.is_empty());
        assert!(update.loosened_lists.is_empty());
        view.apply(&doc, &update).unwrap();
    }
    assert_eq!(doc.work().container_steps - before.container_steps, 4096);
    assert_eq!(doc.work().list_steps - before.list_steps, 4096);
    assert_eq!(doc.work().heading_steps - before.heading_steps, 4096);
    assert_eq!(view.container_index_visits(), index);
    assert_eq!(doc.container_arena_stats(), stats);
    let update = doc
        .append(&format!("{}{}", "  ".repeat(depth), "x".repeat(1_000_000)))
        .unwrap();
    assert_eq!(update.loosened_lists, [ContainerId(2 * depth - 2)]);
    view.apply(&doc, &update).unwrap();
    let before = doc.work();
    let index = view.container_index_visits();
    let stats = doc.container_arena_stats();
    for _ in 0..1000 {
        let update = doc.append("x").unwrap();
        view.apply(&doc, &update).unwrap();
    }
    assert_eq!(doc.work().inline_bytes - before.inline_bytes, 1000);
    assert_eq!(doc.work().container_steps, before.container_steps);
    assert_eq!(doc.work().container_bytes, before.container_bytes);
    assert_eq!(doc.work().list_steps, before.list_steps);
    assert_eq!(doc.work().heading_steps, before.heading_steps);
    assert_eq!(view.container_index_visits(), index);
    assert_eq!(doc.container_arena_stats(), stats);
    assert!(
        lists(&doc)[..depth - 1]
            .iter()
            .all(|t| *t == ListTightness::Tight)
    );
}

#[test]
fn classification_limit_and_partial_tabs_do_not_retry_or_retain_failed_paths() {
    for count in [BLOCK_LOOKAHEAD - 2, BLOCK_LOOKAHEAD - 1] {
        let source = format!("- a\n{}\n- b\n", " ".repeat(count));
        assert_eq!(lists(&parse(&source)), [ListTightness::Loose]);
    }
    for count in [BLOCK_LOOKAHEAD, BLOCK_LOOKAHEAD + 4096] {
        let source = format!("- a\n{}\n- b\n", " ".repeat(count));
        assert_eq!(lists(&parse(&source)), [ListTightness::Tight]);
    }
    for tail in ["\t# title\n", "\t```\n", "\t$$\n", "\t  literal\n"] {
        let source = format!("- a\n\n{tail}");
        assert_eq!(lists(&parse(&source)), [ListTightness::Loose], "{source:?}");
    }
    // New prose accepts a complete partial-tab path and confirms the prior
    // blank as a separator within the same item.
    let doc = parse("- a\n\n\ttext\n");
    assert_eq!(
        doc.blocks().last().unwrap().container,
        doc.blocks()[0].container
    );
    assert_eq!(lists(&doc), [ListTightness::Loose]);
}

#[test]
fn notifications_reconstruct_every_prefix_and_keep_merged_fold_state() {
    let source =
        "- # 中\r\n  - [x] 🙂\r\n\r\n    next\n\n- last\n\n+ other\n\n> - end\n>\n> - final";
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let mut delayed = FoldView::new(&doc, 1).unwrap();
    let mut merged = Update::default();
    let mut scratch = Update::default();
    let mut rebuilt: Vec<Container> = Vec::new();
    let mut loosened = std::collections::BTreeSet::new();
    for chunk in source
        .char_indices()
        .map(|(at, c)| &source[at..at + c.len_utf8()])
        .chain([""])
    {
        if chunk.is_empty() {
            doc.finish_into(&mut scratch);
        } else {
            doc.append_into(chunk, &mut scratch).unwrap();
        }
        for id in scratch.new_containers.clone() {
            rebuilt.push(doc.container(ContainerId(id)).unwrap());
        }
        for &id in scratch
            .closed_containers
            .iter()
            .chain(&scratch.loosened_lists)
        {
            rebuilt[id.0] = doc.container(id).unwrap();
        }
        for node in &mut rebuilt {
            if node.state == SyntaxState::Growing {
                node.source.end = scratch.container_end;
            }
        }
        assert_eq!(rebuilt, containers(&doc));
        for &id in &scratch.loosened_lists {
            assert!(loosened.insert(id));
        }
        view.apply(&doc, &scratch).unwrap();
        for i in scratch.new_containers.clone() {
            view.set_collapsed(&doc, FoldId::Container(ContainerId(i)), true)
                .unwrap();
        }
        merged.merge(scratch.clone()).unwrap();
    }
    delayed.apply(&doc, &merged).unwrap();
    for i in 0..doc.container_count() {
        delayed
            .set_collapsed(&doc, FoldId::Container(ContainerId(i)), true)
            .unwrap();
    }
    for at in 0..=source.len() {
        let mut budget = Budget::new(WorkLimit::unlimited());
        assert_eq!(
            view.hidden_at(&doc, at, &mut budget),
            delayed.hidden_at(&doc, at, &mut budget)
        );
    }
    assert_eq!(merged.loosened_lists.len(), 3);
    let before = doc.work();
    let repeated = doc.finish();
    assert!(repeated.closed_containers.is_empty());
    assert!(repeated.loosened_lists.is_empty());
    assert_eq!(repeated.base_revision, repeated.revision);
    assert_eq!(doc.work(), before);
}

#[test]
fn late_loose_rejects_cached_measurements_without_rewriting_historical_items() {
    let mut doc = Document::new();
    let first = doc
        .append(&format!(
            "{}- **{}**\n\n",
            "- old\n".repeat(4096),
            "x".repeat(20_000)
        ))
        .unwrap();
    let old = doc.blocks().to_vec();
    let mut heights = HeightIndex::default();
    let mut checkpoints = CheckpointIndex::<usize>::new();
    apply_layout_update(&doc, &first, &mut heights, &mut checkpoints).unwrap();
    let ticket = heights.ticket(TileId(0)).unwrap();
    heights.measure(ticket, 17).unwrap();
    let ticket = heights.ticket(TileId(0)).unwrap();
    checkpoints.insert(TileId(1), 42).unwrap();
    let key = DependencyKey::capture(&doc, TileId(0), &mut Budget::new(WorkLimit::unlimited()))
        .unwrap()
        .unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let fold = FoldId::Container(ContainerId(0));
    view.set_collapsed(&doc, fold, true).unwrap();
    let before = doc.work();
    let update = doc.append("- sibling\n").unwrap();
    assert_eq!(update.loosened_lists, [ContainerId(0)]);
    assert_eq!(&doc.blocks()[..old.len()], old);
    assert_eq!(doc.work().list_steps - before.list_steps, 3);
    assert_eq!(doc.work().container_steps - before.container_steps, 3);
    assert!(doc.work().tree_visits - before.tree_visits < 1000);
    assert!(
        update
            .changes
            .iter()
            .any(|c| c.kind == ChangeKind::Structure && c.source.start == 0)
    );
    apply_layout_update(&doc, &update, &mut heights, &mut checkpoints).unwrap();
    view.apply(&doc, &update).unwrap();
    assert!(heights.measure(ticket, 99).is_err());
    assert!(checkpoints.get(TileId(1)).is_none());
    assert_eq!(
        key.validate(&doc, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap(),
        CacheValidity::Changed
    );
    assert!(view.is_collapsed(fold));
    assert_eq!(doc.headings().entries().len(), 0);
}

#[test]
fn generated_groups_match_independent_spacing_identity_and_range_model() {
    let templates = [
        ("- a\n\n- b\n", vec![ListTightness::Loose], 3),
        ("- a\n\n  b\n", vec![ListTightness::Loose], 2),
        ("- a\n\n", vec![ListTightness::Tight], 2),
        ("-\n\n  b\n", vec![ListTightness::Tight], 2),
        (
            "- - a\n\n    b\n",
            vec![ListTightness::Tight, ListTightness::Loose],
            4,
        ),
        (
            "- - a\n\n- b\n",
            vec![ListTightness::Loose, ListTightness::Tight],
            5,
        ),
        (
            "- - a\n  \n  # b\n",
            vec![ListTightness::Loose, ListTightness::Tight],
            4,
        ),
        ("- a\n\n  > b\n", vec![ListTightness::Loose], 3),
    ];
    let mut seed = 0x824587acu64;
    let mut doc = Document::new();
    let mut expected = Vec::new();
    for _ in 0..400 {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let (text, spacing, count) = &templates[seed as usize % templates.len()];
        let depth = (seed as usize >> 8) % 8;
        let prefix = format!("{}{}", ">".repeat(depth), if depth > 0 { " " } else { "" });
        let group = text
            .lines()
            .map(|line| format!("{prefix}{line}\n"))
            .collect::<String>();
        let start = doc.source().len();
        let first = doc.container_count();
        for chunk in group.as_bytes().chunks(1 + (seed as usize >> 16) % 23) {
            doc.append(std::str::from_utf8(chunk).unwrap()).unwrap();
        }
        expected.extend_from_slice(spacing);
        assert_eq!(lists(&doc), expected);
        assert_eq!(doc.container_count() - first, depth + count);
        let list_end = if *text == "-\n\n  b\n" {
            start + prefix.len() * 2 + 3
        } else {
            start + group.len()
        };
        for i in first..first + depth + 1 {
            assert_eq!(
                doc.container(ContainerId(i)).unwrap().source,
                Span::new(
                    start,
                    if i == first + depth {
                        list_end
                    } else {
                        start + group.len()
                    }
                )
            );
        }
        let update = doc.append("# exit\n").unwrap();
        assert!(update.loosened_lists.is_empty());
        assert_eq!(
            doc.container(ContainerId(first + depth))
                .unwrap()
                .source
                .end,
            list_end
        );
    }
    doc.finish();
    assert_eq!(containers(&doc), containers(&parse(doc.source())));
    assert!(doc.work().container_steps < doc.source().len() as u64 * 4);
}

#[test]
fn folded_large_retained_list_and_ancestry_share_budget_without_body_reads() {
    struct Sink;
    impl LayoutSink for Sink {
        fn begin_tile(&mut self, _: TileId) {}
        fn text(&mut self, _: RenderRun<'_>) {
            panic!("hidden body read");
        }
        fn object(&mut self, _: LinkId, _: &LinkInfo) {
            panic!("hidden object read");
        }
        fn end_tile(&mut self, _: TileId) -> u64 {
            1
        }
    }
    impl FoldLayoutSink for Sink {
        fn placeholder(&mut self, _: FoldPlaceholder) {}
    }
    let mut doc = Document::new();
    doc.append(&format!("- {}\n\n", "x".repeat(1_000_000)))
        .unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    view.set_collapsed(&doc, FoldId::Container(ContainerId(0)), true)
        .unwrap();
    let update = doc.append("- sibling\n").unwrap();
    view.apply(&doc, &update).unwrap();
    let mut cursor = doc.container_ancestors(ContainerId(2)).unwrap();
    let mut empty = Budget::new(WorkLimit::default());
    assert_eq!(cursor.advance(&mut empty), Err(Completion::BudgetExhausted));
    assert_eq!(empty.used(), WorkLimit::default());
    let limit = WorkLimit {
        queries: 16,
        objects: 4,
        tiles: 2,
        bytes: 32,
        ..Default::default()
    };
    let mut budget = Budget::new(limit);
    assert_eq!(
        cursor.advance(&mut budget).unwrap().unwrap().0,
        ContainerId(2)
    );
    assert_eq!(
        cursor
            .advance(&mut budget)
            .unwrap()
            .unwrap()
            .1
            .list_tightness,
        Some(ListTightness::Loose)
    );
    assert_eq!(cursor.advance(&mut empty), Ok(None));
    let mut pass = LayoutPass::folded(&doc, &view, TileId(0), 10).unwrap();
    assert_eq!(
        pass.advance_folded(&doc, &mut view, &mut budget, &mut Sink)
            .unwrap(),
        Completion::EndOfDocument
    );
    assert_eq!(budget.used().bytes, 0);
    assert!(budget.used().queries <= limit.queries);
    assert!(budget.used().objects <= limit.objects);
}
