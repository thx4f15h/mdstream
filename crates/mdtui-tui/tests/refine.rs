use mdtui_tui::render::first_frame::{FirstFrame, FirstFrameConfig, FirstFrameStatus};
use mdtui_tui::render::refine::{
    RefineConfig, RefineError, RefineProgress, RefineStatus, RefinedTarget, SourceRefiner,
};
use mdtui3::{
    Budget, Document, HeadingId, HeightIndex, NavAction, Navigator, TILE_BYTES, TileId, WorkLimit,
};

fn parse(source: &str) -> Document {
    let mut document = Document::new();
    document.append(source).unwrap();
    document.finish();
    document
}

fn unlimited() -> Budget {
    Budget::new(WorkLimit::unlimited())
}

fn full_first_frame(document: &Document, width: u16) -> FirstFrame {
    let mut first = FirstFrame::new(
        document,
        FirstFrameConfig {
            top_tile: TileId(0),
            tile_limit: document.tile_count().max(1),
            width,
            row_limit: 100_000,
        },
    )
    .unwrap();
    for _ in 0..10_000 {
        let step = first.advance(document, &mut unlimited()).unwrap();
        if matches!(
            step.status,
            FirstFrameStatus::Covered | FirstFrameStatus::EndOfDocument
        ) {
            return first;
        }
    }
    panic!("full first frame did not settle");
}

fn measured_layout(
    document: &Document,
    width: u16,
) -> (
    HeightIndex,
    mdtui_tui::render::checkpoint::RenderCheckpointStore,
) {
    full_first_frame(document, width).into_layout_parts()
}

fn config(width: u16) -> RefineConfig {
    RefineConfig {
        width,
        max_tiles: 32,
        max_replay_bytes: 256 * 1024,
        row_slack: 64,
    }
}

fn refine(
    document: &Document,
    heights: &mut HeightIndex,
    checkpoints: &mut mdtui_tui::render::checkpoint::RenderCheckpointStore,
    source: usize,
    width: u16,
) -> RefineProgress {
    let mut refiner =
        SourceRefiner::new(document, heights, checkpoints, source, config(width)).unwrap();
    for _ in 0..10_000 {
        let progress = refiner
            .advance(document, heights, checkpoints, &mut unlimited())
            .unwrap();
        if matches!(
            progress,
            RefineProgress::Ready(_) | RefineProgress::NotFound
        ) {
            return progress;
        }
    }
    panic!("source refinement did not settle");
}

fn ready(progress: RefineProgress) -> RefinedTarget {
    match progress {
        RefineProgress::Ready(target) => target,
        other => panic!("expected refined target, got {other:?}"),
    }
}

fn assert_row_round_trip(target: RefinedTarget, heights: &HeightIndex) {
    assert_eq!(heights.row_of(target.anchor), Some(target.row));
    assert!(target.plan_end > target.plan_start);
    assert!(target.plan_end.0 - target.plan_start.0 <= config(1).max_tiles);
}

#[test]
fn same_tile_headings_refine_to_distinct_rows_and_preserve_source() {
    let document = parse("# one\n## two\n### three\nbody\n");
    let first_tile = document.tile_at(0).unwrap();
    assert_eq!(
        document.tile_at(document.source().len() - 1),
        Some(first_tile)
    );
    let (mut heights, mut checkpoints) = measured_layout(&document, 40);
    let entries = document.headings().entries();
    let one = entries[0];
    let two = entries[1];
    let three = entries[2];

    let refined_one = ready(refine(
        &document,
        &mut heights,
        &mut checkpoints,
        one.source.start,
        40,
    ));
    let refined_two = ready(refine(
        &document,
        &mut heights,
        &mut checkpoints,
        two.source.start,
        40,
    ));
    let refined_three = ready(refine(
        &document,
        &mut heights,
        &mut checkpoints,
        three.source.start,
        40,
    ));

    assert_eq!(refined_one.source, one.source.start);
    assert_eq!(refined_two.source, two.source.start);
    assert_eq!(refined_three.source, three.source.start);
    assert_eq!(
        (refined_one.row, refined_two.row, refined_three.row),
        (0, 2, 4)
    );
    assert_eq!((refined_one.line_start, refined_two.line_start), (0, 6));
    assert_eq!(refined_three.line_start, 13);
    for target in [refined_one, refined_two, refined_three] {
        assert_row_round_trip(target, &heights);
    }
}

