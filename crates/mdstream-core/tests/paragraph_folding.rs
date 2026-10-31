use mdstream::*;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}
fn one() -> Budget {
    Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        ..Default::default()
    })
}
fn finish(task: &mut SourceFoldRegistration, doc: &Document, view: &mut FoldView) -> FoldId {
    for _ in 0..30 {
        let mut budget = one();
        let p = task.advance(doc, view, &mut budget).unwrap();
        assert_eq!(budget.used().bytes, 0);
        assert!(p.visited <= 12); // At most six headings per endpoint; no body scan.
        if let Some(id) = p.id {
            assert_eq!(p.completion, Completion::Covered);
            assert_eq!(
                task.advance(doc, view, &mut Budget::new(WorkLimit::default()))
                    .unwrap(),
                p
            );
            return id;
        }
    }
    panic!("registration stalled")
}
fn register(doc: &Document, view: &mut FoldView, block: BlockId, bytes: usize) -> FoldId {
    let mut task = SourceFoldRegistration::paragraph(doc, view, block, bytes).unwrap();
    finish(&mut task, doc, view)
}
fn source_id(id: FoldId) -> SourceFoldId {
    let FoldId::Source(id) = id else { panic!() };
    id
}
fn hidden(doc: &Document, view: &FoldView, at: usize) -> Option<FoldId> {
    match view.hidden_at(doc, at, &mut one()).unwrap() {
        FoldLookup::Visible => None,
        FoldLookup::Hidden(c) => Some(c.id),
        FoldLookup::BudgetExhausted => panic!(),
    }
}

// Scalar-cell wrapping model; column deliberately survives tile/frame changes.
// The paragraph placeholder accounts for its final line in its owner tile.
// Only the separate end event resets the column before subsequent text.
#[derive(Default)]
struct Sink {
    text: String,
    column: usize,
    width: usize,
    rows: u64,
    total: u64,
    bytes: usize,
    positions: Vec<(usize, u64, usize)>,
    placeholders: Vec<FoldPlaceholder>,
    ends: Vec<BlockId>,
    objects: usize,
}
impl Sink {
    fn scalar(&mut self, ch: char) {
        if ch == '\n' {
            self.rows += 1;
            self.total += 1;
            self.column = 0;
        } else {
            if self.column == self.width {
                self.rows += 1;
                self.total += 1;
                self.column = 0;
            }
            self.column += 1;
        }
        self.text.push(ch);
    }
}
impl LayoutSink for Sink {
    fn begin_tile(&mut self, _: TileId) {
        self.rows = 0;
    }
    fn text(&mut self, run: RenderRun<'_>) {
        self.bytes += run.text.len();
        if !run.style.contains(Style::HIDDEN) {
            for (i, ch) in run.text.char_indices() {
                self.positions
                    .push((run.source.start + i, self.total, self.column));
                self.scalar(ch);
            }
        }
    }
    fn object(&mut self, _: LinkId, _: &LinkInfo) {
        self.objects += 1;
    }
    fn end_tile(&mut self, _: TileId) -> u64 {
        self.rows
    }
}
impl FoldLayoutSink for Sink {
    fn placeholder(&mut self, p: FoldPlaceholder) {
        self.scalar('~');
        if p.paragraph.is_some() {
            self.text.push('\n');
            self.rows += 1;
            self.total += 1;
        }
        self.placeholders.push(p);
    }
    fn paragraph_end(&mut self, block: BlockId) {
        self.column = 0;
        self.ends.push(block);
    }
}
fn render(doc: &Document, view: &mut FoldView, start: TileId, width: usize, tiny: bool) -> Sink {
    let mut pass = LayoutPass::folded(doc, view, start, u64::MAX).unwrap();
    let mut sink = Sink {
        width,
        column: usize::from(start.0 != 0) * (width - 1),
        ..Default::default()
    };
    for _ in 0..100_000 {
        let limits = if tiny {
            WorkLimit {
                queries: 1,
                objects: 1,
                bytes: 4,
                tiles: 1,
                ..Default::default()
            }
        } else {
            WorkLimit::unlimited()
        };
        let mut budget = Budget::new(limits);
        let p = pass
            .advance_folded(doc, view, &mut budget, &mut sink)
            .unwrap();
        assert!(budget.used().bytes <= limits.bytes && budget.used().objects <= limits.objects);
        if p == Completion::EndOfDocument {
            return sink;
        }
    }
    panic!("layout stalled")
}

#[test]
fn fixed_paragraph_identity_prefix_limits_and_view_isolation() {
    let doc = parse("a中🙂bc\r\nnext line\r\n\nfollowing\n");
    for (limit, cut) in [(0, 0), (1, 1), (3, 1), (4, 4), (7, 4), (8, 8), (11, 10)] {
        let mut view = FoldView::new(&doc, 2).unwrap();
        let other = FoldView::new(&doc, 2).unwrap();
        let id = register(&doc, &mut view, BlockId(0), limit);
        let c = view.fold_candidate(&doc, id).unwrap().unwrap();
        assert_eq!(c.source, doc.block(BlockId(0)).unwrap().source);
        assert_eq!(c.hidden, Span::new(cut, c.source.end));
        assert_eq!(c.summary, Some(Span::new(0, cut)));
        assert_eq!(
            view.paragraph_fold_block(&doc, source_id(id)).unwrap(),
            Some(BlockId(0))
        );
        assert_eq!(
            other.paragraph_fold_block(&doc, source_id(id)).unwrap(),
            None
        );
        assert!(!c.open && !view.is_collapsed(id));
        view.set_collapsed(&doc, id, true).unwrap();
        for at in 0..c.source.end {
            assert_eq!(hidden(&doc, &view, at), (at >= cut).then_some(id));
        }
        assert_eq!(view.source_fold_at(&doc, 0).unwrap(), Some(c));
        view.reset(3).unwrap();
        assert_eq!(view.fold_candidate(&doc, id).unwrap(), Some(c));
        view.remove_source_fold(&doc, source_id(id)).unwrap();
        assert_eq!(
            view.paragraph_fold_block(&doc, source_id(id)).unwrap(),
            None
        );
        let new = register(&doc, &mut view, BlockId(0), limit);
        assert_ne!(new, id);
        assert_eq!(view.source_fold_count(), 1);
    }
}

#[test]
fn only_closed_top_level_paragraphs_with_nonempty_suffix_are_accepted() {
    for source in [
        "pending",
        "paragraph\n",
        "a|b\n",
        "a|b\n---",
        "> one\n> two\n",
    ] {
        let mut doc = Document::new();
        doc.append(source).unwrap();
        let view = FoldView::new(&doc, 1).unwrap();
        for id in 0..doc.blocks().len() {
            assert!(SourceFoldRegistration::paragraph(&doc, &view, BlockId(id), 0).is_err());
        }
    }
    let doc = parse(
        "# h\n> text\n> other\n\n- item\n\n```\ncode\n```\n\n$$\nmath\n$$\n\na|b\n-| -\nx|y\n\n[^a]: note\n\npara\n",
    );
    for (i, b) in doc.blocks().iter().enumerate() {
        let mut view = FoldView::new(&doc, 1).unwrap();
        let task = SourceFoldRegistration::paragraph(&doc, &view, BlockId(i), 0);
        assert_eq!(
            task.is_ok(),
            b.kind == BlockKind::Paragraph && b.container.is_none()
        );
        if let Ok(mut task) = task {
            finish(&mut task, &doc, &mut view);
        }
        assert!(
            SourceFoldRegistration::paragraph(&doc, &view, BlockId(i), b.source.len()).is_err()
        );
        assert!(SourceFoldRegistration::paragraph(&doc, &view, BlockId(i), usize::MAX).is_err());
    }
    assert!(
        SourceFoldRegistration::paragraph(&doc, &FoldView::new(&doc, 1).unwrap(), BlockId(999), 0)
            .is_err()
    );
}

#[test]
fn reservations_reject_manual_overlap_including_the_visible_prefix_both_orders() {
    let doc = parse("# h\nabcdef\n\nother\n");
    let block = doc.block_at(4).unwrap();
    let source = doc.block(block).unwrap().source;
    for overlap in [
        Span::new(4, 5),
        Span::new(6, 9),
        source,
        Span::new(0, doc.source().len()),
    ] {
        let mut view = FoldView::new(&doc, 1).unwrap();
        let id = register(&doc, &mut view, block, 3);
        assert_eq!(
            view.fold_candidate(&doc, id).unwrap().unwrap().parent,
            Some(FoldId::Heading(HeadingId(0)))
        );
        assert!(SourceFoldRegistration::new(&doc, &view, overlap).is_err());
        view.remove_source_fold(&doc, source_id(id)).unwrap();
        let mut task = SourceFoldRegistration::new(&doc, &view, overlap).unwrap();
        let manual = finish(&mut task, &doc, &mut view);
        assert_eq!(
            view.paragraph_fold_block(&doc, source_id(manual)).unwrap(),
            None
        );
        assert!(SourceFoldRegistration::paragraph(&doc, &view, block, 3).is_err());
    }
}

#[test]
fn atomic_registration_zero_budget_stale_tasks_and_late_definitions() {
    let mut doc = Document::new();
    doc.append("[reference][r] and body\n\n").unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let mut task = SourceFoldRegistration::paragraph(&doc, &view, BlockId(0), 5).unwrap();
    let version = view.version();
    assert_eq!(
        task.advance(&doc, &mut view, &mut Budget::new(WorkLimit::default()))
            .unwrap()
            .completion,
        Completion::BudgetExhausted
    );
    assert_eq!(view.version(), version);
    for _ in 0..4 {
        assert_eq!(task.advance(&doc, &mut view, &mut one()).unwrap().id, None);
        assert_eq!(view.source_fold_count(), 0);
    }
    let other_doc = parse("elsewhere");
    assert_eq!(
        task.advance(&other_doc, &mut view, &mut one()),
        Err(LayoutError::WrongDocument)
    );
    let mut other_view = FoldView::new(&doc, 1).unwrap();
    assert_eq!(
        task.advance(&doc, &mut other_view, &mut one()),
        Err(LayoutError::WrongView)
    );
    let id = finish(&mut task, &doc, &mut view);
    let c = view.fold_candidate(&doc, id).unwrap().unwrap();
    view.set_collapsed(&doc, id, true).unwrap();
    assert_eq!(
        task.advance(&doc, &mut view, &mut one()),
        Err(LayoutError::StaleContinuation)
    );
    let update = doc.append("[r]: /resolved\n").unwrap();
    view.apply(&doc, &update).unwrap();
    assert_eq!(view.fold_candidate(&doc, id).unwrap(), Some(c));
    assert!(view.is_collapsed(id));
    let end = render(&doc, &mut view, TileId(0), 7, true);
    assert_eq!(end.ends, vec![BlockId(0)]);
}

#[test]
fn paragraph_end_resets_wrapping_across_frames_tiles_and_eof() {
    for size in [20, 253, 254, 255, 256, 257, 510, 511, 512, 1025] {
        for ending in ["", "\n", "\r\n", "\n\nfollowing line\n"] {
            let source = format!("{}{}", "x".repeat(size), ending);
            let doc = parse(&source);
            for width in [1, 3, 7, 16] {
                for preview in [0, 1, 9, size - 1] {
                    let mut view = FoldView::new(&doc, 1).unwrap();
                    let id = register(&doc, &mut view, BlockId(0), preview);
                    let candidate = view.fold_candidate(&doc, id).unwrap().unwrap();
                    view.set_collapsed(&doc, id, true).unwrap();
                    let tiny = render(&doc, &mut view, TileId(0), width, true);
                    view.reset(1).unwrap();
                    let big = render(&doc, &mut view, TileId(0), width, false);
                    assert_eq!(tiny.text, big.text);
                    assert_eq!(tiny.positions, big.positions);
                    assert_eq!(tiny.total, big.total);
                    assert_eq!(tiny.ends, vec![BlockId(0)]);
                    assert_eq!(tiny.placeholders.len(), 1);
                    assert_eq!(tiny.placeholders[0].owner, doc.tile_at(preview).unwrap());
                    assert_eq!(tiny.placeholders[0].paragraph, Some(BlockId(0)));
                    let expected = format!(
                        "{}~\n{}",
                        &source[..preview],
                        &source[candidate.hidden.end..]
                    );
                    assert_eq!(tiny.text, expected);
                    let oracle_doc = parse(&expected);
                    let mut oracle_view = FoldView::new(&oracle_doc, 1).unwrap();
                    let oracle = render(&oracle_doc, &mut oracle_view, TileId(0), width, false);
                    assert_eq!(
                        tiny.total, oracle.total,
                        "size={size}, width={width}, prefix={preview}"
                    );
                    assert_eq!(view.heights().total_rows(), u128::from(big.total));
                }
            }
        }
    }
}

