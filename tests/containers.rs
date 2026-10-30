use mdtui3::*;

fn parsed(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn nodes(doc: &Document) -> Vec<Container> {
    (0..doc.container_count())
        .map(|i| doc.container(ContainerId(i)).unwrap())
        .collect()
}

fn visible(doc: &Document) -> String {
    doc.runs_in(Span::new(0, doc.source().len()))
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect()
}

#[test]
fn mixed_containers_have_exact_parents_ranges_and_leaf_membership() {
    let source = "> - [x] first\n>   continued\n>   > deep\n> - second\nplain\n";
    let doc = parsed(source);
    let n = nodes(&doc);
    assert_eq!(n.len(), 5);
    assert_eq!(
        n.iter().map(|c| c.parent).collect::<Vec<_>>(),
        [
            None,
            Some(ContainerId(0)),
            Some(ContainerId(1)),
            Some(ContainerId(2)),
            Some(ContainerId(1))
        ]
    );
    assert_eq!(
        n.iter().map(|c| c.depth).collect::<Vec<_>>(),
        [1, 2, 3, 4, 3]
    );
    assert_eq!(n[0].kind, ContainerKind::Quote);
    assert_eq!(
        n[1].kind,
        ContainerKind::List {
            marker: ListMarker::Bullet(b'-')
        }
    );
    assert_eq!(
        n[2].kind,
        ContainerKind::Item {
            number: None,
            task: Some(TaskMarker {
                checked: true,
                source: Span::new(4, 7)
            })
        }
    );
    let sibling = source.find("> - second").unwrap();
    let outside = source.find("plain").unwrap();
    assert_eq!(n[0].source, Span::new(0, outside));
    assert_eq!(n[1].source, Span::new(0, outside));
    assert_eq!(n[2].source, Span::new(0, sibling));
    assert_eq!(
        n[3].source,
        Span::new(source.find(">   >").unwrap(), sibling)
    );
    assert_eq!(n[4].source, Span::new(sibling, outside));
    assert!(n.iter().all(|c| c.state == SyntaxState::Sealed));
    assert_eq!(
        doc.blocks().iter().map(|b| b.container).collect::<Vec<_>>(),
        [
            Some(ContainerId(2)),
            Some(ContainerId(2)),
            Some(ContainerId(3)),
            Some(ContainerId(4)),
            None
        ]
    );
    assert_eq!(visible(&doc), "first\ncontinued\ndeep\nsecond\nplain\n");
    assert!(doc.links().is_empty());
}

#[test]
fn marker_styles_split_lists_but_numbers_and_item_widths_do_not() {
    let doc = parsed("- a\n- b\n+ c\n2) d\n10) e\n    tail\n10. f\n");
    let lists: Vec<_> = nodes(&doc)
        .into_iter()
        .filter_map(|c| match c.kind {
            ContainerKind::List { marker } => Some(marker),
            _ => None,
        })
        .collect();
    assert_eq!(
        lists,
        [
            ListMarker::Bullet(b'-'),
            ListMarker::Bullet(b'+'),
            ListMarker::Ordered(b')'),
            ListMarker::Ordered(b'.')
        ]
    );
    let b = doc.blocks();
    let parent = |i: usize| doc.container(b[i].container.unwrap()).unwrap().parent;
    assert_eq!(parent(0), parent(1));
    assert_eq!(parent(3), parent(4));
    assert_eq!(b[4].container, b[5].container);
    assert_ne!(b[3].container, b[4].container);
    assert_eq!(visible(&doc), "a\nb\nc\nd\ne\ntail\nf\n");
}

#[test]
fn blanks_explicit_continuations_empty_tasks_and_other_unsupported_inner_blocks() {
    let source = "- [ ]\n  # inner\n  ---\n  ```\n  a | b\n  - | -\n  \n  tail\n\n- fresh\n>\n> >\nplain\n# outer\n";
    let doc = parsed(source);
    let b = doc.blocks();
    let item = b[0].container.unwrap();
    assert!(matches!(
        b[0].kind,
        BlockKind::ListItem { task: Some(_), .. }
    ));
    assert_eq!(b[1].container, Some(item));
    assert_eq!(b[2].container, Some(item));
    assert_eq!(b[3].container, Some(item));
    assert_eq!(doc.headings().entries().len(), 2);
    assert_eq!(doc.tables().len(), 1);
    assert_eq!(
        doc.block(doc.tables()[0].block).unwrap().container,
        Some(item)
    );
    assert!(b.iter().all(|b| !matches!(b.kind, BlockKind::Code { .. })));
    let blank = source.find("\n\n").unwrap() + 1;
    assert_eq!(doc.container(item).unwrap().source.end, blank);
    assert_eq!(
        doc.block(doc.block_at(blank).unwrap()).unwrap().container,
        None
    );
    assert!(visible(&doc).contains("inner\n---\n```\n"));
    // Root code and table classification retain priority over marker-looking bytes.
    let doc = parsed("```\n> - literal\n```\na | b\n- | -\nvalue | row\n\n    > code\n");
    assert_eq!(doc.container_count(), 0);
    assert_eq!(doc.tables().len(), 1);
}

#[test]
fn root_rules_and_lazy_lines_close_containers_but_indented_payload_does_not() {
    for delimiter in ["---\n", "- - -\n", "***\n", "plain\n", "\n"] {
        let doc = parsed(&format!("- item\n{delimiter}"));
        assert_eq!(doc.container(ContainerId(0)).unwrap().source.end, 7);
        assert_eq!(doc.blocks()[1].container, None);
    }
    let doc =
        parsed("> quote\nlazy\n- item\n  continuation\n    - child\n      tail\n  parent again\n");
    assert_eq!(doc.blocks()[1].container, None);
    assert_eq!(doc.blocks()[2].container, doc.blocks()[3].container);
    assert_eq!(doc.blocks()[2].container, doc.blocks()[6].container);
    assert_eq!(doc.blocks()[4].container, doc.blocks()[5].container);
    assert_ne!(doc.blocks()[2].container, doc.blocks()[4].container);
}

#[test]
fn streaming_boundaries_and_notifications_exclude_pending_source() {
    let mut doc = Document::new();
    let first = doc.append("> - a\n").unwrap();
    assert_eq!(first.new_containers, 0..3);
    assert_eq!(first.container_end, 6);
    let snapshot = nodes(&doc);
    let pending = doc.append("outside").unwrap();
    assert_eq!(pending.container_end, 6);
    assert_eq!(nodes(&doc), snapshot);
    assert!(pending.closed_containers.is_empty());
    let closed = doc.append("\n").unwrap();
    assert_eq!(
        closed.closed_containers,
        [ContainerId(2), ContainerId(1), ContainerId(0)]
    );
    assert!(
        nodes(&doc)
            .iter()
            .all(|c| c.source.end == 6 && c.state == SyntaxState::Sealed)
    );
    assert_eq!(doc.blocks()[1].container, None);
    let next = doc.append("> tail").unwrap();
    assert!(next.new_containers.is_empty());
    let eof = doc.finish();
    assert_eq!(eof.new_containers, 3..4);
    assert_eq!(eof.closed_containers, [ContainerId(3)]);
    assert_eq!(
        doc.container(ContainerId(3)).unwrap().source,
        Span::new(14, 20)
    );
    let work = doc.work();
    let eof = doc.finish();
    assert!(eof.closed_containers.is_empty());
    assert!(eof.new_containers.is_empty());
    assert_eq!(doc.work(), work);
}

#[test]
fn update_deltas_reconstruct_all_snapshots_and_merge_without_losing_closures() {
    let source = format!(
        "> - first\n>   > {}\n> - second\n\n- final\n  tail",
        "x".repeat(2200)
    );
    let mut doc = Document::new();
    let mut scratch = Update::default();
    let mut merged = Update::default();
    let mut reconstructed: Vec<Container> = Vec::new();
    let mut all_closed = Vec::new();
    for chunk in source
        .as_bytes()
        .chunks(7)
        .map(|b| std::str::from_utf8(b).unwrap())
        .chain([""])
    {
        if chunk.is_empty() {
            doc.finish_into(&mut scratch);
        } else {
            doc.append_into(chunk, &mut scratch).unwrap();
        }
        for id in scratch.new_containers.clone() {
            reconstructed.push(doc.container(ContainerId(id)).unwrap());
        }
        for &id in &scratch.closed_containers {
            reconstructed[id.0] = doc.container(id).unwrap();
        }
        for node in &mut reconstructed {
            if node.state == SyntaxState::Growing {
                node.source.end = scratch.container_end;
            }
        }
        assert_eq!(reconstructed, nodes(&doc));
        all_closed.extend_from_slice(&scratch.closed_containers);
        merged.merge(scratch.clone()).unwrap();
    }
    assert_eq!(merged.new_containers, 0..doc.container_count());
    assert_eq!(merged.closed_containers, all_closed);
    assert_eq!(merged.container_end, source.len());
    let saved = merged.clone();
    merged.merge(doc.finish()).unwrap();
    assert_eq!(merged, saved);
    assert_eq!(all_closed.len(), doc.container_count());
    all_closed.sort();
    all_closed.dedup();
    assert_eq!(all_closed.len(), doc.container_count());
    assert_eq!(
        merged.merge(parsed("x").finish()),
        Err(LayoutError::WrongDocument)
    );
}

#[test]
fn quote_depth_changes_match_a_generated_independent_range_model() {
    let mut seed = 0x7aca_1123_u64;
    let mut doc = Document::new();
    let mut expected: Vec<Container> = Vec::new();
    let mut path: Vec<usize> = Vec::new();
    let mut start = 0;
    for _ in 0..2000 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let depth = (seed >> 32) as usize % 17;
        let line = format!("{}text\n", ">".repeat(depth));
        while path.len() > depth {
            let old = path.pop().unwrap();
            expected[old].state = SyntaxState::Sealed;
            expected[old].source.end = start;
        }
        while path.len() < depth {
            let parent = path.last().copied().map(ContainerId);
            path.push(expected.len());
            expected.push(Container {
                parent,
                depth: path.len(),
                kind: ContainerKind::Quote,
                source: Span::new(start, start),
                state: SyntaxState::Growing,
            });
        }
        start += line.len();
        for &id in &path {
            expected[id].source.end = start;
        }
        doc.append(&line).unwrap();
        assert_eq!(
            doc.blocks().last().unwrap().container,
            path.last().copied().map(ContainerId)
        );
    }
    assert_eq!(nodes(&doc), expected);
    for &id in &path {
        expected[id].state = SyntaxState::Sealed;
    }
    doc.finish();
    assert_eq!(nodes(&doc), expected);
    assert!(doc.work().container_steps < 3 * doc.source().len() as u64);
    assert!(doc.work().container_bytes < 10 * doc.source().len() as u64);
}

