use mdtui_tui::render::checkpoint::RenderCheckpoint;
use mdtui_tui::render::first_frame::{FirstFrame, FirstFrameConfig, FirstFrameStatus};
use mdtui_tui::render::viewport::{
    TopPosition, ViewportConfig, ViewportFrame, ViewportFrameStatus, ViewportState,
};
use mdtui3::{Budget, Document, HeightState, LayoutError, TILE_BYTES, TileId, WorkLimit};

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

fn config(width: u16, rows: u16) -> ViewportConfig {
    ViewportConfig {
        width,
        rows,
        overscan_rows: 2,
        max_tiles: 64,
        max_replay_bytes: 256 * 1024,
        row_slack: 32,
    }
}

fn state_with_full_layout(document: &Document, width: u16, rows: u16) -> ViewportState {
    ViewportState::from_first_frame(
        document,
        full_first_frame(document, width),
        config(width, rows),
    )
    .unwrap()
}

fn frame_until_ready(state: &mut ViewportState, document: &Document) -> ViewportFrame {
    for _ in 0..10_000 {
        let frame = state.frame(document, &mut unlimited()).unwrap();
        if matches!(
            frame.status,
            ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument
        ) {
            return frame;
        }
    }
    panic!("viewport frame did not settle");
}

fn line_text(line: &mdtui_tui::render::sink::SinkLine) -> String {
    line.cells
        .iter()
        .map(|cell| cell.text.collect_text())
        .collect()
}

#[test]
fn line_page_home_end_and_boundaries_are_programmatic() {
    let document = parse(
        &(0..30)
            .map(|index| format!("line {index}\n\n"))
            .collect::<String>(),
    );
    let mut state = state_with_full_layout(&document, 40, 5);

    assert_eq!(state.top_row(), Some(0));
    assert!(!state.up(&document).unwrap());
    assert!(state.down(&document).unwrap());
    assert_eq!(state.top_row(), Some(1));
    assert!(state.page_down(&document).unwrap());
    assert_eq!(state.top_row(), Some(6));
    assert!(state.page_up(&document).unwrap());
    assert_eq!(state.top_row(), Some(1));
    assert!(state.home(&document).unwrap());
    assert_eq!(state.top_row(), Some(0));
    assert!(state.end(&document).unwrap());
    let bottom = state.top_row().unwrap();
    assert!(bottom > 0);
    assert!(!state.down(&document).unwrap());
    assert!(!state.end(&document).unwrap());
    assert!(state.up(&document).unwrap());
    assert_eq!(state.top_row(), Some(bottom - 1));
}

#[test]
fn long_line_top_position_distinguishes_exact_replay_and_approximate() {
    let source = format!("{}\n", "a".repeat(TILE_BYTES * 2 + 31));
    let document = parse(&source);
    let mut state = state_with_full_layout(&document, 40, 6);
    state.scroll_down(&document, 3).unwrap();
    let anchor = state.top_anchor().unwrap();
    assert!(anchor.tile.0 > 0);

    let exact = frame_until_ready(&mut state, &document);
    assert!(matches!(exact.top, TopPosition::Exact { .. }));
    assert!(!exact.lines.is_empty());
    assert_lines_start_before_owner(&exact, &document, anchor.tile);
    assert_eq!(state.top_anchor().unwrap().tile, anchor.tile);

    state
        .checkpoints_mut()
        .invalidate(anchor.tile.0..anchor.tile.0 + 1)
        .unwrap();
    let replay = frame_until_ready(&mut state, &document);
    let TopPosition::ReplayFrom { from, target, .. } = replay.top else {
        panic!("expected replay top, got {:?}", replay.top);
    };
    assert!(target >= anchor.tile);
    assert_eq!(replay.stats.replay_tiles, target.0 - from.0 + 1);
    assert_lines_start_before_owner(&replay, &document, anchor.tile);

    state.checkpoints_mut().clear();
    let approximate = frame_until_ready(&mut state, &document);
    assert!(matches!(
        approximate.top,
        TopPosition::ApproximateLineStart { .. }
    ));
    assert!(!approximate.lines.is_empty());
    assert_lines_start_before_owner(&approximate, &document, anchor.tile);
}

