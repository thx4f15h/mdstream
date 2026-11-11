use mdstream::*;

#[path = "support/shortcut_snapshot.rs"]
mod model;
#[allow(dead_code)]
#[path = "support/reference_boundary_hashes.rs"]
mod reference_hashes;
include!("fixtures/shortcut_baseline.rs");

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn shown(doc: &Document) -> String {
    doc.display_runs_in(Span::new(0, doc.source().len()))
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect()
}

#[test]
fn unresolved_shortcut_keeps_only_its_original_markers_visible() {
    for (source, expected) in [
        ("[a]", "[a]"),
        ("![a]", "![a]"),
        ("[*a*]", "[a]"),
        ("[a\\]]", "[a]]"),
        ("[a](", "[a]("),
        ("[a](not a link)", "[a](not a link)"),
        ("![a](u '&amp;'&amp;)", "![a](u '&'&)"),
        ("> ![a]\n", "![a]\n"),
        ("# ![a]\n", "![a]\n"),
        ("[中🙂]", "[中🙂]"),
    ] {
        let doc = parse(source);
        assert_eq!(shown(&doc), expected, "{source:?}");
        assert_eq!(doc.links(), [LinkTarget::Reference(SymbolId(0))]);
        let info = doc.link_info(LinkId(0)).unwrap();
        assert_eq!(doc.references()[0].first_use, Some(info.source.start));
        assert_eq!(doc.link_destination(LinkId(0)), None);
        for span in [
            Span::new(info.source.start, info.label.start),
            Span::new(info.label.end, info.source.end),
        ] {
            for r in doc.display_runs_in(span).unwrap() {
                assert_eq!(doc.text(r.source), Some(r.text));
                assert_eq!(r.replacement, None);
                assert_eq!(r.link, None);
                assert_eq!(r.style, Style::default());
                assert_eq!((r.emphasis_depth, r.strong_depth), (0, 0));
            }
        }
    }
    let doc = parse("[a]");
    let runs: Vec<_> = doc.runs_in(Span::new(0, 3)).unwrap().collect();
    assert_eq!(runs.len(), 3);
    for (r, (span, text, link)) in runs.iter().zip([
        (Span::new(0, 1), "[", None),
        (Span::new(1, 2), "a", Some(LinkId(0))),
        (Span::new(2, 3), "]", None),
    ]) {
        assert_eq!(
            (r.source, r.text, r.link, r.style),
            (span, text, link, Style::default())
        );
    }
}

#[test]
fn late_winner_changes_visibility_without_replacing_identity() {
    let mut doc = Document::new();
    doc.append("[a] ![a]\n\n").unwrap();
    let infos: Vec<_> = (0..2).map(|i| *doc.link_info(LinkId(i)).unwrap()).collect();
    assert_eq!(shown(&doc), "[a] ![a]\n\n");
    let update = doc.append("[a]: /u\n").unwrap();
    assert_eq!(shown(&doc), "a a\n\n");
    assert!(
        update
            .changes
            .iter()
            .any(|c| c.kind == ChangeKind::Visibility
                && c.source == Span::new(0, doc.source().len()))
    );
    for (i, info) in infos.iter().enumerate() {
        assert_eq!(doc.link_info(LinkId(i)), Some(info));
        assert_eq!(doc.links()[i], LinkTarget::Reference(SymbolId(0)));
        assert_eq!(doc.link_destination(LinkId(i)), Some("/u"));
    }
    assert_eq!(doc.references()[0].revision, 1);
    let duplicate = doc.append("[a]: /other\n").unwrap();
    assert!(duplicate.references_changed.is_empty());
    assert!(
        !duplicate
            .changes
            .iter()
            .any(|c| c.kind == ChangeKind::Visibility && c.source.start == 0)
    );
}

