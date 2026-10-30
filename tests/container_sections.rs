use mdtui3::*;

fn h(i: usize) -> FoldId {
    FoldId::Heading(HeadingId(i))
}
fn c(i: usize) -> FoldId {
    FoldId::Container(ContainerId(i))
}
fn tiny() -> Budget {
    Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        ..Default::default()
    })
}
fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
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
        assert_eq!(budget.used().bytes, 0);
        result.extend_from_slice(&out);
        if done == Completion::Covered {
            return result;
        }
    }
}
fn batch(doc: &Document, view: &mut FoldView, action: FoldBatchAction) {
    let mut task = FoldBatch::new(doc, view, action).unwrap();
    for _ in 0..doc.blocks().len() + doc.container_count() + 2 {
        if task.advance(doc, view, &mut tiny()).unwrap().completion == Completion::Covered {
            return;
        }
    }
    panic!("batch stalled");
}

// Independent ranges from parser metadata: deliberately no section() or fold
// range APIs. Linear heading search is acceptable only in this test oracle.
fn model(doc: &Document, id: FoldId) -> (Span, Span) {
    match id {
        FoldId::Heading(id) => {
            let headings = doc.headings().entries();
            let heading = headings[id.0];
            let boundary = heading.container.map_or(doc.source().len(), |id| {
                let node = doc.container(id).unwrap();
                if node.state == SyntaxState::Sealed {
                    node.source.end
                } else {
                    doc.source().len()
                }
            });
            let end = headings[id.0 + 1..]
                .iter()
                .find(|next| next.container == heading.container && next.level <= heading.level)
                .map_or(boundary, |next| next.source.start);
            (
                Span::new(heading.source.start, end),
                Span::new(heading.source.end, end),
            )
        }
        FoldId::Container(id) => {
            let node = doc.container(id).unwrap();
            let source = Span::new(
                node.source.start,
                if node.state == SyntaxState::Sealed {
                    node.source.end
                } else {
                    doc.source().len()
                },
            );
            (source, source)
        }
        FoldId::Table(id) => {
            let table = doc.table(id).unwrap();
            let block = doc.block(table.block).unwrap();
            let source = Span::new(
                table.source.start,
                if block.state == SyntaxState::Sealed {
                    table.source.end
                } else {
                    doc.source().len()
                },
            );
            (source, source)
        }
        FoldId::Code(id) => {
            let block = doc.block(id).unwrap();
            let source = Span::new(
                block.source.start,
                if block.state == SyntaxState::Sealed {
                    block.source.end
                } else {
                    doc.source().len()
                },
            );
            (source, source)
        }
        FoldId::TableRows(_) => unreachable!(),
    }
}
fn order(source: Span, id: FoldId) -> (usize, usize, usize) {
    match id {
        FoldId::Container(id) => (source.start, 0, id.0),
        _ => (source.start, 1, 0),
    }
}
fn verify(doc: &Document, view: &FoldView) {
    let candidates = all(doc);
    let mut hidden = Vec::new();
    for candidate in &candidates {
        let (source, body) = model(doc, candidate.id);
        assert_eq!(candidate.source, source, "{:?}", candidate.id);
        assert_eq!(candidate.hidden, body, "{:?}", candidate.id);
        // Infer immediate parent by strict interval/preorder containment.
        let parent = candidates
            .iter()
            .filter(|outer| {
                let (s, _) = model(doc, outer.id);
                s.start <= source.start
                    && s.end >= source.end
                    && order(s, outer.id) < order(source, candidate.id)
            })
            .max_by_key(|outer| order(outer.source, outer.id))
            .map(|outer| outer.id);
        assert_eq!(candidate.parent, parent, "parent of {:?}", candidate.id);
        if view.is_collapsed(candidate.id)
            && view.preview() != Some(candidate.id)
            && !body.is_empty()
        {
            hidden.push((order(source, candidate.id), candidate.id, body));
        }
    }
    hidden.sort_by_key(|&(order, _, _)| order);
    for offset in (0..doc.source().len())
        .step_by(7)
        .chain([doc.source().len()])
    {
        let expected = hidden
            .iter()
            .find(|(_, _, body)| body.range().contains(&offset))
            .map(|(_, id, _)| *id);
        let got = match view.hidden_at(doc, offset, &mut tiny()).unwrap() {
            FoldLookup::Hidden(candidate) => Some(candidate.id),
            FoldLookup::Visible => None,
            FoldLookup::BudgetExhausted => panic!("lookup stalled"),
        };
        assert_eq!(got, expected, "offset {offset}");
    }
    for i in 0..doc.tile_count() {
        let suppressed = hidden.iter().any(|(_, _, body)| {
            let first = doc.tile_at(body.start).unwrap().0 + 1;
            let end = doc
                .tile_at(body.end)
                .map_or(doc.tile_count(), |tile| tile.0);
            first <= i && i < end
        });
        let expected = if suppressed {
            0
        } else {
            view.heights().base_height(TileId(i)).unwrap()
        };
        assert_eq!(view.heights().height(TileId(i)), Some(expected), "tile {i}");
    }
}

