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

fn candidates(doc: &Document) -> Vec<FoldCandidate> {
    let mut out = Vec::new();
    doc.fold_candidates(Span::new(0, doc.source().len()))
        .unwrap()
        .advance(doc, &mut Budget::new(WorkLimit::unlimited()), &mut out)
        .unwrap();
    out
}

fn finish(task: &mut FoldBatch, doc: &Document, view: &mut FoldView) -> FoldBatchProgress {
    for _ in 0..doc.blocks().len() + 2 {
        let mut budget = tiny();
        let progress = task.advance(doc, view, &mut budget).unwrap();
        assert_eq!(budget.used().bytes, 0);
        assert_eq!(budget.used().tiles, 0);
        if let Some(change) = progress.change {
            assert_eq!(change.version, view.version());
        }
        if progress.completion == Completion::Covered {
            let mut empty = Budget::new(WorkLimit::default());
            let repeated = task.advance(doc, view, &mut empty).unwrap();
            assert_eq!(
                repeated,
                FoldBatchProgress {
                    change: None,
                    ..progress
                }
            );
            assert_eq!(empty.used(), WorkLimit::default());
            return progress;
        }
    }
    panic!("batch did not advance");
}

fn run(doc: &Document, view: &mut FoldView, action: FoldBatchAction) -> FoldBatchProgress {
    let mut task = FoldBatch::new(doc, view, action).unwrap();
    finish(&mut task, doc, view)
}

#[test]
fn all_subtree_level_and_focus_preserve_precise_local_choices() {
    let doc = document("# A\na\n### B\nb\n```rs\ncode\n```\n## C\nc\n# D\nd\n# Empty\n");
    assert_eq!(doc.tile_count(), 1);
    let all = candidates(&doc);
    let code = all
        .iter()
        .find(|c| matches!(c.id, FoldId::Code(_)))
        .unwrap()
        .id;
    let mut view = FoldView::new(&doc, 1).unwrap();
    let other = FoldView::new(&doc, 1).unwrap();
    let progress = run(&doc, &mut view, FoldBatchAction::All { collapsed: true });
    assert_eq!(progress.visited, doc.blocks().len());
    assert_eq!(progress.changed, all.len());
    assert!(
        all.iter()
            .all(|c| view.is_collapsed(c.id) && !other.is_collapsed(c.id))
    );

    run(
        &doc,
        &mut view,
        FoldBatchAction::Subtree {
            root: heading(1),
            collapsed: false,
        },
    );
    assert!(!view.is_collapsed(heading(1)));
    assert!(!view.is_collapsed(code));
    assert!(view.is_collapsed(heading(0)));
    assert!(view.is_collapsed(heading(2)));
    run(
        &doc,
        &mut view,
        FoldBatchAction::AtLevel {
            level: 1,
            collapsed: false,
        },
    );
    for id in [0, 3, 4] {
        assert!(!view.is_collapsed(heading(id)));
    }
    assert!(view.is_collapsed(heading(2)));
    run(&doc, &mut view, FoldBatchAction::Focus { target: code });
    for c in &all {
        assert_eq!(
            view.is_collapsed(c.id),
            ![heading(0), heading(1), code].contains(&c.id)
        );
    }
    run(
        &doc,
        &mut view,
        FoldBatchAction::Focus { target: heading(0) },
    );
    assert!(!view.is_collapsed(heading(0)));
    assert!(view.is_collapsed(heading(1)));
    assert!(view.is_collapsed(code));
    let progress = run(
        &doc,
        &mut view,
        FoldBatchAction::Subtree {
            root: code,
            collapsed: false,
        },
    );
    assert_eq!(progress.visited, 1);
    assert_eq!(progress.changed, 1);
    assert!(!view.is_collapsed(code));
}

#[test]
fn every_block_is_budgeted_and_cancellation_keeps_partial_results() {
    let doc = document(&format!("{}# A\na\n# B\nb\n", "- plain\n".repeat(2000)));
    let mut view = FoldView::new(&doc, 1).unwrap();
    let mut task = FoldBatch::new(&doc, &view, FoldBatchAction::All { collapsed: true }).unwrap();
    for limit in [
        WorkLimit::default(),
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
        let progress = task.advance(&doc, &mut view, &mut budget).unwrap();
        assert_eq!(progress.visited, 0);
        assert_eq!(progress.change, None);
        assert_eq!(budget.used(), WorkLimit::default());
    }
    let init = task.advance(&doc, &mut view, &mut tiny()).unwrap();
    assert_eq!(init.visited, 0);
    for step in 1..=2000 {
        let progress = task.advance(&doc, &mut view, &mut tiny()).unwrap();
        assert_eq!(progress.visited, step);
        assert_eq!(progress.changed, 0);
    }
    let progress = task.advance(&doc, &mut view, &mut tiny()).unwrap();
    assert_eq!(progress.changed, 1);
    assert!(progress.change.is_some());
    assert_eq!(progress.completion, Completion::BudgetExhausted);
    // Stop advancing this task; its changes are already visible.
    assert!(view.is_collapsed(heading(0)));
    assert!(!view.is_collapsed(heading(1)));
    let version = view.version();
    let no_op = run(
        &doc,
        &mut view,
        FoldBatchAction::AtLevel {
            level: 6,
            collapsed: true,
        },
    );
    assert_eq!(no_op.changed, 0);
    assert_eq!(view.version(), version);
}

