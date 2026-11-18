use mdstream::*;

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

fn depth_defaults(depth: Option<usize>) -> FoldDefaults {
    FoldDefaults {
        container_depth: depth,
        ..Default::default()
    }
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
                FoldId::Code(_)
                | FoldId::Math(_)
                | FoldId::TableRows(_)
                | FoldId::TableRange(_)
                | FoldId::Source(_) => {
                    unreachable!()
                }
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
    assert_eq!(expected, "R\n[fold]\nEnd\n"); // `out` lazily continues the last item.
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
    let update = doc.append("\n# outside\n").unwrap();
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
    let doc = document(&"> item\n---\n".repeat(4096));
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
fn pending_list_blanks_preview_and_scope_exit_match_interval_height_model() {
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 2).unwrap();
    let mut selected = std::collections::BTreeSet::new();
    let mut preview = None;
    let mut seed = 0xa7514de9u64;
    for step in 0..240 {
        let line = match step % 12 {
            0 => "# Root\n- first\n".to_owned(),
            1 => "\n \r\n".to_owned(),
            2 => "  - nested\n    # section\n".to_owned(),
            3 => "\n\n".to_owned(),
            4 => format!("    {}", "long ".repeat(1500)),
            5 => "tail\n\n- sibling\n".to_owned(),
            6 => "\n  h|h\n  -|-\n  x|y\n\n".to_owned(),
            7 => "  > - quoted\n  >\n  > - next\n".to_owned(),
            8 => "\n".to_owned(),
            9 => "- [x] last\n\n".to_owned(),
            10 => "+ other\n\n".to_owned(),
            _ => "outside".to_owned(),
        };
        let update = doc.append(&line).unwrap();
        view.apply(&doc, &update).unwrap();
        for i in update.new_containers {
            if i % 3 != 0 {
                selected.insert(container(i));
                view.set_collapsed(&doc, container(i), true).unwrap();
            }
        }
        let ids: Vec<_> = (0..doc.container_count()).map(container).collect();
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
        let offset = seed as usize % doc.source().len();
        let expected = ranges(&doc, &effective)
            .into_iter()
            .find(|(span, _)| span.range().contains(&offset));
        match view.hidden_at(&doc, offset, &mut tiny()).unwrap() {
            FoldLookup::Hidden(c) => assert_eq!(Some(c.id), expected.map(|(_, id)| id)),
            FoldLookup::Visible => assert!(expected.is_none()),
            other => panic!("visibility did not finish: {other:?}"),
        }
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

#[test]
fn default_depth_threshold_counts_structural_list_and_item_levels() {
    for threshold in [
        None,
        Some(0),
        Some(1),
        Some(2),
        Some(3),
        Some(4),
        Some(usize::MAX),
    ] {
        let mut doc = Document::new();
        let mut view = FoldView::new(&doc, 1).unwrap();
        let mut other = FoldView::new(&doc, 1).unwrap();
        view.set_defaults(depth_defaults(threshold));
        let mut update = doc
            .append("# Root\n> - first\n>   > inner\n> - sibling\nout\n- root\n")
            .unwrap();
        update.merge(doc.finish()).unwrap();
        view.apply(&doc, &update).unwrap();
        other.apply(&doc, &update).unwrap();
        let depths: Vec<_> = (0..doc.container_count())
            .map(|i| doc.container(ContainerId(i)).unwrap().depth)
            .collect();
        assert_eq!(depths, [1, 2, 3, 4, 3, 1, 2]);
        for (i, depth) in depths.into_iter().enumerate() {
            assert_eq!(
                view.is_collapsed(container(i)),
                threshold.is_some_and(|d| depth >= d)
            );
            assert!(!other.is_collapsed(container(i)));
        }
        assert!(!view.is_collapsed(FoldId::Heading(HeadingId(0))));
        assert_eq!(
            view.default_visits(),
            if threshold.is_some() { 7 } else { 0 }
        );
        let chosen: Vec<_> = (0..doc.container_count())
            .map(container)
            .filter(|&id| view.is_collapsed(id))
            .collect();
        assert_eq!(render(&doc, &mut view, true).text, oracle(&doc, &chosen));
    }
}

#[test]
fn default_depth_preserves_existing_and_explicit_choices_across_growth_and_reset() {
    let mut doc = Document::new();
    doc.append("> old\n").unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    view.set_defaults(depth_defaults(Some(1)));
    assert!(!view.is_collapsed(container(0)));
    let update = doc.append("> > new\n").unwrap();
    view.apply(&doc, &update).unwrap();
    assert!(!view.is_collapsed(container(0)));
    assert!(view.is_collapsed(container(1)));
    view.set_collapsed(&doc, container(0), true).unwrap();
    view.set_collapsed(&doc, container(1), false).unwrap();
    let version = view.version();
    let visits = view.default_visits();
    view.apply(&doc, &update).unwrap();
    assert_eq!(view.version(), version);
    assert_eq!(view.default_visits(), visits);
    let update = doc.append("> > more\n> > > deeper\n").unwrap();
    view.apply(&doc, &update).unwrap();
    assert!(view.is_collapsed(container(2)));
    view.set_defaults(depth_defaults(None));
    view.reset(3).unwrap();
    let mut update = doc.append("> > > > expanded\n").unwrap();
    update.merge(doc.finish()).unwrap();
    view.apply(&doc, &update).unwrap();
    assert_eq!(
        (0..4)
            .map(|i| view.is_collapsed(container(i)))
            .collect::<Vec<_>>(),
        [true, false, true, false]
    );
    assert_eq!(view.default_visits(), 2);
    view.set_collapsed(&doc, container(0), false).unwrap();
    let chosen = [container(2)];
    assert_eq!(render(&doc, &mut view, true).text, oracle(&doc, &chosen));
}

#[test]
fn default_depth_chunk_boundaries_prefixes_and_merged_updates_agree() {
    fn parse(chunks: &[&str], merged: bool, finish: bool) -> (Vec<(FoldCandidate, bool)>, String) {
        let mut doc = Document::new();
        let mut view = FoldView::new(&doc, 1).unwrap();
        view.set_defaults(FoldDefaults {
            heading_levels: [true; 6],
            table: true,
            container_depth: Some(3),
            ..Default::default()
        });
        let mut updates = Update::default();
        for &chunk in chunks {
            let update = doc.append(chunk).unwrap();
            if merged {
                updates.merge(update).unwrap();
            } else {
                view.apply(&doc, &update).unwrap();
            }
        }
        if finish {
            let update = doc.finish();
            if merged {
                updates.merge(update).unwrap();
            } else {
                view.apply(&doc, &update).unwrap();
            }
        }
        if merged {
            view.apply(&doc, &updates).unwrap();
        }
        let found = all(&doc);
        for c in &found {
            let expected = match c.id {
                FoldId::Container(id) => doc.container(id).unwrap().depth >= 3,
                _ => true,
            };
            assert_eq!(view.is_collapsed(c.id), expected);
        }
        let choices = found
            .into_iter()
            .map(|c| (c, view.is_collapsed(c.id)))
            .collect();
        (choices, render(&doc, &mut view, true).text)
    }
    let source = "# R\n> - 界|b\n>   -|-\n>   🙂|y\n> - next\n>   > child\nout\n- end";
    let expected = parse(&[source], false, true);
    for (at, _) in source
        .char_indices()
        .chain(std::iter::once((source.len(), '\0')))
    {
        for merged in [false, true] {
            assert_eq!(
                parse(&[&source[..at], &source[at..]], merged, true),
                expected
            );
        }
        let chunks: Vec<_> = source[..at]
            .char_indices()
            .map(|(i, c)| &source[i..i + c.len_utf8()])
            .collect();
        assert_eq!(
            parse(&chunks, false, false),
            parse(&[&source[..at]], false, false)
        );
    }
}

#[test]
fn default_depth_merged_events_use_current_policy_including_already_closed_nodes() {
    let mut doc = Document::new();
    let mut incremental = FoldView::new(&doc, 1).unwrap();
    let mut merged = FoldView::new(&doc, 1).unwrap();
    let mut switched = FoldView::new(&doc, 1).unwrap();
    for view in [&mut incremental, &mut merged, &mut switched] {
        view.set_defaults(depth_defaults(Some(1)));
    }
    let mut updates = Update::default();
    for chunk in ["> first\n", "> > child\n", "out\n", "- last\n"] {
        let update = doc.append(chunk).unwrap();
        incremental.apply(&doc, &update).unwrap();
        updates.merge(update).unwrap();
    }
    let update = doc.finish();
    incremental.apply(&doc, &update).unwrap();
    updates.merge(update).unwrap();
    switched.set_defaults(depth_defaults(Some(2)));
    merged.apply(&doc, &updates).unwrap();
    switched.apply(&doc, &updates).unwrap();
    for i in 0..doc.container_count() {
        assert!(incremental.is_collapsed(container(i)));
        assert!(merged.is_collapsed(container(i)));
        assert_eq!(
            switched.is_collapsed(container(i)),
            doc.container(ContainerId(i)).unwrap().depth >= 2
        );
    }
    assert_eq!(incremental.default_visits(), doc.container_count() as u64);
    assert_eq!(merged.default_visits(), incremental.default_visits());
    assert_eq!(
        render(&doc, &mut merged, true).text,
        render(&doc, &mut incremental, true).text
    );
}

#[test]
fn default_depth_batch_and_preview_preserve_saved_choices_and_initialize_new_children() {
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 1).unwrap();
    view.set_defaults(FoldDefaults {
        table: true,
        ..depth_defaults(Some(2))
    });
    let update = doc.append("- a|b\n  -|-\n  x|y\n").unwrap();
    view.apply(&doc, &update).unwrap();
    let table = FoldId::Table(TableId(0));
    view.set_preview(&doc, Some(container(1)), &mut tiny())
        .unwrap();
    assert!(view.is_collapsed(container(1)));
    assert!(
        matches!(view.hidden_at(&doc, 0, &mut tiny()).unwrap(), FoldLookup::Hidden(c) if c.id == table)
    );
    let update = doc.append("- next\n  > nested\n").unwrap();
    view.apply(&doc, &update).unwrap();
    assert_eq!(doc.container_count(), 4);
    assert!(view.is_collapsed(container(2)));
    assert!(view.is_collapsed(container(3)));
    batch(&doc, &mut view, FoldBatchAction::All { collapsed: false });
    assert!(!view.is_collapsed(container(1)));
    let update = doc.append("- newest\n").unwrap();
    view.apply(&doc, &update).unwrap();
    assert!(view.is_collapsed(container(4)));
    assert!(!view.is_collapsed(container(1)));
    view.set_preview(&doc, None, &mut tiny()).unwrap();
    batch(
        &doc,
        &mut view,
        FoldBatchAction::Siblings {
            target: container(4),
            collapsed: true,
        },
    );
    assert!(view.is_collapsed(container(1)));
    assert!(view.is_collapsed(container(2)));
    assert!(!view.is_collapsed(container(3)));
    assert!(!view.is_collapsed(table));
    view.set_preview(&doc, Some(container(0)), &mut tiny())
        .unwrap();
    batch(
        &doc,
        &mut view,
        FoldBatchAction::Subtree {
            root: container(0),
            collapsed: true,
        },
    );
    assert!(view.is_collapsed(container(0))); // Saved intent, temporarily expanded.
    let update = doc.append("out\n").unwrap();
    view.apply(&doc, &update).unwrap();
    view.reset(2).unwrap();
    view.set_preview(&doc, None, &mut tiny()).unwrap();
    assert!(
        matches!(view.hidden_at(&doc, 0, &mut tiny()).unwrap(), FoldLookup::Hidden(c) if c.id == container(0))
    );
    batch(&doc, &mut view, FoldBatchAction::Focus { target: table });
    let update = doc.finish();
    view.apply(&doc, &update).unwrap();
    for id in [container(0), container(1), table] {
        assert!(!view.is_collapsed(id));
    }
    for id in [container(2), container(3), container(4)] {
        assert!(view.is_collapsed(id));
    }
    assert_eq!(view.defaults().container_depth, Some(2));
}

#[test]
fn default_depth_policy_changes_invalidate_tasks_without_changing_layout_or_scanning_history() {
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let update = doc.append(&"> item\n---\n".repeat(4096)).unwrap();
    view.apply(&doc, &update).unwrap();
    let mut task = FoldBatch::new(&doc, &view, FoldBatchAction::All { collapsed: true }).unwrap();
    let before = (
        view.default_visits(),
        view.container_index_visits(),
        view.heights().mutation_visits(),
    );
    view.set_defaults(depth_defaults(Some(1)));
    assert_eq!(
        before,
        (
            view.default_visits(),
            view.container_index_visits(),
            view.heights().mutation_visits()
        )
    );
    let mut budget = tiny();
    assert_eq!(
        task.advance(&doc, &mut view, &mut budget),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(budget.used(), WorkLimit::default());
    let mut task = FoldBatch::new(&doc, &view, FoldBatchAction::All { collapsed: false }).unwrap();
    let version = view.version();
    view.set_defaults(depth_defaults(Some(1)));
    assert_eq!(view.version(), version);
    task.advance(&doc, &mut view, &mut tiny()).unwrap();
    let update = doc.append("> new\n").unwrap();
    view.apply(&doc, &update).unwrap();
    assert_eq!(view.default_visits(), 1);
    assert!(view.container_index_visits() - before.1 < 32);
    assert!(view.is_collapsed(container(4096)));
    assert!((0..4096).all(|i| !view.is_collapsed(container(i))));
}

#[test]
fn default_depth_thousand_levels_and_million_bytes_only_initialize_once() {
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 1).unwrap();
    view.set_defaults(depth_defaults(Some(500)));
    let update = doc
        .append(&format!("{}{}", ">".repeat(1000), "x".repeat(1_000_000)))
        .unwrap();
    view.apply(&doc, &update).unwrap();
    assert_eq!(view.default_visits(), 1000);
    assert!(view.container_index_visits() < 12_000);
    let before = (
        view.default_visits(),
        view.range_visits(),
        view.container_index_visits(),
    );
    let parser_work = doc.work();
    for _ in 0..1000 {
        let update = doc.append("x").unwrap();
        view.apply(&doc, &update).unwrap();
    }
    assert_eq!(view.default_visits(), before.0);
    assert_eq!(view.range_visits() - before.1, 1000);
    assert_eq!(view.container_index_visits(), before.2);
    assert_eq!(doc.work().container_steps, parser_work.container_steps);
    let mut sink = Sink::default();
    let mut pass = LayoutPass::folded(&doc, &view, TileId(0), 10).unwrap();
    let mut budget = Budget::new(WorkLimit {
        queries: 16,
        objects: 4,
        tiles: 2,
        ..Default::default()
    });
    assert_eq!(
        pass.advance_folded(&doc, &mut view, &mut budget, &mut sink)
            .unwrap(),
        Completion::EndOfDocument
    );
    assert_eq!(budget.used().bytes, 0);
    assert_eq!(sink.placeholders[0].candidate.id, container(499));
    assert_eq!(sink.text, "[fold]\n");
    let visits = view.range_visits();
    let update = doc.finish();
    view.apply(&doc, &update).unwrap();
    assert_eq!(view.range_visits() - visits, 502); // 501 selected closes + shared tail.
    assert_eq!(view.default_visits(), 1000);
    assert!(view.container_index_nodes() < 4000);
}

#[test]
fn default_depth_streaming_policy_and_explicit_choices_match_interval_height_model() {
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 2).unwrap();
    let mut selected = std::collections::BTreeSet::new();
    let mut preview = None;
    let mut threshold = Some(2);
    let mut seed = 0x8bc25136u64;
    view.set_defaults(depth_defaults(threshold));
    for step in 0..240 {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        if step % 7 == 0 {
            threshold = [None, Some(0), Some(2), Some(4)][seed as usize % 4];
            view.set_defaults(depth_defaults(threshold));
        }
        let line = match step % 6 {
            0 => "> root\n".to_string(),
            1 => "> - item\n".to_string(),
            2 => format!(">   > {}", "long ".repeat(1000)),
            3 => "tail\n> - sibling\n".to_string(),
            4 => "> > inner\n".to_string(),
            _ => "outside\n".to_string(),
        };
        let old_count = doc.container_count();
        let update = doc.append(&line).unwrap();
        view.apply(&doc, &update).unwrap();
        for i in old_count..doc.container_count() {
            if threshold.is_some_and(|d| doc.container(ContainerId(i)).unwrap().depth >= d) {
                selected.insert(container(i));
            }
        }
        let id = container(seed as usize % doc.container_count());
        match step % 9 {
            0 => {
                preview = Some(id);
                view.set_preview(&doc, preview, &mut tiny()).unwrap();
            }
            1 => {
                preview = None;
                view.set_preview(&doc, None, &mut tiny()).unwrap();
            }
            2 => {
                view.reset(step as u64 % 5 + 1).unwrap();
            }
            3 => {
                batch(&doc, &mut view, FoldBatchAction::All { collapsed: false });
                selected.clear();
            }
            _ => {
                if !selected.insert(id) {
                    selected.remove(&id);
                }
                view.toggle(&doc, id).unwrap();
            }
        }
        for i in 0..doc.container_count() {
            assert_eq!(
                view.is_collapsed(container(i)),
                selected.contains(&container(i))
            );
        }
        let effective: Vec<_> = selected
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
        if step % 13 == 0 || step == 239 {
            let sink = render(&doc, &mut view, false);
            assert_eq!(sink.text, oracle(&doc, &effective), "step {step}");
            assert_eq!(
                view.heights().total_rows(),
                sink.text.bytes().filter(|&b| b == b'\n').count() as u128
            );
        }
    }
}

