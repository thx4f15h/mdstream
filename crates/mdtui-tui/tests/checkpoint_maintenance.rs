use std::cell::Cell as DropCell;
use std::rc::Rc;

use mdtui_tui::render::checkpoint::{
    CheckpointMaintenancePhase, RenderCheckpoint, RenderCheckpointStore,
};
use mdtui_tui::render::viewport::{
    ViewportConfig, ViewportError, ViewportFrameStatus, ViewportState,
};
use mdtui3::{
    Budget, CheckpointIndex, Completion, Document, LayoutError, TILE_BYTES, TileId, WorkLimit,
};

fn config(width: u16, rows: u16) -> ViewportConfig {
    ViewportConfig {
        width,
        rows,
        overscan_rows: 2,
        max_tiles: 256,
        max_replay_bytes: 256 * 1024,
        row_slack: 32,
    }
}

fn unlimited() -> Budget {
    Budget::new(WorkLimit::unlimited())
}

fn tiny() -> Budget {
    Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        ..WorkLimit::default()
    })
}

fn checkpoint(value: usize) -> RenderCheckpoint {
    RenderCheckpoint::line_mid(value, 0)
}

fn run_maintenance(store: &mut RenderCheckpointStore) -> Vec<mdtui3::Completion> {
    let mut completions = Vec::new();
    for _ in 0..20_000 {
        let progress = store.advance_maintenance(&mut tiny()).unwrap();
        completions.push(progress.completion);
        if progress.completion == Completion::Covered {
            return completions;
        }
    }
    panic!("checkpoint maintenance did not settle");
}

fn ready_frame(state: &mut ViewportState, document: &Document) -> bool {
    for _ in 0..10_000 {
        let frame = state.frame(document, &mut unlimited()).unwrap();
        if matches!(
            frame.status,
            ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument
        ) {
            return true;
        }
    }
    false
}

#[test]
fn store_window_invalidates_both_sides_and_compacts_payloads() {
    let document = Document::new();
    let mut store = RenderCheckpointStore::new(&document);
    for tile in 0..30 {
        store.insert(TileId(tile), checkpoint(tile)).unwrap();
    }
    let before_arena = store.arena_stats();
    store.start_maintenance(10..20, 30).unwrap();
    let progress = store.advance_maintenance(&mut unlimited()).unwrap();

    assert_eq!(progress.completion, Completion::Covered);
    assert_eq!(progress.phase, None);
    assert_eq!(progress.drain_cursor, 0);
    assert_eq!(progress.drained_payloads, 10);
    assert_eq!(progress.retired_payloads, 10);
    assert_eq!(progress.released_payloads, 20);
    assert_eq!(progress.len, 10);
    let compaction = progress.compaction.unwrap();
    assert!(compaction.retired > 0);
    assert_eq!(compaction.source.capacity_bytes, 0);
    for tile in 0..30 {
        assert_eq!(
            store.exact(TileId(tile)).is_some(),
            (10..20).contains(&tile),
            "tile {tile}"
        );
    }
    assert!(!store.maintenance_in_progress());
    assert_eq!(progress.arena, store.arena_stats());
    assert!(progress.arena.nodes <= before_arena.nodes + 2);
}

#[test]
fn tiny_budget_pauses_both_phases_blocks_writes_and_resumes() {
    let document = Document::new();
    let mut store = RenderCheckpointStore::new(&document);
    for tile in 0..100 {
        store.insert(TileId(tile), checkpoint(tile)).unwrap();
    }
    store.start_maintenance(40..60, 100).unwrap();
    assert_eq!(
        store.insert(TileId(50), checkpoint(999)),
        Err(LayoutError::OutOfOrderUpdate)
    );
    assert_eq!(
        store.apply(&document, &mdtui3::Update::default()),
        Err(LayoutError::OutOfOrderUpdate)
    );

    let mut saw_drain = false;
    let mut saw_compact = false;
    let mut cursor_seen = false;
    for _ in 0..20_000 {
        let progress = store.advance_maintenance(&mut tiny()).unwrap();
        match progress.phase {
            Some(CheckpointMaintenancePhase::Drain) => {
                saw_drain = true;
                cursor_seen |= progress.drain_cursor > 0;
            }
            Some(CheckpointMaintenancePhase::Compact) => saw_compact = true,
            None => {}
        }
        if progress.completion == Completion::Covered {
            assert_eq!(progress.drain_cursor, 0);
            assert!(!store.maintenance_in_progress());
            break;
        }
    }
    assert!(saw_drain);
    assert!(saw_compact);
    assert!(cursor_seen);
    assert_eq!(store.len(), 20);
    for tile in 40..60 {
        assert!(store.exact(TileId(tile)).is_some(), "tile {tile}");
    }
}

