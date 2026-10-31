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
        .filter_map(|node| node.list_tightness)
        .collect()
}

#[test]
fn nested_tail_confirms_only_the_resumed_list() {
    use ListTightness::{Loose, Tight};
    for (source, expected) in [
        ("- outer\n  - inner\n    \n- sibling\n", vec![Loose, Tight]),
        ("- outer\n  - inner\n    \n  # parent\n", vec![Loose, Tight]),
        (
            "- - - inner\n      \n- sibling\n",
            vec![Loose, Tight, Tight],
        ),
        (
            "- - - inner\n      \n  - sibling\n",
            vec![Tight, Loose, Tight],
        ),
        ("- - inner\n    \n    more\n- sibling\n", vec![Tight, Loose]),
        ("- - inner\n    \n  - more\n- sibling\n", vec![Tight, Loose]),
        ("- - inner\n    \n", vec![Tight, Tight]),
        ("- - inner\n    \n# outside\n", vec![Tight, Tight]),
        ("- - inner\n    \n+ other\n", vec![Tight, Tight, Tight]),
        ("- - inner\n    \n  + other\n", vec![Loose, Tight, Tight]),
    ] {
        assert_eq!(lists(&parse(source)), expected, "{source:?}");
    }
}

#[test]
fn trailing_quote_children_propagate_but_internal_blanks_and_new_markers_do_not() {
    use ListTightness::{Loose, Tight};
    for (source, expected) in [
        ("- > quote\n  >\n- sibling\n", vec![Loose]),
        ("- > quote\n  >\n  # parent\n", vec![Loose]),
        ("- > - inner\n  >   \n- sibling\n", vec![Loose, Tight]),
        ("- - > quote\n    >\n- sibling\n", vec![Loose, Tight]),
        ("- > quote\n  >\n  > more\n- sibling\n", vec![Tight]),
        ("- > quote\n  >\n", vec![Tight]),
        ("- >\n- sibling\n", vec![Tight]),
        ("- -\n- sibling\n", vec![Tight, Tight]),
        ("- -\n    \n- sibling\n", vec![Loose, Tight]),
        ("> - - inner\n>     \n> - sibling\n", vec![Loose, Tight]),
        ("> - - inner\n>     \n- new\n", vec![Tight, Tight, Tight]),
    ] {
        assert_eq!(lists(&parse(source)), expected, "{source:?}");
    }
}

#[test]
fn marker_variants_and_new_direct_children_use_the_same_tail() {
    for (first, sibling, indent) in [
        ("-", "-", "  "),
        ("+", "+", "  "),
        ("*", "*", "  "),
        ("9.", "20.", "   "),
        ("1)", "008)", "   "),
        ("- [x]", "- [ ]", "  "),
    ] {
        for tail in [
            format!("{sibling} next\n"),
            format!("{sibling}\n"),
            format!("{indent}# heading\n"),
            format!("{indent}> quote\n"),
            format!("{indent}+ different child\n"),
            format!("{indent}h|h\n{indent}-|-\n"),
            format!("{indent}[r]: /u\n"),
            format!("{indent}[bad]: <bad<url>\n"),
        ] {
            let source = format!("{first} outer\n{indent}- child\n{indent}  \n{tail}");
            let mut expected = vec![ListTightness::Loose, ListTightness::Tight];
            if tail.contains("different child") {
                expected.push(ListTightness::Tight);
            }
            assert_eq!(lists(&parse(&source)), expected, "{source:?}");
        }
    }
}