fn policy_filter(bits: usize) -> ContainerFoldFilter {
    ContainerFoldFilter {
        quotes: bits & 1 != 0,
        lists: bits & 2 != 0,
        items: bits & 4 != 0,
        completed_tasks: bits & 8 != 0,
        incomplete_tasks: bits & 16 != 0,
    }
}

// Independent class bit model, including the union between all items and tasks.
fn policy_matches(bits: usize, kind: ContainerKind, checked: Option<bool>) -> bool {
    let classes = match kind {
        ContainerKind::Quote => 1,
        ContainerKind::List { .. } => 2,
        ContainerKind::Item { .. } => 4 | checked.map_or(0, |c| if c { 8 } else { 16 }),
    };
    bits & classes != 0
}

fn policy_action(bits: usize, collapsed: bool) -> FoldBatchAction {
    FoldBatchAction::Containers {
        filter: policy_filter(bits),
        collapsed,
    }
}

#[test]
fn container_policy_all_class_unions_and_depths_match_independent_model() {
    for bits in 0..32 {
        for depth in [None, Some(0), Some(3), Some(usize::MAX)] {
            let mut doc = Document::new();
            let mut view = FoldView::new(&doc, 1).unwrap();
            view.set_defaults(FoldDefaults {
                containers: policy_filter(bits),
                container_depth: depth,
                ..Default::default()
            });
            let mut update = doc.append("# Root\n> - [x] done\n>   > quote\n> - [ ] open\n> - ordinary\n\n1. [X] ordered\n2. [z] ordinary\n").unwrap();
            update.merge(doc.finish()).unwrap();
            view.apply(&doc, &update).unwrap();
            for i in 0..doc.container_count() {
                let id = ContainerId(i);
                let c = doc.container(id).unwrap();
                assert_eq!(
                    view.is_collapsed(container(i)),
                    policy_matches(bits, c.kind, view.task_checked(&doc, id).unwrap())
                        || depth.is_some_and(|d| c.depth >= d)
                );
            }
            assert_eq!(
                view.default_visits(),
                if bits == 0 && depth.is_none() {
                    0
                } else {
                    doc.container_count() as u64
                }
            );
            assert!(!view.is_collapsed(FoldId::Heading(HeadingId(0))));
            let chosen: Vec<_> = (0..doc.container_count())
                .map(container)
                .filter(|&id| view.is_collapsed(id))
                .collect();
            assert_eq!(render(&doc, &mut view, true).text, oracle(&doc, &chosen));
        }
    }
}

