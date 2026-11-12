use mdstream::*;

fn micro() -> Budget {
    Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        tiles: 1,
        bytes: 6,
        recovery: 1,
        ..Default::default()
    })
}

fn parse(source: &str) -> Document {
    let mut document = Document::new();
    document.append(source).unwrap();
    document.finish();
    document
}

fn register(document: &Document, view: &mut FoldView, span: Span) -> FoldId {
    let mut task = SourceFoldRegistration::overlapping(document, view, span).unwrap();
    for _ in 0..10000 {
        if let Some(id) = task.advance(document, view, &mut micro()).unwrap().id {
            return id;
        }
    }
    panic!("registration stalled")
}

fn project(document: &Document, view: &FoldView, range: Span) -> Vec<FoldProjectionStep> {
    let mut task = FoldProjection::new(document, view, range).unwrap();
    let mut steps = Vec::new();
    let before = task.source_position();
    let mut zero = Budget::new(WorkLimit::default());
    assert_eq!(
        task.next(document, view, &mut zero).unwrap(),
        FoldProjectionStep::BudgetExhausted
    );
    assert_eq!(task.source_position(), before);
    for _ in 0..100000 {
        let mut budget = micro();
        let step = task.next(document, view, &mut budget).unwrap();
        assert_eq!(budget.used().bytes, 0);
        assert!(budget.used().queries <= 1 && budget.used().objects <= 1);
        if step == FoldProjectionStep::Complete {
            return steps;
        }
        if step != FoldProjectionStep::BudgetExhausted {
            steps.push(step);
        }
    }
    panic!("projection stalled")
}

#[test]
fn source_cut_union_frontiers_and_same_tile_placeholders_are_real_events() {
    let document = parse("abcdefghijklmnopqrstuvwxyz\n");
    let mut view = FoldView::new(&document, 3).unwrap();
    let a = register(&document, &mut view, Span::new(2, 9));
    let b = register(&document, &mut view, Span::new(7, 17));
    let c = register(&document, &mut view, Span::new(17, 20));
    for id in [a, b, c] {
        view.set_collapsed(&document, id, true).unwrap();
    }
    let heights = view.heights().arena_stats();
    let total = view.heights().total_rows();
    let steps = project(&document, &view, Span::new(0, 22));
    let placeholders: Vec<_> = steps
        .iter()
        .filter_map(|step| match step {
            FoldProjectionStep::Placeholder(p) => Some(*p),
            _ => None,
        })
        .collect();
    assert_eq!(placeholders.len(), 2);
    assert_eq!(placeholders[0].candidate.id, a);
    assert_eq!(placeholders[1].candidate.id, c);
    assert_eq!(placeholders[0].owner, placeholders[1].owner);
    let mut output = String::new();
    for step in steps {
        match step {
            FoldProjectionStep::Text { source, tile } => {
                assert_eq!(document.tile_at(source.start), Some(tile));
                output.push_str(document.text(source).unwrap());
            }
            FoldProjectionStep::Placeholder(_) => output.push('~'),
            FoldProjectionStep::Hidden { source, .. } => assert!(source.end <= 22),
            _ => {}
        }
    }
    assert_eq!(output, "ab~~uv");
    let remote = project(&document, &view, Span::new(5, 12));
    assert!(remote.iter().all(|s| matches!(s, FoldProjectionStep::Hidden { source, .. } if source.start >= 5 && source.end <= 12)));
    assert_eq!(view.heights().arena_stats(), heights);
    assert_eq!(view.heights().total_rows(), total);
}

// Independent sorted interval union. No production fold lookup or tree is used.
fn union(mut ranges: Vec<(Span, FoldId)>) -> Vec<(Span, FoldId)> {
    ranges.sort_by_key(|(s, id)| (s.start, std::cmp::Reverse(s.end), *id));
    let mut result: Vec<(Span, FoldId)> = Vec::new();
    for (span, id) in ranges {
        if let Some((last, _)) = result.last_mut()
            && span.start < last.end
        {
            last.end = last.end.max(span.end);
        } else {
            result.push((span, id));
        }
    }
    result
}

fn expected(source: &str, components: &[(Span, FoldId)], range: Span) -> String {
    let mut output = String::new();
    let mut position = range.start;
    for (span, _) in components {
        if span.end <= range.start || span.start >= range.end {
            continue;
        }
        if position < span.start {
            output.push_str(&source[position..span.start]);
        }
        if range.start <= span.start {
            output.push('~');
        }
        position = span.end.min(range.end);
    }
    output.push_str(&source[position..range.end]);
    output
}

fn output(document: &Document, steps: &[FoldProjectionStep]) -> String {
    steps
        .iter()
        .map(|step| match step {
            FoldProjectionStep::Text { source, .. } => document.text(*source).unwrap(),
            FoldProjectionStep::Placeholder(_) => "~",
            _ => "",
        })
        .collect()
}

