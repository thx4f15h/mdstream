use mdstream::*;
use std::collections::BTreeMap;

fn one() -> Budget {
    Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        ..Default::default()
    })
}
fn zero() -> Budget {
    Budget::new(WorkLimit::default())
}
fn parse(s: &str) -> Document {
    let mut d = Document::new();
    d.append(s).unwrap();
    d.finish();
    d
}
fn publish<S>(d: &Document, v: &mut FoldView<S>, mut task: TableRowRegistration) -> TableRowFoldId {
    let p = task.advance(d, v, &mut one()).unwrap();
    assert_eq!(p.completion, Completion::Covered);
    let Some(FoldId::TableRange(id)) = p.id else {
        panic!()
    };
    id
}
fn fixed<S>(d: &Document, v: &mut FoldView<S>, a: usize, b: usize) -> TableRowFoldId {
    let t = TableRowRegistration::fixed_overlay(d, v, TableId(0), a..b).unwrap();
    publish(d, v, t)
}
fn prefix<S>(d: &Document, v: &mut FoldView<S>, n: usize) -> TableRowFoldId {
    let t = TableRowRegistration::prefix(d, v, TableId(0), n).unwrap();
    publish(d, v, t)
}
fn choice<S>(d: &Document, v: &mut FoldView<S>, id: TableRowFoldId, on: bool) {
    v.set_collapsed(d, FoldId::TableRange(id), on).unwrap();
}

#[test]
fn registration_edit_bind_actual_document_and_view_before_budget_or_mutation() {
    let d = parse("| H |\n| - |\n| a |\n| b |\n");
    let foreign = parse(d.source());
    let mut a = FoldView::new(&d, 1).unwrap();
    let mut b = FoldView::new(&d, 1).unwrap();
    let mut task = TableRowRegistration::fixed_overlay(&d, &a, TableId(0), 1..2).unwrap();
    let version = a.version();
    let work = a.table_row_work();
    let mut z = zero();
    assert_eq!(
        task.advance(&d, &mut a, &mut z).unwrap().completion,
        Completion::BudgetExhausted
    );
    assert_eq!(a.version(), version);
    assert_eq!(a.table_row_work(), work);
    assert_eq!(z.used(), WorkLimit::default());
    let mut budget = one();
    assert_eq!(
        task.advance(&d, &mut b, &mut budget),
        Err(LayoutError::WrongView)
    );
    assert_eq!(budget.used(), WorkLimit::default());
    assert_eq!(
        task.advance(&foreign, &mut a, &mut budget),
        Err(LayoutError::WrongDocument)
    );
    assert_eq!(budget.used(), WorkLimit::default());
    let id = publish(&d, &mut a, task);
    assert!(b.table_row_fold_info(&d, id).unwrap().is_none());
    assert_eq!(
        b.set_collapsed(&d, FoldId::TableRange(id), true),
        Err(LayoutError::InvalidRange)
    );
    assert_eq!(
        a.table_row_fold_info(&foreign, id),
        Err(LayoutError::WrongDocument)
    );
    let mut edit =
        TableRowEdit::retarget(&d, &a, id, TableRowExtent::Fixed { start: 2, end: 3 }).unwrap();
    let before = a.table_row_fold_info(&d, id).unwrap();
    let version = a.version();
    assert_eq!(
        edit.advance(&d, &mut a, &mut zero()).unwrap().completion,
        Completion::BudgetExhausted
    );
    assert_eq!(a.version(), version);
    assert_eq!(a.table_row_fold_info(&d, id).unwrap(), before);
    assert_eq!(
        edit.advance(&d, &mut b, &mut one()),
        Err(LayoutError::WrongView)
    );
    assert!(
        TableRowEdit::retarget(&d, &a, id, TableRowExtent::Fixed { start: 0, end: 1 }).is_err()
    );
    assert!(
        TableRowEdit::retarget(&d, &a, id, TableRowExtent::Fixed { start: 2, end: 9 }).is_err()
    );
    assert_eq!(a.table_row_fold_info(&d, id).unwrap(), before);
    let p = edit.advance(&d, &mut a, &mut one()).unwrap();
    assert_eq!(p.id, Some(FoldId::TableRange(id)));
    assert_eq!(
        a.table_row_fold_info(&d, id).unwrap().unwrap().rows,
        Some(2..3)
    );
    assert_eq!(
        TableRowRegistration::fixed_overlay(&foreign, &a, TableId(0), 1..2).err(),
        Some(LayoutError::WrongDocument)
    );
    let mut query = TableRowCandidates::all(&d, &a, TableId(0)).unwrap();
    let mut budget = one();
    assert_eq!(query.next(&d, &b, &mut budget), Err(LayoutError::WrongView));
    assert_eq!(budget.used(), WorkLimit::default());
    assert_eq!(
        query.next(&d, &a, &mut zero()).unwrap(),
        TableRowCandidateStep::BudgetExhausted
    );
    assert!(
        matches!(query.next(&d,&a,&mut one()).unwrap(),TableRowCandidateStep::Candidate(i) if i.id==id)
    );
}

