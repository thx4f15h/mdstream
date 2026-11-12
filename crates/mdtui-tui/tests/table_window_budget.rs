use mdtui_tui::render::table::table_geometry_with_widths;
use mdtui_tui::render::table_widths::{
    MAX_OUT_OF_ORDER_ROWS, TableWidthCache, column_window_limit,
};
use mdtui_tui::render::viewport::{ViewportConfig, ViewportFrameStatus, ViewportState};
use mdtui3::{BlockKind, Budget, Completion, Document, TableId, WorkLimit};

fn table_id(document: &Document) -> TableId {
    document
        .blocks()
        .iter()
        .find_map(|b| {
            if let BlockKind::Table { id } = b.kind {
                Some(id)
            } else {
                None
            }
        })
        .unwrap()
}
fn unlimited() -> Budget {
    Budget::new(WorkLimit::unlimited())
}
fn zero() -> Budget {
    Budget::new(WorkLimit::default())
}
fn wide_source(columns: usize) -> String {
    format!(
        "{}|\n{}|\n|x|\n",
        "|h".repeat(columns),
        "|---".repeat(columns)
    )
}

#[test]
fn hundred_thousand_columns_prepare_only_paid_terminal_prefix() {
    let mut document = Document::new();
    document.append(&wide_source(100_000)).unwrap();
    document.finish();
    let id = table_id(&document);
    let table = document.table(id).unwrap();
    assert_eq!(table.columns.len(), 100_000);
    let mut cache = TableWidthCache::new(&document);
    assert_eq!(cache.memory_stats().columns, 0);
    assert_eq!(cache.work().column_metadata_reads, 0);
    assert!(
        cache
            .snapshot_budgeted(&document, id, 80, &mut zero())
            .unwrap()
            .is_none()
    );
    assert_eq!(cache.memory_stats().tables, 0);
    assert_eq!(cache.work().column_metadata_reads, 0);
    let mut steps = 0;
    let snapshot = loop {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            ..WorkLimit::default()
        });
        let result = cache
            .snapshot_budgeted(&document, id, 80, &mut budget)
            .unwrap();
        assert!(budget.used().queries <= 1);
        assert!(budget.used().objects <= 1);
        steps += 1;
        assert!(steps < 40);
        if let Some(snapshot) = result {
            break snapshot;
        }
    };
    assert_eq!(snapshot.widths.len(), column_window_limit(80));
    assert!(snapshot.provisional);
    assert_eq!(cache.work().column_metadata_reads, 28);
    assert_eq!(cache.work().snapshot_columns_copied, 28);
    assert_eq!(cache.memory_stats().columns, 28);
    assert!(cache.memory_stats().tracked_capacity_bytes < 16 * 1024);
    assert!(
        cache
            .snapshot_budgeted(&document, id, 80, &mut zero())
            .unwrap()
            .is_none()
    );
    assert_eq!(cache.work().snapshot_columns_copied, 28);
    let (block_index, block) = document
        .blocks()
        .iter()
        .enumerate()
        .find(|(_, b)| matches!(b.kind, BlockKind::Table { .. }))
        .unwrap();
    let geometry = table_geometry_with_widths(
        &document,
        mdtui3::BlockId(block_index),
        block,
        80,
        &snapshot.widths,
        0..2,
        snapshot.provisional,
    )
    .unwrap();
    assert_eq!(geometry.total_columns, 100_000);
    assert!(!geometry.full_width_exact);
    assert!(geometry.widths_provisional);
    assert!(geometry.column_metadata_reads <= 26);
    assert_eq!(geometry.column_metadata_reads, geometry.visible_columns);
    assert!(geometry.rows.iter().all(|r| r.cells.len() <= 26));
    let mut viewport = ViewportState::new(
        &document,
        ViewportConfig {
            width: 80,
            rows: 24,
            max_tiles: 8,
            ..ViewportConfig::default()
        },
    )
    .unwrap();
    assert_eq!(viewport.table_widths().work().column_metadata_reads, 0);
    let frame = viewport.frame(&document, &mut zero()).unwrap();
    assert_eq!(frame.status, ViewportFrameStatus::BudgetExhausted);
    assert_eq!(viewport.table_widths().memory_stats().tables, 0);
    assert_eq!(viewport.table_widths().work().column_metadata_reads, 0);
    // Real interactive layout also remains on a bounded tile/source window.
    let mut steps = 0;
    loop {
        let mut budget = Budget::new(mdtui_tui::stream::FRAME_WORK);
        let frame = viewport.frame(&document, &mut budget).unwrap();
        steps += 1;
        assert!(steps < 100);
        if matches!(
            frame.status,
            ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument
        ) {
            assert!(!frame.lines.is_empty());
            break;
        }
    }
    assert_eq!(viewport.table_widths().work().column_metadata_reads, 28);
    assert_eq!(viewport.table_widths().memory_stats().columns, 28);
    println!(
        "wide table: columns={}, prepared={}, metadata_reads={}, snapshot_copies={}, capacity={}, geometry_reads={}, frame_steps={steps}",
        table.columns.len(),
        cache.memory_stats().columns,
        cache.work().column_metadata_reads,
        cache.work().snapshot_columns_copied,
        cache.memory_stats().tracked_capacity_bytes,
        geometry.column_metadata_reads
    );
}