#[derive(Default)]
struct RawSink {
    output: String,
    placeholders: Vec<FoldPlaceholder>,
    ends: Vec<BlockId>,
    bytes: usize,
    rows: u64,
    read_spans: Vec<Span>,
    styled: Vec<(usize, char, Style, Option<StylePairId>)>,
    objects: Vec<(LinkId, LinkDisposition)>,
}
impl LayoutSink for RawSink {
    fn begin_tile(&mut self, _: TileId) {
        self.rows = 0;
    }
    fn text(&mut self, run: RenderRun<'_>) {
        self.bytes += run.text.len();
        self.read_spans.push(run.source);
        if !run.style.contains(Style::HIDDEN) {
            self.styled.extend(
                run.text
                    .char_indices()
                    .map(|(offset, ch)| (run.source.start + offset, ch, run.style, run.style_pair)),
            );
            self.output.push_str(run.text);
            self.rows += run.text.bytes().filter(|b| *b == b'\n').count() as u64;
        }
    }
    fn object(&mut self, _: LinkId, _: &LinkInfo) {}
    fn object_with_disposition(&mut self, id: LinkId, _: &LinkInfo, disposition: LinkDisposition) {
        self.objects.push((id, disposition));
    }
    fn end_tile(&mut self, _: TileId) -> u64 {
        self.rows
    }
}
impl FoldLayoutSink for RawSink {
    fn placeholder(&mut self, p: FoldPlaceholder) {
        self.output.push('~');
        self.rows += 1;
        self.placeholders.push(p);
    }
    fn paragraph_end(&mut self, block: BlockId) {
        self.ends.push(block);
    }
}

fn raw(document: &Document, view: &mut FoldView) -> RawSink {
    raw_shared(document, view, &mut document.link_resolver())
}

fn raw_shared(document: &Document, view: &mut FoldView, resolver: &mut LinkResolver) -> RawSink {
    let mut pass = LayoutPass::folded(document, view, TileId(0), u64::MAX).unwrap();
    let mut sink = RawSink::default();
    for _ in 0..100000 {
        pass.swap_link_resolver(resolver).unwrap();
        let completion = pass
            .advance_folded(document, view, &mut micro(), &mut sink)
            .unwrap();
        pass.swap_link_resolver(resolver).unwrap();
        if completion == Completion::EndOfDocument {
            return sink;
        }
    }
    panic!("raw pass stalled")
}

#[test]
fn random_projection_matches_independent_union_and_real_layout_pass() {
    let source = format!("{}\n", "abcdefghijklmnop\n".repeat(70));
    let document = parse(&source);
    let mut view = FoldView::new(&document, 2).unwrap();
    let mut seed = 17017u64;
    let mut random = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        (seed >> 32) as usize
    };
    let mut choices = Vec::new();
    for _ in 0..40 {
        let start = random() % (source.len() - 1);
        let end = (start + 1 + random() % 160).min(source.len());
        choices.push((
            Span::new(start, end),
            register(&document, &mut view, Span::new(start, end)),
            false,
        ));
    }
    let mut preview = None;
    for round in 0..200 {
        let index = random() % choices.len();
        if round % 11 == 0 {
            preview = if preview.is_some() {
                None
            } else {
                Some(choices[index].1)
            };
            view.set_preview(&document, preview, &mut micro()).unwrap();
        } else if round % 17 == 0 {
            view.reset(2).unwrap();
        } else {
            choices[index].2 = !choices[index].2;
            view.toggle(&document, choices[index].1).unwrap();
        }
        let components = union(
            choices
                .iter()
                .filter(|(_, id, on)| *on && Some(*id) != preview)
                .map(|(span, id, _)| (*span, *id))
                .collect(),
        );
        let range = Span::new(0, source.len());
        let steps = project(&document, &view, range);
        assert_eq!(
            output(&document, &steps),
            expected(&source, &components, range)
        );
        for step in &steps {
            if let FoldProjectionStep::Hidden { source, candidate } = step {
                let (component, owner) = components
                    .iter()
                    .find(|(s, _)| s.start <= source.start && source.end <= s.end)
                    .unwrap();
                assert_eq!(candidate.id, *owner);
                assert_eq!(candidate.hidden.start, component.start);
                assert!(candidate.hidden.end <= component.end);
            }
        }
        let sink = raw(&document, &mut view);
        assert_eq!(sink.output, expected(&source, &components, range));
        assert_eq!(
            sink.placeholders
                .iter()
                .map(|p| p.candidate.id)
                .collect::<Vec<_>>(),
            components.iter().map(|(_, id)| *id).collect::<Vec<_>>()
        );
        let start = random() % source.len();
        let end = (start + random() % 400).min(source.len());
        if start < end {
            let clipped = Span::new(start, end);
            let steps = project(&document, &view, clipped);
            assert_eq!(
                output(&document, &steps),
                expected(&source, &components, clipped)
            );
        }
    }
}