#[test]
fn predecessor_structures_payloads_and_scalar_semantics_are_frozen() {
    for (index, &(source, markers, expected)) in BASELINE.iter().enumerate() {
        assert_eq!(
            model::describe(&parse(source), markers, true)
                .bytes()
                .fold(0xcbf29ce484222325u64, |h, b| (h ^ u64::from(b))
                    .wrapping_mul(0x100000001b3)),
            reference_hashes::shortcut(index, source, expected),
            "{source:?}"
        );
    }
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

#[test]
fn every_utf8_cut_prefix_eof_scratch_and_complete_suffix_keeps_the_same_id() {
    for source in [
        "![中🙂]",
        "> [中\r\n> 🙂]",
        "[a][b]",
        "[a][]",
        "![a](u 't')",
        "[a](u '&amp;'&amp;)",
        "[a\nb] | c\n-|-\n",
    ] {
        let expected = model::describe(&parse(source), &[], true);
        for cut in 0..=source.len() {
            let mut doc = Utf8Document::new();
            doc.append(&source.as_bytes()[..cut]).unwrap();
            doc.append(b"").unwrap();
            doc.append(&source.as_bytes()[cut..]).unwrap();
            doc.finish();
            assert_eq!(model::describe(doc.document(), &[], true), expected);
            let mut prefix = Utf8Document::new();
            for byte in source.as_bytes()[..cut].chunks(1) {
                prefix.append(byte).unwrap();
            }
            prefix.finish();
            let mut once = Utf8Document::new();
            once.append(&source.as_bytes()[..cut]).unwrap();
            once.finish();
            assert_eq!(
                model::describe(prefix.document(), &[], true),
                model::describe(once.document(), &[], true)
            );
        }
    }
    for suffix in ["(u)", "[b]", "[]"] {
        let mut doc = Document::new();
        doc.append(&format!("{}![a]", "x".repeat(BLOCK_LOOKAHEAD + 1)))
            .unwrap();
        assert_eq!(doc.links(), [LinkTarget::Reference(SymbolId(0))]);
        let label = doc.link_info(LinkId(0)).unwrap().label;
        assert!(shown(&doc).ends_with("![a]"));
        doc.append(suffix).unwrap();
        assert_eq!(doc.links().len(), 1);
        assert_eq!(doc.link_info(LinkId(0)).unwrap().label, label);
        assert!(shown(&doc).ends_with(if suffix == "(u)" {
            "a"
        } else if suffix == "[b]" {
            "![a][b]"
        } else {
            "![a][]"
        }));
        let span = Span::new(0, doc.source().len());
        let mut scratch = QueryScratch::default();
        assert_eq!(
            doc.runs_in_with_scratch(span, &mut scratch)
                .unwrap()
                .collect::<Vec<_>>(),
            doc.runs_in(span).unwrap().collect::<Vec<_>>()
        );
        assert_eq!(
            doc.display_runs_in_with_scratch(span, &mut scratch)
                .unwrap()
                .collect::<Vec<_>>(),
            doc.display_runs_in(span).unwrap().collect::<Vec<_>>()
        );
    }
    for n in [998, 999, 1000] {
        let source = format!("[{}]", "a".repeat(n));
        let doc = parse(&source);
        assert_eq!(doc.links().len(), usize::from(n <= 999));
        assert_eq!(shown(&doc), source);
    }
    for source in ["[]", "![]", "[ \t]", "[a", "`[a]`", "$[a]$", "[^a]"] {
        assert!(parse(source).links().is_empty(), "{source:?}");
    }
}

#[derive(Default)]
struct Sink {
    text: String,
    objects: Vec<LinkId>,
    placeholders: Vec<Span>,
}

impl FoldLayoutSink for Sink {
    fn placeholder(&mut self, p: FoldPlaceholder) {
        self.placeholders.push(p.candidate.hidden);
    }
}
impl LayoutSink for Sink {
    fn begin_tile(&mut self, _: TileId) {}
    fn text(&mut self, r: RenderRun<'_>) {
        if !r.style.contains(Style::HIDDEN) {
            self.text.push_str(r.text);
        }
    }
    fn object(&mut self, id: LinkId, _: &LinkInfo) {
        self.objects.push(id);
    }
    fn end_tile(&mut self, _: TileId) -> u64 {
        1
    }
}

#[test]
fn raw_layout_display_search_source_and_metadata_share_bounded_work() {
    for source in ["![a]", "[中🙂]", "[*a*]", "> [a\r\n> b]\n"] {
        let doc = parse(source);
        let span = Span::new(0, source.len());
        for limit in [
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
            let mut cursor = doc.display_cursor(span).unwrap();
            let mut budget = Budget::new(limit);
            assert_eq!(
                cursor.next(&doc, &mut budget),
                Ok(DisplayStep::BudgetExhausted)
            );
            assert_eq!(cursor.source_position(), 0);
            assert!(budget.used().bytes <= limit.bytes);
        }
        let mut cursor = doc.display_cursor(span).unwrap();
        let mut output = String::new();
        for _ in 0..source.len() * 4 + 10 {
            let mut budget = Budget::new(frame(6));
            match cursor.next(&doc, &mut budget).unwrap() {
                DisplayStep::Run(r) => {
                    if !r.style.contains(Style::HIDDEN) {
                        output.push_str(r.text);
                    }
                }
                DisplayStep::Complete => break,
                DisplayStep::BudgetExhausted => {}
            }
            assert!(budget.used().bytes <= 6);
            assert!(budget.used().queries <= 1 && budget.used().objects <= 1);
        }
        assert_eq!(output, shown(&doc));
        let mut heights = HeightIndex::new(1).unwrap();
        heights.rebuild(&doc).unwrap();
        let mut pass = LayoutPass::new(&doc, &heights, TileId(0), 100, true).unwrap();
        let mut sink = Sink::default();
        for _ in 0..source.len() * 10 + 30 {
            let mut budget = Budget::new(frame(6));
            let end = pass
                .advance(&doc, &mut heights, &mut budget, &mut sink)
                .unwrap();
            assert!(
                budget.used().bytes <= 6
                    && budget.used().objects <= 1
                    && budget.used().queries <= 1
            );
            if end == Completion::EndOfDocument {
                break;
            }
        }
        let expected: String = doc
            .runs_in(span)
            .unwrap()
            .filter(|r| !r.style.contains(Style::HIDDEN))
            .map(|r| r.text)
            .collect();
        assert_eq!(sink.text, expected);
        assert_eq!(sink.objects, [LinkId(0)]);
        assert_eq!(doc.link_destination(LinkId(0)), None);
        assert!(doc.decoded_link_destination(LinkId(0)).is_none());
        for (at, _) in source.char_indices() {
            let expected = doc
                .runs_in(Span::new(at, source.len()))
                .unwrap()
                .find(|r| !r.style.contains(Style::HIDDEN))
                .map(|r| r.source.start);
            assert_eq!(doc.next_visible(at), expected);
        }
        let mut search = DisplaySearch::new(&doc, "[").unwrap();
        let mut matches = Vec::new();
        for _ in 0..source.len() * 10 + 30 {
            let mut budget = Budget::new(frame(6));
            if search.advance(&doc, &mut budget, &mut matches).unwrap() == Completion::EndOfDocument
            {
                break;
            }
            assert!(
                budget.used().bytes <= 6
                    && budget.used().objects <= 1
                    && budget.used().queries <= 1
            );
        }
        assert_eq!(matches.len(), 1);
    }
    let mut doc = Document::new();
    doc.append("[a]\n\n").unwrap();
    let mut search = SourceSearch::new(&doc, "[a]").unwrap();
    let mut matches = Vec::new();
    search
        .advance(&doc, &mut Budget::new(WorkLimit::unlimited()), &mut matches)
        .unwrap();
    assert_eq!(matches, [Span::new(0, 3)]);
    doc.append("[a]: /u\n").unwrap();
    search
        .advance(&doc, &mut Budget::new(WorkLimit::unlimited()), &mut matches)
        .unwrap();
    assert_eq!(matches, [Span::new(0, 3), Span::new(5, 8)]);
}

#[test]
fn winner_visibility_invalidates_policy_false_tickets_keys_and_tasks() {
    let mut doc = Document::new();
    let first = doc
        .append(&format!(
            "{}[a]\n\n{}\n\n",
            "x".repeat(TILE_BYTES - 1),
            "q".repeat(3 * TILE_BYTES)
        ))
        .unwrap();
    let mut heights = HeightIndex::new(1).unwrap();
    let mut checkpoints = CheckpointIndex::<usize>::new();
    let policy = LayoutPolicy {
        style_affects_size: false,
        references_affect_size: false,
        table_width_affects_size: false,
    };
    apply_layout_update_with_policy(&doc, &first, &mut heights, &mut checkpoints, policy).unwrap();
    let ticket = heights.ticket(TileId(0)).unwrap();
    heights.measure(ticket, 7).unwrap();
    checkpoints.insert(TileId(1), 42).unwrap();
    // Tile zero contains only the opener, but links_in must still capture a.
    let key = DependencyKey::capture(&doc, TileId(0), &mut Budget::new(WorkLimit::unlimited()))
        .unwrap()
        .unwrap();
    let old_info = *doc.link_info(LinkId(0)).unwrap();
    let mut cursor = doc.display_cursor(old_info.source).unwrap();
    let mut search = DisplaySearch::new(&doc, "[").unwrap();
    let mut excerpt = DisplayExcerpt::new(
        &doc,
        old_info.source,
        ExcerptLimits {
            source_bytes: 100,
            display_bytes: 100,
            lines: 10,
        },
    )
    .unwrap();
    let update = doc.append("[a]: /u\n").unwrap();
    let start = TILE_BYTES - 1;
    assert_eq!(
        update
            .changes
            .iter()
            .filter(|c| c.kind == ChangeKind::Visibility && c.source.start == start)
            .count(),
        1
    );
    apply_layout_update_with_policy(&doc, &update, &mut heights, &mut checkpoints, policy).unwrap();
    assert_eq!(
        heights.measure(ticket, 8),
        Err(LayoutError::StaleMeasurement)
    );
    assert_eq!(checkpoints.get(TileId(1)), None);
    assert_eq!(
        key.validate(&doc, &mut Budget::new(WorkLimit::unlimited())),
        Ok(CacheValidity::Changed)
    );
    let mut budget = Budget::new(frame(6));
    assert_eq!(
        cursor.next(&doc, &mut budget),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(
        search.advance(&doc, &mut budget, &mut Vec::new()),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(
        excerpt.next(&doc, &mut budget),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(budget.used(), WorkLimit::default());
    assert_eq!(doc.link_info(LinkId(0)), Some(&old_info));
    let mut batch = first;
    batch.merge(update.clone()).unwrap();
    let mut delayed = HeightIndex::new(1).unwrap();
    delayed.apply_with_policy(&doc, &batch, policy).unwrap();
    assert_eq!(delayed.len(), heights.len());
    assert_eq!(
        batch
            .changes
            .iter()
            .filter(|c| c.kind == ChangeKind::Visibility
                && c.source == Span::new(start, doc.source().len()))
            .count(),
        1
    );
    heights.apply_with_policy(&doc, &update, policy).unwrap(); // replay is inert
    let title = doc.append(" 'title'\n").unwrap();
    assert_eq!(doc.references()[0].revision, 2);
    assert!(
        !title
            .changes
            .iter()
            .any(|c| c.kind == ChangeKind::Visibility && c.source.start == start)
    );
}

#[test]
fn append_and_late_definition_cost_do_not_grow_with_old_uses() {
    for history in [0, 20_000] {
        let mut doc = Document::new();
        doc.append(&"[a] ".repeat(history)).unwrap();
        doc.append("\n\n").unwrap();
        doc.append(&"x".repeat(1_000_000)).unwrap();
        let before = doc.work();
        for _ in 0..1000 {
            doc.append("x").unwrap();
        }
        assert_eq!(doc.work().inline_bytes - before.inline_bytes, 1000);
        assert_eq!(doc.work().tree_visits, before.tree_visits);
        assert_eq!(doc.work().label_bytes, before.label_bytes);
        doc.append("\n\n").unwrap();
        let before = doc.work();
        let update = doc.append("[a]: /u\n").unwrap();
        assert_eq!(doc.work().label_bytes - before.label_bytes, 1);
        assert_eq!(doc.work().inline_bytes, before.inline_bytes);
        assert!(doc.work().tree_visits - before.tree_visits < 100);
        assert_eq!(doc.links().len(), history);
        assert_eq!(update.references_changed, [SymbolId(0)]);
    }
}

#[test]
fn heading_excerpt_table_and_slug_use_the_same_conditional_styles() {
    let mut doc = Document::new();
    doc.append("# ![a]\n\nx | [a]\n-|-\n").unwrap();
    for resolved in [false, true] {
        if resolved {
            doc.append("\n[a]: /u\n").unwrap();
        }
        let mut search = HeadingSearch::new(&doc, "[a]", 6).unwrap();
        let mut hits = Vec::new();
        let mut excerpt = DisplayExcerpt::new(
            &doc,
            doc.headings().get(HeadingId(0)).unwrap().content,
            ExcerptLimits {
                source_bytes: 100,
                display_bytes: 100,
                lines: 5,
            },
        )
        .unwrap();
        let mut preview = TablePreview::new(
            &doc,
            TableId(0),
            TablePreviewLimits {
                rows: 10,
                source_bytes: 100,
                display_bytes: 100,
            },
        )
        .unwrap();
        let mut anchors = HeadingAnchors::new(&doc);
        for limit in [
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
            let mut budget = Budget::new(limit);
            assert_eq!(
                search.advance(&doc, &mut budget, &mut hits),
                Ok(Completion::BudgetExhausted)
            );
            assert!(hits.is_empty());
            assert!(
                budget.used().bytes <= limit.bytes
                    && budget.used().objects <= limit.objects
                    && budget.used().queries <= limit.queries
            );
        }
        let mut text = String::new();
        let mut table = String::new();
        let mut done = [false; 4];
        for _ in 0..500 {
            // Each consumer gets its own frame here. Within each task, setup,
            // hidden syntax, matching and output share that exact frame.
            let mut budget = Budget::new(frame(6));
            done[0] =
                search.advance(&doc, &mut budget, &mut hits).unwrap() == Completion::EndOfDocument;
            assert!(
                budget.used().bytes <= 6
                    && budget.used().objects <= 1
                    && budget.used().queries <= 1
            );
            let mut budget = Budget::new(frame(6));
            match excerpt.next(&doc, &mut budget).unwrap() {
                ExcerptStep::Run(r) => text.push_str(r.text),
                ExcerptStep::Complete(_) => done[1] = true,
                ExcerptStep::BudgetExhausted => {}
            }
            assert!(
                budget.used().bytes <= 6
                    && budget.used().objects <= 1
                    && budget.used().queries <= 1
            );
            let mut budget = Budget::new(frame(6));
            match preview.next(&doc, &mut budget).unwrap() {
                TablePreviewStep::Run { run, .. } => table.push_str(run.text),
                TablePreviewStep::Complete(_) => done[2] = true,
                TablePreviewStep::BudgetExhausted => {}
            }
            assert!(
                budget.used().bytes <= 6
                    && budget.used().objects <= 1
                    && budget.used().queries <= 1
            );
            let mut budget = Budget::new(frame(6));
            done[3] = anchors.advance(&doc, &mut budget).unwrap() == Completion::EndOfDocument;
            assert!(
                budget.used().bytes <= 6
                    && budget.used().objects <= 1
                    && budget.used().queries <= 1
            );
            if done.iter().all(|&v| v) {
                break;
            }
        }
        assert!(done.iter().all(|&v| v));
        assert_eq!(hits.len(), usize::from(!resolved));
        assert_eq!(text, if resolved { "a" } else { "![a]" });
        assert_eq!(table, if resolved { "xa\n" } else { "x[a]\n" });
        assert_eq!(
            anchors
                .get(&doc, HeadingId(0))
                .unwrap()
                .unwrap()
                .slug
                .to_string(),
            "a"
        );
    }
}

#[test]
fn fixed_source_fold_keeps_topology_and_raw_summary_after_late_resolution() {
    let mut doc = Document::new();
    doc.append("[a] rest\n\n").unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let mut registration = SourceFoldRegistration::paragraph(&doc, &view, BlockId(0), 3).unwrap();
    let id = registration
        .advance(&doc, &mut view, &mut Budget::new(WorkLimit::unlimited()))
        .unwrap()
        .id
        .unwrap();
    let before = view.fold_candidate(&doc, id).unwrap().unwrap();
    assert_eq!(doc.text(before.summary.unwrap()), Some("[a]"));
    view.set_collapsed(&doc, id, true).unwrap();
    let update = doc.append("[a]: /u\n").unwrap();
    view.apply(&doc, &update).unwrap();
    assert_eq!(view.fold_candidate(&doc, id).unwrap().unwrap(), before);
    assert!(view.is_collapsed(id));
    assert_eq!(doc.text(before.summary.unwrap()), Some("[a]"));
    let mut pass = LayoutPass::folded(&doc, &view, TileId(0), 100).unwrap();
    let mut folded_sink = Sink::default();
    for _ in 0..100 {
        let mut budget = Budget::new(frame(6));
        let completion = pass
            .advance_folded(&doc, &mut view, &mut budget, &mut folded_sink)
            .unwrap();
        assert!(
            budget.used().bytes <= 6 && budget.used().queries <= 1 && budget.used().objects <= 1
        );
        if completion == Completion::EndOfDocument {
            break;
        }
    }
    assert_eq!(folded_sink.text, "a\n");
    assert_eq!(folded_sink.placeholders, [before.hidden]);
    view.set_collapsed(&doc, id, false).unwrap();
    let mut sink = Sink::default();
    // Ordinary raw layout uses the same semantic visibility as a folded pass.
    let mut heights = HeightIndex::new(1).unwrap();
    heights.rebuild(&doc).unwrap();
    let mut pass = LayoutPass::new(&doc, &heights, TileId(0), 100, true).unwrap();
    pass.advance(
        &doc,
        &mut heights,
        &mut Budget::new(WorkLimit::unlimited()),
        &mut sink,
    )
    .unwrap();
    assert_eq!(sink.text, "a rest\n\n");
    assert_eq!(sink.objects, [LinkId(0)]);
}

#[test]
fn random_labels_supplement_the_explicit_base_visible_witnesses() {
    let alphabet = b"a *`$~_!&\\\r\n\t()]";
    let mut seed = 29_u64;
    let mut accepted = 0;
    for i in 0..60_000 {
        let mut source = String::from("[");
        for _ in 0..1 + i % 25 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            source.push(alphabet[(seed >> 32) as usize % alphabet.len()] as char);
        }
        source.push(']');
        let doc = parse(&source);
        for (i, target) in doc.links().iter().enumerate() {
            let info = doc.link_info(LinkId(i)).unwrap();
            if matches!(target, LinkTarget::Reference(_)) && info.source.end == info.label.end + 1 {
                accepted += 1;
                assert!(
                    doc.runs_in(info.label)
                        .unwrap()
                        .any(|r| !r.style.contains(Style::HIDDEN)),
                    "{source:?}"
                );
            }
        }
    }
    assert_eq!(accepted, 26_552);
}