#[test]
fn repeated_maintenance_is_idempotent_and_budget_exhaustion_keeps_window() {
    let document = Document::new();
    let mut store = RenderCheckpointStore::new(&document);
    for tile in 0..40 {
        store.insert(TileId(tile), checkpoint(tile)).unwrap();
    }
    store.start_maintenance(15..25, 40).unwrap();
    let _ = run_maintenance(&mut store);
    let valid = (15..25)
        .map(|tile| store.exact(TileId(tile)).copied())
        .collect::<Vec<_>>();
    assert!(valid.iter().all(Option::is_some));

    store.start_maintenance(15..25, 40).unwrap();
    let progress = store.advance_maintenance(&mut unlimited()).unwrap();
    assert_eq!(progress.completion, Completion::Covered);
    assert_eq!(progress.drained_payloads, 0);
    assert_eq!(progress.retired_payloads, 0);
    assert_eq!(progress.len, 10);
    assert_eq!(
        (15..25)
            .map(|tile| store.exact(TileId(tile)).copied())
            .collect::<Vec<_>>(),
        valid
    );
}

#[test]
fn zero_full_empty_and_narrow_windows_are_safe() {
    let document = Document::new();
    let mut empty = RenderCheckpointStore::new(&document);
    empty.start_maintenance(0..0, 0).unwrap();
    let progress = empty.advance_maintenance(&mut unlimited()).unwrap();
    assert_eq!(progress.completion, Completion::Covered);
    assert_eq!(progress.len, 0);

    let mut full = RenderCheckpointStore::new(&document);
    for tile in 0..20 {
        full.insert(TileId(tile), checkpoint(tile)).unwrap();
    }
    full.start_maintenance(0..20, 20).unwrap();
    let progress = full.advance_maintenance(&mut unlimited()).unwrap();
    assert_eq!(progress.released_payloads, 0);
    assert_eq!(progress.len, 20);

    let mut narrow = RenderCheckpointStore::new(&document);
    for tile in 0..20 {
        narrow.insert(TileId(tile), checkpoint(tile)).unwrap();
    }
    narrow.start_maintenance(9..10, 20).unwrap();
    let progress = narrow.advance_maintenance(&mut unlimited()).unwrap();
    assert_eq!(progress.len, 1);
    assert!(narrow.exact(TileId(9)).is_some());
    assert!(narrow.exact(TileId(8)).is_none());
    assert!(narrow.exact(TileId(10)).is_none());
}

#[test]
fn viewport_window_and_maintenance_entrypoint_block_frame_and_apply_until_done() {
    let mut document = Document::new();
    let source = "line\n\n".repeat(2_000);
    let update = document.append(&source).unwrap();
    let mut state = ViewportState::new(&document, config(40, 3)).unwrap();
    state.apply_update(&document, &update).unwrap();
    state.scroll_down(&document, 20).unwrap();
    let window = state.checkpoint_window(&document);
    assert!(window.lo < window.hi);
    assert!(window.hi.0 <= document.tile_count());
    let top = state.top_anchor().unwrap().tile;
    assert!(window.lo <= top && top < window.hi);

    for tile in 0..document.tile_count() {
        state
            .checkpoints_mut()
            .insert(TileId(tile), checkpoint(tile))
            .unwrap();
    }
    let first = state
        .maintain_checkpoints(&document, &mut Budget::new(WorkLimit::default()))
        .unwrap();
    assert_eq!(first.completion, Completion::BudgetExhausted);
    assert!(state.checkpoints().maintenance_in_progress());
    assert_eq!(
        state.frame(&document, &mut unlimited()),
        Err(ViewportError::CheckpointMaintenanceInProgress)
    );
    let empty = document.append("").unwrap();
    assert_eq!(
        state.apply_update(&document, &empty),
        Err(ViewportError::CheckpointMaintenanceInProgress)
    );

    for _ in 0..20_000 {
        let progress = state.maintain_checkpoints(&document, &mut tiny()).unwrap();
        if progress.completion == Completion::Covered {
            assert_eq!(progress.drain_cursor, 0);
            break;
        }
    }
    assert!(!state.checkpoints().maintenance_in_progress());
    assert!(ready_frame(&mut state, &document));
    assert!(state.apply_update(&document, &empty).unwrap().revision > 0);
}

#[test]
fn width_change_cancels_checkpoint_maintenance_and_drops_old_tokens() {
    let mut document = Document::new();
    let update = document.append(&"line\n\n".repeat(500)).unwrap();
    let mut state = ViewportState::new(&document, config(40, 3)).unwrap();
    state.apply_update(&document, &update).unwrap();
    for tile in 0..document.tile_count() {
        state
            .checkpoints_mut()
            .insert(TileId(tile), checkpoint(tile))
            .unwrap();
    }
    let token = state.checkpoints().token(TileId(0)).unwrap();
    let progress = state
        .maintain_checkpoints(&document, &mut Budget::new(WorkLimit::default()))
        .unwrap();
    assert_eq!(progress.completion, Completion::BudgetExhausted);
    assert!(state.checkpoints().maintenance_in_progress());

    let change = state.set_content_width(&document, 20).unwrap();
    assert!(change.changed);
    assert!(!state.checkpoints().maintenance_in_progress());
    assert!(!state.checkpoints().validates(token));
    assert!(state.checkpoints().exact(TileId(0)).is_none());
    assert!(ready_frame(&mut state, &document));
}

