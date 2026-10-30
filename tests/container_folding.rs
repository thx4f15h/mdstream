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
fn container(id: usize) -> FoldId {
    FoldId::Container(ContainerId(id))
}
fn tiny() -> Budget {
    Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        ..Default::default()
    })
}

fn all(doc: &Document) -> Vec<FoldCandidate> {
    let mut cursor = doc
        .fold_candidates(Span::new(0, doc.source().len()))
        .unwrap();
    let mut result = Vec::new();
    let mut out = Vec::new();
    loop {
        let mut budget = tiny();
        let done = cursor.advance(doc, &mut budget, &mut out).unwrap();
        assert!(out.len() <= 1);
        result.extend_from_slice(&out);
        if done == Completion::Covered {
            return result;
        }
    }
}

fn batch(doc: &Document, view: &mut FoldView, action: FoldBatchAction) -> FoldBatchProgress {
    let mut task = FoldBatch::new(doc, view, action).unwrap();
    for _ in 0..doc.blocks().len() + doc.container_count() + 2 {
        let progress = task.advance(doc, view, &mut tiny()).unwrap();
        if progress.completion == Completion::Covered {
            return progress;
        }
    }
    panic!("batch did not finish");
}

// Independent interval union from parser metadata and local view intentions.
// Does not use fold_candidate, FoldState, or any height/visibility index.
fn ranges(doc: &Document, ids: &[FoldId]) -> Vec<(Span, FoldId)> {
    let mut ranges: Vec<_> = ids
        .iter()
        .map(|&id| {
            let span = match id {
                FoldId::Container(id) => {
                    let c = doc.container(id).unwrap();
                    Span::new(
                        c.source.start,
                        if c.state == SyntaxState::Sealed {
                            c.source.end
                        } else {
                            doc.source().len()
                        },
                    )
                }
                FoldId::Heading(id) => {
                    let h = doc.headings().get(id).unwrap();
                    let end = doc.headings().entries()[id.0 + 1..]
                        .iter()
                        .find(|n| n.container.is_none() && n.level <= h.level)
                        .map_or(doc.source().len(), |n| n.source.start);
                    Span::new(h.source.end, end)
                }
                FoldId::Table(id) => {
                    let table = doc.table(id).unwrap();
                    let open = doc.block(table.block).unwrap().state != SyntaxState::Sealed;
                    Span::new(
                        table.source.start,
                        if open {
                            doc.source().len()
                        } else {
                            table.source.end
                        },
                    )
                }
                FoldId::Code(_) => unreachable!(),
            };
            (span, id)
        })
        .filter(|(s, _)| !s.is_empty())
        .collect();
    ranges.sort_by_key(|&(s, id)| {
        let tie = match id {
            FoldId::Heading(_) => (0, 0),
            FoldId::Container(id) => (1, id.0),
            _ => (2, 0),
        };
        (s.start, std::cmp::Reverse(s.end), tie)
    });
    let mut end = 0;
    ranges.retain(|(s, _)| {
        if s.start < end {
            false
        } else {
            end = s.end;
            true
        }
    });
    ranges
}

fn oracle(doc: &Document, ids: &[FoldId]) -> String {
    let mut text = String::new();
    let mut at = 0;
    for (span, _) in ranges(doc, ids) {
        for run in doc.runs_in(Span::new(at, span.start)).unwrap() {
            if !run.style.contains(Style::HIDDEN) {
                text.push_str(run.text);
            }
        }
        text.push_str("[fold]\n");
        at = span.end;
    }
    for run in doc.runs_in(Span::new(at, doc.source().len())).unwrap() {
        if !run.style.contains(Style::HIDDEN) {
            text.push_str(run.text);
        }
    }
    text
}

