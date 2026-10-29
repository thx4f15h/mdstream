use mdtui3::*;

#[derive(Default)]
struct Sink {
    text: String,
    placeholders: Vec<FoldPlaceholder>,
    images: usize,
    bytes: usize,
    tiles: usize,
    rows: u64,
}
impl LayoutSink for Sink {
    fn begin_tile(&mut self, _: TileId) {
        self.rows = 0;
        self.tiles += 1;
    }
    fn text(&mut self, run: RenderRun<'_>) {
        self.bytes += run.text.len();
        if !run.style.contains(Style::HIDDEN) {
            self.text.push_str(run.text);
            self.rows += run.text.bytes().filter(|&b| b == b'\n').count() as u64;
        }
    }
    fn object(&mut self, _: LinkId, _: &LinkInfo) {
        self.images += 1;
    }
    fn end_tile(&mut self, _: TileId) -> u64 {
        self.rows
    }
}
impl FoldLayoutSink for Sink {
    fn placeholder(&mut self, placeholder: FoldPlaceholder) {
        self.placeholders.push(placeholder);
        self.text.push_str("[fold]\n");
        self.rows += 1;
    }
}
fn render(doc: &Document, view: &mut FoldView, tiny: bool) -> Sink {
    let mut pass = LayoutPass::folded(doc, view, TileId(0), u64::MAX).unwrap();
    let mut sink = Sink::default();
    for _ in 0..50000 {
        let mut budget = Budget::new(if tiny {
            WorkLimit {
                queries: 1,
                bytes: 4,
                tiles: 1,
                objects: 1,
                ..Default::default()
            }
        } else {
            WorkLimit::unlimited()
        });
        if pass
            .advance_folded(doc, view, &mut budget, &mut sink)
            .unwrap()
            == Completion::EndOfDocument
        {
            return sink;
        }
    }
    panic!("no progress at {}", pass.source_position());
}
fn document(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}
fn heading(id: usize) -> FoldId {
    FoldId::Heading(HeadingId(id))
}

#[test]
fn same_tile_boundaries_empty_sections_nested_choices_and_independent_views() {
    let doc = document("# A\na\n## B\nb\n### C\nc\n## D\nd\n# E\n# F\nf\n");
    assert_eq!(doc.tile_count(), 1);
    let mut view = FoldView::new(&doc, 1).unwrap();
    let mut other = FoldView::new(&doc, 1).unwrap();
    view.set_collapsed(&doc, heading(1), true).unwrap();
    assert_eq!(
        render(&doc, &mut view, true).text,
        "A\na\nB\n[fold]\nD\nd\nE\nF\nf\n"
    );
    view.set_collapsed(&doc, heading(0), true).unwrap();
    let sink = render(&doc, &mut view, true);
    assert_eq!(sink.text, "A\n[fold]\nE\nF\nf\n");
    assert_eq!(sink.placeholders.len(), 1);
    view.set_collapsed(&doc, heading(4), true).unwrap(); // Empty E does not swallow F.
    view.set_collapsed(&doc, heading(0), false).unwrap();
    assert!(view.is_collapsed(heading(1)));
    let sink = render(&doc, &mut view, true);
    assert_eq!(sink.text, "A\na\nB\n[fold]\nD\nd\nE\nF\nf\n");
    assert_eq!(view.heights().total_rows(), 9);
    assert_eq!(
        render(&doc, &mut other, true).text,
        "A\na\nB\nb\nC\nc\nD\nd\nE\nF\nf\n"
    );
}

#[test]
fn code_placeholders_own_height_and_hidden_images_are_skipped() {
    let doc = document(
        "```rust\nlet x = 1;\n```\n```\n```\n    indented\n\n![](/visible)\n# A\n![](/hidden)\n# B\nafter\n",
    );
    let mut view = FoldView::new(&doc, 2).unwrap();
    let ids: Vec<_> = doc
        .blocks()
        .iter()
        .enumerate()
        .filter(|(_, b)| matches!(b.kind, BlockKind::Code { .. }))
        .map(|(i, _)| FoldId::Code(BlockId(i)))
        .collect();
    assert_eq!(ids.len(), 3);
    for &id in &ids {
        view.set_collapsed(&doc, id, true).unwrap();
    }
    view.set_collapsed(&doc, heading(0), true).unwrap();
    let sink = render(&doc, &mut view, true);
    assert_eq!(sink.placeholders.len(), 4);
    assert_eq!(sink.images, 1);
    assert!(!sink.text.contains("let x"));
    assert!(!sink.text.contains("indented"));
    assert!(sink.text.ends_with("B\nafter\n"));
    for p in sink.placeholders {
        assert_eq!(p.owner, doc.tile_at(p.candidate.hidden.start).unwrap());
    }
    let c = doc.fold_candidate(ids[0]).unwrap();
    let mut budget = Budget::new(WorkLimit {
        queries: 1,
        bytes: 2,
        objects: 1,
        ..Default::default()
    });
    assert_eq!(c.summary_text(&doc, 2, &mut budget), Some("ru"));
    assert_eq!(c.summary_text(&doc, 2, &mut budget), None);
}

