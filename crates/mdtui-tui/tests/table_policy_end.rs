use mdtui_tui::render::{
    table_widths::TableWidthPolicy,
    viewport::{ViewportConfig, ViewportFrame, ViewportFrameStatus, ViewportState},
};
use mdtui3::{Budget, Document, TableId, WorkLimit};

fn document(source: &str) -> Document {
    let mut doc = Document::new();
    doc.enable_display_shape_effects().unwrap();
    doc.append(source).unwrap();
    doc
}
fn ready(view: &mut ViewportState, doc: &Document) -> ViewportFrame {
    for _ in 0..10000 {
        let mut b = Budget::new(mdtui_tui::stream::FRAME_WORK);
        let f = view.frame(doc, &mut b).unwrap();
        if matches!(
            f.status,
            ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument
        ) {
            return f;
        }
    }
    panic!("End did not settle");
}
fn text(f: &ViewportFrame) -> String {
    f.lines
        .iter()
        .flat_map(|l| {
            l.cells
                .iter()
                .map(|c| c.text.collect_text())
                .chain(std::iter::once("\n".to_owned()))
        })
        .collect()
}
fn row(n: usize) -> String {
    format!(
        "|{}|\n",
        ('A'..='H')
            .map(|c| format!("{c}{n:02}"))
            .collect::<Vec<_>>()
            .join("|")
    )
}

#[test]
fn end_retains_pan_policy_follow_and_frozen_source_cut_across_changes() {
    let mut doc = document(&format!(
        "p\n\n{}|\n{}|\n{}",
        "|h".repeat(8),
        "|---:".repeat(8),
        (0..30).map(row).collect::<String>()
    ));
    let id = TableId(0);
    let mut view = ViewportState::new(
        &doc,
        ViewportConfig {
            rows: 6,
            width: 40,
            ..ViewportConfig::default()
        },
    )
    .unwrap();
    view.set_table_policy(&doc, TableWidthPolicy::new(3, Some(3)).unwrap())
        .unwrap();
    view.set_table_first_column(&doc, id, 3).unwrap();
    view.seek_end(&doc, true).unwrap();
    let f = ready(&mut view, &doc);
    assert!(text(&f).contains("D29"), "{}", text(&f));
    assert!(!text(&f).contains("A29"));
    assert!(view.following());
    view.set_table_first_column(&doc, id, 5).unwrap();
    let f = ready(&mut view, &doc);
    assert!(text(&f).contains("F29"));
    assert!(!text(&f).contains("D29"));
    assert!(view.following());
    view.set_table_policy(&doc, TableWidthPolicy::new(4, Some(4)).unwrap())
        .unwrap();
    view.set_content_width(&doc, 30).unwrap();
    assert!(text(&ready(&mut view, &doc)).contains("F29"));
    assert!(view.following());
    let u = doc.append(&row(30)).unwrap();
    view.apply_update(&doc, &u).unwrap();
    assert!(text(&ready(&mut view, &doc)).contains("F30"));
    view.seek_end(&doc, false).unwrap();
    let u = doc.append(&row(31)).unwrap();
    view.apply_update(&doc, &u).unwrap();
    let frozen = ready(&mut view, &doc);
    assert!(text(&frozen).contains("F30"));
    assert!(!text(&frozen).contains("F31"));
    assert!(!view.following());
    assert_eq!(view.table_widths().first_column(id), 5);
}

#[test]
fn end_last_columns_jump_over_huge_row_and_delimiter_envelopes() {
    let doc = document(&format!(
        "{}|\n{}|\n|x|\n",
        "|h".repeat(100_000),
        "|---".repeat(100_000)
    ));
    let mut view = ViewportState::new(
        &doc,
        ViewportConfig {
            rows: 8,
            width: 40,
            max_tiles: 8,
            ..ViewportConfig::default()
        },
    )
    .unwrap();
    view.set_table_policy(&doc, TableWidthPolicy::new(3, Some(3)).unwrap())
        .unwrap();
    view.set_table_first_column(&doc, TableId(0), 99_998)
        .unwrap();
    view.seek_end(&doc, true).unwrap();
    let mut zero = Budget::new(WorkLimit::default());
    assert_eq!(
        view.frame(&doc, &mut zero).unwrap().status,
        ViewportFrameStatus::BudgetExhausted
    );
    assert_eq!(view.table_widths().work().column_metadata_reads, 0);
    let frame = ready(&mut view, &doc);
    assert!(text(&frame).contains('h'), "{}", text(&frame));
    assert!(view.end_work().windows <= 3, "{:?}", view.end_work());
    assert!(view.end_work().source_bytes < 2048, "{:?}", view.end_work());
    assert_eq!(view.table_widths().memory_stats().columns, 2);
    assert_eq!(view.table_widths().work().column_metadata_reads, 2);
    println!(
        "End last-column: work={:?} metadata={:?} memory={:?}",
        view.end_work(),
        view.table_widths().work(),
        view.table_widths().memory_stats()
    );
    assert_eq!(
        frame
            .lines
            .iter()
            .find(|l| l.cells.iter().any(|c| c.text == "h"))
            .unwrap()
            .source
            .unwrap()
            .source
            .start,
        0
    );
}