#[test]
fn source_inside_heading_markers_or_content_lands_on_heading_row() {
    let document = parse("# **Alpha** and `code`\n\nbody\n");
    let heading = document.headings().entries()[0];
    let inside_content = document.source()[heading.content.start..]
        .find("Alpha")
        .map(|offset| heading.content.start + offset)
        .unwrap();
    let (mut heights, mut checkpoints) = measured_layout(&document, 40);
    for source in [
        heading.source.start,
        heading.source.start + 1,
        inside_content,
    ] {
        let target = ready(refine(
            &document,
            &mut heights,
            &mut checkpoints,
            source,
            40,
        ));
        assert_eq!(target.row, 0);
        assert_eq!(target.line_start, heading.source.start);
        assert_eq!(target.anchor.tile, TileId(0));
        assert_row_round_trip(target, &heights);
    }
}

#[test]
fn setext_heading_mid_second_physical_line_refines_to_the_heading_row() {
    let document = parse("first line\nsecond line\n===\n\nbody\n");
    let heading = document.headings().entries()[0];
    assert!(heading.source.start < heading.source.end);
    let second_line = document.source()[heading.source.start..heading.source.end]
        .find("second")
        .map(|offset| heading.source.start + offset)
        .unwrap();
    let (mut heights, mut checkpoints) = measured_layout(&document, 40);
    let target = ready(refine(
        &document,
        &mut heights,
        &mut checkpoints,
        second_line,
        40,
    ));
    assert_eq!(target.row, 0);
    assert_eq!(target.line_start, heading.source.start);
    assert_row_round_trip(target, &heights);
}

#[test]
fn cross_tile_long_line_middle_refines_to_wrapped_row_not_owner_tile_start() {
    let source = format!("{}\n", "x".repeat(TILE_BYTES * 2 + 100));
    let document = parse(&source);
    let target_source = source.len() / 2;
    let (mut heights, mut checkpoints) = measured_layout(&document, 20);
    let target = ready(refine(
        &document,
        &mut heights,
        &mut checkpoints,
        target_source,
        20,
    ));
    assert!(target.line_start <= target_source);
    let owner_start = document.tile(target.anchor.tile).unwrap().start;
    assert!(target.line_start < owner_start);
    assert!(target.row > 0);
    assert_row_round_trip(target, &heights);
}

#[test]
fn exact_replay_and_approximate_recovery_converge_on_the_same_row() {
    let source = format!(
        "{}\n\n## Target\n\n{}",
        "a".repeat(TILE_BYTES * 2 + 80),
        "z".repeat(TILE_BYTES)
    );
    let document = parse(&source);
    let target_source = source.find("Target").unwrap();
    let target_tile = document.tile_at(target_source).unwrap();
    assert!(target_tile.0 > 1);
    assert!(target_tile.0 + 1 < document.tile_count());
    let (mut heights, mut checkpoints) = measured_layout(&document, 30);

    let exact = ready(refine(
        &document,
        &mut heights,
        &mut checkpoints,
        target_source,
        30,
    ));
    assert_eq!(exact.status, RefineStatus::Exact);

    checkpoints
        .invalidate(target_tile.0..target_tile.0 + 1)
        .unwrap();
    let replay = ready(refine(
        &document,
        &mut heights,
        &mut checkpoints,
        target_source,
        30,
    ));
    assert!(matches!(replay.status, RefineStatus::ReplayFrom { .. }));

    checkpoints.clear();
    let approximate = ready(refine(
        &document,
        &mut heights,
        &mut checkpoints,
        target_source,
        30,
    ));
    assert_eq!(approximate.status, RefineStatus::Approximate);
    assert_eq!(exact.row, replay.row);
    assert_eq!(exact.row, approximate.row);
    for target in [exact, replay, approximate] {
        assert_row_round_trip(target, &heights);
    }
}