#[test]
fn million_byte_fold_is_skipped_and_append_only_updates_open_ranges() {
    let mut doc = Document::new();
    doc.append("# parent\n## child\n").unwrap();
    doc.append(&"x".repeat(1_000_000)).unwrap();
    let mut view = FoldView::new(&doc, 3).unwrap();
    view.set_collapsed(&doc, heading(1), true).unwrap();
    view.set_collapsed(&doc, heading(0), true).unwrap();
    let visits = view.heights().mutation_visits();
    let parser_work = doc.work();
    let update = doc.append("more\n# after\nvisible\n").unwrap();
    view.apply(&doc, &update).unwrap();
    assert!(doc.work().input_bytes - parser_work.input_bytes < 30);
    assert!(view.heights().mutation_visits() - visits < 1500);
    assert_eq!(view.range_visits(), 2);
    let mut pass = LayoutPass::folded(&doc, &view, TileId(0), 100).unwrap();
    let mut sink = Sink::default();
    let mut budget = Budget::new(WorkLimit {
        queries: 60,
        tiles: 4,
        bytes: 512,
        objects: 30,
        ..Default::default()
    });
    assert_eq!(
        pass.advance_folded(&doc, &mut view, &mut budget, &mut sink)
            .unwrap(),
        Completion::EndOfDocument
    );
    assert_eq!(sink.text, "parent\n[fold]\nafter\nvisible\n");
    assert!(sink.bytes < 512);
    assert!(sink.tiles <= 2);
    view.reset(7).unwrap();
    assert_eq!(view.heights().height(TileId(100)), Some(0));
    assert_eq!(view.heights().base_height(TileId(100)), Some(7));
    view.set_collapsed(&doc, heading(0), false).unwrap();
    assert_eq!(view.heights().height(TileId(100)), Some(0));
    view.set_collapsed(&doc, heading(1), false).unwrap();
    assert_eq!(view.heights().height(TileId(100)), Some(7));
    let mut pass = LayoutPass::folded(&doc, &view, TileId(0), 24).unwrap();
    let mut budget = Budget::new(WorkLimit {
        queries: 20,
        tiles: 4,
        bytes: 100,
        objects: 20,
        ..Default::default()
    });
    let mut sink = Sink::default();
    assert_eq!(
        pass.advance_folded(&doc, &mut view, &mut budget, &mut sink)
            .unwrap(),
        Completion::BudgetExhausted
    );
    assert!(sink.bytes <= 100);
}

#[test]
fn streaming_code_pending_tail_closure_and_setext_retract_old_masks() {
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let update = doc.append("# A\n```rs\n").unwrap();
    view.apply(&doc, &update).unwrap();
    let code = FoldId::Code(BlockId(1));
    view.set_collapsed(&doc, code, true).unwrap();
    for text in ["hello", " world\n", "```\n", "visible\n", "---\n", "body\n"] {
        let update = doc.append(text).unwrap();
        view.apply(&doc, &update).unwrap();
        let sink = render(&doc, &mut view, true);
        assert_eq!(sink.placeholders.len(), 1);
        assert!(!sink.text.contains("hello"));
        if doc.source().contains("visible\n") {
            assert!(sink.text.contains("visible"));
        }
    }
    assert_eq!(doc.headings().entries().len(), 2);
    view.set_collapsed(&doc, heading(1), true).unwrap();
    let update = doc.finish();
    view.apply(&doc, &update).unwrap();
    let sink = render(&doc, &mut view, true);
    assert_eq!(sink.text, "A\n[fold]\nvisible\n[fold]\n");
    assert!(!doc.fold_candidate(code).unwrap().open);
    assert!(!doc.fold_candidate(heading(1)).unwrap().open);
}

