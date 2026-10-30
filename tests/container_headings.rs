use mdtui3::*;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn title(doc: &Document, id: usize) -> String {
    doc.display_runs_in(doc.headings().entries()[id].content)
        .unwrap()
        .filter(|run| !run.style.contains(Style::HIDDEN))
        .map(|run| run.text)
        .collect()
}

fn tiny() -> Budget {
    Budget::new(WorkLimit {
        bytes: 6,
        queries: 1,
        objects: 1,
        ..Default::default()
    })
}

#[test]
fn mixed_atx_setext_and_task_titles_keep_source_identity_and_inline_mapping() {
    let source = "# root\n> ## A**B**&amp;C ###\r\n> - [x] ### task\n>   Title `code`\r\n>   ===\r\n>   > # nested\n> ## peer\n# end\n";
    let doc = parse(source);
    let h = doc.headings();
    assert_eq!(
        h.entries().iter().map(|h| h.level).collect::<Vec<_>>(),
        [1, 2, 3, 1, 1, 2, 1]
    );
    assert_eq!(
        h.entries().iter().map(|h| h.container).collect::<Vec<_>>(),
        [
            None,
            Some(ContainerId(0)),
            Some(ContainerId(2)),
            Some(ContainerId(2)),
            Some(ContainerId(3)),
            Some(ContainerId(0)),
            None
        ]
    );
    assert_eq!(
        (0..7).map(|id| title(&doc, id)).collect::<Vec<_>>(),
        [
            "root",
            "AB&C ",
            "task",
            "Title code",
            "nested",
            "peer",
            "end"
        ]
    );
    assert_eq!(
        h.entries()[3].source,
        Span::new(
            source.find(">   Title").unwrap(),
            source.find(">   >").unwrap()
        )
    );
    assert!(matches!(
        doc.container(ContainerId(2)).unwrap().kind,
        ContainerKind::Item {
            task: Some(TaskMarker { checked: true, .. }),
            ..
        }
    ));
    for heading in h.entries() {
        let block = doc.block(heading.block).unwrap();
        assert_eq!(block.container, heading.container);
        assert_eq!(block.source, heading.source);
        assert_eq!(
            block.kind,
            BlockKind::Heading {
                level: heading.level
            }
        );
        assert_eq!(block.state, SyntaxState::Sealed);
    }
}

#[test]
fn sections_are_local_to_exact_scope_and_current_restores_outer_context() {
    let source = "# root\n> before\n> ## quote\n> - # item\n>   ### child\n>   > # nested\n>   tail\n> - # sibling item\n> ### quote child\n> ## quote peer\noutside\n## root child\n# end\n";
    let doc = parse(source);
    let h = doc.headings();
    assert_eq!(h.entries().len(), 10);
    assert_eq!(
        (0..10).map(|i| h.parent(HeadingId(i))).collect::<Vec<_>>(),
        [
            None,
            None,
            None,
            Some(HeadingId(2)),
            None,
            None,
            Some(HeadingId(1)),
            None,
            Some(HeadingId(0)),
            None
        ]
    );
    assert_eq!(h.first_child(HeadingId(0)), Some(HeadingId(8)));
    assert_eq!(h.first_child(HeadingId(1)), Some(HeadingId(6)));
    assert_eq!(h.next_sibling(HeadingId(1)), Some(HeadingId(7)));
    assert_eq!(h.previous_sibling(HeadingId(7)), Some(HeadingId(1)));
    assert_eq!(h.next_sibling(HeadingId(2)), None);
    assert_eq!(h.section_end(HeadingId(2)), None);
    assert_eq!(
        h.section(HeadingId(2), source.len()).unwrap().end,
        source.find("> - # sibling").unwrap()
    );
    for (needle, expected) in [
        ("before", 0),
        ("tail", 3),
        ("outside", 0),
        ("quote child", 6),
        ("root child", 8),
    ] {
        assert_eq!(
            h.current(source.find(needle).unwrap()),
            Some(HeadingId(expected)),
            "{needle}"
        );
    }
    assert_eq!(h.next(h.entries()[1].source.start, 1), Some(HeadingId(2)));
    assert_eq!(
        h.previous_at_level(h.entries()[5].source.start, 1),
        Some(HeadingId(4))
    );
    let view = FoldView::<()>::new(&doc, 1).unwrap();
    let mut nav = Navigator::new();
    nav.navigate(&doc, view.heights(), NavAction::Heading(HeadingId(3)), 1)
        .unwrap();
    let parent = nav
        .navigate(&doc, view.heights(), NavAction::ParentHeading, 1)
        .unwrap();
    assert_eq!(parent.source, Some(h.entries()[2].source.start));
}

