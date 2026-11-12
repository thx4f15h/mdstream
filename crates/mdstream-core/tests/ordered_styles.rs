use mdstream::*;

fn parse(source: &str) -> Document {
    let mut document = Document::new();
    document.append(source).unwrap();
    document.finish();
    document
}
fn visible(document: &Document) -> String {
    document
        .display_runs_in(Span::new(0, document.source().len()))
        .unwrap()
        .filter(|run| !run.style.contains(Style::HIDDEN))
        .map(|run| run.text)
        .collect()
}
fn path(document: &Document, at: usize, resolver: &mut LinkResolver) -> Vec<StylePairId> {
    let mut task = document.style_path(at).unwrap();
    let before = resolver.style_work();
    assert!(matches!(
        task.advance(document, resolver, &mut Budget::new(WorkLimit::default()))
            .unwrap(),
        StylePathStep::BudgetExhausted
    ));
    assert_eq!(resolver.style_work(), before);
    let mut pairs = Vec::new();
    for _ in 0..100_000 {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 6,
            ..WorkLimit::default()
        });
        match task.advance(document, resolver, &mut budget).unwrap() {
            StylePathStep::Pair(pair) => pairs.push(pair),
            StylePathStep::BudgetExhausted => (),
            StylePathStep::Complete => return pairs,
        }
        assert!(budget.used().queries <= 1 && budget.used().objects <= 1);
    }
    panic!("style path failed to progress");
}

#[test]
fn actual_marker_identity_preserves_same_point_order_and_adjacent_pairs() {
    for (source, needle, expected) in [
        (
            "***x***\n",
            "x",
            vec![InlineStyleKind::Emphasis, InlineStyleKind::Strong],
        ),
        (
            "~~***x***~~\n",
            "x",
            vec![
                InlineStyleKind::Strike,
                InlineStyleKind::Emphasis,
                InlineStyleKind::Strong,
            ],
        ),
        (
            "***~~x~~***\n",
            "x",
            vec![
                InlineStyleKind::Emphasis,
                InlineStyleKind::Strong,
                InlineStyleKind::Strike,
            ],
        ),
        (
            "~~**x**~~\n",
            "x",
            vec![InlineStyleKind::Strike, InlineStyleKind::Strong],
        ),
        (
            "**~~x~~**\n",
            "x",
            vec![InlineStyleKind::Strong, InlineStyleKind::Strike],
        ),
        (
            "*foo [*bar*](/url)*\n",
            "bar",
            vec![InlineStyleKind::Emphasis, InlineStyleKind::Emphasis],
        ),
    ] {
        let doc = parse(source);
        let mut resolver = doc.link_resolver();
        let pairs = path(&doc, source.find(needle).unwrap(), &mut resolver);
        assert_eq!(pairs.iter().map(|p| p.kind).collect::<Vec<_>>(), expected);
        assert!(pairs.windows(2).all(|p| p[0].open.start < p[1].open.start));
        if source == "***x***\n" {
            assert_eq!(
                (pairs[0].open, pairs[0].close),
                (Span::new(0, 1), Span::new(6, 7))
            );
            assert_eq!(
                (pairs[1].open, pairs[1].close),
                (Span::new(1, 3), Span::new(4, 6))
            );
        }

        assert_eq!(
            path(&doc, source.find(needle).unwrap(), &mut resolver),
            pairs
        );
        assert_eq!(
            resolver.style_work().events,
            0,
            "unconditional path must not recover"
        );
    }
    let doc = parse("*a*_b_\n");
    let runs: Vec<_> = doc
        .display_runs_in(Span::new(0, doc.source().len()))
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .collect();
    assert_eq!(runs[0].text, "a");
    assert_eq!(runs[1].text, "b");
    assert_ne!(runs[0].style_pair, runs[1].style_pair);
}