#[test]
fn paragraph_recovery_and_union_extensions_match_pass_once_also_remotely() {
    let document = parse(&format!(
        "{}\n\n{}\n\nTAIL\n",
        "a".repeat(498),
        "b".repeat(498)
    ));
    let mut view = FoldView::new(&document, 2).unwrap();
    let mut registration =
        SourceFoldRegistration::paragraph_overlay(&document, &view, BlockId(0), 20).unwrap();
    let paragraph = loop {
        if let Some(id) = registration
            .advance(&document, &mut view, &mut micro())
            .unwrap()
            .id
        {
            break id;
        }
    };
    let crossing = register(&document, &mut view, Span::new(300, 900));
    for id in [paragraph, crossing] {
        view.set_collapsed(&document, id, true).unwrap();
    }
    let steps = project(&document, &view, Span::new(0, document.source().len()));
    let sink = raw(&document, &mut view);
    assert_eq!(output(&document, &steps), sink.output);
    let endings: Vec<_> = steps
        .iter()
        .filter_map(|s| match s {
            FoldProjectionStep::ParagraphEnd(b) => Some(*b),
            _ => None,
        })
        .collect();
    assert_eq!(endings, [BlockId(0)]);
    assert_eq!(endings, sink.ends);
    assert_eq!(
        steps
            .iter()
            .filter(|s| matches!(s, FoldProjectionStep::Placeholder(_)))
            .count(),
        1
    );
    let remote = project(&document, &view, Span::new(600, document.source().len()));
    assert!(
        !remote
            .iter()
            .any(|s| matches!(s, FoldProjectionStep::Placeholder(_)))
    );
    assert_eq!(
        remote
            .iter()
            .filter(|s| matches!(s, FoldProjectionStep::ParagraphEnd(BlockId(0))))
            .count(),
        1
    );
    // A cut before the paragraph frontier does not fabricate paragraph closure.
    let cut = project(&document, &view, Span::new(25, 30));
    assert!(
        !cut.iter()
            .any(|s| matches!(s, FoldProjectionStep::ParagraphEnd(_)))
    );
}