#[test]
fn discovery_parents_ranges_summaries_and_equal_opener_order() {
    let doc = document("# Root\n> - 界|b\n>   -|-\n>   x|y\n> - second\nout\n");
    let found = all(&doc);
    assert_eq!(
        found.iter().map(|c| c.id).collect::<Vec<_>>(),
        [
            FoldId::Heading(HeadingId(0)),
            container(0),
            container(1),
            container(2),
            FoldId::Table(TableId(0)),
            container(3),
        ]
    );
    for (id, parent) in [
        (container(0), FoldId::Heading(HeadingId(0))),
        (container(1), container(0)),
        (container(2), container(1)),
        (container(3), container(1)),
        (FoldId::Table(TableId(0)), container(2)),
    ] {
        assert_eq!(doc.fold_candidate(id).unwrap().parent, Some(parent));
    }
    let c = doc.fold_candidate(container(2)).unwrap();
    assert_eq!(c.source, doc.container(ContainerId(2)).unwrap().source);
    assert_eq!(c.hidden, c.source);
    assert!(!c.open);
    let mut budget = Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 5,
        ..Default::default()
    });
    assert_eq!(c.summary_text(&doc, 5, &mut budget), Some("> - "));
    assert_eq!(budget.used().bytes, 4);
    assert_eq!(c.summary_text(&doc, 5, &mut budget), None);
    let offset = doc.source().find("second").unwrap();
    assert_eq!(doc.fold_candidate_at(offset).unwrap().id, container(3));
    let mut cursor = doc.fold_candidates(Span::new(offset, offset + 1)).unwrap();
    let mut out = Vec::new();
    cursor
        .advance(&doc, &mut Budget::new(WorkLimit::unlimited()), &mut out)
        .unwrap();
    assert!(out.is_empty()); // No repeated enclosing container openers.
    assert!(doc.fold_candidate(container(999)).is_none());
}

#[test]
fn nested_choices_compose_with_headings_tables_preview_and_independent_views() {
    let doc = document("# Root\n> - a|b\n>   -|-\n>   ![](/image)|y\n> - second\n# Next\ntail\n");
    let mut view = FoldView::new(&doc, 1).unwrap();
    let mut other = FoldView::new(&doc, 1).unwrap();
    let table = FoldId::Table(TableId(0));
    for id in [
        table,
        container(2),
        container(0),
        FoldId::Heading(HeadingId(0)),
    ] {
        view.set_collapsed(&doc, id, true).unwrap();
    }
    let ticket = view.heights().ticket(TileId(0)).unwrap();
    view.set_collapsed(&doc, FoldId::Heading(HeadingId(0)), false)
        .unwrap();
    assert!(view.measure(ticket, 100).is_err());
    let sink = render(&doc, &mut view, true);
    assert_eq!(sink.text, "Root\n[fold]\nNext\ntail\n");
    assert_eq!(sink.placeholders[0].candidate.id, container(0));
    assert_eq!(sink.images, 0);
    assert_eq!(render(&doc, &mut other, true).images, 1);
    view.set_preview(&doc, Some(container(0)), &mut tiny())
        .unwrap();
    let sink = render(&doc, &mut view, true);
    assert_eq!(sink.text, "Root\n[fold]\nsecond\nNext\ntail\n");
    assert_eq!(sink.placeholders[0].candidate.id, container(2));
    view.set_collapsed(&doc, container(2), false).unwrap();
    assert_eq!(
        render(&doc, &mut view, true).placeholders[0].candidate.id,
        table
    );
    view.reset(4).unwrap();
    view.set_preview(&doc, None, &mut tiny()).unwrap();
    assert!(view.is_collapsed(table));
    assert_eq!(
        render(&doc, &mut view, true).text,
        "Root\n[fold]\nNext\ntail\n"
    );
    assert_eq!(view.heights().total_rows(), 4);
}

