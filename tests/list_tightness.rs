use mdtui3::*;

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

fn assert_snapshot(a: &Document, b: &Document) {
    assert_eq!(containers(a), containers(b));
    assert_eq!(a.blocks(), b.blocks());
    assert_eq!(a.tables(), b.tables());
    assert_eq!(a.headings().entries(), b.headings().entries());
    let runs = |doc: &Document| {
        doc.display_runs_in(Span::new(0, doc.source().len()))
            .unwrap()
            .map(|r| (r.source, r.replacement, r.text.to_owned(), r.style))
            .collect::<Vec<_>>()
    };
    assert_eq!(runs(a), runs(b));
}

#[test]
fn explicit_blank_separators_loosen_bullets_ordered_tasks_and_quoted_lists() {
    for (first, next, sibling) in [
        ("- a\n", "  ", "- b\n"),
        ("+ a\n", "  ", "+ b\n"),
        ("* a\n", "  ", "* b\n"),
        ("9) a\n", "   ", "12) b\n"),
        ("1. a\n", "   ", "2. b\n"),
        ("- [x] a\n", "  ", "- [ ] b\n"),
        ("> - a\n", ">   ", "> - b\n"),
    ] {
        let tight = parse(&format!("{first}{next}continuation\n{sibling}"));
        assert_eq!(lists(&tight), [ListTightness::Tight]);
        for tail in [sibling.to_owned(), format!("{next}paragraph\n")] {
            let source = format!("{first}{next}\n{tail}");
            let doc = parse(&source);
            assert_eq!(lists(&doc), [ListTightness::Loose], "{source:?}");
            for node in containers(&doc) {
                assert_eq!(
                    node.list_tightness.is_some(),
                    matches!(node.kind, ContainerKind::List { .. })
                );
            }
        }
    }
}

#[test]
fn leading_trailing_and_marker_only_lines_wait_for_actual_separators() {
    for source in [
        "-\n",
        "- [x]\n",
        "-\n \n text\n",
        "- a\n  \n",
        "- a\n  \n  \n",
        "- a\n  \n# outside\n",
        "- a\n  \n+ other\n",
        "-\n-\n",
    ] {
        assert!(
            lists(&parse(source))
                .iter()
                .all(|t| *t == ListTightness::Tight),
            "{source:?}"
        );
    }
    for source in ["-\n \n-\n", "- [x]\n  \n- [ ]\n", "- a\n  \n-\n"] {
        assert_eq!(lists(&parse(source)), [ListTightness::Loose], "{source:?}");
    }
}

#[test]
fn blanks_affect_only_their_exact_item_and_nested_children_count_as_content() {
    for (source, expected) in [
        (
            "- outer\n  - inner\n    \n    more\n- sibling\n",
            vec![ListTightness::Tight, ListTightness::Loose],
        ),
        (
            "- outer\n  - inner\n    \n- sibling\n",
            vec![ListTightness::Tight, ListTightness::Tight],
        ),
        (
            "- outer\n  > quote\n  >\n  > more\n- sibling\n",
            vec![ListTightness::Tight],
        ),
        ("- outer\n  \n  > quote\n", vec![ListTightness::Loose]),
        (
            "- outer\n  \n  - inner\n",
            vec![ListTightness::Loose, ListTightness::Tight],
        ),
        ("- > quote\n  \n  # next\n", vec![ListTightness::Loose]),
        (
            "- - inner\n  \n  # next\n",
            vec![ListTightness::Loose, ListTightness::Tight],
        ),
        (
            "> - first\n>\n> - new list\n",
            vec![ListTightness::Tight, ListTightness::Tight],
        ),
    ] {
        assert_eq!(lists(&parse(source)), expected, "{source:?}");
    }
}