#[test]
fn continuations_bind_document_view_revision_fold_choices_and_checkpoint_tokens() {
    let mut document = Document::new();
    document.append(&"abcdefgh\n".repeat(120)).unwrap();
    let mut view = FoldView::<usize>::with_state(&document, 1).unwrap();
    view.insert_checkpoint(TileId(0), 10).unwrap();
    let range = Span::new(300, 700);
    assert!(
        FoldProjection::from_checkpoint(&document, &view, range, TileId(0), 299, &mut micro())
            .unwrap()
            .is_none()
    );
    let mut task =
        FoldProjection::from_checkpoint(&document, &view, range, TileId(0), 300, &mut micro())
            .unwrap()
            .unwrap();
    assert_eq!(task.requested(), range);
    assert_eq!(task.source_position(), 0);
    let mut no_recovery = Budget::new(WorkLimit {
        queries: 10,
        objects: 10,
        tiles: 10,
        ..Default::default()
    });
    assert_eq!(
        task.next(&document, &view, &mut no_recovery).unwrap(),
        FoldProjectionStep::BudgetExhausted
    );
    assert_eq!(no_recovery.used().queries, 1);
    assert_eq!(no_recovery.used().recovery, 0);
    let mut work = 0;
    for _ in 0..100 {
        let mut budget = micro();
        let step = task.next(&document, &view, &mut budget).unwrap();
        work += budget.used().recovery;
        if step == FoldProjectionStep::Complete {
            break;
        }
    }
    assert!(work > 0);
    view.layout_parts_mut(&document)
        .unwrap()
        .1
        .invalidate(0..1)
        .unwrap();
    let mut budget = micro();
    assert_eq!(
        task.next(&document, &view, &mut budget),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(
        budget.used(),
        WorkLimit {
            queries: 1,
            ..Default::default()
        }
    );
    let mut task = FoldProjection::new(&document, &view, range).unwrap();
    let foreign = parse("foreign\n");
    assert_eq!(
        task.next(&foreign, &view, &mut micro()),
        Err(LayoutError::WrongDocument)
    );
    let other = FoldView::new(&document, 1).unwrap();
    assert_eq!(
        task.next(&document, &other, &mut micro()),
        Err(LayoutError::WrongView)
    );
    view.reset(2).unwrap();
    assert_eq!(
        task.next(&document, &view, &mut micro()),
        Err(LayoutError::StaleContinuation)
    );
    let mut task = FoldProjection::new(&document, &view, range).unwrap();
    let update = document.append("append\n").unwrap();
    view.apply(&document, &update).unwrap();
    assert_eq!(
        task.next(&document, &view, &mut micro()),
        Err(LayoutError::StaleContinuation)
    );
    for invalid in [Span::new(2, 1), Span::new(0, document.source().len() + 1)] {
        assert!(matches!(
            FoldProjection::new(&document, &view, invalid),
            Err(LayoutError::InvalidRange)
        ));
    }
    let empty = Span::new(document.source().len(), document.source().len());
    let mut task = FoldProjection::new(&document, &view, empty).unwrap();
    assert_eq!(
        task.next(&document, &view, &mut Budget::new(WorkLimit::default()))
            .unwrap(),
        FoldProjectionStep::Complete
    );
    assert_eq!(
        task.next(&document, &view, &mut Budget::new(WorkLimit::default()))
            .unwrap(),
        FoldProjectionStep::Complete
    );
}

#[test]
fn utf8_streaming_fixed_cuts_growth_reclassification_finish_and_stale_tasks() {
    for source in [
        "中文🙂\r\n---\r\nnext\n",
        "| a | b |\n| - | - |\n| x | y |\n",
        "> - para\n>   more\n\n# out\n",
        "```rust\nbody\n```\nout\n",
        "$$\nx+y\n$$\nout\n",
        "[r]: /url\n \"title\"\nnext\n",
    ] {
        let cuts: Vec<_> = (0..=source.len())
            .filter(|&i| source.is_char_boundary(i))
            .collect();
        for &cut in cuts.iter().filter(|&&i| i > 0) {
            let mut document = Document::new();
            document.append(&source[..cut]).unwrap();
            let mut view = FoldView::new(&document, 2).unwrap();
            let mut registration = SourceFoldRegistration::stream(&document, &view, 0).unwrap();
            let id = loop {
                if let Some(id) = registration
                    .advance(&document, &mut view, &mut micro())
                    .unwrap()
                    .id
                {
                    break id;
                }
            };
            view.set_collapsed(&document, id, true).unwrap();
            let mut old = FoldProjection::new(&document, &view, Span::new(0, cut)).unwrap();
            for w in cuts.windows(2).filter(|w| w[0] >= cut) {
                let update = document.append(&source[w[0]..w[1]]).unwrap();
                view.apply(&document, &update).unwrap();
                assert_eq!(
                    old.next(&document, &view, &mut micro()),
                    Err(LayoutError::StaleContinuation)
                );
                let steps = project(&document, &view, Span::new(0, document.source().len()));
                assert_eq!(output(&document, &steps), "~");
                assert!(steps.iter().any(
                    |s| matches!(s, FoldProjectionStep::Placeholder(p) if p.candidate.id == id)
                ));
                old = FoldProjection::new(&document, &view, Span::new(0, document.source().len()))
                    .unwrap();
            }
            let update = document.finish();
            view.apply(&document, &update).unwrap();
            assert_eq!(
                old.next(&document, &view, &mut micro()),
                Err(LayoutError::StaleContinuation)
            );
            let steps = project(&document, &view, Span::new(0, source.len()));
            assert_eq!(output(&document, &steps), "~");
            assert!(steps.iter().any(|s|matches!(s, FoldProjectionStep::Placeholder(p) if p.candidate.id == id && !p.candidate.open)));
        }
    }
    let document = parse("a中🙂b\n");
    let view = FoldView::new(&document, 1).unwrap();
    assert!(matches!(
        FoldProjection::new(&document, &view, Span::new(2, 4)),
        Err(LayoutError::InvalidRange)
    ));
    let mut document = Document::new();
    document.append("fixed").unwrap();
    let mut view = FoldView::new(&document, 2).unwrap();
    let id = register(&document, &mut view, Span::new(0, 5));
    view.set_collapsed(&document, id, true).unwrap();
    let update = document.append(" visible\n").unwrap();
    view.apply(&document, &update).unwrap();
    assert_eq!(
        output(
            &document,
            &project(&document, &view, Span::new(0, document.source().len()))
        ),
        "~ visible\n"
    );
    assert_eq!(
        view.fold_candidate(&document, id).unwrap().unwrap().hidden,
        Span::new(0, 5)
    );
}

#[test]
fn paragraph_late_setext_removes_registration_without_transferring_identity() {
    let mut document = Document::new();
    document.append("a long paragraph\n").unwrap();
    let mut view = FoldView::new(&document, 2).unwrap();
    let mut registration =
        SourceFoldRegistration::open_paragraph(&document, &view, BlockId(0), 4).unwrap();
    let id = loop {
        if let Some(id) = registration
            .advance(&document, &mut view, &mut micro())
            .unwrap()
            .id
        {
            break id;
        }
    };
    view.set_collapsed(&document, id, true).unwrap();
    let mut old =
        FoldProjection::new(&document, &view, Span::new(0, document.source().len())).unwrap();
    let update = document.append("---\nnext\n").unwrap();
    view.apply(&document, &update).unwrap();
    assert_eq!(
        old.next(&document, &view, &mut micro()),
        Err(LayoutError::StaleContinuation)
    );
    assert!(view.fold_candidate(&document, id).unwrap().is_none());
    let steps = project(&document, &view, Span::new(0, document.source().len()));
    assert!(
        !steps
            .iter()
            .any(|s| matches!(s, FoldProjectionStep::Placeholder(_)))
    );
}

#[test]
fn crossing_chain_frontiers_are_paid_and_deep_reveal_rejects_old_projection() {
    let count = 512;
    let document = parse(&"a".repeat((count + 2) * 128));
    let mut view = FoldView::new(&document, 2).unwrap();
    let mut first = None;
    for i in 0..count {
        let id = register(&document, &mut view, Span::new(i * 128, (i + 2) * 128));
        first.get_or_insert(id);
        view.set_collapsed(&document, id, true).unwrap();
    }
    let before = view.fold_index_visits();
    let mut task =
        FoldProjection::new(&document, &view, Span::new(0, document.source().len())).unwrap();
    let mut frontiers = 0;
    let mut queries = 0;
    let mut placeholders = 0;
    for _ in 0..count * 8 {
        let mut budget = micro();
        let step = task.next(&document, &view, &mut budget).unwrap();
        queries += budget.used().queries;
        match step {
            FoldProjectionStep::Hidden { .. } => frontiers += 1,
            FoldProjectionStep::Placeholder(p) => {
                placeholders += 1;
                assert_eq!(Some(p.candidate.id), first);
            }
            FoldProjectionStep::Complete => break,
            _ => {}
        }
    }
    assert_eq!(task.source_position(), document.source().len());
    assert_eq!(placeholders, 1);
    assert!(frontiers > 1 && frontiers <= count);
    assert!(queries >= frontiers * 2 && queries < count * 4 + 16);
    assert!(view.fold_index_visits() - before < (count * 100) as u64);
    let document = parse(&format!("{}body\n", "> ".repeat(600)));
    let mut view = FoldView::new(&document, 2).unwrap();
    for i in 0..document.container_count() {
        view.set_collapsed(&document, FoldId::Container(ContainerId(i)), true)
            .unwrap();
    }
    let mut old =
        FoldProjection::new(&document, &view, Span::new(0, document.source().len())).unwrap();
    let steps = project(&document, &view, Span::new(0, document.source().len()));
    assert!(
        matches!(steps.first(), Some(FoldProjectionStep::Placeholder(p)) if p.candidate.id == FoldId::Container(ContainerId(0)))
    );
    let mut navigation = FoldNavigation::new(
        &document,
        &view,
        &Navigator::new(),
        NavAction::Source(document.source().find("body").unwrap()),
        FoldNavigationPolicy::Reveal,
        24,
    )
    .unwrap();
    for frame in 0..610 {
        if navigation
            .advance(&document, &mut view, &mut micro())
            .unwrap()
            .completion
            == Completion::Covered
        {
            break;
        }
        assert!(frame < 609);
    }
    assert_eq!(
        old.next(&document, &view, &mut micro()),
        Err(LayoutError::StaleContinuation)
    );
}

#[test]
fn hidden_64_mib_does_not_allocate_scan_tiles_or_publish_false_measurements() {
    let mut document = Document::new();
    document.append("HEAD\n\n").unwrap();
    let start = document.source().len();
    document.append(&"x".repeat(64 * 1024 * 1024)).unwrap();
    let end = document.source().len();
    document.append("\n\nTAIL\n").unwrap();
    document.finish();
    let mut view = FoldView::new(&document, 2).unwrap();
    let id = register(&document, &mut view, Span::new(start, end));
    view.set_collapsed(&document, id, true).unwrap();
    let arena = view.heights().arena_stats();
    let mutation = view.heights().mutation_visits();
    let total = view.heights().total_rows();
    let visits = view.fold_index_visits();
    let source_visits = view.source_index_visits();
    let steps = project(&document, &view, Span::new(0, document.source().len()));
    assert_eq!(output(&document, &steps), "HEAD\n\n~\n\nTAIL\n");
    let mut bytes = 0;
    for step in &steps {
        if let FoldProjectionStep::Text { source, .. } = step {
            bytes += source.len();
            assert!(source.end <= start || source.start >= end);
        }
    }
    assert!(bytes < 32 && steps.len() < 10);
    assert!(std::mem::size_of::<FoldProjection>() < 512);
    assert_eq!(view.heights().arena_stats(), arena);
    assert_eq!(view.heights().mutation_visits(), mutation);
    assert_eq!(view.heights().total_rows(), total);
    assert!(view.fold_index_visits() - visits < 128);
    assert!(view.source_index_visits() - source_visits < 128);
    println!(
        "64MiB: projection_size={}, events={}, visible_bytes={}, height_capacity_bytes={}, height_nodes={}, fold_visits={}, source_visits={}",
        std::mem::size_of::<FoldProjection>(),
        steps.len(),
        bytes,
        arena.capacity_bytes,
        arena.nodes,
        view.fold_index_visits() - visits,
        view.source_index_visits() - source_visits
    );
    let sink = raw(&document, &mut view);
    assert_eq!(sink.output, "HEAD\n\n~\n\nTAIL\n");
    assert!(sink.bytes < 32);
}

#[test]
fn seven_families_syntax_hidden_objects_empty_sections_and_same_start_owner() {
    let document = parse(
        "# Root\nbody\n```rust\n```\n$$\nx\n$$\n> quote\n\n| h |\n| - |\n| row |\n# Empty\n# Last\ntail\n",
    );
    let mut view = FoldView::new(&document, 1).unwrap();
    let mut candidates = Vec::new();
    document
        .fold_candidates(Span::new(0, document.source().len()))
        .unwrap()
        .advance(
            &document,
            &mut Budget::new(WorkLimit::unlimited()),
            &mut candidates,
        )
        .unwrap();
    let rows = view.insert_table_rows(&document, TableId(0), 1..2).unwrap();
    let source = register(&document, &mut view, Span::new(7, 10));
    let mut ids: Vec<_> = candidates.iter().map(|c| c.id).collect();
    ids.extend([rows, source]);
    assert!(ids.iter().any(|id| matches!(id, FoldId::Code(_))));
    assert!(ids.iter().any(|id| matches!(id, FoldId::Math(_))));
    assert!(ids.iter().any(|id| matches!(id, FoldId::Container(_))));
    assert!(ids.iter().any(|id| matches!(id, FoldId::Table(_))));
    for id in ids {
        view.set_collapsed(&document, id, true).unwrap();
        let steps = project(&document, &view, Span::new(0, document.source().len()));
        let mut projected = String::new();
        for step in &steps {
            match step {
                FoldProjectionStep::Text { source, .. } => {
                    for run in document
                        .runs_in(*source)
                        .unwrap()
                        .filter(|r| !r.style.contains(Style::HIDDEN))
                    {
                        projected.push_str(run.text);
                    }
                }
                FoldProjectionStep::Placeholder(p) => {
                    assert_eq!(p.candidate.id, id);
                    projected.push('~');
                }
                _ => {}
            }
        }
        let sink = raw(&document, &mut view);
        assert_eq!(projected, sink.output);
        let hidden = view.fold_candidate(&document, id).unwrap().unwrap().hidden;
        assert_eq!(sink.placeholders.len(), usize::from(!hidden.is_empty()));
        view.set_collapsed(&document, id, false).unwrap();
    }
    let mut document = Document::new();
    document.append("# Root\nbody\n").unwrap();
    let mut view = FoldView::new(&document, 1).unwrap();
    let parent = FoldId::Heading(HeadingId(0));
    let hidden_start = document.fold_candidate(parent).unwrap().hidden.start;
    let mut registration = SourceFoldRegistration::stream(&document, &view, hidden_start).unwrap();
    let child = loop {
        if let Some(id) = registration
            .advance(&document, &mut view, &mut micro())
            .unwrap()
            .id
        {
            break id;
        }
    };
    for id in [parent, child] {
        view.set_collapsed(&document, id, true).unwrap();
    }
    let update = document.finish();
    view.apply(&document, &update).unwrap();
    let steps = project(&document, &view, Span::new(0, document.source().len()));
    let placeholders: Vec<_> = steps
        .iter()
        .filter_map(|s| match s {
            FoldProjectionStep::Placeholder(p) => Some(p.candidate.id),
            _ => None,
        })
        .collect();
    assert_eq!(placeholders, [parent]);
}

#[test]
fn restored_hidden_events_split_recovery_cut_and_keep_owner_and_end_binding() {
    let document = parse(&format!("{}\n\nTAIL\n", "a".repeat(1000)));
    let mut view = FoldView::new(&document, 1).unwrap();
    let mut registration =
        SourceFoldRegistration::paragraph_overlay(&document, &view, BlockId(0), 0).unwrap();
    let id = loop {
        if let Some(id) = registration
            .advance(&document, &mut view, &mut micro())
            .unwrap()
            .id
        {
            break id;
        }
    };
    view.set_collapsed(&document, id, true).unwrap();
    view.insert_checkpoint(TileId(0), ()).unwrap();
    let range = Span::new(300, document.source().len());
    let mut zero = Budget::new(WorkLimit::default());
    assert!(
        FoldProjection::from_checkpoint(&document, &view, range, TileId(0), 300, &mut zero)
            .unwrap()
            .is_none()
    );
    assert_eq!(zero.used(), WorkLimit::default());
    let mut task =
        FoldProjection::from_checkpoint(&document, &view, range, TileId(0), 300, &mut micro())
            .unwrap()
            .unwrap();
    let mut spans = Vec::new();
    let mut endings = 0;
    let mut placeholders = 0;
    for frame in 0..100 {
        let before = task.source_position();
        let mut budget = micro();
        let step = task.next(&document, &view, &mut budget).unwrap();
        match step {
            FoldProjectionStep::Hidden { source, candidate } => {
                assert_eq!(candidate.id, id);
                assert!(source.end <= range.start || source.start >= range.start);
                assert_eq!(budget.used().recovery, usize::from(before < range.start));
                spans.push(source);
            }
            FoldProjectionStep::Placeholder(_) => placeholders += 1,
            FoldProjectionStep::ParagraphEnd(_) => endings += 1,
            FoldProjectionStep::Complete => break,
            _ => {}
        }
        assert!(frame < 99);
    }
    assert_eq!(placeholders, 1); // Recovery event at owner zero; not screen output.
    assert_eq!(endings, 1);
    assert_eq!(spans[0], Span::new(0, 300));
    assert_eq!(spans[1].start, 300);
    assert_eq!(task.source_position(), range.end);
}

#[test]
fn committed_010_late_523_keeps_projection_raw_and_shared_layout_style_identity() {
    let mut document = Document::new();
    document.append("*foo [bar* baz]\n\n").unwrap();
    let cut = Span::new(0, document.source().len());
    let hidden = Span::new(11, 14); // Exact original bytes of baz, independent of display.
    assert_eq!(document.text(hidden), Some("baz"));
    let mut view = FoldView::new(&document, 2).unwrap();
    let id = register(&document, &mut view, hidden);
    view.set_collapsed(&document, id, true).unwrap();
    let mut resolver = document.link_resolver();
    let before = raw_shared(&document, &mut view, &mut resolver);
    assert_eq!(before.output, "foo [bar ~]\n\n");
    let pair = StylePairId {
        open: Span::new(0, 1),
        close: Span::new(9, 10),
        kind: InlineStyleKind::Emphasis,
    };
    assert_eq!(
        before
            .styled
            .iter()
            .filter(|(at, _, _, _)| (1..9).contains(at))
            .count(),
        8
    );
    for (at, _, style, identity) in &before.styled {
        if (1..9).contains(at) {
            assert!(style.contains(Style::EMPHASIS));
            assert_eq!(*identity, Some(pair));
        }
    }
    let memo = resolver.style_memo_entries();
    let steps = project(&document, &view, cut);
    assert_eq!(resolver.style_memo_entries(), memo);
    assert_eq!(output(&document, &steps), "*foo [bar* ~]\n\n");
    let mut old_projection = FoldProjection::new(&document, &view, cut).unwrap();
    let mut old_pass = LayoutPass::folded(&document, &view, TileId(0), u64::MAX).unwrap();
    let ticket = view.heights().ticket(TileId(0)).unwrap();
    let update = document.append("[bar* baz]: /u\n").unwrap();
    view.apply(&document, &update).unwrap();
    let mut budget = micro();
    assert_eq!(
        old_projection.next(&document, &view, &mut budget),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(budget.used(), WorkLimit::default());
    assert_eq!(
        old_pass.advance_folded(&document, &mut view, &mut budget, &mut RawSink::default()),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(budget.used(), WorkLimit::default());
    assert_eq!(view.measure(ticket, 99), Err(LayoutError::StaleMeasurement));
    let mut retired = false;
    for _ in 0..1000 {
        if resolver.retire(&mut micro()) == Completion::Covered {
            retired = true;
            break;
        }
    }
    assert!(retired);
    assert_eq!(resolver.memo_entries(), 0);
    resolver.reset(&document).unwrap();
    let after = raw_shared(&document, &mut view, &mut resolver);
    assert_eq!(after.output, "*foo bar* ~\n\n");
    assert!(
        after
            .styled
            .iter()
            .all(|(_, _, style, identity)| !style.contains(Style::EMPHASIS) && identity.is_none())
    );
    assert_eq!(
        view.fold_candidate(&document, id).unwrap().unwrap().hidden,
        hidden
    );
    let steps = project(&document, &view, cut);
    assert_eq!(output(&document, &steps), "*foo [bar* ~]\n\n");
    for span in before.read_spans.iter().chain(&after.read_spans) {
        assert!(span.end <= hidden.start || hidden.end <= span.start);
    }
}

#[test]
fn committed_010_old_label_header_withdrawal_preserves_overlay_and_paragraph_end() {
    let prior = "[prior\n";
    let mut document = Document::new();
    document.append(prior).unwrap();
    document
        .append(&format!(
            "end](u*x*) *x* [h](v) $q$ [^n] {}",
            "a".repeat(1300)
        ))
        .unwrap();
    let old_link = document
        .links_in(Span::new(0, document.source().len()))
        .find(|(_, info)| info.source.start == 0)
        .unwrap()
        .0;
    let hidden = Span::new(document.source().len() - 120, document.source().len() - 80);
    let mut view = FoldView::new(&document, 2).unwrap();
    let overlay = register(&document, &mut view, hidden);
    view.set_collapsed(&document, overlay, true).unwrap();
    let mut registration =
        SourceFoldRegistration::paragraph_overlay(&document, &view, BlockId(0), 2).unwrap();
    let paragraph = loop {
        if let Some(id) = registration
            .advance(&document, &mut view, &mut micro())
            .unwrap()
            .id
        {
            break id;
        }
    };
    view.set_collapsed(&document, paragraph, true).unwrap();
    let mut resolver = document.link_resolver();
    let before = raw_shared(&document, &mut view, &mut resolver);
    assert_eq!(before.ends, [BlockId(0)]);
    let mut old =
        FoldProjection::new(&document, &view, Span::new(0, document.source().len())).unwrap();
    let ticket = view.heights().ticket(TileId(0)).unwrap();
    let update = document.append("|b\n---|---\nx|y\n\n").unwrap();
    view.apply(&document, &update).unwrap();
    assert_eq!(document.tables().len(), 1);
    assert_eq!(
        old.next(&document, &view, &mut micro()),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(view.measure(ticket, 99), Err(LayoutError::StaleMeasurement));
    let candidate = view.fold_candidate(&document, paragraph).unwrap().unwrap();
    assert_eq!(candidate.source, Span::new(0, prior.len()));
    assert_eq!(candidate.hidden, Span::new(2, prior.len()));
    assert_eq!(
        view.fold_candidate(&document, overlay)
            .unwrap()
            .unwrap()
            .hidden,
        hidden
    );
    let mut retired = false;
    for _ in 0..10000 {
        if resolver.retire(&mut micro()) == Completion::Covered {
            retired = true;
            break;
        }
    }
    assert!(retired);
    resolver.reset(&document).unwrap();
    let after = raw_shared(&document, &mut view, &mut resolver);
    assert!(
        after.output.starts_with("[p~end](ux) x h q "),
        "{}",
        after.output
    );
    assert_eq!(after.ends, [BlockId(0)]);
    assert_eq!(
        after
            .placeholders
            .iter()
            .map(|p| p.candidate.id)
            .collect::<Vec<_>>(),
        [paragraph, overlay]
    );
    assert!(!after.objects.iter().any(|(id, _)| *id == old_link));
    let source = document.source();
    let first_x = source.find("u*x*").unwrap() + 2;
    let second_open = source.find(" *x* ").unwrap() + 1;
    assert_eq!(
        after
            .styled
            .iter()
            .filter(|(at, _, _, _)| *at == first_x || *at == second_open + 1)
            .count(),
        2
    );
    for (at, _, style, pair) in &after.styled {
        if *at == first_x || *at == second_open + 1 {
            assert!(style.contains(Style::EMPHASIS));
            let open = if *at == first_x {
                first_x - 1
            } else {
                second_open
            };
            assert_eq!(
                *pair,
                Some(StylePairId {
                    open: Span::new(open, open + 1),
                    close: Span::new(open + 2, open + 3),
                    kind: InlineStyleKind::Emphasis
                })
            );
        }
    }
    let memo = resolver.style_memo_entries();
    let steps = project(&document, &view, Span::new(0, source.len()));
    assert_eq!(resolver.style_memo_entries(), memo);
    let mut expected = String::from("[p~");
    expected.push_str(&source[prior.len()..hidden.start]);
    expected.push('~');
    expected.push_str(&source[hidden.end..]);
    assert_eq!(output(&document, &steps), expected);
    assert_eq!(
        steps
            .iter()
            .filter(|s| matches!(s, FoldProjectionStep::ParagraphEnd(BlockId(0))))
            .count(),
        1
    );
    for span in &after.read_spans {
        for skip in [candidate.hidden, hidden] {
            assert!(span.end <= skip.start || skip.end <= span.start);
        }
    }
    let cell = document.tables()[0].cell(0, 0).unwrap().source;
    let mut maths = document.maths_in(cell).unwrap();
    let (math_id, math) = maths
        .advance(&mut Budget::new(WorkLimit::unlimited()))
        .unwrap()
        .unwrap();
    assert_eq!(document.math_state(math_id), Some(SyntaxState::Sealed));
    assert_eq!(math.source.start, source.find("$q$").unwrap());
    assert!(
        maths
            .advance(&mut Budget::new(WorkLimit::unlimited()))
            .unwrap()
            .is_none()
    );
    let mut notes = document.footnote_references_in(cell).unwrap();
    let (note_id, note) = notes
        .advance(&mut Budget::new(WorkLimit::unlimited()))
        .unwrap()
        .unwrap();
    assert_eq!(
        document.footnote_reference_state(note_id),
        Some(SyntaxState::Sealed)
    );
    assert_eq!(note.source.start, source.find("[^n]").unwrap());
    assert!(
        notes
            .advance(&mut Budget::new(WorkLimit::unlimited()))
            .unwrap()
            .is_none()
    );
}