#[derive(Default)]
struct Sink {
    text: String,
    placeholders: usize,
    bytes: usize,
}
impl LayoutSink for Sink {
    fn begin_tile(&mut self, _: TileId) {}
    fn text(&mut self, run: RenderRun<'_>) {
        self.bytes += run.text.len();
        if !run.style.contains(Style::HIDDEN) {
            self.text.push_str(run.text);
        }
    }
    fn object(&mut self, _: LinkId, _: &LinkInfo) {}
    fn end_tile(&mut self, _: TileId) -> u64 {
        1
    }
}
impl FoldLayoutSink for Sink {
    fn placeholder(&mut self, _: FoldPlaceholder) {
        self.placeholders += 1;
        self.text.push_str("[fold]\n");
    }
}
fn render(doc: &Document, view: &mut FoldView) -> Sink {
    let mut pass = LayoutPass::folded(doc, view, TileId(0), u64::MAX).unwrap();
    let mut sink = Sink::default();
    for _ in 0..10000 {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            tiles: 1,
            bytes: 4,
            ..Default::default()
        });
        if pass
            .advance_folded(doc, view, &mut budget, &mut sink)
            .unwrap()
            == Completion::EndOfDocument
        {
            return sink;
        }
    }
    panic!("layout stalled");
}

#[test]
fn exact_scope_parents_discovery_and_innermost_candidates() {
    let doc = parse(
        "# root\n> ## outer\n> - ### item\n>   body\n>   > # deep\n>   > x\n>   #### child\n>   a|b\n>   -|-\n>   x|y\n> - ## sibling\n> ## peer\nout\n# end\n",
    );
    let expected = [
        (h(0), None),
        (c(0), Some(h(0))),
        (h(1), Some(c(0))),
        (c(1), Some(h(1))),
        (c(2), Some(c(1))),
        (h(2), Some(c(2))),
        (c(3), Some(h(2))),
        (h(3), Some(c(3))),
        (h(4), Some(h(2))),
        (FoldId::Table(TableId(0)), Some(h(4))),
        (c(4), Some(c(1))),
        (h(5), Some(c(4))),
        (h(6), Some(c(0))),
        (h(7), None),
    ];
    assert_eq!(
        all(&doc)
            .iter()
            .map(|c| (c.id, c.parent))
            .collect::<Vec<_>>(),
        expected
    );
    for (text, id) in [
        ("body", h(2)),
        (">   > x", h(3)),
        ("x|y", FoldId::Table(TableId(0))),
        ("out\n", h(0)),
    ] {
        assert_eq!(
            doc.fold_candidate_at(doc.source().find(text).unwrap())
                .unwrap()
                .id,
            id
        );
    }
    verify(&doc, &FoldView::new(&doc, 1).unwrap());
}