#[test]
fn container_policy_marker_confirmation_splits_prefixes_eof_and_merge_agree() {
    fn snapshot(
        chunks: &[&str],
        merged: bool,
        finish: bool,
    ) -> Vec<(Container, bool, Option<bool>)> {
        let mut doc = Document::new();
        let mut view = FoldView::new(&doc, 1).unwrap();
        view.set_defaults(FoldDefaults {
            containers: policy_filter(8),
            ..Default::default()
        });
        let mut updates = Update::default();
        for chunk in chunks {
            let update = doc.append(chunk).unwrap();
            if merged {
                updates.merge(update).unwrap();
            } else {
                view.apply(&doc, &update).unwrap();
            }
        }
        if finish {
            let update = doc.finish();
            if merged {
                updates.merge(update).unwrap();
            } else {
                view.apply(&doc, &update).unwrap();
            }
        }
        if merged {
            view.apply(&doc, &updates).unwrap();
        }
        assert_eq!(view.default_visits(), doc.container_count() as u64);
        (0..doc.container_count())
            .map(|i| {
                let c = doc.container(ContainerId(i)).unwrap();
                let state = view.task_checked(&doc, ContainerId(i)).unwrap();
                assert_eq!(view.is_collapsed(container(i)), state == Some(true));
                (c, view.is_collapsed(container(i)), state)
            })
            .collect()
    }
    for source in [
        "> - [x] 中\r\n>   > next\n> - [ ] 🙂\n\n1. [X] eof",
        "- [x]",
        "- [x",
        "- [x]bad\n",
        "- [ ]\n- [z] plain\n",
    ] {
        let expected = snapshot(&[source], false, true);
        for cut in source.char_indices().map(|(i, _)| i).chain([source.len()]) {
            for merged in [false, true] {
                assert_eq!(
                    snapshot(&[&source[..cut], &source[cut..]], merged, true),
                    expected
                );
                assert_eq!(
                    snapshot(&[&source[..cut]], merged, false),
                    snapshot(&[&source[..cut]], false, false)
                );
            }
            let chars: Vec<_> = source[..cut]
                .char_indices()
                .map(|(i, c)| &source[i..i + c.len_utf8()])
                .collect();
            assert_eq!(
                snapshot(&chars, false, false),
                snapshot(&[&source[..cut]], false, false)
            );
        }
    }
}

