use mdstream::*;
fn unlimited() -> Budget {
    Budget::new(WorkLimit::unlimited())
}
fn one() -> Budget {
    Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        ..Default::default()
    })
}
fn capture<S>(v: &FoldView<S>) -> FoldViewIntent {
    let mut task = v.capture_intent();
    let mut zero = Budget::new(WorkLimit::default());
    assert_eq!(
        task.advance(v, &mut zero).unwrap(),
        Completion::BudgetExhausted
    );
    assert_eq!(zero.used(), WorkLimit::default());
    for _ in 0..1000 {
        if task.advance(v, &mut one()).unwrap() == Completion::Covered {
            return task.finish().unwrap();
        }
    }
    panic!("capture did not complete")
}

#[test]
fn pure_intent_restores_choices_defaults_preview_and_source_ids_without_old_layout_tickets() {
    let mut d = Document::new();
    d.append("# A\n\nalpha line\n\n# B\n\nbeta\n\n- [ ] item\n")
        .unwrap();
    d.finish();
    let mut old = FoldView::<usize>::with_state(&d, 1).unwrap();
    old.set_collapsed(&d, FoldId::Heading(HeadingId(0)), true)
        .unwrap();
    let paragraph = BlockId(
        d.blocks()
            .iter()
            .position(|b| b.kind == BlockKind::Paragraph)
            .unwrap(),
    );
    let mut registration = SourceFoldRegistration::paragraph(&d, &old, paragraph, 2).unwrap();
    let id = registration
        .advance(&d, &mut old, &mut unlimited())
        .unwrap()
        .id
        .unwrap();
    old.set_collapsed(&d, id, true).unwrap();
    old.set_preview(&d, Some(FoldId::Heading(HeadingId(0))), &mut unlimited())
        .unwrap();
    let task = (0..d.container_count())
        .map(ContainerId)
        .find(|&id| {
            matches!(
                d.container(id).unwrap().kind,
                ContainerKind::Item { task: Some(_), .. }
            )
        })
        .unwrap();
    old.set_task_checked(&d, task, true).unwrap();
    let defaults = FoldDefaults {
        code: true,
        heading_levels: [true, false, false, false, false, false],
        ..Default::default()
    };
    old.set_defaults(defaults);
    let ticket = old.heights().ticket(TileId(0)).unwrap();
    old.measure(ticket, 77).unwrap();
    old.insert_checkpoint(TileId(0), 55).unwrap();
    let mut intent = capture(&old);
    assert!(intent.records() >= 4);
    let mut fresh = FoldView::<usize>::with_state(&d, 1).unwrap();
    let mut zero = Budget::new(WorkLimit::default());
    assert_eq!(
        intent.restore(&d, &mut fresh, &mut zero).unwrap(),
        Completion::BudgetExhausted
    );
    assert_eq!(fresh.defaults(), FoldDefaults::default());
    let mut other = Document::new();
    other.finish();
    let mut foreign = FoldView::<usize>::with_state(&other, 1).unwrap();
    assert_eq!(
        intent.restore(&other, &mut foreign, &mut zero),
        Err(LayoutError::WrongDocument)
    );
    assert_eq!(zero.used(), WorkLimit::default());
    for _ in 0..1000 {
        if intent.restore(&d, &mut fresh, &mut one()).unwrap() == Completion::Covered {
            break;
        }
    }
    assert_eq!(fresh.defaults(), defaults);
    assert!(fresh.is_collapsed(FoldId::Heading(HeadingId(0))));
    assert_eq!(fresh.preview(), Some(FoldId::Heading(HeadingId(0))));
    assert!(fresh.is_collapsed(id));
    assert_eq!(fresh.task_checked(&d, task).unwrap(), Some(true));
    assert!(fresh.fold_candidate(&d, id).unwrap().is_some());
    assert_eq!(fresh.checkpoints().len(), 0);
    assert!(fresh.measure(ticket, 7).is_err());
    let mut wrong_owner = FoldView::<usize>::with_state(&d, 1).unwrap();
    assert_eq!(
        intent.restore(&d, &mut wrong_owner, &mut zero),
        Err(LayoutError::InvalidRange)
    );
    assert_eq!(zero.used(), WorkLimit::default());
}