#[test]
fn retained_titles_empty_sections_same_tile_and_parent_intent() {
    let doc = parse("> # A\n> ## B\n> x\n> # C\n> # D\n> y\nout\n");
    let mut view = FoldView::new(&doc, 2).unwrap();
    for id in [h(0), h(1), h(2), h(3)] {
        view.set_collapsed(&doc, id, true).unwrap();
    }
    assert_eq!(
        render(&doc, &mut view).text,
        "A\n[fold]\nC\nD\n[fold]\nout\n"
    );
    view.set_collapsed(&doc, c(0), true).unwrap();
    assert_eq!(render(&doc, &mut view).text, "[fold]\nout\n");
    view.set_collapsed(&doc, c(0), false).unwrap();
    view.set_collapsed(&doc, h(0), false).unwrap();
    assert_eq!(
        render(&doc, &mut view).text,
        "A\nB\n[fold]\nC\nD\n[fold]\nout\n"
    );
    assert_eq!(
        render(&doc, &mut FoldView::new(&doc, 1).unwrap()).placeholders,
        0
    );
    verify(&doc, &view);
}

#[test]
fn late_setext_pending_exit_and_merged_closures_preserve_choices() {
    let mut doc = Document::new();
    doc.append("> # A\n> ## B\n> old\n").unwrap();
    let mut view = FoldView::new(&doc, 2).unwrap();
    for id in [h(0), h(1)] {
        view.set_collapsed(&doc, id, true).unwrap();
    }
    let boundary = doc.source().len();
    let update = doc.append("> New\n").unwrap();
    view.apply(&doc, &update).unwrap();
    verify(&doc, &view);
    let mut merged = doc.append("> ===\n").unwrap();
    merged
        .merge(doc.append("> body\n> ## child\n> more\nout").unwrap())
        .unwrap();
    view.apply(&doc, &merged).unwrap();
    for id in [h(0), h(1)] {
        assert_eq!(doc.fold_candidate(id).unwrap().source.end, boundary);
        assert!(!doc.fold_candidate(id).unwrap().open);
    }
    view.set_collapsed(&doc, h(2), true).unwrap();
    assert_eq!(
        doc.fold_candidate(h(2)).unwrap().source.end,
        doc.source().len()
    );
    let update = doc.append("\n").unwrap();
    view.apply(&doc, &update).unwrap();
    assert_eq!(
        doc.fold_candidate(h(2)).unwrap().source.end,
        doc.source().find("out").unwrap()
    );
    assert!(!doc.fold_candidate(h(2)).unwrap().open);
    verify(&doc, &view);
    let update = doc.finish();
    view.apply(&doc, &update).unwrap();
    let version = view.version();
    view.apply(&doc, &update).unwrap();
    assert_eq!(view.version(), version);
    verify(&doc, &view);
}

#[test]
fn all_utf8_splits_and_prefixes_match_default_fold_snapshots() {
    let source = "> # 界\r\n> x\n> > ## 子\n> > a|b\n> > -|-\n> > x|y\n> New\n> ===\n- # item\n  body\n- ## next\nout";
    let defaults = FoldDefaults {
        heading_levels: [true; 6],
        table: true,
        ..Default::default()
    };
    for cut in source.char_indices().map(|(i, _)| i).chain([source.len()]) {
        let mut doc = Document::new();
        let mut view = FoldView::new(&doc, 2).unwrap();
        view.set_defaults(defaults);
        let first = doc.append(&source[..cut]).unwrap();
        view.apply(&doc, &first).unwrap();
        verify(&doc, &view);
        let update = doc.append(&source[cut..]).unwrap();
        view.apply(&doc, &update).unwrap();
        verify(&doc, &view);
        let update = doc.finish();
        view.apply(&doc, &update).unwrap();
        verify(&doc, &view);
    }
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 2).unwrap();
    view.set_defaults(defaults);
    for ch in source.chars() {
        let update = doc.append(&ch.to_string()).unwrap();
        view.apply(&doc, &update).unwrap();
        verify(&doc, &view);
    }
    let mut merged_doc = Document::new();
    let mut merged_view = FoldView::new(&merged_doc, 2).unwrap();
    merged_view.set_defaults(defaults);
    let mut merged = merged_doc.append("").unwrap();
    for ch in source.chars() {
        merged
            .merge(merged_doc.append(&ch.to_string()).unwrap())
            .unwrap();
    }
    merged_view.apply(&merged_doc, &merged).unwrap();
    assert_eq!(
        render(&doc, &mut view).text,
        render(&merged_doc, &mut merged_view).text
    );
    verify(&merged_doc, &merged_view);
}

