use std::io;

use mdtui_tui::cli::LinkMode;
use mdtui_tui::history::HistoryPosition;
use mdtui_tui::render::frame::HistoryRestoreStatus;
use mdtui_tui::render::frame::{FrameGeometry, FrameMetrics, FrameRuntime};
use mdtui_tui::stream::FRAME_WORK;
use mdtui3::{Budget, TILE_BYTES, WorkLimit};

const WIDTH: u16 = 80;
const HEIGHT: u16 = 24;
const SEGMENT_LIMIT: usize = 4_096;
const EVENT_LIMIT: usize = 4_096;
const LAYOUT_ROWS_LIMIT: u64 = 8_192;
const PREFIX_UNIT: &str = "[hidden]: /url\n";
const PREFIX_BASE_BYTES: usize = 15 * 1024;

fn geometry() -> FrameGeometry {
    FrameGeometry {
        rows: HEIGHT,
        cols: WIDTH,
        content_width: WIDTH,
    }
}

fn work_within(used: WorkLimit, limit: WorkLimit) -> bool {
    used.queries <= limit.queries
        && used.tiles <= limit.tiles
        && used.bytes <= limit.bytes
        && used.objects <= limit.objects
        && used.recovery <= limit.recovery
        && used.dependencies <= limit.dependencies
}

fn add_work(total: &mut WorkLimit, used: WorkLimit) {
    total.queries = total.queries.saturating_add(used.queries);
    total.tiles = total.tiles.saturating_add(used.tiles);
    total.bytes = total.bytes.saturating_add(used.bytes);
    total.objects = total.objects.saturating_add(used.objects);
    total.recovery = total.recovery.saturating_add(used.recovery);
    total.dependencies = total.dependencies.saturating_add(used.dependencies);
}

#[derive(Debug, Clone, Copy)]
struct FrameCost {
    steps: usize,
    total: WorkLimit,
    last: FrameMetrics,
}

fn frame_cost(runtime: &mut FrameRuntime, label: &str) -> FrameCost {
    let mut total = WorkLimit::default();
    for step in 1..=64 {
        let mut budget = Budget::new(FRAME_WORK);
        let commit = runtime
            .frame_to(&mut io::sink(), &mut budget)
            .unwrap_or_else(|error| panic!("{label}: {error}"));
        add_work(&mut total, budget.used());
        if commit.committed {
            return FrameCost {
                steps: step,
                total,
                last: commit.metrics,
            };
        }
    }
    panic!("scroll frame did not commit");
}

fn assert_frame_cost(cost: FrameCost) {
    assert!(cost.steps <= 64);
    assert!(work_within(cost.last.layout_budget, FRAME_WORK));
    assert!(cost.total.queries <= FRAME_WORK.queries.saturating_mul(cost.steps));
    assert!(cost.total.tiles <= FRAME_WORK.tiles.saturating_mul(cost.steps));
    assert!(cost.total.bytes <= FRAME_WORK.bytes.saturating_mul(cost.steps));
    assert!(cost.last.planned_tiles <= FRAME_WORK.tiles);
    assert!(cost.last.visible_tiles <= cost.last.planned_tiles);
    assert!(cost.last.recovery_tiles <= cost.last.planned_tiles);
    assert!(cost.last.diff_rows <= usize::from(HEIGHT));
    assert!(cost.last.layout.segments_consumed <= SEGMENT_LIMIT);
    assert!(cost.last.layout.events_consumed <= EVENT_LIMIT);
    assert!(cost.last.layout.rows <= LAYOUT_ROWS_LIMIT);
    assert_eq!(cost.last.layout.document_queries_in_callbacks, 0);
}

fn heading_source(count: usize) -> String {
    let mut source = String::from("# Section 00000\n\nbody 00000\n\n");
    for index in 1..count {
        source.push_str(&format!(
            "# Section {index:05}\n\nbody paragraph {index:05}\n\n"
        ));
    }
    source
}

fn runtime(source: &str) -> FrameRuntime {
    FrameRuntime::from_finished_source(source, geometry(), LinkMode::Default).unwrap()
}

#[test]
fn page_down_keeps_each_visible_frame_budget_bounded() {
    let source = heading_source(10_000);
    let mut runtime = runtime(&source);
    let first = frame_cost(&mut runtime, "page initial");
    assert_frame_cost(first);
    let mut total = first.total;

    for _ in 0..12 {
        let before = runtime.viewport().top_anchor();
        assert!(runtime.scroll_down(u64::from(HEIGHT)).expect("page down"));
        let cost = frame_cost(&mut runtime, "page down");
        assert_frame_cost(cost);
        add_work(&mut total, cost.total);
        assert_ne!(runtime.viewport().top_anchor(), before);
    }

    assert!(runtime.top_source().is_some());
    println!(
        "pagedown input_bytes={} frames=13 total_work={:?} last={:?}",
        runtime.document().work().input_bytes,
        total,
        runtime.viewport().top_anchor()
    );
}