#[test]
fn unprefixed_blanks_and_marker_changes_keep_existing_scope_boundaries() {
    let source = "- a\n\n- b\n  \n+ c\n  \n1. d\n  \n1) e\n";
    let doc = parse(source);
    assert_eq!(lists(&doc), vec![ListTightness::Tight; 5]);
    assert_eq!(doc.container(ContainerId(0)).unwrap().source.end, 4);
    assert_eq!(doc.blocks()[1].kind, BlockKind::Blank);
    assert_eq!(doc.blocks()[1].container, None);
}

#[test]
fn code_payload_blanks_are_opaque_but_separating_blanks_end_indented_code() {
    for source in [
        "- ```\n  \n  raw\n  \n  ```\n- sibling\n",
        "- > ```\n  >\n  > raw\n  > ```\n- sibling\n",
    ] {
        assert_eq!(lists(&parse(source)), [ListTightness::Tight]);
    }
    for source in [
        "- before\n  \n  ```\n  \n  ```\n",
        "- ```\n  raw\n  ```\n  \n  after\n",
        "- start\n      code\n  \n      next\n",
    ] {
        assert_eq!(lists(&parse(source)), [ListTightness::Loose], "{source:?}");
    }
}

#[test]
fn headings_tables_definitions_and_failed_candidates_obey_the_same_separators() {
    for body in [
        "# title\n",
        "h|h\n  -|-\n",
        "[r]: /url\n",
        "[^n]: raw\n",
        "[ ]: /bad\n",
        "[r]: <bad<target>\n",
        "#nospace\n",
        "-bad\n",
    ] {
        for source in [
            format!("- first\n  \n  {body}"),
            format!("- {body}  \n  tail\n"),
        ] {
            assert_eq!(lists(&parse(&source)), [ListTightness::Loose], "{source:?}");
        }
    }
    assert_eq!(
        lists(&parse("- title\n  ===\n- next\n")),
        [ListTightness::Tight]
    );
    assert_eq!(
        lists(&parse("- a|b\n  -|-\n  x|y\n- next\n")),
        [ListTightness::Tight]
    );
}

#[test]
fn tight_is_tentative_and_loose_is_monotonic_with_exact_once_notifications() {
    let mut doc = Document::new();
    let first = doc.append("- first\n").unwrap();
    assert!(first.loosened_lists.is_empty());
    assert_eq!(
        doc.container(ContainerId(0)).unwrap().state,
        SyntaxState::Growing
    );
    let blank = doc.append("  \n").unwrap();
    assert!(blank.loosened_lists.is_empty());
    let before = containers(&doc);
    let pending = doc.append("- sibling").unwrap();
    assert!(pending.loosened_lists.is_empty());
    assert_eq!(containers(&doc), before);
    let update = doc.append("\n").unwrap();
    assert_eq!(update.loosened_lists, [ContainerId(0)]);
    assert_eq!(lists(&doc), [ListTightness::Loose]);
    assert_eq!(update.closed_containers, [ContainerId(1)]);
    for text in ["  \n", "  more\n", "  \n- last\n", "", "# out\n"] {
        assert!(doc.append(text).unwrap().loosened_lists.is_empty());
        assert_eq!(lists(&doc), [ListTightness::Loose]);
    }
    assert!(doc.finish().loosened_lists.is_empty());
    assert_eq!(
        doc.container(ContainerId(0)).unwrap().state,
        SyntaxState::Sealed
    );
    assert!(doc.finish().loosened_lists.is_empty());
    let work = doc.work();
    doc.finish();
    assert_eq!(doc.work(), work);
}

