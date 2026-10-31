use mdstream::*;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn ids(doc: &Document) -> Vec<BlockId> {
    doc.blocks()
        .iter()
        .enumerate()
        .filter_map(|(i, b)| matches!(b.kind, BlockKind::Math { .. }).then_some(BlockId(i)))
        .collect()
}

fn info(doc: &Document, id: BlockId) -> (Span, bool) {
    let BlockKind::Math { content, closed } = doc.block(id).unwrap().kind else {
        panic!()
    };
    (content, closed)
}

fn tiny() -> Budget {
    Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 4,
        ..Default::default()
    })
}

fn read(doc: &Document, id: BlockId, mapped: bool) -> String {
    let mut cursor = if mapped {
        doc.math_block_payload(id)
    } else {
        doc.math_block_content(id)
    }
    .unwrap();
    let mut result = String::new();
    let mut position = info(doc, id).0.start;
    let mut charged = 0;
    loop {
        let mut budget = tiny();
        let Some(chunk) = cursor.advance(&mut budget).unwrap() else {
            break;
        };
        assert_eq!(chunk.source.start, position);
        assert!(!chunk.source.is_empty() && chunk.source.len() <= 4);
        assert_eq!(budget.used().bytes, chunk.source.len());
        assert_eq!(budget.used().queries, 1);
        assert_eq!(budget.used().objects, 1);
        if !chunk.text.is_empty() {
            assert_eq!(doc.text(chunk.source), Some(chunk.text));
        }
        result.push_str(chunk.text);
        position = cursor.source_position();
        assert_eq!(position, chunk.source.end);
        charged += budget.used().bytes;
    }
    assert_eq!(position, info(doc, id).0.end);
    assert_eq!(charged, info(doc, id).0.len());
    assert_eq!(
        cursor.advance(&mut Budget::new(WorkLimit::default())),
        Ok(None)
    );
    result
}

fn visible(doc: &Document, span: Span) -> String {
    doc.display_runs_in(span)
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect()
}

#[test]
fn delimiter_lines_are_exact_bounded_and_opaque() {
    for leading in ["", " ", "   "] {
        for end in ["\n", "\r\n"] {
            let raw = format!(
                "# *中🙂* &amp; $x$ [^n]{end}{end}> - literal{end}```{end}a|b{end}-|-{end}[r]: /u{end}"
            );
            let source = format!("{leading}$$ \t{end}{raw}{leading}$$\t{end}after\n");
            let doc = parse(&source);
            assert_eq!(ids(&doc), [BlockId(0)]);
            assert!(info(&doc, BlockId(0)).1);
            assert_eq!(read(&doc, BlockId(0), true), raw);
            assert_eq!(
                visible(&doc, doc.blocks()[0].source),
                raw.replace("\r\n", "\n")
            );
            assert!(doc.maths().is_empty());
            assert!(doc.links().is_empty());
            assert!(doc.tables().is_empty());
            assert!(doc.references().is_empty());
            assert!(doc.footnotes().is_empty());
            assert!(doc.headings().entries().is_empty());
            assert_eq!(doc.container_count(), 0);
            for r in doc.runs_in(doc.blocks()[0].source).unwrap() {
                assert!(r.style.contains(Style::MATH));
                assert!(!r.style.contains(Style::CODE));
            }
        }
    }
    for invalid in [
        "$",
        "$$$",
        "$$$$",
        "$$ suffix",
        "\\$$",
        "    $$",
        "\t$$",
        "$$\u{b}",
        "$$\r",
    ] {
        let doc = parse(invalid);
        assert!(ids(&doc).is_empty(), "{invalid:?}");
    }
    let doc = parse("$$\n$\n$$$\n$$ suffix\n    $$\n\\$$\n$$\u{b}\n$$\n");
    assert_eq!(
        read(&doc, BlockId(0), true),
        "$\n$$$\n$$ suffix\n    $$\n\\$$\n$$\u{b}\n"
    );
    assert!(info(&doc, BlockId(0)).1);
}

