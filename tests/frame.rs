use mdtui3::*;

#[derive(Default)]
struct Sink {
    bytes: usize,
    objects: usize,
    tiles: usize,
    rows: u64,
    zero: bool,
}
impl LayoutSink for Sink {
    fn begin_tile(&mut self, _: TileId) {
        self.tiles += 1;
        self.rows = 0;
    }
    fn text(&mut self, run: RenderRun<'_>) {
        self.bytes += run.text.len();
        if !run.style.contains(Style::HIDDEN) {
            self.rows += run.text.bytes().filter(|&b| b == b'\n').count() as u64;
        }
    }
    fn object(&mut self, _: LinkId, _: &LinkInfo) {
        self.objects += 1;
        self.rows += 1;
    }
    fn end_tile(&mut self, _: TileId) -> u64 {
        if self.zero { 0 } else { self.rows }
    }
}

fn limit() -> WorkLimit {
    WorkLimit {
        queries: 8,
        tiles: 2,
        bytes: 17,
        objects: 8,
        recovery: 1,
        dependencies: 2,
    }
}

#[test]
fn tiny_frames_progress_through_large_cells_and_zero_measurements() {
    let mut doc = Document::new();
    doc.append(&format!(
        "| a |\n| - |\n| {} |\n![](/image)\n",
        "中文🙂".repeat(1000)
    ))
    .unwrap();
    doc.finish();
    let mut heights = HeightIndex::default();
    heights.rebuild(&doc).unwrap();
    let mut pass = LayoutPass::new(&doc, &heights, TileId(0), 10, false).unwrap();
    let mut sink = Sink {
        zero: true,
        ..Default::default()
    };
    let mut positions = Vec::new();
    for _ in 0..5000 {
        let mut budget = Budget::new(limit());
        let bytes = sink.bytes;
        let status = pass
            .advance(&doc, &mut heights, &mut budget, &mut sink)
            .unwrap();
        assert!(sink.bytes - bytes <= limit().bytes);
        assert!(budget.used().tiles <= limit().tiles);
        positions.push(pass.source_position());
        if status == Completion::EndOfDocument {
            break;
        }
        assert_eq!(status, Completion::BudgetExhausted);
    }
    assert_eq!(pass.source_position(), doc.source().len());
    assert!(positions.windows(3).all(|w| w[0] < w[2]));
    assert_eq!(sink.objects, 1);
    assert_eq!(heights.total_rows(), 0);
}

#[test]
fn hidden_megabyte_is_skipped_but_empty_alt_image_is_delivered() {
    let mut doc = Document::new();
    doc.append(&format!(
        "```{}\n```\n![](url)\n",
        "hidden ".repeat(200_000)
    ))
    .unwrap();
    doc.finish();
    let image = doc
        .links_in(Span::new(0, doc.source().len()))
        .next()
        .unwrap()
        .1
        .source;
    assert_eq!(doc.next_drawable(image.start), Some(image.start));
    let mut heights = HeightIndex::default();
    heights.rebuild(&doc).unwrap();
    let mut pass = LayoutPass::new(&doc, &heights, TileId(0), 1, false).unwrap();
    let mut sink = Sink::default();
    let mut budget = Budget::new(WorkLimit {
        queries: 20,
        tiles: 4,
        bytes: 512,
        objects: 20,
        ..Default::default()
    });
    assert_eq!(
        pass.advance(&doc, &mut heights, &mut budget, &mut sink)
            .unwrap(),
        Completion::Covered
    );
    assert_eq!(sink.objects, 1);
    assert!(sink.bytes < 512);
    assert!(sink.tiles <= 2);
}