#[test]
fn subtree_focus_and_siblings_distinguish_equal_list_item_and_table_ranges() {
    let doc = document("> - a|b\n>   -|-\n>   x|y\n");
    let mut view = FoldView::new(&doc, 1).unwrap();
    let table = FoldId::Table(TableId(0));
    let p = batch(
        &doc,
        &mut view,
        FoldBatchAction::Subtree {
            root: container(2),
            collapsed: true,
        },
    );
    assert_eq!(p.changed, 2);
    assert!(!view.is_collapsed(container(0)));
    assert!(!view.is_collapsed(container(1)));
    assert!(view.is_collapsed(table));
    batch(&doc, &mut view, FoldBatchAction::All { collapsed: true });
    batch(
        &doc,
        &mut view,
        FoldBatchAction::Focus {
            target: container(1),
        },
    );
    assert!(!view.is_collapsed(container(0)));
    assert!(!view.is_collapsed(container(1)));
    assert!(view.is_collapsed(container(2)));
    assert!(view.is_collapsed(table));
    batch(
        &doc,
        &mut view,
        FoldBatchAction::Subtree {
            root: table,
            collapsed: false,
        },
    );
    assert!(view.is_collapsed(container(2)));
    assert!(!view.is_collapsed(table));

    let doc = document("- a\n  > child\n- b\n- c\n");
    let mut view = FoldView::new(&doc, 1).unwrap();
    batch(
        &doc,
        &mut view,
        FoldBatchAction::Siblings {
            target: container(1),
            collapsed: true,
        },
    );
    for id in 0..doc.container_count() {
        let node = doc.container(ContainerId(id)).unwrap();
        assert_eq!(
            view.is_collapsed(container(id)),
            id != 1 && node.parent == Some(ContainerId(0))
        );
    }
}

#[test]
fn every_utf8_split_and_prefix_updates_masks_and_retracts_pending_siblings() {
    let source = "# R\n> - 界|b\n>   -|-\n>   🙂|y\n> - sibling\nout\n# End\n";
    let parse = |chunks: Vec<&str>, merged: bool| {
        let mut doc = Document::new();
        let mut view = FoldView::new(&doc, 1).unwrap();
        let mut update_batch = Update::default();
        for chunk in chunks {
            let update = doc.append(chunk).unwrap();
            if merged {
                update_batch.merge(update).unwrap();
                continue;
            }
            view.apply(&doc, &update).unwrap();
            for i in update.new_containers {
                view.set_collapsed(&doc, container(i), true).unwrap();
            }
            for i in update.new_tables {
                view.set_collapsed(&doc, FoldId::Table(TableId(i)), true)
                    .unwrap();
            }
            let ids: Vec<_> = (0..doc.container_count())
                .map(container)
                .chain((0..doc.tables().len()).map(|i| FoldId::Table(TableId(i))))
                .collect();
            assert_eq!(render(&doc, &mut view, true).text, oracle(&doc, &ids));
            for offset in 0..doc.source().len() {
                let expected = ranges(&doc, &ids)
                    .into_iter()
                    .find(|(s, _)| s.range().contains(&offset));
                match view.hidden_at(&doc, offset, &mut tiny()).unwrap() {
                    FoldLookup::Hidden(c) => assert_eq!(Some(c.id), expected.map(|(_, id)| id)),
                    FoldLookup::Visible => assert!(expected.is_none()),
                    _ => panic!("one query must cover visibility"),
                }
            }
        }
        let finish = doc.finish();
        if merged {
            update_batch.merge(finish).unwrap();
            view.apply(&doc, &update_batch).unwrap();
            batch(&doc, &mut view, FoldBatchAction::All { collapsed: true });
            for i in 0..doc.headings().entries().len() {
                view.set_collapsed(&doc, FoldId::Heading(HeadingId(i)), false)
                    .unwrap();
            }
        } else {
            view.apply(&doc, &finish).unwrap();
        }
        let text = render(&doc, &mut view, true).text;
        view.reset(3).unwrap();
        assert_eq!(render(&doc, &mut view, true).text, text);
        text
    };
    let expected = parse(vec![source], false);
    assert_eq!(expected, "R\n[fold]\nout\nEnd\n");
    for (at, _) in source
        .char_indices()
        .chain(std::iter::once((source.len(), '\0')))
    {
        for merged in [false, true] {
            assert_eq!(parse(vec![&source[..at], &source[at..]], merged), expected);
        }
    }
    assert_eq!(
        parse(
            source
                .char_indices()
                .map(|(i, c)| &source[i..i + c.len_utf8()])
                .collect(),
            false
        ),
        expected
    );
}