#[test]
fn opaque_payload_blanks_do_not_escape_but_footnote_tails_and_code_separators_do() {
    use ListTightness::{Loose, Tight};
    for (child, expected) in [
        ("- ```\n    \n", Tight),
        ("- $$\n    \n", Tight),
        ("- ```\n    \n    ```\n", Tight),
        ("- $$\n    \n    $$\n", Tight),
        ("- ```\n    ```\n    \n", Loose),
        ("- $$\n    $$\n    \n", Loose),
        ("- first\n        code\n    \n", Loose),
        ("- [^n]: first\n    \n", Loose),
        ("- [^n]: first\n    \n        more\n", Tight),
    ] {
        let source = format!("- outer\n  {child}- sibling\n");
        let doc = parse(&source);
        assert_eq!(lists(&doc), [expected, Tight], "{source:?}");
        assert_eq!(doc.source(), source);
    }
}

#[test]
fn short_tail_waits_for_lf_or_eof_and_long_whitespace_is_not_reclassified() {
    let first = "- - child\n    \n";
    let mut doc = Document::new();
    doc.append(first).unwrap();
    let before = containers(&doc);
    let pending = doc.append("- [x] next").unwrap();
    assert!(pending.closed_containers.is_empty());
    assert!(pending.loosened_lists.is_empty());
    assert_eq!(containers(&doc), before);
    let end = doc.finish();
    assert_eq!(end.loosened_lists, [ContainerId(0)]);
    assert_eq!(
        end.closed_containers,
        [
            ContainerId(3),
            ContainerId(2),
            ContainerId(1),
            ContainerId(4),
            ContainerId(0)
        ]
    );
    assert_eq!(
        doc.container(ContainerId(2)).unwrap().source.end,
        first.len()
    );
    let before = doc.work();
    assert!(doc.finish().loosened_lists.is_empty());
    assert_eq!(doc.work(), before);
    for count in [
        BLOCK_LOOKAHEAD - 2,
        BLOCK_LOOKAHEAD - 1,
        BLOCK_LOOKAHEAD,
        BLOCK_LOOKAHEAD + 4096,
    ] {
        let source = format!("- - child\n{}\n- next\n", " ".repeat(count));
        let expected = if count < BLOCK_LOOKAHEAD {
            ListTightness::Loose
        } else {
            ListTightness::Tight
        };
        let mut streamed = Document::new();
        for byte in source.as_bytes() {
            streamed
                .append(std::str::from_utf8(std::slice::from_ref(byte)).unwrap())
                .unwrap();
        }
        streamed.finish();
        assert_eq!(lists(&streamed), [expected, ListTightness::Tight]);
        assert_eq!(containers(&streamed), containers(&parse(&source)));
    }
    for blank in ["\t\n", "    \r\n", "    \n\n"] {
        let doc = parse(&format!("- - child\n{blank}- next\n"));
        assert_eq!(lists(&doc), [ListTightness::Loose, ListTightness::Tight]);
    }
}

#[test]
fn deep_tail_delivery_costs_one_step_beyond_actual_closures() {
    for depth in [64, 128, 250] {
        let mut doc = Document::new();
        doc.append(&format!("{}child\n", "- ".repeat(depth)))
            .unwrap();
        let mut view = FoldView::new(&doc, 1).unwrap();
        view.set_collapsed(&doc, FoldId::Container(ContainerId(0)), true)
            .unwrap();
        let stats = doc.container_arena_stats();
        let before = doc.work();
        let visits = view.container_index_visits();
        for _ in 0..1000 {
            let update = doc.append(&format!("{}\n", "  ".repeat(depth))).unwrap();
            assert!(update.loosened_lists.is_empty());
            assert!(update.closed_containers.is_empty());
            view.apply(&doc, &update).unwrap();
        }
        assert_eq!(doc.work().list_steps - before.list_steps, 1000);
        assert_eq!(
            doc.work().container_steps - before.container_steps,
            1000 * depth as u64
        );
        assert_eq!(doc.container_arena_stats(), stats);
        assert_eq!(view.container_index_visits(), visits);
        let before = doc.work();
        let update = doc.append("- sibling\n").unwrap();
        assert_eq!(update.loosened_lists, [ContainerId(0)]);
        assert_eq!(update.closed_containers.len(), 2 * depth - 1);
        assert_eq!(doc.work().list_steps - before.list_steps, 4);
        assert_eq!(
            doc.work().container_steps - before.container_steps,
            (2 * depth + 1) as u64
        );
        assert!(lists(&doc)[1..].iter().all(|t| *t == ListTightness::Tight));
    }
    let mut doc = Document::new();
    doc.append(&format!("- {} q\n", ">".repeat(1000))).unwrap();
    let prefix = format!("  {}\n", ">".repeat(1000));
    let before = doc.work();
    for _ in 0..1000 {
        doc.append(&prefix).unwrap();
    }
    assert_eq!(doc.work().list_steps, before.list_steps);
    let before = doc.work();
    let update = doc.append("- sibling\n").unwrap();
    assert_eq!(update.closed_containers.len(), 1001);
    assert_eq!(update.loosened_lists, [ContainerId(0)]);
    assert_eq!(doc.work().list_steps - before.list_steps, 4);
    assert_eq!(doc.work().container_steps - before.container_steps, 1003);
}