#[test]
fn exact_dormant_seal_anchor_and_live_growing_rows_use_only_real_metadata() {
    let mut d = Document::new();
    d.append("| H |\n").unwrap();
    assert!(d.table(TableId(0)).is_none());
    d.append("| - |\n").unwrap();
    let mut v = FoldView::new(&d, 1).unwrap();
    let dormant = prefix(&d, &mut v, 99);
    let anchor = d.table(TableId(0)).unwrap().delimiter.end;
    assert!(
        v.table_row_fold_info(&d, dormant)
            .unwrap()
            .unwrap()
            .candidate
            .hidden
            .is_empty()
    );
    let mut seal = TableRowEdit::seal(&d, &v, dormant).unwrap();
    seal.advance(&d, &mut v, &mut one()).unwrap();
    assert_eq!(
        v.table_row_fold_info(&d, dormant).unwrap().unwrap().extent,
        TableRowExtent::SealedEmpty {
            start: 100,
            source: anchor
        }
    );
    let live = prefix(&d, &mut v, 0);
    let u = d.append("| tiny").unwrap();
    v.apply(&d, &u).unwrap();
    assert_eq!(d.table(TableId(0)).unwrap().rows().len(), 1);
    assert!(
        v.table_row_fold_info(&d, live)
            .unwrap()
            .unwrap()
            .candidate
            .hidden
            .is_empty()
    );
    let u = d.append(&"x".repeat(5000)).unwrap();
    v.apply(&d, &u).unwrap();
    assert!(!d.table(TableId(0)).unwrap().row(1).unwrap().complete);
    let fixed = fixed(&d, &mut v, 1, 2);
    choice(&d, &mut v, fixed, true);
    let old = v
        .table_row_fold_info(&d, fixed)
        .unwrap()
        .unwrap()
        .candidate
        .hidden
        .end;
    let u = d.append("GROW").unwrap();
    v.apply(&d, &u).unwrap();
    assert!(
        v.table_row_fold_info(&d, fixed)
            .unwrap()
            .unwrap()
            .candidate
            .hidden
            .end
            > old
    );
    let mut seal = TableRowEdit::seal(&d, &v, live).unwrap();
    seal.advance(&d, &mut v, &mut one()).unwrap();
    let u = d.append(" |\n| next |\n").unwrap();
    v.apply(&d, &u).unwrap();
    assert_eq!(
        v.table_row_fold_info(&d, live).unwrap().unwrap().rows,
        Some(1..2)
    );
    assert_eq!(
        v.table_row_fold_info(&d, live)
            .unwrap()
            .unwrap()
            .candidate
            .hidden,
        v.table_row_fold_info(&d, fixed)
            .unwrap()
            .unwrap()
            .candidate
            .hidden
    );
    assert_eq!(
        v.table_row_fold_info(&d, dormant)
            .unwrap()
            .unwrap()
            .candidate
            .hidden,
        Span::new(anchor, anchor)
    );
    let u = d.finish();
    v.apply(&d, &u).unwrap();
    assert_eq!(v.table_row_work().watched_groups, 0);
}