#[test]
fn thousand_deep_long_line_never_updates_ancestors_per_append() {
    let mut doc = Document::new();
    doc.append(&format!("{} {}", ">".repeat(1000), "x".repeat(100)))
        .unwrap();
    assert_eq!(doc.container_count(), 1000);
    let before = doc.work();
    for _ in 0..10_000 {
        doc.append("x").unwrap();
    }
    let update = doc.append(&"x".repeat(1_000_000)).unwrap();
    assert_eq!(doc.work().container_steps, before.container_steps);
    assert_eq!(doc.work().container_bytes, before.container_bytes);
    assert_eq!(doc.work().classified_bytes, before.classified_bytes);
    assert!(update.new_containers.is_empty());
    assert!(update.closed_containers.is_empty());
    assert_eq!(
        doc.container(ContainerId(0)).unwrap().source.end,
        doc.source().len()
    );
    assert!(doc.container_arena_stats().capacity_bytes < 200_000);
    let close = doc.finish();
    assert_eq!(doc.work().container_steps - before.container_steps, 1000);
    assert_eq!(
        close.closed_containers,
        (0..1000).rev().map(ContainerId).collect::<Vec<_>>()
    );
}

#[test]
fn deep_lists_consume_each_indentation_run_once_and_blanks_close_once() {
    for depth in [64, 128, 256] {
        let first = format!("{}leaf\n", "- ".repeat(depth));
        let continuation = format!("{}tail\n", "  ".repeat(depth));
        let mut doc = Document::new();
        doc.append(&first).unwrap();
        let before = doc.work();
        for _ in 0..100 {
            doc.append(&continuation).unwrap();
        }
        let after = doc.work();
        assert_eq!(doc.container_count(), depth * 2);
        assert_eq!(
            after.container_steps - before.container_steps,
            depth as u64 * 100
        );
        assert!(after.container_bytes - before.container_bytes < continuation.len() as u64 * 300);
        let update = doc.append("\n").unwrap();
        assert_eq!(update.closed_containers.len(), depth * 2);
        let before = doc.work();
        let update = doc.append(&"\n".repeat(20_000)).unwrap();
        assert!(update.closed_containers.is_empty());
        assert_eq!(before.container_steps, doc.work().container_steps);
        assert_eq!(doc.container_count(), depth * 2);
        assert_eq!(
            doc.container(ContainerId(0)).unwrap().source.end,
            first.len() + continuation.len() * 100
        );
        let capacity = update.closed_containers.capacity();
        let mut scratch = update;
        doc.append_into("- next\n\n", &mut scratch).unwrap();
        let capacity = capacity.max(scratch.closed_containers.capacity());
        doc.append_into("- next\n\n", &mut scratch).unwrap();
        assert_eq!(scratch.closed_containers.capacity(), capacity);
    }
}

