use mdstream::*;

fn parse(s: &str) -> Document {
    let mut d = Document::new();
    d.append(s).unwrap();
    d.finish();
    d
}
fn budget(n: usize) -> Budget {
    Budget::new(WorkLimit {
        queries: n,
        objects: n,
        recovery: n,
        ..Default::default()
    })
}
fn complete(d: &Document, v: &mut FoldView, mut task: SourceFoldRegistration) -> FoldId {
    assert_eq!(task.advance(d, v, &mut budget(0)).unwrap().id, None);
    for _ in 0..10000 {
        let p = task.advance(d, v, &mut budget(1)).unwrap();
        if let Some(id) = p.id {
            assert_eq!(task.advance(d, v, &mut budget(0)).unwrap(), p);
            return id;
        }
    }
    panic!("registration stalled")
}
fn fixed(d: &Document, v: &mut FoldView, s: Span) -> FoldId {
    let t = SourceFoldRegistration::overlapping(d, v, s).unwrap();
    complete(d, v, t)
}
fn stream(d: &Document, v: &mut FoldView, at: usize) -> FoldId {
    let t = SourceFoldRegistration::stream(d, v, at).unwrap();
    complete(d, v, t)
}
fn source(id: FoldId) -> SourceFoldId {
    let FoldId::Source(id) = id else { panic!() };
    id
}
fn all(d: &Document, v: &FoldView, s: Span) -> Vec<FoldCandidate> {
    let mut t = SourceFoldCandidates::new(d, v, s).unwrap();
    let mut result = Vec::new();
    loop {
        let mut page = vec![];
        let done = t.advance(d, v, &mut budget(1), &mut page).unwrap();
        result.extend(page);
        if done == Completion::Covered {
            return result;
        }
    }
}
fn batch(d: &Document, v: &mut FoldView, a: FoldBatchAction) {
    let mut t = FoldBatch::new(d, v, a).unwrap();
    for _ in 0..20000 {
        if t.advance(d, v, &mut budget(1)).unwrap().completion == Completion::Covered {
            return;
        }
    }
    panic!("batch stalled")
}
#[derive(Default)]
struct Sink {
    text: String,
    placeholders: Vec<FoldPlaceholder>,
    rows: u64,
    ends: Vec<BlockId>,
}
impl LayoutSink for Sink {
    fn begin_tile(&mut self, _: TileId) {
        self.rows = 0
    }
    fn text(&mut self, r: RenderRun<'_>) {
        if !r.style.contains(Style::HIDDEN) {
            self.text.push_str(r.text);
            self.rows += r.text.bytes().filter(|b| *b == b'\n').count() as u64;
        }
    }
    fn object(&mut self, _: LinkId, _: &LinkInfo) {}
    fn end_tile(&mut self, _: TileId) -> u64 {
        self.rows
    }
}
impl FoldLayoutSink for Sink {
    fn placeholder(&mut self, p: FoldPlaceholder) {
        self.text.push('◇');
        self.rows += 1;
        self.placeholders.push(p);
    }
    fn paragraph_end(&mut self, b: BlockId) {
        self.ends.push(b)
    }
}
fn render(d: &Document, v: &mut FoldView, tiny: bool) -> Sink {
    let mut t = LayoutPass::folded(d, v, TileId(0), u64::MAX).unwrap();
    let mut s = Sink::default();
    for _ in 0..200000 {
        let mut b = Budget::new(if tiny {
            WorkLimit {
                queries: 1,
                objects: 1,
                recovery: 1,
                tiles: 1,
                bytes: 6,
                ..Default::default()
            }
        } else {
            WorkLimit::unlimited()
        });
        if t.advance_folded(d, v, &mut b, &mut s).unwrap() == Completion::EndOfDocument {
            return s;
        }
    }
    panic!("layout stalled")
}
// Independent union oracle: sort raw ranges, merge strict intersections, retain
// touching boundaries. No production index or registration validator is used.
fn components(mut cs: Vec<FoldCandidate>) -> Vec<(Span, FoldId)> {
    cs.retain(|c| !c.hidden.is_empty());
    cs.sort_by_key(|c| {
        (
            c.hidden.start,
            std::cmp::Reverse(c.hidden.end),
            match c.id {
                FoldId::Heading(_) => 0,
                FoldId::Container(_) => 1,
                _ => 2,
            },
            c.id,
        )
    });
    let mut union: Vec<(Span, FoldId)> = vec![];
    for c in cs {
        if let Some(last) = union.last_mut()
            && c.hidden.start < last.0.end
        {
            last.0.end = last.0.end.max(c.hidden.end)
        } else {
            union.push((c.hidden, c.id));
        }
    }
    union
}
fn verify(d: &Document, v: &mut FoldView, ids: &[FoldId], tiny: bool) {
    let candidates: Vec<_> = ids
        .iter()
        .filter(|id| v.is_collapsed(**id) && v.preview() != Some(**id))
        .filter_map(|id| v.fold_candidate(d, *id).unwrap())
        .collect();
    let union = components(candidates.clone());
    for at in (0..d.source().len()).step_by(11).chain([d.source().len()]) {
        let expected = union.iter().find(|(s, _)| s.range().contains(&at));
        let actual = v.hidden_at(d, at, &mut budget(1)).unwrap();
        match (expected, actual) {
            (None, FoldLookup::Visible) => {}
            (Some((s, owner)), FoldLookup::Hidden(c)) => {
                assert_eq!(c.id, *owner);
                assert_eq!(c.hidden.start, s.start);
                assert!(at < c.hidden.end && c.hidden.end <= s.end);
            }
            x => panic!("visibility at {at}: {x:?}"),
        }
    }
    // Complete tile masks remain the union of each selected interval's interior;
    // overlap boundary tiles can retain estimates until measured locally.
    for i in 0..d.tile_count() {
        let covered = candidates.iter().any(|c| {
            d.tile_at(c.hidden.start).unwrap().0 < i
                && i < d.tile_at(c.hidden.end).map_or(d.tile_count(), |t| t.0)
        });
        if covered {
            assert_eq!(v.heights().height(TileId(i)), Some(0));
        }
    }
    let mut expected = String::new();
    let mut at = 0;
    for (span, _) in &union {
        for r in d.runs_in(Span::new(at, span.start)).unwrap() {
            if !r.style.contains(Style::HIDDEN) {
                expected.push_str(r.text)
            }
        }
        expected.push('◇');
        at = span.end;
    }
    for r in d.runs_in(Span::new(at, d.source().len())).unwrap() {
        if !r.style.contains(Style::HIDDEN) {
            expected.push_str(r.text)
        }
    }
    let sink = render(d, v, tiny);
    assert_eq!(sink.text, expected);
    assert_eq!(
        sink.placeholders
            .iter()
            .map(|p| p.candidate.id)
            .collect::<Vec<_>>(),
        union.iter().map(|x| x.1).collect::<Vec<_>>()
    );
}