#[test]
fn identity_staleness_and_invalid_actions_fail_before_mutation_or_budget() {
    let mut doc = Document::new();
    doc.append("# A\na\n# B\nb\n").unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let action = FoldBatchAction::All { collapsed: true };
    for invalid in [
        FoldBatchAction::AtLevel {
            level: 0,
            collapsed: true,
        },
        FoldBatchAction::AtLevel {
            level: 7,
            collapsed: true,
        },
        FoldBatchAction::Subtree {
            root: heading(99),
            collapsed: true,
        },
        FoldBatchAction::Focus {
            target: FoldId::Code(BlockId(0)),
        },
    ] {
        assert!(matches!(
            FoldBatch::new(&doc, &view, invalid),
            Err(LayoutError::InvalidRange)
        ));
    }
    let mut task = FoldBatch::new(&doc, &view, action).unwrap();
    let mut other_view = FoldView::new(&doc, 1).unwrap();
    let other_doc = document("# A\na\n# B\nb\n");
    let mut budget = tiny();
    assert_eq!(
        task.advance(&doc, &mut other_view, &mut budget),
        Err(LayoutError::WrongView)
    );
    assert_eq!(
        task.advance(&other_doc, &mut view, &mut budget),
        Err(LayoutError::WrongDocument)
    );
    assert_eq!(budget.used(), WorkLimit::default());
    for operation in 0..5 {
        let mut task = FoldBatch::new(&doc, &view, action).unwrap();
        match operation {
            0 => {
                view.toggle(&doc, heading(0)).unwrap();
            }
            1 => {
                view.reset(2).unwrap();
            }
            2 => {
                view.set_defaults(FoldDefaults {
                    code: true,
                    ..Default::default()
                });
            }
            3 => {
                let update = doc.append("more\n").unwrap();
                view.apply(&doc, &update).unwrap();
            }
            _ => {
                let update = doc.finish();
                view.apply(&doc, &update).unwrap();
            }
        }
        let version = view.version();
        let mut budget = tiny();
        assert_eq!(
            task.advance(&doc, &mut view, &mut budget),
            Err(LayoutError::StaleContinuation)
        );
        assert_eq!(budget.used(), WorkLimit::default());
        assert_eq!(view.version(), version);
    }
}

#[test]
fn changes_merge_and_measurements_between_steps_do_not_invalidate_batch() {
    let doc = document(&format!(
        "# A\n{}# B\n{}",
        "body\n".repeat(300),
        "tail\n".repeat(300)
    ));
    let mut view = FoldView::new(&doc, 1).unwrap();
    let old_ticket = view.heights().ticket(TileId(0)).unwrap();
    let mut task = FoldBatch::new(&doc, &view, FoldBatchAction::All { collapsed: true }).unwrap();
    task.advance(&doc, &mut view, &mut tiny()).unwrap();
    task.advance(&doc, &mut view, &mut tiny()).unwrap();
    assert_eq!(
        view.measure(old_ticket, 10),
        Err(LayoutError::StaleMeasurement)
    );
    let ticket = view.heights().ticket(TileId(0)).unwrap();
    view.measure(ticket, 4).unwrap();
    finish(&mut task, &doc, &mut view);
    let mut task = FoldBatch::new(&doc, &view, FoldBatchAction::All { collapsed: false }).unwrap();
    let progress = task
        .advance(&doc, &mut view, &mut Budget::new(WorkLimit::unlimited()))
        .unwrap();
    assert_eq!(progress.changed, 2);
    assert_eq!(
        progress.change.unwrap().source,
        Span::new(4, doc.source().len())
    );
    assert_eq!(progress.change.unwrap().version, view.version());
}