#[test]
fn unresolved_and_suppressed_labels_are_transparent_active_labels_isolate() {
    for (source, expected) in [
        ("*foo [bar* baz]\n", "foo [bar baz]\n"),
        ("*foo [bar* baz]\n\n[bar* baz]: /u\n", "*foo bar* baz\n\n"),
        (
            "*foo [bar* [kid] baz][outer]\n\n[outer]: /u\n",
            "*foo bar* [kid] baz\n\n",
        ),
        (
            "*foo [bar* [kid] baz][outer]\n\n[outer]: /u\n[kid]: /v\n",
            "foo [bar kid baz]outer\n\n",
        ),
    ] {
        let doc = parse(source);
        assert_eq!(visible(&doc), expected);
        let mut resolver = doc.link_resolver();
        let first = path(&doc, 2, &mut resolver);
        let visited = resolver.style_work().events;
        assert!(visited > 0);
        for _ in 0..20 {
            assert_eq!(path(&doc, 2, &mut resolver), first);
        }
        assert_eq!(
            resolver.style_work().events,
            visited,
            "same scope replayed per glyph"
        );
        let mut cursor = doc.display_cursor(Span::new(0, source.len())).unwrap();
        let mut text = String::new();
        for _ in 0..100_000 {
            let mut budget = Budget::new(WorkLimit {
                queries: 1,
                objects: 1,
                bytes: 6,
                ..WorkLimit::default()
            });
            match cursor
                .next_with_resolver(&doc, &mut resolver, &mut budget)
                .unwrap()
            {
                DisplayStep::Run(run) if !run.style.contains(Style::HIDDEN) => {
                    text.push_str(run.text)
                }
                DisplayStep::Run(_) | DisplayStep::BudgetExhausted => (),
                DisplayStep::Complete => break,
            }
        }
        assert_eq!(text, expected);
        assert_eq!(resolver.style_work().events, visited);
        assert_eq!(
            resolver.retire(&mut Budget::new(WorkLimit::default())),
            Completion::BudgetExhausted
        );
        while resolver.retire(&mut Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            ..WorkLimit::default()
        })) != Completion::Covered
        {}
        assert_eq!(resolver.memo_entries(), 0);
        resolver.reset(&doc).unwrap();
    }
}

#[test]
fn sparse_conditional_query_does_not_enter_historical_scopes() {
    let source = format!("{}*foo [bar* baz]\n", "*old*\n\n".repeat(5000));
    let doc = parse(&source);
    let mut resolver = doc.link_resolver();
    let pairs = path(&doc, source.len() - 10, &mut resolver);
    assert_eq!(pairs.len(), 1);
    assert_eq!(resolver.style_work().events, 4);
    assert!(doc.work().style_events < source.len() as u64);
}