#[test]
fn nested_crossing_same_start_and_adjacent_ranges_follow_union_and_height_model() {
    let d = parse(&"0123456789abcdef\n".repeat(100));
    let mut v = FoldView::new(&d, 3).unwrap();
    let spans = [
        Span::new(20, 700),
        Span::new(60, 300),
        Span::new(500, 1000),
        Span::new(1000, 1200),
        Span::new(20, 450),
        Span::new(100, 1000),
        Span::new(1100, 1450),
    ];
    let ids: Vec<_> = spans.into_iter().map(|s| fixed(&d, &mut v, s)).collect();
    for mask in 0..128 {
        for (i, id) in ids.iter().enumerate() {
            v.set_collapsed(&d, *id, mask & (1 << i) != 0).unwrap();
        }
        verify(&d, &mut v, &ids, mask % 2 == 0);
    }
    assert_eq!(all(&d, &v, Span::new(110, 111)).len(), 4);
    assert_eq!(all(&d, &v, Span::new(0, d.source().len())).len(), ids.len());
}

#[test]
fn nested_choice_preview_subtree_focus_removal_and_two_views_are_independent() {
    let d = parse(&"abcde\n".repeat(150));
    let mut a = FoldView::new(&d, 1).unwrap();
    let b = FoldView::new(&d, 2).unwrap();
    let parent = fixed(&d, &mut a, Span::new(0, 800));
    let child = fixed(&d, &mut a, Span::new(30, 700));
    let grandchild = fixed(&d, &mut a, Span::new(100, 400));
    let crossing = fixed(&d, &mut a, Span::new(600, 880));
    let ids = [parent, child, grandchild, crossing];
    batch(&d, &mut a, FoldBatchAction::All { collapsed: true });
    a.set_collapsed(&d, parent, false).unwrap();
    assert!(a.is_collapsed(child) && a.is_collapsed(grandchild));
    a.set_preview(&d, Some(child), &mut budget(1)).unwrap();
    verify(&d, &mut a, &ids, true);
    a.set_collapsed(&d, child, false).unwrap();
    a.set_preview(&d, None, &mut budget(1)).unwrap();
    assert!(!a.is_collapsed(child));
    batch(
        &d,
        &mut a,
        FoldBatchAction::Subtree {
            root: child,
            collapsed: false,
        },
    );
    assert!(!a.is_collapsed(grandchild));
    assert!(a.is_collapsed(crossing));
    batch(&d, &mut a, FoldBatchAction::Focus { target: grandchild });
    assert!(!a.is_collapsed(parent) && !a.is_collapsed(child) && !a.is_collapsed(grandchild));
    assert!(a.is_collapsed(crossing));
    a.reset(4).unwrap();
    verify(&d, &mut a, &ids, false);
    a.remove_source_fold(&d, source(child)).unwrap();
    assert_eq!(a.source_fold_count(), 3);
    assert_eq!(b.source_fold_count(), 0);
    assert!(b.fold_candidate(&d, parent).unwrap().is_none());
}