#[test]
fn consuming_a_tail_prevents_stale_propagation_during_million_byte_growth() {
    let mut doc = Document::new();
    doc.append(&format!(
        "{}first\n{}\n",
        "- ".repeat(250),
        "  ".repeat(250)
    ))
    .unwrap();
    let update = doc
        .append(&format!("{}{}", "  ".repeat(250), "x".repeat(1_000_000)))
        .unwrap();
    assert_eq!(update.loosened_lists, [ContainerId(498)]);
    let before = doc.work();
    let stats = doc.container_arena_stats();
    for _ in 0..1000 {
        doc.append("x").unwrap();
    }
    assert_eq!(doc.work().inline_bytes - before.inline_bytes, 1000);
    assert_eq!(doc.work().container_steps, before.container_steps);
    assert_eq!(doc.work().container_bytes, before.container_bytes);
    assert_eq!(doc.work().list_steps, before.list_steps);
    assert_eq!(doc.container_arena_stats(), stats);
    let update = doc.append("\n- sibling\n").unwrap();
    assert!(update.loosened_lists.is_empty());
    assert_eq!(lists(&doc)[0], ListTightness::Tight);
    assert_eq!(lists(&doc)[249], ListTightness::Loose);
}

#[test]
fn generated_depth_and_resume_scopes_match_spacing_and_source_range_model() {
    let mut seed = 0x65ac184bu64;
    let mut doc = Document::new();
    let mut expected = Vec::new();
    for _ in 0..400 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let depth = 2 + (seed >> 32) as usize % 12;
        let resume = (seed >> 16) as usize % depth;
        let sibling = seed & 1 == 0;
        let quoted = seed & 2 == 0;
        let prefix = if quoted { "> " } else { "" };
        let head = format!(
            "{prefix}{}child\n{prefix}{}\n",
            "- ".repeat(depth),
            "  ".repeat(depth)
        );
        let tail = if sibling {
            format!("{prefix}{}- next\n", "  ".repeat(resume))
        } else {
            format!("{prefix}{}# next\n", "  ".repeat(resume + 1))
        };
        let start = doc.source().len();
        let first = doc.container_count() + usize::from(quoted);
        let group = format!("{head}{tail}");
        for chunk in group.as_bytes().chunks(1 + (seed >> 8) as usize % 31) {
            doc.append(std::str::from_utf8(chunk).unwrap()).unwrap();
        }
        for level in 0..depth {
            expected.push(if level == resume {
                ListTightness::Loose
            } else {
                ListTightness::Tight
            });
            let node = doc.container(ContainerId(first + 2 * level)).unwrap();
            assert_eq!(
                node.source,
                Span::new(
                    start,
                    start
                        + if level > resume {
                            head.len()
                        } else {
                            group.len()
                        }
                )
            );
            assert_eq!(
                node.state,
                if level > resume {
                    SyntaxState::Sealed
                } else {
                    SyntaxState::Growing
                }
            );
        }
        assert_eq!(lists(&doc), expected);
        assert!(doc.append("# exit\n").unwrap().loosened_lists.is_empty());
    }
    doc.finish();
    assert_eq!(containers(&doc), containers(&parse(doc.source())));
}