#[test]
fn remote_start_in_hidden_tail_restores_paragraph_context_without_placeholder() {
    for length in [1023, 1024, 1025, 4095, 4096] {
        let source = format!("{}\n\nfollowing\n", "x".repeat(length));
        let doc = parse(&source);
        let mut view = FoldView::new(&doc, 1).unwrap();
        let id = register(&doc, &mut view, BlockId(0), 5);
        view.set_collapsed(&doc, id, true).unwrap();
        let sink = render(&doc, &mut view, TileId(2), 7, true);
        assert!(sink.placeholders.is_empty());
        assert_eq!(sink.ends, vec![BlockId(0)]);
        assert_eq!(sink.text, "\nfollowing\n");
        assert_eq!(sink.positions.first().unwrap().2, 0);
        assert!(sink.bytes < 20);
    }
}

#[test]
fn pending_end_event_is_budgeted_once_and_blocks_following_text() {
    let doc = parse("abcdef\n\nnext\n");
    let mut view = FoldView::new(&doc, 1).unwrap();
    let id = register(&doc, &mut view, BlockId(0), 0);
    view.set_collapsed(&doc, id, true).unwrap();
    let mut pass = LayoutPass::folded(&doc, &view, TileId(0), u64::MAX).unwrap();
    let mut sink = Sink {
        width: 7,
        ..Default::default()
    };
    let mut budget = Budget::new(WorkLimit {
        objects: 1,
        ..WorkLimit::unlimited()
    });
    assert_eq!(
        pass.advance_folded(&doc, &mut view, &mut budget, &mut sink)
            .unwrap(),
        Completion::BudgetExhausted
    );
    assert_eq!(sink.placeholders.len(), 1);
    assert!(sink.ends.is_empty());
    assert_eq!(sink.text, "~\n");
    let position = pass.source_position();
    for _ in 0..3 {
        let mut budget = Budget::new(WorkLimit {
            objects: 0,
            ..WorkLimit::unlimited()
        });
        assert_eq!(
            pass.advance_folded(&doc, &mut view, &mut budget, &mut sink)
                .unwrap(),
            Completion::BudgetExhausted
        );
        assert_eq!(budget.used(), WorkLimit::default());
        assert_eq!(pass.source_position(), position);
    }
    assert_eq!(
        pass.advance_folded(&doc, &mut view, &mut one(), &mut sink)
            .unwrap(),
        Completion::BudgetExhausted
    );
    assert_eq!(sink.ends, vec![BlockId(0)]);
    assert_eq!(sink.text, "~\n");
    assert_eq!(
        pass.advance_folded(
            &doc,
            &mut view,
            &mut Budget::new(WorkLimit::unlimited()),
            &mut sink
        )
        .unwrap(),
        Completion::EndOfDocument
    );
    assert_eq!(sink.text, "~\n\nnext\n");
    assert_eq!(sink.ends.len(), 1);
}

#[test]
fn million_byte_suffix_skips_objects_and_retains_bounded_summary_mapping() {
    let source = format!(
        "中🙂 **prefix** &amp; {}[link](/x)\n\nnext\n",
        "x".repeat(1_000_000)
    );
    let mut doc = Document::new();
    doc.append(&source).unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let work = doc.work();
    let id = register(&doc, &mut view, BlockId(0), 26);
    let c = view.fold_candidate(&doc, id).unwrap().unwrap();
    let mut excerpt = DisplayExcerpt::new(
        &doc,
        c.summary.unwrap(),
        ExcerptLimits {
            source_bytes: 26,
            display_bytes: 16,
            lines: 1,
        },
    )
    .unwrap();
    let mut output = String::new();
    for _ in 0..100 {
        let mut budget = Budget::new(WorkLimit {
            bytes: 6,
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        match excerpt.next(&doc, &mut budget).unwrap() {
            ExcerptStep::Run(r) => {
                assert!(r.source.end <= c.hidden.start);
                output.push_str(r.text);
            }
            ExcerptStep::Complete(_) => break,
            ExcerptStep::BudgetExhausted => {}
        }
    }
    assert!(output.starts_with("中🙂 prefix") && output.len() <= 16);
    view.set_collapsed(&doc, id, true).unwrap();
    let sink = render(&doc, &mut view, TileId(0), 7, true);
    assert!(sink.bytes < 40);
    assert_eq!(sink.objects, 0);
    assert_eq!(sink.ends, vec![BlockId(0)]);
    assert_eq!(doc.work(), work);
    let before = view.range_visits();
    let update = doc.append(&"z".repeat(1_000_000)).unwrap();
    view.apply(&doc, &update).unwrap();
    assert_eq!(view.range_visits(), before);
    assert_eq!(view.fold_candidate(&doc, id).unwrap(), Some(c));
}

#[test]
fn utf8_all_splits_and_live_prefixes_preserve_accepted_paragraphs() {
    let source = "# Heading\r\n中🙂 *emphasis* &amp;\r\ncontinued [r]\r\n\r\nplain\n---\n\na|b\n-| -\nx|y\n\n> container\n> title\n> ===\n\n[r]: /late\n";
    let baseline = parse(source);
    for split in (0..=source.len()).filter(|&i| source.is_char_boundary(i)) {
        let mut doc = Document::new();
        doc.append(&source[..split]).unwrap();
        doc.append(&source[split..]).unwrap();
        doc.finish();
        assert_eq!(doc.blocks(), baseline.blocks());
        let mut view = FoldView::new(&doc, 1).unwrap();
        let id = register(&doc, &mut view, BlockId(1), 15);
        view.set_collapsed(&doc, id, true).unwrap();
        let sink = render(&doc, &mut view, TileId(0), 9, true);
        assert_eq!(sink.ends, vec![BlockId(1)]);
    }
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let mut accepted = Vec::new();
    for ch in source.chars() {
        let update = doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
        view.apply(&doc, &update).unwrap();
        for i in 0..doc.blocks().len() {
            if let Ok(mut task) = SourceFoldRegistration::paragraph(&doc, &view, BlockId(i), 3) {
                let id = finish(&mut task, &doc, &mut view);
                view.set_collapsed(&doc, id, true).unwrap();
                accepted.push(view.fold_candidate(&doc, id).unwrap().unwrap());
            }
        }
        for &c in &accepted {
            assert_eq!(view.fold_candidate(&doc, c.id).unwrap(), Some(c));
        }
    }
    assert!(!accepted.is_empty());
    let update = doc.finish();
    view.apply(&doc, &update).unwrap();
    for &c in &accepted {
        assert_eq!(view.fold_candidate(&doc, c.id).unwrap(), Some(c));
    }
}

#[test]
fn preview_batch_anchor_and_navigation_preserve_paragraph_choice() {
    let doc = parse("# Root\nabcdefgh\n\n# Other\nbody\n");
    let mut view = FoldView::new(&doc, 1).unwrap();
    let id = register(&doc, &mut view, BlockId(1), 3);
    let heading = FoldId::Heading(HeadingId(0));
    view.set_collapsed(&doc, id, true).unwrap();
    view.set_collapsed(&doc, heading, true).unwrap();
    assert!(render(&doc, &mut view, TileId(0), 8, true).ends.is_empty());
    view.set_collapsed(&doc, heading, false).unwrap();
    assert!(view.is_collapsed(id));
    view.set_preview(&doc, Some(id), &mut one()).unwrap();
    assert!(render(&doc, &mut view, TileId(0), 8, true).ends.is_empty());
    view.set_preview(&doc, None, &mut one()).unwrap();
    assert_eq!(
        render(&doc, &mut view, TileId(0), 8, true).ends,
        vec![BlockId(1)]
    );
    let anchor = FoldAnchor::new(&doc, 12).unwrap();
    assert_eq!(
        anchor
            .resolve(&doc, &view, 10, &mut one())
            .unwrap()
            .unwrap()
            .hidden_by,
        Some(id)
    );
    let mut nav = FoldNavigation::new(
        &doc,
        &view,
        &Navigator::new(),
        NavAction::Source(12),
        FoldNavigationPolicy::Reveal,
        10,
    )
    .unwrap();
    for _ in 0..20 {
        let p = nav.advance(&doc, &mut view, &mut one()).unwrap();
        if p.completion == Completion::Covered {
            assert_eq!(p.target.unwrap().source, Some(12));
            break;
        }
    }
    assert!(!view.is_collapsed(id));
    let mut batch = FoldBatch::new(
        &doc,
        &view,
        FoldBatchAction::Subtree {
            root: id,
            collapsed: true,
        },
    )
    .unwrap();
    for _ in 0..20 {
        if batch
            .advance(&doc, &mut view, &mut one())
            .unwrap()
            .completion
            == Completion::Covered
        {
            break;
        }
    }
    assert!(view.is_collapsed(id) && !view.is_collapsed(heading));
}

#[test]
fn old_measurements_checkpoints_layout_and_registration_are_rejected() {
    let mut doc = Document::new();
    doc.append("paragraph body\n\nother body\n\n").unwrap();
    let mut view = FoldView::new(&doc, 2).unwrap();
    let id = register(&doc, &mut view, BlockId(0), 4);
    let ticket = view.heights().ticket(TileId(0)).unwrap();
    view.insert_checkpoint(TileId(0), ()).unwrap();
    let mut pass = LayoutPass::folded(&doc, &view, TileId(0), 10).unwrap();
    let other = doc.block_at(doc.source().find("other").unwrap()).unwrap();
    let mut task = SourceFoldRegistration::paragraph(&doc, &view, other, 3).unwrap();
    view.set_collapsed(&doc, id, true).unwrap();
    assert!(view.measure(ticket, 99).is_err());
    assert!(view.checkpoints().token(TileId(0)).is_none());
    assert_eq!(
        pass.advance_folded(&doc, &mut view, &mut one(), &mut Sink::default()),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(
        task.advance(&doc, &mut view, &mut one()),
        Err(LayoutError::StaleContinuation)
    );
    let mut stale = SourceFoldRegistration::paragraph(&doc, &view, other, 3).unwrap();
    let mut update = doc.append("next\n").unwrap();
    update.merge(doc.append("---\n").unwrap()).unwrap();
    update.merge(doc.finish()).unwrap();
    view.apply(&doc, &update).unwrap();
    view.apply(&doc, &update).unwrap();
    assert_eq!(
        stale.advance(&doc, &mut view, &mut one()),
        Err(LayoutError::StaleContinuation)
    );
    assert!(view.is_collapsed(id));
}

#[test]
fn historical_registrations_do_not_add_append_or_new_registration_work() {
    let mut doc = Document::new();
    doc.append(&"paragraph body\n\n".repeat(4096)).unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let mut ids = Vec::new();
    for i in 0..4096 {
        let id = register(&doc, &mut view, BlockId(i * 2), 3);
        view.set_collapsed(&doc, id, true).unwrap();
        ids.push(id);
    }
    let visits = view.range_visits();
    let work = doc.work();
    for _ in 0..1000 {
        let update = doc.append("x").unwrap();
        view.apply(&doc, &update).unwrap();
    }
    assert_eq!(view.range_visits(), visits);
    assert_eq!(doc.work().container_steps, work.container_steps);
    assert_eq!(view.source_fold_count(), 4096);
    let mut candidates =
        SourceFoldCandidates::new(&doc, &view, Span::new(16 * 4000, 16 * 4002)).unwrap();
    let mut out = Vec::new();
    let mut found = Vec::new();
    for _ in 0..4 {
        let p = candidates
            .advance(&doc, &view, &mut one(), &mut out)
            .unwrap();
        found.extend(out.iter().map(|c| c.id));
        if p == Completion::Covered {
            break;
        }
    }
    assert_eq!(found, ids[4000..4002]);
}

// Full interval oracle, independent of source/predecessor and mask indexes.
fn verify_model(doc: &Document, view: &FoldView, candidates: &[FoldCandidate]) {
    let mut collapsed = candidates
        .iter()
        .filter(|c| view.is_collapsed(c.id) && view.preview() != Some(c.id))
        .collect::<Vec<_>>();
    collapsed.sort_by_key(|c| (c.hidden.start, std::cmp::Reverse(c.hidden.end)));
    for at in (0..doc.source().len()).step_by(17).chain(
        candidates
            .iter()
            .flat_map(|c| [c.hidden.start, c.hidden.end]),
    ) {
        let expected = collapsed
            .iter()
            .find(|c| c.hidden.range().contains(&at))
            .map(|c| c.id);
        assert_eq!(hidden(doc, view, at), expected, "at {at}");
    }
    for i in 0..doc.tile_count() {
        let covered = collapsed.iter().any(|c| {
            doc.tile_at(c.hidden.start).unwrap().0 < i
                && i < doc.tile_at(c.hidden.end).map_or(doc.tile_count(), |t| t.0)
        });
        assert_eq!(
            view.heights().height(TileId(i)),
            Some(if covered {
                0
            } else {
                view.heights().base_height(TileId(i)).unwrap()
            })
        );
    }
}

#[test]
fn random_visibility_height_and_streaming_models() {
    let mut doc = Document::new();
    doc.append("# Root\n").unwrap();
    let mut view = FoldView::new(&doc, 3).unwrap();
    let mut paragraphs = Vec::new();
    for turn in 0..240 {
        let start = doc.source().len();
        let update = doc
            .append(&format!("line {turn} {}\n\n", "x".repeat(turn % 11 * 71)))
            .unwrap();
        view.apply(&doc, &update).unwrap();
        let block = doc.block_at(start).unwrap();
        let id = register(&doc, &mut view, block, turn % 6);
        view.set_collapsed(&doc, id, turn % 3 != 0).unwrap();
        paragraphs.push(view.fold_candidate(&doc, id).unwrap().unwrap());
        if turn % 11 == 0 {
            view.set_preview(&doc, Some(id), &mut one()).unwrap();
        }
        if turn % 17 == 0 {
            view.toggle(&doc, FoldId::Heading(HeadingId(0))).unwrap();
        }
        if turn % 23 == 0 {
            view.reset(2).unwrap();
        }
        let mut candidates = paragraphs.clone();
        candidates.push(doc.fold_candidate(FoldId::Heading(HeadingId(0))).unwrap());
        verify_model(&doc, &view, &candidates);
    }
    let update = doc.finish();
    view.apply(&doc, &update).unwrap();
    let mut candidates = paragraphs;
    candidates.push(doc.fold_candidate(FoldId::Heading(HeadingId(0))).unwrap());
    let mut seed = 81u64;
    for turn in 0..1000 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let id = candidates[(seed >> 32) as usize % candidates.len()].id;
        match seed % 13 {
            0 => view.reset(3).unwrap(),
            1 => {
                view.set_preview(&doc, Some(id), &mut one()).unwrap();
            }
            2 => {
                view.set_preview(&doc, None, &mut one()).unwrap();
            }
            _ => {
                view.toggle(&doc, id).unwrap();
            }
        }
        verify_model(&doc, &view, &candidates);
        if turn % 200 == 0 {
            let small = render(&doc, &mut view, TileId(0), 17, true);
            view.reset(3).unwrap();
            let large = render(&doc, &mut view, TileId(0), 17, false);
            assert_eq!(small.text, large.text);
            assert_eq!(small.positions, large.positions);
            assert_eq!(small.ends, large.ends);
        }
    }
}

#[test]
fn inline_styles_mapping_and_chunked_resume_at_every_prefix_boundary() {
    for source in [
        "**strong** &NotEqualTilde; 🙂 `one\ntwo` $math$ [label](/url)\n\nafter\n",
        "*across\nline*  \nhard\\\nbreak\r\n\r\nafter\r\n",
        "[unresolved][later] ![image](/img) `unterminated\n\nafter\n",
    ] {
        let doc = parse(source);
        let span = doc.block(BlockId(0)).unwrap().source;
        for bytes in 0..span.len() {
            let mut view = FoldView::new(&doc, 1).unwrap();
            let id = register(&doc, &mut view, BlockId(0), bytes);
            let c = view.fold_candidate(&doc, id).unwrap().unwrap();
            let mut expected = String::new();
            for run in doc.runs_in(c.summary.unwrap()).unwrap() {
                if !run.style.contains(Style::HIDDEN) {
                    expected.push_str(run.text);
                }
            }
            expected.push_str("~\n");
            for run in doc.runs_in(Span::new(c.hidden.end, source.len())).unwrap() {
                if !run.style.contains(Style::HIDDEN) {
                    expected.push_str(run.text);
                }
            }
            view.set_collapsed(&doc, id, true).unwrap();
            let sink = render(&doc, &mut view, TileId(0), 7, true);
            assert_eq!(sink.text, expected);
            assert_eq!(sink.ends, vec![BlockId(0)]);
            assert_eq!(doc.text(c.summary.unwrap()).unwrap().len(), c.hidden.start);
            // Display mappings and styles are unchanged by view registration.
            let display: Vec<_> = doc
                .display_runs_in(c.summary.unwrap())
                .unwrap()
                .map(|r| (r.source, r.replacement, r.text.to_string(), r.style))
                .collect();
            view.set_collapsed(&doc, id, false).unwrap();
            let expanded: Vec<_> = doc
                .display_runs_in(c.summary.unwrap())
                .unwrap()
                .map(|r| (r.source, r.replacement, r.text.to_string(), r.style))
                .collect();
            assert_eq!(display, expanded);
        }
    }
}

#[test]
fn restored_checkpoint_recovery_charges_paragraph_end_and_never_replays_hidden_body() {
    let source = format!("{}\n\nnext\n", "x".repeat(1_000_000));
    let doc = parse(&source);
    let mut view = FoldView::new(&doc, 1).unwrap();
    let id = register(&doc, &mut view, BlockId(0), 3);
    view.set_collapsed(&doc, id, true).unwrap();
    view.insert_checkpoint(TileId(0), ()).unwrap();
    let mut pass =
        LayoutPass::folded_from_checkpoint(&doc, &view, TileId(0), TileId(2), u64::MAX, 1024)
            .unwrap()
            .unwrap();
    let mut sink = Sink {
        width: 7,
        ..Default::default()
    };
    let mut charged_bytes = 0;
    for _ in 0..100 {
        let mut zero = Budget::new(WorkLimit {
            recovery: 0,
            ..WorkLimit::unlimited()
        });
        if pass.next_tile().0 < 2 {
            assert_eq!(
                pass.advance_folded(&doc, &mut view, &mut zero, &mut sink)
                    .unwrap(),
                Completion::BudgetExhausted
            );
        }
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            tiles: 1,
            bytes: 4,
            recovery: 1,
            ..Default::default()
        });
        let p = pass
            .advance_folded(&doc, &mut view, &mut budget, &mut sink)
            .unwrap();
        charged_bytes += budget.used().bytes;
        assert!(budget.used().recovery <= 1);
        if p == Completion::EndOfDocument {
            break;
        }
    }
    assert_eq!(sink.text, "xxx~\n\nnext\n");
    assert_eq!(sink.ends, vec![BlockId(0)]);
    assert!(charged_bytes < 20);
    assert_eq!(pass.source_position(), doc.source().len());
}

