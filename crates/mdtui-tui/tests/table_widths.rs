use mdtui_tui::render::first_frame::{FirstFrame, FirstFrameConfig, FirstFrameStatus};
use mdtui_tui::render::table::{table_geometry, table_geometry_with_widths};
use mdtui_tui::render::table_widths::{TableWidthCache, TableWidthError};
use mdtui_tui::render::viewport::{ViewportConfig, ViewportState};
use mdtui3::{BlockId, BlockKind, Budget, Completion, Document, TableId, TileId, WorkLimit};
use std::collections::HashMap;

fn parse(source: &str) -> Document {
    let mut document = Document::new();
    document.append(source).unwrap();
    document.finish();
    document
}

fn first_table(document: &Document) -> (TableId, BlockId, &mdtui3::Block) {
    document
        .blocks()
        .iter()
        .enumerate()
        .find_map(|(index, block)| match block.kind {
            BlockKind::Table { id } => Some((id, BlockId(index), block)),
            _ => None,
        })
        .unwrap()
}

fn tiny_budget() -> Budget {
    Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 256,
        ..WorkLimit::default()
    })
}

fn measure_cache(document: &Document, table_id: TableId, width: usize) -> (TableWidthCache, bool) {
    let mut cache = TableWidthCache::new(document);
    let mut exhausted = false;
    for _ in 0..100_000 {
        let result = cache
            .advance(document, table_id, width, &mut tiny_budget())
            .unwrap();
        exhausted |= result.completion == Completion::BudgetExhausted;
        if result.stable_for_width {
            return (cache, exhausted);
        }
    }
    panic!("table width measurement did not stabilize");
}

fn generated_table(rows: usize) -> String {
    let mut source = String::from("| Name | Note | Value |\n| --- | --- | --- |\n");
    for index in 0..rows {
        match index % 3 {
            0 => source.push_str(&format!("| item-{index} | 中文 | 🙂 |\n")),
            1 => source.push_str(&format!("| row-{index} | plain | {index} |\n")),
            _ => source.push_str(&format!("| x-{index} | emoji🙂 | ok |\n")),
        }
    }
    source
}

#[test]
fn large_table_first_screen_reads_only_hints_and_the_visible_row_window() {
    let document = parse(&generated_table(10_000));
    let (table_id, block_id, block) = first_table(&document);
    let table = document.table(table_id).unwrap();
    assert_eq!(table.rows().len(), 10_001);

    let mut cache = TableWidthCache::new(&document);
    let hints = cache.snapshot(&document, table_id).unwrap();
    let stats = cache.stats(&document, table_id, 80).unwrap();
    assert_eq!((stats.rows_read, stats.cells_read), (0, 0));
    let geometry = table_geometry_with_widths(
        &document,
        block_id,
        block,
        80,
        &hints.widths,
        0..4,
        hints.provisional,
    )
    .unwrap();
    assert_eq!(geometry.rows.len(), 4);

    let mut first = FirstFrame::new(
        &document,
        FirstFrameConfig {
            top_tile: TileId(0),
            tile_limit: 8,
            width: 80,
            row_limit: 1_000,
        },
    )
    .unwrap();
    let step = first
        .advance(&document, &mut Budget::new(WorkLimit::unlimited()))
        .unwrap();
    assert!(matches!(
        step.status,
        FirstFrameStatus::Covered | FirstFrameStatus::EndOfDocument
    ));
    let sink_stats = first.sink_stats().unwrap();
    assert!(sink_stats.table_rows_prepared > 0);
    assert!(sink_stats.table_rows_prepared < 500);
}

#[test]
fn width_measurements_converge_to_static_visible_prefix_and_hints_are_safe() {
    let document = parse(&generated_table(200));
    let (table_id, block_id, block) = first_table(&document);
    let full = table_geometry(&document, block_id, block, 500).unwrap();
    let table = document.table(table_id).unwrap();
    for (column, geometry) in table.columns.iter().zip(full.columns.iter()) {
        assert!(
            column.source_width_hint >= geometry.content_width,
            "hint {} understated visible width {}",
            column.source_width_hint,
            geometry.content_width
        );
    }

    let (mut cache, saw_budget) = measure_cache(&document, table_id, 500);
    assert!(saw_budget);
    let snapshot = cache.snapshot(&document, table_id).unwrap();
    assert!(!snapshot.provisional);
    let measured = table_geometry_with_widths(
        &document,
        block_id,
        block,
        500,
        &snapshot.widths,
        0..table.rows().len(),
        snapshot.provisional,
    )
    .unwrap();
    assert_eq!(measured.visible_columns, full.visible_columns);
    assert_eq!(measured.visible_width, full.visible_width);
    assert_eq!(measured.columns, full.columns);
    assert_eq!(measured.rows.len(), full.rows.len());
}

