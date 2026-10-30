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
fn raw(table: usize, start: usize, end: usize) -> FoldId {
    FoldId::TableRows(TableRowRange {
        table: TableId(table),
        start,
        end,
    })
}
fn remove(doc: &Document, view: &mut FoldView, id: FoldId) {
    let FoldId::TableRows(range) = id else {
        panic!()
    };
    view.remove_table_rows(doc, range).unwrap();
}
fn hidden(doc: &Document, view: &FoldView, at: usize) -> Option<FoldId> {
    match view.hidden_at(doc, at, &mut one()).unwrap() {
        FoldLookup::Hidden(c) => Some(c.id),
        FoldLookup::Visible => None,
        FoldLookup::BudgetExhausted => panic!(),
    }
}
fn batch(doc: &Document, view: &mut FoldView, action: FoldBatchAction) -> FoldBatchProgress {
    let mut task = FoldBatch::new(doc, view, action).unwrap();
    for _ in 0..doc.blocks().len() + doc.container_count() + view.table_row_range_count() + 3 {
        let mut budget = one();
        let progress = task.advance(doc, view, &mut budget).unwrap();
        assert!(budget.used().objects <= 1);
        if progress.completion == Completion::Covered {
            return progress;
        }
    }
    panic!("batch stalled")
}