#[test]
fn all_utf8_splits_and_intermediate_prefixes_include_late_spacing_metadata() {
    for source in [
        "- **中**\r\n  \r\n- 🙂\r\n",
        "> 9. a\n>    \n>    - b\n>      \n>      c\n> 10. last",
        "-\n \n text\n \n- [x]\n  \n- [ ]",
        "- ```\n  \n  ```\n  \n  tail\n",
        "- > q\n  >\n  > q\n  \n  h|h\n  -|-\n",
    ] {
        let whole = parse(source);
        for (split, _) in source.char_indices().chain([(source.len(), ' ')]) {
            let mut doc = Document::new();
            doc.append(&source[..split]).unwrap();
            doc.append(&source[split..]).unwrap();
            doc.finish();
            assert_snapshot(&doc, &whole);
        }
        let mut stream = Document::new();
        for (start, ch) in source.char_indices() {
            let end = start + ch.len_utf8();
            stream.append(&source[start..end]).unwrap();
            let mut prefix = Document::new();
            prefix.append(&source[..end]).unwrap();
            assert_snapshot(&stream, &prefix);
        }
        stream.finish();
        assert_snapshot(&stream, &whole);
    }
}

#[test]
fn eof_and_bounded_classification_never_retry_long_whitespace_candidates() {
    let mut doc = Document::new();
    doc.append("- a\n  \n- b").unwrap();
    assert_eq!(lists(&doc), [ListTightness::Tight]);
    assert_eq!(doc.finish().loosened_lists, [ContainerId(0)]);
    assert_eq!(
        doc.container(ContainerId(0)).unwrap().state,
        SyntaxState::Sealed
    );
    // Long all-space lines commit as code at the existing prefix limit. They
    // are not reclassified as separators when the LF eventually arrives.
    let source = format!("- a\n{}\n  b\n", " ".repeat(BLOCK_LOOKAHEAD + 40));
    assert_eq!(lists(&parse(&source)), [ListTightness::Tight]);
    let mut stream = Document::new();
    for ch in source.chars() {
        stream.append(&ch.to_string()).unwrap();
    }
    stream.finish();
    assert_snapshot(&stream, &parse(&source));
}