#[test]
fn end_sparse_table_rows_publish_logical_offsets_in_the_original_tile() {
    let doc = document(&format!(
        "p\n\n|h|\n|---|\n{}",
        (0..200).map(|i| format!("|{i}|\n")).collect::<String>()
    ));
    let mut view = ViewportState::new(
        &doc,
        ViewportConfig {
            rows: 5,
            width: 40,
            ..ViewportConfig::default()
        },
    )
    .unwrap();
    view.seek_end(&doc, true).unwrap();
    let frame = ready(&mut view, &doc);
    assert!(text(&frame).contains("199"));
    let table = doc.table(TableId(0)).unwrap();
    let expected = &table.rows()[197];
    assert_eq!(frame.lines[0].source.unwrap().source, expected.source);
    let anchor = frame.top.anchor().unwrap();
    let tile = doc.tile(anchor.tile).unwrap();
    let first = table
        .rows()
        .partition_point(|row| row.source.end.saturating_sub(1) < tile.start);
    assert_eq!(anchor.offset, (197 - first) as u64);
}

#[test]
fn end_giant_cell_keeps_one_header_and_eof_precision_eventually_finishes() {
    use mdtui_tui::render::viewport::EofCalibrationStatus;
    let mut doc = document(&format!("|h|\n|---|\n|{}|\n", "x".repeat(1_000_000)));
    doc.finish();
    let mut view = ViewportState::new(
        &doc,
        ViewportConfig {
            rows: 8,
            width: 40,
            ..ViewportConfig::default()
        },
    )
    .unwrap();
    view.seek_end(&doc, true).unwrap();
    let frame = ready(&mut view, &doc);
    assert_eq!(frame.lines.len(), 5, "{}", text(&frame));
    let mut complete = false;
    for _ in 0..100 {
        let p = view
            .advance_eof_calibration(&doc, &mut Budget::new(mdtui_tui::stream::FRAME_WORK))
            .unwrap();
        if p.status == EofCalibrationStatus::Calibrated {
            assert_eq!(p.frame.unwrap().lines.len(), 5);
            complete = true;
            break;
        }
    }
    assert!(complete);
    assert_eq!(
        view.table_widths_mut()
            .snapshot(&doc, TableId(0))
            .unwrap()
            .widths,
        [1_000_000]
    );
    assert!(view.following());
}

#[test]
fn end_column_window_matches_full_frame_with_one_query_one_object_six_bytes() {
    let doc = document(&format!(
        "p\n\n{}|\n{}|\n{}",
        "|h".repeat(8),
        "|---".repeat(8),
        (0..30).map(row).collect::<String>()
    ));
    let config = ViewportConfig {
        rows: 5,
        width: 30,
        max_tiles: 8,
        ..ViewportConfig::default()
    };
    let setup = || {
        let mut view = ViewportState::new(&doc, config).unwrap();
        view.set_table_policy(&doc, TableWidthPolicy::new(3, Some(3)).unwrap())
            .unwrap();
        view.set_table_first_column(&doc, TableId(0), 5).unwrap();
        view.seek_end(&doc, true).unwrap();
        view
    };
    let mut full = setup();
    let expected = ready(&mut full, &doc);
    let mut small = setup();
    let mut result = None;
    let mut frames = 0;
    let before = small.table_widths().work();
    let mut zero = Budget::new(WorkLimit::default());
    assert_eq!(
        small.frame(&doc, &mut zero).unwrap().status,
        ViewportFrameStatus::BudgetExhausted
    );
    assert_eq!(zero.used(), WorkLimit::default());
    assert_eq!(small.table_widths().work(), before);
    assert_eq!(small.table_widths().memory_stats().columns, 0);
    for _ in 0..20_000 {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 6,
            tiles: 1,
            recovery: 1,
            dependencies: 1,
        });
        let frame = small.frame(&doc, &mut budget).unwrap();
        frames += 1;
        assert!(
            budget.used().tiles <= 1
                && budget.used().recovery <= 1
                && budget.used().dependencies <= 1
        );
        assert!(
            budget.used().queries <= 1 && budget.used().objects <= 1 && budget.used().bytes <= 6
        );
        if frame.status == ViewportFrameStatus::EndOfDocument {
            result = Some(frame);
            break;
        }
    }
    let actual = result.expect("micro End must progress");
    assert_eq!(actual.lines, expected.lines);
    assert_eq!(actual.top, expected.top);
    assert_eq!(small.table_widths().work().column_metadata_reads, 3);
    println!(
        "micro End table: frames={frames} work={:?} columns={:?}",
        small.end_work(),
        small.table_widths().work()
    );
}