#[test]
fn randomized_overlays_with_syntax_crossings_match_independent_union_model() {
    let d = parse(&format!(
        "# Outer\n{}\n```rust\n{}\n```\n## Inner\n{}",
        "ab\n".repeat(180),
        "cd\n".repeat(200),
        "ef\n".repeat(200)
    ));
    let mut v = FoldView::new(&d, 2).unwrap();
    let mut syntax = vec![];
    d.fold_candidates(Span::new(0, d.source().len()))
        .unwrap()
        .advance(&d, &mut Budget::new(WorkLimit::unlimited()), &mut syntax)
        .unwrap();
    let mut ids: Vec<_> = syntax.iter().map(|c| c.id).collect();
    let mut seed = 37u64;
    for _ in 0..80 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let start = seed as usize % (d.source().len() - 1);
        let end = start + 1 + (seed >> 32) as usize % (d.source().len() - start);
        ids.push(fixed(&d, &mut v, Span::new(start, end)));
    }
    for step in 0..500 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        v.toggle(&d, ids[seed as usize % ids.len()]).unwrap();
        if step % 19 == 0 {
            verify(&d, &mut v, &ids, true);
        }
    }
}

#[test]
fn anchors_project_to_component_start_reveal_only_actual_covering_folds() {
    let d = parse(&"abcdef\n".repeat(200));
    let mut v = FoldView::new(&d, 2).unwrap();
    let a = fixed(&d, &mut v, Span::new(0, 500));
    let b = fixed(&d, &mut v, Span::new(400, 1000));
    v.set_collapsed(&d, a, true).unwrap();
    v.set_collapsed(&d, b, true).unwrap();
    let anchor = FoldAnchor::new(&d, 800).unwrap();
    let p = anchor.resolve(&d, &v, 20, &mut budget(1)).unwrap().unwrap();
    assert_eq!(p.hidden_by, Some(a));
    assert_eq!(p.target.source, Some(0));
    let mut nav = FoldNavigation::new(
        &d,
        &v,
        &Navigator::new(),
        NavAction::Source(800),
        FoldNavigationPolicy::Reveal,
        20,
    )
    .unwrap();
    for _ in 0..10 {
        if nav.advance(&d, &mut v, &mut budget(1)).unwrap().completion == Completion::Covered {
            break;
        }
    }
    assert!(v.is_collapsed(a));
    assert!(!v.is_collapsed(b));
    assert_eq!(
        anchor
            .resolve(&d, &v, 20, &mut budget(1))
            .unwrap()
            .unwrap()
            .target
            .source,
        Some(800)
    );
}

#[test]
fn pending_and_all_syntax_hosts_stream_across_reclassification_every_utf8_cut() {
    let corpora = [
        "中文🙂\r\n---\r\nnext\n",
        "| a | b |\n| - | - |\n| x | y |\n",
        "> - para\n>   more\n\n# out\n",
        "```rust\nbody\n```\nout\n",
        "$$\nx+y\n$$\nout\n",
        "[r]: /url\n \"title\"\nnext\n",
    ];
    for input in corpora {
        let boundaries: Vec<_> = (0..=input.len())
            .filter(|n| input.is_char_boundary(*n))
            .collect();
        for &cut in boundaries.iter().filter(|n| **n > 0) {
            let mut d = Document::new();
            d.append(&input[..cut]).unwrap();
            let mut v = FoldView::new(&d, 2).unwrap();
            let mut ids = vec![];
            let last = *boundaries.iter().rfind(|n| **n < cut).unwrap();
            for start in if last == 0 { vec![0] } else { vec![0, last] } {
                ids.push(stream(&d, &mut v, start));
            }
            for id in &ids {
                v.set_collapsed(&d, *id, true).unwrap();
            }
            for w in boundaries.windows(2).filter(|w| w[0] >= cut) {
                let u = d.append(&input[w[0]..w[1]]).unwrap();
                v.apply(&d, &u).unwrap();
                for id in &ids {
                    let c = v.fold_candidate(&d, *id).unwrap().unwrap();
                    assert_eq!(c.source.end, d.source().len());
                    assert!(c.open);
                }
                verify(&d, &mut v, &ids, true);
            }
            let u = d.finish();
            v.apply(&d, &u).unwrap();
            for id in &ids {
                assert!(!v.fold_candidate(&d, *id).unwrap().unwrap().open);
            }
            verify(&d, &mut v, &ids, true);
            let baseline = parse(input);
            assert_eq!(d.source(), baseline.source());
            assert_eq!(d.blocks(), baseline.blocks());
            assert_eq!(
                d.runs_in(Span::new(0, d.source().len()))
                    .unwrap()
                    .map(|r| (r.source, r.text, r.style))
                    .collect::<Vec<_>>(),
                baseline
                    .runs_in(Span::new(0, baseline.source().len()))
                    .unwrap()
                    .map(|r| (r.source, r.text, r.style))
                    .collect::<Vec<_>>()
            );
        }
    }
}