#[test]
fn inactive_view_new_syntax_gets_saved_defaults_and_cancelled_capture_retires_by_record() {
    let mut d = Document::new();
    d.append("# A\n\na\n").unwrap();
    let mut old = FoldView::new(&d, 1).unwrap();
    let defaults = FoldDefaults {
        heading_levels: [true, false, false, false, false, false],
        ..Default::default()
    };
    old.set_defaults(defaults);
    let mut intent = capture(&old);
    d.append("# B\n\nb\n").unwrap();
    d.finish();
    let mut fresh = FoldView::new(&d, 1).unwrap();
    for _ in 0..100 {
        if intent.restore(&d, &mut fresh, &mut one()).unwrap() == Completion::Covered {
            break;
        }
    }
    assert!(!fresh.is_collapsed(FoldId::Heading(HeadingId(0))));
    assert!(fresh.is_collapsed(FoldId::Heading(HeadingId(1))));
    let mut task = fresh.capture_intent();
    task.advance(&fresh, &mut one()).unwrap();
    let mut zero = Budget::new(WorkLimit::default());
    assert_eq!(task.retire(&mut zero), Completion::BudgetExhausted);
    assert_eq!(zero.used(), WorkLimit::default());
    assert_eq!(
        task.advance(&fresh, &mut zero),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(task.retire(&mut one()), Completion::Covered);
}

#[test]
fn append_and_eof_during_paid_capture_and_restore_keep_one_snapshot_and_current_geometry() {
    let mut d = Document::new();
    d.append("# A\n\nalpha\n").unwrap();
    let mut old = FoldView::new(&d, 1).unwrap();
    old.set_collapsed(&d, FoldId::Heading(HeadingId(0)), true)
        .unwrap();
    old.set_defaults(FoldDefaults {
        heading_levels: [true, false, false, false, false, false],
        ..Default::default()
    });
    let mut capture = old.capture_intent();
    capture.advance(&old, &mut one()).unwrap();
    d.append("# B\n\nbeta\n").unwrap();
    while capture.advance(&old, &mut one()).unwrap() != Completion::Covered {}
    let mut intent = capture.finish().unwrap();
    let mut fresh = FoldView::new(&d, 1).unwrap();
    intent.restore(&d, &mut fresh, &mut one()).unwrap();
    let update = d.finish();
    fresh.apply(&d, &update).unwrap();
    while intent.restore(&d, &mut fresh, &mut one()).unwrap() != Completion::Covered {}
    assert!(fresh.is_collapsed(FoldId::Heading(HeadingId(0))));
    assert!(fresh.is_collapsed(FoldId::Heading(HeadingId(1))));
    assert_eq!(fresh.heights().len(), d.tile_count());
}

#[test]
fn restored_table_row_registration_rebuilds_growing_and_fixed_boundary_indexes() {
    let mut d = Document::new();
    d.append(&format!("a|b\n---|---\n1|{}", "x".repeat(1200)))
        .unwrap();
    let mut old = FoldView::new(&d, 1).unwrap();
    let rows = TableRowRange {
        table: TableId(0),
        start: 1,
        end: 2,
    };
    let id = old
        .insert_table_rows(&d, rows.table, rows.start..rows.end)
        .unwrap();
    old.set_collapsed(&d, id, true).unwrap();
    let mut intent = capture(&old);
    d.append(" continued").unwrap();
    let mut fresh = FoldView::new(&d, 1).unwrap();
    while intent.restore(&d, &mut fresh, &mut one()).unwrap() != Completion::Covered {}
    assert!(fresh.is_collapsed(id));
    assert_eq!(fresh.table_row_range_count(), 1);
    let actual = fresh.fold_candidate(&d, id).unwrap().unwrap();
    assert_eq!(actual.hidden, d.fold_candidate(id).unwrap().hidden);
    let update = d.append("\n3|4\n").unwrap();
    fresh.apply(&d, &update).unwrap();
    assert_eq!(
        fresh.fold_candidate(&d, id).unwrap().unwrap().hidden,
        d.fold_candidate(id).unwrap().hidden
    );
    let update = d.finish();
    fresh.apply(&d, &update).unwrap();
    assert!(fresh.is_collapsed(id));
}

#[test]
fn stale_partial_capture_keeps_its_records_for_paid_disposal() {
    let mut d = Document::new();
    d.append("# A\n\na\n\n# B\n\nb\n").unwrap();
    d.finish();
    let mut view = FoldView::new(&d, 1).unwrap();
    view.set_collapsed(&d, FoldId::Heading(HeadingId(0)), true)
        .unwrap();
    view.set_collapsed(&d, FoldId::Heading(HeadingId(1)), true)
        .unwrap();
    let mut capture = view.capture_intent();
    capture.advance(&view, &mut one()).unwrap();
    capture.advance(&view, &mut one()).unwrap();
    view.set_collapsed(&d, FoldId::Heading(HeadingId(0)), false)
        .unwrap();
    let mut zero = Budget::new(WorkLimit::default());
    assert_eq!(
        capture.advance(&view, &mut zero),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(zero.used(), WorkLimit::default());
    assert_eq!(capture.retire(&mut zero), Completion::BudgetExhausted);
    assert_eq!(capture.retire(&mut one()), Completion::BudgetExhausted);
    assert_eq!(capture.retire(&mut one()), Completion::Covered);
}

#[test]
fn capture_rejects_another_view_of_the_same_document_with_equal_versions_before_work() {
    let mut d = Document::new();
    d.append("# A\n\na\n\n# B\n\nb\n").unwrap();
    d.finish();
    let mut a = FoldView::new(&d, 1).unwrap();
    let mut b = FoldView::new(&d, 1).unwrap();
    a.set_collapsed(&d, FoldId::Heading(HeadingId(0)), true)
        .unwrap();
    b.set_collapsed(&d, FoldId::Heading(HeadingId(1)), true)
        .unwrap();
    assert_eq!(a.version(), b.version());
    let mut capture = a.capture_intent();
    for limit in [WorkLimit::default(), WorkLimit::unlimited()] {
        let mut budget = Budget::new(limit);
        assert_eq!(
            capture.advance(&b, &mut budget),
            Err(LayoutError::InvalidRange)
        );
        assert_eq!(budget.used(), WorkLimit::default());
    }
    while capture.advance(&a, &mut one()).unwrap() != Completion::Covered {}
    let mut intent = capture.finish().unwrap();
    let mut fresh = FoldView::new(&d, 1).unwrap();
    while intent.restore(&d, &mut fresh, &mut one()).unwrap() != Completion::Covered {}
    assert!(fresh.is_collapsed(FoldId::Heading(HeadingId(0))));
    assert!(!fresh.is_collapsed(FoldId::Heading(HeadingId(1))));
}

#[test]
fn stable_table_row_ids_choices_policy_and_empty_seal_survive_a_new_layout_owner() {
    let mut d = Document::new();
    d.append("| H |\n| - |\n| a |\n| b |\n| c |\n\nstop\n\n| U |\n| - |\n")
        .unwrap();
    let mut old = FoldView::<usize>::with_state(&d, 1).unwrap();
    let policy = Some(TableRowPolicy {
        visible_body_rows: 1,
    });
    assert!(old.set_table_row_policy(&d, policy, &mut one()).unwrap());
    let mut register = |table, rows: Option<std::ops::Range<usize>>, prefix| {
        let mut task = if let Some(rows) = rows {
            TableRowRegistration::fixed_overlay(&d, &old, table, rows).unwrap()
        } else {
            TableRowRegistration::prefix(&d, &old, table, prefix).unwrap()
        };
        let Some(FoldId::TableRange(id)) = task.advance(&d, &mut old, &mut one()).unwrap().id
        else {
            panic!()
        };
        id
    };
    let auto = register(TableId(0), None, 1);
    let fixed = register(TableId(0), Some(1..3), 0);
    let expanded = register(TableId(0), Some(2..3), 0);
    let empty = register(TableId(1), None, 99);
    old.set_collapsed(&d, FoldId::TableRange(fixed), true)
        .unwrap();
    let mut seal = TableRowEdit::seal(&d, &old, empty).unwrap();
    seal.advance(&d, &mut old, &mut one()).unwrap();
    let mut old_owner_edit = TableRowEdit::seal(&d, &old, auto).unwrap();
    let before =
        [auto, fixed, expanded, empty].map(|id| old.table_row_fold_info(&d, id).unwrap().unwrap());
    let ticket = old.heights().ticket(TileId(0)).unwrap();
    old.insert_checkpoint(TileId(0), 9).unwrap();
    let mut intent = capture(&old);
    let mut fresh = FoldView::<usize>::with_state(&d, 1).unwrap();
    for _ in 0..1000 {
        if intent.restore(&d, &mut fresh, &mut one()).unwrap() == Completion::Covered {
            break;
        }
    }
    assert_eq!(fresh.table_row_policy(), policy);
    for info in before {
        assert_eq!(fresh.table_row_fold_info(&d, info.id).unwrap(), Some(info));
    }
    assert_eq!(fresh.checkpoints().len(), 0);
    assert!(fresh.measure(ticket, 99).is_err());
    assert!(!fresh.is_collapsed(FoldId::TableRange(expanded)));
    assert_eq!(fresh.table_row_work().registrations, 4);
    assert_eq!(fresh.table_row_work().selected_groups, 2);
    let mut zero = Budget::new(WorkLimit::default());
    assert_eq!(
        old_owner_edit.advance(&d, &mut fresh, &mut zero),
        Err(LayoutError::WrongView)
    );
    assert_eq!(zero.used(), WorkLimit::default());
}

#[test]
fn a_closed_note_table_has_no_physical_growth_watch_while_its_host_is_live() {
    let mut host = Document::new();
    host.append("[^n]: | H |\n    | - |\n    | a |\n# root\n")
        .unwrap();
    let body = host.footnote_body(FootnoteId(0)).unwrap();
    body.with_renderer(body.key(), body.host_revision(), |native| {
        assert!(!native.is_finished());
        assert!(native.is_input_sealed());
        let mut view = FoldView::new(native, 1).unwrap();
        let mut task = TableRowRegistration::prefix(native, &view, TableId(0), 0).unwrap();
        let Some(FoldId::TableRange(id)) = task.advance(native, &mut view, &mut one()).unwrap().id
        else {
            panic!()
        };
        assert!(
            view.table_row_fold_info(native, id)
                .unwrap()
                .unwrap()
                .closed
        );
        assert_eq!(view.table_row_work().watched_groups, 0);
    })
    .unwrap();
}

#[test]
fn stable_row_policy_is_new_only_across_inactive_append_and_pending_restore() {
    let mut d = Document::new();
    d.append("| H |\n| - |\n| a |\n| b |\n").unwrap();
    let mut old = FoldView::new(&d, 1).unwrap();
    old.set_table_row_policy(
        &d,
        Some(TableRowPolicy {
            visible_body_rows: 1,
        }),
        &mut one(),
    )
    .unwrap();
    let mut prefix = TableRowRegistration::prefix(&d, &old, TableId(0), 1).unwrap();
    let Some(FoldId::TableRange(id)) = prefix.advance(&d, &mut old, &mut one()).unwrap().id else {
        panic!()
    };
    old.set_collapsed(&d, FoldId::TableRange(id), false)
        .unwrap();
    let mut intent = capture(&old);
    d.append("\nnext\n\n| U |\n| - |\n| c |\n| d |\n").unwrap();
    let mut fresh = FoldView::new(&d, 1).unwrap();
    for _ in 0..4 {
        assert_eq!(
            intent.restore(&d, &mut fresh, &mut one()).unwrap(),
            Completion::BudgetExhausted
        );
    }
    let u = d.append("\nnext\n\n| V |\n| - |\n| e |\n| f |\n").unwrap();
    fresh.apply(&d, &u).unwrap();
    let u = d.finish();
    fresh.apply(&d, &u).unwrap();
    let mut done = false;
    for _ in 0..1000 {
        if intent.restore(&d, &mut fresh, &mut one()).unwrap() == Completion::Covered {
            done = true;
            break;
        }
    }
    assert!(done);
    assert!(
        !fresh
            .table_row_fold_info(&d, id)
            .unwrap()
            .unwrap()
            .collapsed
    );
    assert_eq!(fresh.table_row_work().registrations, 3);
    assert_eq!(fresh.table_row_work().watched_groups, 0);
    for table in [TableId(1), TableId(2)] {
        let mut rows = TableRowCandidates::all(&d, &fresh, table).unwrap();
        let TableRowCandidateStep::Candidate(info) = rows.next(&d, &fresh, &mut one()).unwrap()
        else {
            panic!()
        };
        assert!(info.collapsed && info.closed);
        assert_eq!(info.extent, TableRowExtent::Live { start: 2 });
    }
    let mut prefix = TableRowRegistration::prefix(&d, &fresh, TableId(0), 0).unwrap();
    assert_eq!(
        prefix.advance(&d, &mut fresh, &mut one()).unwrap().id,
        Some(FoldId::TableRange(id))
    );
}

#[test]
fn stable_row_capture_cancel_and_actual_group_index_retirement_are_paid() {
    let mut d = Document::new();
    d.append("| H |\n| - |\n| a |\n| b |\n").unwrap();
    let mut view = FoldView::new(&d, 1).unwrap();
    for _ in 0..128 {
        let mut r = TableRowRegistration::fixed_overlay(&d, &view, TableId(0), 1..3).unwrap();
        let id = r.advance(&d, &mut view, &mut one()).unwrap().id.unwrap();
        view.set_collapsed(&d, id, true).unwrap();
    }
    let mut capture = view.capture_intent();
    for _ in 0..24 {
        capture.advance(&view, &mut one()).unwrap();
    }
    let records = capture.retire(&mut Budget::new(WorkLimit::default()));
    assert_eq!(records, Completion::BudgetExhausted);
    for _ in 0..1000 {
        if capture.retire(&mut one()) == Completion::Covered {
            break;
        }
    }
    let before = view.table_row_work();
    assert_eq!(before.registrations, 128);
    assert_eq!(before.selected_groups, 1);
    let mut zero = Budget::new(WorkLimit::default());
    assert_eq!(view.retire_choices(&mut zero), Completion::BudgetExhausted);
    assert_eq!(zero.used(), WorkLimit::default());
    assert_eq!(view.table_row_work(), before);
    let mut done = false;
    for _ in 0..10_000 {
        let mut b = one();
        if view.retire_choices(&mut b) == Completion::Covered {
            done = true;
            break;
        }
        assert!(b.used().queries <= 1 && b.used().objects <= 1);
    }
    assert!(done);
    let after = view.table_row_work();
    assert_eq!(after.registrations, 0);
    assert_eq!(after.selected_groups, 0);
    assert_eq!(after.row_index_nodes, 0);
    assert_eq!(after.watched_groups, 0);
}

#[test]
fn note_table_registrations_reject_another_body_and_actual_native_owner_before_work() {
    let mut host = Document::new();
    host.append("[^a]: | H |\n    | - |\n    | one |\n[^b]: | H |\n    | - |\n    | two |\n")
        .unwrap();
    host.finish();
    let a = host.footnote_body(FootnoteId(0)).unwrap();
    let b = host.footnote_body(FootnoteId(1)).unwrap();
    assert!(a.with_renderer(b.key(), host.revision(), |_| ()).is_err());
    let (mut view, mut edit) = a
        .with_renderer(a.key(), host.revision(), |native| {
            let mut view = FoldView::new(native, 1).unwrap();
            let mut r = TableRowRegistration::prefix(native, &view, TableId(0), 0).unwrap();
            let Some(FoldId::TableRange(id)) = r.advance(native, &mut view, &mut one()).unwrap().id
            else {
                panic!()
            };
            let edit = TableRowEdit::seal(native, &view, id).unwrap();
            (view, edit)
        })
        .unwrap();
    b.with_renderer(b.key(), host.revision(), |native| {
        assert!(native.table(TableId(0)).is_some());
        let mut zero = Budget::new(WorkLimit::default());
        assert_eq!(
            edit.advance(native, &mut view, &mut zero),
            Err(LayoutError::WrongDocument)
        );
        assert_eq!(zero.used(), WorkLimit::default());
    })
    .unwrap();
}
