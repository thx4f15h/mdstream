use mdtui3::*;

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