#[test]
fn deep_heading_jump_and_source_restore_are_bounded() {
    let source = heading_source(10_000);
    let mut runtime = runtime(&source);
    let first = frame_cost(&mut runtime, "deep initial");
    assert_frame_cost(first);
    let first_position = runtime.navigation_position().expect("initial position");

    assert!(runtime.set_sidebar_visible(true).unwrap().visible);
    assert!(runtime.sidebar_selection_end());
    let mut jump_budget = Budget::new(FRAME_WORK);
    assert!(
        runtime
            .activate_sidebar_selection(&mut jump_budget)
            .expect("deep heading jump")
    );
    assert!(work_within(jump_budget.used(), FRAME_WORK));
    let last = source.rfind("# Section").expect("last heading");
    assert_eq!(runtime.top_source(), Some(last));
    assert_eq!(
        runtime.body_current_heading(),
        Some(mdtui3::HeadingId(9_999))
    );

    let jump_frame = frame_cost(&mut runtime, "deep jump frame");
    assert_frame_cost(jump_frame);
    let last_position = runtime.navigation_position().expect("last position");

    let mut back_budget = Budget::new(FRAME_WORK);
    let mut restored_back = runtime
        .restore_navigation_position(first_position, &mut back_budget)
        .expect("restore first");
    assert!(work_within(back_budget.used(), FRAME_WORK));
    for _ in 0..32 {
        if matches!(restored_back, HistoryRestoreStatus::Ready { .. }) {
            break;
        }
        assert_eq!(restored_back, HistoryRestoreStatus::BudgetExhausted);
        back_budget = Budget::new(FRAME_WORK);
        restored_back = runtime
            .restore_navigation_position(first_position, &mut back_budget)
            .expect("resume restore first");
        assert!(work_within(back_budget.used(), FRAME_WORK));
    }
    assert!(matches!(restored_back, HistoryRestoreStatus::Ready { .. }));
    assert_eq!(runtime.top_source(), Some(first_position.source));
    let back_frame = frame_cost(&mut runtime, "deep restore frame");
    assert_frame_cost(back_frame);

    let mut forward_budget = Budget::new(FRAME_WORK);
    let mut restored_forward = runtime
        .restore_navigation_position(last_position, &mut forward_budget)
        .expect("restore last");
    assert!(work_within(forward_budget.used(), FRAME_WORK));
    for _ in 0..32 {
        if matches!(restored_forward, HistoryRestoreStatus::Ready { .. }) {
            break;
        }
        assert_eq!(restored_forward, HistoryRestoreStatus::BudgetExhausted);
        forward_budget = Budget::new(FRAME_WORK);
        restored_forward = runtime
            .restore_navigation_position(last_position, &mut forward_budget)
            .expect("resume restore last");
        assert!(work_within(forward_budget.used(), FRAME_WORK));
    }
    assert!(matches!(
        restored_forward,
        HistoryRestoreStatus::Ready { .. }
    ));
    assert_eq!(runtime.top_source(), Some(last));
    let forward_frame = frame_cost(&mut runtime, "deep forward frame");
    assert_frame_cost(forward_frame);

    println!(
        "deep-jump input_bytes={} jump_used={:?} back_used={:?} forward_used={:?} jump={:?} back={:?} forward={:?}",
        runtime.document().work().input_bytes,
        jump_budget.used(),
        back_budget.used(),
        forward_budget.used(),
        jump_frame.last,
        back_frame.last,
        forward_frame.last
    );
}

#[test]
fn prefix_before_viewport_does_not_grow_scroll_layout() {
    let measurements = [128, 256, 512].map(measure_prefix_scroll);
    for pair in measurements.windows(2) {
        assert!(pair[1].0 > pair[0].0);
    }
    let baseline = &measurements[0].1;
    for (input_bytes, cost) in &measurements[1..] {
        assert!(*input_bytes > measurements[0].0);
        assert_eq!(cost.last.layout, baseline.last.layout);
        assert_eq!(cost.last.layout_budget, baseline.last.layout_budget);
        assert_eq!(cost.last.planned_tiles, baseline.last.planned_tiles);
        assert_eq!(cost.last.visible_tiles, baseline.last.visible_tiles);
        assert_eq!(cost.last.recovery_tiles, baseline.last.recovery_tiles);
        assert_eq!(cost.last.diff_rows, baseline.last.diff_rows);
    }
}