#[test]
fn late_parent_loose_invalidates_old_layout_without_revisiting_historical_children() {
    for count in [1, 4096] {
        let mut doc = Document::new();
        let first = doc
            .append(&format!(
                "{}- **{}** [link](/u)\n  - child\n    \n",
                "- old\n".repeat(count),
                "x".repeat(20_000)
            ))
            .unwrap();
        let old = doc.blocks().to_vec();
        let links = doc.links().to_vec();
        let info = *doc.link_info(LinkId(0)).unwrap();
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
        assert_eq!(doc.links(), links);
        assert_eq!(*doc.link_info(LinkId(0)).unwrap(), info);
        assert_eq!(doc.work().list_steps - before.list_steps, 4);
        assert_eq!(doc.work().container_steps - before.container_steps, 5);
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
    }
}

#[test]
fn closure_and_loose_notifications_rebuild_every_prefix_and_merge_once() {
    let source = "- # 中\r\n  - **🙂** [l](u)\r\n    \r\n- next\n  > - child\n  >   \n  # parent\n- last\n\n> - outer\n>   - child\n>     \n> - sibling";
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
        for i in scratch.new_containers.clone() {
            rebuilt.push(doc.container(ContainerId(i)).unwrap());
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
    assert_eq!(loosened.len(), 2);
    assert_eq!(merged.loosened_lists.len(), 2);
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
    let capacity = scratch.loosened_lists.capacity();
    let before = doc.work();
    doc.finish_into(&mut scratch);
    assert!(scratch.loosened_lists.is_empty());
    assert_eq!(scratch.loosened_lists.capacity(), capacity);
    assert_eq!(doc.work(), before);
}

#[test]
fn propagated_metadata_and_folded_million_byte_body_share_a_resumable_budget() {
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
    doc.append(&format!("- - {}\n    \n", "x".repeat(1_000_000)))
        .unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let fold = FoldId::Container(ContainerId(0));
    view.set_collapsed(&doc, fold, true).unwrap();
    let mut stale = LayoutPass::folded(&doc, &view, TileId(0), 10).unwrap();
    let mut batch = FoldBatch::new(&doc, &view, FoldBatchAction::All { collapsed: false }).unwrap();
    let update = doc.append("- sibling\n").unwrap();
    assert_eq!(update.loosened_lists, [ContainerId(0)]);
    view.apply(&doc, &update).unwrap();
    let mut empty = Budget::new(WorkLimit::default());
    assert!(
        stale
            .advance_folded(&doc, &mut view, &mut empty, &mut Sink)
            .is_err()
    );
    assert!(batch.advance(&doc, &mut view, &mut empty).is_err());
    assert_eq!(empty.used(), WorkLimit::default());
    let mut cursor = doc.container_ancestors(ContainerId(4)).unwrap();
    assert_eq!(cursor.advance(&mut empty), Err(Completion::BudgetExhausted));
    let mut pass = LayoutPass::folded(&doc, &view, TileId(0), 10).unwrap();
    let mut budget = Budget::new(WorkLimit {
        queries: 16,
        objects: 4,
        tiles: 2,
        bytes: 0,
        ..Default::default()
    });
    assert_eq!(
        cursor.advance(&mut budget).unwrap().unwrap().0,
        ContainerId(4)
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
    assert_eq!(
        pass.advance_folded(&doc, &mut view, &mut budget, &mut Sink)
            .unwrap(),
        Completion::EndOfDocument
    );
    assert_eq!(budget.used().bytes, 0);
    assert!(view.is_collapsed(fold));
}