#[test]
fn sealed_empty_left_affinity_keeps_next_heading_batch_out_of_old_table() {
    let mut d = Document::new();
    d.append("| H |\n| - |\n").unwrap();
    let mut v = FoldView::new(&d, 1).unwrap();
    let id = prefix(&d, &mut v, 99);
    TableRowEdit::seal(&d, &v, id)
        .unwrap()
        .advance(&d, &mut v, &mut one())
        .unwrap();
    let cut = d.source().len();
    let u = d.append("# Next\nbody\n").unwrap();
    v.apply(&d, &u).unwrap();
    assert_eq!(d.table(TableId(0)).unwrap().source.end, cut);
    assert_eq!(d.headings().get(HeadingId(0)).unwrap().source.start, cut);
    let mut batch = FoldBatch::new(
        &d,
        &v,
        FoldBatchAction::Subtree {
            root: FoldId::Heading(HeadingId(0)),
            collapsed: false,
        },
    )
    .unwrap();
    let mut visited = 0;
    for _ in 0..20 {
        let p = batch.advance(&d, &mut v, &mut one()).unwrap();
        visited = p.visited;
        if p.completion == Completion::Covered {
            break;
        }
    }
    assert!(v.is_collapsed(FoldId::TableRange(id)));
    assert_eq!(visited, 2); // Heading + paragraph; no old row registration.
    let mut batch = FoldBatch::new(
        &d,
        &v,
        FoldBatchAction::Subtree {
            root: FoldId::Table(TableId(0)),
            collapsed: false,
        },
    )
    .unwrap();
    for _ in 0..20 {
        if batch.advance(&d, &mut v, &mut one()).unwrap().completion == Completion::Covered {
            break;
        }
    }
    assert!(!v.is_collapsed(FoldId::TableRange(id)));
    assert_eq!(
        v.table_row_fold_info(&d, id).unwrap().unwrap().extent,
        TableRowExtent::SealedEmpty {
            start: 100,
            source: cut
        }
    );
}

// Independent physical row/source/mask oracle. Never uses candidate(), registry
// grouping or core index summaries to obtain its expected intervals.
fn verify(d: &Document, v: &FoldView, specs: &BTreeMap<TableRowFoldId, (usize, usize, bool)>) {
    let t = d.table(TableId(0)).unwrap();
    let mut all = specs
        .iter()
        .filter(|(id, (_, _, on))| *on && v.preview() != Some(FoldId::TableRange(**id)))
        .map(|(&id, &(a, b, _))| {
            (
                Span::new(t.rows()[a].source.start, t.rows()[b - 1].source.end),
                id,
            )
        })
        .collect::<Vec<_>>();
    all.sort_by_key(|&(s, id)| (s.start, std::cmp::Reverse(s.end), id));
    let mut union: Vec<(Span, TableRowFoldId)> = Vec::new();
    for &(s, id) in &all {
        if let Some((p, _)) = union.last_mut()
            && s.start < p.end
        {
            p.end = p.end.max(s.end);
            continue;
        }
        union.push((s, id));
    }
    for at in (0..d.source().len()).step_by(23) {
        let expected = union.iter().find(|(s, _)| s.start <= at && at < s.end);
        match v.hidden_at(d, at, &mut one()).unwrap() {
            FoldLookup::Hidden(c) => {
                let expected = expected.unwrap();
                assert_eq!(c.hidden.start, expected.0.start);
                assert_eq!(c.id, FoldId::TableRange(expected.1));
            }
            FoldLookup::Visible => assert!(expected.is_none()),
            _ => panic!(),
        }
    }
    for i in 0..d.tile_count() {
        let covered = all.iter().any(|&(s, _)| {
            let start = d.tile_at(s.start).unwrap().0 + 1;
            let end = d.tile_at(s.end).map_or(d.tile_count(), |t| t.0);
            start <= i && i < end
        });
        assert_eq!(
            v.heights().height(TileId(i)),
            Some(if covered {
                0
            } else {
                v.heights().base_height(TileId(i)).unwrap()
            })
        );
    }
    let mut p = FoldProjection::new(d, v, Span::new(0, d.source().len())).unwrap();
    let mut ph = Vec::new();
    let mut hidden = 0;
    let mut visible = 0;
    let mut complete = false;
    for _ in 0..200_000 {
        let step = p
            .next(
                d,
                v,
                &mut Budget::new(WorkLimit {
                    queries: 1,
                    objects: 1,
                    tiles: 1,
                    bytes: 6,
                    ..Default::default()
                }),
            )
            .unwrap();
        match step {
            FoldProjectionStep::Placeholder(p) => ph.push(p.candidate.hidden),
            FoldProjectionStep::Hidden { source, .. } => hidden += source.len(),
            FoldProjectionStep::Text { source, .. } => visible += source.len(),
            FoldProjectionStep::Complete => {
                complete = true;
                break;
            }
            _ => {}
        }
    }
    assert!(complete);
    assert_eq!(ph, union.iter().map(|x| x.0).collect::<Vec<_>>());
    assert_eq!(hidden, union.iter().map(|x| x.0.len()).sum::<usize>());
    assert_eq!(visible + hidden, d.source().len());
}