#[test]
fn nested_scope_payload_mapping_and_code_precedence() {
    let source = "# root\n> - [x] $$\r\n>   中\\alpha &amp;\r\n>   > - # literal\n>   \n>   $$\r\n>   after\n";
    let doc = parse(source);
    let id = ids(&doc)[0];
    let block = doc.block(id).unwrap();
    assert_eq!(id, BlockId(1));
    assert_eq!(block.container, Some(ContainerId(2)));
    assert_eq!(doc.container_count(), 3);
    assert_eq!(read(&doc, id, true), "中\\alpha &amp;\r\n> - # literal\n\n");
    assert_eq!(
        read(&doc, id, false),
        ">   中\\alpha &amp;\r\n>   > - # literal\n>   \n"
    );
    assert_eq!(block.source.end, source.find(">   after").unwrap());
    assert_eq!(doc.headings().entries().len(), 1);
    for offset in block.source.range() {
        assert_eq!(doc.block_at(offset), Some(id));
        assert_eq!(doc.fold_candidate_at(offset).unwrap().id, FoldId::Math(id));
    }
    assert_eq!(
        doc.fold_candidate(FoldId::Math(id)).unwrap().parent,
        Some(FoldId::Container(ContainerId(2)))
    );
    for source in [
        "```\n$$\nraw\n$$\n```\n",
        "> ```\n> $$\n> raw\n> $$\n> ```\n",
        "    $$\n    raw\n    $$\n",
    ] {
        assert!(ids(&parse(source)).is_empty());
    }
    let doc = parse("text $$inline$$\n\n# $$heading$$\na|$$cell$$\n-|-\n");
    assert!(ids(&doc).is_empty());
    assert_eq!(doc.maths().len(), 3);
}

#[test]
fn explicit_scope_exit_eof_and_empty_blocks_preserve_closed_flag() {
    for (head, body, boundary) in [
        ("> $$\n", "> raw\n", "outside\n"),
        ("> > $$\n", "> > raw\n", "> # outer\n"),
        ("- $$\n", "  raw\n", "- sibling\n"),
        ("> - $$\n", ">   raw\n", "> - sibling\n"),
        ("> $$\n", "> raw\n", "\n"),
        ("> $$\n", "> raw\n", ""),
    ] {
        let doc = parse(&format!("{head}{body}{boundary}"));
        let block = &doc.blocks()[0];
        assert_eq!(block.source, Span::new(0, head.len() + body.len()));
        assert_eq!(block.state, SyntaxState::Sealed);
        assert!(!info(&doc, BlockId(0)).1);
        assert_eq!(read(&doc, BlockId(0), true), "raw\n");
    }
    let doc = parse("> > $$\n> > inner\n> $$\n> outer\n> $$");
    assert_eq!(ids(&doc), [BlockId(0), BlockId(1)]);
    assert!(!info(&doc, BlockId(0)).1);
    assert!(info(&doc, BlockId(1)).1);
    for (source, closed) in [("$$", false), ("$$\n$$", true), ("$$\n", false)] {
        let doc = parse(source);
        assert_eq!(info(&doc, BlockId(0)).1, closed);
        assert_eq!(read(&doc, BlockId(0), true), "");
    }
    // A bare opener interrupts lazy text, and a missing prefix ends math.
    let doc = parse("> paragraph\n$$\nroot\n$$\n");
    assert_eq!(doc.block(ids(&doc)[0]).unwrap().container, None);
}

#[test]
fn pending_delimiters_and_long_failures_never_reclassify_old_text() {
    let mut doc = Document::new();
    doc.append("$$").unwrap();
    assert!(ids(&doc).is_empty());
    doc.append(" suffix$$\n").unwrap();
    assert!(ids(&doc).is_empty());
    assert_eq!(doc.maths().len(), 1);
    let mut doc = Document::new();
    doc.append("> $$\n> body\n> $$").unwrap();
    let committed = doc.blocks()[0].source.end;
    assert!(!info(&doc, BlockId(0)).1);
    assert_eq!(info(&doc, BlockId(0)).0.end, committed);
    assert_eq!(
        doc.fold_candidate(FoldId::Math(BlockId(0)))
            .unwrap()
            .source
            .end,
        doc.source().len()
    );
    doc.append("$\n> $$\n").unwrap();
    assert_eq!(read(&doc, BlockId(0), true), "body\n$$$\n");
    assert!(info(&doc, BlockId(0)).1);
    for prefix in ["", "> "] {
        let long = format!("{prefix}$${}\n", " ".repeat(BLOCK_LOOKAHEAD));
        assert!(ids(&parse(&long)).is_empty());
        let doc = parse(&format!("{prefix}$$\n{long}{prefix}$$\n"));
        assert_eq!(ids(&doc).len(), 1);
        assert_eq!(
            read(&doc, BlockId(0), true),
            format!("$${}\n", " ".repeat(BLOCK_LOOKAHEAD))
        );
    }
}

