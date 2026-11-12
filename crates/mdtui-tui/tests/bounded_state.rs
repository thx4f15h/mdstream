use std::cell::Cell as DropCell;
use std::io;
use std::rc::Rc;
use std::time::Instant;

use mdtui_tui::cli::LinkMode;
use mdtui_tui::render::checkpoint::RenderCheckpoint;
use mdtui_tui::render::frame::{FrameGeometry, FrameRuntime};
use mdtui_tui::render::table_widths::TableWidthCache;
use mdtui_tui::render::viewport::{CheckpointRetentionWindow, ViewportConfig, ViewportState};
use mdtui_tui::sidebar::{SidebarModel, SidebarStatus};
use mdtui_tui::stream::FRAME_WORK;
use mdtui3::{Budget, CheckpointIndex, Completion, Document, TILE_BYTES, TileId, WorkLimit};

const SIXTY_FOUR_MIB: usize = 64 * 1024 * 1024;

fn geometry(rows: u16, cols: u16) -> FrameGeometry {
    FrameGeometry {
        rows,
        cols,
        content_width: cols,
    }
}

fn viewport_config(width: u16, rows: u16) -> ViewportConfig {
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

fn checkpoint(value: usize) -> RenderCheckpoint {
    RenderCheckpoint::line_mid(value, 0)
}

fn finish_sidebar(model: &mut SidebarModel, document: &Document) {
    for _ in 0..100_000 {
        if model.advance(document, &mut unlimited()).unwrap() == SidebarStatus::Complete {
            return;
        }
    }
    panic!("sidebar model did not settle");
}

fn checkpoint_window_len(window: CheckpointRetentionWindow) -> usize {
    window.hi.0.saturating_sub(window.lo.0)
}

fn sidebar_retained_bound(model: &SidebarModel, width: usize) -> usize {
    model.rows() * (model.limits().display_bytes.saturating_add(width * 4 + 16))
}

fn add_work(total: &mut WorkLimit, used: WorkLimit) {
    total.queries = total.queries.saturating_add(used.queries);
    total.tiles = total.tiles.saturating_add(used.tiles);
    total.bytes = total.bytes.saturating_add(used.bytes);
    total.objects = total.objects.saturating_add(used.objects);
    total.recovery = total.recovery.saturating_add(used.recovery);
    total.dependencies = total.dependencies.saturating_add(used.dependencies);
}

#[test]
fn screen_buffers_follow_terminal_geometry_not_document_length() {
    let short = "# Short\n\nbody\n";
    let long = {
        let mut source = String::new();
        for index in 0..10_000 {
            source.push_str(&format!("# Heading {index:05}\n\nbody {index:05}\n\n"));
        }
        source
    };

    let mut measurements = Vec::new();
    for (rows, cols) in [(40, 120), (24, 80)] {
        for source in [short, long.as_str()] {
            let mut runtime =
                FrameRuntime::from_finished_source(source, geometry(rows, cols), LinkMode::Default)
                    .unwrap();
            let commit = runtime.frame_to(&mut io::sink(), &mut unlimited()).unwrap();
            assert!(commit.committed);
            let before = runtime.screen().buffer_stats();
            assert_eq!((before.rows, before.cols), (rows, cols));
            assert_eq!(before.front_cells, usize::from(rows) * usize::from(cols));
            assert_eq!(before.back_cells, usize::from(rows) * usize::from(cols));
            assert_eq!(before.front_capacity_cells, before.front_cells);
            assert_eq!(before.back_capacity_cells, before.back_cells);
            assert_eq!(before.line_sources, usize::from(rows));
            assert_eq!(before.line_source_capacity, before.line_sources);

            let _ = runtime.scroll_down(100).unwrap();
            let commit = runtime.frame_to(&mut io::sink(), &mut unlimited()).unwrap();
            assert!(commit.committed);
            assert_eq!(runtime.screen().buffer_stats(), before);
            measurements.push((rows, cols, before));
        }
    }

    for (rows, cols, stats) in &measurements {
        assert_eq!((stats.rows, stats.cols), (*rows, *cols));
        assert_eq!(
            stats.front_capacity_cells,
            usize::from(*rows) * usize::from(*cols)
        );
    }
}

#[test]
fn sidebar_retains_only_the_visible_window_before_and_after_finish() {
    let mut document = Document::new();
    let title = "deterministic heading text ".repeat(6);
    let mut source = String::new();
    for index in 0..10_000 {
        source.push_str(&format!("# H{index:05} {title}\n\n"));
    }
    document.append(&source).unwrap();

    let mut model = SidebarModel::new(&document, 9_960, 40, 36);
    finish_sidebar(&mut model, &document);
    let first = model.memory_stats();
    assert_eq!(first.entries, 40);
    assert!(
        first.retained_text_bytes <= sidebar_retained_bound(&model, 36),
        "sidebar retained {} bytes: {first:?}",
        first.retained_text_bytes
    );
    assert!(model.stats().source_bytes_read <= 40 * model.limits().source_bytes);
    assert_eq!(model.stats().headings_considered, 40);
    assert!(model.entries().all(|entry| entry.id.0 >= 9_960));

    model.reconfigure(&document, 0, 40).unwrap();
    finish_sidebar(&mut model, &document);
    let moved = model.memory_stats();
    assert_eq!(moved.entries, 40);
    assert!(moved.retained_text_bytes <= sidebar_retained_bound(&model, 36));
    assert!(model.entries().all(|entry| entry.id.0 < 40));
    assert_eq!(model.stats().headings_considered, 40);

    let revision = document.revision();
    document.finish();
    assert_ne!(document.revision(), revision);
    assert!(!model.matches(&document));

    let mut rebuilt = SidebarModel::new(&document, 9_960, 40, 36);
    finish_sidebar(&mut rebuilt, &document);
    let eof = rebuilt.memory_stats();
    assert_eq!(eof.entries, 40);
    assert!(eof.retained_text_bytes <= sidebar_retained_bound(&rebuilt, 36));
    assert!(rebuilt.stats().source_bytes_read <= 40 * rebuilt.limits().source_bytes);
    assert!(rebuilt.stats().source_bytes_read < title.len() * 10_000 / 100);
}

#[test]
fn huge_setext_sidebar_text_stays_width_and_budget_bounded() {
    let title = "long setext title material ".repeat(10_000);
    let source = format!("{title}\n{}\n", "=".repeat(80));
    let mut document = Document::new();
    document.append(&source).unwrap();
    document.finish();
    assert_eq!(document.headings().entries().len(), 1);

    let mut model = SidebarModel::new(&document, 0, 1, 40);
    finish_sidebar(&mut model, &document);
    let stats = model.memory_stats();
    let entry = model.entry(0).unwrap();
    assert!(entry.truncated);
    assert!(model.stats().source_bytes_read <= model.limits().source_bytes);
    assert!(model.stats().display_bytes_emitted <= model.limits().display_bytes);
    assert!(stats.retained_text_bytes <= sidebar_retained_bound(&model, 40));
    assert!(entry.text.ends_with('…'));
    assert!(mdtui_tui::render::text::display_width(&entry.text) <= 40);
}

#[test]
fn eof_anchor_index_is_measured_separately_from_the_sidebar_window() {
    let title = "anchor title text ".repeat(6);
    let mut source = String::new();
    for index in 0..2_000 {
        source.push_str(&format!("# A{index:04} {title}\n\n"));
    }
    let mut runtime =
        FrameRuntime::from_finished_source(&source, geometry(24, 80), LinkMode::Default).unwrap();
    assert_eq!(runtime.document().work().input_bytes, source.len() as u64);

    let mut total = WorkLimit::default();
    let mut steps = 0;
    for _ in 0..100_000 {
        let mut budget = Budget::new(FRAME_WORK);
        let status = runtime.advance_anchor_index(&mut budget).unwrap();
        add_work(&mut total, budget.used());
        steps += 1;
        if matches!(status, mdtui_tui::fragment::AnchorBuildStatus::Complete) {
            break;
        }
    }
    assert!(runtime.anchor_index_complete());
    assert_eq!(runtime.anchor_indexed_headings(), Some(2_000));
    assert!(steps >= 1);
    println!(
        "eof-anchors heads=2000 source_bytes={} steps={steps} work={total:?}",
        source.len()
    );
}

#[test]
fn table_width_cache_stores_measurement_state_not_table_body_text() {
    let mut source = String::from("| Name | Note | Value |\n| --- | --- | --- |\n");
    let note = "payload-text-that-must-not-be-copied-into-the-width-cache-".repeat(4);
    for index in 0..10_000 {
        source.push_str(&format!("| item-{index:05} | {note} | {index} |\n"));
    }
    let mut document = Document::new();
    document.append(&source).unwrap();
    document.finish();
    let table_id = document
        .tables()
        .first()
        .map(|_| mdtui3::TableId(0))
        .expect("table");
    let rows = document.table(table_id).unwrap().rows().len();

    let mut cache = TableWidthCache::new(&document);
    let _ = cache.snapshot(&document, table_id).unwrap();
    let advance = cache
        .advance(&document, table_id, 60, &mut unlimited())
        .unwrap();
    assert_eq!(advance.completion, Completion::Covered);
    let stats = cache.stats(&document, table_id, 60).unwrap();
    let memory = cache.memory_stats();
    assert_eq!(memory.tables, 1);
    assert_eq!(memory.columns, 3);
    assert_eq!(memory.observed_rows, 0); // Sequential scans retain only a frontier.
    assert!(stats.rows_read <= rows * 3);
    assert!(stats.cells_read <= rows * 3);
    assert!(
        memory.tracked_capacity_bytes * 32 < source.len(),
        "width cache tracked {} bytes for {} source bytes: {memory:?}",
        memory.tracked_capacity_bytes,
        source.len()
    );
}

#[test]
fn table_width_cache_retains_only_tables_in_the_visible_window() {
    let mut source = String::new();
    let body = "x".repeat(900);
    for table in 0..500 {
        source.push_str(&format!(
            "| H{table} | Note |\n| --- | --- |\n| value-{table} | {body} |\n\n"
        ));
    }
    let mut document = Document::new();
    document.append(&source).unwrap();
    document.finish();
    assert_eq!(document.tables().len(), 500);
    assert!(document.tile_count() > 1_000);

    let mut state = ViewportState::new(&document, viewport_config(80, 8)).unwrap();
    let mut max_tables = 0;
    let mut frames = 0;
    loop {
        let frame = state
            .frame(&document, &mut unlimited())
            .expect("multi-table frame");
        assert!(matches!(
            frame.status,
            mdtui_tui::render::viewport::ViewportFrameStatus::Ready
                | mdtui_tui::render::viewport::ViewportFrameStatus::EndOfDocument
        ));
        frames += 1;
        max_tables = max_tables.max(state.table_widths().memory_stats().tables);
        assert!(
            max_tables <= 128,
            "table cache retained {max_tables} tables"
        );
        if !state.scroll_down(&document, 32).unwrap() {
            break;
        }
    }
    assert!(frames > 10);
    assert!(max_tables > 0);
}

#[test]
fn checkpoint_store_compaction_releases_both_windows_and_reports_arena() {
    let document = Document::new();
    let mut store = mdtui_tui::render::checkpoint::RenderCheckpointStore::new(&document);
    for tile in 0..1_024 {
        store.insert(TileId(tile), checkpoint(tile)).unwrap();
    }
    let before = store.arena_stats();
    store.start_maintenance(300..700, 1_024).unwrap();
    let progress = store.advance_maintenance(&mut unlimited()).unwrap();

    assert_eq!(progress.completion, Completion::Covered);
    assert_eq!(progress.phase, None);
    assert_eq!(progress.drain_cursor, 0);
    assert_eq!(progress.drained_payloads, 300);
    assert_eq!(progress.retired_payloads, 324);
    assert_eq!(progress.released_payloads, 624);
    assert_eq!(progress.len, 400);
    let compaction = progress.compaction.expect("compaction stats");
    assert!(compaction.retired > 0);
    assert_eq!(compaction.source.capacity_bytes, 0);
    assert!(compaction.destination.capacity_bytes > 0);
    assert_eq!(progress.arena, store.arena_stats());
    assert!(progress.arena.capacity_bytes <= before.capacity_bytes);
    assert!(progress.arena.nodes < before.nodes);
    assert!(store.exact(TileId(299)).is_none());
    assert!(store.exact(TileId(300)).is_some());
    assert!(store.exact(TileId(699)).is_some());
    assert!(store.exact(TileId(700)).is_none());

    store.start_maintenance(300..700, 1_024).unwrap();
    let repeated = store.advance_maintenance(&mut unlimited()).unwrap();
    assert_eq!(repeated.completion, Completion::Covered);
    assert_eq!(repeated.drained_payloads, 0);
    assert_eq!(repeated.retired_payloads, 0);
    assert_eq!(repeated.released_payloads, 0);
    assert_eq!(repeated.len, 400);
}

#[test]
fn viewport_scroll_and_append_retire_old_payloads_on_both_sides() {
    let mut document = Document::new();
    let mut source = String::new();
    for index in 0..50_000 {
        source.push_str(&format!(
            "paragraph {index:05} abcdefghijklmnopqrstuvwxyz\n\n"
        ));
    }
    let update = document.append(&source).unwrap();
    let mut state = ViewportState::new(&document, viewport_config(40, 8)).unwrap();
    state.apply_update(&document, &update).unwrap();
    let tile_count = document.tile_count();
    assert!(tile_count > 2_000);

    assert!(state.scroll_down(&document, 500).unwrap());
    let first_window = state.checkpoint_window(&document);
    assert!(first_window.lo.0 > 0);
    assert!(first_window.hi.0 < tile_count);
    for tile in 0..tile_count {
        state
            .checkpoints_mut()
            .insert(TileId(tile), checkpoint(tile))
            .unwrap();
    }
    let first = state
        .maintain_checkpoints(&document, &mut unlimited())
        .unwrap();
    assert_eq!(first.completion, Completion::Covered);
    assert_eq!(first.drain_cursor, 0);
    assert!(first.drained_payloads > 0);
    assert!(first.retired_payloads > 0);
    assert!(state.checkpoints().len() <= checkpoint_window_len(first_window));

    assert!(state.scroll_down(&document, 1_000).unwrap());
    let second_window = state.checkpoint_window(&document);
    assert!(second_window.lo.0 > first_window.hi.0);
    for tile in second_window.range() {
        state
            .checkpoints_mut()
            .insert(TileId(tile), checkpoint(tile))
            .unwrap();
    }
    let stale_upper_end = second_window.hi.0.saturating_add(64).min(tile_count);
    for tile in second_window.hi.0..stale_upper_end {
        state
            .checkpoints_mut()
            .insert(TileId(tile), checkpoint(tile))
            .unwrap();
    }
    let second = state
        .maintain_checkpoints(&document, &mut unlimited())
        .unwrap();
    assert_eq!(second.completion, Completion::Covered);
    assert!(second.drained_payloads > 0);
    assert!(second.retired_payloads > 0);
    assert!(second.released_payloads >= second.drained_payloads + second.retired_payloads);
    assert!(state.checkpoints().len() <= checkpoint_window_len(second_window));
    assert!(
        state.checkpoints().exact(second_window.hi).is_none(),
        "upper stale checkpoint survived compaction"
    );

    let before_append = document.tile_count();
    let stale_window = state.checkpoint_window(&document);
    let append = document
        .append("\n\n## Appended tail\n\ntail paragraph\n\n")
        .unwrap();
    for tile in append.new_tiles.clone() {
        state
            .checkpoints_mut()
            .insert(TileId(tile), checkpoint(tile))
            .unwrap();
    }
    let stale_end = stale_window
        .hi
        .0
        .saturating_add(64)
        .min(document.tile_count());
    for tile in stale_window.hi.0..stale_end {
        state
            .checkpoints_mut()
            .insert(TileId(tile), checkpoint(tile))
            .unwrap();
    }
    state.apply_update(&document, &append).unwrap();
    assert!(document.tile_count() >= before_append);
    let after_append = state.checkpoint_window(&document);
    let third = state
        .maintain_checkpoints(&document, &mut unlimited())
        .unwrap();
    assert_eq!(third.completion, Completion::Covered);
    assert!(third.retired_payloads > 0);
    assert!(state.checkpoints().len() <= checkpoint_window_len(after_append));
    assert!(state.checkpoints().arena_stats().capacity_bytes > 0);
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
fn payload_drop_probe_proves_drain_and_compaction_release() {
    let drops = Rc::new(DropCell::new(0));
    let mut index = CheckpointIndex::new();
    for value in 0..1_024 {
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
    index.invalidate(0..300).unwrap();
    index.invalidate(700..usize::MAX).unwrap();

    let mut cursor = 0;
    let mut drained = Vec::new();
    assert_eq!(
        index.drain_before(TileId(300), &mut cursor, &mut unlimited(), &mut drained),
        Completion::Covered
    );
    assert_eq!(drained.len(), 300);
    assert!(drained.iter().all(|payload| payload.value < 300));
    assert_eq!(drops.get(), 0);
    drained.clear();
    assert_eq!(drops.get(), 300);
    assert_eq!(cursor, index.node_count());

    let mut task = index.into_compaction();
    assert!(task.stats().source.capacity_bytes > 0);
    let mut retired = Vec::new();
    assert_eq!(
        task.advance(&mut unlimited(), &mut retired),
        Completion::Covered
    );
    assert_eq!(retired.len(), 324);
    assert!(retired.iter().all(|payload| payload.value >= 700));
    let stats = task.stats();
    assert!(stats.retired > 0);
    assert_eq!(stats.source.capacity_bytes, 0);
    assert!(stats.destination.capacity_bytes > 0);
    assert_eq!(drops.get(), 300);
    retired.clear();
    assert_eq!(drops.get(), 624);

    let index = match task.finish() {
        Ok(index) => index,
        Err(_) => panic!("covered compaction finishes"),
    };
    assert_eq!(index.len(), 400);
    for tile in 300..700 {
        assert!(index.get(TileId(tile)).is_some(), "tile {tile}");
    }
    drop(index);
    assert_eq!(drops.get(), 1_024);
}

fn large_source(bytes: usize) -> String {
    const BLOCK: &str = "paragraph abcdefghijklmnopqrstuvwxyz 0123456789\n\n";
    let mut source = String::with_capacity(bytes);
    while source.len() < bytes {
        source.push_str(BLOCK);
    }
    source.truncate(bytes);
    source
}

#[test]
#[ignore = "manual 64 MiB bounded-state acceptance; not a CI wall-clock assertion"]
fn manual_64_mib_viewport_scroll_keeps_checkpoint_arena_bounded() {
    let started = Instant::now();
    let source = large_source(SIXTY_FOUR_MIB);
    assert_eq!(source.len(), SIXTY_FOUR_MIB);
    let mut document = Document::new();
    document.append(&source).unwrap();
    document.finish();
    let tiles = document.tile_count();
    assert_eq!(tiles, SIXTY_FOUR_MIB.div_ceil(TILE_BYTES));

    let mut state = ViewportState::new(&document, viewport_config(80, 24)).unwrap();
    for tile in 0..tiles {
        state
            .checkpoints_mut()
            .insert(TileId(tile), checkpoint(tile))
            .unwrap();
    }

    let mut max_len = 0_usize;
    let mut max_nodes = 0_usize;
    let mut total_released = 0_usize;
    let step = (tiles / 64).max(1) as u64;
    for iteration in 0..64 {
        if iteration > 0 {
            assert!(state.scroll_down(&document, step).unwrap());
        }
        let window = state.checkpoint_window(&document);
        for tile in window.range() {
            state
                .checkpoints_mut()
                .insert(TileId(tile), checkpoint(tile))
                .unwrap();
        }
        let stale_end = window.hi.0.saturating_add(128).min(tiles);
        for tile in window.hi.0..stale_end {
            state
                .checkpoints_mut()
                .insert(TileId(tile), checkpoint(tile))
                .unwrap();
        }
        let progress = state
            .maintain_checkpoints(&document, &mut unlimited())
            .unwrap();
        assert_eq!(progress.completion, Completion::Covered);
        assert_eq!(progress.drain_cursor, 0);
        assert!(state.checkpoints().len() <= checkpoint_window_len(window));
        let arena = state.checkpoints().arena_stats();
        max_len = max_len.max(state.checkpoints().len());
        max_nodes = max_nodes.max(arena.nodes);
        total_released = total_released.saturating_add(progress.released_payloads);
        assert!(
            progress.retired_payloads > 0 || iteration == 0,
            "iteration {iteration} did not retire upper stale payloads"
        );
    }

    assert!(max_len <= 24 * 5 + 1, "checkpoint window grew to {max_len}");
    assert!(
        max_nodes < 4_096,
        "checkpoint arena grew to {max_nodes} nodes"
    );
    assert!(total_released > 0);
    println!(
        "bounded-64m source={} tiles={tiles} max_len={max_len} max_nodes={max_nodes} released={total_released} elapsed={:?}",
        source.len(),
        started.elapsed()
    );
}