#[test]
fn ancestry_queries_are_borrowed_bounded_and_resume_without_repeating_nodes() {
    let doc = parsed(&format!("{}x", ">".repeat(1000)));
    assert!(doc.container(ContainerId(1000)).is_none());
    assert!(doc.container_ancestors(ContainerId(1000)).is_none());
    let mut cursor = doc.container_ancestors(ContainerId(999)).unwrap();
    let mut empty = Budget::new(WorkLimit {
        queries: 1,
        ..WorkLimit::default()
    });
    assert_eq!(cursor.advance(&mut empty), Err(Completion::BudgetExhausted));
    assert_eq!(empty.used(), WorkLimit::default());
    for i in (0..1000).rev() {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            ..WorkLimit::default()
        });
        let (id, container) = cursor.advance(&mut budget).unwrap().unwrap();
        assert_eq!(id, ContainerId(i));
        assert_eq!(container.depth, i + 1);
        if i > 0 {
            assert_eq!(
                cursor.advance(&mut budget),
                Err(Completion::BudgetExhausted)
            );
        }
    }
    assert_eq!(
        cursor.advance(&mut Budget::new(WorkLimit::default())),
        Ok(None)
    );
}

#[test]
fn bounded_prefix_policy_is_chunk_invariant_at_every_observed_prefix() {
    let samples = [
        format!("{}tail\n", ">".repeat(BLOCK_LOOKAHEAD + 20)),
        format!("{}9. x\n", ">".repeat(BLOCK_LOOKAHEAD - 1)),
        format!(
            "> - [X] {}\r\n>   continuation\r\n> - sibling",
            "é🙂".repeat(200)
        ),
        "-\n x\n-\t[x]\n \ttext\n000000001) nine\n1234567890. literal\n".to_owned(),
    ];
    for source in samples {
        let mut streamed = Document::new();
        for (at, c) in source.char_indices() {
            let end = at + c.len_utf8();
            streamed.append(&source[at..end]).unwrap();
            let mut whole = Document::new();
            whole.append(&source[..end]).unwrap();
            assert_eq!(nodes(&streamed), nodes(&whole));
            assert_eq!(streamed.blocks(), whole.blocks());
            assert_eq!(visible(&streamed), visible(&whole));
        }
        streamed.finish();
        let whole = parsed(&source);
        assert_eq!(nodes(&streamed), nodes(&whole));
        assert_eq!(streamed.blocks(), whole.blocks());
    }
    let doc = parsed(&format!("{}tail", ">".repeat(BLOCK_LOOKAHEAD + 20)));
    assert_eq!(doc.container_count(), BLOCK_LOOKAHEAD);
    assert_eq!(visible(&doc), format!("{}tail", ">".repeat(20)));
}