#[derive(Default)]
struct Sink {
    text: String,
    placeholders: Vec<FoldPlaceholder>,
    bytes: usize,
    rows: u64,
    objects: usize,
}
impl LayoutSink for Sink {
    fn begin_tile(&mut self, _: TileId) {
        self.rows = 0;
    }
    fn text(&mut self, run: RenderRun<'_>) {
        self.bytes += run.text.len();
        if !run.style.contains(Style::HIDDEN) {
            self.text.push_str(run.text);
            self.rows += run.text.bytes().filter(|&b| b == b'\n').count() as u64;
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
    fn placeholder(&mut self, placeholder: FoldPlaceholder) {
        self.placeholders.push(placeholder);
        self.text.push_str("[fold]\n");
        self.rows += 1;
    }
}
fn render(doc: &Document, view: &mut FoldView, small: bool) -> Sink {
    let mut pass = LayoutPass::folded(doc, view, TileId(0), u64::MAX).unwrap();
    let mut sink = Sink::default();
    for _ in 0..100_000 {
        let mut budget = Budget::new(if small {
            WorkLimit {
                queries: 1,
                objects: 1,
                bytes: 4,
                tiles: 1,
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
    panic!("layout stalled")
}

// Independent interval union from parser metadata. No fold_candidate or view
// range index is used to derive visibility, placeholder order or tile coverage.
fn interval(doc: &Document, id: FoldId) -> Span {
    match id {
        FoldId::TableRows(r) => {
            let rows = doc.table(r.table).unwrap().rows();
            Span::new(rows[r.start].source.start, rows[r.end - 1].source.end)
        }
        FoldId::Table(id) => {
            let t = doc.table(id).unwrap();
            Span::new(
                t.source.start,
                if doc.block(t.block).unwrap().state == SyntaxState::Sealed {
                    t.source.end
                } else {
                    doc.source().len()
                },
            )
        }
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
                .find(|next| next.container == h.container && next.level <= h.level)
                .map_or(doc.source().len(), |next| next.source.start);
            Span::new(h.source.end, end)
        }
        FoldId::Code(_) | FoldId::Math(_) | FoldId::Source(_) => unreachable!(),
    }
}
fn model(doc: &Document, view: &FoldView, ids: &[FoldId]) -> Vec<(Span, FoldId)> {
    let mut ranges: Vec<_> = ids
        .iter()
        .copied()
        .filter(|&id| view.is_collapsed(id) && view.preview() != Some(id))
        .map(|id| (interval(doc, id), id))
        .filter(|(span, _)| !span.is_empty())
        .collect();
    ranges.sort_by_key(|&(s, id)| {
        (
            s.start,
            std::cmp::Reverse(s.end),
            match id {
                FoldId::Container(id) => (0, id.0),
                _ => (1, 0),
            },
        )
    });
    ranges
}
fn verify(doc: &Document, view: &FoldView, ids: &[FoldId]) {
    let ranges = model(doc, view, ids);
    for at in (0..doc.source().len())
        .step_by(17)
        .chain([doc.source().len()])
    {
        let expected = ranges
            .iter()
            .find(|(s, _)| s.range().contains(&at))
            .map(|(_, id)| *id);
        assert_eq!(hidden(doc, view, at), expected, "at {at}");
    }
    for i in 0..doc.tile_count() {
        let covered = ranges.iter().any(|(s, _)| {
            let first = doc.tile_at(s.start).unwrap().0 + 1;
            let end = doc.tile_at(s.end).map_or(doc.tile_count(), |id| id.0);
            first <= i && i < end
        });
        assert_eq!(
            view.heights().height(TileId(i)),
            Some(if covered {
                0
            } else {
                view.heights().base_height(TileId(i)).unwrap()
            }),
            "tile {i}"
        );
    }
}
fn oracle(doc: &Document, view: &FoldView, ids: &[FoldId]) -> String {
    let mut result = String::new();
    let mut at = 0;
    for (span, _) in model(doc, view, ids) {
        if span.start < at {
            continue;
        }
        for run in doc.runs_in(Span::new(at, span.start)).unwrap() {
            if !run.style.contains(Style::HIDDEN) {
                result.push_str(run.text);
            }
        }
        result.push_str("[fold]\n");
        at = span.end;
    }
    for run in doc.runs_in(Span::new(at, doc.source().len())).unwrap() {
        if !run.style.contains(Style::HIDDEN) {
            result.push_str(run.text);
        }
    }
    result
}

#[test]
fn identities_overlap_and_removal_are_atomic_and_view_local() {
    let doc = parse("| h |\n| - |\n| a |\n| b |\n| c |\n| d |\n");
    let mut view = FoldView::new(&doc, 1).unwrap();
    let mut other = FoldView::new(&doc, 1).unwrap();
    for (table, rows) in [
        (0, 0..1),
        (0, 1..1),
        (0, std::ops::Range { start: 3, end: 2 }),
        (0, 1..6),
        (1, 1..2),
    ] {
        let version = view.version();
        assert_eq!(
            view.insert_table_rows(&doc, TableId(table), rows),
            Err(LayoutError::InvalidRange)
        );
        assert_eq!(view.version(), version);
    }
    let a = view.insert_table_rows(&doc, TableId(0), 1..3).unwrap();
    assert_eq!(a, raw(0, 1, 3));
    assert!(doc.fold_candidate(a).is_some());
    assert_eq!(
        other.set_collapsed(&doc, a, true),
        Err(LayoutError::InvalidRange)
    );
    let mut budget = one();
    assert_eq!(
        other.set_preview(&doc, Some(a), &mut budget),
        Err(LayoutError::InvalidRange)
    );
    assert_eq!(budget.used(), WorkLimit::default());
    assert!(matches!(
        FoldBatch::new(&doc, &other, FoldBatchAction::Focus { target: a }),
        Err(LayoutError::InvalidRange)
    ));
    for rows in [1..2, 2..4, 1..5] {
        assert_eq!(
            view.insert_table_rows(&doc, TableId(0), rows),
            Err(LayoutError::InvalidRange)
        );
    }
    let b = view.insert_table_rows(&doc, TableId(0), 3..5).unwrap();
    view.set_collapsed(&doc, a, true).unwrap();
    let version = view.version();
    assert_eq!(view.insert_table_rows(&doc, TableId(0), 1..3).unwrap(), a);
    assert_eq!(view.version(), version);
    assert!(view.is_collapsed(a));
    view.set_preview(&doc, Some(a), &mut one()).unwrap();
    assert_eq!(
        view.insert_table_rows(&doc, TableId(0), 2..3),
        Err(LayoutError::InvalidRange)
    );
    remove(&doc, &mut view, a);
    assert_eq!(view.preview(), None);
    assert!(!view.is_collapsed(a));
    assert_eq!(view.table_row_range_count(), 1);
    assert_eq!(
        view.insert_table_rows(&doc, TableId(0), 2..3).unwrap(),
        raw(0, 2, 3)
    );
    remove(&doc, &mut view, b);
}

#[test]
fn same_tile_adjacent_placeholders_and_ancestor_choices_keep_rows() {
    let doc = parse("# Root\n> | h |\n> | - |\n> | a |\n> | b |\n> | c |\n> | d |\noutside\n");
    assert_eq!(doc.tile_count(), 1);
    let mut view = FoldView::new(&doc, 1).unwrap();
    let a = view.insert_table_rows(&doc, TableId(0), 1..3).unwrap();
    let b = view.insert_table_rows(&doc, TableId(0), 3..4).unwrap();
    let ids = [
        FoldId::Heading(HeadingId(0)),
        FoldId::Container(ContainerId(0)),
        FoldId::Table(TableId(0)),
        a,
        b,
    ];
    for id in [a, b] {
        view.set_collapsed(&doc, id, true).unwrap();
    }
    let candidate = doc.fold_candidate(a).unwrap();
    assert_eq!(candidate.parent, Some(FoldId::Table(TableId(0))));
    assert_eq!(candidate.hidden, interval(&doc, a));
    let mut budget = Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 3,
        ..Default::default()
    });
    assert_eq!(candidate.summary_text(&doc, 3, &mut budget), Some("> |"));
    let expected = oracle(&doc, &view, &ids);
    let sink = render(&doc, &mut view, true);
    assert_eq!(sink.text, expected);
    assert_eq!(
        sink.placeholders
            .iter()
            .map(|p| p.candidate.id)
            .collect::<Vec<_>>(),
        [a, b]
    );
    assert!(sink.text.contains("d\n"));
    for parent in ids[..3].iter().copied() {
        view.set_collapsed(&doc, parent, true).unwrap();
        let expected = oracle(&doc, &view, &ids);
        assert_eq!(render(&doc, &mut view, true).text, expected);
        view.set_collapsed(&doc, parent, false).unwrap();
        assert!(view.is_collapsed(a) && view.is_collapsed(b));
        assert_eq!(render(&doc, &mut view, true).text, sink.text);
    }
    assert_eq!(
        view.heights().total_rows(),
        sink.text.bytes().filter(|&b| b == b'\n').count() as u128
    );
}

#[test]
fn batch_all_subtree_siblings_focus_and_preview_include_registered_ranges() {
    let doc = parse("# Root\n| h |\n| - |\n| a |\n| b |\n| c |\n\n# Other\ntext\n");
    let mut view = FoldView::new(&doc, 1).unwrap();
    let ids: Vec<_> = (1..=3)
        .map(|i| view.insert_table_rows(&doc, TableId(0), i..i + 1).unwrap())
        .collect();
    let table = FoldId::Table(TableId(0));
    let p = batch(&doc, &mut view, FoldBatchAction::All { collapsed: true });
    assert_eq!(p.changed, 6); // two headings, table, three ranges
    batch(
        &doc,
        &mut view,
        FoldBatchAction::Subtree {
            root: table,
            collapsed: false,
        },
    );
    assert!(ids.iter().all(|&id| !view.is_collapsed(id)));
    assert!(view.is_collapsed(FoldId::Heading(HeadingId(0))));
    batch(
        &doc,
        &mut view,
        FoldBatchAction::Subtree {
            root: ids[0],
            collapsed: true,
        },
    );
    assert!(view.is_collapsed(ids[0]) && !view.is_collapsed(ids[1]));
    batch(
        &doc,
        &mut view,
        FoldBatchAction::Siblings {
            target: ids[0],
            collapsed: true,
        },
    );
    assert!(ids.iter().all(|&id| view.is_collapsed(id)));
    batch(&doc, &mut view, FoldBatchAction::Focus { target: ids[1] });
    assert!(!view.is_collapsed(table) && !view.is_collapsed(ids[1]));
    assert!(view.is_collapsed(ids[0]) && view.is_collapsed(ids[2]));
    view.set_preview(&doc, Some(ids[0]), &mut one()).unwrap();
    batch(&doc, &mut view, FoldBatchAction::All { collapsed: false });
    assert_eq!(view.preview(), Some(ids[0]));
    view.set_preview(&doc, None, &mut one()).unwrap();
    assert!(ids.iter().all(|&id| !view.is_collapsed(id)));
    batch(&doc, &mut view, FoldBatchAction::All { collapsed: true });
    assert!(ids.iter().all(|&id| view.is_collapsed(id)));
    batch(
        &doc,
        &mut view,
        FoldBatchAction::AtLevel {
            level: 1,
            collapsed: false,
        },
    );
    assert!(ids.iter().all(|&id| view.is_collapsed(id)));
}

#[test]
fn navigation_anchor_budget_and_stale_tasks_cover_row_masks() {
    let doc = parse("# Root\n| h |\n| - |\n| 中🙂 |\n| next |\n");
    let mut view = FoldView::new(&doc, 1).unwrap();
    let row = view.insert_table_rows(&doc, TableId(0), 1..2).unwrap();
    let table = FoldId::Table(TableId(0));
    let root = FoldId::Heading(HeadingId(0));
    let target = doc.source().find('中').unwrap();
    let anchor = FoldAnchor::new(&doc, target).unwrap();
    let old = view.heights().ticket(TileId(0)).unwrap();
    let mut pass = LayoutPass::folded(&doc, &view, TileId(0), 20).unwrap();
    for id in [row, table, root] {
        view.set_collapsed(&doc, id, true).unwrap();
    }
    assert!(view.measure(old, 9).is_err());
    assert_eq!(
        pass.advance_folded(&doc, &mut view, &mut one(), &mut Sink::default()),
        Err(LayoutError::StaleContinuation)
    );
    let mut empty = Budget::new(WorkLimit::default());
    assert_eq!(
        view.hidden_at(&doc, target, &mut empty).unwrap(),
        FoldLookup::BudgetExhausted
    );
    assert_eq!(
        view.set_preview(&doc, Some(row), &mut empty)
            .unwrap()
            .completion,
        Completion::BudgetExhausted
    );
    assert_eq!(view.preview(), None);
    let mut nav = FoldNavigation::new(
        &doc,
        &view,
        &Navigator::new(),
        NavAction::Source(target),
        FoldNavigationPolicy::Reveal,
        10,
    )
    .unwrap();
    let mut frames = 0;
    loop {
        frames += 1;
        let p = nav.advance(&doc, &mut view, &mut one()).unwrap();
        if p.completion == Completion::Covered {
            assert_eq!(p.target.unwrap().source, Some(target));
            break;
        }
        assert!(frames < 10);
    }
    assert_eq!(frames, 6);
    assert_eq!(
        anchor
            .resolve(&doc, &view, 10, &mut one())
            .unwrap()
            .unwrap()
            .hidden_by,
        None
    );
    view.set_collapsed(&doc, row, true).unwrap();
    let mapped = anchor
        .resolve(&doc, &view, 10, &mut one())
        .unwrap()
        .unwrap();
    assert_eq!(mapped.hidden_by, Some(row));
    assert_eq!(mapped.target.source, Some(interval(&doc, row).start));
    let mut batch = FoldBatch::new(&doc, &view, FoldBatchAction::All { collapsed: false }).unwrap();
    view.insert_table_rows(&doc, TableId(0), 2..3).unwrap();
    assert_eq!(
        batch.advance(&doc, &mut view, &mut one()),
        Err(LayoutError::StaleContinuation)
    );
    let other = parse(doc.source());
    assert_eq!(
        view.insert_table_rows(&other, TableId(0), 1..2),
        Err(LayoutError::WrongDocument)
    );
}

#[test]
fn million_byte_last_row_grows_but_fixed_range_excludes_new_and_pending_rows() {
    let mut doc = Document::new();
    doc.append("| h |\n| - |\n| before |\n| ").unwrap();
    doc.append(&"x".repeat(1_000_000)).unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let range = view.insert_table_rows(&doc, TableId(0), 2..3).unwrap();
    assert!(doc.fold_candidate(range).unwrap().open);
    view.set_collapsed(&doc, range, true).unwrap();
    let baseline = view.range_visits();
    let work = doc.work();
    for _ in 0..1000 {
        let update = doc.append("x").unwrap();
        view.apply(&doc, &update).unwrap();
    }
    assert_eq!(view.range_visits() - baseline, 1000);
    assert_eq!(doc.work().table_bytes - work.table_bytes, 1000);
    let sink = render(&doc, &mut view, true);
    assert!(sink.bytes < 64);
    assert_eq!(sink.placeholders.len(), 1);
    let update = doc.append(" |\n").unwrap();
    view.apply(&doc, &update).unwrap();
    assert!(!doc.fold_candidate(range).unwrap().open);
    let end = interval(&doc, range).end;
    let visits = view.range_visits();
    for chunk in ["| new", " |\n", "# pending", "\n"] {
        let update = doc.append(chunk).unwrap();
        view.apply(&doc, &update).unwrap();
        assert_eq!(interval(&doc, range).end, end);
        assert_eq!(hidden(&doc, &view, end), None);
    }
    assert_eq!(view.range_visits() - visits, 1); // completed EOF boundary is refreshed once
    assert!(render(&doc, &mut view, true).text.contains("new"));
}

#[test]
fn utf8_splits_prefix_snapshots_and_eof_agree() {
    let source = "> | 名 | v |\r\n> | - | - |\r\n> | 中🙂 | &amp; |\r\n> | b |\n> | c |\nout\n";
    let reference = parse(source);
    let mut expected_view = FoldView::new(&reference, 1).unwrap();
    for row in 1..reference.table(TableId(0)).unwrap().rows().len() {
        let id = expected_view
            .insert_table_rows(&reference, TableId(0), row..row + 1)
            .unwrap();
        expected_view.set_collapsed(&reference, id, true).unwrap();
    }
    let expected = render(&reference, &mut expected_view, true).text;
    for split in (0..=source.len()).filter(|&i| source.is_char_boundary(i)) {
        let mut doc = Document::new();
        let mut view = FoldView::new(&doc, 1).unwrap();
        let mut ids = Vec::new();
        for chunk in [&source[..split], &source[split..]] {
            let update = doc.append(chunk).unwrap();
            view.apply(&doc, &update).unwrap();
            if let Some(table) = doc.table(TableId(0)) {
                for row in 1..table.rows().len() {
                    let id = view
                        .insert_table_rows(&doc, TableId(0), row..row + 1)
                        .unwrap();
                    view.set_collapsed(&doc, id, true).unwrap();
                    if !ids.contains(&id) {
                        ids.push(id);
                    }
                }
            }
            verify(&doc, &view, &ids);
            let expected = oracle(&doc, &view, &ids);
            assert_eq!(render(&doc, &mut view, true).text, expected);
        }
        let update = doc.finish();
        view.apply(&doc, &update).unwrap();
        assert_eq!(render(&doc, &mut view, true).text, expected);
    }
}

#[test]
fn merged_updates_and_finish_keep_incomplete_ranges_stable() {
    let mut doc = Document::new();
    doc.append(&format!("| h |\n| - |\n| {}", "x".repeat(1024)))
        .unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let id = view.insert_table_rows(&doc, TableId(0), 1..2).unwrap();
    view.set_collapsed(&doc, id, true).unwrap();
    view.set_preview(&doc, Some(id), &mut one()).unwrap();
    let mut update = doc.append("🙂").unwrap();
    update
        .merge(doc.append(" |\n| next |\n\n").unwrap())
        .unwrap();
    update.merge(doc.finish()).unwrap();
    view.apply(&doc, &update).unwrap();
    view.apply(&doc, &update).unwrap(); // replay is harmless
    view.set_preview(&doc, None, &mut one()).unwrap();
    assert_eq!(
        hidden(&doc, &view, doc.source().find('🙂').unwrap()),
        Some(id)
    );
    assert_eq!(
        hidden(&doc, &view, doc.source().find("next").unwrap()),
        None
    );
    assert!(!doc.fold_candidate(id).unwrap().open);
    let visits = view.range_visits();
    let update = doc.finish();
    view.apply(&doc, &update).unwrap();
    assert_eq!(visits, view.range_visits());

    let mut doc = Document::new();
    doc.append(&format!("| h |\n| - |\n| {}", "x".repeat(1024)))
        .unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let id = view.insert_table_rows(&doc, TableId(0), 1..2).unwrap();
    view.set_collapsed(&doc, id, true).unwrap();
    let update = doc.finish();
    view.apply(&doc, &update).unwrap();
    assert!(!doc.fold_candidate(id).unwrap().open);
    verify(&doc, &view, &[id]);
}

#[test]
fn randomized_visibility_height_and_render_model() {
    let mut source = String::from("# Root\n> | h |\n> | - |\n");
    for row in 0..96 {
        source.push_str(&format!("> | {row}{} |\n", "x".repeat(row % 13 * 40)));
    }
    source.push_str("outside\n");
    let doc = parse(&source);
    let mut view = FoldView::new(&doc, 3).unwrap();
    let mut ids = vec![
        FoldId::Heading(HeadingId(0)),
        FoldId::Container(ContainerId(0)),
        FoldId::Table(TableId(0)),
    ];
    for row in (1..=96).step_by(3) {
        ids.push(
            view.insert_table_rows(&doc, TableId(0), row..row + 2)
                .unwrap(),
        );
    }
    let mut seed = 19u64;
    for turn in 0..1200 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let id = ids[(seed >> 32) as usize % ids.len()];
        match seed % 13 {
            0 => view.reset(turn % 5 + 1).unwrap(),
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
        verify(&doc, &view, &ids);
        if turn % 100 == 0 {
            let expected = oracle(&doc, &view, &ids);
            assert_eq!(render(&doc, &mut view, false).text, expected);
        }
    }
}

#[test]
fn thousands_of_ranges_append_without_historical_refresh_and_reuse_storage() {
    let mut doc = Document::new();
    doc.append(&format!(
        "| h |\n| - |\n{}",
        "| body body body body |\n".repeat(8192)
    ))
    .unwrap();
    let mut view = FoldView::new(&doc, 3).unwrap();
    let mut ids = Vec::new();
    for row in (1..=8192).step_by(2) {
        let id = view
            .insert_table_rows(&doc, TableId(0), row..row + 1)
            .unwrap();
        view.set_collapsed(&doc, id, true).unwrap();
        ids.push(id);
    }
    assert_eq!(view.table_row_range_count(), 4096);
    for (i, &id) in ids.iter().enumerate() {
        let span = interval(&doc, id);
        assert_eq!(hidden(&doc, &view, span.start), Some(id));
        assert_eq!(hidden(&doc, &view, span.end), None);
        if i % 100 == 0 {
            let before = view.heights().mutation_visits();
            view.toggle(&doc, id).unwrap();
            view.toggle(&doc, id).unwrap();
            assert!(view.heights().mutation_visits() - before < 300);
        }
    }
    let visits = view.range_visits();
    let update = doc.append("| next |\n").unwrap();
    view.apply(&doc, &update).unwrap();
    assert_eq!(view.range_visits(), visits);
    assert_eq!(view.table_row_range_count(), 4096);
    for id in ids {
        remove(&doc, &mut view, id);
    }
    assert_eq!(view.table_row_range_count(), 0);
    for _ in 0..1000 {
        let id = view.insert_table_rows(&doc, TableId(0), 1..8193).unwrap();
        view.toggle(&doc, id).unwrap();
        remove(&doc, &mut view, id);
    }
    assert_eq!(view.table_row_range_count(), 0);
}

#[test]
fn hidden_images_and_preview_reads_share_budget_without_mutating_masks() {
    let doc = parse("| h |\n| - |\n| ![hidden](/h) |\n| ![visible](/v) |\n");
    let mut view = FoldView::new(&doc, 1).unwrap();
    let id = view.insert_table_rows(&doc, TableId(0), 1..2).unwrap();
    view.set_collapsed(&doc, id, true).unwrap();
    let before = (view.version(), view.heights().total_rows());
    let mut preview = TablePreview::new(
        &doc,
        TableId(0),
        TablePreviewLimits {
            rows: 3,
            source_bytes: 32,
            display_bytes: 16,
        },
    )
    .unwrap();
    let mut budget = Budget::new(WorkLimit {
        queries: 3,
        objects: 3,
        bytes: 6,
        ..Default::default()
    });
    assert!(matches!(
        view.hidden_at(&doc, interval(&doc, id).start, &mut budget)
            .unwrap(),
        FoldLookup::Hidden(_)
    ));
    while let TablePreviewStep::Run { .. } = preview.next(&doc, &mut budget).unwrap() {}
    assert_eq!(before, (view.version(), view.heights().total_rows()));
    let sink = render(&doc, &mut view, true);
    assert_eq!(sink.objects, 1);
    assert_eq!(sink.placeholders.len(), 1);
}

#[test]
fn six_open_sections_whole_table_and_row_refresh_only_eight_ranges() {
    let mut doc = Document::new();
    doc.append(&format!(
        "# a\n## b\n### c\n#### d\n##### e\n###### f\n| h |\n| - |\n| {}",
        "x".repeat(1024)
    ))
    .unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let id = view.insert_table_rows(&doc, TableId(0), 1..2).unwrap();
    batch(&doc, &mut view, FoldBatchAction::All { collapsed: true });
    assert!(view.is_collapsed(id));
    let before = view.range_visits();
    let update = doc.append("more").unwrap();
    view.apply(&doc, &update).unwrap();
    assert_eq!(view.range_visits() - before, 8);
    let update = doc.append(" |\n| later |\n# next\n").unwrap();
    view.apply(&doc, &update).unwrap();
    for i in 0..6 {
        view.set_collapsed(&doc, FoldId::Heading(HeadingId(i)), false)
            .unwrap();
    }
    view.set_collapsed(&doc, FoldId::Table(TableId(0)), false)
        .unwrap();
    assert!(view.is_collapsed(id));
    let sink = render(&doc, &mut view, true);
    assert_eq!(sink.placeholders.len(), 1);
    assert!(sink.text.contains("later"));
    assert!(sink.text.ends_with("next\n"));
}

#[test]
fn streamed_ranges_nested_scope_and_other_tables_preserve_fixed_bounds() {
    let mut doc = Document::new();
    doc.append("# Root\n> - ## Inner\n>   | h |\n>   | - |\n")
        .unwrap();
    let mut view = FoldView::new(&doc, 2).unwrap();
    let inner = FoldId::Heading(HeadingId(1));
    let mut ids = Vec::new();
    for row in 1..=240 {
        let update = doc
            .append(&format!(">   | {row} {} |\n", "x".repeat(row % 7 * 43)))
            .unwrap();
        view.apply(&doc, &update).unwrap();
        let id = view
            .insert_table_rows(&doc, TableId(0), row..row + 1)
            .unwrap();
        ids.push(id);
        view.set_collapsed(&doc, id, row % 3 != 0).unwrap();
        if row % 7 == 0 {
            view.set_preview(&doc, Some(id), &mut one()).unwrap();
        }
        if row % 11 == 0 {
            view.reset(3).unwrap();
        }
        verify(&doc, &view, &ids);
        if row % 20 == 0 {
            let expected = oracle(&doc, &view, &ids);
            assert_eq!(render(&doc, &mut view, false).text, expected);
        }
    }
    assert_eq!(
        doc.fold_candidate(FoldId::Table(TableId(0)))
            .unwrap()
            .parent,
        Some(inner)
    );
    view.set_collapsed(&doc, inner, true).unwrap();
    let selected = interval(&doc, ids[0]).start;
    assert_eq!(hidden(&doc, &view, selected), Some(inner));
    let update = doc.append("outside\n\n| h2 |\n| - |\n| v2 |\n").unwrap();
    view.apply(&doc, &update).unwrap();
    let second = view.insert_table_rows(&doc, TableId(1), 1..2).unwrap();
    view.set_collapsed(&doc, second, true).unwrap();
    assert_eq!(
        hidden(&doc, &view, interval(&doc, second).start),
        Some(second)
    );
    view.set_collapsed(&doc, inner, false).unwrap();
    assert_eq!(hidden(&doc, &view, selected), Some(ids[0]));
    view.set_preview(&doc, None, &mut one()).unwrap();
    ids.push(second);
    let update = doc.finish();
    view.apply(&doc, &update).unwrap();
    verify(&doc, &view, &ids);
    let expected = oracle(&doc, &view, &ids);
    assert_eq!(render(&doc, &mut view, true).text, expected);
}

#[test]
fn completed_eof_row_releases_growing_tile_once_including_utf8_boundaries() {
    for size in [240, 241, 242, 243, 244, 245, 246, 247, 248, 249, 500, 800] {
        for suffix in ["next", "🙂", "中"] {
            let mut doc = Document::new();
            doc.append(&format!("h|v\n-|-\n{}|v\n", "x".repeat(size)))
                .unwrap();
            let mut view = FoldView::new(&doc, 2).unwrap();
            let id = view.insert_table_rows(&doc, TableId(0), 1..2).unwrap();
            view.set_collapsed(&doc, id, true).unwrap();
            verify(&doc, &view, &[id]);
            let fixed = doc.fold_candidate(id).unwrap();
            assert!(!fixed.open);
            let visits = view.range_visits();
            let mut update = doc.append(suffix).unwrap();
            update.merge(doc.append("|v\n").unwrap()).unwrap();
            view.apply(&doc, &update).unwrap();
            assert_eq!(doc.fold_candidate(id).unwrap(), fixed);
            assert_eq!(view.range_visits() - visits, 1);
            verify(&doc, &view, &[id]);
            let expected = oracle(&doc, &view, &[id]);
            let sink = render(&doc, &mut view, true);
            assert_eq!(sink.text, expected);
            assert!(sink.text.contains(suffix));
            let visits = view.range_visits();
            let update = doc.append("later|v\n").unwrap();
            view.apply(&doc, &update).unwrap();
            assert_eq!(view.range_visits(), visits);
        }
    }
}