#[test]
fn independent_random_overlap_mask_height_placeholder_and_source_oracle() {
    let d = parse(&format!(
        "| H |\n| - |\n{}",
        (0..24)
            .map(|n| format!("| {n}{} |\n", "x".repeat(260)))
            .collect::<String>()
    ));
    let mut v = FoldView::new(&d, 2).unwrap();
    let mut specs = BTreeMap::new();
    let mut rng = 33u64;
    for turn in 0..700 {
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
        let a = 1 + rng as usize % 24;
        let b = a + 1 + (rng >> 16) as usize % (25 - a);
        if specs.is_empty() || turn % 4 == 0 {
            let id = fixed(&d, &mut v, a, b);
            choice(&d, &mut v, id, true);
            specs.insert(id, (a, b, true));
        } else {
            let id = *specs
                .keys()
                .nth((rng >> 32) as usize % specs.len())
                .unwrap();
            match turn % 5 {
                0 => {
                    let mut t = TableRowEdit::remove(&d, &v, id).unwrap();
                    t.advance(&d, &mut v, &mut one()).unwrap();
                    specs.remove(&id);
                }
                1 => {
                    let mut t = TableRowEdit::retarget(
                        &d,
                        &v,
                        id,
                        TableRowExtent::Fixed { start: a, end: b },
                    )
                    .unwrap();
                    t.advance(&d, &mut v, &mut one()).unwrap();
                    let on = specs[&id].2;
                    specs.insert(id, (a, b, on));
                }
                2 => {
                    v.set_preview(&d, Some(FoldId::TableRange(id)), &mut one())
                        .unwrap();
                }
                _ => {
                    let on = !specs[&id].2;
                    choice(&d, &mut v, id, on);
                    specs.get_mut(&id).unwrap().2 = on;
                }
            }
        }
        verify(&d, &v, &specs);
    }
}