#[test]
fn late_setext_can_shorten_a_collapsed_section_across_many_tiles() {
    let mut doc = Document::new();
    doc.append("## old\nbody\n\n").unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    view.set_collapsed(&doc, heading(0), true).unwrap();
    let title = "t".repeat(5000);
    let update = doc.append(&format!("{title}\n")).unwrap();
    view.apply(&doc, &update).unwrap();
    assert_eq!(view.heights().height(TileId(5)), Some(0));
    let update = doc.append("===\ntext\n").unwrap();
    view.apply(&doc, &update).unwrap();
    assert!(view.heights().height(TileId(5)).unwrap() > 0);
    let sink = render(&doc, &mut view, true);
    assert_eq!(sink.text, format!("old\n[fold]\n{title}\ntext\n"));
}

#[test]
fn fold_changes_reject_old_tickets_passes_and_checkpoint_dependencies() {
    let doc = document(&format!("# A\n{}\n# B\nlast\n", "line\n".repeat(200)));
    let mut view = FoldView::<usize>::with_state(&doc, 1).unwrap();
    view.insert_checkpoint(TileId(0), 7).unwrap();
    view.insert_checkpoint(TileId(2), 9).unwrap();
    let ticket = view
        .heights()
        .ticket_from(TileId(3), view.checkpoints(), TileId(2))
        .unwrap();
    let mut pass = LayoutPass::folded(&doc, &view, TileId(0), 24).unwrap();
    let mut budget = Budget::new(WorkLimit::unlimited());
    let mut tiles = Vec::new();
    let cursor = view
        .heights()
        .viewport_budgeted(
            0..100,
            0,
            &mut Budget::new(WorkLimit::default()),
            &mut tiles,
        )
        .next
        .unwrap();
    view.set_collapsed(&doc, heading(0), true).unwrap();
    assert_eq!(view.measure(ticket, 99), Err(LayoutError::StaleMeasurement));
    assert!(view.checkpoints().previous(TileId(3)).is_none());
    assert_eq!(
        pass.advance_folded(&doc, &mut view, &mut budget, &mut Sink::default()),
        Err(LayoutError::StaleContinuation)
    );
    assert!(matches!(
        view.heights()
            .resume_viewport(cursor, &mut budget, &mut tiles),
        Err(LayoutError::StaleContinuation)
    ));
    let version = view.version();
    assert!(
        view.set_collapsed(&doc, heading(0), true)
            .unwrap()
            .is_none()
    );
    assert_eq!(view.version(), version);
    let other = document("# other\n");
    assert_eq!(
        view.toggle(&other, heading(0)),
        Err(LayoutError::WrongDocument)
    );

    let doc = document("# Empty\n");
    let mut view = FoldView::new(&doc, 1).unwrap();
    let ticket = view.heights().ticket(TileId(0)).unwrap();
    let mut pass = LayoutPass::folded(&doc, &view, TileId(0), 1).unwrap();
    view.toggle(&doc, heading(0)).unwrap();
    assert_eq!(view.measure(ticket, 99), Err(LayoutError::StaleMeasurement));
    assert_eq!(
        pass.advance_folded(&doc, &mut view, &mut budget, &mut Sink::default()),
        Err(LayoutError::StaleContinuation)
    );
}

#[test]
fn update_replay_batching_and_wrong_document_are_atomic() {
    let mut doc = Document::new();
    doc.append("# A\n").unwrap();
    let mut one = FoldView::new(&doc, 1).unwrap();
    let mut batch = FoldView::new(&doc, 1).unwrap();
    for view in [&mut one, &mut batch] {
        view.set_collapsed(&doc, heading(0), true).unwrap();
    }
    let mut merged = Update::default();
    for text in ["body\n", "## child\n", "inner\n", "# next\n", "visible\n"] {
        let update = doc.append(text).unwrap();
        one.apply(&doc, &update).unwrap();
        merged.merge(update).unwrap();
    }
    batch.apply(&doc, &merged).unwrap();
    assert_eq!(
        render(&doc, &mut one, true).text,
        render(&doc, &mut batch, true).text
    );
    let height = batch.heights().total_rows();
    let version = batch.version();
    batch.apply(&doc, &merged).unwrap();
    assert_eq!(batch.heights().total_rows(), height);
    assert_eq!(batch.version(), version);
    let mut wrong = Document::new();
    let update = wrong.append("wrong").unwrap();
    assert_eq!(
        batch.apply(&wrong, &update),
        Err(LayoutError::WrongDocument)
    );
    assert_eq!(batch.heights().total_rows(), height);
}