#[test]
fn setext_is_one_physical_payload_and_never_crosses_container_boundaries() {
    for (source, expected) in [
        ("> first\n> second\n> -\n", vec!["second"]),
        ("- title\n  -\n", vec!["title"]),
        ("- title\n-\n", vec![]),
        ("> title\n===\n", vec![]),
        ("> title\n> > ===\n", vec![]),
        ("> title\n>\n> ===\n", vec![]),
        ("> title\n>     ===\n", vec![]),
        ("> ####### no\n> #no\n> \\# no\n", vec![]),
        ("> #\n- ##\n", vec!["", ""]),
    ] {
        let doc = parse(source);
        assert_eq!(
            (0..doc.headings().entries().len())
                .map(|i| title(&doc, i))
                .collect::<Vec<_>>(),
            expected,
            "{source:?}"
        );
    }
    // The delimiter must finish within the existing classification prefix.
    let doc = parse(&format!(
        "> title\n> {}\n",
        "=".repeat(BLOCK_LOOKAHEAD + 10)
    ));
    assert!(doc.headings().entries().is_empty());
}

fn compare(a: &Document, b: &Document) {
    assert_eq!(a.blocks(), b.blocks());
    assert_eq!(a.headings().entries(), b.headings().entries());
    assert_eq!(a.container_count(), b.container_count());
    for i in 0..a.container_count() {
        assert_eq!(a.container(ContainerId(i)), b.container(ContainerId(i)));
    }
    for i in 0..a.headings().entries().len() {
        let id = HeadingId(i);
        assert_eq!(a.headings().parent(id), b.headings().parent(id));
        assert_eq!(
            a.headings().section(id, a.source().len()),
            b.headings().section(id, b.source().len())
        );
        assert_eq!(title(a, i), title(b, i));
    }
    for (at, _) in a.source().char_indices().chain([(a.source().len(), '\0')]) {
        assert_eq!(
            a.headings().current(at),
            b.headings().current(at),
            "offset {at}"
        );
    }
}

#[test]
fn every_utf8_cut_and_live_prefix_matches_single_append_including_long_lines() {
    let source = format!(
        "# root\n> - ## 中文 &amp;🙂\r\n>   > text\n>   > =\n>   long {}中\r\n>   ---\n> # {}\n- # end",
        "x".repeat(BLOCK_LOOKAHEAD + 7),
        "y".repeat(BLOCK_LOOKAHEAD + 9)
    );
    let whole = parse(&source);
    for (cut, _) in source.char_indices().chain([(source.len(), '\0')]) {
        let mut split = Document::new();
        split.append(&source[..cut]).unwrap();
        split.append(&source[cut..]).unwrap();
        split.finish();
        compare(&whole, &split);
    }
    let mut live = Document::new();
    for ch in source.chars() {
        live.append(ch.encode_utf8(&mut [0; 4])).unwrap();
        let mut once = Document::new();
        once.append(live.source()).unwrap();
        compare(&live, &once);
    }
    live.finish();
    compare(&whole, &live);
}