#[test]
fn full_suffix_chain_scope_matrix_uses_current_interpretation_and_preclassified_runs() {
    for mask in 0usize..16 {
        for r in [false, true] {
            for wrapper in [false, true] {
                for marker in ["*", "**", "~~"] {
                    let body = format!("[a{marker} [b][r]][*x*][**y**][~z~][_w_]");
                    let mut source = if wrapper {
                        format!("{marker}pre [ancestor {body}](u) tail\n\n")
                    } else {
                        format!("{marker}pre {body} tail\n\n")
                    };
                    let yes = [mask & 1 != 0, mask & 2 != 0, mask & 4 != 0, mask & 8 != 0];
                    for (i, key) in ["*x*", "**y**", "~z~", "_w_"].iter().enumerate() {
                        if yes[i] {
                            source.push_str(&format!("[{key}]: /{}\n", ["x", "y", "z", "w"][i]));
                        }
                    }
                    if r {
                        source.push_str("[r]: /r\n");
                    }
                    // Independent retained-grammar pairing from the original
                    // 003 matrix: !r -> outer/x, y/z, w; r -> b/r, x/y, z/w.
                    let first_active = !r && yes[0];
                    let any_child = if r { true } else { yes[0] || yes[2] || yes[3] };
                    let wrapper_active = wrapper && !any_child;
                    let doc = parse(&source);
                    let mut resolver = doc.link_resolver();
                    let prefix = path(&doc, source.find("pre").unwrap(), &mut resolver);
                    assert_eq!(
                        prefix.len(),
                        usize::from(!first_active && !wrapper_active),
                        "{source:?}"
                    );
                    let hidden = [first_active, r && yes[1], !r && yes[2], r && yes[3]];
                    for (i, (needle, kind)) in [
                        ("*x*", InlineStyleKind::Emphasis),
                        ("**y**", InlineStyleKind::Strong),
                        ("~z~", InlineStyleKind::Strike),
                        ("_w_", InlineStyleKind::Emphasis),
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        let payload = source.find(needle).unwrap() + if i == 1 { 2 } else { 1 };
                        let run = doc
                            .runs_in(Span::new(payload, payload + 1))
                            .unwrap()
                            .next()
                            .unwrap();
                        assert_eq!(
                            run.style.contains(Style::HIDDEN),
                            hidden[i],
                            "{needle} {source:?}"
                        );
                        if !hidden[i] {
                            let pairs = path(&doc, payload, &mut resolver);
                            assert_eq!(pairs.len(), 1, "{needle} {source:?}");
                            assert_eq!(pairs[0].kind, kind);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn far_prefix_dependencies_visibility_tickets_and_real_layout_invalidate_together() {
    let mut doc = Document::new();
    doc.append(&format!("*{} [x* z]\n\n", "a".repeat(20_000)))
        .unwrap();
    let before = doc.runs_in(Span::new(0, 2)).unwrap().next().unwrap();
    assert!(before.style.contains(Style::HIDDEN));
    let mut capture = DependencyCapture::new(&doc, TileId(0)).unwrap();
    let mut key = None;
    for _ in 0..100_000 {
        let mut b = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 6,
            dependencies: 1,
            ..WorkLimit::default()
        });
        match capture.advance(&doc, &mut b).unwrap() {
            DependencyCaptureStep::Captured(k) => {
                key = Some(k);
                break;
            }
            DependencyCaptureStep::BudgetExhausted => (),
            DependencyCaptureStep::Complete => panic!("capture missing"),
        }
        assert!(b.used().queries <= 1 && b.used().objects <= 1 && b.used().bytes <= 6);
    }
    let key = key.expect("remote scope recovery completes");
    assert_eq!(
        key.validate(&doc, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap(),
        CacheValidity::Valid
    );
    let mut heights = HeightIndex::default();
    heights.rebuild(&doc).unwrap();
    let ticket = heights.ticket(TileId(0)).unwrap();
    let mut pass = LayoutPass::new(&doc, &heights, TileId(0), 1, false).unwrap();
    #[derive(Default)]
    struct Sink {
        runs: usize,
    }
    impl LayoutSink for Sink {
        fn begin_tile(&mut self, _: TileId) {}
        fn text(&mut self, r: RenderRun<'_>) {
            if !r.style.contains(Style::HIDDEN) {
                self.runs += 1;
            }
        }
        fn object(&mut self, _: LinkId, _: &LinkInfo) {}
        fn end_tile(&mut self, _: TileId) -> u64 {
            1
        }
    }
    let mut sink = Sink::default();
    for _ in 0..100_000 {
        let mut b = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 6,
            tiles: 1,
            dependencies: 1,
            recovery: 1,
        });
        if pass.advance(&doc, &mut heights, &mut b, &mut sink).unwrap()
            != Completion::BudgetExhausted
        {
            break;
        }
    }
    assert!(sink.runs > 0);
    let update = doc.append("[x* z]: /u\n").unwrap();
    assert!(update.dirty.iter().any(|s| s.start == 0));
    assert_eq!(
        key.validate(&doc, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap(),
        CacheValidity::Changed
    );
    heights
        .apply_with_policy(
            &doc,
            &update,
            LayoutPolicy {
                style_affects_size: false,
                ..LayoutPolicy::default()
            },
        )
        .unwrap();
    assert_eq!(
        heights.measure(ticket, 1),
        Err(LayoutError::StaleMeasurement)
    );
    assert!(matches!(
        pass.advance(
            &doc,
            &mut heights,
            &mut Budget::new(WorkLimit::unlimited()),
            &mut sink
        ),
        Err(LayoutError::StaleContinuation)
    ));
    let after = doc.runs_in(Span::new(0, 1)).unwrap().next().unwrap();
    assert!(!after.style.contains(Style::HIDDEN));
}

#[test]
fn deep_current_assignment_recovery_and_path_cancellation_pay_for_actual_entries() {
    let marker = "*".repeat(4097);
    let source = format!("{marker}中🙂{marker} [r]\n\n");
    let mut d = Document::new();
    for c in source.chars() {
        d.append(c.encode_utf8(&mut [0; 4])).unwrap();
    }
    assert!(d.work().style_events <= 8);
    assert!(d.work().style_index_steps < 4097 * 6);
    let at = 4097;
    let mut resolver = d.link_resolver();
    let mut task = d.style_path(at).unwrap();
    for _ in 0..100_000 {
        let mut b = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 6,
            ..WorkLimit::default()
        });
        task.advance(&d, &mut resolver, &mut b).unwrap();
        if resolver.memo_entries() > 128 {
            break;
        }
    }
    let allocated = resolver.memo_entries();
    assert!(allocated > 128);
    assert_eq!(
        task.retire(&mut Budget::new(WorkLimit::default())),
        Completion::Covered
    );
    let mut paid = 0;
    loop {
        let mut b = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            ..WorkLimit::default()
        });
        let done = resolver.retire(&mut b);
        paid += b.used().objects;
        if done == Completion::Covered {
            break;
        }
    }
    assert_eq!(paid, allocated);
    resolver.reset(&d).unwrap();
    let pairs = path(&d, at, &mut resolver);
    assert_eq!(pairs.len(), 2049);
    assert!(resolver.memo_entries() < 12 * 4097);
    assert_eq!(pairs[0].kind, InlineStyleKind::Emphasis);
    let mut task = d.style_path(at).unwrap();
    // Materialize a deep parent path, then cancel before output. Each retained
    // path element is separately retired; the semantic memo is shared.
    for _ in 0..1500 {
        task.advance(
            &d,
            &mut resolver,
            &mut Budget::new(WorkLimit {
                queries: 1,
                objects: 1,
                ..WorkLimit::default()
            }),
        )
        .unwrap();
    }
    assert_eq!(
        task.retire(&mut Budget::new(WorkLimit::default())),
        Completion::BudgetExhausted
    );
    for _ in 0..4097 {
        if task.retire(&mut Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            ..WorkLimit::default()
        })) == Completion::Covered
        {
            break;
        }
    }
    d.append("[r]: /u\n").unwrap();
    assert!(matches!(
        task.advance(&d, &mut resolver, &mut Budget::new(WorkLimit::unlimited())),
        Err(LayoutError::StaleContinuation)
    ));
}