#[test]
fn container_policy_existing_explicit_and_overridden_choices_survive_append() {
    let mut doc = Document::new();
    doc.append("- [x] old\n").unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let mut other = FoldView::new(&doc, 1).unwrap();
    view.set_defaults(FoldDefaults {
        containers: policy_filter(8),
        ..Default::default()
    });
    assert!(!view.is_collapsed(container(1)));
    let update = doc.append("- [x] new\n").unwrap();
    view.apply(&doc, &update).unwrap();
    other.apply(&doc, &update).unwrap();
    assert!(view.is_collapsed(container(2)));
    view.set_collapsed(&doc, container(2), false).unwrap();
    let before = (
        doc.revision(),
        doc.work(),
        doc.source().to_owned(),
        doc.container(ContainerId(2)),
    );
    view.set_task_checked(&doc, ContainerId(2), false).unwrap();
    assert_eq!(
        (
            doc.revision(),
            doc.work(),
            doc.source().to_owned(),
            doc.container(ContainerId(2))
        ),
        before
    );
    assert_eq!(
        view.task_checked(&doc, ContainerId(2)).unwrap(),
        Some(false)
    );
    assert_eq!(
        other.task_checked(&doc, ContainerId(2)).unwrap(),
        Some(true)
    );
    assert!(!view.is_collapsed(container(2)));
    let visits = view.default_visits();
    view.apply(&doc, &update).unwrap();
    assert_eq!(view.default_visits(), visits);
    view.reset(3).unwrap();
    let mut update = doc.append("  grows\n- [x] later\n").unwrap();
    update.merge(doc.finish()).unwrap();
    view.apply(&doc, &update).unwrap();
    other.apply(&doc, &update).unwrap();
    assert_eq!(view.default_visits() - visits, 1);
    assert!(!view.is_collapsed(container(2)));
    assert!(view.is_collapsed(container(3)));
    assert_eq!(
        view.task_checked(&doc, ContainerId(2)).unwrap(),
        Some(false)
    );
    assert_eq!(
        other.task_checked(&doc, ContainerId(2)).unwrap(),
        Some(true)
    );
    view.set_task_checked(&doc, ContainerId(2), true).unwrap();
    assert!(!view.is_collapsed(container(2))); // Defaults are never re-run.
    let version = view.version();
    assert_eq!(
        view.set_task_checked(&doc, ContainerId(2), true).unwrap(),
        None
    );
    assert_eq!(view.version(), version);
}