#[test]
fn new_close_loosen_and_shared_tail_deltas_reconstruct_snapshots_and_merge() {
    let source = "- a\n  \n  - b\n    \n    c\n- d\n\n> - e\n>   \n> - f";
    let mut doc = Document::new();
    let mut scratch = Update::default();
    let mut merged = Update::default();
    let mut rebuilt: Vec<Container> = Vec::new();
    let mut loosened = Vec::new();
    for chunk in source.as_bytes().chunks(3).chain([&b""[..]]) {
        if chunk.is_empty() {
            doc.finish_into(&mut scratch);
        } else {
            doc.append_into(std::str::from_utf8(chunk).unwrap(), &mut scratch)
                .unwrap();
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
        loosened.extend_from_slice(&scratch.loosened_lists);
        merged.merge(scratch.clone()).unwrap();
    }
    assert_eq!(loosened.len(), 3);
    assert_eq!(merged.loosened_lists, loosened);
    let saved = merged.clone();
    merged.merge(doc.finish()).unwrap();
    assert_eq!(merged, saved);
    let capacity = scratch.loosened_lists.capacity();
    doc.finish_into(&mut scratch);
    assert_eq!(scratch.loosened_lists.capacity(), capacity);
    assert!(scratch.loosened_lists.is_empty());
    assert_eq!(
        merged.merge(parse("x").finish()),
        Err(LayoutError::WrongDocument)
    );

    let mut doc = Document::new();
    doc.append_into("- a\n  \n  b\n\n", &mut scratch).unwrap();
    let capacity = scratch.loosened_lists.capacity();
    for _ in 0..100 {
        doc.append_into("- a\n  \n  b\n\n", &mut scratch).unwrap();
        assert_eq!(scratch.loosened_lists.len(), 1);
        assert_eq!(scratch.loosened_lists.capacity(), capacity);
    }
}

#[test]
fn late_loose_invalidates_cached_layout_without_rewriting_old_blocks() {
    let mut doc = Document::new();
    let first = doc
        .append(&format!("- **{}**\n  \n", "x".repeat(20_000)))
        .unwrap();
    let old_blocks = doc.blocks().to_vec();
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
    assert_eq!(&doc.blocks()[..old_blocks.len()], old_blocks);
    assert_eq!(update.changed_blocks.start, old_blocks.len() - 1);
    assert_eq!(update.loosened_lists, [ContainerId(0)]);
    assert!(
        update
            .changes
            .iter()
            .any(|c| c.kind == ChangeKind::Structure && c.source.start == 0)
    );
    assert!(update.dirty.iter().any(|s| s.start == 0));
    assert!(doc.work().tree_visits - before.tree_visits < 1000);
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
    assert!(
        doc.runs(TileId(0))
            .unwrap()
            .any(|r| r.style.contains(Style::STRONG))
    );
    let mut batch = first;
    batch.merge(update).unwrap();
    let mut delayed = FoldView::new(&doc, 1).unwrap();
    delayed.set_collapsed(&doc, fold, true).unwrap();
    let mut budget = Budget::new(WorkLimit::unlimited());
    assert_eq!(
        view.hidden_at(&doc, 10, &mut budget),
        delayed.hidden_at(&doc, 10, &mut budget)
    );
    let mut rebuilt = HeightIndex::default();
    rebuilt.apply(&doc, &batch).unwrap();
    assert_eq!(rebuilt.len(), heights.len());
}

#[test]
fn deep_lists_and_quotes_never_propagate_blank_state_to_ancestors() {
    for depth in [64, 128, 250] {
        let mut doc = Document::new();
        doc.append(&format!("{}first\n", "- ".repeat(depth)))
            .unwrap();
        let prefix = "  ".repeat(depth);
        let before = doc.work();
        for _ in 0..1000 {
            let update = doc.append(&format!("{prefix}\n")).unwrap();
            assert!(update.loosened_lists.is_empty());
        }
        assert_eq!(
            doc.work().container_steps - before.container_steps,
            depth as u64 * 1000
        );
        assert_eq!(doc.work().list_steps - before.list_steps, 1000);
        let before = doc.work();
        let update = doc.append(&format!("{prefix}next\n")).unwrap();
        assert_eq!(update.loosened_lists, [ContainerId(2 * depth - 2)]);
        assert_eq!(doc.work().list_steps - before.list_steps, 2);
        assert!(
            lists(&doc)[..depth - 1]
                .iter()
                .all(|t| *t == ListTightness::Tight)
        );
    }
    let mut doc = Document::new();
    doc.append(&format!("{} - a\n", ">".repeat(1000))).unwrap();
    let before = doc.work();
    for _ in 0..4096 {
        doc.append("lazy\n").unwrap();
    }
    assert_eq!(doc.work().container_steps - before.container_steps, 4096);
    assert_eq!(doc.work().list_steps - before.list_steps, 4096);
    let before = doc.work();
    let update = doc
        .append(&format!(
            "{}   \n{} - b\n",
            ">".repeat(1000),
            ">".repeat(1000)
        ))
        .unwrap();
    assert_eq!(update.loosened_lists, [ContainerId(1000)]);
    assert_eq!(doc.work().list_steps - before.list_steps, 4);
}

#[test]
fn million_byte_growth_and_many_historical_items_do_constant_spacing_work() {
    for count in [1, 4096] {
        let mut doc = Document::new();
        doc.append(&"- old\n".repeat(count)).unwrap();
        let old = doc.blocks().to_vec();
        doc.append("  \n").unwrap();
        let before = doc.work();
        let update = doc.append("- new\n").unwrap();
        assert_eq!(update.loosened_lists, [ContainerId(0)]);
        assert_eq!(doc.work().list_steps - before.list_steps, 3);
        assert_eq!(doc.work().container_steps - before.container_steps, 3);
        assert!(doc.work().tree_visits - before.tree_visits < 1000);
        assert_eq!(&doc.blocks()[..old.len()], old);
        let before = doc.work();
        let update = doc.append(&format!("  {}", "x".repeat(1_000_000))).unwrap();
        assert!(update.loosened_lists.is_empty());
        assert_eq!(doc.work().list_steps - before.list_steps, 1);
        let before = doc.work();
        for _ in 0..1000 {
            doc.append("x").unwrap();
        }
        assert_eq!(doc.work().inline_bytes - before.inline_bytes, 1000);
        assert_eq!(doc.work().list_steps, before.list_steps);
        assert_eq!(doc.work().container_steps, before.container_steps);
        assert_eq!(doc.work().classified_bytes, before.classified_bytes);
        let update = doc.append("\n  \n  again\n").unwrap();
        assert!(update.loosened_lists.is_empty());
        assert!(!update.changes.iter().any(|c| c.source.start == 0));
    }
}

#[test]
fn generated_nested_list_groups_match_independent_expected_spacing() {
    let templates = [
        ("- a\n  b\n- c\n", vec![ListTightness::Tight]),
        ("- a\n  \n- b\n", vec![ListTightness::Loose]),
        ("- a\n  \n  # h\n", vec![ListTightness::Loose]),
        (
            "- a\n  - b\n    \n    c\n- d\n",
            vec![ListTightness::Tight, ListTightness::Loose],
        ),
        (
            "- a\n  \n  - b\n    \n",
            vec![ListTightness::Loose, ListTightness::Tight],
        ),
        ("- > a\n  >\n  > b\n- c\n", vec![ListTightness::Tight]),
        ("- ```\n  \n  ```\n", vec![ListTightness::Tight]),
    ];
    let mut source = String::new();
    let mut expected = Vec::new();
    let mut seed = 0x2454_59ab_u64;
    let mut streamed = Document::new();
    for _ in 0..400 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let (text, values) = &templates[(seed >> 32) as usize % templates.len()];
        let depth = (seed as usize >> 5) % 8;
        let prefix = if depth == 0 {
            String::new()
        } else {
            format!("{} ", ">".repeat(depth))
        };
        let group = text
            .lines()
            .map(|line| format!("{prefix}{line}\n"))
            .collect::<String>();
        source.push_str(&group);
        source.push('\n');
        expected.extend_from_slice(values);
        let start = streamed.source().len();
        let mut at = start;
        while at < source.len() {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let end = (at + 1 + (seed >> 32) as usize % 37).min(source.len());
            streamed.append(&source[at..end]).unwrap();
            at = end;
        }
        assert_eq!(lists(&streamed), expected);
    }
    streamed.finish();
    assert_snapshot(&streamed, &parse(&source));
    assert!(streamed.work().list_steps < source.len() as u64);
}

