use mdtui3::*;

fn document(text: &str) -> Document {
    let mut doc = Document::new();
    doc.append(text).unwrap();
    doc.finish();
    doc
}

fn heading(id: usize) -> FoldId {
    FoldId::Heading(HeadingId(id))
}

fn tiny() -> Budget {
    Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        ..Default::default()
    })
}

fn preview<S>(doc: &Document, view: &mut FoldView<S>, id: Option<FoldId>) -> FoldChange {
    let mut budget = tiny();
    let progress = view.set_preview(doc, id, &mut budget).unwrap();
    assert_eq!(progress.completion, Completion::Covered);
    assert_eq!(budget.used(), tiny().remaining());
    assert_eq!(view.preview(), id);
    let change = progress.change.unwrap();
    assert_eq!(change.version, view.version());
    change
}

fn hidden<S>(doc: &Document, view: &FoldView<S>, source: usize) -> Option<FoldId> {
    match view.hidden_at(doc, source, &mut tiny()).unwrap() {
        FoldLookup::Hidden(candidate) => Some(candidate.id),
        FoldLookup::Visible => None,
        FoldLookup::BudgetExhausted => panic!("lookup did not progress"),
    }
}

#[test]
fn switching_preview_preserves_local_choices_and_ancestor_visibility() {
    let doc = document("# A\nintro\n## B\nbody\n```rs\ncode\n```\n# C\ntail\n# Empty\n");
    assert_eq!(doc.tile_count(), 1);
    let body = doc.source().find("body").unwrap();
    let code_source = doc.source().find("code").unwrap();
    let code = FoldId::Code(doc.block_at(code_source).unwrap());
    let mut view = FoldView::new(&doc, 1).unwrap();
    let other = FoldView::new(&doc, 1).unwrap();
    for id in [heading(0), heading(1), code] {
        view.set_collapsed(&doc, id, true).unwrap();
    }
    preview(&doc, &mut view, Some(heading(1)));
    assert_eq!(hidden(&doc, &view, body), Some(heading(0)));
    assert!(view.is_collapsed(heading(1)));
    preview(&doc, &mut view, Some(heading(0)));
    assert_eq!(hidden(&doc, &view, body), Some(heading(1)));
    assert_eq!(
        hidden(&doc, &view, doc.source().find("intro").unwrap()),
        None
    );
    view.set_collapsed(&doc, heading(0), false).unwrap();
    preview(&doc, &mut view, Some(heading(1)));
    assert_eq!(hidden(&doc, &view, body), None);
    assert_eq!(hidden(&doc, &view, code_source), Some(code));
    view.set_collapsed(&doc, heading(1), false).unwrap();
    preview(&doc, &mut view, Some(code));
    assert_eq!(hidden(&doc, &view, code_source), None);
    let change = preview(&doc, &mut view, Some(heading(2)));
    assert_eq!(hidden(&doc, &view, code_source), Some(code));
    assert_eq!(
        change.source.start,
        doc.fold_candidate(code).unwrap().hidden.start
    );
    assert_eq!(
        change.source.end,
        doc.fold_candidate(heading(2)).unwrap().hidden.end
    );
    preview(&doc, &mut view, None);
    assert!(!view.is_collapsed(heading(2)));
    preview(&doc, &mut view, Some(heading(3))); // Empty EOF section.
    assert!(preview(&doc, &mut view, None).source.is_empty());
    assert_eq!(other.preview(), None);
    assert_eq!(hidden(&doc, &other, code_source), None);
}