#[test]
fn container_policy_filtered_batches_charge_unmatched_nodes_and_share_budget() {
    let doc = document(
        "> - [x] done\n> - [ ] pending\n> - plain\n\n# Heading\n```\ncode\n```\na|b\n-|-\nx|y\n",
    );
    let mut view = FoldView::new(&doc, 1).unwrap();
    let rows = view.insert_table_rows(&doc, TableId(0), 1..2).unwrap();
    let noncontainers: Vec<_> = all(&doc)
        .into_iter()
        .filter(|c| !matches!(c.id, FoldId::Container(_)))
        .map(|c| c.id)
        .chain([rows])
        .collect();
    for &id in &noncontainers {
        view.set_collapsed(&doc, id, true).unwrap();
    }
    view.set_task_checked(&doc, ContainerId(2), false).unwrap();
    view.set_task_checked(&doc, ContainerId(3), true).unwrap();
    let work = doc.work();
    let mut task = FoldBatch::new(&doc, &view, policy_action(8, true)).unwrap();
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
        assert_eq!(budget.used(), WorkLimit::default());
        assert_eq!(progress.completion, Completion::BudgetExhausted);
    }
    for i in 0..doc.container_count() {
        let mut budget = tiny();
        let progress = task.advance(&doc, &mut view, &mut budget).unwrap();
        assert_eq!(progress.visited, i + 1);
        assert_eq!(progress.changed, usize::from(i >= 3));
        assert_eq!(budget.used(), tiny().remaining());
        let paused = task.advance(&doc, &mut view, &mut budget).unwrap();
        assert_eq!(paused.visited, progress.visited);
        assert_eq!(paused.change, None);
    }
    let mut empty = Budget::new(WorkLimit::default());
    assert_eq!(
        task.advance(&doc, &mut view, &mut empty)
            .unwrap()
            .completion,
        Completion::Covered
    );
    assert_eq!(empty.used(), WorkLimit::default());
    assert_eq!(doc.work(), work);
    assert!(!view.is_collapsed(container(2)));
    assert!(view.is_collapsed(container(3)));
    assert!(noncontainers.iter().all(|&id| view.is_collapsed(id)));
    let empty_filter = batch(&doc, &mut view, policy_action(0, false));
    assert_eq!(empty_filter.visited, doc.container_count());
    assert_eq!(empty_filter.changed, 0);
    assert_eq!(batch(&doc, &mut view, policy_action(4, false)).changed, 1);
    assert!(noncontainers.iter().all(|&id| view.is_collapsed(id)));
}

