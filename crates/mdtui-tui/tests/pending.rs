use mdtui_tui::render::first_frame::{FirstFrame, FirstFrameConfig, FirstFrameStatus};
use mdtui_tui::render::viewport::{
    EofCalibrationStatus, ViewportConfig, ViewportFrame, ViewportFrameStatus, ViewportState,
};
use mdtui_tui::screen::{Cell, ContentArea, Screen};
use mdtui_tui::static_render::render_static;
use mdtui3::{Budget, Document, TileId, WorkLimit};

fn config(width: u16, rows: u16, max_tiles: usize) -> ViewportConfig {
    ViewportConfig {
        width,
        rows,
        overscan_rows: 2,
        max_tiles,
        max_replay_bytes: 256 * 1024,
        row_slack: 32,
    }
}

fn unlimited() -> Budget {
    Budget::new(WorkLimit::unlimited())
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

fn frame_text(frame: &ViewportFrame) -> String {
    frame
        .lines
        .iter()
        .flat_map(|line| line.cells.iter())
        .map(|cell| cell.text.as_str())
        .collect()
}

fn line_text(line: &mdtui_tui::render::sink::SinkLine) -> String {
    line.cells.iter().map(|cell| cell.text.as_str()).collect()
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
    for _ in 0..100_000 {
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

fn calibrate(state: &mut ViewportState, document: &mut Document) -> ViewportFrame {
    for _ in 0..20_000 {
        let progress = state
            .finish_and_calibrate(document, &mut unlimited())
            .unwrap();
        if progress.status == EofCalibrationStatus::Calibrated {
            return progress.frame.unwrap();
        }
    }
    panic!("calibration did not settle");
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
    }
    screen
}

fn assert_final_matches_static(frame: &ViewportFrame, document: &Document, width: u16, rows: u16) {
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
    }
}

#[test]
fn empty_document_character_appends_show_evolving_pending_text() {
    let mut document = Document::new();
    let mut state = ViewportState::new(&document, config(40, 5, 32)).unwrap();
    let mut expected = String::new();
    let mut previous: Option<ViewportFrame> = None;
    for character in "pending".chars() {
        expected.push(character);
        let update = document.append(&character.to_string()).unwrap();
        state.apply_update(&document, &update).unwrap();
        let frame = ready_frame(&mut state, &document);
        assert!(frame_text(&frame).contains(&expected));
        assert!(frame.matches(&document));
        if let Some(previous) = previous {
            assert!(!previous.matches(&document));
        }
        previous = Some(frame);
    }
    let frame = previous.unwrap();
    let line = frame
        .lines
        .iter()
        .find(|line| !line.cells.is_empty())
        .unwrap();
    assert_eq!(line.source.unwrap().block, None);
    assert!(line.cells.iter().all(|cell| cell.source.is_some()));
}

#[test]
fn long_pending_line_wraps_and_preserves_pending_line_source() {
    let mut document = Document::new();
    let source = "x".repeat(500);
    let update = document.append(&source).unwrap();
    let mut state = ViewportState::new(&document, config(20, 8, 64)).unwrap();
    state.apply_update(&document, &update).unwrap();
    let frame = ready_frame(&mut state, &document);
    assert!(frame.rows > 1);
    assert!(
        frame
            .lines
            .iter()
            .all(|line| line.source.is_none_or(|source| source.block.is_none()))
    );
    assert!(frame_text(&frame).contains("xxxxxxxx"));
    for line in &frame.lines {
        let width: usize = line.cells.iter().map(|cell| usize::from(cell.width)).sum();
        assert!(width <= 20, "pending line exceeded width: {width}");
    }
}

#[test]
fn pending_raw_text_preserves_source_and_wide_cells_while_pending() {
    let mut document = Document::new();
    let source = "plain **bold** [link](https://example.com) 🙂 中";
    let update = document.append(source).unwrap();
    let mut state = ViewportState::new(&document, config(60, 6, 32)).unwrap();
    state.apply_update(&document, &update).unwrap();
    let frame = ready_frame(&mut state, &document);
    let cells = frame
        .lines
        .iter()
        .flat_map(|line| line.cells.iter())
        .collect::<Vec<_>>();
    assert!(cells.iter().any(|cell| cell.width == 2));
    assert!(
        cells
            .iter()
            .all(|cell| cell.source.is_some_and(|span| span.end <= source.len()))
    );
    assert!(frame_text(&frame).contains("bold"));
    assert!(frame_text(&frame).contains("link"));
}

#[test]
fn pending_and_committed_blocks_are_not_duplicated() {
    let mut document = Document::new();
    let update = document.append("committed\npending-tail").unwrap();
    let mut state = ViewportState::new(&document, config(40, 6, 32)).unwrap();
    state.apply_update(&document, &update).unwrap();
    let frame = ready_frame(&mut state, &document);
    let text = frame_text(&frame);
    assert_eq!(text.matches("committed").count(), 1);
    assert_eq!(text.matches("pending-tail").count(), 1);
    let committed = frame
        .lines
        .iter()
        .find(|line| line_text(line).contains("committed"))
        .unwrap();
    let pending = frame
        .lines
        .iter()
        .find(|line| line_text(line).contains("pending-tail"))
        .unwrap();
    assert!(committed.source.unwrap().block.is_some());
    assert_eq!(pending.source.unwrap().block, None);
}

#[test]
fn pending_is_only_loaded_when_inside_visible_plan() {
    let mut source = String::new();
    for index in 0..200 {
        source.push_str(&format!("paragraph {index}\n\n"));
    }
    source.push_str("far-pending-tail");
    let mut document = Document::new();
    document.append(&source).unwrap();
    let first = full_first_frame(&document, 40);
    let mut state = ViewportState::from_first_frame(&document, first, config(40, 4, 16)).unwrap();
    let top = ready_frame(&mut state, &document);
    assert!(!frame_text(&top).contains("far-pending-tail"));
    state.end(&document).unwrap();
    let bottom = ready_frame(&mut state, &document);
    assert!(frame_text(&bottom).contains("far-pending-tail"));
    assert!(bottom.stats.planned_tiles <= 16);
}

#[test]
fn tiny_budget_resumes_pending_scan() {
    let mut document = Document::new();
    let update = document.append(&"pending ".repeat(100)).unwrap();
    let mut state = ViewportState::new(&document, config(30, 6, 64)).unwrap();
    state.apply_update(&document, &update).unwrap();
    let mut retry = false;
    let frame = loop {
        let frame = state
            .frame(
                &document,
                &mut Budget::new(WorkLimit {
                    queries: 1,
                    tiles: 1,
                    objects: 1,
                    bytes: 16,
                    ..WorkLimit::default()
                }),
            )
            .unwrap();
        match frame.status {
            ViewportFrameStatus::PendingScan | ViewportFrameStatus::BudgetExhausted => {
                retry = true;
            }
            ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument => break frame,
        }
    };
    assert!(retry);
    assert!(frame_text(&frame).contains("pending"));
}

#[test]
fn setext_and_table_pending_replaced_by_final_calibration() {
    for (source, width, pending_needle) in [
        ("Title\n===", 40, "==="),
        ("| A | B |\n| --- | --- |\n| one | two", 40, "one"),
    ] {
        let mut document = Document::new();
        let update = document.append(source).unwrap();
        let mut state = ViewportState::new(&document, config(width, 8, 32)).unwrap();
        state.apply_update(&document, &update).unwrap();
        let pending = ready_frame(&mut state, &document);
        assert!(frame_text(&pending).contains(pending_needle));
        let final_frame = calibrate(&mut state, &mut document);
        assert!(!pending.matches(&document));
        assert_final_matches_static(&final_frame, &document, width, 8);
    }
}

#[test]
fn empty_append_and_repeated_finish_do_not_duplicate_pending_text() {
    let mut document = Document::new();
    let update = document.append("tail-once").unwrap();
    let mut state = ViewportState::new(&document, config(40, 6, 32)).unwrap();
    state.apply_update(&document, &update).unwrap();
    let frame = ready_frame(&mut state, &document);
    assert_eq!(frame_text(&frame).matches("tail-once").count(), 1);

    let empty = document.append("").unwrap();
    let applied = state.apply_update(&document, &empty).unwrap();
    assert_eq!(applied.previous_revision, applied.revision);
    let frame = ready_frame(&mut state, &document);
    assert_eq!(frame_text(&frame).matches("tail-once").count(), 1);

    let final_frame = calibrate(&mut state, &mut document);
    assert_eq!(frame_text(&final_frame).matches("tail-once").count(), 1);
    let repeated = document.finish();
    let applied = state.begin_eof_calibration(&document, &repeated).unwrap();
    assert_eq!(applied.previous_revision, applied.revision);
    let progress = state
        .advance_eof_calibration(&document, &mut Budget::new(WorkLimit::default()))
        .unwrap();
    assert_eq!(progress.status, EofCalibrationStatus::Calibrated);
    assert_eq!(
        frame_text(&progress.frame.unwrap())
            .matches("tail-once")
            .count(),
        1
    );
}

#[test]
fn unclosed_code_and_table_pending_text_is_visible_then_final() {
    for (source, needle, width) in [
        ("```rust\nlet pending = 1", "let pending", 40),
        ("| A | B |\n| --- | --- |\n| pending", "pending", 40),
    ] {
        let mut document = Document::new();
        let update = document.append(source).unwrap();
        let mut state = ViewportState::new(&document, config(width, 8, 32)).unwrap();
        state.apply_update(&document, &update).unwrap();
        let pending = ready_frame(&mut state, &document);
        assert!(frame_text(&pending).contains(needle));
        let final_frame = calibrate(&mut state, &mut document);
        assert_final_matches_static(&final_frame, &document, width, 8);
    }
}