fn assert_lines_start_before_owner(frame: &ViewportFrame, document: &Document, owner: TileId) {
    let owner_start = document.tile(owner).unwrap().start;
    assert!(frame.lines.iter().all(|line| {
        line.source
            .is_some_and(|source| source.source.start < owner_start)
    }));
}

#[test]
fn render_plan_stays_near_the_viewport_for_a_large_document() {
    let source = (0..1_000)
        .map(|index| format!("paragraph {index}\n\n"))
        .collect::<String>();
    let document = parse(&source);
    let work_before = document.work();
    let mut viewport_config = config(80, 10);
    viewport_config.max_tiles = 16;
    let mut state = ViewportState::new(&document, viewport_config).unwrap();
    state.scroll_down(&document, 300).unwrap();
    let frame = frame_until_ready(&mut state, &document);
    assert!(frame.stats.planned_tiles <= viewport_config.max_tiles);
    assert!(frame.stats.visible_tiles <= viewport_config.max_tiles);
    assert!(frame.stats.rendered_tiles <= viewport_config.max_tiles);
    assert!(!frame.lines.is_empty());
    assert_eq!(document.work(), work_before);
}

#[test]
fn top_anchor_survives_height_measurement_and_overscan_is_used() {
    let source = (0..200)
        .map(|index| format!("heading {index}\n\n"))
        .collect::<String>();
    let document = parse(&source);
    let mut viewport_config = config(40, 8);
    viewport_config.overscan_rows = 4;
    let mut state = ViewportState::new(&document, viewport_config).unwrap();
    state.scroll_down(&document, 20).unwrap();
    let anchor = state.top_anchor().unwrap();
    let frame = frame_until_ready(&mut state, &document);
    assert_eq!(state.top_anchor().unwrap().tile, anchor.tile);
    assert!(frame.stats.visible_tiles >= 1);
    assert!(frame.stats.planned_tiles >= frame.stats.visible_tiles);
    assert!(frame.stats.plan_start.unwrap() < anchor.tile);
}

#[test]
fn viewport_row_resize_keeps_the_top_source_anchor() {
    let document = parse(
        &(0..100)
            .map(|index| format!("line {index}\n\n"))
            .collect::<String>(),
    );
    let mut state = state_with_full_layout(&document, 40, 8);
    state.scroll_down(&document, 15).unwrap();
    let anchor = state.top_anchor().unwrap();
    state.set_viewport_rows(3).unwrap();
    assert_eq!(state.top_anchor(), Some(anchor));
    let frame = frame_until_ready(&mut state, &document);
    assert_eq!(state.top_anchor().unwrap().tile, anchor.tile);
    assert!(frame.rows <= 3);
}

#[test]
fn content_width_change_clears_width_tokens_and_preserves_nonzero_top_source() {
    let document = parse(
        &(0..300)
            .map(|index| format!("paragraph {index} abcdefghijklmnopqrstuvwxyz\n\n"))
            .collect::<String>(),
    );
    let mut state = state_with_full_layout(&document, 40, 8);
    state.scroll_down(&document, 40).unwrap();
    let top_source = state.top_source(&document).expect("top source");
    assert!(top_source > 0);
    let tile = state.top_anchor().unwrap().tile;
    let height_ticket = state.heights().ticket(tile).unwrap();
    state
        .checkpoints_mut()
        .insert(tile, RenderCheckpoint::line_start())
        .unwrap();
    let checkpoint_token = state.checkpoints().token(tile).unwrap();

    let change = state.set_content_width(&document, 20).unwrap();
    assert!(change.changed);
    assert_eq!(change.previous_width, 40);
    assert_eq!(change.width, 20);
    assert_eq!(change.top_source, Some(top_source));
    assert_eq!(change.estimated_rows, 1);
    assert_eq!(state.config().width, 20);
    assert_eq!(state.top_source(&document), Some(top_source));
    assert_eq!(
        state.heights().height_state(tile),
        Some(HeightState::Estimated)
    );
    assert_eq!(
        state.heights_mut().measure(height_ticket, 1),
        Err(LayoutError::StaleMeasurement)
    );
    assert!(!state.checkpoints().validates(checkpoint_token));
    assert!(state.checkpoints().exact(tile).is_none());
    assert!(!state.has_pending_frame());

    let frame = frame_until_ready(&mut state, &document);
    assert_eq!(state.top_source(&document), Some(top_source));
    assert!(frame.matches(&document));
    assert!(!frame.lines.is_empty());
}