#[test]
fn many_live_nested_tails_have_constant_append_work_and_budgeted_deep_registration() {
    let mut d = Document::new();
    d.append(&"x".repeat(5000)).unwrap();
    let mut v = FoldView::new(&d, 2).unwrap();
    let ids: Vec<_> = (0..4096)
        .map(|at| {
            let id = stream(&d, &mut v, at);
            v.set_collapsed(&d, id, true).unwrap();
            id
        })
        .collect();
    let u = d.append(&"y".repeat(1_000_000)).unwrap();
    v.apply(&d, &u).unwrap();
    for _ in 0..20 {
        let visits = v.range_visits();
        let work = v.heights().mutation_visits();
        let indexes = (v.fold_index_visits(), v.source_index_visits());
        let u = d.append("z").unwrap();
        v.apply(&d, &u).unwrap();
        assert!(v.range_visits() - visits <= 2);
        assert!(v.heights().mutation_visits() - work < 1000);
        assert_eq!((v.fold_index_visits(), v.source_index_visits()), indexes);
    }
    for id in &ids {
        assert_eq!(
            v.fold_candidate(&d, *id).unwrap().unwrap().source.end,
            d.source().len()
        );
    }
    let sink = render(&d, &mut v, true);
    assert_eq!(sink.text, "◇");
    assert_eq!(sink.placeholders.len(), 1);
    let actual = v.fold_candidate(&d, ids[0]).unwrap().unwrap().hidden;
    let change = v.remove_source_fold(&d, source(ids[0])).unwrap().unwrap();
    assert_eq!(change.source, actual);
    let mut deep = Document::new();
    deep.append(&format!("{}body\n", "> ".repeat(1000)))
        .unwrap();
    let mut view = FoldView::new(&deep, 1).unwrap();
    let mut t = SourceFoldRegistration::stream(&deep, &view, 1000).unwrap();
    let mut steps = 0;
    loop {
        let p = t.advance(&deep, &mut view, &mut budget(1)).unwrap();
        steps += 1;
        if p.id.is_some() {
            assert!(p.visited > 100);
            break;
        }
        assert_eq!(view.source_fold_count(), 0);
    }
    assert!(steps > 100);
}

#[test]
fn multiple_seals_share_eof_tile_then_preserve_fixed_cuts_and_ticket_invalidation() {
    let mut d = Document::new();
    d.append(&format!("{}\n\n", "abcde\n".repeat(170))).unwrap();
    let mut v = FoldView::new(&d, 2).unwrap();
    let ids: Vec<_> = [0, 10, 200]
        .into_iter()
        .map(|s| stream(&d, &mut v, s))
        .collect();
    for id in &ids {
        v.set_collapsed(&d, *id, true).unwrap();
    }
    let end = d.source().len();
    let old = v.heights().ticket(TileId(0)).unwrap();
    for id in &ids {
        let t = SourceFoldRegistration::seal_tail(&d, &v, source(*id)).unwrap();
        assert_eq!(complete(&d, &mut v, t), *id);
    }
    assert!(v.measure(old, 8).is_err());
    let u = d.append("visible\n").unwrap();
    v.apply(&d, &u).unwrap();
    for id in &ids {
        assert_eq!(v.fold_candidate(&d, *id).unwrap().unwrap().source.end, end);
    }
    verify(&d, &mut v, &ids, true);
    assert!(render(&d, &mut v, false).text.ends_with("visible\n"));
    for id in ids {
        v.remove_source_fold(&d, source(id)).unwrap();
    }
    assert_eq!(v.source_fold_count(), 0);
    assert_eq!((v.source_index_nodes(), v.fold_index_nodes()), (0, 0));
    assert_eq!(render(&d, &mut v, true).text, d.source());
}

#[test]
fn paragraph_preview_survives_overlay_nesting_then_reclassifies_without_id_transfer() {
    let mut d = Document::new();
    d.append(&format!("> {}\n", "a".repeat(600))).unwrap();
    let mut v = FoldView::new(&d, 2).unwrap();
    let tail = stream(&d, &mut v, 10);
    let t = SourceFoldRegistration::paragraph_overlay(&d, &v, BlockId(0), 20).unwrap();
    let p = complete(&d, &mut v, t);
    v.set_collapsed(&d, p, true).unwrap();
    let s = render(&d, &mut v, true);
    assert_eq!(s.ends, vec![BlockId(0)]);
    v.set_collapsed(&d, tail, true).unwrap();
    verify(&d, &mut v, &[p, tail], true);
    v.set_collapsed(&d, tail, false).unwrap();
    assert!(v.is_collapsed(p));
    let u = d.append("> more\n> ---\n").unwrap();
    v.apply(&d, &u).unwrap();
    assert!(v.fold_candidate(&d, p).unwrap().is_none());
    assert!(v.fold_candidate(&d, tail).unwrap().is_some());
    assert!(!d.headings().entries().is_empty());
}