#[test]
fn container_code_all_utf8_splits_prefixes_and_merged_updates_match_fold_model() {
    let source = "# root\n> ## 中\r\n> - [x] ```rs\r\n>   > *🙂*\n>   ```\n>       raw\n>   \n>   ~~~\n>   # opaque\n> - sibling\n>   a|b\n>   -|-\n>   x|y\n>     code\nout";
    let defaults = FoldDefaults {
        code: true,
        table: true,
        heading_levels: [true; 6],
        container_depth: Some(2),
    };
    let expected = parse(source);
    for cut in source.char_indices().map(|(i, _)| i).chain([source.len()]) {
        let mut doc = Document::new();
        let mut view = FoldView::new(&doc, 2).unwrap();
        view.set_defaults(defaults);
        for part in [&source[..cut], &source[cut..]] {
            let update = doc.append(part).unwrap();
            view.apply(&doc, &update).unwrap();
            verify(&doc, &view);
        }
        let update = doc.finish();
        view.apply(&doc, &update).unwrap();
        verify(&doc, &view);
        assert_eq!(doc.blocks(), expected.blocks());
    }
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 2).unwrap();
    view.set_defaults(defaults);
    let mut merged_doc = Document::new();
    let mut merged_view = FoldView::new(&merged_doc, 2).unwrap();
    merged_view.set_defaults(defaults);
    let mut merged = merged_doc.append("").unwrap();
    for ch in source.chars() {
        let part = ch.to_string();
        let update = doc.append(&part).unwrap();
        view.apply(&doc, &update).unwrap();
        verify(&doc, &view);
        let mut once = Document::new();
        once.append(doc.source()).unwrap();
        assert_eq!(doc.blocks(), once.blocks());
        merged.merge(merged_doc.append(&part).unwrap()).unwrap();
    }
    merged_view.apply(&merged_doc, &merged).unwrap();
    verify(&merged_doc, &merged_view);
    assert_eq!(
        render(&doc, &mut view).text,
        render(&merged_doc, &mut merged_view).text
    );
}

#[test]
fn container_code_preview_batch_navigation_and_same_tile_layout_compose() {
    let source = "> # H\n> ```rs\n> raw\n> ```\n> a|b\n> -|-\n> x|y\n>     indented\nout\n";
    let doc = parse(source);
    let mut view = FoldView::new(&doc, 2).unwrap();
    let code = FoldId::Code(BlockId(1));
    let indented = FoldId::Code(BlockId(3));
    let table = FoldId::Table(TableId(0));
    assert_eq!(doc.fold_candidate(code).unwrap().parent, Some(h(0)));
    assert_eq!(doc.fold_candidate(indented).unwrap().parent, Some(h(0)));
    for id in [code, indented, table] {
        view.set_collapsed(&doc, id, true).unwrap();
    }
    assert_eq!(
        render(&doc, &mut view).text,
        "H\n[fold]\n[fold]\n[fold]\nout\n"
    );
    view.set_collapsed(&doc, c(0), true).unwrap();
    view.set_preview(&doc, Some(c(0)), &mut tiny()).unwrap();
    assert_eq!(render(&doc, &mut view).placeholders, 3);
    view.set_preview(&doc, None, &mut tiny()).unwrap();
    assert_eq!(render(&doc, &mut view).text, "[fold]\nout\n");
    view.set_collapsed(&doc, c(0), false).unwrap();
    batch(
        &doc,
        &mut view,
        FoldBatchAction::Siblings {
            target: code,
            collapsed: false,
        },
    );
    assert!(view.is_collapsed(code));
    assert!(!view.is_collapsed(table));
    assert!(!view.is_collapsed(indented));
    batch(&doc, &mut view, FoldBatchAction::All { collapsed: true });
    let offset = source.find("raw").unwrap();
    let mut navigation = FoldNavigation::new(
        &doc,
        &view,
        &Navigator::new(),
        NavAction::Source(offset),
        FoldNavigationPolicy::Reveal,
        10,
    )
    .unwrap();
    let mut finished = false;
    for _ in 0..10 {
        let mut budget = tiny();
        let result = navigation.advance(&doc, &mut view, &mut budget).unwrap();
        assert_eq!(budget.used().bytes, 0);
        if result.completion == Completion::Covered {
            assert_eq!(result.target.unwrap().source, Some(offset));
            finished = true;
            break;
        }
    }
    assert!(finished);
    assert!(!view.is_collapsed(code));
    assert!(view.is_collapsed(table));
    assert!(view.is_collapsed(indented));
    assert_eq!(
        render(&doc, &mut view).text,
        "H\nraw\n[fold]\n[fold]\nout\n"
    );
    verify(&doc, &view);
    assert_eq!(
        render(&doc, &mut FoldView::new(&doc, 2).unwrap()).placeholders,
        0
    );
}