#[test]
fn budget_cursor_pauses_atomically_and_charges_prefixes_and_utf8() {
    let doc = parse("> $$\n> 🙂中\r\n> $$\n");
    let id = BlockId(0);
    let mut cursor = doc.math_block_payload(id).unwrap();
    let mut zero = Budget::new(WorkLimit::default());
    let start = cursor.source_position();
    assert_eq!(cursor.advance(&mut zero), Err(Completion::BudgetExhausted));
    assert_eq!(cursor.source_position(), start);
    let mut shared = tiny();
    let gap = cursor.advance(&mut shared).unwrap().unwrap();
    assert_eq!(gap.text, "");
    assert_eq!(doc.text(gap.source), Some("> "));
    let position = cursor.source_position();
    assert_eq!(
        cursor.advance(&mut shared),
        Err(Completion::BudgetExhausted)
    );
    let mut short = Budget::new(WorkLimit {
        bytes: 3,
        queries: 1,
        objects: 1,
        ..Default::default()
    });
    assert_eq!(cursor.advance(&mut short), Err(Completion::BudgetExhausted));
    assert_eq!(short.used(), WorkLimit::default());
    assert_eq!(cursor.source_position(), position);
    assert_eq!(cursor.advance(&mut tiny()).unwrap().unwrap().text, "🙂");
    assert_eq!(read(&doc, id, true), "🙂中\r\n");
    assert!(doc.math_block_payload(BlockId(99)).is_none());
    assert!(parse("text\n").math_block_content(BlockId(0)).is_none());
}

#[test]
fn notifications_keep_identity_merge_and_invalidate_old_content_on_growth_and_seal() {
    let mut doc = Document::new();
    let mut heights = HeightIndex::default();
    heights.rebuild(&doc).unwrap();
    let mut checkpoints = CheckpointIndex::<()>::new();
    let first = doc
        .append(&format!("> $$\n> {}", "x".repeat(2000)))
        .unwrap();
    apply_layout_update(&doc, &first, &mut heights, &mut checkpoints).unwrap();
    assert_eq!(first.new_blocks, 0..1);
    let start = doc.blocks()[0].source.start;
    for chunk in ["more", "\n> $$\n"] {
        let ticket = heights.ticket(TileId(0)).unwrap();
        let update = doc.append(chunk).unwrap();
        assert!(update.new_blocks.is_empty());
        assert!(update.changed_blocks.contains(&0));
        assert!(update.new_maths.is_empty());
        assert!(
            update
                .changes
                .iter()
                .any(|c| c.kind == ChangeKind::Structure && c.source.start == start)
        );
        apply_layout_update(&doc, &update, &mut heights, &mut checkpoints).unwrap();
        assert!(heights.measure(ticket, 10).is_err());
    }
    assert!(info(&doc, BlockId(0)).1);
    let saved = doc.blocks()[0].clone();
    let mut update = doc.append("> $$\n> tail").unwrap();
    update.merge(doc.finish()).unwrap();
    apply_layout_update(&doc, &update, &mut heights, &mut checkpoints).unwrap();
    assert_eq!(doc.blocks()[0], saved);
    assert_eq!(ids(&doc), [BlockId(0), BlockId(1)]);
    assert!(!info(&doc, BlockId(1)).1);
    assert_eq!(doc.blocks()[1].state, SyntaxState::Sealed);
    let revision = doc.revision();
    apply_layout_update(&doc, &update, &mut heights, &mut checkpoints).unwrap();
    assert!(doc.finish().changed_blocks.is_empty());
    assert_eq!(doc.revision(), revision);
    assert_eq!(heights.len(), doc.tile_count());
}