#[test]
fn container_policy_task_changes_invalidate_tasks_tickets_and_validate_identity() {
    let mut doc = Document::new();
    doc.append("- [ ] task\n- ordinary\n").unwrap();
    let mut view = FoldView::<u32>::with_state(&doc, 1).unwrap();
    let other = document("- [x] other\n");
    assert_eq!(view.task_checked(&doc, ContainerId(0)).unwrap(), None);
    for id in [ContainerId(0), ContainerId(2), ContainerId(99)] {
        assert_eq!(
            view.set_task_checked(&doc, id, true),
            Err(LayoutError::InvalidRange)
        );
    }
    assert_eq!(
        view.task_checked(&other, ContainerId(1)),
        Err(LayoutError::WrongDocument)
    );
    assert_eq!(
        view.set_task_checked(&other, ContainerId(1), true),
        Err(LayoutError::WrongDocument)
    );
    view.insert_checkpoint(TileId(0), 42).unwrap();
    let ticket = view.heights().ticket(TileId(0)).unwrap();
    let mut task = FoldBatch::new(&doc, &view, policy_action(8, true)).unwrap();
    let mut nav = FoldNavigation::new(
        &doc,
        &view,
        &Navigator::new(),
        NavAction::Home,
        FoldNavigationPolicy::Reveal,
        1,
    )
    .unwrap();
    let mut pass = LayoutPass::folded(&doc, &view, TileId(0), 10).unwrap();
    let change = view
        .set_task_checked(&doc, ContainerId(1), true)
        .unwrap()
        .unwrap();
    assert_eq!(&doc.source()[change.source.range()], "[ ]");
    assert_eq!(view.measure(ticket, 99), Err(LayoutError::StaleMeasurement));
    assert!(view.checkpoints().previous(TileId(0)).is_none());
    let mut budget = tiny();
    assert_eq!(
        task.advance(&doc, &mut view, &mut budget),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(
        nav.advance(&doc, &mut view, &mut budget),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(
        pass.advance_folded(&doc, &mut view, &mut budget, &mut Sink::default()),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(budget.used(), WorkLimit::default());
    let update = doc.append("  continuation\n").unwrap();
    assert_eq!(
        view.task_checked(&doc, ContainerId(1)),
        Err(LayoutError::OutOfOrderUpdate)
    );
    assert_eq!(
        view.set_task_checked(&doc, ContainerId(1), false),
        Err(LayoutError::OutOfOrderUpdate)
    );
    view.apply(&doc, &update).unwrap();
    assert_eq!(view.task_checked(&doc, ContainerId(1)).unwrap(), Some(true));
    let mut task = FoldBatch::new(&doc, &view, policy_action(8, true)).unwrap();
    view.set_task_checked(&doc, ContainerId(1), true).unwrap(); // No-op keeps task valid.
    assert_eq!(
        task.advance(&doc, &mut view, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap()
            .changed,
        1
    );
}

#[test]
fn container_policy_preview_batches_and_navigation_preserve_local_intent() {
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 1).unwrap();
    view.set_defaults(FoldDefaults {
        containers: policy_filter(8),
        ..Default::default()
    });
    let update = doc.append("- [x] parent\n").unwrap();
    view.apply(&doc, &update).unwrap();
    view.set_preview(&doc, Some(container(1)), &mut tiny())
        .unwrap();
    let update = doc.append("  - [x] child\n").unwrap();
    view.apply(&doc, &update).unwrap();
    assert!(view.is_collapsed(container(3)));
    view.set_task_checked(&doc, ContainerId(1), false).unwrap();
    batch(&doc, &mut view, policy_action(16, false));
    assert!(!view.is_collapsed(container(1)));
    assert_eq!(view.preview(), Some(container(1)));
    view.set_preview(&doc, None, &mut tiny()).unwrap();
    let target = doc.source().find("child").unwrap();
    assert!(
        matches!(view.hidden_at(&doc, target, &mut tiny()).unwrap(), FoldLookup::Hidden(c) if c.id == container(3))
    );
    let mut nav = FoldNavigation::new(
        &doc,
        &view,
        &Navigator::new(),
        NavAction::Source(target),
        FoldNavigationPolicy::Reveal,
        4,
    )
    .unwrap();
    let mut reached = None;
    for _ in 0..12 {
        let progress = nav.advance(&doc, &mut view, &mut tiny()).unwrap();
        if progress.completion == Completion::Covered {
            reached = progress.target;
            break;
        }
    }
    assert_eq!(reached.unwrap().source, Some(target));
    assert!(!view.is_collapsed(container(3)));
    let update = doc.append("    more\n  - [x] new sibling\n").unwrap();
    view.apply(&doc, &update).unwrap();
    assert!(!view.is_collapsed(container(3)));
    assert!(view.is_collapsed(container(4)));
    assert_eq!(
        view.task_checked(&doc, ContainerId(1)).unwrap(),
        Some(false)
    );
}

#[test]
fn container_policy_thousand_levels_million_bytes_and_sparse_batch_are_bounded() {
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 1).unwrap();
    view.set_defaults(FoldDefaults {
        containers: policy_filter(1 | 8),
        ..Default::default()
    });
    let update = doc
        .append(&format!(
            "{}- [x] {}",
            ">".repeat(1000),
            "x".repeat(1_000_000)
        ))
        .unwrap();
    view.apply(&doc, &update).unwrap();
    assert_eq!(doc.container_count(), 1002);
    assert_eq!(view.default_visits(), 1002);
    let before = (
        view.default_visits(),
        view.range_visits(),
        view.container_index_visits(),
        doc.work(),
    );
    for _ in 0..1000 {
        let update = doc.append("x").unwrap();
        view.apply(&doc, &update).unwrap();
    }
    assert_eq!(view.default_visits(), before.0);
    assert_eq!(view.range_visits() - before.1, 1000);
    assert_eq!(view.container_index_visits(), before.2);
    assert_eq!(doc.work().container_steps, before.3.container_steps);
    let progress = batch(&doc, &mut view, policy_action(16, true));
    assert_eq!((progress.visited, progress.changed), (1002, 0));
    let mut sink = Sink::default();
    let mut pass = LayoutPass::folded(&doc, &view, TileId(0), 10).unwrap();
    let mut budget = Budget::new(WorkLimit {
        queries: 16,
        objects: 4,
        tiles: 2,
        ..Default::default()
    });
    assert_eq!(
        pass.advance_folded(&doc, &mut view, &mut budget, &mut sink)
            .unwrap(),
        Completion::EndOfDocument
    );
    assert_eq!(budget.used().bytes, 0);
    assert_eq!(sink.text, "[fold]\n");
    let before = view.container_index_visits();
    view.set_task_checked(&doc, ContainerId(1001), false)
        .unwrap();
    assert_eq!(view.container_index_visits(), before);
    assert!(view.is_collapsed(container(1001))); // No implicit refolding or ancestry walk.
    let update = doc.finish();
    view.apply(&doc, &update).unwrap();
    assert_eq!(view.default_visits(), 1002);
}

#[test]
fn container_policy_historical_siblings_do_not_increase_append_initialization() {
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 1).unwrap();
    view.set_defaults(FoldDefaults {
        containers: policy_filter(8),
        ..Default::default()
    });
    let update = doc.append(&"- [x] old\n".repeat(4096)).unwrap();
    view.apply(&doc, &update).unwrap();
    let before = (view.default_visits(), view.container_index_visits());
    let update = doc.append("- [ ] new\n").unwrap();
    view.apply(&doc, &update).unwrap();
    assert_eq!(view.default_visits() - before.0, 1);
    assert!(view.container_index_visits() - before.1 < 100);
    view.set_task_checked(&doc, ContainerId(4097), true)
        .unwrap();
    let work = doc.work();
    let mut task = FoldBatch::new(&doc, &view, policy_action(8, false)).unwrap();
    let mut budget = Budget::new(WorkLimit {
        queries: 4,
        objects: 4,
        ..Default::default()
    });
    let progress = task.advance(&doc, &mut view, &mut budget).unwrap();
    assert_eq!((progress.visited, progress.changed), (4, 3));
    assert_eq!(budget.used().bytes, 0);
    let mut budget = Budget::new(WorkLimit {
        queries: 5000,
        objects: 5000,
        ..Default::default()
    });
    let progress = task.advance(&doc, &mut view, &mut budget).unwrap();
    assert_eq!((progress.visited, progress.changed), (4098, 4096));
    assert_eq!(doc.work(), work);
    let visits = view.default_visits();
    let update = doc.append("- [x] future\n").unwrap();
    view.apply(&doc, &update).unwrap();
    assert_eq!(view.default_visits() - visits, 1);
    assert!(view.is_collapsed(container(4098)));
    assert!(!view.is_collapsed(container(4097)));
}

#[test]
fn container_policy_streaming_filters_overrides_and_preview_match_interval_height_model() {
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 2).unwrap();
    let mut choices = std::collections::BTreeSet::new();
    let mut states = std::collections::BTreeMap::new();
    let mut preview = None;
    let mut bits = 8;
    let mut depth = None;
    let mut seed = 0x834f52b6u64;
    for step in 0..320 {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        if step % 7 == 0 {
            bits = seed as usize % 32;
            depth = if step % 14 == 0 { Some(4) } else { None };
            view.set_defaults(FoldDefaults {
                containers: policy_filter(bits),
                container_depth: depth,
                ..Default::default()
            });
        }
        let old_count = doc.container_count();
        let line = match step % 5 {
            0 => "> - [x] parent\n".to_owned(),
            1 => ">   - [ ] child\n".to_owned(),
            2 => format!(">     {}\n", "long ".repeat(800)),
            3 => "> - ordinary\n".to_owned(),
            _ => "\n".to_owned(),
        };
        let update = doc.append(&line).unwrap();
        view.apply(&doc, &update).unwrap();
        for i in old_count..doc.container_count() {
            let id = ContainerId(i);
            let c = doc.container(id).unwrap();
            let checked = match c.kind {
                ContainerKind::Item { task, .. } => task.map(|t| t.checked),
                _ => None,
            };
            if let Some(value) = checked {
                states.insert(id, value);
            }
            if policy_matches(bits, c.kind, checked) || depth.is_some_and(|d| c.depth >= d) {
                choices.insert(container(i));
            }
        }
        let i = seed as usize % doc.container_count();
        let id = ContainerId(i);
        match step % 6 {
            0 => {
                preview = Some(container(i));
                view.set_preview(&doc, preview, &mut tiny()).unwrap();
            }
            1 => {
                preview = None;
                view.set_preview(&doc, None, &mut tiny()).unwrap();
            }
            2 => {
                let (&task, &old) = states.iter().nth(seed as usize % states.len()).unwrap();
                states.insert(task, !old);
                view.set_task_checked(&doc, task, !old).unwrap();
            }
            3 => {
                let selected = (seed >> 8) as usize % 32;
                let collapsed = seed & 32 != 0;
                batch(&doc, &mut view, policy_action(selected, collapsed));
                for j in 0..doc.container_count() {
                    if policy_matches(
                        selected,
                        doc.container(ContainerId(j)).unwrap().kind,
                        states.get(&ContainerId(j)).copied(),
                    ) {
                        if collapsed {
                            choices.insert(container(j));
                        } else {
                            choices.remove(&container(j));
                        }
                    }
                }
            }
            4 => {
                view.reset(step % 4 + 1).unwrap();
            }
            _ => {
                view.toggle(&doc, container(i)).unwrap();
                if !choices.insert(container(i)) {
                    choices.remove(&container(i));
                }
            }
        }
        assert_eq!(
            view.task_checked(&doc, id).unwrap(),
            states.get(&id).copied()
        );
        for i in 0..doc.container_count() {
            assert_eq!(
                view.is_collapsed(container(i)),
                choices.contains(&container(i))
            );
        }
        let effective: Vec<_> = choices
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
        if step % 13 == 0 || step == 319 {
            let sink = render(&doc, &mut view, false);
            assert_eq!(sink.text, oracle(&doc, &effective), "step {step}");
            assert_eq!(
                view.heights().total_rows(),
                sink.text.bytes().filter(|&b| b == b'\n').count() as u128
            );
        }
    }
}