fn assert_model(doc: &Document, view: &FoldView, all: &[FoldCandidate], collapsed: &[bool]) {
    for (candidate, &expected) in all.iter().zip(collapsed) {
        assert_eq!(view.is_collapsed(candidate.id), expected);
    }
    for tile in 0..doc.tile_count() {
        let masked = all.iter().zip(collapsed).any(|(candidate, &closed)| {
            if !closed || candidate.hidden.is_empty() {
                return false;
            }
            let first = doc.tile_at(candidate.hidden.start).unwrap().0;
            let end = doc
                .tile_at(candidate.hidden.end)
                .map_or(doc.tile_count(), |t| t.0);
            first < tile && tile < end
        });
        assert_eq!(
            view.heights().height(TileId(tile)).unwrap(),
            if masked {
                0
            } else {
                view.heights().base_height(TileId(tile)).unwrap()
            }
        );
    }
    for offset in (0..doc.source().len()).step_by(113) {
        let hidden = all
            .iter()
            .zip(collapsed)
            .any(|(c, &closed)| closed && c.hidden.range().contains(&offset));
        assert_eq!(
            matches!(
                view.hidden_at(doc, offset, &mut tiny()).unwrap(),
                FoldLookup::Hidden(_)
            ),
            hidden
        );
    }
}

#[test]
fn seeded_batches_match_parent_chain_and_visible_interval_models() {
    let mut seed = 0xface_cafe_u64;
    let mut random = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        (seed >> 32) as usize
    };
    let mut text = String::new();
    for _ in 0..50 {
        text.push_str(&format!(
            "{} title\n{}\n```\ncode\n```\n",
            "#".repeat(random() % 6 + 1),
            "body".repeat(90)
        ));
    }
    let doc = document(&text);
    let all = candidates(&doc);
    let mut expected = vec![false; all.len()];
    let mut view = FoldView::new(&doc, 2).unwrap();
    for round in 0..100 {
        let target = all[random() % all.len()].id;
        let collapsed = random() % 2 == 0;
        let action = match random() % 4 {
            0 => FoldBatchAction::All { collapsed },
            1 => FoldBatchAction::Subtree {
                root: target,
                collapsed,
            },
            2 => FoldBatchAction::AtLevel {
                level: (random() % 6 + 1) as u8,
                collapsed,
            },
            _ => FoldBatchAction::Focus { target },
        };
        let ancestry = |id| {
            let mut path = vec![id];
            let mut parent = all.iter().find(|c| c.id == id).unwrap().parent;
            while let Some(id) = parent {
                path.push(id);
                parent = all.iter().find(|c| c.id == id).unwrap().parent;
            }
            path
        };
        for (candidate, state) in all.iter().zip(&mut expected) {
            match action {
                FoldBatchAction::All { collapsed } => *state = collapsed,
                FoldBatchAction::Subtree { root, collapsed }
                    if ancestry(candidate.id).contains(&root) =>
                {
                    *state = collapsed
                }
                FoldBatchAction::AtLevel { level, collapsed } => {
                    if let FoldId::Heading(id) = candidate.id
                        && doc.headings().get(id).unwrap().level == level
                    {
                        *state = collapsed;
                    }
                }
                FoldBatchAction::Focus { target } => {
                    *state = !ancestry(target).contains(&candidate.id)
                }
                _ => {}
            }
        }
        run(&doc, &mut view, action);
        if round % 7 == 0 {
            view.reset(3).unwrap();
        }
        assert_model(&doc, &view, &all, &expected);
    }
}

#[test]
fn million_byte_subtree_and_many_candidates_use_bounded_steps() {
    let doc = document(&format!(
        "# Root\n```\n{}\n```\n# Next\n",
        "x".repeat(1_000_000)
    ));
    let mut view = FoldView::new(&doc, 1).unwrap();
    let before = doc.work();
    let mutations = view.heights().mutation_visits();
    let progress = run(
        &doc,
        &mut view,
        FoldBatchAction::Subtree {
            root: heading(0),
            collapsed: true,
        },
    );
    assert_eq!(progress.visited, 2);
    assert_eq!(progress.changed, 2);
    assert!(view.heights().mutation_visits() - mutations < 4000);
    assert_eq!(doc.work(), before);

    let doc = document(&"# title\n".repeat(20_000));
    let mut view = FoldView::new(&doc, 1).unwrap();
    let mut task = FoldBatch::new(&doc, &view, FoldBatchAction::All { collapsed: true }).unwrap();
    let mut budget = Budget::new(WorkLimit {
        queries: 4,
        objects: 4,
        ..Default::default()
    });
    let progress = task.advance(&doc, &mut view, &mut budget).unwrap();
    assert_eq!(progress.visited, 3);
    assert_eq!(progress.changed, 3);
    assert_eq!(progress.completion, Completion::BudgetExhausted);
    assert_eq!(finish(&mut task, &doc, &mut view).changed, 20_000);
}