#[test]
fn legacy_contract_both_orders_preview_parent_and_old_eof_boundary() {
    let mut d = Document::new();
    d.append(&format!("| H |\n| - |\n| a |\n| {} |\n", "x".repeat(5000)))
        .unwrap();
    let mut v = FoldView::new(&d, 2).unwrap();
    let legacy = v.insert_table_rows(&d, TableId(0), 1..2).unwrap();
    let a = fixed(&d, &mut v, 1, 3);
    choice(&d, &mut v, a, true);
    assert_eq!(v.insert_table_rows(&d, TableId(0), 1..2).unwrap(), legacy);
    assert_eq!(
        v.insert_table_rows(&d, TableId(0), 2..3),
        Err(LayoutError::InvalidRange)
    );
    let b = fixed(&d, &mut v, 2, 3);
    choice(&d, &mut v, b, true);
    v.set_collapsed(&d, FoldId::Table(TableId(0)), true)
        .unwrap();
    v.set_collapsed(&d, FoldId::Table(TableId(0)), false)
        .unwrap();
    assert!(v.is_collapsed(FoldId::TableRange(a)) && v.is_collapsed(FoldId::TableRange(b)));
    v.set_preview(&d, Some(FoldId::TableRange(a)), &mut one())
        .unwrap();
    v.set_preview(&d, Some(legacy), &mut one()).unwrap();
    v.set_preview(&d, Some(FoldId::TableRange(b)), &mut one())
        .unwrap();
    v.set_preview(&d, None, &mut one()).unwrap();
    let specs = BTreeMap::from([(a, (1, 3, true)), (b, (2, 3, true))]);
    verify(&d, &v, &specs);
    let work = v.table_row_work();
    let u = d.append("| next |\n").unwrap();
    v.apply(&d, &u).unwrap();
    verify(&d, &v, &specs);
    assert_eq!(v.table_row_work().group_refreshes - work.group_refreshes, 1);
    assert_eq!(v.table_row_work().watched_groups, 0);
    let at = d.table(TableId(0)).unwrap().row(3).unwrap().source.start;
    assert_eq!(
        v.hidden_at(&d, at, &mut one()).unwrap(),
        FoldLookup::Visible
    );
    v.reset(3).unwrap();
    verify(&d, &v, &specs);
}

#[test]
fn live_group_append_and_new_only_policy_do_not_refresh_history_or_all_members() {
    let mut d = Document::new();
    d.append("| H |\n| - |\n| a |\n").unwrap();
    let mut v = FoldView::new(&d, 1).unwrap();
    let mut ids = Vec::new();
    for _ in 0..1000 {
        let task = TableRowRegistration::live(&d, &v, TableId(0), 1).unwrap();
        let id = publish(&d, &mut v, task);
        choice(&d, &mut v, id, true);
        ids.push(id);
    }
    let before = v.table_row_work();
    for _ in 0..1000 {
        let u = d.append("| x |\n").unwrap();
        v.apply(&d, &u).unwrap();
    }
    assert_eq!(
        v.table_row_work().group_refreshes - before.group_refreshes,
        1000
    );
    assert_eq!(
        v.table_row_work().row_endpoint_reads - before.row_endpoint_reads,
        2000
    );
    assert_eq!(v.fold_index_nodes(), 1);
    let u = d.append("\nAFTER\n").unwrap();
    v.apply(&d, &u).unwrap();
    assert_eq!(v.table_row_work().watched_groups, 0);
    let before = v.table_row_work();
    let u = d.append("next\n").unwrap();
    v.apply(&d, &u).unwrap();
    assert_eq!(v.table_row_work().group_refreshes, before.group_refreshes);
    assert!(
        !v.set_table_row_policy(
            &d,
            Some(TableRowPolicy {
                visible_body_rows: 1
            }),
            &mut zero()
        )
        .unwrap()
    );
    assert!(
        v.set_table_row_policy(
            &d,
            Some(TableRowPolicy {
                visible_body_rows: 1
            }),
            &mut one()
        )
        .unwrap()
    );
    let u = d.append("\n| N |\n| - |\n| one |\n| two |\n").unwrap();
    v.apply(&d, &u).unwrap();
    let mut auto = TableRowAutoFold::for_update(&d, &v, &u).unwrap();
    assert_eq!(
        auto.advance(&d, &mut v, &mut zero()).unwrap().completion,
        Completion::BudgetExhausted
    );
    auto.advance(&d, &mut v, &mut one()).unwrap();
    let mut list = TableRowCandidates::all(&d, &v, TableId(1)).unwrap();
    let TableRowCandidateStep::Candidate(info) = list.next(&d, &v, &mut one()).unwrap() else {
        panic!()
    };
    assert_eq!(info.rows, Some(2..3));
    assert!(info.collapsed);
    assert!(ids.iter().all(|&id| v.is_collapsed(FoldId::TableRange(id))));
}