#[test]
fn continuations_validate_append_resize_and_view_and_share_query_budget() {
    let mut doc = Document::new();
    doc.append(&"x".repeat(5000)).unwrap();
    let mut heights = HeightIndex::default();
    heights.rebuild(&doc).unwrap();
    let mut out = Vec::new();
    let mut budget = Budget::new(WorkLimit {
        queries: 2,
        tiles: 2,
        ..Default::default()
    });
    let page = heights.viewport_budgeted(0..20, 0, &mut budget, &mut out);
    assert_eq!(out.len(), 2);
    let page = heights
        .resume_viewport(page.next.unwrap(), &mut budget, &mut out)
        .unwrap();
    assert!(out.is_empty());
    assert_eq!(page.completion, Completion::BudgetExhausted);
    heights.reset(1).unwrap();
    assert!(matches!(
        heights.resume_viewport(page.next.unwrap(), &mut budget, &mut out),
        Err(LayoutError::StaleContinuation)
    ));
    let mut pass = LayoutPass::new(&doc, &heights, TileId(10), 5, false).unwrap();
    assert!(pass.is_approximate());
    let update = doc.append("more").unwrap();
    heights.apply(&doc, &update).unwrap();
    assert_eq!(
        pass.advance(&doc, &mut heights, &mut budget, &mut Sink::default()),
        Err(LayoutError::StaleContinuation)
    );
}

#[test]
fn remote_checkpoints_do_not_force_unbounded_recovery() {
    let mut doc = Document::new();
    doc.append(&"x".repeat(1_000_000)).unwrap();
    let mut checkpoints = CheckpointIndex::new();
    checkpoints.rebuild(&doc);
    checkpoints.insert(TileId(0), 7).unwrap();
    let mut budget = Budget::new(WorkLimit {
        queries: 2,
        ..Default::default()
    });
    assert_eq!(
        checkpoints
            .recovery(&doc, TileId(3000), 1024, &mut budget)
            .unwrap(),
        None
    );
    assert_eq!(
        checkpoints
            .recovery(&doc, TileId(2), 1024, &mut budget)
            .unwrap(),
        Some((TileId(0), &7))
    );
    assert_eq!(budget.used().queries, 2);
}

#[test]
fn recovery_steps_are_shared_and_resumable() {
    let mut doc = Document::new();
    doc.append(&"x\n".repeat(2000)).unwrap();
    doc.finish();
    let mut heights = HeightIndex::default();
    heights.rebuild(&doc).unwrap();
    let mut pass = LayoutPass::from_checkpoint(&doc, &heights, TileId(0), TileId(2), 1, 512)
        .unwrap()
        .unwrap();
    let mut sink = Sink::default();
    for _ in 0..2000 {
        let mut budget = Budget::new(WorkLimit {
            queries: 10,
            tiles: 3,
            bytes: 64,
            objects: 10,
            recovery: 1,
            dependencies: 0,
        });
        let status = pass
            .advance(&doc, &mut heights, &mut budget, &mut sink)
            .unwrap();
        assert!(budget.used().recovery <= 1);
        if status == Completion::Covered {
            assert!(pass.next_tile().0 >= 3);
            return;
        }
    }
    panic!("recovery did not make progress");
}

#[test]
fn enlarging_offscreen_source_does_not_increase_frame_work() {
    let mut observations = Vec::new();
    for size in [4096, 1_000_000] {
        let mut doc = Document::new();
        doc.append(&format!(
            "{}{}",
            "visible **text**\n\n".repeat(100),
            "tail".repeat(size / 4)
        ))
        .unwrap();
        let mut heights = HeightIndex::default();
        heights.rebuild(&doc).unwrap();
        let mut pass = LayoutPass::new(&doc, &heights, TileId(0), 24, false).unwrap();
        let mut budget = Budget::new(limit());
        let mut sink = Sink::default();
        let status = pass
            .advance(&doc, &mut heights, &mut budget, &mut sink)
            .unwrap();
        observations.push((pass.source_position(), budget.used(), sink.bytes, status));
    }
    assert_eq!(observations[0], observations[1]);
}

#[test]
fn exact_query_budget_reports_covered_and_end_without_an_extra_query() {
    let mut heights = HeightIndex::default();
    heights.grow_to(2).unwrap();
    let mut out = Vec::new();
    for (end, expected) in [(2, Completion::Covered), (3, Completion::EndOfDocument)] {
        let mut budget = Budget::new(WorkLimit {
            queries: 2,
            tiles: 2,
            ..Default::default()
        });
        let page = heights.viewport_budgeted(0..end, 0, &mut budget, &mut out);
        assert_eq!(page.completion, expected);
        assert!(page.next.is_none());
        assert_eq!(out.len(), 2);
    }
}