#[test]
fn container_code_streaming_toggles_and_resize_match_interval_height_model() {
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 2).unwrap();
    let parts = [
        "> # section\n",
        "> - ```rs\n",
        ">   raw\n",
        ">   ```\n",
        ">       indented\n",
        ">   \n",
        ">   ~~~\n",
        "> - sibling\n",
        "> a|b\n> -|-\n> x|y\n",
        "outside\n",
        "> > ```\n",
        "> next\n",
    ];
    let mut seed = 0x327a_bb13_u64;
    for step in 0..240 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let part = parts[(seed >> 32) as usize % parts.len()];
        let update = doc.append(part).unwrap();
        view.apply(&doc, &update).unwrap();
        let candidates = all(&doc);
        if !candidates.is_empty() {
            let candidate = candidates[seed as usize % candidates.len()];
            view.set_collapsed(&doc, candidate.id, seed & 1 == 0)
                .unwrap();
            if step % 7 == 0 {
                view.set_preview(&doc, Some(candidate.id), &mut tiny())
                    .unwrap();
            }
        }
        if step % 11 == 0 {
            view.set_preview(&doc, None, &mut tiny()).unwrap();
        }
        if step % 13 == 0 {
            view.reset(step % 5 + 1).unwrap();
        }
        verify(&doc, &view);
    }
    let update = doc.finish();
    view.apply(&doc, &update).unwrap();
    verify(&doc, &view);
}

#[test]
fn container_code_folded_million_byte_body_has_zero_layout_reads() {
    let source = format!("> ```\n> {}\n> ```\n", "x".repeat(1_000_000));
    let doc = parse(&source);
    let mut view = FoldView::new(&doc, 2).unwrap();
    view.set_collapsed(&doc, FoldId::Code(BlockId(0)), true)
        .unwrap();
    let mut pass = LayoutPass::folded(&doc, &view, TileId(0), u64::MAX).unwrap();
    let mut sink = Sink::default();
    let mut bytes = 0;
    let mut finished = false;
    for _ in 0..10 {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            tiles: 1,
            ..Default::default()
        });
        let result = pass
            .advance_folded(&doc, &mut view, &mut budget, &mut sink)
            .unwrap();
        bytes += budget.used().bytes;
        if result == Completion::EndOfDocument {
            finished = true;
            break;
        }
    }
    assert!(finished);
    assert_eq!(bytes, 0);
    assert_eq!(sink.bytes, 0);
    assert_eq!(sink.placeholders, 1);
}

#[test]
fn batch_actions_match_parent_chains_including_same_opener() {
    let doc = parse(
        "> - # A\n>   ### child\n>   > # Q\n>   > x\n>   ## peer\n>   a|b\n>   -|-\n>   x|y\n> - # B\n>   y\n",
    );
    let candidates = all(&doc);
    for target in candidates.iter().map(|c| c.id) {
        let ancestors = |mut id| {
            let mut result = vec![id];
            while let Some(parent) = candidates.iter().find(|c| c.id == id).unwrap().parent {
                result.push(parent);
                id = parent;
            }
            result
        };
        for action in [
            FoldBatchAction::Subtree {
                root: target,
                collapsed: true,
            },
            FoldBatchAction::Siblings {
                target,
                collapsed: true,
            },
            FoldBatchAction::Focus { target },
            FoldBatchAction::AtLevel {
                level: 1,
                collapsed: true,
            },
        ] {
            let mut view = FoldView::new(&doc, 2).unwrap();
            batch(&doc, &mut view, action);
            for candidate in &candidates {
                let expected = match action {
                    FoldBatchAction::Subtree { .. } => ancestors(candidate.id).contains(&target),
                    FoldBatchAction::Focus { .. } => !ancestors(target).contains(&candidate.id),
                    FoldBatchAction::Siblings { .. } => {
                        candidate.id != target
                            && candidate.parent == doc.fold_candidate(target).unwrap().parent
                    }
                    FoldBatchAction::AtLevel { .. } => {
                        matches!(candidate.id, FoldId::Heading(id) if doc.headings().get(id).unwrap().level == 1)
                    }
                    _ => unreachable!(),
                };
                assert_eq!(
                    view.is_collapsed(candidate.id),
                    expected,
                    "{action:?}: {:?}",
                    candidate.id
                );
            }
            verify(&doc, &view);
        }
    }
}