#[test]
fn same_view_checkpoints_compaction_batch_and_actual_covering_reveal_keep_independent_choices() {
    let mut d = Document::new();
    d.append(&format!(
        "| H |\n| - |\n{}",
        (0..8)
            .map(|n| format!("| {n}{} |\n", "x".repeat(800)))
            .collect::<String>()
    ))
    .unwrap();
    let mut v = FoldView::<usize>::with_state(&d, 2).unwrap();
    let a = fixed(&d, &mut v, 1, 4);
    let b = fixed(&d, &mut v, 3, 8);
    choice(&d, &mut v, a, true);
    choice(&d, &mut v, b, true);
    let stale = v.heights().ticket(TileId(0)).unwrap();
    choice(&d, &mut v, a, false);
    assert_eq!(v.measure(stale, 99), Err(LayoutError::StaleMeasurement));
    choice(&d, &mut v, a, true);
    {
        let (h, cp) = v.layout_parts_mut(&d).unwrap();
        cp.insert_for_layout(h, TileId(0), 17).unwrap();
        cp.insert_for_layout(h, TileId(1), 23).unwrap();
        cp.invalidate(1..2).unwrap();
    }
    let token = v.checkpoints().token(TileId(0)).unwrap();
    let mut owner_probe =
        TableRowEdit::retarget(&d, &v, a, TableRowExtent::Fixed { start: 1, end: 4 }).unwrap();
    let mut task = v.into_checkpoint_compaction();
    let mut retired = Vec::new();
    assert_eq!(
        task.advance(&d, &mut zero(), &mut retired).unwrap(),
        Completion::BudgetExhausted
    );
    loop {
        if task.advance(&d, &mut one(), &mut retired).unwrap() == Completion::Covered {
            break;
        }
    }
    let mut v = match task.finish(&d) {
        Ok(v) => v,
        Err(_) => panic!(),
    };
    assert_eq!(
        owner_probe
            .advance(&d, &mut v, &mut zero())
            .unwrap()
            .completion,
        Completion::BudgetExhausted
    );
    assert!(v.checkpoints().validates(token));
    assert_eq!(retired, vec![23]);
    v.reset(3).unwrap();
    assert!(v.is_collapsed(FoldId::TableRange(a)) && v.is_collapsed(FoldId::TableRange(b)));
    let mut all = FoldBatch::new(&d, &v, FoldBatchAction::All { collapsed: false }).unwrap();
    loop {
        let mut q = one();
        if all.advance(&d, &mut v, &mut q).unwrap().completion == Completion::Covered {
            break;
        }
        assert!(q.used().queries <= 1 && q.used().objects <= 1);
    }
    assert!(!v.is_collapsed(FoldId::TableRange(a)) && !v.is_collapsed(FoldId::TableRange(b)));
    choice(&d, &mut v, a, true);
    choice(&d, &mut v, b, true);
    let target = d.table(TableId(0)).unwrap().row(6).unwrap().source.start;
    let origin = Navigator::new();
    let mut reveal = FoldNavigation::new(
        &d,
        &v,
        &origin,
        NavAction::Source(target),
        FoldNavigationPolicy::Reveal,
        8,
    )
    .unwrap();
    for _ in 0..100 {
        if reveal.advance(&d, &mut v, &mut one()).unwrap().completion == Completion::Covered {
            break;
        }
    }
    assert!(v.is_collapsed(FoldId::TableRange(a)));
    assert!(!v.is_collapsed(FoldId::TableRange(b)));
    let common = [
        fixed(&d, &mut v, 1, 8),
        fixed(&d, &mut v, 3, 8),
        fixed(&d, &mut v, 7, 8),
    ];
    for id in common {
        choice(&d, &mut v, id, true);
    }
    let target = d.table(TableId(0)).unwrap().row(5).unwrap().source.start;
    let mut reveal = FoldNavigation::new(
        &d,
        &v,
        &origin,
        NavAction::Source(target),
        FoldNavigationPolicy::Reveal,
        8,
    )
    .unwrap();
    for _ in 0..100 {
        if reveal.advance(&d, &mut v, &mut one()).unwrap().completion == Completion::Covered {
            break;
        }
    }
    assert!(!v.is_collapsed(FoldId::TableRange(common[0])));
    assert!(!v.is_collapsed(FoldId::TableRange(common[1])));
    assert!(v.is_collapsed(FoldId::TableRange(common[2])));
    assert!(v.is_collapsed(FoldId::TableRange(a)));
    let u = d.append("| later |\n").unwrap();
    v.apply(&d, &u).unwrap();
    assert_eq!(
        v.table_row_fold_info(&d, a).unwrap().unwrap().rows,
        Some(1..4)
    );
}