#[test]
fn deep_open_chain_has_one_tail_update_and_no_per_ancestor_append_work() {
    let mut doc = Document::new();
    doc.append(&format!("{}{}", ">".repeat(1000), "x".repeat(1_000_000)))
        .unwrap();
    assert_eq!(doc.container_count(), 1000);
    let mut view = FoldView::new(&doc, 1).unwrap();
    batch(&doc, &mut view, FoldBatchAction::All { collapsed: true });
    let before = doc.work();
    let visits = view.range_visits();
    let mutations = view.container_index_visits();
    let height_visits = view.heights().mutation_visits();
    for _ in 0..1000 {
        let update = doc.append("x").unwrap();
        view.apply(&doc, &update).unwrap();
    }
    assert_eq!(view.range_visits() - visits, 1000);
    assert_eq!(view.container_index_visits(), mutations);
    assert!(view.heights().mutation_visits() - height_visits < 500_000);
    assert_eq!(doc.work().container_steps, before.container_steps);
    assert_eq!(doc.work().heading_steps, before.heading_steps);
    let mut pass = LayoutPass::folded(&doc, &view, TileId(0), 20).unwrap();
    let mut budget = Budget::new(WorkLimit {
        queries: 16,
        tiles: 2,
        bytes: 64,
        objects: 4,
        ..Default::default()
    });
    let mut sink = Sink::default();
    assert_eq!(
        pass.advance_folded(&doc, &mut view, &mut budget, &mut sink)
            .unwrap(),
        Completion::EndOfDocument
    );
    assert_eq!(budget.used().bytes, 0);
    assert_eq!(sink.text, "[fold]\n");
    assert_eq!(sink.bytes, 0);
    assert_eq!(sink.tiles, 1);
    assert_eq!(
        sink.placeholders[0].candidate.source.end,
        doc.source().len()
    );
    let work = view.container_index_visits();
    assert!(
        matches!(view.hidden_at(&doc, doc.source().len()-1, &mut tiny()).unwrap(), FoldLookup::Hidden(c) if c.id == container(0))
    );
    assert!(view.container_index_visits() - work <= 24);
    let visits = view.range_visits();
    let update = doc.append("\noutside\n").unwrap();
    view.apply(&doc, &update).unwrap();
    assert_eq!(update.closed_containers.len(), 1000);
    assert_eq!(view.range_visits() - visits, 1001);
    assert_eq!(render(&doc, &mut view, true).text, "[fold]\noutside\n");
    assert!(view.container_index_nodes() < 4000);
    for i in 0..999 {
        view.set_collapsed(&doc, container(i), false).unwrap();
    }
    let sink = render(&doc, &mut view, true);
    assert_eq!(sink.placeholders[0].candidate.id, container(999));
    assert_eq!(sink.text, "[fold]\noutside\n");
}