#[test]
fn rejected_or_unfunded_changes_leave_preview_and_budget_untouched() {
    let mut doc = Document::new();
    doc.append("# A\nbody\n# B\ntail\n").unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    view.set_collapsed(&doc, heading(0), true).unwrap();
    for initial in [None, Some(heading(0))] {
        if initial.is_some() {
            preview(&doc, &mut view, initial);
        }
        let version = view.version();
        let rows = view.heights().total_rows();
        let visits = view.heights().mutation_visits();
        for target in [Some(heading(1)), None] {
            if target == initial {
                continue;
            }
            for limit in [
                WorkLimit {
                    queries: 1,
                    ..Default::default()
                },
                WorkLimit {
                    objects: 1,
                    ..Default::default()
                },
            ] {
                let mut budget = Budget::new(limit);
                assert_eq!(
                    view.set_preview(&doc, target, &mut budget).unwrap(),
                    FoldPreviewProgress {
                        completion: Completion::BudgetExhausted,
                        change: None,
                    }
                );
                assert_eq!(budget.used(), WorkLimit::default());
                assert_eq!(view.preview(), initial);
                assert_eq!(view.version(), version);
                assert_eq!(view.heights().total_rows(), rows);
                assert_eq!(view.heights().mutation_visits(), visits);
            }
        }
        let mut budget = Budget::new(WorkLimit::default());
        assert_eq!(
            view.set_preview(&doc, initial, &mut budget).unwrap(),
            FoldPreviewProgress {
                completion: Completion::Covered,
                change: None,
            }
        );
        for id in [
            heading(999),
            FoldId::Code(BlockId(999)),
            FoldId::Code(BlockId(0)),
        ] {
            assert_eq!(
                view.set_preview(&doc, Some(id), &mut budget),
                Err(LayoutError::InvalidRange)
            );
        }
        let wrong = document("# other\n");
        assert_eq!(
            view.set_preview(&wrong, None, &mut budget),
            Err(LayoutError::WrongDocument)
        );
        assert_eq!(view.preview(), initial);
        assert_eq!(view.version(), version);
        assert_eq!(budget.used(), WorkLimit::default());
    }
    let update = doc.append("more\n").unwrap();
    let mut budget = tiny();
    assert_eq!(
        view.set_preview(&doc, None, &mut budget),
        Err(LayoutError::OutOfOrderUpdate)
    );
    assert_eq!(budget.used(), WorkLimit::default());
    assert_eq!(view.preview(), Some(heading(0)));
    view.apply(&doc, &update).unwrap();
    preview(&doc, &mut view, None);
}

#[test]
fn explicit_and_batch_choices_take_effect_when_preview_ends() {
    let doc = document("# A\nintro\n## B\nbody\n");
    let body = doc.source().find("body").unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    preview(&doc, &mut view, Some(heading(1)));
    view.toggle(&doc, heading(1)).unwrap();
    assert!(view.is_collapsed(heading(1)));
    assert_eq!(hidden(&doc, &view, body), None);
    preview(&doc, &mut view, None);
    assert_eq!(hidden(&doc, &view, body), Some(heading(1)));
    preview(&doc, &mut view, Some(heading(1)));
    for collapsed in [false, true] {
        let mut batch = FoldBatch::new(
            &doc,
            &view,
            FoldBatchAction::Subtree {
                root: heading(1),
                collapsed,
            },
        )
        .unwrap();
        let result = batch
            .advance(&doc, &mut view, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap();
        assert_eq!(result.completion, Completion::Covered);
        assert_eq!(result.changed, 1);
        assert_eq!(view.is_collapsed(heading(1)), collapsed);
        assert_eq!(hidden(&doc, &view, body), None);
    }
    view.set_collapsed(&doc, heading(0), true).unwrap();
    let mut nav = FoldNavigation::new(
        &doc,
        &view,
        &Navigator::new(),
        NavAction::Source(body),
        FoldNavigationPolicy::Reveal,
        1,
    )
    .unwrap();
    let result = nav
        .advance(&doc, &mut view, &mut Budget::new(WorkLimit::unlimited()))
        .unwrap();
    assert_eq!(result.target.unwrap().source, Some(body));
    assert!(!view.is_collapsed(heading(0))); // Reveal opens only the actual blocker.
    assert!(view.is_collapsed(heading(1)));
    assert_eq!(view.preview(), Some(heading(1)));
    preview(&doc, &mut view, None);
    assert_eq!(hidden(&doc, &view, body), Some(heading(1)));
}

#[test]
fn streaming_defaults_setext_and_resize_preserve_preview_and_current_ranges() {
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 1).unwrap();
    view.set_defaults(FoldDefaults {
        heading_levels: [true; 6],
        code: true,
        ..Default::default()
    });
    let update = doc.append("# A\n").unwrap();
    view.apply(&doc, &update).unwrap();
    preview(&doc, &mut view, Some(heading(0)));
    for ch in "body\n\nLate title\n---\n```rs\ncode\n".chars() {
        let update = doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
        view.apply(&doc, &update).unwrap();
        view.apply(&doc, &update).unwrap();
        assert_eq!(view.preview(), Some(heading(0)));
        assert!(view.is_collapsed(heading(0)));
        view.reset(2).unwrap();
    }
    let title = doc.source().find("Late title").unwrap();
    assert_eq!(doc.headings().entries().len(), 2);
    assert!(view.is_collapsed(heading(1)));
    assert_eq!(hidden(&doc, &view, title), None);
    let code = FoldId::Code(doc.block_at(doc.source().find("code").unwrap()).unwrap());
    assert!(view.is_collapsed(code));
    preview(&doc, &mut view, None);
    assert_eq!(hidden(&doc, &view, title), Some(heading(0)));
    view.set_collapsed(&doc, heading(0), false).unwrap();
    view.set_collapsed(&doc, heading(1), false).unwrap();
    preview(&doc, &mut view, Some(code));
    for ch in "new code\n```\n# Next\ntail\n".chars() {
        let update = doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
        view.apply(&doc, &update).unwrap();
    }
    let update = doc.finish();
    view.apply(&doc, &update).unwrap();
    view.reset(3).unwrap();
    let source = doc.source().find("new code").unwrap();
    assert_eq!(hidden(&doc, &view, source), None);
    assert!(view.is_collapsed(code));
    let change = preview(&doc, &mut view, None);
    assert_eq!(change.source, doc.fold_candidate(code).unwrap().hidden);
    assert_eq!(hidden(&doc, &view, source), Some(code));
    assert_eq!(
        hidden(&doc, &view, doc.source().find("# Next").unwrap()),
        None
    );
    assert!(view.is_collapsed(heading(2)));
}