#[test]
fn deep_million_byte_growth_keeps_constant_state_and_no_ancestor_rescan() {
    let prefix = ">".repeat(1000);
    let mut doc = Document::new();
    doc.append(&format!("{prefix}$$\n{prefix}{}", "x".repeat(50)))
        .unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    view.set_collapsed(&doc, FoldId::Container(ContainerId(0)), true)
        .unwrap();
    view.set_collapsed(&doc, FoldId::Math(BlockId(0)), true)
        .unwrap();
    let before = doc.work();
    let visits = view.container_index_visits();
    let sections = view.section_index_visits();
    let ranges = view.range_visits();
    let capacity = doc.container_arena_stats();
    let body = "x".repeat(1000);
    for _ in 0..1000 {
        let update = doc.append(&body).unwrap();
        view.apply(&doc, &update).unwrap();
        assert!(update.new_blocks.is_empty());
        assert!(update.closed_containers.is_empty());
    }
    let after = doc.work();
    assert_eq!(after.input_bytes - before.input_bytes, 1_000_000);
    assert_eq!(after.classified_bytes, before.classified_bytes);
    assert_eq!(after.inline_bytes, before.inline_bytes);
    assert_eq!(after.container_bytes, before.container_bytes);
    assert_eq!(after.container_steps, before.container_steps);
    assert_eq!(after.heading_steps, before.heading_steps);
    assert_eq!(doc.container_arena_stats(), capacity);
    assert_eq!(view.container_index_visits(), visits);
    assert_eq!(view.section_index_visits(), sections);
    assert!(view.range_visits() - ranges <= 3000);
    let update = doc.append(&format!("\n{prefix}$$\n")).unwrap();
    view.apply(&doc, &update).unwrap();
    assert!(info(&doc, BlockId(0)).1);
    assert!(doc.work().classified_bytes - after.classified_bytes <= 2 * (BLOCK_LOOKAHEAD as u64));
    assert!(doc.work().tree_visits - after.tree_visits < 2000);
    assert_eq!(doc.work().inline_bytes, 0);
    assert_eq!(doc.blocks().len(), 1);
}

#[test]
fn deep_lists_and_historical_blocks_do_not_add_refresh_work() {
    let depth = 250;
    let prefix = " ".repeat(depth * 2);
    let mut doc = Document::new();
    doc.append(&format!("{}$$\n", "- ".repeat(depth))).unwrap();
    let before = doc.work();
    doc.append(&format!("{prefix}- > raw\n{prefix}$$\n"))
        .unwrap();
    assert_eq!(doc.container_count(), depth * 2);
    assert_eq!(read(&doc, BlockId(0), true), "- > raw\n");
    assert_eq!(
        doc.work().container_steps - before.container_steps,
        2 * depth as u64
    );
    assert!(doc.work().container_bytes - before.container_bytes < 6 * depth as u64);
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 1).unwrap();
    view.set_defaults(FoldDefaults {
        math: true,
        ..Default::default()
    });
    let update = doc.append(&"> $$\n> x\n> $$\n".repeat(4096)).unwrap();
    view.apply(&doc, &update).unwrap();
    let ranges = view.range_visits();
    let defaults = view.default_visits();
    let before = doc.work();
    let update = doc.append("> text\n").unwrap();
    view.apply(&doc, &update).unwrap();
    assert_eq!(ids(&doc).len(), 4096);
    assert!(view.range_visits() - ranges <= 2);
    assert!(view.default_visits() - defaults <= 1);
    assert!(doc.work().classified_bytes - before.classified_bytes < 30);
    assert!(view.is_collapsed(FoldId::Math(BlockId(4095))));
}

#[test]
fn generated_ranges_and_payloads_match_independent_physical_line_model() {
    let mut source = String::new();
    let mut expected = Vec::new();
    let mut seed = 0x1bb45a_u64;
    for i in 0..400 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let depth = (seed >> 32) as usize % 20 + 1;
        let prefix = ">".repeat(depth);
        let start = source.len();
        source.push_str(&format!("{prefix} - $$\r\n"));
        let body_start = source.len();
        let mut payload = String::new();
        for j in 0..(seed >> 56) as usize % 6 {
            let text = format!("*中🙂_{i}_{j}* $x$ &amp; | \\alpha\r\n");
            source.push_str(&format!("{prefix}   {text}"));
            payload.push_str(&text);
        }
        let body_end = source.len();
        let closed = seed & 1 == 0;
        if closed {
            source.push_str(&format!("{prefix}   $$\n"));
        }
        expected.push((
            Span::new(start, source.len()),
            Span::new(body_start, body_end),
            payload,
            closed,
            depth + 2,
        ));
        source.push_str("outside\n");
    }
    let doc = parse(&source);
    assert_eq!(ids(&doc).len(), expected.len());
    for (id, (span, content, payload, closed, depth)) in ids(&doc).into_iter().zip(expected) {
        let block = doc.block(id).unwrap();
        assert_eq!(block.source, span);
        assert_eq!(info(&doc, id), (content, closed));
        assert_eq!(read(&doc, id, true), payload);
        assert_eq!(visible(&doc, span), payload.replace("\r\n", "\n"));
        assert_eq!(
            doc.container(block.container.unwrap()).unwrap().depth,
            depth
        );
        assert_eq!(doc.fold_candidate(FoldId::Math(id)).unwrap().source, span);
    }
    assert!(doc.work().classified_bytes <= 2 * source.len() as u64);
    assert!(doc.work().container_bytes <= 4 * source.len() as u64);
    assert!(doc.maths().is_empty());
}