#[test]
fn reveal_deep_path_pays_for_each_node_and_anchor_recovers_original_source() {
    let doc = document(&format!(
        "{}# inner\n{}body\n# End\n",
        ">".repeat(200),
        ">".repeat(200)
    ));
    let mut view = FoldView::new(&doc, 1).unwrap();
    for i in 0..doc.container_count() {
        view.set_collapsed(&doc, container(i), true).unwrap();
    }
    let source = doc.source().find("body").unwrap();
    let anchor = FoldAnchor::new(&doc, source).unwrap();
    assert_eq!(
        anchor
            .resolve(&doc, &view, 10, &mut tiny())
            .unwrap()
            .unwrap()
            .hidden_by,
        Some(container(0))
    );
    let mut task = FoldNavigation::new(
        &doc,
        &view,
        &Navigator::new(),
        NavAction::Source(source),
        FoldNavigationPolicy::Reveal,
        10,
    )
    .unwrap();
    let mut opened = 0;
    let mut done = false;
    for _ in 0..204 {
        let before = view.container_index_visits();
        let result = task.advance(&doc, &mut view, &mut tiny()).unwrap();
        assert!(view.container_index_visits() - before < 50);
        opened += usize::from(result.change.is_some());
        if result.completion == Completion::Covered {
            assert_eq!(result.target.unwrap().source, Some(source));
            done = true;
            break;
        }
    }
    assert!(done);
    assert_eq!(opened, 200);
    let target = anchor
        .resolve(&doc, &view, 10, &mut tiny())
        .unwrap()
        .unwrap();
    assert_eq!(target.hidden_by, None);
    assert_eq!(target.target.source, Some(source));
    view.set_collapsed(&doc, container(0), true).unwrap();
    let mut task = FoldNavigation::new(
        &doc,
        &view,
        &Navigator::new(),
        NavAction::NextHeading { max_level: 6 },
        FoldNavigationPolicy::VisibleOnly,
        10,
    )
    .unwrap();
    let result = task
        .advance(&doc, &mut view, &mut Budget::new(WorkLimit::unlimited()))
        .unwrap();
    assert_eq!(
        result.target.unwrap().source,
        Some(doc.source().find("# End").unwrap())
    );
    assert!(view.is_collapsed(container(0)));
}

#[test]
fn preview_survives_growth_close_finish_and_replay_without_stale_masks() {
    let mut doc = Document::new();
    doc.append("- first\n  > child\n").unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    for i in 0..3 {
        view.set_collapsed(&doc, container(i), true).unwrap();
    }
    view.set_preview(&doc, Some(container(0)), &mut tiny())
        .unwrap();
    let mut merged = doc.append("  > more\n").unwrap();
    merged.merge(doc.append("- next\n").unwrap()).unwrap();
    merged.merge(doc.finish()).unwrap();
    view.apply(&doc, &merged).unwrap();
    assert_eq!(render(&doc, &mut view, true).text, "[fold]\nnext\n");
    view.reset(3).unwrap();
    view.set_preview(&doc, None, &mut tiny()).unwrap();
    assert_eq!(render(&doc, &mut view, true).text, "[fold]\n");
    let version = view.version();
    let visits = view.container_index_visits();
    view.apply(&doc, &merged).unwrap();
    assert_eq!(view.version(), version);
    assert_eq!(view.container_index_visits(), visits);
    view.set_collapsed(&doc, container(0), false).unwrap();
    view.set_collapsed(&doc, container(1), false).unwrap();
    assert_eq!(render(&doc, &mut view, true).text, "first\n[fold]\nnext\n");
}

#[test]
fn seeded_toggle_preview_and_resize_model_checks_heights_and_lookup() {
    let source = (0..30)
        .map(|i| {
            format!(
                "# H{i}\n> - a|b\n>   -|-\n>   {}|y\n> - next\n>   > child\nout\n",
                "x\n>   ".repeat(5)
            )
        })
        .collect::<String>();
    let doc = document(&source);
    let ids: Vec<_> = all(&doc).iter().map(|c| c.id).collect();
    let mut chosen = std::collections::BTreeSet::new();
    let mut preview = None;
    let mut view = FoldView::new(&doc, 1).unwrap();
    let mut seed = 0x719cab31u64;
    for step in 0..2000 {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let id = ids[seed as usize % ids.len()];
        match step % 13 {
            0 => {
                preview = Some(id);
                view.set_preview(&doc, preview, &mut tiny()).unwrap();
            }
            1 => {
                preview = None;
                view.set_preview(&doc, None, &mut tiny()).unwrap();
            }
            2 => view.reset(step % 5 + 1).unwrap(),
            _ => {
                if !chosen.insert(id) {
                    chosen.remove(&id);
                }
                view.toggle(&doc, id).unwrap();
            }
        }
        let effective: Vec<_> = chosen
            .iter()
            .copied()
            .filter(|&id| Some(id) != preview)
            .collect();
        let offset = seed as usize % doc.source().len();
        let expected = ranges(&doc, &effective)
            .into_iter()
            .find(|(s, _)| s.range().contains(&offset));
        match view.hidden_at(&doc, offset, &mut tiny()).unwrap() {
            FoldLookup::Hidden(c) => assert_eq!(Some(c.id), expected.map(|(_, id)| id)),
            FoldLookup::Visible => assert!(expected.is_none()),
            _ => panic!("budget"),
        }
        if step % 17 == 0 {
            let sink = render(&doc, &mut view, false);
            assert_eq!(sink.text, oracle(&doc, &effective), "step {step}");
            assert_eq!(
                view.heights().total_rows(),
                sink.text.bytes().filter(|&b| b == b'\n').count() as u128
            );
        }
    }
    assert!(view.container_index_nodes() < 4 * doc.container_count());
}