#[test]
fn defaults_initialize_new_nodes_once_and_explicit_operations_win() {
    let mut doc = Document::new();
    doc.append("# Existing\nold\n").unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let defaults = FoldDefaults {
        heading_levels: [true; 6],
        code: true,
    };
    view.set_defaults(defaults);
    assert_eq!(view.defaults(), defaults);
    let version = view.version();
    view.set_defaults(defaults);
    assert_eq!(view.version(), version);
    assert!(!view.is_collapsed(heading(0)));
    let update = doc.append("## New\nbody\n```rs\ncode\n").unwrap();
    view.apply(&doc, &update).unwrap();
    let code = candidates(&doc)
        .into_iter()
        .find(|c| matches!(c.id, FoldId::Code(_)))
        .unwrap()
        .id;
    assert!(view.is_collapsed(heading(1)));
    assert!(view.is_collapsed(code));
    run(&doc, &mut view, FoldBatchAction::All { collapsed: false });
    let visits = view.default_visits();
    for ch in "more code\n".chars() {
        let update = doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
        view.apply(&doc, &update).unwrap();
        assert!(!view.is_collapsed(heading(1)));
        assert!(!view.is_collapsed(code));
    }
    assert_eq!(view.default_visits(), visits);
    let update = doc.append("```\n# Later\nbody\n").unwrap();
    view.apply(&doc, &update).unwrap();
    assert!(view.is_collapsed(heading(2)));
    let visits = view.default_visits();
    let version = view.version();
    view.apply(&doc, &update).unwrap();
    assert_eq!(view.default_visits(), visits);
    assert_eq!(view.version(), version);
    view.set_defaults(FoldDefaults::default());
    assert!(view.is_collapsed(heading(2)));
    let update = doc.append("# Expanded\n").unwrap();
    view.apply(&doc, &update).unwrap();
    assert!(!view.is_collapsed(heading(3)));
}

#[test]
fn defaults_handle_setext_finish_batch_updates_and_all_character_boundaries() {
    let text = "# A\na\nlate 界\n---\nbody\n```rs\ncode\n```\n# B\nb\n    code\n";
    for chunked in [false, true] {
        let mut doc = Document::new();
        let mut view = FoldView::new(&doc, 2).unwrap();
        view.set_defaults(FoldDefaults {
            heading_levels: [true, true, false, false, false, false],
            code: true,
        });
        let mut merged = Update::default();
        for ch in text.chars() {
            let update = doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            if chunked {
                view.apply(&doc, &update).unwrap();
            } else {
                merged.merge(update).unwrap();
            }
            if chunked {
                let all = candidates(&doc);
                assert_model(&doc, &view, &all, &vec![true; all.len()]);
            }
        }
        let update = doc.finish();
        if chunked {
            view.apply(&doc, &update).unwrap();
        } else {
            merged.merge(update).unwrap();
            view.apply(&doc, &merged).unwrap();
        }
        let all = candidates(&doc);
        assert_model(&doc, &view, &all, &vec![true; all.len()]);
        assert_eq!(
            view.default_visits(),
            (doc.blocks().len() + doc.headings().entries().len()) as u64
        );
    }
}

#[test]
fn default_growth_does_not_revisit_historical_candidates_or_long_setext_text() {
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 1).unwrap();
    view.set_defaults(FoldDefaults {
        heading_levels: [true; 6],
        code: true,
    });
    let update = doc
        .append(&format!(
            "{}{}\n",
            "# history\nbody\n".repeat(10_000),
            "x".repeat(1_000_000)
        ))
        .unwrap();
    view.apply(&doc, &update).unwrap();
    let visits = view.default_visits();
    let ranges = view.range_visits();
    let update = doc.append("---\n").unwrap();
    let before = doc.work();
    view.apply(&doc, &update).unwrap();
    assert_eq!(doc.work(), before);
    assert_eq!(view.default_visits() - visits, 1);
    assert!(view.range_visits() - ranges <= 7);
    assert!(view.is_collapsed(heading(10_000)));
}

#[test]
fn empty_batch_finishes_and_default_heading_levels_are_selective() {
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let progress = run(&doc, &mut view, FoldBatchAction::All { collapsed: true });
    assert_eq!(progress.visited, 0);
    assert_eq!(progress.changed, 0);
    view.set_defaults(FoldDefaults {
        heading_levels: [false, true, false, true, false, true],
        code: false,
    });
    let mut text = String::new();
    for level in 1..=6 {
        text.push_str(&format!("{} heading\nbody\n", "#".repeat(level)));
    }
    text.push_str("```\ncode\n```");
    let update = doc.append(&text).unwrap();
    view.apply(&doc, &update).unwrap();
    let update = doc.finish();
    view.apply(&doc, &update).unwrap();
    for id in 0..6 {
        assert_eq!(view.is_collapsed(heading(id)), id % 2 == 1);
    }
    let code = candidates(&doc)
        .into_iter()
        .find(|c| matches!(c.id, FoldId::Code(_)))
        .unwrap();
    assert!(!view.is_collapsed(code.id));
}