#[test]
fn defaults_preview_batch_and_navigation_keep_independent_choices() {
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 1).unwrap();
    view.set_defaults(FoldDefaults {
        math: true,
        ..Default::default()
    });
    let update = doc
        .append("# root\n> ## inner\n> $$\n> 中\n> $$\n> h|h\n> -|-\n> a|b\n")
        .unwrap();
    view.apply(&doc, &update).unwrap();
    let other = FoldView::new(&doc, 1).unwrap();
    let id = ids(&doc)[0];
    let math = FoldId::Math(id);
    let root = FoldId::Heading(HeadingId(0));
    let inner = FoldId::Heading(HeadingId(1));
    let container = FoldId::Container(doc.block(id).unwrap().container.unwrap());
    let at = doc.source().find('中').unwrap();
    assert!(view.is_collapsed(math));
    assert!(!other.is_collapsed(math));
    assert_eq!(doc.fold_candidate(math).unwrap().parent, Some(inner));
    view.set_collapsed(&doc, math, false).unwrap();
    let update = doc.append("> $$\n> next\n> $$\n").unwrap();
    view.apply(&doc, &update).unwrap();
    assert!(!view.is_collapsed(math));
    let next = FoldId::Math(ids(&doc)[1]);
    assert!(view.is_collapsed(next));
    view.set_collapsed(&doc, math, true).unwrap();
    for ancestor in [root, container, inner] {
        view.set_collapsed(&doc, ancestor, true).unwrap();
    }
    let mut zero = Budget::new(WorkLimit::default());
    assert_eq!(
        view.set_preview(&doc, Some(math), &mut zero)
            .unwrap()
            .completion,
        Completion::BudgetExhausted
    );
    assert_eq!(view.preview(), None);
    view.set_preview(&doc, Some(math), &mut tiny()).unwrap();
    assert_eq!(
        view.hidden_at(&doc, at, &mut tiny()).unwrap(),
        FoldLookup::Hidden(doc.fold_candidate(root).unwrap())
    );
    view.set_preview(&doc, None, &mut tiny()).unwrap();
    let mut navigation = FoldNavigation::new(
        &doc,
        &view,
        &Navigator::new(),
        NavAction::Source(at),
        FoldNavigationPolicy::Reveal,
        10,
    )
    .unwrap();
    let mut steps = 0;
    loop {
        steps += 1;
        let progress = navigation.advance(&doc, &mut view, &mut tiny()).unwrap();
        if progress.completion == Completion::Covered {
            assert_eq!(progress.target.unwrap().source, Some(at));
            break;
        }
        assert!(steps < 20);
    }
    assert!(!view.is_collapsed(math));
    assert!(view.is_collapsed(next));
    let mut batch = FoldBatch::new(
        &doc,
        &view,
        FoldBatchAction::Subtree {
            root: container,
            collapsed: true,
        },
    )
    .unwrap();
    loop {
        let mut budget = tiny();
        let progress = batch.advance(&doc, &mut view, &mut budget).unwrap();
        assert_eq!(budget.used().bytes, 0);
        if progress.completion == Completion::Covered {
            break;
        }
    }
    assert!(view.is_collapsed(math));
    assert!(view.is_collapsed(FoldId::Table(TableId(0))));
    let wrong = FoldId::Math(BlockId(0));
    assert_eq!(
        view.set_preview(&doc, Some(wrong), &mut tiny()),
        Err(LayoutError::InvalidRange)
    );
    assert!(FoldBatch::new(&doc, &view, FoldBatchAction::Focus { target: wrong }).is_err());
    let update = doc.finish();
    view.apply(&doc, &update).unwrap();
    assert_eq!(
        batch.advance(&doc, &mut view, &mut tiny()),
        Err(LayoutError::StaleContinuation)
    );
}