fn register_open(doc: &Document, view: &mut FoldView, block: BlockId, bytes: usize) -> FoldId {
    let mut task = SourceFoldRegistration::open_paragraph(doc, view, block, bytes).unwrap();
    finish(&mut task, doc, view)
}

#[test]
fn open_paragraph_identity_pending_tail_and_utf8_prefix_limits() {
    for (limit, cut) in [(0, 0), (1, 1), (3, 1), (4, 4), (7, 4), (8, 8), (11, 10)] {
        let mut doc = Document::new();
        doc.append("a中🙂bc\r\n").unwrap();
        let mut view = FoldView::new(&doc, 2).unwrap();
        let other = FoldView::new(&doc, 2).unwrap();
        let id = register_open(&doc, &mut view, BlockId(0), limit);
        assert!(other.fold_candidate(&doc, id).unwrap().is_none());
        view.set_collapsed(&doc, id, true).unwrap();
        for chunk in ["pending", " 中", "🙂\r", "\n", "\nfollowing\n"] {
            let update = doc.append(chunk).unwrap();
            view.apply(&doc, &update).unwrap();
            let c = view.fold_candidate(&doc, id).unwrap().unwrap();
            assert_eq!(c.id, id);
            assert_eq!(c.hidden.start, cut);
            assert_eq!(c.summary, Some(Span::new(0, cut)));
            assert_eq!(
                view.paragraph_fold_block(&doc, source_id(id)).unwrap(),
                Some(BlockId(0))
            );
            assert_eq!(
                c.source.end,
                if c.open {
                    doc.source().len()
                } else {
                    doc.blocks()[0].source.end
                }
            );
            verify_model(&doc, &view, &[c]);
        }
        let c = view.fold_candidate(&doc, id).unwrap().unwrap();
        assert!(!c.open);
        view.reset(1).unwrap();
        let sink = render(&doc, &mut view, TileId(0), 7, true);
        assert_eq!(sink.ends, vec![BlockId(0)]);
        assert!(sink.text.ends_with("\nfollowing\n"));
        let update = doc.finish();
        view.apply(&doc, &update).unwrap();
        assert_eq!(view.fold_candidate(&doc, id).unwrap(), Some(c));
        assert!(view.is_collapsed(id));
    }
}

#[test]
fn open_paragraph_registration_is_atomic_and_rejects_ineligible_blocks() {
    for text in [
        "pending",
        "> text\n",
        "- text\n",
        "# title\n",
        "```\ncode\n",
        "\n",
        "$$\nmath\n",
    ] {
        let mut doc = Document::new();
        doc.append(text).unwrap();
        let view = FoldView::new(&doc, 1).unwrap();
        for i in 0..=doc.blocks().len() {
            assert!(SourceFoldRegistration::open_paragraph(&doc, &view, BlockId(i), 0).is_err());
        }
    }
    let mut doc = Document::new();
    doc.append("# root\nparagraph\n").unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    for bytes in [doc.blocks()[1].source.len(), usize::MAX] {
        assert!(SourceFoldRegistration::open_paragraph(&doc, &view, BlockId(1), bytes).is_err());
    }
    let mut task = SourceFoldRegistration::open_paragraph(&doc, &view, BlockId(1), 3).unwrap();
    for _ in 0..4 {
        assert_eq!(
            task.advance(&doc, &mut view, &mut Budget::new(WorkLimit::default()))
                .unwrap()
                .completion,
            Completion::BudgetExhausted
        );
        assert_eq!(view.source_fold_count(), 0);
    }
    let id = finish(&mut task, &doc, &mut view);
    assert_eq!(
        view.fold_candidate(&doc, id).unwrap().unwrap().parent,
        Some(FoldId::Heading(HeadingId(0)))
    );
    assert!(SourceFoldRegistration::open_paragraph(&doc, &view, BlockId(1), 0).is_err());
    view.remove_source_fold(&doc, source_id(id)).unwrap();
    let newer = register_open(&doc, &mut view, BlockId(1), 0);
    assert_ne!(id, newer);
    let update = doc.finish();
    view.apply(&doc, &update).unwrap();
    assert!(SourceFoldRegistration::open_paragraph(&doc, &view, BlockId(1), 0).is_err());
}

#[test]
fn open_paragraph_late_kind_changes_remove_choices_and_preview() {
    for (initial, suffix, kind) in [
        ("title\n", "===\nbody\n", BlockKind::Heading { level: 1 }),
        ("title\n", "---\nbody\n", BlockKind::Heading { level: 2 }),
        ("a|b\n", "-|-\nx|y\n", BlockKind::Table { id: TableId(0) }),
    ] {
        for mode in 0..3 {
            let mut doc = Document::new();
            doc.append(initial).unwrap();
            let mut view = FoldView::new(&doc, 2).unwrap();
            let id = register_open(&doc, &mut view, BlockId(0), 1);
            view.set_collapsed(&doc, id, mode != 0).unwrap();
            if mode == 2 {
                view.set_preview(&doc, Some(id), &mut one()).unwrap();
            }
            let ticket = view.heights().ticket(TileId(0)).unwrap();
            view.insert_checkpoint(TileId(0), ()).unwrap();
            let mut pass = LayoutPass::folded(&doc, &view, TileId(0), 20).unwrap();
            let update = doc.append(suffix).unwrap();
            view.apply(&doc, &update).unwrap();
            assert_eq!(doc.blocks()[0].kind, kind);
            assert_eq!(view.fold_candidate(&doc, id).unwrap(), None);
            assert_eq!(view.source_fold_count(), 0);
            assert_eq!(view.preview(), None);
            assert!(!view.is_collapsed(id));
            assert!(view.measure(ticket, 100).is_err());
            assert!(view.checkpoints().token(TileId(0)).is_none());
            assert_eq!(
                pass.advance_folded(&doc, &mut view, &mut one(), &mut Sink::default()),
                Err(LayoutError::StaleContinuation)
            );
            assert!(view.set_collapsed(&doc, id, true).is_err());
            assert!(view.set_preview(&doc, Some(id), &mut one()).is_err());
            let sink = render(&doc, &mut view, TileId(0), 8, true);
            assert!(sink.placeholders.is_empty() && sink.ends.is_empty());
            verify_model(&doc, &view, &[]);
        }
    }
}