fn observe_history(document: &Document, cache: &mut TableWidthCache, id: TableId) {
    // Deliberately skip the scan frontier; visits cannot collapse into a prefix.
    for row in (1..20_000).step_by(31) {
        let result = cache
            .observe_visible_rows(document, id, 80, row..row + 1, &mut unlimited())
            .unwrap();
        assert_eq!(result.completion, Completion::Covered);
    }
    let memory = cache.memory_stats();
    assert_eq!(memory.columns, 10);
    assert_eq!(memory.observed_rows, 10 * MAX_OUT_OF_ORDER_ROWS);
    assert!(memory.tracked_capacity_bytes < 128 * 1024);
    assert!(cache.work().evicted_observations > 6_000);
}

#[test]
fn out_of_order_history_has_bounded_eviction_and_revision_offscreen_resize_cleanup() {
    let mut document = Document::new();
    let mut source = format!("{}|\n{}|\n", "|h".repeat(10), "|---".repeat(10));
    for row in 0..20_000 {
        source.push_str(if row == 0 {
            "|long|x|x|x|x|x|x|x|x|x|\n"
        } else {
            "|x|x|x|x|x|x|x|x|x|x|\n"
        });
    }
    document.append(&source).unwrap();
    let id = table_id(&document);
    let mut cache = TableWidthCache::new(&document);
    observe_history(&document, &mut cache, id);
    // The earliest (widest) observation is evicted; its maximum is retained.
    let provisional = cache.snapshot(&document, id).unwrap();
    assert!(provisional.widths[0] >= 4);
    let before = cache.work();
    let update = document.append("|z|z|z|z|z|z|z|z|z|z|\n").unwrap();
    cache.apply_update(&document, &update).unwrap();
    assert_eq!(cache.memory_stats().columns, 0);
    assert_eq!(
        cache.work().dropped_observations - before.dropped_observations,
        320
    );
    observe_history(&document, &mut cache, id);
    let before = cache.work();
    cache.retain_tables(&[]);
    assert_eq!(
        cache.work().dropped_observations - before.dropped_observations,
        320
    );
    assert_eq!(cache.memory_stats().tables, 0);
    observe_history(&document, &mut cache, id);
    let before = cache.work();
    cache.limit_columns(20);
    assert_eq!(cache.memory_stats().columns, 8);
    assert_eq!(cache.memory_stats().observed_rows, 256);
    assert_eq!(
        cache.work().dropped_observations - before.dropped_observations,
        64
    );
    assert_eq!(cache.work().dropped_columns - before.dropped_columns, 2);
    let mut viewport = ViewportState::new(&document, ViewportConfig::default()).unwrap();
    *viewport.table_widths_mut() = cache;
    let before = viewport.table_widths().work();
    viewport.set_content_width(&document, 20).unwrap();
    assert_eq!(viewport.table_widths().memory_stats().tables, 0);
    assert_eq!(
        viewport.table_widths().work().dropped_observations - before.dropped_observations,
        256
    );
    // Restarting a full scan after bounded eviction still yields exact maxima.
    let cache = viewport.table_widths_mut();
    let result = cache.advance(&document, id, 80, &mut unlimited()).unwrap();
    assert!(result.stable_for_width);
    let exact = cache.snapshot(&document, id).unwrap();
    assert_eq!(exact.widths[0], 4);
    assert!(exact.exact.iter().all(|v| *v));
    println!(
        "history: evictions={}, dropped_observations={}, dropped_columns={}, metadata_reads={}",
        cache.work().evicted_observations,
        cache.work().dropped_observations,
        cache.work().dropped_columns,
        cache.work().column_metadata_reads
    );
}