fn restore(d: &Document, v: &mut FoldView, s: &SourceFoldSnapshot, host: SourceFoldHost) -> FoldId {
    let mut task = SourceFoldRestore::new(d, v, s, host).unwrap();
    assert_eq!(task.previous_id(), s.id);
    let count = v.source_fold_count();
    for _ in 0..20000 {
        // No recovery allowance: validation may progress but publication cannot.
        let mut b = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        let p = task.advance(d, v, &mut b).unwrap();
        assert_eq!(p.id, None);
        assert_eq!(v.source_fold_count(), count);
        if b.used().queries == 0 {
            break;
        }
    }
    let p = task.advance(d, v, &mut budget(1)).unwrap();
    assert_eq!(p.completion, Completion::Covered);
    let id = p.id.unwrap();
    assert_ne!(source(id), s.id);
    assert_eq!(task.advance(d, v, &mut budget(0)).unwrap(), p);
    id
}

#[test]
fn explicit_reload_maps_hosts_to_new_ids_and_restores_nested_saved_intent() {
    let old = parse("# first\naaaa\n\n# host\n0123456789\n\n");
    let mut before = FoldView::new(&old, 1).unwrap();
    let host = FoldId::Heading(HeadingId(1));
    let base = old.fold_candidate(host).unwrap().source.start;
    let a = fixed(&old, &mut before, Span::new(base + 7, base + 15));
    let b = fixed(&old, &mut before, Span::new(base + 9, base + 12));
    before.set_collapsed(&old, a, true).unwrap();
    before.set_collapsed(&old, b, true).unwrap();
    before.set_preview(&old, Some(a), &mut budget(1)).unwrap();
    let sa = SourceFoldSnapshot::capture(&old, &before, source(a), SourceFoldHost::Syntax(host))
        .unwrap();
    let sb = SourceFoldSnapshot::capture(&old, &before, source(b), SourceFoldHost::Syntax(host))
        .unwrap();
    let new = parse("# inserted\nnew\n# first\naaaa\n\n# host\n0123456789\n\n");
    let mut after = FoldView::new(&new, 3).unwrap();
    let mapped = SourceFoldHost::Syntax(FoldId::Heading(HeadingId(2)));
    let na = restore(&new, &mut after, &sa, mapped);
    let nb = restore(&new, &mut after, &sb, mapped);
    assert!(after.is_collapsed(na) && after.is_collapsed(nb));
    assert_eq!(after.preview(), None);
    let base = new
        .fold_candidate(FoldId::Heading(HeadingId(2)))
        .unwrap()
        .source
        .start;
    assert_eq!(
        after.fold_candidate(&new, na).unwrap().unwrap().source,
        Span::new(base + 7, base + 15)
    );
    verify(&new, &mut after, &[na, nb], true);
    after.set_collapsed(&new, na, false).unwrap();
    assert!(after.is_collapsed(nb));
    assert!(SourceFoldRestore::new(&new, &after, &sa, SourceFoldHost::Document).is_err());
    assert!(
        SourceFoldRestore::new(
            &new,
            &after,
            &sa,
            SourceFoldHost::Syntax(FoldId::Code(BlockId(0)))
        )
        .is_err()
    );
}

#[test]
fn reload_open_tail_and_container_paragraph_keep_explicit_host_contracts() {
    let mut d = Document::new();
    d.append(&format!("> {}\n", "x".repeat(300))).unwrap();
    let mut v = FoldView::new(&d, 1).unwrap();
    let tail = stream(&d, &mut v, 4);
    let t = SourceFoldRegistration::paragraph_overlay(&d, &v, BlockId(0), 10).unwrap();
    let para = complete(&d, &mut v, t);
    v.set_collapsed(&d, tail, true).unwrap();
    v.set_collapsed(&d, para, true).unwrap();
    assert!(
        SourceFoldSnapshot::capture(
            &d,
            &v,
            source(tail),
            SourceFoldHost::Syntax(FoldId::Container(ContainerId(0)))
        )
        .is_ok()
    );
    let st = SourceFoldSnapshot::capture(&d, &v, source(tail), SourceFoldHost::Document).unwrap();
    let sp =
        SourceFoldSnapshot::capture(&d, &v, source(para), SourceFoldHost::Paragraph(BlockId(0)))
            .unwrap();
    let mut new = Document::new();
    new.append(d.source()).unwrap();
    let mut after = FoldView::new(&new, 1).unwrap();
    let nt = restore(&new, &mut after, &st, SourceFoldHost::Document);
    let np = restore(&new, &mut after, &sp, SourceFoldHost::Paragraph(BlockId(0)));
    let u = new.append("> more\n").unwrap();
    after.apply(&new, &u).unwrap();
    assert_eq!(
        after.fold_candidate(&new, nt).unwrap().unwrap().source.end,
        new.source().len()
    );
    assert_eq!(
        after.paragraph_fold_block(&new, source(np)).unwrap(),
        Some(BlockId(0))
    );
    after.set_collapsed(&new, nt, false).unwrap();
    let s = render(&new, &mut after, true);
    assert_eq!(s.ends, vec![BlockId(0)]);
}