#[test]
fn open_paragraph_late_table_retraction_keeps_only_a_nonempty_suffix() {
    for bytes in [0, 3, 5, 1000] {
        for mode in 0..3 {
            let mut doc = Document::new();
            doc.append(&format!("lead\n{}", "x".repeat(2048))).unwrap();
            let mut view = FoldView::new(&doc, 1).unwrap();
            let id = register_open(&doc, &mut view, BlockId(0), bytes);
            view.set_collapsed(&doc, id, mode != 0).unwrap();
            if mode == 2 {
                view.set_preview(&doc, Some(id), &mut one()).unwrap();
            }
            let update = doc.append("|b\n-|-\nx|y\n").unwrap();
            view.apply(&doc, &update).unwrap();
            assert_eq!(doc.blocks()[0].source, Span::new(0, 5));
            assert_eq!(doc.tables().len(), 1);
            if bytes < 5 {
                let c = view.fold_candidate(&doc, id).unwrap().unwrap();
                assert_eq!(c.hidden, Span::new(bytes, 5));
                assert!(!c.open);
                assert_eq!(view.is_collapsed(id), mode != 0);
                view.set_preview(&doc, None, &mut one()).unwrap();
                verify_model(&doc, &view, &[c]);
            } else {
                assert_eq!(view.fold_candidate(&doc, id).unwrap(), None);
                assert_eq!(view.preview(), None);
                verify_model(&doc, &view, &[]);
            }
            assert!(hidden(&doc, &view, 5).is_none());
        }
    }
}

#[test]
fn open_paragraph_failed_probes_and_inline_closure_preserve_id() {
    for (prefix, tail) in [
        ("*prefix\n", "emphasis*\n"),
        ("`prefix\n", "code`\n"),
        ("$prefix\n", "math$\n"),
        ("[prefix\n", "link](/url)\n"),
        ("prefix\n", "=== nope\n"),
        ("a|b\n", "-|bad\n"),
        ("prefix\n", "-\r"),
    ] {
        let mut doc = Document::new();
        doc.append(prefix).unwrap();
        let mut view = FoldView::new(&doc, 1).unwrap();
        let id = register_open(&doc, &mut view, BlockId(0), 2);
        view.set_collapsed(&doc, id, true).unwrap();
        for ch in tail.chars() {
            let update = doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            view.apply(&doc, &update).unwrap();
            let c = view.fold_candidate(&doc, id).unwrap().unwrap();
            assert_eq!(c.hidden.end, doc.source().len());
            verify_model(&doc, &view, &[c]);
        }
        let update = doc.append("x\n\nnext\n").unwrap();
        view.apply(&doc, &update).unwrap();
        let c = view.fold_candidate(&doc, id).unwrap().unwrap();
        assert!(!c.open);
        let small = render(&doc, &mut view, TileId(0), 7, true);
        view.reset(2).unwrap();
        let big = render(&doc, &mut view, TileId(0), 7, false);
        assert_eq!(small.text, big.text);
        assert_eq!(small.positions, big.positions);
        assert_eq!(small.ends, vec![BlockId(0)]);
    }
}

#[test]
fn open_paragraph_million_byte_growth_has_one_refresh_and_no_body_recovery() {
    let mut doc = Document::new();
    doc.append("[prefix]\n").unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let id = register_open(&doc, &mut view, BlockId(0), 8);
    view.set_collapsed(&doc, id, true).unwrap();
    let update = doc.append(&"x".repeat(1_000_000)).unwrap();
    view.apply(&doc, &update).unwrap();
    let work = doc.work();
    let visits = view.range_visits();
    for _ in 0..1000 {
        let update = doc.append("x").unwrap();
        view.apply(&doc, &update).unwrap();
    }
    assert_eq!(view.range_visits() - visits, 1000);
    assert_eq!(doc.work().input_bytes - work.input_bytes, 1000);
    assert_eq!(doc.work().inline_bytes - work.inline_bytes, 1000);
    assert_eq!(doc.work().classified_bytes, work.classified_bytes);
    assert_eq!(doc.work().container_steps, work.container_steps);
    let work = doc.work();
    let sink = render(&doc, &mut view, TileId(0), 8, true);
    assert_eq!(sink.text, "prefix~\n");
    assert_eq!(sink.ends, vec![BlockId(0)]);
    assert!(sink.bytes <= 8 && sink.objects <= 1);
    assert_eq!(doc.work(), work);
    let update = doc.append("\n\nnext\n\n[prefix]: /late\n").unwrap();
    view.apply(&doc, &update).unwrap();
    let c = view.fold_candidate(&doc, id).unwrap().unwrap();
    assert!(!c.open);
    let sink = render(&doc, &mut view, TileId(2), 8, true);
    assert!(sink.placeholders.is_empty());
    assert_eq!(sink.ends, vec![BlockId(0)]);
    assert!(sink.bytes < 40 && sink.text.contains("next"));
    let visits = view.range_visits();
    for _ in 0..1000 {
        let update = doc.append("z").unwrap();
        view.apply(&doc, &update).unwrap();
    }
    assert_eq!(view.range_visits(), visits);
    assert_eq!(view.fold_candidate(&doc, id).unwrap(), Some(c));
}

#[test]
fn open_paragraph_all_splits_live_prefixes_and_merged_updates_agree() {
    for (initial, tail) in [
        (
            "中🙂 *prefix\r\n".to_string(),
            "across*\r\n\r\nafter\n".to_string(),
        ),
        ("中🙂\n".to_string(), "more\n===\nnext\n".to_string()),
        ("a|b\n".to_string(), "-|-\nx|y\n\n> quote\n".to_string()),
        (
            format!("lead\n{}", "x".repeat(1100)),
            "|中\r\n-|-\r\nx|y\r\n".to_string(),
        ),
        ("a|b\n".to_string(), "not a delimiter\n\nend\n".to_string()),
    ] {
        for bytes in [0, 3] {
            let mut baseline = Document::new();
            baseline.append(&initial).unwrap();
            let mut expected = FoldView::new(&baseline, 1).unwrap();
            let baseline_id = register_open(&baseline, &mut expected, BlockId(0), bytes);
            expected
                .set_collapsed(&baseline, baseline_id, true)
                .unwrap();
            let mut update = baseline.append(&tail).unwrap();
            update.merge(baseline.finish()).unwrap();
            expected.apply(&baseline, &update).unwrap();
            let shape = |c: FoldCandidate| (c.parent, c.source, c.hidden, c.summary, c.open);
            let expected_candidate = expected
                .fold_candidate(&baseline, baseline_id)
                .unwrap()
                .map(shape);
            let expected_sink = render(&baseline, &mut expected, TileId(0), 9, true);
            for split in (0..=tail.len()).filter(|&i| tail.is_char_boundary(i)) {
                for merged in [false, true] {
                    let mut doc = Document::new();
                    doc.append(&initial).unwrap();
                    let mut view = FoldView::new(&doc, 1).unwrap();
                    let id = register_open(&doc, &mut view, BlockId(0), bytes);
                    view.set_collapsed(&doc, id, true).unwrap();
                    let mut update = doc.append(&tail[..split]).unwrap();
                    if !merged {
                        view.apply(&doc, &update).unwrap();
                    }
                    let second = doc.append(&tail[split..]).unwrap();
                    if merged {
                        update.merge(second).unwrap();
                    } else {
                        update = second;
                    }
                    update.merge(doc.finish()).unwrap();
                    view.apply(&doc, &update).unwrap();
                    view.apply(&doc, &update).unwrap();
                    assert_eq!(doc.blocks(), baseline.blocks());
                    assert_eq!(
                        view.fold_candidate(&doc, id).unwrap().map(shape),
                        expected_candidate
                    );
                    let sink = render(&doc, &mut view, TileId(0), 9, true);
                    assert_eq!(sink.text, expected_sink.text);
                    assert_eq!(sink.positions, expected_sink.positions);
                    assert_eq!(sink.ends, expected_sink.ends);
                }
            }
            let mut doc = Document::new();
            doc.append(&initial).unwrap();
            let mut view = FoldView::new(&doc, 1).unwrap();
            let id = register_open(&doc, &mut view, BlockId(0), bytes);
            view.set_collapsed(&doc, id, true).unwrap();
            for ch in tail.chars() {
                let update = doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
                view.apply(&doc, &update).unwrap();
                let mut oracle = Document::new();
                oracle.append(&initial).unwrap();
                let mut oracle_view = FoldView::new(&oracle, 1).unwrap();
                let oid = register_open(&oracle, &mut oracle_view, BlockId(0), bytes);
                oracle_view.set_collapsed(&oracle, oid, true).unwrap();
                let update = oracle.append(&doc.source()[initial.len()..]).unwrap();
                oracle_view.apply(&oracle, &update).unwrap();
                assert_eq!(
                    view.fold_candidate(&doc, id).unwrap().map(shape),
                    oracle_view.fold_candidate(&oracle, oid).unwrap().map(shape)
                );
                let candidates: Vec<_> =
                    view.fold_candidate(&doc, id).unwrap().into_iter().collect();
                verify_model(&doc, &view, &candidates);
            }
        }
    }
}

#[test]
fn open_paragraph_history_does_not_increase_refresh_work_or_reservations() {
    let mut doc = Document::new();
    doc.append(&"paragraph\n\n".repeat(4096)).unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    for i in 0..4096 {
        let id = register(&doc, &mut view, BlockId(i * 2), 3);
        view.set_collapsed(&doc, id, true).unwrap();
    }
    let update = doc.append("active\n").unwrap();
    view.apply(&doc, &update).unwrap();
    let block = BlockId(8192);
    let id = register_open(&doc, &mut view, block, 3);
    view.set_collapsed(&doc, id, true).unwrap();
    let visits = view.range_visits();
    let update = doc.append(&"x".repeat(1_000_000)).unwrap();
    view.apply(&doc, &update).unwrap();
    assert_eq!(view.range_visits() - visits, 1);
    assert_eq!(view.source_fold_count(), 4097);
    let start = doc.blocks()[block.0].source.start;
    let mut candidates =
        SourceFoldCandidates::new(&doc, &view, Span::new(start + 100, start + 300)).unwrap();
    let mut out = Vec::new();
    assert_eq!(
        candidates
            .advance(&doc, &view, &mut one(), &mut out)
            .unwrap(),
        Completion::Covered
    );
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].id, id);
    let update = doc.append("\n===\n").unwrap();
    view.apply(&doc, &update).unwrap();
    assert_eq!(view.source_fold_count(), 4096);
    assert!(!view.is_collapsed(id));
    let visits = view.range_visits();
    for _ in 0..1000 {
        let update = doc.append("x").unwrap();
        view.apply(&doc, &update).unwrap();
    }
    assert_eq!(view.range_visits(), visits);
}

#[test]
fn open_paragraph_preview_navigation_and_batch_follow_the_current_end() {
    let mut doc = Document::new();
    doc.append("# root\nparagraph\n").unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let id = register_open(&doc, &mut view, BlockId(1), 3);
    view.set_collapsed(&doc, id, true).unwrap();
    view.set_preview(&doc, Some(id), &mut one()).unwrap();
    let update = doc.append(&"x".repeat(2048)).unwrap();
    view.apply(&doc, &update).unwrap();
    assert!(view.is_collapsed(id));
    assert_eq!(hidden(&doc, &view, 2000), None);
    view.reset(2).unwrap();
    view.set_preview(&doc, None, &mut one()).unwrap();
    assert_eq!(hidden(&doc, &view, 2000), Some(id));
    let root = FoldId::Heading(HeadingId(0));
    view.set_collapsed(&doc, root, true).unwrap();
    let mut nav = FoldNavigation::new(
        &doc,
        &view,
        &Navigator::new(),
        NavAction::Source(2000),
        FoldNavigationPolicy::Reveal,
        10,
    )
    .unwrap();
    let mut done = false;
    for _ in 0..30 {
        let p = nav.advance(&doc, &mut view, &mut one()).unwrap();
        if p.completion == Completion::Covered {
            assert_eq!(p.target.unwrap().source, Some(2000));
            done = true;
            break;
        }
    }
    assert!(done && !view.is_collapsed(root) && !view.is_collapsed(id));
    let mut batch = FoldBatch::new(
        &doc,
        &view,
        FoldBatchAction::Subtree {
            root: id,
            collapsed: true,
        },
    )
    .unwrap();
    let mut done = false;
    for _ in 0..20 {
        if batch
            .advance(&doc, &mut view, &mut one())
            .unwrap()
            .completion
            == Completion::Covered
        {
            done = true;
            break;
        }
    }
    assert!(done && view.is_collapsed(id) && !view.is_collapsed(root));
    let anchor = FoldAnchor::new(&doc, 2000).unwrap();
    assert_eq!(
        anchor
            .resolve(&doc, &view, 10, &mut one())
            .unwrap()
            .unwrap()
            .hidden_by,
        Some(id)
    );
    let update = doc.append("\n\nnext\n").unwrap();
    view.apply(&doc, &update).unwrap();
    assert_eq!(hidden(&doc, &view, 2000), Some(id));
    let sink = render(&doc, &mut view, TileId(0), 7, true);
    assert_eq!(sink.ends, vec![BlockId(1)]);
    assert!(sink.text.ends_with("\nnext\n"));
}