#[test]
fn preview_defaults_table_and_stale_budget_contracts() {
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 2).unwrap();
    view.set_defaults(FoldDefaults {
        heading_levels: [true; 6],
        table: true,
        ..Default::default()
    });
    let update = doc.append("> # A\n> body\n").unwrap();
    view.apply(&doc, &update).unwrap();
    let ticket = view.heights().ticket(TileId(0)).unwrap();
    let mut task = FoldBatch::new(&doc, &view, FoldBatchAction::All { collapsed: true }).unwrap();
    let mut budget = Budget::new(WorkLimit::default());
    assert_eq!(
        view.set_preview(&doc, Some(h(0)), &mut budget)
            .unwrap()
            .completion,
        Completion::BudgetExhausted
    );
    view.set_preview(&doc, Some(h(0)), &mut tiny()).unwrap();
    assert_eq!(
        task.advance(&doc, &mut view, &mut tiny()),
        Err(LayoutError::StaleContinuation)
    );
    assert!(view.measure(ticket, 9).is_err());
    let update = doc.append("> ## B\n> a|b\n> -|-\n> x|y\n").unwrap();
    view.apply(&doc, &update).unwrap();
    assert!(view.is_collapsed(h(1)));
    assert!(view.is_collapsed(FoldId::Table(TableId(0))));
    assert_eq!(
        doc.fold_candidate(FoldId::Table(TableId(0)))
            .unwrap()
            .parent,
        Some(h(1))
    );
    view.set_collapsed(&doc, h(0), false).unwrap();
    let update = doc.finish();
    view.apply(&doc, &update).unwrap();
    view.set_preview(&doc, None, &mut tiny()).unwrap();
    view.reset(4).unwrap();
    assert!(!view.is_collapsed(h(0)));
    verify(&doc, &view);
    let invalid = h(99);
    assert_eq!(
        view.set_preview(&doc, Some(invalid), &mut budget),
        Err(LayoutError::InvalidRange)
    );
    let wrong = parse("> # A\n");
    assert_eq!(
        view.hidden_at(&wrong, 0, &mut budget),
        Err(LayoutError::WrongDocument)
    );
    assert_eq!(budget.used(), WorkLimit::default());
}