fn measure_prefix_scroll(multiplier: usize) -> (u64, FrameCost) {
    let mut source = PREFIX_UNIT.repeat(PREFIX_BASE_BYTES / PREFIX_UNIT.len() * multiplier);
    source.push_str("\n# Target after prefix\n\n");
    for index in 0..200 {
        source.push_str(&format!("visible paragraph {index:03}\n\n"));
    }
    let input_bytes = source.len() as u64;
    let mut runtime = runtime(&source);
    assert!(runtime.set_sidebar_visible(true).unwrap().visible);
    assert!(runtime.sidebar_selection_end());
    let mut jump = Budget::new(FRAME_WORK);
    assert!(
        runtime
            .activate_sidebar_selection(&mut jump)
            .expect("prefix target jump")
    );
    let jump_frame = frame_cost(&mut runtime, "prefix jump frame");
    assert_frame_cost(jump_frame);
    assert!(runtime.scroll_down(1).expect("prefix scroll"));
    let cost = frame_cost(&mut runtime, "prefix scroll frame");
    assert_frame_cost(cost);
    println!(
        "prefix-scroll x{multiplier} input_bytes={input_bytes} metrics={:?}",
        cost.last
    );
    (input_bytes, cost)
}

#[test]
fn cross_tile_long_line_replay_reports_tiles_and_keeps_line_start_source() {
    let source = format!("{}\n\n", "x".repeat(TILE_BYTES * 12));
    let mut runtime = runtime(&source);
    let _ = frame_cost(&mut runtime, "long-line initial");
    let target_source = source.len() / 2;
    let mut restore_budget = Budget::new(FRAME_WORK);
    let restored = runtime
        .restore_navigation_position(
            HistoryPosition {
                source: target_source,
                row_offset: 0,
            },
            &mut restore_budget,
        )
        .expect("long-line restore");
    assert!(matches!(restored, HistoryRestoreStatus::Ready { .. }));
    assert!(work_within(restore_budget.used(), FRAME_WORK));
    let _ = frame_cost(&mut runtime, "long-line restore frame");
    let anchor = runtime.viewport().top_anchor().expect("long-line anchor");
    let line_start = runtime.top_source().expect("line start");
    assert!(line_start <= target_source);
    assert!(line_start < runtime.document().tile(anchor.tile).unwrap().start);
    runtime
        .viewport_mut()
        .checkpoints_mut()
        .invalidate(anchor.tile.0..anchor.tile.0 + 1)
        .expect("invalidate target checkpoint");

    println!(
        "long-line after restore top_position={:?} anchor={anchor:?} checkpoints={} recovery={:?}",
        runtime.viewport().top_position(),
        runtime.viewport().checkpoints().len(),
        runtime.viewport().checkpoints().plan_recovery(anchor.tile)
    );
    let cost = frame_cost(&mut runtime, "long-line replay frame");
    assert_frame_cost(cost);
    assert!(cost.last.recovery_tiles > 0);
    let replay_anchor = runtime.viewport().top_anchor().expect("replay anchor");
    let replay_line_start = runtime.top_source().expect("replay line start");
    assert!(replay_line_start <= target_source);
    assert!(replay_line_start < runtime.document().tile(replay_anchor.tile).unwrap().start);
    println!(
        "long-line target={target_source} anchor={anchor:?} line_start={line_start} replay_anchor={replay_anchor:?} replay_line_start={replay_line_start} restore_used={:?} frame={:?}",
        restore_budget.used(),
        cost.last
    );
}

#[test]
fn core_update_invalidation_is_reported_separately_from_visible_frame_work() {
    let source = heading_source(1_000);
    let mut runtime = FrameRuntime::new(geometry(), LinkMode::Default).unwrap();
    let update = runtime
        .append_bytes(source.as_bytes())
        .expect("append scale source")
        .expect("nonempty update");
    let (document, viewport) = runtime.document_and_viewport_mut();
    let applied = viewport
        .apply_update(document, &update)
        .expect("apply scale update");
    let earliest = applied.earliest_changed_tile.expect("changed tile");
    let core_span = document.tile_count().saturating_sub(earliest.0);
    let input_bytes = document.work().input_bytes;
    assert!(core_span > 100);
    assert_eq!(input_bytes, source.len() as u64);

    let cost = frame_cost(&mut runtime, "post-update frame");
    assert_frame_cost(cost);
    assert!(cost.last.planned_tiles <= FRAME_WORK.tiles);
    println!(
        "core-update input_bytes={input_bytes} earliest={earliest:?} core_span_tiles={core_span} frame={:?}",
        cost.last
    );
}

#[test]
fn eof_anchor_index_cost_is_one_time_not_a_scroll_frame_bound() {
    let source = heading_source(1_000);
    let mut runtime = runtime(&source);
    assert!(runtime.document().is_finished());
    let mut total = WorkLimit::default();
    let mut steps = 0;
    for _ in 0..100_000 {
        let mut budget = Budget::new(FRAME_WORK);
        let status = runtime
            .advance_anchor_index(&mut budget)
            .expect("anchor index step");
        add_work(&mut total, budget.used());
        steps += 1;
        if matches!(status, mdtui_tui::fragment::AnchorBuildStatus::Complete) {
            break;
        }
    }
    assert!(runtime.anchor_index_complete());
    assert!(steps >= 1);
    assert!(runtime.anchor_indexed_headings().unwrap_or(0) >= 1_000);
    println!(
        "eof-anchor steps={steps} work={total:?} indexed={:?}",
        runtime.anchor_indexed_headings()
    );
}