#[test]
fn open_paragraph_old_registration_readers_and_measurements_are_rejected() {
    let mut doc = Document::new();
    doc.append("paragraph\n").unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let mut stale = SourceFoldRegistration::open_paragraph(&doc, &view, BlockId(0), 0).unwrap();
    let id = register_open(&doc, &mut view, BlockId(0), 3);
    assert_eq!(
        stale.advance(&doc, &mut view, &mut one()),
        Err(LayoutError::StaleContinuation)
    );
    view.set_collapsed(&doc, id, true).unwrap();
    let mut candidates =
        SourceFoldCandidates::new(&doc, &view, Span::new(0, doc.source().len())).unwrap();
    let mut pass = LayoutPass::folded(&doc, &view, TileId(0), 20).unwrap();
    let ticket = view.heights().ticket(TileId(0)).unwrap();
    view.insert_checkpoint(TileId(0), ()).unwrap();
    let mut update = doc.append("pending").unwrap();
    update.merge(doc.append("\n\nnext\n").unwrap()).unwrap();
    view.apply(&doc, &update).unwrap();
    view.apply(&doc, &update).unwrap();
    assert!(view.measure(ticket, 9).is_err());
    assert!(view.checkpoints().token(TileId(0)).is_none());
    assert_eq!(
        candidates.advance(&doc, &view, &mut one(), &mut Vec::new()),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(
        pass.advance_folded(&doc, &mut view, &mut one(), &mut Sink::default()),
        Err(LayoutError::StaleContinuation)
    );
    assert!(view.is_collapsed(id));
    view.remove_source_fold(&doc, source_id(id)).unwrap();
    let update = doc.append("more\n").unwrap();
    view.apply(&doc, &update).unwrap();
    let block = doc.block_at(doc.source().find("next").unwrap()).unwrap();
    let mut stale = SourceFoldRegistration::open_paragraph(&doc, &view, block, 1).unwrap();
    let update = doc.append("tail").unwrap();
    view.apply(&doc, &update).unwrap();
    assert_eq!(
        stale.advance(&doc, &mut view, &mut one()),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(view.source_fold_count(), 0);
}

#[test]
fn open_paragraph_pending_end_and_checkpoint_recovery_use_shared_budget() {
    let mut doc = Document::new();
    doc.append(&"x".repeat(1_000_000)).unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let id = register_open(&doc, &mut view, BlockId(0), 0);
    view.set_collapsed(&doc, id, true).unwrap();
    let mut pass = LayoutPass::folded(&doc, &view, TileId(0), u64::MAX).unwrap();
    let mut sink = Sink {
        width: 7,
        ..Default::default()
    };
    let mut budget = Budget::new(WorkLimit {
        objects: 1,
        ..WorkLimit::unlimited()
    });
    assert_eq!(
        pass.advance_folded(&doc, &mut view, &mut budget, &mut sink)
            .unwrap(),
        Completion::BudgetExhausted
    );
    assert_eq!(sink.placeholders.len(), 1);
    assert!(sink.ends.is_empty());
    for _ in 0..3 {
        let mut budget = Budget::new(WorkLimit {
            objects: 0,
            ..WorkLimit::unlimited()
        });
        assert_eq!(
            pass.advance_folded(&doc, &mut view, &mut budget, &mut sink)
                .unwrap(),
            Completion::BudgetExhausted
        );
        assert_eq!(budget.used(), WorkLimit::default());
    }
    assert_eq!(
        pass.advance_folded(
            &doc,
            &mut view,
            &mut Budget::new(WorkLimit::unlimited()),
            &mut sink
        )
        .unwrap(),
        Completion::EndOfDocument
    );
    assert_eq!(sink.ends, vec![BlockId(0)]);
    assert_eq!(sink.bytes, 0);
    let update = doc.append("\n\nnext\n").unwrap();
    view.apply(&doc, &update).unwrap();
    view.insert_checkpoint(TileId(0), ()).unwrap();
    let mut pass =
        LayoutPass::folded_from_checkpoint(&doc, &view, TileId(0), TileId(2), u64::MAX, 1024)
            .unwrap()
            .unwrap();
    let mut sink = Sink {
        width: 7,
        ..Default::default()
    };
    let mut done = false;
    for _ in 0..100 {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            tiles: 1,
            bytes: 4,
            recovery: 1,
            ..Default::default()
        });
        let completion = pass
            .advance_folded(&doc, &mut view, &mut budget, &mut sink)
            .unwrap();
        assert!(budget.used().recovery <= 1);
        if completion == Completion::EndOfDocument {
            done = true;
            break;
        }
    }
    assert!(done && sink.bytes < 20);
    assert_eq!(sink.text, "~\n\nnext\n");
    assert_eq!(sink.ends, vec![BlockId(0)]);
}

#[test]
fn open_paragraph_streaming_interval_and_height_model() {
    let mut doc = Document::new();
    doc.append("# root\n").unwrap();
    let mut view = FoldView::new(&doc, 3).unwrap();
    let mut ids = Vec::new();
    let mut seed = 123u64;
    for turn in 0..240 {
        let start = doc.source().len();
        let update = doc.append(&format!("paragraph {turn}\n")).unwrap();
        view.apply(&doc, &update).unwrap();
        let block = doc.block_at(start).unwrap();
        let id = register_open(&doc, &mut view, block, turn % 6);
        ids.push((id, block, start + turn % 6));
        for chunk in [
            "x".repeat(turn % 5 * 129 + 1),
            if turn % 3 == 0 { "\n===\n\n" } else { "\n\n" }.to_string(),
        ] {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let update = doc.append(&chunk).unwrap();
            view.apply(&doc, &update).unwrap();
            let mut expected = Vec::new();
            for &(id, block, cut) in &ids {
                let b = doc.block(block).unwrap();
                if b.kind != BlockKind::Paragraph {
                    assert_eq!(view.fold_candidate(&doc, id).unwrap(), None);
                    continue;
                }
                let open = b.state != SyntaxState::Sealed;
                let end = if open {
                    doc.source().len()
                } else {
                    b.source.end
                };
                let c = view.fold_candidate(&doc, id).unwrap().unwrap();
                assert_eq!(c.source, Span::new(b.source.start, end));
                assert_eq!(c.hidden, Span::new(cut, end));
                assert_eq!(c.open, open);
                expected.push(c);
            }
            for i in 0..doc.headings().entries().len() {
                expected.push(doc.fold_candidate(FoldId::Heading(HeadingId(i))).unwrap());
            }
            let target = expected[(seed >> 32) as usize % expected.len()].id;
            view.toggle(&doc, target).unwrap();
            if turn % 7 == 0 {
                view.set_preview(&doc, Some(target), &mut one()).unwrap();
            }
            if turn % 13 == 0 {
                view.reset(2).unwrap();
            }
            verify_model(&doc, &view, &expected);
        }
    }
}

#[test]
fn open_paragraph_reservation_released_on_retraction_and_removal() {
    let mut doc = Document::new();
    doc.append("old paragraph\n\nactive\n").unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let manual = {
        let mut task = SourceFoldRegistration::new(&doc, &view, Span::new(0, 3)).unwrap();
        finish(&mut task, &doc, &mut view)
    };
    let id = register_open(&doc, &mut view, BlockId(2), 3);
    view.set_collapsed(&doc, manual, true).unwrap();
    view.set_collapsed(&doc, id, true).unwrap();
    let update = doc.append("\nnext\n\n").unwrap();
    view.apply(&doc, &update).unwrap();
    assert!(SourceFoldRegistration::new(&doc, &view, Span::new(15, 16)).is_err());
    let after = doc.source().find("next").unwrap();
    let mut task = SourceFoldRegistration::new(&doc, &view, Span::new(after, after + 4)).unwrap();
    let next = finish(&mut task, &doc, &mut view);
    view.set_collapsed(&doc, next, true).unwrap();
    let candidates = [manual, id, next].map(|id| view.fold_candidate(&doc, id).unwrap().unwrap());
    verify_model(&doc, &view, &candidates);
    view.remove_source_fold(&doc, source_id(id)).unwrap();
    let block = doc.blocks()[2].source;
    let mut task = SourceFoldRegistration::new(&doc, &view, block).unwrap();
    let replaced = finish(&mut task, &doc, &mut view);
    assert_ne!(id, replaced);
    assert_eq!(view.source_fold_count(), 3);
}

#[test]
fn open_paragraph_eof_and_retracted_boundaries_match_scalar_layout_model() {
    for length in [255, 256, 257, 511, 512, 513, 1023, 1024] {
        for width in [1, 3, 7, 17] {
            for bytes in [0, 3, length - 1] {
                for suffix in ["\n\nnext\n", "\n# next\n", "\n> next\n", ""] {
                    let mut doc = Document::new();
                    doc.append(&format!("{}\n", "x".repeat(length))).unwrap();
                    let mut view = FoldView::new(&doc, 1).unwrap();
                    let id = register_open(&doc, &mut view, BlockId(0), bytes);
                    view.set_collapsed(&doc, id, true).unwrap();
                    // Pending source can cross a tile without joining the final
                    // paragraph. Freeze the original end when classified later.
                    let update = doc.append(suffix).unwrap();
                    view.apply(&doc, &update).unwrap();
                    let update = doc.finish();
                    view.apply(&doc, &update).unwrap();
                    let c = view.fold_candidate(&doc, id).unwrap().unwrap();
                    assert!(!c.open);
                    verify_model(&doc, &view, &[c]);
                    let tiny = render(&doc, &mut view, TileId(0), width, true);
                    view.reset(1).unwrap();
                    let big = render(&doc, &mut view, TileId(0), width, false);
                    assert_eq!(tiny.text, big.text);
                    assert_eq!(tiny.positions, big.positions);
                    assert_eq!(tiny.ends, vec![BlockId(0)]);
                    let expected = format!(
                        "{}~\n{}",
                        &doc.source()[..bytes],
                        &doc.source()[c.hidden.end..]
                    );
                    let oracle = parse(&expected);
                    let mut oracle_view = FoldView::new(&oracle, 1).unwrap();
                    let oracle = render(&oracle, &mut oracle_view, TileId(0), width, false);
                    assert_eq!(tiny.text, oracle.text);
                    assert_eq!(tiny.total, oracle.total);
                    assert_eq!(view.heights().total_rows(), u128::from(big.total));
                }
            }
        }
    }
}

#[test]
fn open_paragraph_closes_once_before_deep_containers_and_never_revisits_them() {
    let mut doc = Document::new();
    doc.append("paragraph\n").unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let id = register_open(&doc, &mut view, BlockId(0), 3);
    view.set_collapsed(&doc, id, true).unwrap();
    let update = doc.append(&format!("{} body\n", ">".repeat(1000))).unwrap();
    view.apply(&doc, &update).unwrap();
    let c = view.fold_candidate(&doc, id).unwrap().unwrap();
    assert!(!c.open);
    assert_eq!(c.source, Span::new(0, 10));
    assert_eq!(doc.container_count(), 1000);
    let visits = view.range_visits();
    let update = doc.append(&"x".repeat(1_000_000)).unwrap();
    view.apply(&doc, &update).unwrap();
    let work = doc.work();
    for _ in 0..1000 {
        let update = doc.append("x").unwrap();
        view.apply(&doc, &update).unwrap();
    }
    assert_eq!(view.range_visits(), visits);
    assert_eq!(doc.work().container_steps, work.container_steps);
    assert_eq!(view.fold_candidate(&doc, id).unwrap(), Some(c));
    assert!(
        SourceFoldRegistration::open_paragraph(&doc, &view, BlockId(doc.blocks().len() - 1), 0)
            .is_err()
    );
}