#[test]
fn thousand_nested_crossing_and_container_rows_reuse_real_metadata_and_one_header() {
    let container_prefix = "> ".repeat(1000);
    let d = parse(&format!(
        "{container_prefix}| H |\r\n{container_prefix}| - |\r\n{container_prefix}| a |\r\n{container_prefix}| 界 |\r\n{container_prefix}| 🙂 |\r\n"
    ));
    let mut v = FoldView::new(&d, 1).unwrap();
    let row = prefix(&d, &mut v, 0);
    let at = d.table(TableId(0)).unwrap().row(0).unwrap().source.start;
    assert_eq!(
        v.hidden_at(&d, at, &mut one()).unwrap(),
        FoldLookup::Visible
    );
    v.set_collapsed(&d, FoldId::Container(ContainerId(0)), true)
        .unwrap();
    v.set_collapsed(&d, FoldId::Container(ContainerId(0)), false)
        .unwrap();
    assert!(v.is_collapsed(FoldId::TableRange(row)));
    let d = parse(&format!("| H |\n| - |\n{}", "| x |\n".repeat(1100)));
    let mut v = FoldView::new(&d, 1).unwrap();
    for end in 2..1002 {
        let id = fixed(&d, &mut v, 1, end);
        choice(&d, &mut v, id, true);
    }
    let at = d.table(TableId(0)).unwrap().row(999).unwrap().source.start;
    let before = v.fold_index_visits();
    assert!(
        matches!(v.hidden_at(&d,at,&mut one()).unwrap(),FoldLookup::Hidden(c) if c.hidden.start==d.table(TableId(0)).unwrap().row(1).unwrap().source.start)
    );
    assert!(v.fold_index_visits() - before < 40);
    let mut v = FoldView::new(&d, 1).unwrap();
    for start in 1..1001 {
        let id = fixed(&d, &mut v, start, start + 2);
        choice(&d, &mut v, id, true);
    }
    let before = v.fold_index_visits();
    assert!(
        matches!(v.hidden_at(&d,at,&mut one()).unwrap(),FoldLookup::Hidden(c) if c.hidden.start==d.table(TableId(0)).unwrap().row(1).unwrap().source.start)
    );
    assert!(v.fold_index_visits() - before < 40);
    let mut projection = FoldProjection::new(&d, &v, Span::new(0, d.source().len())).unwrap();
    let mut ph = Vec::new();
    for _ in 0..10_000 {
        match projection
            .next(
                &d,
                &v,
                &mut Budget::new(WorkLimit {
                    queries: 1,
                    objects: 1,
                    tiles: 1,
                    ..Default::default()
                }),
            )
            .unwrap()
        {
            FoldProjectionStep::Placeholder(p) => ph.push(p),
            FoldProjectionStep::Complete => break,
            _ => {}
        }
    }
    assert_eq!(ph.len(), 1);
    assert_eq!(
        ph[0].candidate.hidden,
        Span::new(
            d.table(TableId(0)).unwrap().row(1).unwrap().source.start,
            d.table(TableId(0)).unwrap().row(1001).unwrap().source.end
        )
    );
}