fn oracle(doc: &Document, ids: &[FoldId]) -> String {
    // Independent union of selected source intervals, with outer ranges taking
    // precedence. No FoldState, height tree, or viewport query is used here.
    let mut ranges: Vec<_> = ids
        .iter()
        .map(|&id| match id {
            FoldId::Heading(id) => {
                let h = doc.headings().get(id).unwrap();
                let end = doc.headings().entries()[id.0 + 1..]
                    .iter()
                    .find(|next| next.level <= h.level)
                    .map_or(doc.source().len(), |next| next.source.start);
                Span::new(h.source.end, end)
            }
            FoldId::Code(id) => {
                let b = doc.block(id).unwrap();
                Span::new(
                    b.source.start,
                    if b.state == SyntaxState::Sealed {
                        b.source.end
                    } else {
                        doc.source().len()
                    },
                )
            }
        })
        .filter(|r| !r.is_empty())
        .collect();
    ranges.sort_by_key(|r| (r.start, std::cmp::Reverse(r.end)));
    let mut output = String::new();
    let mut cursor = 0;
    for range in ranges {
        if range.start < cursor {
            continue;
        }
        for run in doc.runs_in(Span::new(cursor, range.start)).unwrap() {
            if !run.style.contains(Style::HIDDEN) {
                output.push_str(run.text);
            }
        }
        output.push_str("[fold]\n");
        cursor = range.end;
    }
    for run in doc.runs_in(Span::new(cursor, doc.source().len())).unwrap() {
        if !run.style.contains(Style::HIDDEN) {
            output.push_str(run.text);
        }
    }
    output
}

#[test]
fn seeded_nested_toggles_and_resize_match_source_union() {
    let mut source = String::new();
    for i in 0..32 {
        source.push_str(&format!(
            "{} H{i}\n{}\n```rs\ncode {i}\n```\n",
            "#".repeat(1 + (i * 7 % 6)),
            "中文🙂\n".repeat(15 + i % 8)
        ));
    }
    let doc = document(&source);
    let mut view = FoldView::new(&doc, 1).unwrap();
    let ids: Vec<_> = doc
        .headings()
        .entries()
        .iter()
        .enumerate()
        .map(|(i, _)| heading(i))
        .chain(
            doc.blocks()
                .iter()
                .enumerate()
                .filter(|(_, b)| matches!(b.kind, BlockKind::Code { .. }))
                .map(|(i, _)| FoldId::Code(BlockId(i))),
        )
        .collect();
    let mut selected = Vec::new();
    let mut seed = 0x90f1_8ab7_u64;
    for step in 0..160 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let id = ids[(seed >> 32) as usize % ids.len()];
        if let Some(at) = selected.iter().position(|&value| value == id) {
            selected.swap_remove(at);
        } else {
            selected.push(id);
        }
        view.toggle(&doc, id).unwrap();
        if step % 7 == 0 {
            view.reset(2 + step % 5).unwrap();
        }
        let sink = render(&doc, &mut view, false);
        let expected = oracle(&doc, &selected);
        assert_eq!(sink.text, expected, "step {step}");
        assert_eq!(
            view.heights().total_rows(),
            expected.bytes().filter(|&b| b == b'\n').count() as u128,
            "step {step}"
        );
    }
    assert!(view.heights().node_count() < doc.tile_count() * 4);
}

#[test]
fn every_streaming_prefix_agrees_with_independent_source_projection() {
    let input = format!(
        "# 大标题\n## 子标题\n{}\n```rs\n正文🙂\n```\nnext\n===\nend\n",
        "字🙂".repeat(240)
    );
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let mut selected = Vec::new();
    let mut seed = 42_u64;
    for ch in input.chars() {
        let update = doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
        view.apply(&doc, &update).unwrap();
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        if !doc.headings().entries().is_empty() && seed.is_multiple_of(11) {
            let id = heading((seed >> 32) as usize % doc.headings().entries().len());
            view.toggle(&doc, id).unwrap();
            if let Some(at) = selected.iter().position(|&old| old == id) {
                selected.swap_remove(at);
            } else {
                selected.push(id);
            }
        }
        if let Some((index, _)) = doc
            .blocks()
            .iter()
            .enumerate()
            .find(|(_, b)| matches!(b.kind, BlockKind::Code { .. }))
        {
            let id = FoldId::Code(BlockId(index));
            if !selected.contains(&id) {
                view.set_collapsed(&doc, id, true).unwrap();
                selected.push(id);
            }
        }
        assert_eq!(
            render(&doc, &mut view, false).text,
            oracle(&doc, &selected),
            "prefix {}",
            doc.source().len()
        );
    }
    let update = doc.finish();
    view.apply(&doc, &update).unwrap();
    assert_eq!(render(&doc, &mut view, true).text, oracle(&doc, &selected));
}