#[test]
fn registrations_discovery_recovery_and_layout_reject_stale_and_foreign_views() {
    let mut d = Document::new();
    d.append("pending body").unwrap();
    let mut a = FoldView::new(&d, 1).unwrap();
    let mut b = FoldView::new(&d, 1).unwrap();
    let mut t = SourceFoldRegistration::stream(&d, &a, 1).unwrap();
    assert_eq!(
        t.advance(&d, &mut b, &mut budget(1)),
        Err(LayoutError::WrongView)
    );
    let id = stream(&d, &mut a, 0);
    a.set_collapsed(&d, id, true).unwrap();
    assert_eq!(
        t.advance(&d, &mut a, &mut budget(1)),
        Err(LayoutError::StaleContinuation)
    );
    let saved = SourceFoldSnapshot::capture(&d, &a, source(id), SourceFoldHost::Document).unwrap();
    let mut restore = SourceFoldRestore::new(&d, &b, &saved, SourceFoldHost::Document).unwrap();
    let mut discover = SourceFoldCandidates::new(&d, &a, Span::new(0, 1)).unwrap();
    let mut pass = LayoutPass::folded(&d, &a, TileId(0), 10).unwrap();
    let mut u1 = d.append("\n").unwrap();
    let u2 = d.append("\nend\n").unwrap();
    u1.merge(u2).unwrap();
    let merged = u1;
    a.apply(&d, &merged).unwrap();
    b.apply(&d, &merged).unwrap();
    a.apply(&d, &merged).unwrap();
    assert_eq!(
        restore.advance(&d, &mut b, &mut budget(1)),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(
        discover.advance(&d, &a, &mut budget(1), &mut vec![]),
        Err(LayoutError::StaleContinuation)
    );
    assert!(
        pass.advance_folded(&d, &mut a, &mut budget(1), &mut Sink::default())
            .is_err()
    );
    assert_eq!(
        a.fold_candidate(&d, id).unwrap().unwrap().source.end,
        d.source().len()
    );
    let other = parse("other\n");
    assert_eq!(
        a.hidden_at(&other, 0, &mut budget(0)),
        Err(LayoutError::WrongDocument)
    );
}

#[test]
fn pending_fixed_cuts_and_explicit_seals_survive_new_syntax_in_both_orders() {
    for input in [
        "a\n---\n",
        "a | b\n- | -\n",
        "> ```\ncode\n> ```\n",
        "[r]: /url\n 'title'\n",
    ] {
        let mut d = Document::new();
        d.append(&input[..1]).unwrap();
        let mut v = FoldView::new(&d, 2).unwrap();
        let id = stream(&d, &mut v, 0);
        v.set_collapsed(&d, id, true).unwrap();
        let t = SourceFoldRegistration::seal_tail(&d, &v, source(id)).unwrap();
        assert_eq!(complete(&d, &mut v, t), id);
        let u = d.append(&input[1..]).unwrap();
        v.apply(&d, &u).unwrap();
        assert_eq!(
            v.fold_candidate(&d, id).unwrap().unwrap().source,
            Span::new(0, 1)
        );
        verify(&d, &mut v, &[id], true);
        let u = d.finish();
        v.apply(&d, &u).unwrap();
        verify(&d, &mut v, &[id], true);
    }
    for first_paragraph in [false, true] {
        let mut d = Document::new();
        d.append(&format!("> {}\n", "b".repeat(300))).unwrap();
        let mut v = FoldView::new(&d, 1).unwrap();
        let mut ids = vec![];
        if !first_paragraph {
            ids.push(stream(&d, &mut v, 4));
        }
        let t = SourceFoldRegistration::paragraph_overlay(&d, &v, BlockId(0), 20).unwrap();
        ids.push(complete(&d, &mut v, t));
        if first_paragraph {
            ids.push(stream(&d, &mut v, 4));
        }
        for id in &ids {
            v.set_collapsed(&d, *id, true).unwrap();
        }
        let u = d.append("> more\n").unwrap();
        v.apply(&d, &u).unwrap();
        verify(&d, &mut v, &ids, true);
    }
}

#[test]
fn overlay_rows_cross_in_both_orders_and_do_not_mask_legacy_strict_validation() {
    let d = parse("| h |\n| - |\n| a |\n| b |\n| c |\n");
    let table = d.table(TableId(0)).unwrap();
    let a = table.row(1).unwrap().source;
    let b = table.row(2).unwrap().source;
    let selection = Span::new(a.start + 2, b.start + 2);
    for rows_first in [false, true] {
        let mut v = FoldView::new(&d, 1).unwrap();
        let mut ids = vec![];
        if rows_first {
            ids.push(v.insert_table_rows(&d, TableId(0), 1..2).unwrap());
        }
        ids.push(fixed(&d, &mut v, selection));
        if !rows_first {
            ids.push(v.insert_table_rows(&d, TableId(0), 1..2).unwrap());
        }
        for id in &ids {
            v.set_collapsed(&d, *id, true).unwrap();
        }
        verify(&d, &mut v, &ids, true);
    }
    let mut v = FoldView::new(&d, 1).unwrap();
    let t = SourceFoldRegistration::new(&d, &v, selection).unwrap();
    complete(&d, &mut v, t);
    fixed(&d, &mut v, Span::new(0, d.source().len()));
    assert_eq!(
        v.insert_table_rows(&d, TableId(0), 1..2),
        Err(LayoutError::InvalidRange)
    );
}

#[test]
fn registration_storage_cap_and_final_recovery_publication_are_atomic() {
    let d = parse("x");
    let mut v = FoldView::new(&d, 1).unwrap();
    fixed(&d, &mut v, Span::new(0, 1));
    let mut t = SourceFoldRegistration::overlapping(&d, &v, Span::new(0, 1)).unwrap();
    assert_eq!(
        t.advance(&d, &mut v, &mut Budget::new(WorkLimit::unlimited())),
        Err(LayoutError::InvalidRange)
    );
    assert_eq!(v.source_fold_count(), 1);
}

#[test]
fn nested_parent_lookup_and_siblings_reparent_lazily_without_rewriting_descendants() {
    let d = parse(&"abcd\n".repeat(250));
    let mut v = FoldView::new(&d, 1).unwrap();
    let outer = fixed(&d, &mut v, Span::new(0, 1100));
    let child = fixed(&d, &mut v, Span::new(100, 400));
    let grandchild = fixed(&d, &mut v, Span::new(150, 250));
    let peer = fixed(&d, &mut v, Span::new(500, 1000));
    let peer_child = fixed(&d, &mut v, Span::new(600, 800));
    for (id, parent) in [
        (child, outer),
        (grandchild, child),
        (peer, outer),
        (peer_child, peer),
    ] {
        assert_eq!(
            v.fold_candidate(&d, id).unwrap().unwrap().parent,
            Some(parent)
        );
    }
    batch(
        &d,
        &mut v,
        FoldBatchAction::Siblings {
            target: child,
            collapsed: true,
        },
    );
    assert!(v.is_collapsed(peer));
    for id in [outer, child, grandchild, peer_child] {
        assert!(!v.is_collapsed(id));
    }
    v.remove_source_fold(&d, source(child)).unwrap();
    assert_eq!(
        v.fold_candidate(&d, grandchild).unwrap().unwrap().parent,
        Some(outer)
    );
    v.remove_source_fold(&d, source(outer)).unwrap();
    assert_eq!(
        v.fold_candidate(&d, grandchild).unwrap().unwrap().parent,
        None
    );
    assert!(v.is_collapsed(peer));
}

#[test]
fn shared_eof_groups_handle_exact_tiles_unicode_rollover_and_independent_removal() {
    for size in [252, 253, 254, 255, 256, 510, 511, 512] {
        for appended in ["x", "🙂", "\n"] {
            let mut d = Document::new();
            d.append(&format!("{}\n\n", "a".repeat(size))).unwrap();
            let mut v = FoldView::new(&d, 2).unwrap();
            let end = d.source().len();
            let ids: Vec<_> = [0, 10, 100]
                .into_iter()
                .map(|s| fixed(&d, &mut v, Span::new(s, end)))
                .collect();
            for id in &ids {
                v.set_collapsed(&d, *id, true).unwrap();
            }
            let u = d.append(appended).unwrap();
            v.apply(&d, &u).unwrap();
            verify(&d, &mut v, &ids, true);
            for (i, id) in ids.iter().enumerate() {
                v.remove_source_fold(&d, source(*id)).unwrap();
                verify(&d, &mut v, &ids[i + 1..], true);
            }
        }
    }
}

#[test]
fn paragraph_union_frontiers_end_once_in_full_and_remote_budgeted_layout() {
    let d = parse(&format!("{}\n\n{}\n\n", "a".repeat(498), "b".repeat(498)));
    let mut v = FoldView::new(&d, 1).unwrap();
    let t = SourceFoldRegistration::paragraph_overlay(&d, &v, BlockId(0), 20).unwrap();
    let paragraph = complete(&d, &mut v, t);
    let crossing = fixed(&d, &mut v, Span::new(300, 900));
    for id in [paragraph, crossing] {
        v.set_collapsed(&d, id, true).unwrap();
    }
    assert_eq!(render(&d, &mut v, true).ends, vec![BlockId(0)]);
    let mut task = LayoutPass::folded(&d, &v, TileId(2), u64::MAX).unwrap();
    let mut sink = Sink::default();
    loop {
        let mut b = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            recovery: 1,
            bytes: 6,
            tiles: 1,
            ..Default::default()
        });
        if task.advance_folded(&d, &mut v, &mut b, &mut sink).unwrap() == Completion::EndOfDocument
        {
            break;
        }
    }
    assert_eq!(sink.ends, vec![BlockId(0)]);
    assert!(sink.placeholders.is_empty());
}

