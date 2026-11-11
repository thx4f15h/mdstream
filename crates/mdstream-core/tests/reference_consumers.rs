use mdstream::*;
fn parse(s: &str) -> Document {
    let mut d = Document::new();
    d.append(s).unwrap();
    d.finish();
    d
}
fn frame(bytes: usize) -> WorkLimit {
    WorkLimit {
        bytes,
        queries: 1,
        objects: 1,
        tiles: 1,
        dependencies: 1,
        recovery: 1,
    }
}
fn bound(b: &Budget, l: WorkLimit) {
    let u = b.used();
    assert!(
        u.bytes <= l.bytes
            && u.queries <= l.queries
            && u.objects <= l.objects
            && u.dependencies <= l.dependencies
            && u.tiles <= l.tiles
            && u.recovery <= l.recovery
    );
}
#[derive(Default)]
struct Sink {
    text: String,
    objects: Vec<LinkId>,
    placeholders: Vec<Span>,
}
impl LayoutSink for Sink {
    fn begin_tile(&mut self, _: TileId) {}
    fn text(&mut self, r: RenderRun<'_>) {
        if !r.style.contains(Style::HIDDEN) {
            self.text.push_str(r.text)
        }
    }
    fn object(&mut self, id: LinkId, _: &LinkInfo) {
        self.objects.push(id)
    }
    fn end_tile(&mut self, _: TileId) -> u64 {
        1
    }
}
impl FoldLayoutSink for Sink {
    fn placeholder(&mut self, p: FoldPlaceholder) {
        self.placeholders.push(p.candidate.hidden)
    }
}
#[test]
fn budgeted_consumers_follow_current_reference_boundaries() {
    let mut checks = 0;
    for source in [
        "[a][b]",
        "![*a*][]",
        "> [a\n> b][c]\n",
        "[a][*b*&amp;\\!]",
        "![][b]",
    ] {
        let d = parse(source);
        let span = Span::new(0, source.len());
        let mut scratch = QueryScratch::default();
        assert_eq!(
            d.runs_in(span).unwrap().collect::<Vec<_>>(),
            d.runs_in_with_scratch(span, &mut scratch)
                .unwrap()
                .collect::<Vec<_>>()
        );
        assert_eq!(
            d.display_runs_in(span).unwrap().collect::<Vec<_>>(),
            d.display_runs_in_with_scratch(span, &mut scratch)
                .unwrap()
                .collect::<Vec<_>>()
        );
        for l in [
            frame(0),
            frame(1),
            WorkLimit {
                queries: 0,
                ..frame(6)
            },
            WorkLimit {
                objects: 0,
                ..frame(6)
            },
        ] {
            let mut c = d.display_cursor(span).unwrap();
            let mut b = Budget::new(l);
            assert_eq!(c.next(&d, &mut b), Ok(DisplayStep::BudgetExhausted));
            assert_eq!(c.source_position(), 0);
            bound(&b, l);
            checks += 1;
        }
        let mut heights = HeightIndex::new(1).unwrap();
        heights.rebuild(&d).unwrap();
        let mut pass = LayoutPass::new(&d, &heights, TileId(0), 100, true).unwrap();
        let mut sink = Sink::default();
        let mut done = false;
        for _ in 0..1000 {
            let l = frame(6);
            let mut b = Budget::new(l);
            done = pass.advance(&d, &mut heights, &mut b, &mut sink).unwrap()
                == Completion::EndOfDocument;
            bound(&b, l);
            if done {
                break;
            }
        }
        assert!(done);
        let expected: String = d
            .runs_in(span)
            .unwrap()
            .filter(|r| !r.style.contains(Style::HIDDEN))
            .map(|r| r.text)
            .collect();
        assert_eq!(sink.text, expected);
        assert_eq!(sink.objects.len(), d.links().len());
        for zero in ["query", "object", "dependency"] {
            let l = match zero {
                "query" => WorkLimit {
                    queries: 0,
                    ..frame(6)
                },
                "object" => WorkLimit {
                    objects: 0,
                    ..frame(6)
                },
                _ => WorkLimit {
                    dependencies: 0,
                    ..frame(6)
                },
            };
            let mut b = Budget::new(l);
            assert!(
                DependencyKey::capture(&d, TileId(0), &mut b)
                    .unwrap()
                    .is_none()
            );
            bound(&b, l);
            checks += 1;
        }
        for (at, _) in source.char_indices() {
            let expected = d
                .runs_in(Span::new(at, source.len()))
                .unwrap()
                .find(|r| !r.style.contains(Style::HIDDEN))
                .map(|r| r.source.start);
            assert_eq!(d.next_visible(at), expected);
        }
    }
    // Three cursors deliberately share the SAME six-byte budget.
    let d = parse("![a][b]\n\n[b]: /u&amp;v 'T&amp;Z'\n");
    let info = *d.link_info(LinkId(0)).unwrap();
    let mut display = d.display_cursor(info.source).unwrap();
    let mut url = d.decoded_link_destination(LinkId(0)).unwrap();
    let mut title = d.decoded_link_title(LinkId(0)).unwrap();
    let (mut u, mut t) = (String::new(), String::new());
    let mut done = [false; 3];
    let mut frames = 0;
    for _ in 0..1000 {
        let l = frame(6);
        let mut b = Budget::new(l);
        if !done[0] {
            done[0] = display.next(&d, &mut b).unwrap() == DisplayStep::Complete;
        }
        if !done[1] {
            match url.advance(&mut b) {
                Ok(Some(c)) => u.push_str(c.text),
                Ok(None) => done[1] = true,
                Err(Completion::BudgetExhausted) => {}
                v => panic!("{v:?}"),
            }
        }
        if !done[2] {
            match title.advance(&mut b) {
                Ok(Some(c)) => t.push_str(c.text),
                Ok(None) => done[2] = true,
                Err(Completion::BudgetExhausted) => {}
                v => panic!("{v:?}"),
            }
        }
        bound(&b, l);
        frames += 1;
        if done.iter().all(|x| *x) {
            break;
        }
    }
    assert!(done.iter().all(|x| *x));
    assert_eq!(u, "/u&v");
    assert_eq!(t, "T&Z");
    // Visibility already invalidates tickets even with references_affect_size=false.
    let mut d = Document::new();
    let first = d.append("[a][b] rest\n\n").unwrap();
    let info = *d.link_info(LinkId(0)).unwrap();
    let mut heights = HeightIndex::new(1).unwrap();
    let mut checkpoints = CheckpointIndex::<usize>::new();
    let policy = LayoutPolicy {
        style_affects_size: false,
        references_affect_size: false,
        table_width_affects_size: false,
    };
    apply_layout_update_with_policy(&d, &first, &mut heights, &mut checkpoints, policy).unwrap();
    let ticket = heights.ticket(TileId(0)).unwrap();
    heights.measure(ticket, 3).unwrap();
    let key = DependencyKey::capture(&d, TileId(0), &mut Budget::new(WorkLimit::unlimited()))
        .unwrap()
        .unwrap();
    let mut stale = d.display_cursor(info.source).unwrap();
    let mut search = DisplaySearch::new(&d, "[").unwrap();
    let mut excerpt = DisplayExcerpt::new(
        &d,
        info.source,
        ExcerptLimits {
            source_bytes: 100,
            display_bytes: 100,
            lines: 10,
        },
    )
    .unwrap();
    let mut raw = SourceSearch::new(&d, "[a][b]").unwrap();
    let mut hits = Vec::new();
    raw.advance(&d, &mut Budget::new(WorkLimit::unlimited()), &mut hits)
        .unwrap();
    assert_eq!(hits, [Span::new(0, 6)]);
    let mut view = FoldView::new(&d, 1).unwrap();
    let mut reg = SourceFoldRegistration::paragraph(&d, &view, BlockId(0), 6).unwrap();
    let fold = reg
        .advance(&d, &mut view, &mut Budget::new(WorkLimit::unlimited()))
        .unwrap()
        .id
        .unwrap();
    let oldfold = view.fold_candidate(&d, fold).unwrap().unwrap();
    view.set_collapsed(&d, fold, true).unwrap();
    let update = d.append("[b]: /u\n").unwrap();
    assert!(
        update
            .changes
            .iter()
            .any(|c| c.kind == ChangeKind::Visibility && c.source.start == 0)
    );
    assert_eq!(d.link_info(LinkId(0)), Some(&info));
    apply_layout_update_with_policy(&d, &update, &mut heights, &mut checkpoints, policy).unwrap();
    assert_eq!(
        heights.measure(ticket, 4),
        Err(LayoutError::StaleMeasurement)
    );
    assert_eq!(
        key.validate(&d, &mut Budget::new(WorkLimit::unlimited())),
        Ok(CacheValidity::Changed)
    );
    let mut b = Budget::new(frame(6));
    assert_eq!(stale.next(&d, &mut b), Err(LayoutError::StaleContinuation));
    assert_eq!(
        search.advance(&d, &mut b, &mut Vec::new()),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(
        excerpt.next(&d, &mut b),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(b.used(), WorkLimit::default());
    raw.advance(&d, &mut Budget::new(WorkLimit::unlimited()), &mut hits)
        .unwrap();
    assert_eq!(hits, [Span::new(0, 6)]);
    view.apply(&d, &update).unwrap();
    assert_eq!(view.fold_candidate(&d, fold).unwrap().unwrap(), oldfold);
    assert_eq!(d.text(oldfold.summary.unwrap()), Some("[a][b]"));
    let mut pass = LayoutPass::folded(&d, &view, TileId(0), 100).unwrap();
    let mut sink = Sink::default();
    let mut done = false;
    for _ in 0..1000 {
        let l = frame(6);
        let mut b = Budget::new(l);
        done = pass
            .advance_folded(&d, &mut view, &mut b, &mut sink)
            .unwrap()
            == Completion::EndOfDocument;
        bound(&b, l);
        if done {
            break;
        }
    }
    assert!(done);
    assert_eq!(sink.placeholders, [oldfold.hidden]);
    let duplicate = d.append("[b]: /duplicate\n").unwrap();
    assert!(duplicate.references_changed.is_empty());
    assert!(
        !duplicate
            .changes
            .iter()
            .any(|c| c.kind == ChangeKind::Visibility && c.source.start == 0)
    );

    for resolved in [false, true] {
        let mut d = Document::new();
        d.append("# ![a][b]\n\nx | [a][b]\n-|- \n").unwrap();
        if resolved {
            d.append("\n[b]: /u\n").unwrap();
        }
        let mut search = HeadingSearch::new(&d, "a", 6).unwrap();
        let mut hits = Vec::new();
        let mut excerpt = DisplayExcerpt::new(
            &d,
            d.headings().get(HeadingId(0)).unwrap().content,
            ExcerptLimits {
                source_bytes: 100,
                display_bytes: 100,
                lines: 5,
            },
        )
        .unwrap();
        let mut preview = TablePreview::new(
            &d,
            TableId(0),
            TablePreviewLimits {
                rows: 10,
                source_bytes: 100,
                display_bytes: 100,
            },
        )
        .unwrap();
        let mut anchors = HeadingAnchors::new(&d);
        let mut done = [false; 4];
        let (mut text, mut table) = (String::new(), String::new());
        for l in [
            frame(0),
            frame(1),
            WorkLimit {
                queries: 0,
                ..frame(6)
            },
            WorkLimit {
                objects: 0,
                ..frame(6)
            },
        ] {
            let mut b = Budget::new(l);
            assert_eq!(
                search.advance(&d, &mut b, &mut hits),
                Ok(Completion::BudgetExhausted)
            );
            assert!(hits.is_empty());
            bound(&b, l);
            checks += 1;
        }
        for _ in 0..1000 {
            let l = frame(6);
            let mut b = Budget::new(l);
            done[0] = search.advance(&d, &mut b, &mut hits).unwrap() == Completion::EndOfDocument;
            bound(&b, l);
            let mut b = Budget::new(l);
            match excerpt.next(&d, &mut b).unwrap() {
                ExcerptStep::Run(r) => text.push_str(r.text),
                ExcerptStep::Complete(_) => done[1] = true,
                ExcerptStep::BudgetExhausted => {}
            }
            bound(&b, l);
            let mut b = Budget::new(l);
            match preview.next(&d, &mut b).unwrap() {
                TablePreviewStep::Run { run, .. } => table.push_str(run.text),
                TablePreviewStep::Complete(_) => done[2] = true,
                TablePreviewStep::BudgetExhausted => {}
            }
            bound(&b, l);
            let mut b = Budget::new(l);
            done[3] = anchors.advance(&d, &mut b).unwrap() == Completion::EndOfDocument;
            bound(&b, l);
            if done.iter().all(|x| *x) {
                break;
            }
        }
        assert!(done.iter().all(|x| *x));
        assert_eq!(hits.len(), 1);
        assert_eq!(text, if resolved { "a" } else { "![a][b]" });
        assert_eq!(table, if resolved { "xa\n" } else { "x[a][b]\n" });
        assert_eq!(
            anchors
                .get(&d, HeadingId(0))
                .unwrap()
                .unwrap()
                .slug
                .to_string(),
            if resolved { "a" } else { "ab" }
        );
    }

    println!(
        "current-core consumer contracts PASS checks={checks} shared_display_url_title_frames={frames}; scratch/raw layout/objects/display/metadata/source search/dependencies/cache/stale policy-false/Fold/source summary/duplicate"
    );
}