#[test]
fn navigator_reconcile_uses_distinct_heading_rows_and_failure_does_not_move() {
    let document = parse("# one\n## two\n### three\n\n[x]: https://example.com/\n");
    let (mut heights, mut checkpoints) = measured_layout(&document, 40);
    let mut navigator = Navigator::new();
    let _ = navigator
        .navigate(&document, &heights, NavAction::Heading(HeadingId(0)), 2)
        .unwrap();

    let one = ready(refine(
        &document,
        &mut heights,
        &mut checkpoints,
        document.headings().entries()[0].source.start,
        40,
    ));
    let applied_one = one.apply_to_navigator(&mut navigator, &heights, 2);
    assert_eq!(applied_one.source, Some(0));
    assert_eq!(navigator.top(&heights), applied_one.top);

    let two = ready(refine(
        &document,
        &mut heights,
        &mut checkpoints,
        document.headings().entries()[1].source.start,
        40,
    ));
    let applied_two = two.apply_to_navigator(&mut navigator, &heights, 2);
    assert_eq!(applied_two.source, Some(6));
    assert!(applied_two.top > applied_one.top);
    assert_eq!(navigator.top(&heights), applied_two.top);
    assert_eq!(
        navigator
            .navigate(
                &document,
                &heights,
                NavAction::NextHeading { max_level: 6 },
                2,
            )
            .unwrap()
            .source,
        Some(13)
    );

    let before_failure = navigator.top(&heights);
    let height_visits = heights.mutation_visits();
    let checkpoint_len = checkpoints.len();
    let hidden_source = document.source().find("[x]").unwrap();
    assert_eq!(
        refine(&document, &mut heights, &mut checkpoints, hidden_source, 40),
        RefineProgress::NotFound
    );
    assert_eq!(navigator.top(&heights), before_failure);
    assert_eq!(heights.mutation_visits(), height_visits);
    assert_eq!(checkpoints.len(), checkpoint_len);
}

#[test]
fn tiny_budget_resumes_source_refinement() {
    let source = format!(
        "{}## Destination\n\n{}",
        "paragraph text\n\n".repeat(80),
        "tail\n".repeat(20)
    );
    let document = parse(&source);
    let target = document.source().find("Destination").unwrap();
    let mut heights = HeightIndex::default();
    heights.rebuild(&document).unwrap();
    let mut checkpoints = mdtui_tui::render::checkpoint::RenderCheckpointStore::new(&document);
    let mut refiner =
        SourceRefiner::new(&document, &heights, &checkpoints, target, config(40)).unwrap();
    let mut retry = false;
    let mut result = None;
    for _ in 0..20_000 {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            tiles: 1,
            objects: 1,
            bytes: 8,
            recovery: 1,
            ..WorkLimit::default()
        });
        match refiner
            .advance(&document, &mut heights, &mut checkpoints, &mut budget)
            .unwrap()
        {
            RefineProgress::PendingScan | RefineProgress::BudgetExhausted => retry = true,
            progress @ RefineProgress::Ready(_) => {
                result = Some(progress);
                break;
            }
            RefineProgress::NotFound => panic!("destination was not rendered"),
        }
    }
    assert!(retry);
    let target = ready(result.expect("refinement settled"));
    assert_eq!(
        target.source,
        document.source().find("Destination").unwrap()
    );
    assert_row_round_trip(target, &heights);
}

#[test]
fn large_document_refinement_plans_only_a_bounded_window() {
    let source = format!(
        "{}## Destination\n\n{}",
        "paragraph text\n\n".repeat(2_000),
        "tail paragraph\n\n".repeat(2_000)
    );
    let document = parse(&source);
    let target_source = document.source().find("Destination").unwrap();
    let target_tile = document.tile_at(target_source).unwrap();
    assert!(target_tile.0 > 32);
    let mut heights = HeightIndex::default();
    heights.rebuild(&document).unwrap();
    let mut checkpoints = mdtui_tui::render::checkpoint::RenderCheckpointStore::new(&document);
    let work_before = document.work();
    let mut refiner =
        SourceRefiner::new(&document, &heights, &checkpoints, target_source, config(40)).unwrap();
    let target = loop {
        match refiner
            .advance(&document, &mut heights, &mut checkpoints, &mut unlimited())
            .unwrap()
        {
            RefineProgress::Ready(target) => break target,
            RefineProgress::PendingScan | RefineProgress::BudgetExhausted => {}
            RefineProgress::NotFound => panic!("destination was not rendered"),
        }
    };
    assert_eq!(document.work(), work_before);
    assert!(target.plan_start.0 > 0);
    assert!(target.plan_end.0 - target.plan_start.0 <= config(40).max_tiles);
    assert!(target.plan_end.0 < document.tile_count());
    assert_eq!(target.status, RefineStatus::Approximate);
    assert_row_round_trip(target, &heights);
}

#[test]
fn append_makes_source_refinement_stale() {
    let mut document = Document::new();
    document.append("alpha\n").unwrap();
    let mut heights = HeightIndex::default();
    heights.rebuild(&document).unwrap();
    let mut checkpoints = mdtui_tui::render::checkpoint::RenderCheckpointStore::new(&document);
    let mut refiner = SourceRefiner::new(&document, &heights, &checkpoints, 0, config(40)).unwrap();
    document.append("beta\n").unwrap();
    assert!(!refiner.matches(&document));
    assert_eq!(
        refiner.advance(&document, &mut heights, &mut checkpoints, &mut unlimited()),
        Err(RefineError::Stale)
    );
}