#[test]
fn measurement_growth_marks_visible_prefix_change_for_redraw() {
    let document = parse(concat!(
        "| H | B |\n",
        "| --- | --- |\n",
        "| [x](https://example.com/very/long/destination) | b |\n",
    ));
    let (table_id, block_id, block) = first_table(&document);
    let mut cache = TableWidthCache::new(&document);
    let before = cache.snapshot(&document, table_id).unwrap();
    let geometry = table_geometry_with_widths(
        &document,
        block_id,
        block,
        20,
        &before.widths,
        0..document.table(table_id).unwrap().rows().len(),
        before.provisional,
    )
    .unwrap();
    assert_eq!(geometry.visible_columns, 1);

    let mut saw_redraw = false;
    for _ in 0..1_000 {
        let advance = cache
            .advance(&document, table_id, 20, &mut tiny_budget())
            .unwrap();
        saw_redraw |= advance.width_changed
            || advance.visible_columns_before != advance.visible_columns_after;
        if advance.stable_for_width {
            break;
        }
    }
    assert!(saw_redraw);
    let after = cache.snapshot(&document, table_id).unwrap();
    let measured = table_geometry_with_widths(
        &document,
        block_id,
        block,
        20,
        &after.widths,
        0..document.table(table_id).unwrap().rows().len(),
        after.provisional,
    )
    .unwrap();
    let full = table_geometry(&document, block_id, block, 20).unwrap();
    assert_eq!(measured.visible_columns, full.visible_columns);
    assert_eq!(measured.visible_columns, 2);
}

#[test]
fn narrow_width_stops_before_measuring_offscreen_columns() {
    let document = parse(concat!(
        "| A | B | C |\n",
        "| --- | --- | --- |\n",
        "| a | ",
        "very-wide-column-content-that-cannot-fit | c |\n",
        "| b | also-very-wide-content-that-cannot-fit | d |\n",
    ));
    let (table_id, block_id, block) = first_table(&document);
    let (mut cache, _) = measure_cache(&document, table_id, 20);
    let stats = cache.stats(&document, table_id, 20).unwrap();
    assert!(stats.stable_for_width);
    assert!(stats.columns_complete < stats.columns_total);
    let snapshot = cache.snapshot(&document, table_id).unwrap();
    let geometry = table_geometry_with_widths(
        &document,
        block_id,
        block,
        20,
        &snapshot.widths,
        0..document.table(table_id).unwrap().rows().len(),
        snapshot.provisional,
    )
    .unwrap();
    let full = table_geometry(&document, block_id, block, 20).unwrap();
    assert_eq!(geometry.visible_columns, full.visible_columns);
    assert!(geometry.visible_width <= 20 || geometry.clipped_first_column);
}

#[test]
fn sparse_cells_and_missing_columns_remain_safe() {
    let document = parse(concat!(
        "| A | B | C |\n",
        "| --- | --- | --- |\n",
        "| one |  |  |\n",
        "|  | two |\n",
        "|  |  | 三 |\n",
    ));
    let (table_id, block_id, block) = first_table(&document);
    let (mut cache, _) = measure_cache(&document, table_id, 80);
    let snapshot = cache.snapshot(&document, table_id).unwrap();
    let geometry = table_geometry_with_widths(
        &document,
        block_id,
        block,
        80,
        &snapshot.widths,
        0..document.table(table_id).unwrap().rows().len(),
        snapshot.provisional,
    )
    .unwrap();
    let full = table_geometry(&document, block_id, block, 80).unwrap();
    assert_eq!(geometry.visible_columns, full.visible_columns);
    assert_eq!(geometry.rows[3].cells[2].content_width, 2);
    assert_eq!(geometry.rows[1].cells[1].content_width, 0);
}