#[test]
fn thousands_of_closed_siblings_have_logarithmic_lookup_and_bounded_storage() {
    let doc = document(&"> item\noutside\n".repeat(4096));
    assert_eq!(doc.container_count(), 4096);
    let mut view = FoldView::new(&doc, 1).unwrap();
    batch(&doc, &mut view, FoldBatchAction::All { collapsed: true });
    let nodes = view.container_index_nodes();
    assert!(nodes <= 8191);
    for i in 0..4096 {
        let c = doc.container(ContainerId(i)).unwrap();
        let before = view.container_index_visits();
        assert!(
            matches!(view.hidden_at(&doc, c.source.start, &mut tiny()).unwrap(), FoldLookup::Hidden(c) if c.id == container(i))
        );
        assert_eq!(
            view.hidden_at(&doc, c.source.end, &mut tiny()).unwrap(),
            FoldLookup::Visible
        );
        assert!(view.container_index_visits() - before < 80);
        view.toggle(&doc, container(i)).unwrap();
        view.toggle(&doc, container(i)).unwrap();
    }
    assert_eq!(view.container_index_nodes(), nodes);
}

#[test]
fn invalid_and_stale_operations_do_not_charge_or_mutate_view() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<FoldView>();
    let mut doc = document("> x\n");
    let mut view = FoldView::new(&doc, 1).unwrap();
    view.set_collapsed(&doc, container(0), true).unwrap();
    let version = view.version();
    let mut budget = Budget::new(WorkLimit::default());
    assert_eq!(
        view.hidden_at(&doc, 1, &mut budget).unwrap(),
        FoldLookup::BudgetExhausted
    );
    assert_eq!(
        view.set_preview(&doc, Some(container(0)), &mut budget)
            .unwrap()
            .completion,
        Completion::BudgetExhausted
    );
    assert_eq!(
        view.set_preview(&doc, Some(container(9)), &mut budget),
        Err(LayoutError::InvalidRange)
    );
    assert_eq!(view.version(), version);
    assert_eq!(budget.used(), WorkLimit::default());
    doc = Document::new();
    doc.append("> x\n").unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let mut candidates = doc
        .fold_candidates(Span::new(0, doc.source().len()))
        .unwrap();
    let mut task = FoldBatch::new(&doc, &view, FoldBatchAction::All { collapsed: true }).unwrap();
    let mut pass = LayoutPass::folded(&doc, &view, TileId(0), 10).unwrap();
    let update = doc.append("> y\n").unwrap();
    assert_eq!(
        view.hidden_at(&doc, 0, &mut budget),
        Err(LayoutError::OutOfOrderUpdate)
    );
    view.apply(&doc, &update).unwrap();
    assert_eq!(
        candidates.advance(&doc, &mut budget, &mut Vec::new()),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(
        task.advance(&doc, &mut view, &mut budget),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(
        pass.advance_folded(&doc, &mut view, &mut budget, &mut Sink::default()),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(budget.used(), WorkLimit::default());
    let other = document("> z\n");
    assert_eq!(
        view.set_collapsed(&other, container(0), true),
        Err(LayoutError::WrongDocument)
    );
}

#[test]
fn mixed_streaming_open_masks_and_closed_masks_match_interval_model() {
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 2).unwrap();
    let mut selected = std::collections::BTreeSet::new();
    let mut preview = None;
    let mut seed = 0xa7514de9u64;
    for step in 0..240 {
        let line = match step % 8 {
            0 => "# Root\n".to_string(),
            1 => "> first\n".to_string(),
            2 => "> > nested\n".to_string(),
            3 => format!("> > {}", "long ".repeat(1500)),
            4 => "tail\n> - a|b\n>   -|-\n>   x|y\n".to_string(),
            5 => "> - sibling\n>   > child\n".to_string(),
            6 => "> outer\n".to_string(),
            _ => "outside".to_string(),
        };
        let update = doc.append(&line).unwrap();
        view.apply(&doc, &update).unwrap();
        for i in update.new_containers {
            if i % 3 != 0 {
                selected.insert(container(i));
                view.set_collapsed(&doc, container(i), true).unwrap();
            }
        }
        let ids: Vec<_> = all(&doc).iter().map(|c| c.id).collect();
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let id = ids[seed as usize % ids.len()];
        match step % 5 {
            0 => {
                preview = Some(id);
                view.set_preview(&doc, preview, &mut tiny()).unwrap();
            }
            1 => {
                preview = None;
                view.set_preview(&doc, None, &mut tiny()).unwrap();
            }
            2 => view.reset(3).unwrap(),
            _ => {
                if !selected.insert(id) {
                    selected.remove(&id);
                }
                view.toggle(&doc, id).unwrap();
            }
        }
        let effective: Vec<_> = selected
            .iter()
            .copied()
            .filter(|&id| Some(id) != preview)
            .collect();
        let sink = render(&doc, &mut view, false);
        assert_eq!(sink.text, oracle(&doc, &effective), "step {step}");
        assert_eq!(
            view.heights().total_rows(),
            sink.text.bytes().filter(|&b| b == b'\n').count() as u128
        );
    }
    let update = doc.finish();
    view.apply(&doc, &update).unwrap();
    view.set_preview(&doc, None, &mut tiny()).unwrap();
    assert_eq!(
        render(&doc, &mut view, false).text,
        oracle(&doc, &selected.into_iter().collect::<Vec<_>>())
    );
}

#[test]
fn empty_eof_and_task_containers_keep_metadata_and_own_one_placeholder() {
    for source in [
        ">",
        ">\n",
        "- \n",
        "- [x] done\n  continued\n",
        "1. one\n   next",
    ] {
        let doc = document(source);
        let before: Vec<_> = (0..doc.container_count())
            .map(|i| doc.container(ContainerId(i)).unwrap())
            .collect();
        assert!(!before.is_empty());
        let mut view = FoldView::new(&doc, 1).unwrap();
        batch(&doc, &mut view, FoldBatchAction::All { collapsed: true });
        let sink = render(&doc, &mut view, true);
        assert_eq!(sink.text, "[fold]\n");
        assert_eq!(sink.placeholders.len(), 1);
        assert_eq!(sink.placeholders[0].candidate.id, container(0));
        assert_eq!(sink.placeholders[0].owner, TileId(0));
        assert_eq!(view.heights().total_rows(), 1);
        batch(&doc, &mut view, FoldBatchAction::All { collapsed: false });
        assert_eq!(render(&doc, &mut view, true).text, oracle(&doc, &[]));
        assert_eq!(doc.source(), source);
        assert_eq!(
            (0..doc.container_count())
                .map(|i| doc.container(ContainerId(i)).unwrap())
                .collect::<Vec<_>>(),
            before
        );
    }
}
