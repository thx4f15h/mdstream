use mdtui_tui::render::first_frame::{
    FirstFrame, FirstFrameConfig, FirstFrameError, FirstFrameStatus,
};
use mdtui_tui::screen::{ContentArea, Screen};
use mdtui_tui::static_render::render_static;
use mdtui3::{Budget, Document, HeightState, Span, TILE_BYTES, TileId, WorkLimit};

fn append(source: &str) -> Document {
    let mut document = Document::new();
    document.append(source).unwrap();
    document
}

fn unlimited() -> Budget {
    Budget::new(WorkLimit::unlimited())
}

fn config(document: &Document, width: u16, row_limit: u64) -> FirstFrameConfig {
    FirstFrameConfig {
        top_tile: TileId(0),
        tile_limit: document.tile_count().max(1),
        width,
        row_limit,
    }
}

fn run_to_end(first: &mut FirstFrame, document: &Document) -> FirstFrameStatus {
    for _ in 0..64 {
        let step = first.advance(document, &mut unlimited()).unwrap();
        match step.status {
            FirstFrameStatus::EndOfDocument => return step.status,
            FirstFrameStatus::Covered => return step.status,
            FirstFrameStatus::BudgetExhausted | FirstFrameStatus::PendingScan => {}
        }
    }
    panic!("first frame did not complete");
}

fn assert_static_equivalent(first: &FirstFrame, document: &Document, width: u16) {
    let height = u16::try_from(first.lines().len())
        .unwrap()
        .saturating_add(16)
        .max(16);
    let expected = render_static(document, width, height).unwrap();
    assert_eq!(first.lines().len(), usize::from(expected.stats.rows));
    let rows = expected.stats.rows.max(1);
    let mut actual = Screen::new(rows, width, ContentArea::new(0, width)).unwrap();
    assert_eq!(
        first.write_to_screen(&mut actual, 0),
        Ok(expected.stats.rows)
    );
    for row in 0..expected.stats.rows {
        assert_eq!(
            actual.line_source(row),
            expected.screen.line_source(row),
            "row {row}"
        );
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
fn incomplete_scan_returns_pending_without_layout_or_measure() {
    let document = append("hello **world**\n");
    let mut first = FirstFrame::new(&document, config(&document, 40, 100)).unwrap();
    let mut budget = Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 1,
        ..WorkLimit::default()
    });
    let step = first.advance(&document, &mut budget).unwrap();
    assert_eq!(step.status, FirstFrameStatus::PendingScan);
    assert!(first.sink().is_none());
    assert_eq!(first.rows(), 0);
    assert_eq!(first.processed_tiles(), 0);
    assert_eq!(
        first.heights().height_state(TileId(0)),
        Some(HeightState::Estimated)
    );
}

#[test]
fn empty_info_fence_produces_first_frame_label_without_finish() {
    let document = append("```rust\n```\n");
    assert!(!document.is_finished());
    let mut first = FirstFrame::new(&document, config(&document, 40, 20)).unwrap();
    assert_eq!(
        run_to_end(&mut first, &document),
        FirstFrameStatus::EndOfDocument
    );
    assert_eq!(first.lines()[0].cells[0].text, "[");
    assert!(
        first.lines()[0]
            .cells
            .iter()
            .map(|cell| cell.text.collect_text())
            .collect::<String>()
            .contains("[rust]")
    );
    assert_eq!(first.sink_stats().unwrap().document_queries_in_callbacks, 0);
    assert_static_equivalent(&first, &document, 40);
}

#[test]
fn first_frame_cells_match_static_for_paragraph_long_line_and_mixed_tile() {
    let long_line = format!("{}\n", "a".repeat(TILE_BYTES * 2 + 10));
    for source in [
        "hello **world** and `code`\n",
        long_line.as_str(),
        "before\n```rust\n```\nafter\n",
    ] {
        let document = append(source);
        let mut first = FirstFrame::new(&document, config(&document, 80, 1_000)).unwrap();
        assert_eq!(
            run_to_end(&mut first, &document),
            FirstFrameStatus::EndOfDocument
        );
        assert!(first.layout_attempts() > 0);
        assert!(!document.is_finished());
        assert_static_equivalent(&first, &document, 80);
    }
}

#[test]
fn tiny_budget_resumes_without_lost_or_duplicate_rows() {
    let document = append("alpha\n\n- list item\n\n```rust\n```\n\nomega\n");
    let mut first = FirstFrame::new(&document, config(&document, 40, 1_000)).unwrap();
    let mut saw_budget = false;
    for _ in 0..2_000 {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            tiles: 1,
            objects: 1,
            bytes: 10,
            ..WorkLimit::default()
        });
        let step = first.advance(&document, &mut budget).unwrap();
        match step.status {
            FirstFrameStatus::BudgetExhausted | FirstFrameStatus::PendingScan => {
                saw_budget = true;
            }
            FirstFrameStatus::Covered | FirstFrameStatus::EndOfDocument => break,
        }
    }
    assert!(saw_budget);
    assert_eq!(first.processed_tiles(), first.plan().segments.len());
    for segment in &first.plan().segments {
        assert!(matches!(
            first.heights().height_state(segment.tile),
            Some(HeightState::Measured | HeightState::Local)
        ));
    }
    assert_static_equivalent(&first, &document, 40);
}

#[test]
fn revision_change_invalidates_first_frame_progress() {
    let mut document = append("one\n");
    let mut first = FirstFrame::new(&document, config(&document, 40, 20)).unwrap();
    document.append("two\n").unwrap();
    assert!(!first.matches(&document));
    let mut budget = unlimited();
    assert_eq!(
        first.advance(&document, &mut budget),
        Err(FirstFrameError::Stale)
    );
}

#[test]
fn large_unfinished_document_gets_a_bounded_first_batch() {
    let mut document = Document::new();
    document
        .append(&"short paragraph text\n\n".repeat(2_000))
        .unwrap();
    assert!(!document.is_finished());
    let before_work = document.work();
    let document_id = document.id();
    let mut first = FirstFrame::new(
        &document,
        FirstFrameConfig {
            top_tile: TileId(0),
            tile_limit: 4,
            width: 80,
            row_limit: 10,
        },
    )
    .unwrap();
    assert!(
        first
            .initialize(&document, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap()
    );
    assert_eq!(first.plan().tile_end, TileId(4));
    assert!(first.plan().tile_end.0 < document.tile_count());

    let mut frames = 0;
    while first.rows() == 0 && frames < 500 {
        frames += 1;
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            tiles: 1,
            objects: 1,
            bytes: 32,
            ..WorkLimit::default()
        });
        let step = first.advance(&document, &mut budget).unwrap();
        if step.status == FirstFrameStatus::Covered {
            break;
        }
    }
    assert!(frames < 500);
    assert!(!first.lines().is_empty());
    assert!(!document.is_finished());
    assert_eq!(document.id(), document_id);
    assert_eq!(document.work(), before_work);
    assert_eq!(
        first.scanner().requested(),
        Span::new(
            document.tile(TileId(0)).unwrap().start,
            document.tile(TileId(3)).unwrap().end,
        )
    );
}