fn register_container(doc: &Document, view: &mut FoldView, block: BlockId, bytes: usize) -> FoldId {
    let mut task = SourceFoldRegistration::container_paragraph(doc, view, block, bytes).unwrap();
    let count = view.source_fold_count();
    for _ in 0..2 * doc.container_count() + 30 {
        let mut budget = one();
        let p = task.advance(doc, view, &mut budget).unwrap();
        assert_eq!(budget.used().bytes, 0);
        if let Some(id) = p.id {
            assert_eq!(p.completion, Completion::Covered);
            assert_eq!(view.source_fold_count(), count + 1);
            return id;
        }
        assert_eq!(view.source_fold_count(), count);
    }
    panic!("container registration stalled")
}

#[test]
fn container_paragraph_first_leaf_identity_and_closed_multiline_ranges() {
    let source = "> a中🙂bc\r\n> second\r\n>\r\n> third\n\n- item\n  continued\n- sibling\n";
    let doc = parse(source);
    for (text, end) in [
        ("> a", source.find(">\r").unwrap()),
        ("> third", source.find("\n\n-").unwrap() + 1),
        ("- item", source.find("- sibling").unwrap()),
        ("- sibling", source.len()),
    ] {
        let start = source.find(text).unwrap();
        let first = doc.block_at(start).unwrap();
        let mut view = FoldView::new(&doc, 2).unwrap();
        let id = register_container(&doc, &mut view, first, 3);
        let c = view.fold_candidate(&doc, id).unwrap().unwrap();
        assert_eq!(c.source, Span::new(start, end));
        assert_eq!(c.hidden, Span::new(start + 3, end));
        assert!(!c.open);
        assert_eq!(
            view.paragraph_fold_block(&doc, source_id(id)).unwrap(),
            Some(first)
        );
        assert_eq!(
            c.parent,
            doc.block(first).unwrap().container.map(FoldId::Container)
        );
        view.set_collapsed(&doc, id, true).unwrap();
        verify_model(&doc, &view, &[c]);
        view.reset(1).unwrap();
        assert_eq!(view.fold_candidate(&doc, id).unwrap(), Some(c));
    }
    for (limit, cut) in [
        (0, 0),
        (3, 3),
        (5, 3),
        (6, 6),
        (9, 6),
        (10, 10),
        (13, 12),
        (15, 15),
    ] {
        let mut view = FoldView::new(&doc, 1).unwrap();
        let id = register_container(&doc, &mut view, BlockId(0), limit);
        assert_eq!(
            view.fold_candidate(&doc, id).unwrap().unwrap().hidden.start,
            cut
        );
    }
}

#[test]
fn container_paragraph_rejects_other_leaves_nontext_and_overlap_atomically() {
    let doc = parse(
        "> first\n> second\n>\n> # heading\n> ```\n> code\n> ```\n> [r]: /url\n> a|b\n> -|-\n\nroot\n",
    );
    let mut view = FoldView::new(&doc, 1).unwrap();
    for i in 1..doc.blocks().len() {
        assert!(
            SourceFoldRegistration::container_paragraph(&doc, &view, BlockId(i), 0).is_err(),
            "{i}: {:?}",
            doc.blocks()[i]
        );
    }
    assert!(SourceFoldRegistration::container_paragraph(&doc, &view, BlockId(0), 17).is_err());
    let mut task = SourceFoldRegistration::container_paragraph(&doc, &view, BlockId(0), 4).unwrap();
    let p = task
        .advance(&doc, &mut view, &mut Budget::new(WorkLimit::default()))
        .unwrap();
    assert_eq!(p.completion, Completion::BudgetExhausted);
    assert_eq!(view.source_fold_count(), 0);
    let id = register_container(&doc, &mut view, BlockId(0), 4);
    assert_eq!(
        task.advance(&doc, &mut view, &mut one()),
        Err(LayoutError::StaleContinuation)
    );
    assert!(SourceFoldRegistration::new(&doc, &view, Span::new(0, 3)).is_err());
    assert!(SourceFoldRegistration::container_paragraph(&doc, &view, BlockId(0), 1).is_err());
    let other = FoldView::new(&doc, 1).unwrap();
    assert!(other.fold_candidate(&doc, id).unwrap().is_none());
    view.remove_source_fold(&doc, source_id(id)).unwrap();
    let mut manual = SourceFoldRegistration::new(&doc, &view, Span::new(3, 7)).unwrap();
    finish(&mut manual, &doc, &mut view);
    assert!(SourceFoldRegistration::container_paragraph(&doc, &view, BlockId(0), 1).is_err());
}

#[test]
fn container_paragraph_growth_scope_changes_and_blank_boundaries() {
    for (initial, tail, end) in [
        ("> one\n", "> two\n>\n> next\n", 12),
        ("> one\n", "lazy\n\nnext\n", 11),
        ("> one\n", "> > nested\n", 6),
        ("- one\n", "  two\n- next\n", 12),
        ("> one\n", "> # next\n", 6),
        ("> one\n", "> ```\n> code\n", 6),
        ("> one\n", "> $$\n> math\n", 6),
        ("> one\n", "> [r]: /url\n", 6),
        ("> one\n", "> [^f]: note\n", 6),
    ] {
        let mut doc = Document::new();
        doc.append(initial).unwrap();
        let mut view = FoldView::new(&doc, 1).unwrap();
        let id = register_container(&doc, &mut view, BlockId(0), 3);
        view.set_collapsed(&doc, id, true).unwrap();
        assert!(view.fold_candidate(&doc, id).unwrap().unwrap().open);
        let update = doc.append(tail).unwrap();
        view.apply(&doc, &update).unwrap();
        let c = view.fold_candidate(&doc, id).unwrap().unwrap();
        assert_eq!(c.source, Span::new(0, end), "{initial:?} {tail:?}");
        assert!(!c.open && view.is_collapsed(id));
        let visits = view.range_visits();
        let update = doc.append("more").unwrap();
        view.apply(&doc, &update).unwrap();
        assert_eq!(view.fold_candidate(&doc, id).unwrap(), Some(c));
        // One old-EOF correction may be required, but never a paragraph refresh.
        assert!(view.range_visits() - visits <= 1);
    }
}

#[test]
fn container_paragraph_late_setext_and_tables_remove_or_retract() {
    for (initial, tail, expected_end) in [
        ("> one\n> two\n", "> ===\n", None),
        ("> a|b\n", "> -|-\n", None),
        ("> lead\n> a|b\n", "> -|-\n", Some(7)),
        ("> lead\n> a|b\n", "> nope\n>\n", Some(20)),
    ] {
        for preview in [false, true] {
            let mut doc = Document::new();
            doc.append(initial).unwrap();
            let mut view = FoldView::new(&doc, 1).unwrap();
            let id = register_container(&doc, &mut view, BlockId(0), 3);
            view.set_collapsed(&doc, id, true).unwrap();
            if preview {
                view.set_preview(&doc, Some(id), &mut one()).unwrap();
            }
            let update = doc.append(tail).unwrap();
            view.apply(&doc, &update).unwrap();
            let c = view.fold_candidate(&doc, id).unwrap();
            assert_eq!(
                c.map(|c| c.source.end),
                expected_end,
                "{initial:?} {tail:?}"
            );
            if expected_end.is_none() {
                assert_eq!(view.source_fold_count(), 0);
                assert!(!view.is_collapsed(id));
                assert_eq!(view.preview(), None);
                assert!(
                    SourceFoldRegistration::container_paragraph(&doc, &view, BlockId(0), 0)
                        .is_err()
                );
            } else {
                assert!(!c.unwrap().open);
                assert!(view.is_collapsed(id));
                assert_eq!(view.preview(), preview.then_some(id));
            }
        }
    }
    let mut doc = Document::new();
    doc.append("> lead\n> a|b\n").unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let id = register_container(&doc, &mut view, BlockId(0), 8);
    let update = doc.append("> -|-\n").unwrap();
    view.apply(&doc, &update).unwrap();
    assert!(view.fold_candidate(&doc, id).unwrap().is_none());
    // The surviving paragraph remains independently registerable with a shorter prefix.
    register_container(&doc, &mut view, BlockId(0), 3);
}

#[test]
fn container_paragraph_deep_million_byte_growth_and_thousands_of_leaves() {
    let mut doc = Document::new();
    doc.append(&format!("{}body\n", ">".repeat(1000))).unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let id = register_container(&doc, &mut view, BlockId(0), 1002);
    view.set_collapsed(&doc, id, true).unwrap();
    let update = doc
        .append(&format!("{}{}", ">".repeat(1000), "x".repeat(1_000_000)))
        .unwrap();
    view.apply(&doc, &update).unwrap();
    let before = doc.work();
    let visits = view.range_visits();
    for _ in 0..1000 {
        let update = doc.append("x").unwrap();
        view.apply(&doc, &update).unwrap();
    }
    assert_eq!(view.range_visits() - visits, 1000);
    assert_eq!(doc.work().container_steps, before.container_steps);
    assert_eq!(doc.work().heading_steps, before.heading_steps);
    assert_eq!(doc.work().paragraph_closures, 0);
    assert_eq!(doc.work().input_bytes - before.input_bytes, 1000);
    assert_eq!(doc.work().inline_bytes - before.inline_bytes, 1000);
    let update = doc.append("\n\n# outside\n").unwrap();
    view.apply(&doc, &update).unwrap();
    assert_eq!(doc.work().paragraph_closures, 1);
    let sink = render(&doc, &mut view, TileId(5), 8, true);
    assert_eq!(sink.ends, vec![BlockId(0)]);
    assert!(
        sink.bytes < 20 && sink.text.contains("outside"),
        "bytes={} text={:?}",
        sink.bytes,
        sink.text
    );
    assert!(sink.placeholders.is_empty());

    let mut doc = Document::new();
    doc.append(&"> line\n".repeat(4096)).unwrap();
    assert_eq!(doc.work().paragraph_closures, 0);
    let old = doc.blocks().to_vec();
    doc.append(">\n").unwrap();
    assert_eq!(doc.work().paragraph_closures, 1);
    assert_eq!(&doc.blocks()[..old.len()], old.as_slice());
    let mut view = FoldView::new(&doc, 1).unwrap();
    let id = register_container(&doc, &mut view, BlockId(0), 3);
    assert_eq!(
        view.fold_candidate(&doc, id).unwrap().unwrap().source.end,
        7 * 4096
    );
    for i in [1, 2048, 4095] {
        assert!(SourceFoldRegistration::container_paragraph(&doc, &view, BlockId(i), 0).is_err());
    }
}

#[test]
fn container_paragraph_closed_history_does_not_add_append_refreshes() {
    let mut doc = Document::new();
    doc.append(&"> body\n>\n".repeat(4096)).unwrap();
    assert_eq!(doc.work().paragraph_closures, 4096);
    let mut view = FoldView::new(&doc, 1).unwrap();
    for i in 0..4096 {
        let id = register_container(&doc, &mut view, BlockId(i * 2), 3);
        view.set_collapsed(&doc, id, true).unwrap();
    }
    let update = doc.append("> active\n").unwrap();
    view.apply(&doc, &update).unwrap();
    let id = register_container(&doc, &mut view, BlockId(8192), 3);
    let visits = view.range_visits();
    for _ in 0..1000 {
        let update = doc.append("z").unwrap();
        view.apply(&doc, &update).unwrap();
    }
    assert_eq!(view.range_visits() - visits, 1000);
    assert_eq!(doc.work().paragraph_closures, 4096);
    assert_eq!(view.source_fold_count(), 4097);
    let update = doc.finish();
    view.apply(&doc, &update).unwrap();
    assert!(!view.fold_candidate(&doc, id).unwrap().unwrap().open);
    assert_eq!(doc.work().paragraph_closures, 4097);
}