#[test]
fn unconditional_windows_walk_ordered_boundaries_once_and_skip_historical_pairs() {
    let d = parse(&format!(
        "{}{}",
        "*old*\n\n".repeat(5000),
        "*a*_b_ ~~c~~ **d** ".repeat(4096)
    ));
    let start = d.source().find("*a*").unwrap();
    let mut runs = d.runs_in(Span::new(start, d.source().len())).unwrap();
    let mut count = 0;
    while runs.next().is_some() {
        count += 1;
    }
    assert!(count > 4 * 4096);
    assert!(runs.ordered_style_steps() < 4 * count + 12);
    let mut window = d.runs_in(Span::new(start, start + 6)).unwrap();
    while window.next().is_some() {}
    assert!(window.ordered_style_steps() < 32);
}

#[test]
fn display_search_and_real_folded_layout_consume_the_same_conditional_styles() {
    for defined in [false, true] {
        let source = if defined {
            "*foo [bar* baz]\n\n[bar* baz]: /u\n"
        } else {
            "*foo [bar* baz]\n\n"
        };
        let d = parse(source);
        let query = if defined {
            "*foo bar* baz"
        } else {
            "foo [bar baz]"
        };
        let limit = WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 6,
            tiles: 1,
            recovery: 1,
            dependencies: 1,
        };
        let mut search = DisplaySearch::new(&d, query).unwrap();
        let mut hits = Vec::new();
        assert_eq!(
            search
                .advance(&d, &mut Budget::new(WorkLimit::default()), &mut hits)
                .unwrap(),
            Completion::BudgetExhausted
        );
        for _ in 0..20_000 {
            let mut b = Budget::new(limit);
            if search.advance(&d, &mut b, &mut hits).unwrap() == Completion::EndOfDocument {
                break;
            }
            assert!(b.used().bytes <= 6 && b.used().objects <= 1 && b.used().queries <= 1);
        }
        assert_eq!(hits.len(), 1);
        let resolver = search.take_link_resolver();
        assert_eq!(resolver.style_work().events, 4);
        let mut view = FoldView::new(&d, 1).unwrap();
        let mut registration =
            SourceFoldRegistration::overlapping(&d, &view, Span::new(1, 4)).unwrap();
        let fold = loop {
            if let Some(id) = registration
                .advance(&d, &mut view, &mut Budget::new(limit))
                .unwrap()
                .id
            {
                break id;
            }
        };
        view.set_collapsed(&d, fold, true).unwrap();
        let mut pass = LayoutPass::folded(&d, &view, TileId(0), u64::MAX).unwrap();
        #[derive(Default)]
        struct Sink(String);
        impl LayoutSink for Sink {
            fn begin_tile(&mut self, _: TileId) {}
            fn text(&mut self, r: RenderRun<'_>) {
                if !r.style.contains(Style::HIDDEN) {
                    self.0.push_str(r.text);
                }
            }
            fn object(&mut self, _: LinkId, _: &LinkInfo) {}
            fn end_tile(&mut self, _: TileId) -> u64 {
                1
            }
        }
        impl FoldLayoutSink for Sink {
            fn placeholder(&mut self, _: FoldPlaceholder) {
                self.0.push('◇');
            }
        }
        let mut sink = Sink::default();
        for _ in 0..20_000 {
            let mut b = Budget::new(limit);
            if pass
                .advance_folded(&d, &mut view, &mut b, &mut sink)
                .unwrap()
                == Completion::EndOfDocument
            {
                break;
            }
            assert!(b.used().bytes <= 6 && b.used().objects <= 1 && b.used().queries <= 1);
        }
        assert!(sink.0.contains('◇'));
        assert!(!sink.0.contains("foo"));
        assert_eq!(pass.take_link_resolver().style_work().events, 4);
    }
}

#[test]
fn image_boundaries_follow_image_activity_independently_of_child_links() {
    for image in [false, true] {
        for child in [false, true] {
            let mut source = "**pre ![z** [kid] q][i] tail\n\n".to_owned();
            if image {
                source.push_str("[i]: /img\n");
            }
            if child {
                source.push_str("[kid]: /u\n");
            }
            let d = parse(&source);
            let mut resolver = d.link_resolver();
            let pairs = path(&d, source.find("pre").unwrap(), &mut resolver);
            assert_eq!(pairs.len(), usize::from(!image));
            if !image {
                assert_eq!(pairs[0].kind, InlineStyleKind::Strong);
            }
            assert_eq!(
                path(&d, source.find("z**").unwrap(), &mut resolver).len(),
                usize::from(!image)
            );
            assert!(path(&d, source.find("tail").unwrap(), &mut resolver).is_empty());
        }
    }
}