#[test]
fn container_policy_batch_partial_work_is_kept_on_invalidation_and_wrong_view_rejected() {
    let mut doc = Document::new();
    doc.append("- [x] first\n- [x] second\n").unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let mut other = FoldView::new(&doc, 1).unwrap();
    let mut task = FoldBatch::new(&doc, &view, policy_action(8, true)).unwrap();
    let mut budget = tiny();
    assert_eq!(
        task.advance(&doc, &mut other, &mut budget),
        Err(LayoutError::WrongView)
    );
    let wrong = document("- [x] wrong\n");
    assert_eq!(
        task.advance(&wrong, &mut view, &mut budget),
        Err(LayoutError::WrongDocument)
    );
    assert_eq!(budget.used(), WorkLimit::default());
    let mut budget = Budget::new(WorkLimit {
        queries: 2,
        objects: 2,
        ..Default::default()
    });
    let progress = task.advance(&doc, &mut view, &mut budget).unwrap();
    assert_eq!((progress.visited, progress.changed), (2, 1));
    assert!(view.is_collapsed(container(1)));
    assert!(!view.is_collapsed(container(2)));
    view.set_defaults(FoldDefaults {
        containers: policy_filter(16),
        ..Default::default()
    });
    let mut budget = tiny();
    assert_eq!(
        task.advance(&doc, &mut view, &mut budget),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(budget.used(), WorkLimit::default());
    let mut task = FoldBatch::new(&doc, &view, policy_action(8, true)).unwrap();
    let update = doc.append("- [ ] future\n").unwrap();
    view.apply(&doc, &update).unwrap();
    assert_eq!(
        task.advance(&doc, &mut view, &mut budget),
        Err(LayoutError::StaleContinuation)
    );
    assert!(view.is_collapsed(container(1)));
    assert!(!view.is_collapsed(container(2)));
    assert!(view.is_collapsed(container(3)));
    assert_eq!(budget.used(), WorkLimit::default());
    let empty = Document::new();
    let mut empty_view = FoldView::new(&empty, 1).unwrap();
    let mut task = FoldBatch::new(&empty, &empty_view, policy_action(31, true)).unwrap();
    let mut budget = Budget::new(WorkLimit::default());
    assert_eq!(
        task.advance(&empty, &mut empty_view, &mut budget)
            .unwrap()
            .completion,
        Completion::Covered
    );
}

#[test]
fn container_policy_merged_update_uses_apply_time_defaults_once() {
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 1).unwrap();
    view.set_defaults(FoldDefaults {
        containers: policy_filter(16),
        ..Default::default()
    });
    let mut updates = doc.append("- [ ] first\n").unwrap();
    updates
        .merge(doc.append("- [x] second\n").unwrap())
        .unwrap();
    updates.merge(doc.finish()).unwrap();
    view.set_defaults(FoldDefaults {
        containers: policy_filter(8),
        ..Default::default()
    });
    view.apply(&doc, &updates).unwrap();
    assert!(!view.is_collapsed(container(1)));
    assert!(view.is_collapsed(container(2)));
    assert_eq!(view.default_visits(), 3);
    view.set_task_checked(&doc, ContainerId(1), true).unwrap();
    view.set_task_checked(&doc, ContainerId(2), false).unwrap();
    view.set_defaults(FoldDefaults::default());
    let before = (view.version(), view.default_visits());
    view.apply(&doc, &updates).unwrap();
    assert_eq!((view.version(), view.default_visits()), before);
    assert!(!view.is_collapsed(container(1)));
    assert!(view.is_collapsed(container(2)));
    assert_eq!(view.task_checked(&doc, ContainerId(1)).unwrap(), Some(true));
    assert_eq!(
        view.task_checked(&doc, ContainerId(2)).unwrap(),
        Some(false)
    );
}

#[test]
fn nested_tail_propagation_preview_and_scope_close_match_interval_height_model() {
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 2).unwrap();
    let mut selected = std::collections::BTreeSet::new();
    let mut preview = None;
    let mut seed = 0x1ca5994fu64;
    for step in 0..240 {
        let line = match step % 8 {
            0 => "# Root\n- # outer\n  - # inner\n",
            1 => "    \r\n",
            2 => "- sibling\n  > - child\n  >   \n",
            3 => "  h|h\n  -|-\n  x|y\n",
            4 => "  - # child\n    \n    more\n    \n",
            5 => "  # parent\n",
            6 => "  - > child\n    >\n",
            _ => "- [x] next\n# outside\n",
        };
        let update = doc.append(line).unwrap();
        view.apply(&doc, &update).unwrap();
        for i in update.new_containers {
            if i % 3 != 0 {
                selected.insert(container(i));
                view.set_collapsed(&doc, container(i), true).unwrap();
            }
        }
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let id = container(seed as usize % doc.container_count());
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