#[test]
fn container_paragraph_pending_end_and_checkpoint_recovery_use_shared_budget() {
    let mut doc = Document::new();
    doc.append(&format!("> {}", "x".repeat(1_000_000))).unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let id = register_container(&doc, &mut view, BlockId(0), 0);
    view.set_collapsed(&doc, id, true).unwrap();
    let mut pass = LayoutPass::folded(&doc, &view, TileId(0), u64::MAX).unwrap();
    let mut sink = Sink {
        width: 7,
        ..Default::default()
    };
    let mut budget = Budget::new(WorkLimit {
        objects: 1,
        ..WorkLimit::unlimited()
    });
    assert_eq!(
        pass.advance_folded(&doc, &mut view, &mut budget, &mut sink)
            .unwrap(),
        Completion::BudgetExhausted
    );
    assert_eq!(sink.placeholders.len(), 1);
    assert!(sink.ends.is_empty());
    for _ in 0..3 {
        let mut budget = Budget::new(WorkLimit {
            objects: 0,
            ..WorkLimit::unlimited()
        });
        assert_eq!(
            pass.advance_folded(&doc, &mut view, &mut budget, &mut sink)
                .unwrap(),
            Completion::BudgetExhausted
        );
        assert_eq!(budget.used(), WorkLimit::default());
    }
    assert_eq!(
        pass.advance_folded(
            &doc,
            &mut view,
            &mut Budget::new(WorkLimit::unlimited()),
            &mut sink
        )
        .unwrap(),
        Completion::EndOfDocument
    );
    assert_eq!(sink.ends, vec![BlockId(0)]);
    assert_eq!(sink.bytes, 0);
    let update = doc.append("\n\nnext\n").unwrap();
    view.apply(&doc, &update).unwrap();
    view.insert_checkpoint(TileId(0), ()).unwrap();
    let mut pass =
        LayoutPass::folded_from_checkpoint(&doc, &view, TileId(0), TileId(2), u64::MAX, 1024)
            .unwrap()
            .unwrap();
    let mut sink = Sink {
        width: 7,
        ..Default::default()
    };
    let mut done = false;
    for _ in 0..100 {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            tiles: 1,
            bytes: 4,
            recovery: 1,
            ..Default::default()
        });
        let completion = pass
            .advance_folded(&doc, &mut view, &mut budget, &mut sink)
            .unwrap();
        assert!(budget.used().recovery <= 1);
        if completion == Completion::EndOfDocument {
            done = true;
            break;
        }
    }
    assert!(done && sink.bytes < 20);
    assert_eq!(sink.text, "~\n\nnext\n");
    assert_eq!(sink.ends, vec![BlockId(0)]);
}

#[test]
fn container_paragraph_old_registration_readers_and_measurements_are_rejected() {
    let mut doc = Document::new();
    doc.append("> paragraph\n").unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let mut stale =
        SourceFoldRegistration::container_paragraph(&doc, &view, BlockId(0), 0).unwrap();
    let id = register_container(&doc, &mut view, BlockId(0), 3);
    assert_eq!(
        stale.advance(&doc, &mut view, &mut one()),
        Err(LayoutError::StaleContinuation)
    );
    view.set_collapsed(&doc, id, true).unwrap();
    let mut candidates =
        SourceFoldCandidates::new(&doc, &view, Span::new(0, doc.source().len())).unwrap();
    let mut pass = LayoutPass::folded(&doc, &view, TileId(0), 20).unwrap();
    let ticket = view.heights().ticket(TileId(0)).unwrap();
    view.insert_checkpoint(TileId(0), ()).unwrap();
    let mut update = doc.append("pending").unwrap();
    update.merge(doc.append("\n\n> next\n").unwrap()).unwrap();
    view.apply(&doc, &update).unwrap();
    view.apply(&doc, &update).unwrap();
    assert!(view.measure(ticket, 9).is_err());
    assert!(view.checkpoints().token(TileId(0)).is_none());
    assert_eq!(
        candidates.advance(&doc, &view, &mut one(), &mut Vec::new()),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(
        pass.advance_folded(&doc, &mut view, &mut one(), &mut Sink::default()),
        Err(LayoutError::StaleContinuation)
    );
    assert!(view.is_collapsed(id));
    view.remove_source_fold(&doc, source_id(id)).unwrap();
    let update = doc.append("more\n").unwrap();
    view.apply(&doc, &update).unwrap();
    let block = doc.block_at(doc.source().find("next").unwrap()).unwrap();
    let mut stale = SourceFoldRegistration::container_paragraph(&doc, &view, block, 1).unwrap();
    let update = doc.append("tail").unwrap();
    view.apply(&doc, &update).unwrap();
    assert_eq!(
        stale.advance(&doc, &mut view, &mut one()),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(view.source_fold_count(), 0);
}

#[test]
fn container_paragraph_preview_batch_anchor_and_navigation_preserve_choice() {
    let doc = parse("> # Root\n> abcdefgh\n>\n> # Other\n> body\n");
    let mut view = FoldView::new(&doc, 1).unwrap();
    let id = register_container(&doc, &mut view, BlockId(1), 3);
    let heading = FoldId::Container(ContainerId(0));
    view.set_collapsed(&doc, id, true).unwrap();
    view.set_collapsed(&doc, heading, true).unwrap();
    assert!(render(&doc, &mut view, TileId(0), 8, true).ends.is_empty());
    view.set_collapsed(&doc, heading, false).unwrap();
    assert!(view.is_collapsed(id));
    view.set_preview(&doc, Some(id), &mut one()).unwrap();
    assert!(render(&doc, &mut view, TileId(0), 8, true).ends.is_empty());
    view.set_preview(&doc, None, &mut one()).unwrap();
    assert_eq!(
        render(&doc, &mut view, TileId(0), 8, true).ends,
        vec![BlockId(1)]
    );
    let anchor = FoldAnchor::new(&doc, 16).unwrap();
    assert_eq!(
        anchor
            .resolve(&doc, &view, 10, &mut one())
            .unwrap()
            .unwrap()
            .hidden_by,
        Some(id)
    );
    let mut nav = FoldNavigation::new(
        &doc,
        &view,
        &Navigator::new(),
        NavAction::Source(16),
        FoldNavigationPolicy::Reveal,
        10,
    )
    .unwrap();
    for _ in 0..20 {
        let p = nav.advance(&doc, &mut view, &mut one()).unwrap();
        if p.completion == Completion::Covered {
            assert_eq!(p.target.unwrap().source, Some(16));
            break;
        }
    }
    assert!(!view.is_collapsed(id));
    let mut batch = FoldBatch::new(
        &doc,
        &view,
        FoldBatchAction::Subtree {
            root: id,
            collapsed: true,
        },
    )
    .unwrap();
    for _ in 0..20 {
        if batch
            .advance(&doc, &mut view, &mut one())
            .unwrap()
            .completion
            == Completion::Covered
        {
            break;
        }
    }
    assert!(view.is_collapsed(id) && !view.is_collapsed(heading));
}

#[test]
fn container_paragraph_all_splits_live_prefixes_and_merged_updates_agree() {
    let cases = [
        (
            "> 中🙂 *prefix\r\n".to_string(),
            "> across*\r\n>\r\n> after\n".to_string(),
        ),
        (
            "> 中🙂\n".to_string(),
            "> more\n> ===\n> next\n".to_string(),
        ),
        ("> a|b\n".to_string(), "> -|-\n> x|y\n\nend\n".to_string()),
        (
            format!("> lead\n> {}", "x".repeat(1100)),
            "|中\r\n> -|-\r\n> x|y\r\n".to_string(),
        ),
        (
            "> a|b\n".to_string(),
            "> not a delimiter\n>\n> end\n".to_string(),
        ),
        (
            "- [x] 中\n".to_string(),
            "  lazy\nno prefix\n- next\n".to_string(),
        ),
        ("- > one\n".to_string(), "\t中\r\n\n# outside".to_string()),
        ("> text\n".to_string(), "> more\r".to_string()),
    ];
    let shape = |c: FoldCandidate| (c.parent, c.source, c.hidden, c.summary, c.open);
    for (initial, tail) in cases {
        for bytes in [0, 3] {
            let mut baseline = Document::new();
            baseline.append(&initial).unwrap();
            let mut expected = FoldView::new(&baseline, 1).unwrap();
            let bid = register_container(&baseline, &mut expected, BlockId(0), bytes);
            expected.set_collapsed(&baseline, bid, true).unwrap();
            let mut update = baseline.append(&tail).unwrap();
            update.merge(baseline.finish()).unwrap();
            expected.apply(&baseline, &update).unwrap();
            let candidate = expected.fold_candidate(&baseline, bid).unwrap().map(shape);
            let layout = render(&baseline, &mut expected, TileId(0), 9, true);
            for split in (0..=tail.len()).filter(|&i| tail.is_char_boundary(i)) {
                for merged in [false, true] {
                    let mut doc = Document::new();
                    doc.append(&initial).unwrap();
                    let mut view = FoldView::new(&doc, 1).unwrap();
                    let id = register_container(&doc, &mut view, BlockId(0), bytes);
                    view.set_collapsed(&doc, id, true).unwrap();
                    let mut update = doc.append(&tail[..split]).unwrap();
                    if !merged {
                        view.apply(&doc, &update).unwrap();
                    }
                    let next = doc.append(&tail[split..]).unwrap();
                    if merged {
                        update.merge(next).unwrap();
                    } else {
                        update = next;
                    }
                    update.merge(doc.finish()).unwrap();
                    view.apply(&doc, &update).unwrap();
                    view.apply(&doc, &update).unwrap();
                    assert_eq!(doc.blocks(), baseline.blocks());
                    assert_eq!(view.fold_candidate(&doc, id).unwrap().map(shape), candidate);
                    let sink = render(&doc, &mut view, TileId(0), 9, true);
                    assert_eq!(sink.text, layout.text);
                    assert_eq!(sink.positions, layout.positions);
                    assert_eq!(sink.ends, layout.ends);
                }
            }
            let mut doc = Document::new();
            doc.append(&initial).unwrap();
            let mut view = FoldView::new(&doc, 1).unwrap();
            let id = register_container(&doc, &mut view, BlockId(0), bytes);
            view.set_collapsed(&doc, id, true).unwrap();
            for ch in tail.chars() {
                let update = doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
                view.apply(&doc, &update).unwrap();
                let mut oracle = Document::new();
                oracle.append(&initial).unwrap();
                let mut ov = FoldView::new(&oracle, 1).unwrap();
                let oid = register_container(&oracle, &mut ov, BlockId(0), bytes);
                let update = oracle.append(&doc.source()[initial.len()..]).unwrap();
                ov.apply(&oracle, &update).unwrap();
                assert_eq!(
                    view.fold_candidate(&doc, id).unwrap().map(shape),
                    ov.fold_candidate(&oracle, oid).unwrap().map(shape)
                );
                verify_model(
                    &doc,
                    &view,
                    &view
                        .fold_candidate(&doc, id)
                        .unwrap()
                        .into_iter()
                        .collect::<Vec<_>>(),
                );
            }
        }
    }
}

#[test]
fn container_paragraph_layout_matches_independent_visible_scalar_model() {
    for length in [7, 255, 256, 257, 511, 512, 513, 1024] {
        for width in [1, 3, 7, 17] {
            for bytes in [0, 3, length + 4] {
                for tail in [">\n> next\n", "\n# outside\n", ""] {
                    let mut doc = Document::new();
                    doc.append(&format!("> {}\n> second line\n", "x".repeat(length)))
                        .unwrap();
                    let mut view = FoldView::new(&doc, 1).unwrap();
                    let id = register_container(&doc, &mut view, BlockId(0), bytes);
                    view.set_collapsed(&doc, id, true).unwrap();
                    let mut update = doc.append(tail).unwrap();
                    update.merge(doc.finish()).unwrap();
                    view.apply(&doc, &update).unwrap();
                    let c = view.fold_candidate(&doc, id).unwrap().unwrap();
                    let mut oracle = Sink {
                        width,
                        ..Default::default()
                    };
                    for span in [
                        Span::new(0, c.hidden.start),
                        Span::new(c.hidden.end, doc.source().len()),
                    ] {
                        if span.start == c.hidden.end {
                            oracle.scalar('~');
                            oracle.scalar('\n');
                        }
                        for run in doc.runs_in(span).unwrap() {
                            if !run.style.contains(Style::HIDDEN) {
                                for ch in run.text.chars() {
                                    oracle.scalar(ch);
                                }
                            }
                        }
                    }
                    let sink = render(&doc, &mut view, TileId(0), width, true);
                    assert_eq!(sink.text, oracle.text);
                    assert_eq!(sink.total, oracle.total);
                    assert_eq!(sink.ends, vec![BlockId(0)]);
                    assert_eq!(view.heights().total_rows(), u128::from(sink.total));
                    view.reset(1).unwrap();
                    let large = render(&doc, &mut view, TileId(0), width, false);
                    assert_eq!(sink.positions, large.positions);
                    assert_eq!(sink.text, large.text);
                }
            }
        }
    }
}

#[test]
fn container_paragraph_streaming_visibility_and_height_model() {
    let mut doc = Document::new();
    doc.append("> # Root\n").unwrap();
    let mut view = FoldView::new(&doc, 3).unwrap();
    let mut ids = Vec::new();
    let mut seed = 123u64;
    for turn in 0..240 {
        let start = doc.source().len();
        let update = doc.append(&format!("> line {turn}\n")).unwrap();
        view.apply(&doc, &update).unwrap();
        let block = doc.block_at(start).unwrap();
        let id = register_container(&doc, &mut view, block, 3);
        let continuation = format!("> {}\n", "x".repeat(turn % 7 * 97 + 1));
        let update = doc.append(&continuation).unwrap();
        view.apply(&doc, &update).unwrap();
        assert_eq!(
            view.fold_candidate(&doc, id).unwrap().unwrap().source,
            Span::new(start, doc.source().len())
        );
        let end = doc.source().len();
        let setext = turn % 3 == 0;
        let update = doc
            .append(if setext { "> ===\n>\n" } else { ">\n" })
            .unwrap();
        view.apply(&doc, &update).unwrap();
        if setext {
            assert_eq!(view.fold_candidate(&doc, id).unwrap(), None);
        } else {
            let c = view.fold_candidate(&doc, id).unwrap().unwrap();
            assert_eq!(c.source, Span::new(start, end));
            assert_eq!(c.hidden.start, start + 3);
            assert!(!c.open);
            ids.push(c);
        }
        let mut expected = ids.clone();
        for i in 0..doc.container_count() {
            expected.push(
                doc.fold_candidate(FoldId::Container(ContainerId(i)))
                    .unwrap(),
            );
        }
        for i in 0..doc.headings().entries().len() {
            expected.push(doc.fold_candidate(FoldId::Heading(HeadingId(i))).unwrap());
        }
        for _ in 0..4 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let target = expected[(seed >> 32) as usize % expected.len()].id;
            view.toggle(&doc, target).unwrap();
            if seed.is_multiple_of(7) {
                view.set_preview(&doc, Some(target), &mut one()).unwrap();
            }
            if seed.is_multiple_of(13) {
                view.reset(2).unwrap();
            }
            verify_model(&doc, &view, &expected);
        }
    }
}

#[test]
fn container_paragraph_long_late_headers_and_delimiters_preserve_only_prose() {
    for success in [false, true] {
        for first_header in [false, true] {
            let mut doc = Document::new();
            let lead = if first_header { "" } else { "> lead\n" };
            doc.append(&format!("{lead}> {}", "x".repeat(1100)))
                .unwrap();
            let mut view = FoldView::new(&doc, 1).unwrap();
            let id = register_container(&doc, &mut view, BlockId(0), 3);
            view.set_collapsed(&doc, id, true).unwrap();
            let update = doc.append("|other\n").unwrap();
            view.apply(&doc, &update).unwrap();
            let update = doc
                .append(&format!("> {}|{}", "-".repeat(2048), "-".repeat(2048)))
                .unwrap();
            view.apply(&doc, &update).unwrap();
            assert!(view.fold_candidate(&doc, id).unwrap().unwrap().open);
            assert_eq!(doc.work().paragraph_closures, 0);
            let update = doc.append(if success { "\n" } else { "x\n>\n" }).unwrap();
            view.apply(&doc, &update).unwrap();
            let c = view.fold_candidate(&doc, id).unwrap();
            if success && first_header {
                assert!(c.is_none());
                assert_eq!(doc.work().paragraph_closures, 0);
            } else {
                let c = c.unwrap();
                assert!(!c.open);
                assert_eq!(
                    c.source.end,
                    if success {
                        lead.len()
                    } else {
                        doc.source().len() - 2
                    }
                );
                assert_eq!(doc.work().paragraph_closures, 1);
                verify_model(&doc, &view, &[c]);
            }
        }
    }
}

#[test]
fn container_paragraph_index_is_chunk_invariant_from_first_leaf_to_eof() {
    fn ranges(doc: &Document) -> Vec<(BlockId, Span, bool)> {
        let mut view = FoldView::new(doc, 1).unwrap();
        let mut out = Vec::new();
        for i in 0..doc.blocks().len() {
            if let Ok(mut task) =
                SourceFoldRegistration::container_paragraph(doc, &view, BlockId(i), 0)
            {
                let id = finish(&mut task, doc, &mut view);
                let c = view.fold_candidate(doc, id).unwrap().unwrap();
                out.push((BlockId(i), c.source, c.open));
            }
        }
        out
    }
    for source in [
        "> 中\r\n> 🙂\r\n>\r\n- one\n  two\n- next\n\nroot\n",
        "> one\n> two\n> ===\n> body\n>\n> third\n",
        "> lead\n> a|b\n> -|-\n> x|y\n\n> after\n",
        "- > *em\n\t中*\nno prefix\n\nnext\n",
        "> [ref\n> label]\n> `code\n> end` $math\n> end$\n>\n[ref label]: /late\n",
    ] {
        let baseline = parse(source);
        let expected = ranges(&baseline);
        for split in (0..=source.len()).filter(|&i| source.is_char_boundary(i)) {
            let mut doc = Document::new();
            doc.append(&source[..split]).unwrap();
            doc.append(&source[split..]).unwrap();
            doc.finish();
            assert_eq!(ranges(&doc), expected);
            assert_eq!(
                doc.work().paragraph_closures,
                baseline.work().paragraph_closures
            );
        }
        let mut doc = Document::new();
        for ch in source.chars() {
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            let mut whole = Document::new();
            whole.append(doc.source()).unwrap();
            assert_eq!(ranges(&doc), ranges(&whole));
        }
    }
}

#[test]
fn container_paragraph_nested_list_registration_charges_both_ancestor_paths() {
    let mut doc = Document::new();
    for depth in 0..250 {
        doc.append(&format!("{}- item {depth}\n", "  ".repeat(depth)))
            .unwrap();
    }
    assert_eq!(doc.work().paragraph_closures, 249);
    let block = BlockId(249);
    let mut view = FoldView::new(&doc, 1).unwrap();
    let mut task = SourceFoldRegistration::container_paragraph(&doc, &view, block, 501).unwrap();
    let id = loop {
        let p = task.advance(&doc, &mut view, &mut one()).unwrap();
        if let Some(id) = p.id {
            assert_eq!(p.visited, 1000); // 250 lists + 250 items, twice.
            break id;
        }
        assert_eq!(view.source_fold_count(), 0);
    };
    let before = doc.work();
    let update = doc.append("lazy continuation\n").unwrap();
    view.apply(&doc, &update).unwrap();
    assert_eq!(doc.work().paragraph_closures, before.paragraph_closures);
    assert!(view.fold_candidate(&doc, id).unwrap().unwrap().open);
    let update = doc.finish();
    view.apply(&doc, &update).unwrap();
    assert_eq!(doc.work().paragraph_closures, 250);
    let c = view.fold_candidate(&doc, id).unwrap().unwrap();
    assert_eq!(c.source.end, doc.source().len());
    assert!(!c.open);
}

fn auto_policy(doc: &Document, view: &mut FoldView, update: &Update) {
    let mut task = ParagraphAutoFold::for_update(doc, view, update).unwrap();
    for _ in 0..100_000 {
        if task.advance(doc, view, &mut one()).unwrap().completion == Completion::Covered {
            return;
        }
    }
    panic!("automatic policy stalled");
}

#[test]
fn paragraph_auto_streaming_choices_match_interval_and_height_model() {
    let mut doc = Document::new();
    doc.append("> # Root\n").unwrap();
    let mut view = FoldView::new(&doc, 3).unwrap();
    view.set_defaults(FoldDefaults {
        paragraphs: Some(ParagraphFoldPolicy {
            min_bytes: 24,
            preview_bytes: 3,
        }),
        ..Default::default()
    });
    let mut expected = Vec::new();
    let mut seed = 991u64;
    for turn in 0..240 {
        let start = doc.source().len();
        let u = doc.append("> short\n").unwrap();
        view.apply(&doc, &u).unwrap();
        auto_policy(&doc, &mut view, &u);
        assert!(view.source_fold_at(&doc, start).unwrap().is_none());
        let u = doc
            .append(&format!("> {}\n", "x".repeat(24 + turn % 7 * 127)))
            .unwrap();
        view.apply(&doc, &u).unwrap();
        auto_policy(&doc, &mut view, &u);
        let c = view.source_fold_at(&doc, start).unwrap().unwrap();
        assert_eq!(c.source, Span::new(start, doc.source().len()));
        assert_eq!(c.hidden.start, start + 3);
        assert!(view.is_collapsed(c.id));
        let end = doc.source().len();
        let promoted = turn % 3 == 0;
        let u = doc
            .append(if promoted { "> ===\n>\n" } else { ">\n" })
            .unwrap();
        view.apply(&doc, &u).unwrap();
        auto_policy(&doc, &mut view, &u);
        if promoted {
            assert!(view.fold_candidate(&doc, c.id).unwrap().is_none());
        } else {
            let c = view.fold_candidate(&doc, c.id).unwrap().unwrap();
            assert_eq!(c.source, Span::new(start, end));
            assert!(!c.open);
            expected.push(c);
        }
        assert_eq!(view.source_fold_count(), expected.len());
        let mut candidates = expected.clone();
        candidates.push(
            doc.fold_candidate(FoldId::Container(ContainerId(0)))
                .unwrap(),
        );
        for i in 0..doc.headings().entries().len() {
            candidates.push(doc.fold_candidate(FoldId::Heading(HeadingId(i))).unwrap());
        }
        for _ in 0..4 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let target = candidates[(seed >> 32) as usize % candidates.len()].id;
            view.toggle(&doc, target).unwrap();
            if seed.is_multiple_of(7) {
                view.set_preview(&doc, Some(target), &mut one()).unwrap();
            }
            if seed.is_multiple_of(13) {
                view.reset(2).unwrap();
            }
            verify_model(&doc, &view, &candidates);
        }
    }
}