#[test]
fn streaming_scope_ranges_pending_input_and_object_updates_are_explicit() {
    let mut doc = Document::new();
    doc.append("# root\n> ## quote\n> body\n").unwrap();
    let end = doc.source().len();
    let old_heading = doc.headings().entries()[1];
    let pending = doc.append("outside").unwrap();
    assert!(pending.closed_containers.is_empty());
    assert_eq!(
        doc.headings()
            .section(HeadingId(1), doc.source().len())
            .unwrap()
            .end,
        end
    );
    assert_eq!(doc.headings().current(end + 2), Some(HeadingId(1)));
    let closed = doc.append("\n").unwrap();
    assert_eq!(closed.closed_containers, [ContainerId(0)]);
    assert_eq!(doc.headings().current(end), Some(HeadingId(0)));
    assert_eq!(doc.headings().entries()[1], old_heading);
    doc.append("> next\n").unwrap();
    let block = BlockId(doc.blocks().len() - 1);
    let update = doc.append("> ===\n").unwrap();
    assert!(update.new_blocks.is_empty());
    assert_eq!(update.new_headings, 2..3);
    assert!(update.changed_blocks.contains(&block.0));
    assert_eq!(doc.headings().entries()[2].block, block);
    doc.finish();
    assert_eq!(
        doc.headings().current(doc.source().len()),
        Some(HeadingId(0))
    );
    let before = doc.work();
    doc.finish();
    assert_eq!(doc.work(), before);
}

#[test]
fn seeded_scope_index_matches_linear_section_and_navigation_model() {
    let mut seed = 0x719a_305c_u64;
    let mut source = String::from("# root\n");
    for i in 0..1200 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let depth = (seed >> 32) as usize % 19;
        let level = (seed >> 48) as usize % 6 + 1;
        source.push_str(&format!(
            "{}{} {}\n",
            ">".repeat(depth),
            "#".repeat(level),
            i
        ));
    }
    let doc = parse(&source);
    let index = doc.headings();
    let all = index.entries();
    let mut parents = Vec::new();
    let mut sections = Vec::new();
    for (i, h) in all.iter().enumerate() {
        let parent = (0..i)
            .rev()
            .find(|&p| all[p].container == h.container && all[p].level < h.level)
            .map(HeadingId);
        parents.push(parent);
        assert_eq!(index.parent(HeadingId(i)), parent);
        let end = (i + 1..all.len())
            .find(|&p| all[p].container == h.container && all[p].level <= h.level)
            .map(HeadingId);
        assert_eq!(index.section_end(HeadingId(i)), end);
        let boundary = h
            .container
            .map_or(source.len(), |id| doc.container(id).unwrap().source.end);
        let section = Span::new(
            h.source.start,
            end.map_or(boundary, |id| all[id.0].source.start),
        );
        sections.push(section);
        assert_eq!(index.section(HeadingId(i), source.len()), Some(section));
    }
    for (i, h) in all.iter().enumerate() {
        let siblings = |p: &usize| all[*p].container == h.container && parents[*p] == parents[i];
        assert_eq!(
            index.next_sibling(HeadingId(i)),
            (i + 1..all.len()).find(siblings).map(HeadingId)
        );
        assert_eq!(
            index.previous_sibling(HeadingId(i)),
            (0..i).rev().find(siblings).map(HeadingId)
        );
        assert_eq!(
            index.first_child(HeadingId(i)),
            (i + 1..all.len())
                .find(|&p| parents[p] == Some(HeadingId(i)))
                .map(HeadingId)
        );
    }
    for at in 0..source.len() {
        let current = (0..all.len())
            .rev()
            .find(|&i| sections[i].range().contains(&at))
            .map(HeadingId);
        assert_eq!(index.current(at), current, "offset {at}");
    }
    let before = doc.work();
    assert_eq!(
        index.current(source.len()),
        index
            .entries()
            .iter()
            .enumerate()
            .rev()
            .find(|(_, h)| h.container.is_none())
            .map(|(i, _)| HeadingId(i))
    );
    assert_eq!(doc.work(), before);
    assert!(before.heading_steps <= 2 * doc.container_count() as u64 + 2 * all.len() as u64 + 1);
}

