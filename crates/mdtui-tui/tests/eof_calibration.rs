use mdtui_tui::render::viewport::{
    EofCalibrationProgress, EofCalibrationStatus, ViewportConfig, ViewportFrame,
    ViewportFrameStatus, ViewportState,
};
use mdtui_tui::screen::{Cell, ContentArea, Screen};
use mdtui_tui::static_render::render_static;
use mdtui3::{BlockKind, Budget, Document, TableId, WorkLimit};

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

fn unlimited() -> Budget {
    Budget::new(WorkLimit::unlimited())
}

fn tiny_budget() -> Budget {
    Budget::new(WorkLimit {
        queries: 1,
        tiles: 1,
        objects: 1,
        bytes: 64,
        recovery: 1,
        ..WorkLimit::default()
    })
}

fn ready_frame(state: &mut ViewportState, document: &Document) -> ViewportFrame {
    for _ in 0..20_000 {
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

fn calibrate(state: &mut ViewportState, document: &mut Document) -> EofCalibrationProgress {
    for _ in 0..20_000 {
        let progress = state
            .finish_and_calibrate(document, &mut unlimited())
            .unwrap();
        if progress.status == EofCalibrationStatus::Calibrated {
            return progress;
        }
    }
    panic!("EOF calibration did not settle");
}

fn append_chunks(state: &mut ViewportState, document: &mut Document, source: &str) {
    let mut start = 0;
    while start < source.len() {
        let mut end = (start + 37).min(source.len());
        while !source.is_char_boundary(end) {
            end -= 1;
        }
        let update = document.append(&source[start..end]).unwrap();
        state.apply_update(document, &update).unwrap();
        start = end;
    }
}

fn frame_screen(frame: &ViewportFrame, width: u16, height: u16) -> Screen {
    let mut screen = Screen::new(height, width, ContentArea::new(0, width)).unwrap();
    for (row, line) in frame.lines.iter().enumerate() {
        let row = u16::try_from(row).unwrap();
        let mut column = 0_u16;
        for source in &line.cells {
            screen
                .write_cell(
                    row,
                    column,
                    Cell::new(
                        source.text.clone(),
                        source.style,
                        source.width,
                        source.link,
                        source.source,
                    )
                    .unwrap(),
                )
                .unwrap();
            column += u16::from(source.width);
        }
        if let Some(line_source) = line.source {
            screen.set_line_source(row, line_source).unwrap();
        }
    }
    screen
}

fn assert_frame_matches_static(frame: &ViewportFrame, document: &Document, width: u16, rows: u16) {
    let expected = render_static(document, width, 512).unwrap();
    let actual = frame_screen(frame, width, rows);
    for row in 0..frame.rows {
        for column in 0..width {
            assert_eq!(
                actual.cell(row, column),
                expected.screen.cell(row, column),
                "row {row} column {column}"
            );
        }
        assert_eq!(
            actual.line_source(row),
            expected.screen.line_source(row),
            "row {row} source"
        );
    }
}

fn first_table(document: &Document) -> TableId {
    document
        .blocks()
        .iter()
        .find_map(|block| match block.kind {
            BlockKind::Table { id } => Some(id),
            _ => None,
        })
        .expect("table")
}

#[test]
fn streaming_fixtures_after_finish_match_one_shot_static_cells() {
    let sources = [
        ("inline", include_str!("fixtures/spec/inline.md")),
        ("tables", include_str!("fixtures/spec/tables.md")),
    ];
    for (name, source) in sources {
        for width in [20, 80] {
            let mut document = Document::new();
            document.enable_display_shape_effects().unwrap();
            let mut state = ViewportState::new(&document, config(width, 12)).unwrap();
            append_chunks(&mut state, &mut document, source);
            let before = ready_frame(&mut state, &document);
            let top_source = state.top_source(&document);
            let progress = calibrate(&mut state, &mut document);
            let frame = progress.frame.expect("calibrated frame");
            assert_eq!(progress.status, EofCalibrationStatus::Calibrated);
            assert!(progress.tables_stable_for_width);
            assert!(!before.matches(&document), "{name} width {width}");
            assert!(frame.matches(&document));
            assert_eq!(state.top_source(&document), top_source);
            assert_frame_matches_static(&frame, &document, width, 12);
        }
    }
}

#[test]
fn setext_late_confirmation_calibrates_to_heading_cells() {
    let mut document = Document::new();
    document.enable_display_shape_effects().unwrap();
    let mut state = ViewportState::new(&document, config(40, 8)).unwrap();
    let update = document.append("Title\n===").unwrap();
    state.apply_update(&document, &update).unwrap();
    let before = ready_frame(&mut state, &document);
    let progress = calibrate(&mut state, &mut document);
    let frame = progress.frame.unwrap();
    assert!(!before.matches(&document));
    assert_eq!(frame.lines[0].cells[0].style.heading_level(), Some(1));
    assert_eq!(state.top_source(&document), Some(0));
    assert_frame_matches_static(&frame, &document, 40, 8);
}

#[test]
fn unclosed_code_and_table_constructs_match_static_after_finish() {
    for (source, width) in [
        ("```rust\nlet x = 1", 40),
        ("| A | B |\n| --- | --- |\n| one | two", 40),
    ] {
        let mut document = Document::new();
        document.enable_display_shape_effects().unwrap();
        let mut state = ViewportState::new(&document, config(width, 10)).unwrap();
        let update = document.append(source).unwrap();
        state.apply_update(&document, &update).unwrap();
        let before = ready_frame(&mut state, &document);
        let progress = calibrate(&mut state, &mut document);
        assert!(!before.matches(&document));
        assert_frame_matches_static(&progress.frame.unwrap(), &document, width, 10);
    }
}

#[test]
fn eof_calibration_preserves_top_source_in_middle_and_tail() {
    for tail in [false, true] {
        let source = format!(
            "{}\n\n{}",
            "paragraph line\n\n".repeat(40),
            "tail line\n\n".repeat(20)
        );
        let mut document = Document::new();
        document.enable_display_shape_effects().unwrap();
        let update = document.append(&source).unwrap();
        let mut state = ViewportState::new(&document, config(40, 6)).unwrap();
        state.apply_update(&document, &update).unwrap();
        let _ = ready_frame(&mut state, &document);
        if tail {
            state.end(&document).unwrap();
        } else {
            state.scroll_down(&document, 17).unwrap();
        }
        let _ = ready_frame(&mut state, &document);
        let source = state.top_source(&document);
        let progress = calibrate(&mut state, &mut document);
        assert_eq!(state.top_source(&document), source);
        assert_eq!(progress.top_source, source);
        assert!(progress.frame.unwrap().matches(&document));
    }
}

#[test]
fn tiny_budget_calibration_resumes_and_can_be_exited_while_pending() {
    let mut source = String::from("Title\n===\n\n| A | B |\n| --- | --- |\n");
    for index in 0..40 {
        source.push_str(&format!("| row-{index} | 中文🙂 |\n"));
    }
    let mut document = Document::new();
    document.enable_display_shape_effects().unwrap();
    let update = document.append(&source).unwrap();
    let mut state = ViewportState::new(&document, config(30, 8)).unwrap();
    state.apply_update(&document, &update).unwrap();
    let first = state
        .finish_and_calibrate(&mut document, &mut tiny_budget())
        .unwrap();
    assert_ne!(first.status, EofCalibrationStatus::Calibrated);

    let mut pending = 0;
    let progress = loop {
        let progress = state
            .finish_and_calibrate(&mut document, &mut tiny_budget())
            .unwrap();
        match progress.status {
            EofCalibrationStatus::Calibrated => break progress,
            EofCalibrationStatus::BudgetExhausted | EofCalibrationStatus::PendingScan => {
                pending += 1;
                assert!(pending < 20_000);
            }
        }
    };
    assert!(pending > 0);
    assert_frame_matches_static(&progress.frame.unwrap(), &document, 30, 8);
}

#[test]
fn visible_table_widths_become_final_during_calibration() {
    let mut source =
        String::from("| H | B |\n| --- | --- |\n| [x](https://example.com/long) | b |\n");
    for index in 0..60 {
        source.push_str(&format!("| value-{index} | 中文 |\n"));
    }
    let mut document = Document::new();
    document.enable_display_shape_effects().unwrap();
    let update = document.append(&source).unwrap();
    let mut state = ViewportState::new(&document, config(24, 8)).unwrap();
    state.apply_update(&document, &update).unwrap();
    let table_id = first_table(&document);
    assert!(
        state
            .table_widths_mut()
            .snapshot(&document, table_id)
            .unwrap()
            .provisional
    );

    let progress = calibrate(&mut state, &mut document);
    assert!(progress.tables_stable_for_width);
    let stats = state
        .table_widths_mut()
        .stats(&document, table_id, 24)
        .unwrap();
    assert!(stats.stable_for_width);
    assert_frame_matches_static(&progress.frame.unwrap(), &document, 24, 8);
}

#[test]
fn repeated_finish_returns_cached_calibration_without_rework() {
    let mut document = Document::new();
    document.enable_display_shape_effects().unwrap();
    let mut state = ViewportState::new(&document, config(40, 8)).unwrap();
    let update = document.append("# Final\n\nbody\n").unwrap();
    state.apply_update(&document, &update).unwrap();
    let calibrated = calibrate(&mut state, &mut document);
    let frame = calibrated.frame.clone().unwrap();
    let visits = state.heights().mutation_visits();
    let checkpoints = state.checkpoints().len();

    let repeated = document.finish();
    let applied = state.begin_eof_calibration(&document, &repeated).unwrap();
    assert_eq!(applied.previous_revision, applied.revision);
    let progress = state
        .advance_eof_calibration(&document, &mut Budget::new(WorkLimit::default()))
        .unwrap();
    assert_eq!(progress.status, EofCalibrationStatus::Calibrated);
    assert_eq!(progress.frame.unwrap(), frame);
    assert_eq!(state.heights().mutation_visits(), visits);
    assert_eq!(state.checkpoints().len(), checkpoints);

    let progress = state
        .finish_and_calibrate(&mut document, &mut Budget::new(WorkLimit::default()))
        .unwrap();
    assert_eq!(progress.status, EofCalibrationStatus::Calibrated);
}

#[test]
fn external_finish_update_can_start_eof_calibration() {
    let mut document = Document::new();
    document.enable_display_shape_effects().unwrap();
    let mut state = ViewportState::new(&document, config(40, 8)).unwrap();
    let update = document.append("## External finish\n\nbody\n").unwrap();
    state.apply_update(&document, &update).unwrap();
    let _ = ready_frame(&mut state, &document);

    let finish = document.finish();
    let applied = state.begin_eof_calibration(&document, &finish).unwrap();
    assert!(applied.previous_revision < applied.revision);
    let progress = loop {
        let progress = state
            .advance_eof_calibration(&document, &mut tiny_budget())
            .unwrap();
        match progress.status {
            EofCalibrationStatus::Calibrated => break progress,
            EofCalibrationStatus::BudgetExhausted | EofCalibrationStatus::PendingScan => {}
        }
    };
    assert_frame_matches_static(&progress.frame.unwrap(), &document, 40, 8);
}