#[test]
fn table_row_append_invalidates_old_widths_and_remeasures_new_content() {
    let mut document = Document::new();
    document
        .append("| Name | Value |\n| --- | --- |\n| one | short |\n")
        .unwrap();
    let (table_id, _, _) = first_table(&document);
    let (mut cache, _) = measure_cache(&document, table_id, 80);
    let before = cache.snapshot(&document, table_id).unwrap();

    let update = document
        .append("| much-longer-name | wider-value |\n")
        .unwrap();
    assert!(cache.apply_update(&document, &update).unwrap());
    let after = cache.snapshot(&document, table_id).unwrap();
    assert!(after.provisional);
    assert_ne!(after, before);

    let (mut cache, _) = measure_cache(&document, table_id, 80);
    let snapshot = cache.snapshot(&document, table_id).unwrap();
    let (_, block_id, block) = first_table(&document);
    let full = table_geometry(&document, block_id, block, 80).unwrap();
    let measured = table_geometry_with_widths(
        &document,
        block_id,
        block,
        80,
        &snapshot.widths,
        0..document.table(table_id).unwrap().rows().len(),
        snapshot.provisional,
    )
    .unwrap();
    assert_eq!(measured.visible_columns, full.visible_columns);
    assert_eq!(measured.columns, full.columns);
}

#[test]
fn finish_revision_clears_widths_before_final_measurement() {
    let mut document = Document::new();
    document
        .append("| A | B |\n| --- | --- |\n| 中文 | 🙂 |\n")
        .unwrap();
    let (table_id, _, _) = first_table(&document);
    let (mut cache, _) = measure_cache(&document, table_id, 80);
    assert!(!cache.snapshot(&document, table_id).unwrap().provisional);

    let finish = document.finish();
    assert!(cache.apply_update(&document, &finish).unwrap());
    assert!(cache.snapshot(&document, table_id).unwrap().provisional);
    let (mut cache, _) = measure_cache(&document, table_id, 80);
    let snapshot = cache.snapshot(&document, table_id).unwrap();
    let (_, block_id, block) = first_table(&document);
    let full = table_geometry(&document, block_id, block, 80).unwrap();
    let measured = table_geometry_with_widths(
        &document,
        block_id,
        block,
        80,
        &snapshot.widths,
        0..document.table(table_id).unwrap().rows().len(),
        snapshot.provisional,
    )
    .unwrap();
    assert_eq!(measured.visible_columns, full.visible_columns);
    assert_eq!(measured.visible_width, full.visible_width);
}

#[test]
fn wrong_document_and_out_of_order_updates_do_not_mutate_cache() {
    let mut document = Document::new();
    document.append("| A |\n| --- |\n| x |\n").unwrap();
    let (table_id, _, _) = first_table(&document);
    let mut cache = TableWidthCache::new(&document);

    let mut foreign = Document::new();
    let foreign_update = foreign.append("| B |\n| --- |\n| y |\n").unwrap();
    assert_eq!(
        cache.apply_update(&foreign, &foreign_update),
        Err(TableWidthError::WrongDocument)
    );
    let _first = document.append("| c |\n").unwrap();
    let second = document.append("| d |\n").unwrap();
    assert_eq!(
        cache.apply_update(&document, &second),
        Err(TableWidthError::OutOfOrderUpdate)
    );
    assert_eq!(
        cache.snapshot(&foreign, table_id),
        Err(TableWidthError::WrongDocument)
    );
    assert_eq!(cache.revision(), 1);
}

#[test]
fn viewport_incremental_update_invalidates_its_table_width_cache() {
    let mut document = Document::new();
    document
        .append("| A | B |\n| --- | --- |\n| one | two |\n")
        .unwrap();
    let (table_id, _, _) = first_table(&document);
    let mut state = ViewportState::new(
        &document,
        ViewportConfig {
            width: 80,
            rows: 8,
            overscan_rows: 2,
            max_tiles: 32,
            max_replay_bytes: 256 * 1024,
            row_slack: 32,
        },
    )
    .unwrap();
    let result = state
        .table_widths_mut()
        .advance(
            &document,
            table_id,
            80,
            &mut Budget::new(WorkLimit::unlimited()),
        )
        .unwrap();
    assert!(result.stable_for_width);
    assert!(
        !state
            .table_widths_mut()
            .snapshot(&document, table_id)
            .unwrap()
            .provisional
    );

    let update = document.append("| longer | values |\n").unwrap();
    state.apply_update(&document, &update).unwrap();
    let snapshot = state
        .table_widths_mut()
        .snapshot(&document, table_id)
        .unwrap();
    let stats = state
        .table_widths_mut()
        .stats(&document, table_id, 80)
        .unwrap();
    assert!(snapshot.provisional);
    assert_eq!((stats.rows_read, stats.cells_read), (0, 0));
}