#[test]
fn paragraph_auto_layout_is_budgeted_and_invalidates_old_measurements_and_passes() {
    for prefix in ["", "> ", "- "] {
        for bytes in [0, 3, 5, 8, 17, 31] {
            let source = format!("{prefix}中🙂 *bold* {}\n\nnext\n", "x".repeat(4096));
            let doc = parse(&source);
            let mut automatic = FoldView::new(&doc, 1).unwrap();
            let mut manual = FoldView::new(&doc, 1).unwrap();
            automatic.set_defaults(FoldDefaults {
                paragraphs: Some(ParagraphFoldPolicy {
                    min_bytes: 100,
                    preview_bytes: bytes,
                }),
                ..Default::default()
            });
            let ticket = automatic.heights().ticket(TileId(0)).unwrap();
            automatic.insert_checkpoint(TileId(0), ()).unwrap();
            let mut stale = LayoutPass::folded(&doc, &automatic, TileId(0), 100).unwrap();
            let mut task =
                ParagraphAutoFold::new(&doc, &automatic, Span::new(0, source.len())).unwrap();
            loop {
                if task
                    .advance(&doc, &mut automatic, &mut one())
                    .unwrap()
                    .completion
                    == Completion::Covered
                {
                    break;
                }
            }
            assert_eq!(automatic.source_fold_count(), 1);
            assert!(automatic.measure(ticket, 9).is_err());
            assert!(automatic.checkpoints().token(TileId(0)).is_none());
            assert_eq!(
                stale.advance_folded(&doc, &mut automatic, &mut one(), &mut Sink::default()),
                Err(LayoutError::StaleContinuation)
            );
            let id = if prefix.is_empty() {
                register(&doc, &mut manual, BlockId(0), bytes)
            } else {
                register_container(&doc, &mut manual, BlockId(0), bytes)
            };
            manual.set_collapsed(&doc, id, true).unwrap();
            for width in [1, 7, 19, 80] {
                automatic.reset(1).unwrap();
                manual.reset(1).unwrap();
                let got = render(&doc, &mut automatic, TileId(0), width, true);
                let want = render(&doc, &mut manual, TileId(0), width, false);
                assert_eq!(got.text, want.text);
                assert_eq!(got.positions, want.positions);
                assert_eq!(got.ends, want.ends);
                assert_eq!(got.total, want.total);
                assert_eq!(automatic.heights().total_rows(), u128::from(got.total));
                assert!(got.bytes < 80);
                assert_eq!(got.objects, 0);
            }
        }
    }
}