#[test]
fn deep_reveal_and_visible_navigation_use_one_budgeted_ancestor_per_step() {
    let mut source = String::new();
    for depth in 1..=150 {
        source.push_str(&format!("{}# H\n", ">".repeat(depth)));
    }
    source.push_str(&format!("{}body\nout\n# End\n", ">".repeat(150)));
    let doc = parse(&source);
    let mut view = FoldView::new(&doc, 1).unwrap();
    batch(&doc, &mut view, FoldBatchAction::All { collapsed: true });
    let offset = source.find("body").unwrap();
    let anchor = FoldAnchor::new(&doc, offset).unwrap();
    assert_eq!(
        anchor
            .resolve(&doc, &view, 10, &mut tiny())
            .unwrap()
            .unwrap()
            .hidden_by,
        Some(c(0))
    );
    let mut visible = FoldNavigation::new(
        &doc,
        &view,
        &Navigator::new(),
        NavAction::NextHeading { max_level: 6 },
        FoldNavigationPolicy::VisibleOnly,
        10,
    )
    .unwrap();
    let mut frames = 0;
    loop {
        frames += 1;
        let progress = visible.advance(&doc, &mut view, &mut tiny()).unwrap();
        if progress.completion == Completion::Covered {
            assert_eq!(progress.target.unwrap().source, source.find("# End"));
            break;
        }
        assert!(frames < 8);
    }
    let mut reveal = FoldNavigation::new(
        &doc,
        &view,
        &Navigator::new(),
        NavAction::Source(offset),
        FoldNavigationPolicy::Reveal,
        10,
    )
    .unwrap();
    let mut opened = 0;
    for _ in 0..304 {
        let mut budget = tiny();
        let before = view.section_index_visits();
        let progress = reveal.advance(&doc, &mut view, &mut budget).unwrap();
        assert_eq!(budget.used().bytes, 0);
        assert!(view.section_index_visits() - before < 100);
        opened += usize::from(progress.change.is_some());
        if progress.completion == Completion::Covered {
            assert_eq!(progress.target.unwrap().source, Some(offset));
            break;
        }
    }
    assert_eq!(opened, 300);
    assert!(view.is_collapsed(h(150)));
    assert_eq!(
        anchor
            .resolve(&doc, &view, 10, &mut tiny())
            .unwrap()
            .unwrap()
            .hidden_by,
        None
    );
}

#[test]
fn visible_navigation_skips_scoped_subtree_and_retains_its_title() {
    let doc = parse(&format!("> # A\n{}> # B\n", "> ## hidden\n".repeat(10_000)));
    let mut view = FoldView::new(&doc, 1).unwrap();
    view.set_collapsed(&doc, h(0), true).unwrap();
    let mut nav = Navigator::new();
    for (action, expected) in [
        (
            NavAction::NextHeading { max_level: 6 },
            doc.headings().entries()[10_001].source.start,
        ),
        (NavAction::PreviousHeading { max_level: 6 }, 0),
    ] {
        let mut task = FoldNavigation::new(
            &doc,
            &view,
            &nav,
            action,
            FoldNavigationPolicy::VisibleOnly,
            10,
        )
        .unwrap();
        let mut steps = 0;
        loop {
            steps += 1;
            let result = task.advance(&doc, &mut view, &mut tiny()).unwrap();
            assert!(result.change.is_none());
            if result.completion == Completion::Covered {
                let target = result.target.unwrap();
                assert_eq!(target.source, Some(expected));
                assert_eq!(steps, 4);
                nav.apply_target(target);
                break;
            }
            assert!(steps < 5);
        }
    }
    assert!(view.is_collapsed(h(0)));
}

#[test]
fn hundreds_of_open_scopes_and_million_byte_body_have_constant_append_work() {
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 2).unwrap();
    view.set_defaults(FoldDefaults {
        heading_levels: [true; 6],
        ..Default::default()
    });
    for depth in 1..=500 {
        let update = doc.append(&format!("{}# H\n", ">".repeat(depth))).unwrap();
        view.apply(&doc, &update).unwrap();
    }
    let update = doc
        .append(&format!("{}{}", ">".repeat(500), "x".repeat(1_000_000)))
        .unwrap();
    view.apply(&doc, &update).unwrap();
    let work = doc.work();
    let visits = view.range_visits();
    let index = view.section_index_visits();
    for _ in 0..1000 {
        let update = doc.append("x").unwrap();
        view.apply(&doc, &update).unwrap();
    }
    assert_eq!(doc.work().heading_steps, work.heading_steps);
    assert_eq!(doc.work().container_steps, work.container_steps);
    assert_eq!(view.range_visits() - visits, 1000);
    assert_eq!(view.section_index_visits(), index);
    let sink = render(&doc, &mut view);
    assert_eq!(sink.placeholders, 1);
    assert_eq!(sink.text, "H\n[fold]\n");
    assert!(sink.bytes < 16);
    let before = view.range_visits();
    let update = doc.append("\nout\n").unwrap();
    view.apply(&doc, &update).unwrap();
    assert_eq!(view.range_visits() - before, 501);
    let boundary = doc.source().find("out").unwrap();
    for i in 0..500 {
        assert_eq!(doc.fold_candidate(h(i)).unwrap().source.end, boundary);
    }
    assert!(matches!(
        view.hidden_at(&doc, boundary, &mut tiny()).unwrap(),
        FoldLookup::Visible
    ));
}