#[test]
fn million_byte_setext_confirmation_and_deep_streaming_do_not_scan_ancestors() {
    let mut doc = Document::new();
    let prefix = ">".repeat(1000);
    doc.append(&format!("{prefix}# head\n{prefix}{}", "x".repeat(2000)))
        .unwrap();
    let before = doc.work();
    for _ in 0..1000 {
        doc.append(&"x".repeat(1000)).unwrap();
    }
    let after = doc.work();
    assert_eq!(after.heading_steps, before.heading_steps);
    assert_eq!(after.container_steps, before.container_steps);
    assert_eq!(after.classified_bytes, before.classified_bytes);
    doc.append("\n").unwrap();
    let before = doc.work();
    let first_tile = doc.tile_at(doc.blocks()[1].source.start + 1024).unwrap();
    let key = doc.tile_key(first_tile).unwrap();
    let update = doc.append(&format!("{prefix}=\n")).unwrap();
    assert_eq!(update.new_headings, 1..2);
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert_eq!(doc.work().heading_steps - before.heading_steps, 2);
    assert!(doc.work().tree_visits - before.tree_visits < 200);
    assert_ne!(doc.tile_key(first_tile).unwrap(), key);
    assert_eq!(doc.headings().entries()[1].content.len(), 1_002_000);
    let before = doc.work();
    let closed = doc.append("outside\n").unwrap();
    assert_eq!(closed.closed_containers.len(), 1000);
    assert_eq!(doc.work().heading_steps - before.heading_steps, 1001);
    assert_eq!(doc.headings().current(doc.source().len() - 1), None);
}

#[test]
fn deep_title_search_anchors_and_root_fold_navigation_share_tiny_budgets() {
    let mut doc = Document::new();
    let mut view = FoldView::<()>::new(&doc, 1).unwrap();
    view.set_defaults(FoldDefaults {
        heading_levels: [true; 6],
        ..Default::default()
    });
    let update = doc
        .append(&format!(
            "# same\n{}# same\n{}body\noutside\n# end\n",
            ">".repeat(1000),
            ">".repeat(1000)
        ))
        .unwrap();
    view.apply(&doc, &update).unwrap();
    let update = doc.finish();
    view.apply(&doc, &update).unwrap();
    let inner = HeadingId(1);
    assert!(doc.fold_candidate(FoldId::Heading(inner)).is_none());
    assert_eq!(
        view.set_collapsed(&doc, FoldId::Heading(inner), true),
        Err(LayoutError::InvalidRange)
    );
    let mut empty = Budget::new(WorkLimit::default());
    assert_eq!(
        view.set_preview(&doc, Some(FoldId::Heading(inner)), &mut empty),
        Err(LayoutError::InvalidRange)
    );
    assert!(matches!(
        FoldBatch::new(
            &doc,
            &view,
            FoldBatchAction::Subtree {
                root: FoldId::Heading(inner),
                collapsed: true
            }
        ),
        Err(LayoutError::InvalidRange)
    ));
    let mut anchors = HeadingAnchors::new(&doc);
    let mut search = HeadingSearch::new(&doc, "same", 6).unwrap();
    let mut hits = Vec::new();
    for _ in 0..1000 {
        let mut budget = tiny();
        let a = anchors.advance(&doc, &mut budget).unwrap();
        let s = search.advance(&doc, &mut budget, &mut hits).unwrap();
        assert!(
            budget.used().bytes <= 6 && budget.used().queries <= 1 && budget.used().objects <= 1
        );
        if a == Completion::EndOfDocument && s == Completion::EndOfDocument {
            break;
        }
    }
    assert_eq!(anchors.indexed_headings(), 3);
    assert_eq!(
        anchors.get(&doc, inner).unwrap().unwrap().slug.to_string(),
        "same-1"
    );
    assert_eq!(
        hits.iter().map(|h| h.heading).collect::<Vec<_>>(),
        [HeadingId(0), inner]
    );
    let source = doc.headings().entries()[1].source.start;
    assert!(
        matches!(view.hidden_at(&doc, source, &mut tiny()).unwrap(), FoldLookup::Hidden(c) if c.id == FoldId::Heading(HeadingId(0)))
    );
    let mut task = FoldNavigation::new(
        &doc,
        &view,
        &Navigator::new(),
        NavAction::Heading(inner),
        FoldNavigationPolicy::Reveal,
        1,
    )
    .unwrap();
    let mut reached = false;
    for _ in 0..8 {
        let mut budget = tiny();
        let progress = task.advance(&doc, &mut view, &mut budget).unwrap();
        assert_eq!(budget.used().bytes, 0);
        if progress.completion == Completion::Covered {
            assert_eq!(progress.target.unwrap().source, Some(source));
            reached = true;
            break;
        }
    }
    assert!(reached);
    assert!(!view.is_collapsed(FoldId::Heading(HeadingId(0))));
    assert!(view.is_collapsed(FoldId::Heading(HeadingId(2))));
    assert!(view.range_visits() <= 7);
}