#[test]
fn bounded_discovery_lookup_and_stale_cursors() {
    let mut doc = document("# A\nbody\n```rs\ncode\n```\n## B\nbody\n");
    let mut cursor = doc
        .fold_candidates(Span::new(0, doc.source().len()))
        .unwrap();
    let mut out = Vec::new();
    let mut ids = Vec::new();
    loop {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        let done = cursor.advance(&doc, &mut budget, &mut out).unwrap();
        ids.extend(out.iter().map(|c| c.id));
        assert!(out.len() <= 1);
        if done == Completion::Covered {
            break;
        }
    }
    assert_eq!(ids, [heading(0), FoldId::Code(BlockId(2)), heading(1)]);
    let code = doc.fold_candidate(ids[1]).unwrap();
    assert_eq!(code.parent, Some(heading(0)));
    assert_eq!(doc.fold_candidate_at(code.source.start).unwrap().id, ids[1]);
    let mut view = FoldView::new(&doc, 1).unwrap();
    view.toggle(&doc, heading(0)).unwrap();
    assert_eq!(
        view.hidden_at(
            &doc,
            code.source.start,
            &mut Budget::new(WorkLimit::default())
        )
        .unwrap(),
        FoldLookup::BudgetExhausted
    );
    assert!(
        matches!(view.hidden_at(&doc, code.source.start, &mut Budget::new(WorkLimit::unlimited())).unwrap(), FoldLookup::Hidden(c) if c.id == heading(0))
    );
    let mut other = Document::new();
    other.append("# next\n").unwrap();
    assert_eq!(
        cursor.advance(&other, &mut Budget::new(WorkLimit::unlimited()), &mut out),
        Err(LayoutError::WrongDocument)
    );
    doc = Document::new();
    doc.append("# live\n").unwrap();
    let mut cursor = doc
        .fold_candidates(Span::new(0, doc.source().len()))
        .unwrap();
    doc.append("body").unwrap();
    assert_eq!(
        cursor.advance(&doc, &mut Budget::new(WorkLimit::unlimited()), &mut out),
        Err(LayoutError::StaleContinuation)
    );
}

#[test]
fn placeholders_revive_syntax_zero_tiles_and_recovery_is_bounded() {
    let doc = document("```\n```\n");
    let mut view = FoldView::new(&doc, 1).unwrap();
    assert!(render(&doc, &mut view, true).text.is_empty());
    assert_eq!(view.heights().total_rows(), 0);
    view.toggle(&doc, FoldId::Code(BlockId(0))).unwrap();
    assert_eq!(render(&doc, &mut view, true).text, "[fold]\n");
    assert_eq!(view.heights().total_rows(), 1);
    view.toggle(&doc, FoldId::Code(BlockId(0))).unwrap();
    assert!(render(&doc, &mut view, true).text.is_empty());
    assert_eq!(view.heights().total_rows(), 0);

    let doc = document(&format!("# A\n{}\n# B\nend\n", "x\n".repeat(1000)));
    let mut view = FoldView::<usize>::with_state(&doc, 1).unwrap();
    assert!(
        LayoutPass::folded_from_checkpoint(&doc, &view, TileId(0), TileId(2), 1, 512)
            .unwrap()
            .is_none()
    );
    view.insert_checkpoint(TileId(0), 0).unwrap();
    assert!(
        LayoutPass::folded_from_checkpoint(&doc, &view, TileId(0), TileId(3), 1, 512)
            .unwrap()
            .is_none()
    );
    let mut pass = LayoutPass::folded_from_checkpoint(&doc, &view, TileId(0), TileId(2), 1, 512)
        .unwrap()
        .unwrap();
    assert!(!pass.is_approximate());
    let mut sink = Sink::default();
    for _ in 0..1000 {
        let mut budget = Budget::new(WorkLimit {
            queries: 8,
            tiles: 2,
            bytes: 16,
            objects: 4,
            recovery: 1,
            ..Default::default()
        });
        let done = pass
            .advance_folded(&doc, &mut view, &mut budget, &mut sink)
            .unwrap();
        assert!(budget.used().recovery <= 1);
        if done == Completion::Covered {
            assert!(pass.next_tile().0 >= 3);
            return;
        }
    }
    panic!("recovery did not progress");
}