#[test]
fn start_switch_and_end_invalidate_tickets_checkpoints_and_tasks() {
    let doc = document("# A\nbody\n# B\ntail\n");
    let mut view = FoldView::<u32>::with_state(&doc, 1).unwrap();
    view.set_collapsed(&doc, heading(0), true).unwrap();
    for target in [Some(heading(0)), Some(heading(1)), None] {
        view.insert_checkpoint(TileId(0), 42).unwrap();
        let ticket = view.heights().ticket(TileId(0)).unwrap();
        let mut batch =
            FoldBatch::new(&doc, &view, FoldBatchAction::All { collapsed: true }).unwrap();
        let mut nav = FoldNavigation::new(
            &doc,
            &view,
            &Navigator::new(),
            NavAction::Home,
            FoldNavigationPolicy::Reveal,
            1,
        )
        .unwrap();
        preview(&doc, &mut view, target);
        assert_eq!(view.measure(ticket, 99), Err(LayoutError::StaleMeasurement));
        assert!(view.checkpoints().previous(TileId(0)).is_none());
        let mut budget = tiny();
        assert_eq!(
            batch.advance(&doc, &mut view, &mut budget),
            Err(LayoutError::StaleContinuation)
        );
        assert_eq!(
            nav.advance(&doc, &mut view, &mut budget),
            Err(LayoutError::StaleContinuation)
        );
        assert_eq!(budget.used(), WorkLimit::default());
    }
}

#[test]
fn million_byte_preview_toggles_and_restoration_have_bounded_index_work() {
    let mut doc = Document::new();
    doc.append(&format!("# A\n{}", "word\n".repeat(200_000)))
        .unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    view.set_collapsed(&doc, heading(0), true).unwrap();
    assert!(doc.tile_count() > 500);
    for _ in 0..20 {
        let work = doc.work();
        let visits = view.heights().mutation_visits();
        preview(&doc, &mut view, Some(heading(0)));
        assert_eq!(doc.work(), work);
        assert!(view.heights().mutation_visits() - visits < 1500);
        let update = doc.append("more\n").unwrap();
        view.apply(&doc, &update).unwrap();
        let work = doc.work();
        let visits = view.heights().mutation_visits();
        preview(&doc, &mut view, None);
        assert_eq!(doc.work(), work);
        assert!(view.heights().mutation_visits() - visits < 1500);
        assert_eq!(
            hidden(&doc, &view, doc.source().len() - 2),
            Some(heading(0))
        );
        assert_eq!(
            view.heights().prefix_rows(doc.tile_count()).unwrap(),
            view.heights().total_rows()
        );
        assert!(view.heights().total_rows() <= 2);
    }
    assert!(view.heights().node_count() < 300);
}