#[test]
fn end_pan_targets_the_top_painted_table_when_multiple_tables_are_visible() {
    use mdtui_tui::{
        cli::LinkMode,
        render::frame::{FrameGeometry, FrameRuntime},
    };
    let mut runtime = FrameRuntime::new(
        FrameGeometry {
            rows: 16,
            cols: 40,
            content_width: 40,
        },
        LinkMode::Default,
    )
    .unwrap();
    let source = "|a|b|\n|---|---|\n|one|two|\n\n|c|d|\n|---|---|\n|three|four|\n";
    let update = runtime.append_bytes(source.as_bytes()).unwrap().unwrap();
    runtime.apply_update(&update).unwrap();
    runtime.seek_end(true).unwrap();
    for _ in 0..1000 {
        if runtime
            .frame_to(
                &mut Vec::new(),
                &mut Budget::new(mdtui_tui::stream::FRAME_WORK),
            )
            .unwrap()
            .committed
        {
            break;
        }
    }
    assert!(runtime.pan_table(1).unwrap());
    assert_eq!(
        runtime.viewport().table_widths().first_column(TableId(0)),
        1
    );
    assert_eq!(
        runtime.viewport().table_widths().first_column(TableId(1)),
        0
    );
    assert!(runtime.viewport().following());
}

#[test]
fn selected_end_cells_use_conditional_styles_and_late_definitions_under_micro_budget() {
    let mut doc = document(&format!(
        "|skip|selected|\n|---|---|\n{}|skip|**tail [x** z]|\n\n",
        "|skip|body|\n".repeat(40)
    ));
    let mut view = ViewportState::new(
        &doc,
        ViewportConfig {
            rows: 6,
            width: 40,
            ..ViewportConfig::default()
        },
    )
    .unwrap();
    view.set_table_first_column(&doc, TableId(0), 1).unwrap();
    view.seek_end(&doc, true).unwrap();
    let limit = WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 6,
        tiles: 1,
        recovery: 1,
        dependencies: 1,
    };
    let settle = |view: &mut ViewportState, doc: &Document| {
        let before = view.shared_links().stats();
        view.frame(doc, &mut Budget::new(WorkLimit::default()))
            .unwrap();
        assert_eq!(view.shared_links().stats(), before);
        for _ in 0..200_000 {
            let mut b = Budget::new(limit);
            let f = view.frame(doc, &mut b).unwrap();
            assert!(b.used().queries <= 1 && b.used().objects <= 1 && b.used().bytes <= 6);
            if matches!(
                f.status,
                ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument
            ) {
                return f;
            }
        }
        panic!("conditional End did not settle");
    };
    let old = settle(&mut view, &doc);
    assert!(text(&old).contains("tail [x z]"), "{}", text(&old));
    assert!(!text(&old).contains("skip"));
    let u = doc.append("[x** z]: /u\n").unwrap();
    view.apply_update(&doc, &u).unwrap();
    assert!(!old.matches(&doc));
    let changed = settle(&mut view, &doc);
    assert!(
        text(&changed).contains("**tail x** z"),
        "{}",
        text(&changed)
    );
    assert!(view.following());
    assert_eq!(view.table_widths().first_column(TableId(0)), 1);
    assert!(view.table_widths().work().column_metadata_reads <= 2);
    let cached = ready(&mut view, &doc);
    assert_eq!(text(&cached), text(&changed));
    let stats = view.shared_links().stats();
    let again = ready(&mut view, &doc);
    assert_eq!(view.shared_links().stats(), stats);
    assert_eq!(text(&again), text(&changed));
}