#[test]
fn viewport_width_progress_discards_pending_frame_for_redraw() {
    let document = parse(concat!(
        "| H | B |\n",
        "| --- | --- |\n",
        "| [x](https://example.com/very/long/destination) | b |\n",
    ));
    let (table_id, _, _) = first_table(&document);
    let mut state = ViewportState::new(
        &document,
        ViewportConfig {
            width: 20,
            rows: 8,
            overscan_rows: 2,
            max_tiles: 32,
            max_replay_bytes: 256 * 1024,
            row_slack: 32,
        },
    )
    .unwrap();
    let _ = state
        .frame(
            &document,
            &mut Budget::new(WorkLimit {
                queries: 1,
                objects: 1,
                bytes: 8,
                ..WorkLimit::default()
            }),
        )
        .unwrap();
    assert!(state.has_pending_frame());

    let mut redraw = false;
    for _ in 0..1_000 {
        let advance = state
            .advance_table_widths(&document, table_id, 20, &mut tiny_budget())
            .unwrap();
        if advance.width_changed || advance.visible_columns_before != advance.visible_columns_after
        {
            redraw = true;
            break;
        }
    }
    assert!(redraw);
    assert!(!state.has_pending_frame());
}

#[test]
fn p31_partial_table_uses_measured_width_snapshot() {
    let mut source = String::from(concat!(
        "| H | B |\n",
        "| --- | --- |\n",
        "| [x](https://example.com/very/long/destination) | b |\n",
    ));
    for index in 0..200 {
        source.push_str(&format!("| a{index} | c |\n"));
    }
    let document = parse(&source);
    let (table_id, _, _) = first_table(&document);
    let (mut cache, _) = measure_cache(&document, table_id, 20);
    let snapshot = cache.snapshot(&document, table_id).unwrap();
    assert!(!snapshot.provisional);
    let mut widths = HashMap::new();
    widths.insert(table_id, snapshot);

    let mut first = FirstFrame::new(
        &document,
        FirstFrameConfig {
            top_tile: TileId(0),
            tile_limit: 1,
            width: 20,
            row_limit: 100,
        },
    )
    .unwrap();
    first.set_table_widths(widths).unwrap();
    let step = first
        .advance(&document, &mut Budget::new(WorkLimit::unlimited()))
        .unwrap();
    assert!(matches!(
        step.status,
        FirstFrameStatus::Covered | FirstFrameStatus::EndOfDocument
    ));
    let text = first
        .lines()
        .iter()
        .flat_map(|line| line.cells.iter())
        .map(|cell| cell.text.collect_text())
        .collect::<String>();
    assert!(text.contains('B'));
    assert!(first.sink_stats().unwrap().table_rows_prepared < 50);
}

#[test]
fn mathematical_precision_preserves_a_suspended_context_across_unrelated_appends() {
    let mut document = Document::new();
    document
        .append("| a | b |\n| - | - |\n| \\(\\frac{x}{y}\\) | z |\n\n")
        .unwrap();
    let mut cache = TableWidthCache::new(&document);
    for step in 0..20000 {
        if step > 0 && step % 8 == 0 {
            let update = document.append("unrelated\n\n").unwrap();
            cache.apply_update(&document, &update).unwrap();
        }
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 6,
            ..Default::default()
        });
        let result = cache
            .advance(&document, TableId(0), 80, &mut budget)
            .unwrap();
        assert!(
            budget.used().queries <= 1 && budget.used().objects <= 1 && budget.used().bytes <= 6
        );
        if result.stable_for_width {
            assert_eq!(
                cache.snapshot(&document, TableId(0)).unwrap().widths,
                [3, 1]
            );
            return;
        }
    }
    panic!("continued mathematical precision stalled");
}