#[test]
fn long_title_growth_moves_placeholder_and_setext_confirmation_never_rescans() {
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 1).unwrap();
    view.set_defaults(FoldDefaults {
        heading_levels: [true; 6],
        ..Default::default()
    });
    let update = doc.append(&format!("> # {}", "x".repeat(2000))).unwrap();
    view.apply(&doc, &update).unwrap();
    for chunk in ["abc", "界", "\n", "> body\n", "> New\n", "> ===\n"] {
        let update = doc.append(chunk).unwrap();
        view.apply(&doc, &update).unwrap();
        verify(&doc, &view);
    }
    let update = doc
        .append(&format!("> {}\n", "z".repeat(1_000_000)))
        .unwrap();
    view.apply(&doc, &update).unwrap();
    let before = doc.work();
    let visits = view.section_index_visits();
    let update = doc.append("> ===\n").unwrap();
    view.apply(&doc, &update).unwrap();
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert!(view.section_index_visits() - visits < 40);
    assert_eq!(doc.headings().entries()[2].content.len(), 1_000_000);
    assert!(view.is_collapsed(h(2)));
    let update = doc.finish();
    view.apply(&doc, &update).unwrap();
    assert!(!doc.fold_candidate(h(2)).unwrap().open);
    assert!(doc.fold_candidate(h(2)).unwrap().hidden.is_empty());
}

#[test]
fn thousands_of_closed_sections_lookup_logarithmically_and_reuse_index_nodes() {
    let source = (0..4096)
        .map(|i| format!("> # H{i}\n> body\n"))
        .collect::<String>();
    let doc = parse(&source);
    let mut view = FoldView::new(&doc, 1).unwrap();
    for i in 0..4096 {
        view.set_collapsed(&doc, h(i), true).unwrap();
    }
    let nodes = view.section_index_nodes();
    assert!(nodes < 4 * 4096);
    for i in (0..4096).step_by(7) {
        let offset = doc.headings().entries()[i].source.end;
        let visits = view.section_index_visits();
        assert!(
            matches!(view.hidden_at(&doc, offset, &mut tiny()).unwrap(), FoldLookup::Hidden(c) if c.id == h(i))
        );
        assert!(view.section_index_visits() - visits < 40);
        view.set_collapsed(&doc, h(i), false).unwrap();
        view.set_collapsed(&doc, h(i), true).unwrap();
    }
    assert_eq!(view.section_index_nodes(), nodes);
}

#[test]
fn streaming_toggle_preview_and_resize_match_independent_interval_height_model() {
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 2).unwrap();
    view.set_defaults(FoldDefaults {
        heading_levels: [true, false, true, false, true, false],
        table: true,
        ..Default::default()
    });
    let mut seed = 0x5b123fd_u64;
    for step in 0..240 {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let depth = seed as usize % 5 + 1;
        let prefix = ">".repeat(depth);
        let chunk = match step % 7 {
            0 => format!("{prefix}{} H{step}\n", "#".repeat(seed as usize % 6 + 1)),
            1 => format!("{prefix}a|b\n{prefix}-|-\n{prefix}x|y\n"),
            2 => format!("{prefix}{}\n", "x".repeat(180)),
            3 => format!("{prefix}New\n{prefix}===\n"),
            4 => format!("{prefix}- # item\n{prefix}  body\n"),
            5 => "out\n".to_owned(),
            _ => format!("{prefix}## child"),
        };
        let update = doc.append(&chunk).unwrap();
        view.apply(&doc, &update).unwrap();
        let ids = all(&doc);
        let id = ids[seed as usize % ids.len()].id;
        match step % 11 {
            0 => {
                view.set_preview(&doc, Some(id), &mut tiny()).unwrap();
            }
            1 => {
                view.set_preview(&doc, None, &mut tiny()).unwrap();
            }
            2 => view.reset(1 + step % 5).unwrap(),
            _ => {
                view.toggle(&doc, id).unwrap();
            }
        }
        verify(&doc, &view);
    }
    let update = doc.finish();
    view.apply(&doc, &update).unwrap();
    verify(&doc, &view);
}
