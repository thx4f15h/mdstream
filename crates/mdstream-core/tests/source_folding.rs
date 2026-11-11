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
fn register(doc: &Document, view: &mut FoldView, source: Span) -> Result<FoldId, LayoutError> {
    let mut task = SourceFoldRegistration::new(doc, view, source)?;
    for _ in 0..doc.container_count() * 4 + doc.headings().entries().len() * 2 + 20 {
        let mut budget = one();
        let p = task.advance(doc, view, &mut budget)?;
        assert_eq!(budget.used().bytes, 0);
        assert!(budget.used().queries <= 1 && budget.used().objects <= 1);
        if let Some(id) = p.id {
            assert_eq!(p.completion, Completion::Covered);
            assert_eq!(
                task.advance(doc, view, &mut Budget::new(WorkLimit::default()))?,
                p
            );
            return Ok(id);
        }
    }
    panic!("registration stalled")
}
fn remove(doc: &Document, view: &mut FoldView, id: FoldId) {
    let FoldId::Source(id) = id else { panic!() };
    assert!(view.remove_source_fold(doc, id).unwrap().is_some());
}
fn hidden(doc: &Document, view: &FoldView, at: usize) -> Option<FoldCandidate> {
    match view.hidden_at(doc, at, &mut one()).unwrap() {
        FoldLookup::Visible => None,
        FoldLookup::Hidden(c) => Some(c),
        FoldLookup::BudgetExhausted => panic!(),
    }
}
fn batch(doc: &Document, view: &mut FoldView, action: FoldBatchAction) -> FoldBatchProgress {
    let mut task = FoldBatch::new(doc, view, action).unwrap();
    for _ in 0..doc.blocks().len()
        + doc.container_count()
        + view.source_fold_count()
        + view.table_row_range_count()
        + 5
    {
        let p = task.advance(doc, view, &mut one()).unwrap();
        if p.completion == Completion::Covered {
            return p;
        }
    }
    panic!("batch stalled")
}
#[derive(Default)]
struct Sink {
    text: String,
    rows: u64,
    bytes: usize,
    objects: usize,
    placeholders: Vec<FoldPlaceholder>,
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
    fn placeholder(&mut self, p: FoldPlaceholder) {
        self.placeholders.push(p);
        self.text.push_str("[fold]\n");
        self.rows += 1;
    }
}
fn render(doc: &Document, view: &mut FoldView, tiny: bool) -> Sink {
    let mut pass = LayoutPass::folded(doc, view, TileId(0), u64::MAX).unwrap();
    let mut sink = Sink::default();
    for _ in 0..100_000 {
        let mut budget = Budget::new(if tiny {
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
fn syntax(doc: &Document) -> Vec<FoldCandidate> {
    let mut out = Vec::new();
    doc.fold_candidates(Span::new(0, doc.source().len()))
        .unwrap()
        .advance(doc, &mut Budget::new(WorkLimit::unlimited()), &mut out)
        .unwrap();
    out
}
fn contains(a: Span, b: Span) -> bool {
    a.start <= b.start && b.end <= a.end
}
fn compatible(a: Span, b: Span) -> bool {
    b.is_empty() || a.end <= b.start || b.end <= a.start || contains(a, b) || contains(b, a)
}
// Full linear oracle, deliberately independent of the endpoint validator and
// of the point-lookup indexes. Only raw intervals determine visibility/height.
fn verify(doc: &Document, view: &FoldView, candidates: &[FoldCandidate]) -> String {
    let mut ranges: Vec<_> = candidates
        .iter()
        .filter(|c| view.is_collapsed(c.id) && view.preview() != Some(c.id) && !c.hidden.is_empty())
        .collect();
    ranges.sort_by_key(|c| (c.hidden.start, std::cmp::Reverse(c.hidden.end), c.id));
    for at in (0..doc.source().len())
        .step_by(13)
        .chain([doc.source().len()])
    {
        let expected = ranges
            .iter()
            .find(|c| c.hidden.range().contains(&at))
            .map(|c| c.hidden);
        assert_eq!(hidden(doc, view, at).map(|c| c.hidden), expected, "at {at}");
    }
    for i in 0..doc.tile_count() {
        let covered = ranges.iter().any(|c| {
            doc.tile_at(c.hidden.start).unwrap().0 < i
                && i < doc.tile_at(c.hidden.end).map_or(doc.tile_count(), |t| t.0)
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
    let mut out = String::new();
    let mut at = 0;
    for c in ranges {
        if c.hidden.start < at {
            continue;
        }
        for r in doc.runs_in(Span::new(at, c.hidden.start)).unwrap() {
            if !r.style.contains(Style::HIDDEN) {
                out.push_str(r.text);
            }
        }
        out.push_str("[fold]\n");
        at = c.hidden.end;
    }
    for r in doc.runs_in(Span::new(at, doc.source().len())).unwrap() {
        if !r.style.contains(Style::HIDDEN) {
            out.push_str(r.text);
        }
    }
    out
}

#[test]
fn fixed_identity_disjoint_ranges_utf8_and_view_isolation() {
    let doc = parse("a中🙂bcdef\n");
    let mut view = FoldView::new(&doc, 2).unwrap();
    for s in [
        Span::new(0, 0),
        Span::new(4, 1),
        Span::new(1, 2),
        Span::new(0, 100),
    ] {
        assert!(matches!(
            SourceFoldRegistration::new(&doc, &view, s),
            Err(LayoutError::InvalidRange)
        ));
    }
    let a = register(&doc, &mut view, Span::new(1, 8)).unwrap();
    let b = register(&doc, &mut view, Span::new(8, 10)).unwrap();
    let original = view.fold_candidate(&doc, a).unwrap().unwrap();
    assert!(doc.fold_candidate(a).is_none());
    assert_eq!(original.summary, Some(Span::new(1, 8)));
    assert!(!original.open && !view.is_collapsed(a));
    for s in [
        Span::new(1, 8),
        Span::new(0, 4),
        Span::new(4, 9),
        Span::new(4, 8),
        Span::new(0, 11),
    ] {
        assert_eq!(register(&doc, &mut view, s), Err(LayoutError::InvalidRange));
    }
    view.set_collapsed(&doc, a, true).unwrap();
    view.set_preview(&doc, Some(a), &mut one()).unwrap();
    assert_eq!(
        register(&doc, &mut view, Span::new(1, 8)),
        Err(LayoutError::InvalidRange)
    );
    let mut other = FoldView::new(&doc, 2).unwrap();
    assert_eq!(
        other.set_collapsed(&doc, a, true),
        Err(LayoutError::InvalidRange)
    );
    let different = register(&doc, &mut other, Span::new(1, 8)).unwrap();
    assert_ne!(a, different);
    view.reset(4).unwrap();
    assert_eq!(view.fold_candidate(&doc, a).unwrap(), Some(original));
    remove(&doc, &mut view, a);
    assert_eq!(view.preview(), None);
    assert!(view.fold_candidate(&doc, a).unwrap().is_none());
    assert_eq!(
        view.set_collapsed(&doc, a, true),
        Err(LayoutError::InvalidRange)
    );
    assert!(view.fold_candidate(&doc, b).unwrap().is_some());
    let new = register(&doc, &mut view, Span::new(1, 8)).unwrap();
    assert_ne!(new, a);
}

#[test]
fn endpoint_validation_matches_full_structural_interval_model() {
    let doc = parse(
        "# Root\ntext\n> - ## Inner\n>   before\n>   ```\n>   code\n>   ```\n>   | h |\n>   | - |\n>   | r |\nout\n\n## Next\n$$\nx\n$$\nend\n",
    );
    let mut candidates = syntax(&doc);
    let rows = FoldId::TableRows(TableRowRange {
        table: TableId(0),
        start: 1,
        end: 2,
    });
    candidates.push(doc.fold_candidate(rows).unwrap());
    let boundaries: Vec<_> = (0..=doc.source().len()).collect();
    let mut seed = 31u64;
    for _ in 0..2400 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let a = boundaries[(seed >> 32) as usize % boundaries.len()];
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let b = boundaries[(seed >> 32) as usize % boundaries.len()];
        if a == b {
            continue;
        }
        let span = Span::new(a.min(b), a.max(b));
        let expected = candidates
            .iter()
            .all(|c| compatible(span, c.source) && compatible(span, c.hidden));
        let mut view = FoldView::new(&doc, 1).unwrap();
        view.insert_table_rows(&doc, TableId(0), 1..2).unwrap();
        let result = register(&doc, &mut view, span);
        assert_eq!(result.is_ok(), expected, "{span:?}, {result:?}");
        assert_eq!(view.source_fold_count(), usize::from(expected));
    }
}

#[test]
fn structural_containment_parent_choices_and_row_crossings_both_orders() {
    let doc =
        parse("# Root\nintro\n> ```\n> code\n> ```\n\n| h |\n| - |\n| abc |\n| def |\n\nend\n");
    let mut view = FoldView::new(&doc, 2).unwrap();
    let quote = doc
        .fold_candidate(FoldId::Container(ContainerId(0)))
        .unwrap();
    let outer = register(&doc, &mut view, quote.source).unwrap();
    let code = doc
        .fold_candidate_at(doc.source().find("code").unwrap())
        .unwrap();
    for id in [outer, code.id, quote.id] {
        view.set_collapsed(&doc, id, true).unwrap();
    }
    let root = FoldId::Heading(HeadingId(0));
    assert_eq!(
        view.fold_candidate(&doc, outer).unwrap().unwrap().parent,
        Some(code.id)
    );
    view.set_collapsed(&doc, root, true).unwrap();
    assert_eq!(hidden(&doc, &view, quote.source.start).unwrap().id, root);
    view.set_collapsed(&doc, root, false).unwrap();
    view.set_collapsed(&doc, quote.id, false).unwrap();
    view.set_collapsed(&doc, code.id, false).unwrap();
    assert_eq!(hidden(&doc, &view, quote.source.start).unwrap().id, outer);
    let row = doc.table(TableId(0)).unwrap().row(1).unwrap().source;
    let next = doc.table(TableId(0)).unwrap().row(2).unwrap().source;
    let crossing = Span::new(row.start + 2, next.start + 2);
    let manual = register(&doc, &mut view, crossing).unwrap();
    assert_eq!(
        view.insert_table_rows(&doc, TableId(0), 1..2),
        Err(LayoutError::InvalidRange)
    );
    remove(&doc, &mut view, manual);
    view.insert_table_rows(&doc, TableId(0), 1..2).unwrap();
    assert_eq!(
        register(&doc, &mut view, crossing),
        Err(LayoutError::InvalidRange)
    );
    let inner = register(&doc, &mut view, Span::new(row.start + 2, row.end - 2)).unwrap();
    let all = syntax(&doc)
        .into_iter()
        .chain([
            doc.fold_candidate(FoldId::TableRows(TableRowRange {
                table: TableId(0),
                start: 1,
                end: 2,
            }))
            .unwrap(),
            view.fold_candidate(&doc, inner).unwrap().unwrap(),
            view.fold_candidate(&doc, outer).unwrap().unwrap(),
        ])
        .collect::<Vec<_>>();
    batch(&doc, &mut view, FoldBatchAction::All { collapsed: true });
    let expected = verify(&doc, &view, &all);
    assert_eq!(render(&doc, &mut view, true).text, expected);
}

#[test]
fn budgeted_registration_is_atomic_and_rejects_stale_or_foreign_tasks() {
    let doc = parse(&format!("{}text\n", ">".repeat(1000)));
    let mut view = FoldView::new(&doc, 1).unwrap();
    let span = Span::new(1000, 1004);
    let mut task = SourceFoldRegistration::new(&doc, &view, span).unwrap();
    let before = view.version();
    assert_eq!(
        task.advance(&doc, &mut view, &mut Budget::new(WorkLimit::default()))
            .unwrap()
            .completion,
        Completion::BudgetExhausted
    );
    for _ in 0..1000 {
        assert!(
            task.advance(&doc, &mut view, &mut one())
                .unwrap()
                .id
                .is_none()
        );
        assert_eq!(view.source_fold_count(), 0);
        assert_eq!(view.version(), before);
    }
    let mut other = FoldView::new(&doc, 1).unwrap();
    assert_eq!(
        task.advance(&doc, &mut other, &mut one()),
        Err(LayoutError::WrongView)
    );
    let alien = parse("other");
    assert_eq!(
        task.advance(&alien, &mut view, &mut one()),
        Err(LayoutError::WrongDocument)
    );
    view.reset(2).unwrap();
    assert_eq!(
        task.advance(&doc, &mut view, &mut one()),
        Err(LayoutError::StaleContinuation)
    );
    let mut task = SourceFoldRegistration::new(&doc, &view, span).unwrap();
    let mut frames = 0;
    let p = loop {
        frames += 1;
        let p = task.advance(&doc, &mut view, &mut one()).unwrap();
        if p.id.is_some() {
            break p;
        }
    };
    assert_eq!(p.visited, 2000);
    assert!(frames < 2010);
    assert_eq!(view.source_fold_count(), 1);
}

#[test]
fn enumeration_intersects_predecessor_and_preserves_budget_and_identity_checks() {
    let doc = parse(&"abcdefghij".repeat(100));
    let mut view = FoldView::new(&doc, 1).unwrap();
    let ids: Vec<_> = (0..1000)
        .step_by(10)
        .map(|i| register(&doc, &mut view, Span::new(i, i + 7)).unwrap())
        .collect();
    let mut cursor = SourceFoldCandidates::new(&doc, &view, Span::new(25, 42)).unwrap();
    let mut out = Vec::new();
    assert_eq!(
        cursor
            .advance(
                &doc,
                &view,
                &mut Budget::new(WorkLimit::default()),
                &mut out
            )
            .unwrap(),
        Completion::BudgetExhausted
    );
    let mut got = Vec::new();
    loop {
        let p = cursor.advance(&doc, &view, &mut one(), &mut out).unwrap();
        assert!(out.len() <= 1);
        got.extend(out.iter().map(|c| c.id));
        if p == Completion::Covered {
            break;
        }
    }
    assert_eq!(got, ids[2..5]);
    assert_eq!(view.source_fold_at(&doc, 23).unwrap().unwrap().id, ids[2]);
    assert!(view.source_fold_at(&doc, 27).unwrap().is_none());
    assert!(view.source_fold_at(&doc, 1000).unwrap().is_none());
    let mut empty = SourceFoldCandidates::new(&doc, &view, Span::new(23, 23)).unwrap();
    assert_eq!(
        empty
            .advance(
                &doc,
                &view,
                &mut Budget::new(WorkLimit::default()),
                &mut out
            )
            .unwrap(),
        Completion::Covered
    );
    assert!(out.is_empty());
    view.toggle(&doc, ids[0]).unwrap();
    assert_eq!(
        cursor.advance(&doc, &view, &mut one(), &mut out),
        Err(LayoutError::StaleContinuation)
    );
}

#[test]
fn batch_subtree_siblings_focus_and_preview_preserve_syntax_parents() {
    let doc = parse("# Root\nabcdef\n```\ncode\n```\n## Next\ntext\n");
    let mut view = FoldView::new(&doc, 1).unwrap();
    let a = register(&doc, &mut view, Span::new(7, 9)).unwrap();
    let b = register(&doc, &mut view, Span::new(9, 11)).unwrap();
    let title = register(&doc, &mut view, Span::new(2, 6)).unwrap();
    let code = doc
        .fold_candidate_at(doc.source().find("code").unwrap())
        .unwrap();
    let same = register(&doc, &mut view, code.source).unwrap();
    let root = FoldId::Heading(HeadingId(0));
    assert_eq!(
        batch(
            &doc,
            &mut view,
            FoldBatchAction::Subtree {
                root: same,
                collapsed: true
            }
        )
        .changed,
        1
    );
    assert!(!view.is_collapsed(code.id));
    batch(
        &doc,
        &mut view,
        FoldBatchAction::Siblings {
            target: a,
            collapsed: true,
        },
    );
    assert!(!view.is_collapsed(a));
    assert!(view.is_collapsed(b) && view.is_collapsed(title) && view.is_collapsed(code.id));
    batch(&doc, &mut view, FoldBatchAction::Focus { target: same });
    assert!(!view.is_collapsed(root) && !view.is_collapsed(code.id) && !view.is_collapsed(same));
    assert!(view.is_collapsed(a) && view.is_collapsed(title));
    view.set_preview(&doc, Some(b), &mut one()).unwrap();
    batch(&doc, &mut view, FoldBatchAction::All { collapsed: false });
    assert_eq!(view.preview(), Some(b));
    view.set_preview(&doc, None, &mut one()).unwrap();
    for id in [a, b, same, title, code.id, root] {
        assert!(!view.is_collapsed(id));
    }
    batch(&doc, &mut view, FoldBatchAction::All { collapsed: true });
    batch(
        &doc,
        &mut view,
        FoldBatchAction::AtLevel {
            level: 1,
            collapsed: false,
        },
    );
    batch(
        &doc,
        &mut view,
        FoldBatchAction::Containers {
            filter: ContainerFoldFilter::default(),
            collapsed: false,
        },
    );
    assert!(!view.is_collapsed(root));
    for id in [a, b, same, title] {
        assert!(view.is_collapsed(id));
    }
}

#[test]
fn anchors_navigation_placeholders_and_old_layout_work_are_consistent() {
    let doc = parse("# Root\nintro\n## Hidden\nbody\n## Visible\ntail\n");
    let mut view = FoldView::new(&doc, 1).unwrap();
    let start = doc.source().find("## Hidden").unwrap();
    let end = doc.source().find("## Visible").unwrap();
    let id = register(&doc, &mut view, Span::new(start, end)).unwrap();
    let target = doc.source().find("body").unwrap();
    let anchor = FoldAnchor::new(&doc, target).unwrap();
    let ticket = view.heights().ticket(TileId(0)).unwrap();
    let mut layout = LayoutPass::folded(&doc, &view, TileId(0), 10).unwrap();
    view.set_collapsed(&doc, id, true).unwrap();
    assert_eq!(
        view.measure(ticket, 100),
        Err(LayoutError::StaleMeasurement)
    );
    assert_eq!(
        layout.advance_folded(&doc, &mut view, &mut one(), &mut Sink::default()),
        Err(LayoutError::StaleContinuation)
    );
    assert!(
        anchor
            .resolve(&doc, &view, 10, &mut Budget::new(WorkLimit::default()))
            .unwrap()
            .is_none()
    );
    let mapped = anchor
        .resolve(&doc, &view, 10, &mut one())
        .unwrap()
        .unwrap();
    assert_eq!(mapped.hidden_by, Some(id));
    assert_eq!(mapped.target.source, Some(start));
    let mut origin = Navigator::new();
    let root_target = origin
        .navigate(&doc, view.heights(), NavAction::Heading(HeadingId(0)), 10)
        .unwrap();
    origin.apply_target(root_target);
    // Starting at the root skips the entire manual range and its hidden title.
    let mut nav = FoldNavigation::new(
        &doc,
        &view,
        &origin,
        NavAction::NextHeading { max_level: 6 },
        FoldNavigationPolicy::VisibleOnly,
        10,
    )
    .unwrap();
    loop {
        let p = nav.advance(&doc, &mut view, &mut one()).unwrap();
        if p.completion == Completion::Covered {
            assert_eq!(p.target.unwrap().source, Some(end));
            break;
        }
    }
    view.set_collapsed(&doc, FoldId::Heading(HeadingId(0)), true)
        .unwrap();
    let mut reveal = FoldNavigation::new(
        &doc,
        &view,
        &origin,
        NavAction::Source(target),
        FoldNavigationPolicy::Reveal,
        10,
    )
    .unwrap();
    let mut frames = 0;
    loop {
        frames += 1;
        let p = reveal.advance(&doc, &mut view, &mut one()).unwrap();
        if p.completion == Completion::Covered {
            assert_eq!(p.target.unwrap().source, Some(target));
            break;
        }
        assert!(frames < 10);
    }
    assert!(!view.is_collapsed(id));
    assert_eq!(
        anchor
            .resolve(&doc, &view, 10, &mut one())
            .unwrap()
            .unwrap()
            .hidden_by,
        None
    );
    view.set_collapsed(&doc, id, true).unwrap();
    let output = render(&doc, &mut view, true);
    assert_eq!(output.placeholders.len(), 1);
    assert_eq!(output.placeholders[0].candidate.id, id);
    assert!(!output.text.contains("Hidden") && output.text.contains("Visible"));
}

#[test]
fn fixed_ranges_survive_utf8_chunk_splits_late_setext_tables_and_merged_updates() {
    let prefix = "# Root\nold 中🙂 text\n\n";
    let suffix = "> next\r\n> title\r\n> ---\r\n> a|b\n> -|-\n> c|d\n\n# Next\n";
    let start = prefix.find('中').unwrap();
    let end = start + "中🙂".len();
    for split in (0..=suffix.len()).filter(|&i| suffix.is_char_boundary(i)) {
        let mut doc = Document::new();
        doc.append(prefix).unwrap();
        let mut view = FoldView::new(&doc, 2).unwrap();
        let id = register(&doc, &mut view, Span::new(start, end)).unwrap();
        let fixed = view.fold_candidate(&doc, id).unwrap().unwrap();
        view.set_collapsed(&doc, id, true).unwrap();
        let mut old = SourceFoldRegistration::new(&doc, &view, Span::new(end, end + 1)).unwrap();
        let mut update = doc.append(&suffix[..split]).unwrap();
        update.merge(doc.append(&suffix[split..]).unwrap()).unwrap();
        update.merge(doc.finish()).unwrap();
        view.apply(&doc, &update).unwrap();
        view.apply(&doc, &update).unwrap();
        assert_eq!(
            old.advance(&doc, &mut view, &mut one()),
            Err(LayoutError::StaleContinuation)
        );
        assert_eq!(view.fold_candidate(&doc, id).unwrap(), Some(fixed));
        assert_eq!(hidden(&doc, &view, start).unwrap().id, id);
        assert!(hidden(&doc, &view, end).is_none());
        let mut candidates = syntax(&doc);
        candidates.push(fixed);
        let expected = verify(&doc, &view, &candidates);
        assert_eq!(render(&doc, &mut view, true).text, expected);
    }
}

#[test]
fn mutable_candidates_are_excluded_until_locally_closed() {
    for source in [
        "pending",
        "paragraph\n",
        "> first\n> second\n",
        "a|b\n",
        "a|b\n---",
        "```\ncode\n",
        "$$\nmath\n",
        "|a|\n|-|\n|row|\n",
    ] {
        let mut doc = Document::new();
        let mut view = FoldView::new(&doc, 1).unwrap();
        for ch in source.chars() {
            let update = doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            view.apply(&doc, &update).unwrap();
            assert_eq!(doc.source_fold_limit(), 0, "{source:?} {:?}", doc.source());
            assert!(matches!(
                SourceFoldRegistration::new(&doc, &view, Span::new(0, doc.source().len())),
                Err(LayoutError::InvalidRange)
            ));
        }
        let update = doc.finish();
        view.apply(&doc, &update).unwrap();
        assert_eq!(doc.source_fold_limit(), source.len());
        register(&doc, &mut view, Span::new(0, source.len())).unwrap();
    }
    let mut doc = Document::new();
    doc.append("> first\n> second\n> ---\n> \n").unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let end = doc.source().find("> ---").unwrap();
    let id = register(&doc, &mut view, Span::new(2, end - 1)).unwrap();
    assert!(matches!(
        view.fold_candidate(&doc, id).unwrap().unwrap().parent,
        Some(FoldId::Heading(_))
    ));
}

#[test]
fn eof_tile_boundary_refreshes_once_and_does_not_hide_new_source() {
    for size in [240, 245, 250, 255, 256, 257, 500, 800] {
        let mut doc = Document::new();
        doc.append(&format!("{}\n\n", "x".repeat(size))).unwrap();
        let old_end = doc.source().len();
        assert_eq!(doc.source_fold_limit(), old_end);
        let mut view = FoldView::new(&doc, 2).unwrap();
        let tail = register(&doc, &mut view, Span::new(20, old_end)).unwrap();
        let head = register(&doc, &mut view, Span::new(0, 10)).unwrap();
        view.set_collapsed(&doc, tail, true).unwrap();
        view.set_collapsed(&doc, head, true).unwrap(); // must not clear tail refresh slot
        let before = view.range_visits();
        let update = doc.append("🙂 new\n\n").unwrap();
        view.apply(&doc, &update).unwrap();
        assert_eq!(view.range_visits() - before, 1);
        assert!(hidden(&doc, &view, old_end).is_none());
        let fixed = [head, tail].map(|id| view.fold_candidate(&doc, id).unwrap().unwrap());
        let expected = verify(&doc, &view, &fixed);
        assert_eq!(render(&doc, &mut view, true).text, expected);
        let before = view.range_visits();
        let update = doc.append("more\n").unwrap();
        view.apply(&doc, &update).unwrap();
        assert_eq!(view.range_visits(), before);
    }
}

#[test]
fn randomized_interval_height_layout_and_preview_model() {
    let mut source = String::from("# Root\n");
    for i in 0..16 {
        source.push_str(&format!(
            "## Title{i}\n> before {}\n> ```\n> {}\n> ```\n\n",
            "x".repeat(i * 21),
            "y".repeat(i * 32)
        ));
    }
    let doc = parse(&source);
    let mut view = FoldView::new(&doc, 3).unwrap();
    let mut candidates = syntax(&doc);
    for i in 0..16 {
        let at = source.find(&format!("## Title{i}\n")).unwrap();
        let end = at + format!("## Title{i}\n").len();
        let id = register(&doc, &mut view, Span::new(at + 3, end - 1)).unwrap();
        candidates.push(view.fold_candidate(&doc, id).unwrap().unwrap());
        let at = source[end..].find("before").unwrap() + end;
        let id = register(&doc, &mut view, Span::new(at, at + 6)).unwrap();
        candidates.push(view.fold_candidate(&doc, id).unwrap().unwrap());
    }
    let mut seed = 83u64;
    for turn in 0..1000 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let id = candidates[(seed >> 32) as usize % candidates.len()].id;
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
        let expected = verify(&doc, &view, &candidates);
        if turn % 40 == 0 {
            assert_eq!(render(&doc, &mut view, false).text, expected);
        }
    }
}

#[test]
fn historical_registrations_do_not_grow_append_work_or_retain_removed_ids() {
    let mut doc = Document::new();
    doc.append(&format!("{}\n\n", "abcdefgh".repeat(8192)))
        .unwrap();
    let mut view = FoldView::new(&doc, 2).unwrap();
    let mut ids = Vec::new();
    for start in (0..65536).step_by(16) {
        let id = register(&doc, &mut view, Span::new(start, start + 8)).unwrap();
        view.set_collapsed(&doc, id, true).unwrap();
        ids.push(id);
    }
    assert_eq!(view.source_fold_count(), 4096);
    let visits = view.range_visits();
    doc.append(&"x".repeat(1_000_000))
        .map(|update| view.apply(&doc, &update).unwrap())
        .unwrap();
    let work = doc.work();
    for _ in 0..1000 {
        let update = doc.append("x").unwrap();
        view.apply(&doc, &update).unwrap();
    }
    assert_eq!(view.range_visits(), visits);
    assert_eq!(view.source_fold_count(), 4096);
    assert_eq!(doc.work().container_steps, work.container_steps);
    assert_eq!(doc.work().heading_steps, work.heading_steps);
    for id in ids {
        remove(&doc, &mut view, id);
    }
    assert_eq!(view.source_fold_count(), 0);
    for _ in 0..1000 {
        let id = register(&doc, &mut view, Span::new(0, 8)).unwrap();
        view.toggle(&doc, id).unwrap();
        remove(&doc, &mut view, id);
    }
    assert_eq!(view.source_fold_count(), 0);
}

#[test]
fn million_byte_fold_skips_body_objects_and_summary_uses_shared_budget() {
    let source = format!("before\n\n{}![image](/x)\n\nafter\n", "x".repeat(1_000_000));
    let doc = parse(&source);
    let mut view = FoldView::new(&doc, 1).unwrap();
    let end = source.find("after").unwrap();
    let id = register(&doc, &mut view, Span::new(8, end)).unwrap();
    view.set_collapsed(&doc, id, true).unwrap();
    let c = view.fold_candidate(&doc, id).unwrap().unwrap();
    let mut budget = Budget::new(WorkLimit {
        queries: 2,
        objects: 1,
        bytes: 4,
        ..Default::default()
    });
    assert!(matches!(
        view.hidden_at(&doc, 100, &mut budget).unwrap(),
        FoldLookup::Hidden(_)
    ));
    assert_eq!(c.summary_text(&doc, 4, &mut budget), Some("xxxx"));
    assert_eq!(c.summary_text(&doc, 1, &mut budget), None);
    assert_eq!(budget.used().bytes, 4);
    let sink = render(&doc, &mut view, true);
    assert_eq!(sink.text, "before\n\n[fold]\nafter\n");
    assert_eq!(sink.objects, 0);
    assert_eq!(sink.placeholders.len(), 1);
    assert!(sink.bytes < 32);
}

#[test]
fn enclosing_manual_at_subtree_boundary_does_not_change_ancestor_choice() {
    let doc = parse("```\ncode\n```\n\nrest\n");
    let mut view = FoldView::new(&doc, 1).unwrap();
    let outer = register(&doc, &mut view, Span::new(0, doc.source().len())).unwrap();
    view.set_collapsed(&doc, outer, true).unwrap();
    let code = FoldId::Code(BlockId(0));
    assert_eq!(
        batch(
            &doc,
            &mut view,
            FoldBatchAction::Subtree {
                root: code,
                collapsed: true
            }
        )
        .changed,
        1
    );
    assert!(view.is_collapsed(outer));
    batch(
        &doc,
        &mut view,
        FoldBatchAction::Subtree {
            root: outer,
            collapsed: false,
        },
    );
    assert!(!view.is_collapsed(outer) && !view.is_collapsed(code));
    batch(&doc, &mut view, FoldBatchAction::Focus { target: code });
    assert!(!view.is_collapsed(outer));
}

#[test]
fn every_live_prefix_keeps_accepted_ranges_compatible_with_future_syntax() {
    let sources = [
        "# Root\nold\n\n> first\n> second\n> ---\n> \n> | a |\n> | - |\n> | b |\n\n# End\ntail\n",
        "- item\n  \n  > ```\n  > code\n  > ```\n\n- sibling\n\n$$\nmath\n$$\n\nend\n",
        "text\n| x |\n| ---- |\n| y |\n\n> multi\n> line\n> ===\n> body\n\nend\n",
        "[^a]: note\n    body\n\n    next\nend\n\n> a|b\n> ----x\n> next\n> \nend\n",
    ];
    for source in sources {
        let mut doc = Document::new();
        let mut view = FoldView::new(&doc, 2).unwrap();
        let mut accepted = Vec::new();
        let mut previous_limit = 0;
        for ch in source.chars() {
            let update = doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            view.apply(&doc, &update).unwrap();
            let limit = doc.source_fold_limit();
            assert!(limit >= previous_limit);
            previous_limit = limit;
            if limit == 0 {
                continue;
            }
            for start in [0, limit / 2, limit - 1] {
                let span = Span::new(start, limit);
                if let Ok(id) = register(&doc, &mut view, span) {
                    accepted.push(span);
                    remove(&doc, &mut view, id);
                }
            }
            for span in &accepted {
                assert!(
                    syntax(&doc)
                        .iter()
                        .all(|c| compatible(*span, c.source) && compatible(*span, c.hidden)),
                    "prefix {:?}, {span:?}",
                    doc.source()
                );
            }
        }
        doc.finish();
        for span in accepted {
            assert!(
                syntax(&doc)
                    .iter()
                    .all(|c| compatible(span, c.source) && compatible(span, c.hidden))
            );
        }
    }
}

#[test]
fn streaming_registration_visibility_height_model_and_checkpoints() {
    let mut doc = Document::new();
    doc.append("# Root\n").unwrap();
    let mut view = FoldView::new(&doc, 2).unwrap();
    let mut fixed = Vec::new();
    for turn in 0..240 {
        let start = doc.source().len();
        let update = doc
            .append(&format!("text {turn} {}\n\n", "x".repeat(turn % 11 * 43)))
            .unwrap();
        view.apply(&doc, &update).unwrap();
        let id = register(&doc, &mut view, Span::new(start, doc.source().len() - 1)).unwrap();
        fixed.push(view.fold_candidate(&doc, id).unwrap().unwrap());
        view.insert_checkpoint(TileId(0), ()).unwrap();
        view.set_collapsed(&doc, id, turn % 3 != 0).unwrap();
        if turn % 7 == 0 {
            view.set_preview(&doc, Some(id), &mut one()).unwrap();
        }
        if turn % 11 == 0 {
            view.reset(3).unwrap();
        }
        if turn % 17 == 0 {
            view.toggle(&doc, FoldId::Heading(HeadingId(0))).unwrap();
        }
        let candidates = syntax(&doc)
            .into_iter()
            .chain(fixed.iter().copied())
            .collect::<Vec<_>>();
        let expected = verify(&doc, &view, &candidates);
        if turn % 20 == 0 {
            assert_eq!(render(&doc, &mut view, false).text, expected);
        }
    }
    let update = doc.finish();
    view.apply(&doc, &update).unwrap();
    let candidates = syntax(&doc).into_iter().chain(fixed).collect::<Vec<_>>();
    let expected = verify(&doc, &view, &candidates);
    assert_eq!(render(&doc, &mut view, true).text, expected);
}

#[test]
fn source_and_closed_literal_share_eof_without_leaving_a_stale_tile_mask() {
    for fence in ["```", "$$"] {
        let mut doc = Document::new();
        doc.append(&format!("{fence}\n{}\n{fence}\n", "x".repeat(800)))
            .unwrap();
        let old_end = doc.source().len();
        let literal = doc.fold_candidate_at(0).unwrap();
        let mut view = FoldView::new(&doc, 2).unwrap();
        let source = register(&doc, &mut view, Span::new(0, old_end)).unwrap();
        let fixed = view.fold_candidate(&doc, source).unwrap().unwrap();
        view.set_collapsed(&doc, literal.id, true).unwrap();
        view.set_collapsed(&doc, source, true).unwrap();
        let before = view.range_visits();
        let update = doc.append("appended 🙂\n\n").unwrap();
        view.apply(&doc, &update).unwrap();
        let expected = verify(&doc, &view, &[literal, fixed]);
        assert_eq!(view.range_visits() - before, 2);
        assert_eq!(render(&doc, &mut view, true).text, expected);
        view.set_collapsed(&doc, literal.id, false).unwrap();
        assert_eq!(hidden(&doc, &view, 100).unwrap().id, source);
        assert!(hidden(&doc, &view, old_end).is_none());
        let before = view.range_visits();
        let update = doc.append("later\n").unwrap();
        view.apply(&doc, &update).unwrap();
        assert_eq!(view.range_visits(), before);
    }
}

fn complete_source_task(
    doc: &Document,
    view: &mut FoldView,
    mut task: SourceFoldRegistration,
) -> Result<FoldId, LayoutError> {
    for _ in 0..doc.container_count() * 4 + doc.headings().entries().len() * 2 + 20 {
        let p = task.advance(doc, view, &mut one())?;
        if let Some(id) = p.id {
            return Ok(id);
        }
    }
    panic!("source task stalled")
}
fn tail(doc: &Document, view: &mut FoldView, start: usize) -> FoldId {
    let task = SourceFoldRegistration::tail(doc, view, start).unwrap();
    complete_source_task(doc, view, task).unwrap()
}
fn seal(doc: &Document, view: &mut FoldView, id: FoldId) -> Result<FoldId, LayoutError> {
    let FoldId::Source(id) = id else { panic!() };
    let task = SourceFoldRegistration::seal_tail(doc, view, id)?;
    complete_source_task(doc, view, task)
}

#[test]
fn root_tail_grows_real_registry_mask_and_finishes_with_same_id() {
    let mut doc = Document::new();
    doc.append("lead 中🙂\n\n").unwrap();
    let mut view = FoldView::new(&doc, 3).unwrap();
    let id = tail(&doc, &mut view, 5);
    assert!(!view.is_collapsed(id));
    view.set_collapsed(&doc, id, true).unwrap();
    let old_eof = FoldAnchor::new(&doc, doc.source().len()).unwrap();
    for chunk in ["# H\n", "> body\n", "x\n"].into_iter().chain(["z"; 300]) {
        let update = doc.append(chunk).unwrap();
        view.apply(&doc, &update).unwrap();
        let c = view.fold_candidate(&doc, id).unwrap().unwrap();
        assert_eq!(c.source, Span::new(5, doc.source().len()));
        assert_eq!(c.hidden, c.source);
        assert_eq!(c.summary, Some(c.source));
        assert_eq!(c.parent, None);
        assert!(c.open);
        assert_eq!(view.source_fold_at(&doc, c.source.end).unwrap(), None);
        let expected = verify(&doc, &view, &[c]);
        assert_eq!(render(&doc, &mut view, true).text, expected);
    }
    assert_eq!(
        old_eof
            .resolve(&doc, &view, 10, &mut one())
            .unwrap()
            .unwrap()
            .hidden_by,
        Some(id)
    );
    let update = doc.finish();
    view.apply(&doc, &update).unwrap();
    let c = view.fold_candidate(&doc, id).unwrap().unwrap();
    assert!(!c.open);
    assert_eq!(c.source.end, doc.source().len());
    assert_eq!(view.source_fold_count(), 1);
    assert_eq!(render(&doc, &mut view, true).placeholders.len(), 1);
}

#[test]
fn root_tail_seals_same_id_and_refreshes_fixed_eof_tile_once() {
    for size in [240, 255, 256, 257, 800] {
        let mut doc = Document::new();
        doc.append(&format!("{}\n\n", "x".repeat(size))).unwrap();
        let mut view = FoldView::new(&doc, 2).unwrap();
        let id = tail(&doc, &mut view, 5);
        view.set_collapsed(&doc, id, true).unwrap();
        let end = doc.source().len();
        let version = view.version();
        assert_eq!(seal(&doc, &mut view, id), Ok(id));
        assert_eq!(view.version(), version + 1);
        assert!(!view.fold_candidate(&doc, id).unwrap().unwrap().open);
        assert_eq!(seal(&doc, &mut view, id), Err(LayoutError::InvalidRange));
        let before = view.range_visits();
        let update = doc.append("🙂 new\n\n").unwrap();
        view.apply(&doc, &update).unwrap();
        assert_eq!(view.range_visits() - before, 1);
        let c = view.fold_candidate(&doc, id).unwrap().unwrap();
        assert_eq!(c.source.end, end);
        assert_eq!(hidden(&doc, &view, end), None);
        let expected = verify(&doc, &view, &[c]);
        assert_eq!(render(&doc, &mut view, true).text, expected);
        let before = view.range_visits();
        let update = doc.append("more").unwrap();
        view.apply(&doc, &update).unwrap();
        assert_eq!(view.range_visits(), before);
    }
}

// Parser snapshots are typed public API values from a separate, unfolded
// document receiving exactly the same chunks. Expected output is never written.
fn root_tail_model(doc: &Document, baseline: &Document, view: &mut FoldView, id: FoldId) {
    let range = Span::new(0, doc.source().len());
    assert_eq!(doc.source(), baseline.source());
    assert_eq!(
        doc.runs_in(range).unwrap().collect::<Vec<_>>(),
        baseline.runs_in(range).unwrap().collect::<Vec<_>>()
    );
    assert_eq!(
        doc.display_runs_in(range).unwrap().collect::<Vec<_>>(),
        baseline.display_runs_in(range).unwrap().collect::<Vec<_>>()
    );
    let original = syntax(baseline);
    assert_eq!(syntax(doc), original); // Includes actual syntax parents and IDs.
    let c = view.fold_candidate(doc, id).unwrap().unwrap();
    assert_eq!(c.source, Span::new(5, doc.source().len()));
    assert_eq!(
        (c.hidden, c.summary, c.parent, c.open),
        (c.source, Some(c.source), None, !doc.is_finished())
    );
    let FoldId::Source(source_id) = id else {
        panic!()
    };
    assert_eq!(view.paragraph_fold_block(doc, source_id).unwrap(), None);
    assert!(original.iter().all(|s| s.source.start >= 5));
    let candidates = original.into_iter().chain([c]).collect::<Vec<_>>();
    let expected = verify(doc, view, &candidates);
    // Exact point/anchor model, including old EOFs and the half-open current EOF.
    for at in (0..=doc.source().len()).filter(|&at| doc.source().is_char_boundary(at)) {
        let expected = candidates
            .iter()
            .filter(|s| {
                view.is_collapsed(s.id)
                    && view.preview() != Some(s.id)
                    && s.hidden.range().contains(&at)
            })
            .min_by_key(|s| (s.hidden.start, std::cmp::Reverse(s.hidden.end), s.id));
        assert_eq!(
            hidden(doc, view, at).map(|s| s.hidden),
            expected.map(|s| s.hidden)
        );
        let a = FoldAnchor::new(doc, at)
            .unwrap()
            .resolve(doc, view, 10, &mut one())
            .unwrap()
            .unwrap();
        assert_eq!(
            a.target.source,
            Some(expected.map_or(at, |s| s.hidden.start))
        );
        assert_eq!(
            view.source_fold_at(doc, at).unwrap(),
            c.source.range().contains(&at).then_some(c)
        );
    }
    assert_eq!(render(doc, view, true).text, expected);
    let mut window =
        SourceFoldCandidates::new(doc, view, Span::new(c.source.end - 1, c.source.end)).unwrap();
    let mut out = Vec::new();
    assert_eq!(
        window
            .advance(doc, view, &mut Budget::new(WorkLimit::default()), &mut out)
            .unwrap(),
        Completion::BudgetExhausted
    );
    assert!(out.is_empty());
    let mut budget = one();
    assert_eq!(
        window.advance(doc, view, &mut budget, &mut out).unwrap(),
        Completion::Covered
    );
    assert_eq!(out, [c]);
    assert_eq!(
        (
            budget.used().queries,
            budget.used().objects,
            budget.used().bytes
        ),
        (1, 1, 0)
    );
}

// Deliberately linear fixed-range oracle: source/hidden laminarity and future
// growth crossing, without endpoint lookup or ancestry traversal.
fn root_tail_can_seal(doc: &Document) -> bool {
    let span = Span::new(5, doc.source().len());
    span.end <= doc.source_fold_limit()
        && syntax(doc).iter().all(|c| {
            [c.source, c.hidden].into_iter().all(|s| {
                compatible(span, s) && !(c.open && span.start < s.start && s.start < span.end)
            })
        })
}

#[test]
fn root_tail_eight_corpora_every_utf8_cut_prefix_eof_and_typed_model() {
    let corpora = [
        "# H\n> body\n> ```\n> code\n> ```\n\n$$\nmath\n$$\n",
        "title\n---\nbody\n",
        "before\nh|v\n-|-\na|b\n\n",
        "> multi\n> line\n> ===\n> \n> h|v\n> -|-\n> a|b\n",
        "[r]: /url\n  \"title\"\n\n[r]\n",
        "[r]: /url\nnot title\n\n[x](url\n \"a\nb\")\n",
        "> `a\n> b` &amp; $a\n> b$ [a\n> b](/u)\n",
        "中🙂\r\n\r\n> h|v\r\n> -|-\r\n> pending🙂",
    ];
    for body in corpora {
        for cut in (0..=body.len()).filter(|&n| body.is_char_boundary(n)) {
            for prefix_eof in [false, true] {
                let mut doc = Document::new();
                let mut baseline = Document::new();
                for d in [&mut doc, &mut baseline] {
                    d.append("lead 中🙂\n\n").unwrap();
                }
                let mut view = FoldView::new(&doc, 2).unwrap();
                let id = tail(&doc, &mut view, 5);
                view.set_collapsed(&doc, id, true).unwrap();
                let update = doc.append(&body[..cut]).unwrap();
                baseline.append(&body[..cut]).unwrap();
                view.apply(&doc, &update).unwrap();
                root_tail_model(&doc, &baseline, &mut view, id);
                // Exercise the real seal at every prefix with a fresh view.
                let mut sealing = FoldView::new(&doc, 2).unwrap();
                let seal_id = tail(&doc, &mut sealing, 5);
                let before = sealing.version();
                let accepted = root_tail_can_seal(&doc);
                assert_eq!(
                    seal(&doc, &mut sealing, seal_id).is_ok(),
                    accepted,
                    "{body:?} cut={cut}"
                );
                assert_eq!(sealing.version(), before + u64::from(accepted));
                assert_eq!(
                    sealing.fold_candidate(&doc, seal_id).unwrap().unwrap().open,
                    !accepted
                );
                if !prefix_eof {
                    let update = doc.append(&body[cut..]).unwrap();
                    baseline.append(&body[cut..]).unwrap();
                    view.apply(&doc, &update).unwrap();
                    view.set_preview(&doc, Some(id), &mut one()).unwrap();
                    root_tail_model(&doc, &baseline, &mut view, id);
                    view.set_preview(&doc, None, &mut one()).unwrap();
                }
                let update = doc.finish();
                baseline.finish();
                view.apply(&doc, &update).unwrap();
                root_tail_model(&doc, &baseline, &mut view, id);
            }
        }
    }
}

#[test]
fn root_tail_rejects_unstable_utf8_and_all_syntax_host_envelopes() {
    for text in ["", "pending", "title\n", "h|v\n", "[r]: /u\n", "> title\n"] {
        let mut doc = Document::new();
        doc.append(text).unwrap();
        let view = FoldView::new(&doc, 1).unwrap();
        for start in [doc.source_fold_limit(), doc.source().len(), usize::MAX] {
            assert!(matches!(
                SourceFoldRegistration::tail(&doc, &view, start),
                Err(LayoutError::InvalidRange)
            ));
        }
    }
    for body in [
        "# H\nbody\n\n",
        "> body\n> \n",
        "```\ncode\n```\n\n",
        "$$\nmath\n$$\n\n",
        "h|v\n-|-\na|b\n\n",
        "- body\n\n",
    ] {
        let mut doc = Document::new();
        doc.append(&format!("lead 中🙂\n\n{body}")).unwrap();
        let mut view = FoldView::new(&doc, 1).unwrap();
        for c in syntax(&doc) {
            for start in [
                c.source.start,
                c.source.end.saturating_sub(1),
                c.hidden.start,
            ] {
                if start >= doc.source_fold_limit() || !doc.source().is_char_boundary(start) {
                    continue;
                }
                if doc.fold_candidate_at(start).is_none() {
                    continue;
                }
                let mut task = SourceFoldRegistration::tail(&doc, &view, start).unwrap();
                let before = view.version();
                let mut budget = one();
                assert_eq!(
                    task.advance(&doc, &mut view, &mut budget),
                    Err(LayoutError::InvalidRange)
                );
                assert_eq!(budget.used(), one().remaining());
                assert_eq!((view.version(), view.source_fold_count()), (before, 0));
            }
        }
        for start in [6, 7, 9, 10, 11] {
            // Inside 中 or 🙂.
            assert!(!doc.source().is_char_boundary(start));
            assert!(matches!(
                SourceFoldRegistration::tail(&doc, &view, start),
                Err(LayoutError::InvalidRange)
            ));
        }
        doc.finish();
        let view = FoldView::new(&doc, 1).unwrap();
        assert!(matches!(
            SourceFoldRegistration::tail(&doc, &view, 5),
            Err(LayoutError::InvalidRange)
        ));
    }
}

#[test]
fn root_tail_shared_budget_registration_and_conditional_seal_are_atomic() {
    for amount in [0, 1, 6] {
        let mut doc = Document::new();
        doc.append("lead 中🙂\n\n").unwrap();
        let mut view = FoldView::new(&doc, 2).unwrap();
        let before = view.version();
        let mut task = SourceFoldRegistration::tail(&doc, &view, 5).unwrap();
        for limit in [
            WorkLimit::default(),
            WorkLimit {
                queries: 6,
                ..Default::default()
            },
            WorkLimit {
                objects: 6,
                ..Default::default()
            },
        ] {
            let mut b = Budget::new(limit);
            assert_eq!(
                task.advance(&doc, &mut view, &mut b).unwrap().completion,
                Completion::BudgetExhausted
            );
            assert_eq!(b.used(), WorkLimit::default());
            assert_eq!((view.version(), view.source_fold_count()), (before, 0));
        }
        let mut b = Budget::new(WorkLimit {
            queries: amount,
            objects: amount,
            ..Default::default()
        });
        let p = task.advance(&doc, &mut view, &mut b).unwrap();
        assert_eq!(b.used().queries, amount.min(2));
        assert_eq!(b.used().objects, amount.min(2));
        if amount < 2 {
            assert_eq!(view.source_fold_count(), 0);
        }
        let id = if let Some(id) = p.id {
            id
        } else {
            complete_source_task(&doc, &mut view, task).unwrap()
        };
        view.set_collapsed(&doc, id, true).unwrap();
        view.set_preview(&doc, Some(id), &mut one()).unwrap();
        let FoldId::Source(sid) = id else { panic!() };
        let before = view.version();
        let mut seal_task = SourceFoldRegistration::seal_tail(&doc, &view, sid).unwrap();
        let mut b = Budget::new(WorkLimit {
            queries: 6,
            objects: 6,
            ..Default::default()
        });
        let p = seal_task.advance(&doc, &mut view, &mut b).unwrap();
        assert_eq!(p.id, Some(id));
        assert_eq!(
            (b.used().queries, b.used().objects, b.used().bytes),
            (5, 5, 0)
        );
        assert_eq!(view.version(), before + 1);
        assert_eq!(view.preview(), Some(id));
        assert!(view.is_collapsed(id));
        // The remaining shared frame can perform exactly one anchor operation.
        assert!(FoldAnchor::new(&doc, 6).unwrap_err() == LayoutError::InvalidRange);
        assert!(
            FoldAnchor::new(&doc, 5)
                .unwrap()
                .resolve(&doc, &view, 10, &mut b)
                .unwrap()
                .is_some()
        );
        assert_eq!(b.used().queries, 6);
        assert_eq!(
            view.set_preview(&doc, None, &mut b).unwrap().completion,
            Completion::BudgetExhausted
        );
        view.set_preview(&doc, None, &mut one()).unwrap();
        assert_eq!(hidden(&doc, &view, 5).unwrap().id, id);
    }
    // Stable EOF is necessary but not sufficient: an open heading inside the
    // tail can cross a fixed end on future append. Fail after paid validation.
    let mut doc = Document::new();
    doc.append("lead text\n\n").unwrap();
    let mut view = FoldView::new(&doc, 2).unwrap();
    let id = tail(&doc, &mut view, 5);
    for suffix in ["pending", "\n\n# H\n"] {
        let update = doc.append(suffix).unwrap();
        view.apply(&doc, &update).unwrap();
        let before = (
            view.version(),
            view.heights().mutation_visits(),
            view.fold_candidate(&doc, id).unwrap(),
        );
        let result = seal(&doc, &mut view, id);
        assert_eq!(result, Err(LayoutError::InvalidRange));
        assert_eq!(
            (
                view.version(),
                view.heights().mutation_visits(),
                view.fold_candidate(&doc, id).unwrap()
            ),
            before
        );
    }
}

#[test]
fn root_tail_future_syntax_rows_defaults_batches_and_preview_keep_overlay_parents() {
    let mut doc = Document::new();
    doc.append("lead text\n\n").unwrap();
    let mut view = FoldView::new(&doc, 2).unwrap();
    let id = tail(&doc, &mut view, 5);
    view.set_defaults(FoldDefaults {
        heading_levels: [true; 6],
        code: true,
        math: true,
        table: true,
        container_depth: Some(1),
        ..Default::default()
    });
    let mut other = FoldView::new(&doc, 2).unwrap();
    let other_id = tail(&doc, &mut other, 5);
    assert_ne!(id, other_id);
    assert_eq!(other.fold_candidate(&doc, id).unwrap(), None);
    let update = doc
        .append("# H\nintro\n> body\n> ```\n> code\n> ```\n> \n> h|v\n> -|-\n> a|b\n> c|d")
        .unwrap();
    view.apply(&doc, &update).unwrap();
    other.apply(&doc, &update).unwrap();
    let mut all = syntax(&doc);
    assert!(all.iter().all(|s| view.is_collapsed(s.id)));
    let table = all
        .iter()
        .find_map(|s| {
            if let FoldId::Table(t) = s.id {
                Some(t)
            } else {
                None
            }
        })
        .unwrap();
    // Complete registered rows keep their Table parent inside the overlay.
    let rows = view
        .insert_table_rows(&doc, table, 1..doc.table(table).unwrap().rows().len())
        .unwrap();
    let row = view.fold_candidate(&doc, rows).unwrap().unwrap();
    assert_eq!(row.parent, Some(FoldId::Table(table)));
    all.push(row);
    let parents: Vec<_> = all.iter().map(|c| (c.id, c.parent)).collect();
    view.set_collapsed(&doc, rows, true).unwrap();
    for target in [id, FoldId::Heading(HeadingId(0)), rows] {
        batch(&doc, &mut view, FoldBatchAction::Focus { target });
        let target_c = view.fold_candidate(&doc, target).unwrap().unwrap();
        for c in all
            .iter()
            .copied()
            .chain([view.fold_candidate(&doc, id).unwrap().unwrap()])
        {
            let ancestor = c.id == target
                || (contains(c.source, target_c.source) && c.source != target_c.source);
            assert_eq!(
                view.is_collapsed(c.id),
                !ancestor,
                "focus {target:?} candidate {c:?}"
            );
        }
        let c = view.fold_candidate(&doc, id).unwrap().unwrap();
        let candidates = all.iter().copied().chain([c]).collect::<Vec<_>>();
        let expected = verify(&doc, &view, &candidates);
        assert_eq!(render(&doc, &mut view, true).text, expected);
    }
    batch(
        &doc,
        &mut view,
        FoldBatchAction::Subtree {
            root: id,
            collapsed: false,
        },
    );
    assert!(all.iter().all(|c| !view.is_collapsed(c.id)));
    batch(
        &doc,
        &mut view,
        FoldBatchAction::Siblings {
            target: id,
            collapsed: true,
        },
    );
    for c in &all {
        assert_eq!(view.is_collapsed(c.id), c.parent.is_none());
    }
    for (candidate, parent) in parents {
        assert_eq!(
            view.fold_candidate(&doc, candidate)
                .unwrap()
                .unwrap()
                .parent,
            parent
        );
    }
    view.set_collapsed(&doc, id, true).unwrap();
    view.set_preview(&doc, Some(id), &mut one()).unwrap();
    assert!(view.is_collapsed(id));
    assert!(hidden(&doc, &view, doc.source().find("body").unwrap()).is_some()); // inner heading still collapsed
    view.set_collapsed(&doc, id, false).unwrap();
    view.reset(3).unwrap();
    assert_eq!(view.preview(), Some(id));
    let update = doc.append("> more\n").unwrap();
    view.apply(&doc, &update).unwrap();
    other.apply(&doc, &update).unwrap();
    assert_eq!(
        view.fold_candidate(&doc, id).unwrap().unwrap().source.end,
        doc.source().len()
    );
    let update = doc.finish();
    view.apply(&doc, &update).unwrap();
    other.apply(&doc, &update).unwrap();
    view.set_preview(&doc, None, &mut one()).unwrap();
    assert!(!view.is_collapsed(id));
    assert!(!other.is_collapsed(other_id));
    view.set_collapsed(&doc, id, true).unwrap();
    view.set_preview(&doc, Some(id), &mut one()).unwrap();
    remove(&doc, &mut view, id);
    assert_eq!(view.preview(), None);
    assert_eq!(view.fold_candidate(&doc, id).unwrap(), None);
    assert!(!view.is_collapsed(id));
    assert_eq!(view.source_fold_count(), 0);
}

#[test]
fn root_tail_overlap_all_source_modes_both_orders_and_auto_recovery() {
    for preview in [false, true] {
        let mut doc = Document::new();
        doc.append("lead text\n\nsecond body\n\n> closed\n> \n> open\n")
            .unwrap();
        let mut view = FoldView::new(&doc, 2).unwrap();
        let id = tail(&doc, &mut view, 5);
        if preview {
            view.set_preview(&doc, Some(id), &mut one()).unwrap();
        }
        let head = register(&doc, &mut view, Span::new(0, 4)).unwrap();
        assert_ne!(head, id);
        assert!(matches!(
            SourceFoldRegistration::tail(&doc, &view, 5),
            Err(LayoutError::InvalidRange)
        ));
        assert!(matches!(
            SourceFoldRegistration::new(&doc, &view, Span::new(6, 8)),
            Err(LayoutError::InvalidRange)
        ));
        assert!(matches!(
            SourceFoldRegistration::paragraph(&doc, &view, BlockId(1), 1),
            Err(LayoutError::InvalidRange)
        ));
        let first = doc
            .blocks()
            .iter()
            .enumerate()
            .find(|(_, b)| {
                b.container.is_some() && b.source.start == doc.source().find("> closed").unwrap()
            })
            .map(|(i, _)| BlockId(i))
            .unwrap();
        assert!(matches!(
            SourceFoldRegistration::container_paragraph(&doc, &view, first, 1),
            Err(LayoutError::InvalidRange)
        ));
        view.set_defaults(FoldDefaults {
            paragraphs: Some(ParagraphFoldPolicy {
                min_bytes: 0,
                preview_bytes: 1,
            }),
            ..Default::default()
        });
        let mut auto =
            ParagraphAutoFold::new(&doc, &view, Span::new(0, doc.source().len())).unwrap();
        let progress = loop {
            let p = auto.advance(&doc, &mut view, &mut one()).unwrap();
            if p.completion == Completion::Covered {
                break p;
            }
        };
        assert_eq!(progress.registered, 0);
        assert!(progress.skipped >= 2);
        remove(&doc, &mut view, id);
        let mut auto =
            ParagraphAutoFold::new(&doc, &view, Span::new(0, doc.source().len())).unwrap();
        let progress = loop {
            let p = auto.advance(&doc, &mut view, &mut one()).unwrap();
            if p.completion == Completion::Covered {
                break p;
            }
        };
        assert!(progress.registered >= 2); // removing tail added no paragraph exclusions
        assert!(matches!(
            SourceFoldRegistration::tail(&doc, &view, 5),
            Err(LayoutError::InvalidRange)
        ));
    }
    for mode in 0..4 {
        let mut doc = Document::new();
        doc.append("lead text\n\nsecond body\n").unwrap();
        let mut view = FoldView::new(&doc, 1).unwrap();
        let task = match mode {
            0 => SourceFoldRegistration::new(&doc, &view, Span::new(6, 8)),
            1 => SourceFoldRegistration::paragraph(&doc, &view, BlockId(0), 1),
            2 => SourceFoldRegistration::open_paragraph(
                &doc,
                &view,
                doc.block_at(doc.source().find("second body").unwrap())
                    .unwrap(),
                1,
            ),
            _ => {
                let u = doc.append("\n> body\n").unwrap();
                view.apply(&doc, &u).unwrap();
                let b = doc.block_at(doc.source().find("> body").unwrap()).unwrap();
                SourceFoldRegistration::container_paragraph(&doc, &view, b, 1)
            }
        }
        .unwrap_or_else(|e| panic!("mode {mode}: {e:?}"));
        let id = complete_source_task(&doc, &mut view, task).unwrap();
        view.set_preview(&doc, Some(id), &mut one()).unwrap();
        assert!(matches!(
            SourceFoldRegistration::tail(&doc, &view, 5),
            Err(LayoutError::InvalidRange)
        ));
    }
}

#[test]
fn root_tail_updates_tasks_replay_merged_and_foreign_errors_are_precise() {
    let mut doc = Document::new();
    doc.append("lead text\n\n").unwrap();
    let mut view = FoldView::new(&doc, 2).unwrap();
    let mut task = SourceFoldRegistration::tail(&doc, &view, 5).unwrap();
    let mut other = FoldView::new(&doc, 2).unwrap();
    assert_eq!(
        task.advance(&doc, &mut other, &mut one()),
        Err(LayoutError::WrongView)
    );
    let foreign = parse("lead text\n\n");
    assert_eq!(
        task.advance(&foreign, &mut view, &mut one()),
        Err(LayoutError::WrongDocument)
    );
    assert_eq!(task.advance(&doc, &mut view, &mut one()).unwrap().id, None);
    view.reset(2).unwrap();
    assert_eq!(
        task.advance(&doc, &mut view, &mut one()),
        Err(LayoutError::StaleContinuation)
    );
    let id = tail(&doc, &mut view, 5);
    let FoldId::Source(sid) = id else { panic!() };
    assert!(matches!(
        SourceFoldRegistration::seal_tail(&doc, &other, sid),
        Err(LayoutError::InvalidRange)
    ));
    let mut seal_task = SourceFoldRegistration::seal_tail(&doc, &view, sid).unwrap();
    let old_version = view.version();
    let first = doc.append("a").unwrap();
    assert_eq!(
        seal_task.advance(&doc, &mut view, &mut one()),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(
        view.fold_candidate(&doc, id),
        Err(LayoutError::OutOfOrderUpdate)
    );
    let second = doc.append("中\n\n").unwrap();
    assert_eq!(view.apply(&doc, &first), Err(LayoutError::OutOfOrderUpdate));
    assert_eq!(
        view.apply(&doc, &second),
        Err(LayoutError::OutOfOrderUpdate)
    );
    assert_eq!(view.version(), old_version);
    let mut merged = first.clone();
    merged.merge(second.clone()).unwrap();
    let before = view.range_visits();
    view.apply(&doc, &merged).unwrap();
    assert_eq!(view.range_visits() - before, 1);
    let frozen = (
        view.version(),
        view.range_visits(),
        view.heights().mutation_visits(),
    );
    view.apply(&doc, &merged).unwrap();
    view.apply(&doc, &second).unwrap();
    assert_eq!(
        (
            view.version(),
            view.range_visits(),
            view.heights().mutation_visits()
        ),
        frozen
    );
    assert_eq!(view.apply(&doc, &first), Err(LayoutError::OutOfOrderUpdate));
    let mut foreign = Document::new();
    let update = foreign.append("x").unwrap();
    assert_eq!(
        view.apply(&foreign, &update),
        Err(LayoutError::WrongDocument)
    );
    let mut stale = SourceFoldRegistration::seal_tail(&doc, &view, sid).unwrap();
    view.toggle(&doc, id).unwrap();
    assert_eq!(
        stale.advance(&doc, &mut view, &mut one()),
        Err(LayoutError::StaleContinuation)
    );
    remove(&doc, &mut view, id);
    assert!(matches!(
        SourceFoldRegistration::seal_tail(&doc, &view, sid),
        Err(LayoutError::InvalidRange)
    ));
    let replacement = tail(&doc, &mut view, 5);
    assert_ne!(replacement, id);
    let update = doc.finish();
    view.apply(&doc, &update).unwrap();
    assert!(doc.append("x").is_err());
    let frozen = (view.version(), view.range_visits());
    let update = doc.finish();
    view.apply(&doc, &update).unwrap();
    assert_eq!((view.version(), view.range_visits()), frozen);
}

#[test]
fn root_tail_growth_seal_remove_invalidate_layout_dependencies_but_keep_front() {
    for mutation in 0..3 {
        let mut doc = Document::new();
        doc.append(&format!(
            "{}\n\ntail {}\n\n",
            "a".repeat(4096),
            "b".repeat(4096)
        ))
        .unwrap();
        let start = doc.source().find("tail").unwrap() + 5;
        let mut view = FoldView::new(&doc, 7).unwrap();
        let id = tail(&doc, &mut view, start);
        view.set_collapsed(&doc, id, true).unwrap();
        let owner = doc.tile_at(start).unwrap();
        view.insert_checkpoint(TileId(0), ()).unwrap();
        view.insert_checkpoint(owner, ()).unwrap();
        let front = view
            .heights()
            .ticket_from(TileId(1), view.checkpoints(), TileId(0))
            .unwrap();
        let ticket = view
            .heights()
            .ticket_from(owner, view.checkpoints(), owner)
            .unwrap();
        let base = view.heights().base_height(TileId(owner.0 + 1));
        let key = DependencyKey::capture(&doc, owner, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap()
            .unwrap();
        let mut layout = LayoutPass::folded(&doc, &view, owner, 20).unwrap();
        let mut window =
            SourceFoldCandidates::new(&doc, &view, Span::new(start, doc.source().len())).unwrap();
        let mut batch =
            FoldBatch::new(&doc, &view, FoldBatchAction::All { collapsed: false }).unwrap();
        let mut nav = FoldNavigation::new(
            &doc,
            &view,
            &Navigator::new(),
            NavAction::Source(start + 1),
            FoldNavigationPolicy::Reveal,
            20,
        )
        .unwrap();
        match mutation {
            0 => {
                let u = doc.append("more\n\n").unwrap();
                view.apply(&doc, &u).unwrap();
            }
            1 => {
                assert_eq!(seal(&doc, &mut view, id), Ok(id));
            }
            _ => remove(&doc, &mut view, id),
        }
        assert_eq!(view.measure(ticket, 30), Err(LayoutError::StaleMeasurement));
        assert_eq!(view.measure(front, 11), Ok(()));
        assert_eq!(view.checkpoints().get(owner), None);
        assert_eq!(view.checkpoints().get(TileId(0)), Some(&()));
        assert_eq!(view.heights().base_height(TileId(owner.0 + 1)), base);
        assert_eq!(
            layout.advance_folded(&doc, &mut view, &mut one(), &mut Sink::default()),
            Err(LayoutError::StaleContinuation)
        );
        assert_eq!(
            window.advance(&doc, &view, &mut one(), &mut Vec::new()),
            Err(LayoutError::StaleContinuation)
        );
        assert_eq!(
            batch.advance(&doc, &mut view, &mut one()),
            Err(LayoutError::StaleContinuation)
        );
        assert_eq!(
            nav.advance(&doc, &mut view, &mut one()),
            Err(LayoutError::StaleContinuation)
        );
        // Parser semantic validity cannot substitute for the layout ticket.
        assert_eq!(
            key.validate(&doc, &mut one()).unwrap(),
            CacheValidity::Valid
        );
    }
}

#[test]
fn root_tail_one_slot_work_is_logarithmic_with_real_history_and_million_byte_tail() {
    for count in [16, 256, 4096] {
        let mut doc = Document::new();
        doc.append(&format!("{}\n\n", "abcdefgh".repeat(count * 2)))
            .unwrap();
        let mut view = FoldView::new(&doc, 7).unwrap();
        for start in (0..count * 16).step_by(16) {
            let id = register(&doc, &mut view, Span::new(start, start + 8)).unwrap();
            view.set_collapsed(&doc, id, true).unwrap();
        }
        let start = doc.source().len() - 2;
        let id = tail(&doc, &mut view, start);
        view.set_collapsed(&doc, id, true).unwrap();
        for turn in 0..8 {
            let ranges = view.range_visits();
            let h = view.heights().mutation_visits();
            let g = view.heights().dependency_mutation_visits();
            let c = view.checkpoints().mutation_visits();
            let update = doc
                .append(if turn == 0 { "中🙂" } else { "abcdefghi" })
                .unwrap();
            let parser = doc.work();
            view.apply(&doc, &update).unwrap();
            assert_eq!(doc.work(), parser);
            assert_eq!(view.range_visits() - ranges, 1);
            let log = usize::BITS - doc.tile_count().max(1).leading_zeros();
            assert!(view.heights().mutation_visits() - h <= 64 * u64::from(log + 1));
            assert!(view.heights().dependency_mutation_visits() - g <= 32 * u64::from(log + 1));
            assert!(view.checkpoints().mutation_visits() - c <= 32 * u64::from(log + 1));
            let mut out = Vec::new();
            let mut cursor =
                SourceFoldCandidates::new(&doc, &view, Span::new(start + 1, doc.source().len()))
                    .unwrap();
            let mut b = one();
            cursor.advance(&doc, &view, &mut b, &mut out).unwrap();
            assert_eq!(out.len(), 1);
            assert_eq!(out[0].id, id);
            assert_eq!(b.used(), one().remaining());
        }
    }
    let mut doc = Document::new();
    doc.append("lead text\n\n").unwrap();
    let mut view = FoldView::new(&doc, 7).unwrap();
    let id = tail(&doc, &mut view, 5);
    view.set_collapsed(&doc, id, true).unwrap();
    let update = doc
        .append(&format!("{}![image](/u)", "x".repeat(1_000_000)))
        .unwrap();
    let parser = doc.work();
    view.apply(&doc, &update).unwrap();
    assert_eq!(doc.work(), parser);
    let c = view.fold_candidate(&doc, id).unwrap().unwrap();
    let mut b = Budget::new(WorkLimit {
        queries: 2,
        objects: 1,
        bytes: 4,
        ..Default::default()
    });
    assert!(matches!(
        view.hidden_at(&doc, 900_000, &mut b).unwrap(),
        FoldLookup::Hidden(_)
    ));
    assert_eq!(c.summary_text(&doc, 4, &mut b), Some("text"));
    assert_eq!(b.used().bytes, 4);
    let mut pass = LayoutPass::folded(&doc, &view, TileId(0), 20).unwrap();
    let mut sink = TailSink::default();
    let mut frames = 0;
    loop {
        frames += 1;
        let mut b = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 6,
            tiles: 1,
            ..Default::default()
        });
        if pass
            .advance_folded(&doc, &mut view, &mut b, &mut sink)
            .unwrap()
            == Completion::EndOfDocument
        {
            break;
        }
        assert!(frames < 20);
    }
    assert_eq!(sink.inner.text, "lead [fold]\n");
    assert_eq!(sink.inner.placeholders.len(), 1);
    assert_eq!(sink.ends, 0);
    assert!(sink.inner.bytes <= 5);
    assert_eq!(sink.inner.objects, 0);
    assert_eq!(view.heights().base_height(TileId(100)), Some(7));
    assert_eq!(view.heights().height(TileId(100)), Some(0));
    // A remote start inside the tail emits neither its owner nor paragraph_end.
    let mut pass = LayoutPass::folded(&doc, &view, TileId(100), 20).unwrap();
    let mut remote = TailSink::default();
    for _ in 0..6 {
        if pass
            .advance_folded(&doc, &mut view, &mut one(), &mut remote)
            .unwrap()
            == Completion::EndOfDocument
        {
            break;
        }
    }
    assert_eq!(
        (
            remote.inner.bytes,
            remote.inner.placeholders.len(),
            remote.ends
        ),
        (0, 0, 0)
    );
}

#[derive(Default)]
struct TailSink {
    inner: Sink,
    ends: usize,
}
impl LayoutSink for TailSink {
    fn begin_tile(&mut self, tile: TileId) {
        self.inner.begin_tile(tile);
    }
    fn text(&mut self, run: RenderRun<'_>) {
        self.inner.text(run);
    }
    fn object(&mut self, id: LinkId, object: &LinkInfo) {
        self.inner.object(id, object);
    }
    fn end_tile(&mut self, tile: TileId) -> u64 {
        self.inner.end_tile(tile)
    }
}
impl FoldLayoutSink for TailSink {
    fn placeholder(&mut self, p: FoldPlaceholder) {
        self.inner.placeholder(p);
    }
    fn paragraph_end(&mut self, _: BlockId) {
        self.ends += 1;
    }
}

#[test]
fn root_tail_seal_pays_each_reachable_ancestor_and_resumes_without_mutation() {
    let mut doc = Document::new();
    doc.append("lead text\n\n").unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let id = tail(&doc, &mut view, 5);
    let prefix = ">".repeat(1000);
    let update = doc
        .append(&format!("{prefix} ```\n{prefix} code\n{prefix} ```\n"))
        .unwrap();
    view.apply(&doc, &update).unwrap();
    assert_eq!(doc.source_fold_limit(), doc.source().len());
    let FoldId::Source(sid) = id else { panic!() };
    let mut task = SourceFoldRegistration::seal_tail(&doc, &view, sid).unwrap();
    let before = (view.version(), view.fold_candidate(&doc, id).unwrap());
    let mut steps = 0;
    let mut visited = 0;
    loop {
        let mut b = one();
        steps += 1;
        match task.advance(&doc, &mut view, &mut b) {
            Ok(p) => {
                assert_eq!(p.completion, Completion::BudgetExhausted);
                visited = p.visited;
            }
            Err(e) => {
                assert_eq!(e, LayoutError::InvalidRange);
                break;
            }
        }
        assert_eq!(
            (view.version(), view.fold_candidate(&doc, id).unwrap()),
            before
        );
        assert!(steps < 10);
    }
    // A thousand requested containers is not a thousand reachable ancestors:
    // the closed code is visited, then the first open container rejects seal.
    assert_eq!(visited, 1);
    assert_eq!(steps, 5);
    assert_eq!(
        (view.version(), view.fold_candidate(&doc, id).unwrap()),
        before
    );
}

#[test]
fn root_tail_contains_open_table_rows_in_both_registration_orders() {
    for tail_first in [false, true] {
        let mut doc = Document::new();
        doc.append("lead text\n\n").unwrap();
        let mut view = FoldView::new(&doc, 2).unwrap();
        let early = tail_first.then(|| tail(&doc, &mut view, 5));
        let u = doc
            .append(&format!("| h |\n| - |\n| {}", "x".repeat(1024)))
            .unwrap();
        view.apply(&doc, &u).unwrap();
        let row_id = FoldId::TableRows(TableRowRange {
            table: TableId(0),
            start: 1,
            end: 2,
        });
        assert!(doc.fold_candidate(row_id).unwrap().open);
        assert_eq!(
            view.insert_table_rows(&doc, TableId(0), 1..2).unwrap(),
            row_id
        );
        let id = early.unwrap_or_else(|| tail(&doc, &mut view, 5));
        view.set_collapsed(&doc, id, true).unwrap();
        view.set_collapsed(&doc, row_id, true).unwrap();
        for chunk in ["🙂", " |\n", "| next |\n\n"] {
            let u = doc.append(chunk).unwrap();
            view.apply(&doc, &u).unwrap();
            let c = view.fold_candidate(&doc, id).unwrap().unwrap();
            let r = view.fold_candidate(&doc, row_id).unwrap().unwrap();
            assert_eq!(r.parent, Some(FoldId::Table(TableId(0))));
            let expected = verify(&doc, &view, &[c, r]);
            assert_eq!(render(&doc, &mut view, true).text, expected);
        }
    }
}

#[test]
fn root_tail_navigation_and_preview_batch_choice_survive_growth_and_seal() {
    let mut doc = Document::new();
    doc.append("lead text\n\n").unwrap();
    let mut view = FoldView::new(&doc, 2).unwrap();
    let id = tail(&doc, &mut view, 5);
    let update = doc.append("# H\nbody\n").unwrap();
    view.apply(&doc, &update).unwrap();
    view.set_collapsed(&doc, id, true).unwrap();
    let heading = FoldId::Heading(HeadingId(0));
    view.set_collapsed(&doc, heading, true).unwrap();
    for policy in [
        FoldNavigationPolicy::VisibleOnly,
        FoldNavigationPolicy::Reveal,
    ] {
        let action = if policy == FoldNavigationPolicy::VisibleOnly {
            NavAction::NextHeading { max_level: 6 }
        } else {
            NavAction::Source(doc.source().find("body").unwrap())
        };
        let mut nav =
            FoldNavigation::new(&doc, &view, &Navigator::new(), action, policy, 10).unwrap();
        for step in 0..10 {
            let p = nav.advance(&doc, &mut view, &mut one()).unwrap();
            if p.completion == Completion::Covered {
                assert_eq!(p.target.is_some(), policy == FoldNavigationPolicy::Reveal);
                break;
            }
            assert!(step < 9);
        }
    }
    assert!(!view.is_collapsed(id));
    assert!(!view.is_collapsed(heading));
    // Start a separate stable tail so a successful manual seal can be checked
    // with both saved choices changed by a batch while preview is active.
    for collapsed in [false, true] {
        let mut doc = Document::new();
        doc.append("lead text\n\n").unwrap();
        let mut view = FoldView::new(&doc, 2).unwrap();
        let id = tail(&doc, &mut view, 5);
        view.set_preview(&doc, Some(id), &mut one()).unwrap();
        batch(
            &doc,
            &mut view,
            FoldBatchAction::Subtree {
                root: id,
                collapsed,
            },
        );
        let u = doc.append("next\n\n").unwrap();
        view.apply(&doc, &u).unwrap();
        view.reset(3).unwrap();
        assert_eq!(view.preview(), Some(id));
        let end = doc.source().len();
        assert_eq!(seal(&doc, &mut view, id), Ok(id));
        let u = doc.append("visible\n\n").unwrap();
        view.apply(&doc, &u).unwrap();
        view.set_preview(&doc, None, &mut one()).unwrap();
        assert_eq!(view.is_collapsed(id), collapsed);
        assert_eq!(
            view.fold_candidate(&doc, id).unwrap().unwrap().source.end,
            end
        );
        assert_eq!(hidden(&doc, &view, 5).is_some(), collapsed);
        assert_eq!(hidden(&doc, &view, end), None);
    }
}

#[test]
fn root_tail_consumers_share_one_frame_budget_without_hidden_allowance() {
    let mut doc = Document::new();
    doc.append("lead text\n\n").unwrap();
    let mut view = FoldView::new(&doc, 2).unwrap();
    let mut b = Budget::new(WorkLimit {
        queries: 6,
        objects: 6,
        bytes: 6,
        tiles: 1,
        ..Default::default()
    });
    let mut task = SourceFoldRegistration::tail(&doc, &view, 5).unwrap();
    let id = task.advance(&doc, &mut view, &mut b).unwrap().id.unwrap();
    view.set_preview(&doc, Some(id), &mut b).unwrap();
    let c = view.fold_candidate(&doc, id).unwrap().unwrap();
    let mut cursor = SourceFoldCandidates::new(&doc, &view, c.source).unwrap();
    cursor
        .advance(&doc, &view, &mut b, &mut Vec::new())
        .unwrap();
    assert_eq!(
        view.hidden_at(&doc, 5, &mut b).unwrap(),
        FoldLookup::Visible
    );
    let mut display = DisplayExcerpt::new(
        &doc,
        c.source,
        ExcerptLimits {
            source_bytes: 6,
            display_bytes: 6,
            lines: 2,
        },
    )
    .unwrap();
    assert!(matches!(
        display.next(&doc, &mut b).unwrap(),
        ExcerptStep::Run(_)
    ));
    assert_eq!(b.used().queries, 6);
    assert_eq!(b.used().bytes, 6);
    assert_eq!(c.summary_text(&doc, 1, &mut b), None);
    let before = b.used();
    let FoldId::Source(sid) = id else { panic!() };
    let mut seal = SourceFoldRegistration::seal_tail(&doc, &view, sid).unwrap();
    assert_eq!(
        seal.advance(&doc, &mut view, &mut b).unwrap().completion,
        Completion::BudgetExhausted
    );
    let mut batch = FoldBatch::new(&doc, &view, FoldBatchAction::All { collapsed: true }).unwrap();
    assert_eq!(
        batch.advance(&doc, &mut view, &mut b).unwrap().completion,
        Completion::BudgetExhausted
    );
    let mut nav = FoldNavigation::new(
        &doc,
        &view,
        &Navigator::new(),
        NavAction::Source(5),
        FoldNavigationPolicy::Reveal,
        10,
    )
    .unwrap();
    assert_eq!(
        nav.advance(&doc, &mut view, &mut b).unwrap().completion,
        Completion::BudgetExhausted
    );
    let mut pass = LayoutPass::folded(&doc, &view, TileId(0), 10).unwrap();
    assert_eq!(
        pass.advance_folded(&doc, &mut view, &mut b, &mut Sink::default())
            .unwrap(),
        Completion::BudgetExhausted
    );
    assert_eq!(b.used(), before);
    // Dependency discovery must pay even on a hidden owner tile.
    let u = doc.append("[r](/u)\n\n").unwrap();
    view.apply(&doc, &u).unwrap();
    let mut b = Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        ..Default::default()
    });
    assert!(
        DependencyKey::capture(&doc, TileId(0), &mut b)
            .unwrap()
            .is_none()
    );
    assert_eq!(b.used().dependencies, 0);
    assert_eq!(b.used().queries, 1);
}