#[test]
fn tiny_budget_resumes_without_losing_the_viewport() {
    let document = parse("first\n\nsecond\n\nthird\n\nfourth\n");
    let mut state = state_with_full_layout(&document, 40, 3);
    state.scroll_down(&document, 2).unwrap();
    let mut saw_budget = false;
    let mut frame = None;
    let mut budget_count = 0_usize;
    let mut scan_count = 0_usize;
    for _ in 0..2_000 {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 8,
            tiles: 1,
            ..WorkLimit::default()
        });
        let candidate = state.frame(&document, &mut budget).unwrap();
        match candidate.status {
            ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument => {
                frame = Some(candidate);
                break;
            }
            ViewportFrameStatus::BudgetExhausted | ViewportFrameStatus::PendingScan => {
                saw_budget = true;
                if candidate.status == ViewportFrameStatus::BudgetExhausted {
                    budget_count += 1;
                } else {
                    scan_count += 1;
                }
            }
        }
    }
    assert!(saw_budget);
    let frame = frame.unwrap_or_else(|| {
        panic!(
            "frame completed: budget={budget_count} scan={scan_count} top={:?}",
            state.top_anchor()
        )
    });
    assert!(!frame.lines.is_empty());
    assert_eq!(
        state.top_row(),
        state.heights().row_of(state.top_anchor().unwrap())
    );
}

#[test]
fn tiny_budget_resumes_checkpoint_replay_with_recovery_units() {
    let source = format!("{}\n", "r".repeat(TILE_BYTES * 2 + 17));
    let document = parse(&source);
    let mut state = state_with_full_layout(&document, 20, 5);
    state.scroll_down(&document, 4).unwrap();
    let anchor = state.top_anchor().unwrap();
    state
        .checkpoints_mut()
        .invalidate(anchor.tile.0..anchor.tile.0 + 1)
        .unwrap();

    let mut frame = None;
    for _ in 0..4_000 {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 8,
            tiles: 1,
            recovery: 1,
            ..WorkLimit::default()
        });
        let candidate = state.frame(&document, &mut budget).unwrap();
        if matches!(
            candidate.status,
            ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument
        ) {
            frame = Some(candidate);
            break;
        }
    }
    let frame = frame.expect("replay frame completed");
    assert!(matches!(frame.top, TopPosition::ReplayFrom { .. }));
    assert!(frame.stats.replay_tiles > 0);
    assert!(!frame.lines.is_empty());
}

#[test]
fn append_marks_old_viewport_stale() {
    let mut document = Document::new();
    document.append("one\n").unwrap();
    let mut state = ViewportState::new(&document, config(40, 4)).unwrap();
    assert!(state.matches(&document));
    document.append("two\n").unwrap();
    assert!(!state.matches(&document));
    assert_eq!(
        state.frame(&document, &mut unlimited()),
        Err(mdtui_tui::render::viewport::ViewportError::Stale)
    );
}

#[test]
fn empty_document_has_an_empty_ready_frame() {
    let document = parse("");
    let mut state = ViewportState::new(&document, config(40, 4)).unwrap();
    let frame = state.frame(&document, &mut unlimited()).unwrap();
    assert_eq!(frame.status, ViewportFrameStatus::Ready);
    assert_eq!(frame.top, TopPosition::Empty);
    assert!(frame.lines.is_empty());
    assert_eq!(frame.rows, 0);
}

#[test]
fn wide_line_rows_have_line_start_sources_not_the_owner_tile() {
    let source = format!("prefix {}\n", "x".repeat(TILE_BYTES * 2));
    let document = parse(&source);
    let mut state = state_with_full_layout(&document, 20, 4);
    state.scroll_down(&document, 4).unwrap();
    let frame = frame_until_ready(&mut state, &document);
    let owner = state.top_anchor().unwrap().tile;
    let owner_start = document.tile(owner).unwrap().start;
    assert!(frame.lines.iter().all(|line| {
        line.source
            .is_some_and(|source| source.source.start < owner_start)
    }));
    assert!(frame.lines.iter().any(|line| line_text(line).contains('x')));
}