#[test]
fn reload_live_tail_maps_closed_syntax_start_across_inserted_prefix() {
    let body = "```rust\ncode\n```\nafter\n\n";
    for old_finished in [false, true] {
        let mut d = Document::new();
        d.append(body).unwrap();
        let mut before = FoldView::new(&d, 1).unwrap();
        let tail = stream(&d, &mut before, 9);
        before.set_collapsed(&d, tail, true).unwrap();
        if old_finished {
            let u = d.finish();
            before.apply(&d, &u).unwrap();
        }
        let saved = SourceFoldSnapshot::capture(
            &d,
            &before,
            source(tail),
            SourceFoldHost::Syntax(FoldId::Code(BlockId(0))),
        )
        .unwrap();
        for finished in [false, true] {
            let prefix = "# inserted\n\n";
            let mut new = Document::new();
            new.append(&format!("{prefix}{body}")).unwrap();
            if finished {
                new.finish();
            }
            let code = new
                .blocks()
                .iter()
                .position(|b| matches!(b.kind, BlockKind::Code { .. }))
                .unwrap();
            let mut after = FoldView::new(&new, 2).unwrap();
            let id = restore(
                &new,
                &mut after,
                &saved,
                SourceFoldHost::Syntax(FoldId::Code(BlockId(code))),
            );
            let c = after.fold_candidate(&new, id).unwrap().unwrap();
            assert_eq!(c.source, Span::new(prefix.len() + 9, new.source().len()));
            assert_eq!(c.open, !finished && !old_finished);
            if !finished {
                let end = new.source().len();
                let u = new.append("new tail\n").unwrap();
                after.apply(&new, &u).unwrap();
                assert_eq!(
                    after.fold_candidate(&new, id).unwrap().unwrap().source.end,
                    if old_finished {
                        end
                    } else {
                        new.source().len()
                    }
                );
            }
            verify(&new, &mut after, &[id], true);
        }
    }
}

