use mdtui_tui::render::first_frame::{FirstFrame, FirstFrameConfig, FirstFrameStatus};
use mdtui_tui::render::viewport::{
    ViewportConfig, ViewportError, ViewportFrame, ViewportFrameStatus, ViewportState,
};
use mdtui3::{Budget, Document, HeightState, TILE_BYTES, TileId, WorkLimit};

fn first_frame(document: &Document, width: u16) -> FirstFrame {
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
        let step = first
            .advance(document, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap();
        if matches!(
            step.status,
            FirstFrameStatus::Covered | FirstFrameStatus::EndOfDocument
        ) {
            return first;
        }
    }
    panic!("first frame did not settle");
}

fn config(width: u16, rows: u16) -> ViewportConfig {
    ViewportConfig {
        width,
        rows,
        overscan_rows: 2,
        max_tiles: 16,
        max_replay_bytes: 256 * 1024,
        row_slack: 32,
    }
}

fn ready_frame(state: &mut ViewportState, document: &Document) -> ViewportFrame {
    for _ in 0..10_000 {
        let frame = state
            .frame(document, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap();
        if matches!(
            frame.status,
            ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument
        ) {
            return frame;
        }
    }
    panic!("viewport frame did not settle");
}

fn frame_text(frame: &ViewportFrame) -> String {
    frame
        .lines
        .iter()
        .flat_map(|line| line.cells.iter())
        .map(|cell| cell.text.collect_text())
        .collect()
}

fn append(document: &mut Document, text: &str) -> mdtui3::Update {
    document.append(text).unwrap()
}

#[test]
fn sequential_updates_keep_top_source_and_produce_new_revision_frames() {
    let mut document = Document::new();
    document.enable_display_shape_effects().unwrap();
    let mut state = ViewportState::new(&document, config(40, 4)).unwrap();

    let first = append(&mut document, "first\n");
    let applied = state.apply_update(&document, &first).unwrap();
    assert_eq!((applied.previous_revision, applied.revision), (0, 1));
    assert!(applied.layout_changed);
    assert!(!applied.discarded_render);
    assert_eq!(state.top_source(&document), Some(0));
    let frame = ready_frame(&mut state, &document);
    assert_eq!(frame.revision, 1);
    assert!(frame.matches(&document));
    assert!(frame_text(&frame).contains("first"));

    let second = append(&mut document, "second\n");
    let applied = state.apply_update(&document, &second).unwrap();
    assert_eq!((applied.previous_revision, applied.revision), (1, 2));
    assert_eq!(state.top_source(&document), Some(0));
    let frame = ready_frame(&mut state, &document);
    assert_eq!(frame.revision, 2);
    assert!(frame.matches(&document));
    assert!(frame_text(&frame).contains("first"));
}

#[test]
fn append_after_the_top_tile_preserves_the_exact_anchor() {
    let mut document = Document::new();
    document.enable_display_shape_effects().unwrap();
    append(&mut document, &format!("head\n\n{}\n", "body ".repeat(300)));
    let first = first_frame(&document, 40);
    let mut state = ViewportState::from_first_frame(&document, first, config(40, 6)).unwrap();
    let _ = ready_frame(&mut state, &document);
    let anchor = state.top_anchor().unwrap();
    let source = state.top_source(&document).unwrap();

    let update = append(&mut document, &format!("tail\n{}", "more ".repeat(100)));
    let applied = state.apply_update(&document, &update).unwrap();
    assert!(applied.layout_changed);
    assert!(!applied.top_reanchored);
    assert_eq!(state.top_anchor(), Some(anchor));
    assert_eq!(state.top_source(&document), Some(source));
    let frame = ready_frame(&mut state, &document);
    assert_eq!(frame.revision, document.revision());
    assert!(frame_text(&frame).contains("head"));
    assert_eq!(state.top_source(&document), Some(source));
}

#[test]
fn append_inside_the_top_tile_keeps_the_anchor_and_source() {
    let mut document = Document::new();
    document.enable_display_shape_effects().unwrap();
    append(&mut document, "alpha\n");
    let mut state = ViewportState::new(&document, config(40, 4)).unwrap();
    let _ = ready_frame(&mut state, &document);
    let source = state.top_source(&document).unwrap();
    assert_eq!(source, 0);

    let update = append(&mut document, "beta\n");
    let applied = state.apply_update(&document, &update).unwrap();
    assert_eq!(applied.earliest_changed_tile, Some(TileId(0)));
    assert!(!applied.top_reanchored);
    assert_eq!(state.top_source(&document), Some(source));
    assert_eq!(state.top_anchor().unwrap().tile, TileId(0));
    let frame = ready_frame(&mut state, &document);
    assert_eq!(frame.revision, 2);
    assert!(frame_text(&frame).starts_with("alpha"));
    assert_eq!(state.top_source(&document), Some(source));
}

#[test]
fn pending_frame_is_stale_after_append_and_discarded_by_apply() {
    let mut document = Document::new();
    document.enable_display_shape_effects().unwrap();
    append(
        &mut document,
        &format!("{}\n", "paragraph text ".repeat(100)),
    );
    let mut state = ViewportState::new(&document, config(40, 5)).unwrap();
    assert!(
        state
            .initialize(&document, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap()
    );
    let pending = state
        .frame(
            &document,
            &mut Budget::new(WorkLimit {
                queries: 1,
                tiles: 1,
                objects: 1,
                bytes: 8,
                ..WorkLimit::default()
            }),
        )
        .unwrap();
    assert!(matches!(
        pending.status,
        ViewportFrameStatus::PendingScan | ViewportFrameStatus::BudgetExhausted
    ));
    assert!(state.has_pending_frame());
    assert_eq!(pending.revision, 1);
    let mut foreign = Document::new();
    foreign.enable_display_shape_effects().unwrap();
    foreign.append("foreign\n").unwrap();
    assert!(!pending.matches(&foreign));

    let update = append(&mut document, "appended tail\n");
    assert!(!pending.matches(&document));
    assert_eq!(
        state.frame(&document, &mut Budget::new(WorkLimit::unlimited())),
        Err(ViewportError::Stale)
    );
    let applied = state.apply_update(&document, &update).unwrap();
    assert!(applied.discarded_render);
    assert!(!state.has_pending_frame());
    let frame = ready_frame(&mut state, &document);
    assert_eq!(frame.revision, document.revision());
    assert!(frame.matches(&document));
}

#[test]
fn empty_append_and_repeated_finish_do_not_discard_or_invalidate() {
    let mut document = Document::new();
    document.enable_display_shape_effects().unwrap();
    append(&mut document, &format!("{}\n", "line\n\n".repeat(400)));
    let mut state = ViewportState::new(&document, config(40, 5)).unwrap();
    let _ = state
        .frame(
            &document,
            &mut Budget::new(WorkLimit {
                queries: 1,
                tiles: 1,
                objects: 1,
                bytes: 8,
                ..WorkLimit::default()
            }),
        )
        .unwrap();
    assert!(state.has_pending_frame());

    let empty = append(&mut document, "");
    let applied = state.apply_update(&document, &empty).unwrap();
    assert_eq!(applied.previous_revision, applied.revision);
    assert!(!applied.layout_changed);
    assert!(!applied.discarded_render);
    assert!(state.has_pending_frame());
    let frame = ready_frame(&mut state, &document);
    assert!(frame.matches(&document));

    let _ = state
        .frame(
            &document,
            &mut Budget::new(WorkLimit {
                queries: 1,
                tiles: 1,
                objects: 1,
                bytes: 8,
                ..WorkLimit::default()
            }),
        )
        .unwrap();
    assert!(state.has_pending_frame());
    let eof = document.finish();
    let applied = state.apply_update(&document, &eof).unwrap();
    assert_eq!(applied.revision, document.revision());
    assert!(applied.discarded_render);
    assert!(!state.has_pending_frame());
    let _ = state
        .frame(
            &document,
            &mut Budget::new(WorkLimit {
                queries: 1,
                tiles: 1,
                objects: 1,
                bytes: 8,
                ..WorkLimit::default()
            }),
        )
        .unwrap();
    assert!(state.has_pending_frame());
    let visits = state.heights().mutation_visits();
    let checkpoints = state.checkpoints().len();
    let repeated = document.finish();
    assert_eq!(repeated.base_revision, repeated.revision);
    let applied = state.apply_update(&document, &repeated).unwrap();
    assert!(!applied.layout_changed);
    assert!(!applied.discarded_render);
    assert!(state.has_pending_frame());
    assert_eq!(state.heights().mutation_visits(), visits);
    assert_eq!(state.checkpoints().len(), checkpoints);
}

#[test]
fn offscreen_invalidated_tiles_are_remeasured_only_when_scrolled_back() {
    let mut document = Document::new();
    document.enable_display_shape_effects().unwrap();
    append(
        &mut document,
        &format!("intro\n\n{}", "a".repeat(TILE_BYTES * 5)),
    );
    let first = first_frame(&document, 20);
    let mut state = ViewportState::from_first_frame(&document, first, config(20, 8)).unwrap();
    let old_last = TileId(document.tile_count() - 1);
    assert!(matches!(
        state.heights().height_state(old_last),
        Some(HeightState::Measured | HeightState::Local)
    ));

    let update = append(&mut document, &"b".repeat(TILE_BYTES * 2));
    assert!(state.checkpoints().exact(old_last).is_some());
    let applied = state.apply_update(&document, &update).unwrap();
    let changed = applied.earliest_changed_tile.unwrap();
    assert!(changed <= old_last);
    assert!(state.checkpoints().exact(changed).is_none());
    if changed.0 > 0 {
        assert!(matches!(
            state.heights().height_state(TileId(changed.0 - 1)),
            Some(HeightState::Measured | HeightState::Local)
        ));
    }
    assert!(matches!(
        state.heights().height_state(changed),
        Some(HeightState::Expired | HeightState::Estimated)
    ));

    state.end(&document).unwrap();
    let frame = ready_frame(&mut state, &document);
    assert_eq!(frame.revision, document.revision());
    assert!(frame.stats.planned_tiles <= state.config().max_tiles);
    assert!(frame.stats.rendered_tiles <= state.config().max_tiles);
    assert!(frame_text(&frame).contains('b'));
    assert!(matches!(
        state.heights().height_state(changed),
        Some(HeightState::Measured | HeightState::Local)
    ));
    assert!(frame.stats.rendered_tiles > 0);
}

#[test]
fn top_at_tail_boundary_survives_append_without_jumping() {
    let mut document = Document::new();
    document.enable_display_shape_effects().unwrap();
    append(
        &mut document,
        &format!("{}\n", "tail text ".repeat(TILE_BYTES)),
    );
    let first = first_frame(&document, 30);
    let mut state = ViewportState::from_first_frame(&document, first, config(30, 5)).unwrap();
    state.end(&document).unwrap();
    let _ = ready_frame(&mut state, &document);
    let source = state.top_source(&document).unwrap();
    let old_tile = state.top_anchor().unwrap().tile;

    let update = append(&mut document, "continuation\n");
    let applied = state.apply_update(&document, &update).unwrap();
    assert_eq!(state.top_source(&document), Some(source));
    assert!(applied.top_reanchored || state.top_anchor().unwrap().tile >= old_tile);
    let frame = ready_frame(&mut state, &document);
    assert_eq!(frame.revision, document.revision());
    assert_eq!(state.top_source(&document), Some(source));
}

#[test]
fn out_of_order_and_foreign_updates_are_rejected_without_rebinding() {
    let mut document = Document::new();
    document.enable_display_shape_effects().unwrap();
    append(&mut document, "base\n");
    let mut state = ViewportState::new(&document, config(40, 4)).unwrap();
    let first = append(&mut document, "one\n");
    let second = append(&mut document, "two\n");
    let top = state.top_anchor();
    let source = state.top_source(&document);
    let visits = state.heights().mutation_visits();
    let checkpoints = state.checkpoints().len();
    assert_eq!(
        state.apply_update(&document, &second),
        Err(ViewportError::OutOfOrderUpdate)
    );
    assert!(!state.matches(&document));

    let mut foreign = Document::new();
    foreign.enable_display_shape_effects().unwrap();
    let foreign_update = append(&mut foreign, "foreign\n");
    assert_eq!(
        state.apply_update(&foreign, &foreign_update),
        Err(ViewportError::WrongDocument)
    );
    assert_eq!(state.top_anchor(), top);
    assert_eq!(state.top_source(&document), source);
    assert_eq!(state.heights().mutation_visits(), visits);
    assert_eq!(state.checkpoints().len(), checkpoints);

    let mut fresh = ViewportState::new(&document, config(40, 4)).unwrap();
    assert_eq!(
        fresh.apply_update(&document, &first),
        Err(ViewportError::OutOfOrderUpdate)
    );
}