#[test]
fn many_open_heading_scopes_do_not_add_per_append_or_fold_work() {
    let mut doc = Document::new();
    let mut view = FoldView::<()>::new(&doc, 1).unwrap();
    view.set_defaults(FoldDefaults {
        heading_levels: [true; 6],
        ..Default::default()
    });
    for depth in 0..=500 {
        let update = doc
            .append(&format!("{}# scope\n", ">".repeat(depth)))
            .unwrap();
        view.apply(&doc, &update).unwrap();
    }
    let update = doc
        .append(&format!("{}{}", ">".repeat(500), "x".repeat(1024)))
        .unwrap();
    view.apply(&doc, &update).unwrap();
    let before = doc.work();
    let visits = view.range_visits();
    for _ in 0..1000 {
        let update = doc.append("x").unwrap();
        view.apply(&doc, &update).unwrap();
    }
    assert_eq!(doc.work().heading_steps, before.heading_steps);
    assert_eq!(doc.work().container_steps, before.container_steps);
    assert_eq!(view.range_visits() - visits, 1000);
    let before = doc.work();
    let update = doc.append("\n# done\n").unwrap();
    view.apply(&doc, &update).unwrap();
    assert_eq!(update.closed_containers.len(), 500);
    assert_eq!(doc.work().heading_steps - before.heading_steps, 502);
    let boundary = doc.source().rfind("# done").unwrap();
    for i in 0..=500 {
        assert_eq!(
            doc.headings()
                .section(HeadingId(i), doc.source().len())
                .unwrap()
                .end,
            boundary
        );
    }
    assert!(
        matches!(view.hidden_at(&doc, boundary - 1, &mut tiny()).unwrap(), FoldLookup::Hidden(c) if c.id == FoldId::Heading(HeadingId(0)))
    );
}

#[test]
fn atx_requires_a_separator_or_complete_line_inside_the_classification_window() {
    for suffix in ["#not-a-heading\n", "#\n", "# title\n", "## title\n"] {
        let source = format!("{}{}", ">".repeat(BLOCK_LOOKAHEAD - 1), suffix);
        let doc = parse(&source);
        assert!(doc.headings().entries().is_empty(), "{suffix:?}");
        let mut split = Document::new();
        split.append(&source[..BLOCK_LOOKAHEAD]).unwrap();
        split.append(&source[BLOCK_LOOKAHEAD..]).unwrap();
        split.finish();
        compare(&doc, &split);
    }
    // The separator fits even though none of the title does yet.
    let source = format!("{}# title\n", ">".repeat(BLOCK_LOOKAHEAD - 2));
    let doc = parse(&source);
    assert_eq!(doc.headings().entries().len(), 1);
    assert_eq!(title(&doc, 0), "title");
}