#[test]
fn finish_keeps_collapsed_syntax_parent_placeholder_over_same_start_stream_child() {
    for (input, parent) in [
        ("# h\nbody\n", FoldId::Heading(HeadingId(0))),
        ("> body\n", FoldId::Container(ContainerId(0))),
    ] {
        let mut d = Document::new();
        d.append(input).unwrap();
        let mut v = FoldView::new(&d, 1).unwrap();
        let cut = d.fold_candidate(parent).unwrap().hidden.start;
        let child = stream(&d, &mut v, cut);
        for id in [parent, child] {
            v.set_collapsed(&d, id, true).unwrap();
        }
        for finished in [false, true] {
            if finished {
                let u = d.finish();
                v.apply(&d, &u).unwrap();
            }
            assert!(
                matches!(v.hidden_at(&d, cut, &mut budget(1)).unwrap(), FoldLookup::Hidden(c) if c.id == parent)
            );
            let s = render(&d, &mut v, true);
            assert_eq!(s.placeholders.len(), 1);
            assert_eq!(s.placeholders[0].candidate.id, parent);
        }
        v.set_collapsed(&d, child, false).unwrap();
        v.set_collapsed(&d, child, true).unwrap();
        assert!(
            matches!(v.hidden_at(&d, cut, &mut budget(1)).unwrap(), FoldLookup::Hidden(c) if c.id == parent)
        );
        v.set_collapsed(&d, parent, false).unwrap();
        assert!(
            matches!(v.hidden_at(&d, cut, &mut budget(1)).unwrap(), FoldLookup::Hidden(c) if c.id == child)
        );
    }
}

#[test]
fn equal_envelopes_use_the_same_tail_order_for_parents_subtree_and_focus() {
    let mut d = Document::new();
    d.append("body\n\n").unwrap();
    let mut v = FoldView::new(&d, 1).unwrap();
    let fixed = fixed(&d, &mut v, Span::new(0, d.source().len()));
    let tail = stream(&d, &mut v, 0);
    for finished in [false, true] {
        if finished {
            let u = d.finish();
            v.apply(&d, &u).unwrap();
        }
        assert_eq!(
            v.fold_candidate(&d, fixed).unwrap().unwrap().parent,
            Some(tail)
        );
        assert_eq!(v.fold_candidate(&d, tail).unwrap().unwrap().parent, None);
        batch(
            &d,
            &mut v,
            FoldBatchAction::Subtree {
                root: fixed,
                collapsed: true,
            },
        );
        assert!(v.is_collapsed(fixed));
        assert!(!v.is_collapsed(tail));
        batch(&d, &mut v, FoldBatchAction::Focus { target: fixed });
        assert!(!v.is_collapsed(fixed));
        assert!(!v.is_collapsed(tail));
        batch(
            &d,
            &mut v,
            FoldBatchAction::Subtree {
                root: tail,
                collapsed: true,
            },
        );
        assert!(v.is_collapsed(fixed) && v.is_collapsed(tail));
        v.set_collapsed(&d, tail, false).unwrap();
    }
}