#[test]
fn folded_million_byte_math_never_reads_body_and_pending_scope_exit_retracts_tail() {
    struct Sink(usize);
    impl LayoutSink for Sink {
        fn begin_tile(&mut self, _: TileId) {}
        fn text(&mut self, _: RenderRun<'_>) {
            panic!("fold read body");
        }
        fn object(&mut self, _: LinkId, _: &LinkInfo) {
            panic!("unexpected object");
        }
        fn end_tile(&mut self, _: TileId) -> u64 {
            1
        }
    }
    impl FoldLayoutSink for Sink {
        fn placeholder(&mut self, _: FoldPlaceholder) {
            self.0 += 1;
        }
    }
    let mut doc = Document::new();
    doc.append(&format!("> $$\n> {}\n", "x".repeat(1_000_000)))
        .unwrap();
    let id = FoldId::Math(BlockId(0));
    let mut view = FoldView::new(&doc, 1).unwrap();
    view.set_collapsed(&doc, id, true).unwrap();
    let mut pass = LayoutPass::folded(&doc, &view, TileId(0), 1).unwrap();
    let mut budget = Budget::new(WorkLimit {
        queries: 10,
        objects: 4,
        tiles: 2,
        ..Default::default()
    });
    let mut sink = Sink(0);
    assert_eq!(
        pass.advance_folded(&doc, &mut view, &mut budget, &mut sink)
            .unwrap(),
        Completion::Covered
    );
    assert_eq!(budget.used().bytes, 0);
    assert_eq!(sink.0, 1);
    let boundary = doc.source().len();
    let update = doc.append("out").unwrap();
    view.apply(&doc, &update).unwrap();
    assert!(matches!(
        view.hidden_at(&doc, boundary, &mut tiny()).unwrap(),
        FoldLookup::Hidden(_)
    ));
    let update = doc.append("side\n").unwrap();
    view.apply(&doc, &update).unwrap();
    assert_eq!(doc.blocks()[0].source.end, boundary);
    assert!(!info(&doc, BlockId(0)).1);
    assert_eq!(
        view.hidden_at(&doc, boundary, &mut tiny()).unwrap(),
        FoldLookup::Visible
    );
}

#[test]
fn streaming_fold_ranges_and_visible_heights_match_interval_model() {
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let mut chosen = Vec::new();
    for step in 0..240 {
        let update = doc
            .append(match step % 6 {
                0 => "# section\n> $$\n",
                1 => "> 中🙂 &amp;\n",
                2 => "> $$$ literal\n",
                3 => "> $$\n",
                4 => "> a|b\n> -|-\n> c|d\n",
                _ => "outside\n",
            })
            .unwrap();
        view.apply(&doc, &update).unwrap();
        if step % 6 == 0 {
            let id = *ids(&doc).last().unwrap();
            let math = FoldId::Math(id);
            chosen.push(math);
            view.set_collapsed(&doc, math, true).unwrap();
            if step % 12 == 0 {
                let parent = FoldId::Container(doc.block(id).unwrap().container.unwrap());
                chosen.push(parent);
                view.set_collapsed(&doc, parent, true).unwrap();
            }
        }
        let ranges: Vec<_> = chosen
            .iter()
            .map(|id| match *id {
                FoldId::Math(id) => {
                    let block = doc.block(id).unwrap();
                    Span::new(
                        block.source.start,
                        if block.state == SyntaxState::Growing {
                            doc.source().len()
                        } else {
                            block.source.end
                        },
                    )
                }
                FoldId::Container(id) => {
                    let node = doc.container(id).unwrap();
                    Span::new(
                        node.source.start,
                        if node.state == SyntaxState::Growing {
                            doc.source().len()
                        } else {
                            node.source.end
                        },
                    )
                }
                _ => unreachable!(),
            })
            .collect();
        for (offset, _) in doc.source().char_indices() {
            let expected = ranges.iter().any(|r| r.range().contains(&offset));
            assert_eq!(
                matches!(
                    view.hidden_at(&doc, offset, &mut tiny()).unwrap(),
                    FoldLookup::Hidden(_)
                ),
                expected
            );
        }
        let mut rows = 0;
        for i in 0..doc.tile_count() {
            let tile = doc.tile(TileId(i)).unwrap();
            let hidden = ranges
                .iter()
                .any(|r| doc.tile_at(r.start).unwrap().0 < i && tile.end <= r.end);
            assert_eq!(
                view.heights().height(TileId(i)),
                Some(u64::from(!hidden)),
                "step {step}, tile {i}: {tile:?}, ranges {ranges:?}"
            );
            rows += u128::from(!hidden);
        }
        assert_eq!(view.heights().total_rows(), rows);
    }
}