#[test]
fn metadata_queries_share_the_ancestry_budget_and_hidden_layout_reads_no_body() {
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
    doc.append(&format!("- {}\n  \n", "x".repeat(1_000_000)))
        .unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let fold = FoldId::Container(ContainerId(0));
    view.set_collapsed(&doc, fold, true).unwrap();
    let update = doc.append("- sibling\n").unwrap();
    view.apply(&doc, &update).unwrap();
    let mut cursor = doc.container_ancestors(ContainerId(2)).unwrap();
    let mut empty = Budget::new(WorkLimit::default());
    assert_eq!(cursor.advance(&mut empty), Err(Completion::BudgetExhausted));
    assert_eq!(empty.used(), WorkLimit::default());
    for expected in [None, Some(ListTightness::Loose)] {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        assert_eq!(
            cursor
                .advance(&mut budget)
                .unwrap()
                .unwrap()
                .1
                .list_tightness,
            expected
        );
        assert_eq!(budget.used().queries, 1);
        assert_eq!(budget.used().objects, 1);
    }
    assert_eq!(cursor.advance(&mut empty), Ok(None));
    let mut pass = LayoutPass::folded(&doc, &view, TileId(0), 10).unwrap();
    let mut budget = Budget::new(WorkLimit {
        queries: 16,
        tiles: 2,
        objects: 4,
        bytes: 32,
        ..Default::default()
    });
    assert_eq!(
        pass.advance_folded(&doc, &mut view, &mut budget, &mut Sink)
            .unwrap(),
        Completion::EndOfDocument
    );
    assert_eq!(budget.used().bytes, 0);
}