#[test]
fn checkpoint_window_clips_at_document_boundaries_and_empty_document() {
    let empty = Document::new();
    let empty_state = ViewportState::new(&empty, config(40, 4)).unwrap();
    assert_eq!(empty_state.checkpoint_window(&empty).range(), 0..0);

    let mut source = Document::new();
    source.append(&"line\n\n".repeat(2_000)).unwrap();
    let state = ViewportState::new(&source, config(40, 2)).unwrap();
    let top = state.checkpoint_window(&source);
    assert_eq!(top.lo, TileId(0));
    assert!(top.hi.0 > 0);

    let mut tail = ViewportState::new(&source, config(40, 2)).unwrap();
    tail.end(&source).unwrap();
    let at_end = tail.checkpoint_window(&source);
    assert_eq!(at_end.hi.0, source.tile_count());

    let narrow = ViewportState::new(&source, config(40, 1)).unwrap();
    let narrow_window = narrow.checkpoint_window(&source);
    assert!(narrow_window.lo < narrow_window.hi);
    assert!(narrow_window.hi.0 <= source.tile_count());
}

#[test]
fn representative_100k_tile_scroll_keeps_valid_payloads_window_bounded() {
    let document = Document::new();
    let mut store = RenderCheckpointStore::new(&document);
    let window = 512_usize;
    let mut max_len = 0;
    let mut max_nodes = 0;
    for position in (0..100_000).step_by(500) {
        for offset in 0..500 {
            let tile = TileId(position + offset);
            store.insert(tile, checkpoint(tile.0)).unwrap();
        }
        let lo = position.saturating_sub(window / 2);
        let hi = (position + window / 2).min(100_000);
        store.start_maintenance(lo..hi, 100_000).unwrap();
        let progress = store.advance_maintenance(&mut unlimited()).unwrap();
        assert_eq!(progress.completion, Completion::Covered);
        assert_eq!(progress.drain_cursor, 0);
        assert!(store.len() <= hi - lo);
        max_len = max_len.max(store.len());
        max_nodes = max_nodes.max(store.arena_stats().nodes);
        assert!(store.exact(TileId(position)).is_some());
    }
    assert!(max_len <= window);
    assert!(max_nodes < 10_000, "arena grew to {max_nodes} nodes");
}

#[test]
#[ignore = "manual 64 MiB rolling-window model"]
fn manual_64_mib_rolling_window_keeps_payloads_bounded() {
    let document = Document::new();
    let mut store = RenderCheckpointStore::new(&document);
    let tiles = (64 * 1024 * 1024) / TILE_BYTES;
    let window = 1_024_usize;
    for position in (0..tiles).step_by(4_096) {
        for offset in 0..4_096 {
            let tile = TileId(position + offset);
            store.insert(tile, checkpoint(tile.0)).unwrap();
        }
        let lo = position.saturating_sub(window / 2);
        let hi = (position + 4_096 + window / 2).min(tiles);
        store.start_maintenance(lo..hi, tiles).unwrap();
        let progress = store.advance_maintenance(&mut unlimited()).unwrap();
        assert_eq!(progress.completion, Completion::Covered);
        assert!(store.len() <= hi - lo);
    }
}

#[derive(Debug)]
struct DropPayload {
    value: usize,
    drops: Rc<DropCell<usize>>,
}

impl Drop for DropPayload {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
    }
}

#[test]
fn core_drop_payloads_prove_lower_drain_and_upper_retirement_release_after_finish() {
    let drops = Rc::new(DropCell::new(0));
    let mut index = CheckpointIndex::new();
    for value in 0..20 {
        index
            .insert(
                TileId(value),
                DropPayload {
                    value,
                    drops: drops.clone(),
                },
            )
            .unwrap();
    }
    index.invalidate(0..8).unwrap();
    index.invalidate(12..usize::MAX).unwrap();
    let mut cursor = 0;
    let mut drained = Vec::new();
    assert_eq!(
        index.drain_before(TileId(8), &mut cursor, &mut unlimited(), &mut drained),
        Completion::Covered
    );
    assert_eq!(drained.len(), 8);
    assert!(drained.iter().all(|payload| payload.value < 8));
    assert_eq!(drops.get(), 0);
    drained.clear();
    assert_eq!(drops.get(), 8);

    let mut task = index.into_compaction();
    assert!(task.stats().source.capacity_bytes > 0);
    let mut retired = Vec::new();
    assert_eq!(
        task.advance(&mut unlimited(), &mut retired),
        Completion::Covered
    );
    assert_eq!(retired.len(), 8);
    assert!(retired.iter().all(|payload| payload.value >= 12));
    assert_eq!(drops.get(), 8);
    let stats = task.stats();
    assert!(stats.retired > 0);
    assert_eq!(stats.source.capacity_bytes, 0);
    retired.clear();
    assert_eq!(drops.get(), 16);
    let index = match task.finish() {
        Ok(index) => index,
        Err(_) => panic!("covered compaction finishes"),
    };
    assert_eq!(index.len(), 4);
    for tile in 8..12 {
        assert!(index.get(TileId(tile)).is_some());
    }
    drop(index);
    assert_eq!(drops.get(), 20);
}
