mod support;

use mdtui_tui::cli::LinkMode;
use mdtui_tui::render::checkpoint::RenderCheckpoint;
use mdtui_tui::render::frame::{
    BEGIN_SYNCHRONIZED_UPDATE, END_SYNCHRONIZED_UPDATE, FIRST_FRAME_CLEAR, FrameCommit,
    FrameGeometry, FrameRuntime,
};
use mdtui_tui::render::viewport::{CONTENT_WIDTH_RESET_ESTIMATE, ViewportFrameStatus};
use mdtui_tui::screen::{Cell, Screen};
use mdtui_tui::theme::{MONOKAI, Rgb, Underline};
use mdtui3::{Budget, HeightState, LayoutError, TableId, TileId, WorkLimit};
use support::terminal::TerminalScreen;
use vt100::Color;

const CLEAR: &[u8] = b"\x1b[2J";
const HOME: &[u8] = b"\x1b[H";

fn unlimited() -> Budget {
    Budget::new(WorkLimit::unlimited())
}

fn commit(runtime: &mut FrameRuntime, output: &mut Vec<u8>) -> FrameCommit {
    for _ in 0..10_000 {
        let commit = runtime
            .frame_to(output, &mut unlimited())
            .expect("frame commit must succeed");
        if commit.committed {
            assert!(matches!(
                commit.status,
                Some(ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument)
            ));
            return commit;
        }
    }
    panic!("viewport frame did not settle");
}

fn count(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|window| *window == needle)
        .count()
}

fn long_document(with_table: bool) -> String {
    let mut source = String::new();
    if with_table {
        source.push_str("| A | B |\n| --- | --- |\n| one | two |\n\n");
    }
    for index in 0..600 {
        source.push_str(&format!(
            "line {index:03} abcdefghijklmnopqrstuvwxyz 0123456789\n\n"
        ));
    }
    source
}

fn color(value: Option<Rgb>) -> Color {
    value.map_or(Color::Default, |value| {
        Color::Rgb(value.red, value.green, value.blue)
    })
}

fn assert_terminal_matches_screen(terminal: &TerminalScreen, screen: &Screen) {
    let (rows, cols) = terminal.size();
    assert_eq!((rows, cols), (screen.rows(), screen.cols()));
    for row in 0..rows {
        for column in 0..cols {
            let expected = screen
                .front_cell(row, column)
                .expect("screen front cell must exist");
            let actual = terminal
                .cell(row, column)
                .expect("terminal cell must exist");
            if expected.is_continuation() {
                assert!(
                    actual.text.is_empty() || actual.text == " ",
                    "row {row} column {column}: unexpected continuation text {:?}",
                    actual.text
                );
            } else if expected.text().trim().is_empty() {
                assert!(
                    actual.text.trim().is_empty(),
                    "row {row} column {column}: expected blank, got {:?}",
                    actual.text
                );
            } else {
                assert_eq!(
                    actual.text,
                    expected.text(),
                    "row {row} column {column}: text mismatch"
                );
            }

            let style = MONOKAI.resolve(expected.style());
            assert_eq!(actual.foreground, color(style.foreground));
            assert_eq!(actual.background, color(style.background));
            assert_eq!(actual.bold, style.bold);
            assert_eq!(actual.italic, style.italic);
            assert_eq!(
                actual.underline,
                style.underline != Underline::None,
                "row {row} column {column}: underline mismatch"
            );
            assert_eq!(actual.wide, expected.width() == 2);
            assert_eq!(actual.wide_continuation, expected.is_continuation());
            assert!(!actual.dim);
            assert!(!actual.inverse);
        }
    }
}

#[test]
fn first_frame_clears_once_and_resize_width_frames_are_diff_only() {
    let document = long_document(false);
    let mut runtime = FrameRuntime::from_finished_source(
        &document,
        FrameGeometry {
            rows: 40,
            cols: 120,
            content_width: 120,
        },
        LinkMode::Default,
    )
    .unwrap();
    let mut terminal = TerminalScreen::new(40, 120);
    let mut all = Vec::new();

    let mut first = Vec::new();
    let first_commit = commit(&mut runtime, &mut first);
    assert!(first_commit.cleared);
    assert!(first_commit.first_frame);
    assert!(!first_commit.full_repaint);
    assert!(first.starts_with(FIRST_FRAME_CLEAR));
    assert_eq!(count(&first, CLEAR), 1);
    assert_eq!(count(&first, HOME), 1);
    terminal.feed(&first);
    all.extend_from_slice(&first);
    assert_terminal_matches_screen(&terminal, runtime.screen());

    assert!(runtime.scroll_down(5).unwrap());
    let mut scrolled = Vec::new();
    let scrolled_commit = commit(&mut runtime, &mut scrolled);
    assert!(!scrolled_commit.cleared);
    assert!(!scrolled_commit.full_repaint);
    assert_eq!(count(&scrolled, CLEAR), 0);
    assert_eq!(count(&scrolled, HOME), 0);
    terminal.feed(&scrolled);
    all.extend_from_slice(&scrolled);
    assert_terminal_matches_screen(&terminal, runtime.screen());

    let resize = runtime.resize(24, 80).unwrap();
    assert!(resize.height_changed);
    assert!(resize.width_changed);
    assert_eq!(resize.content_width, 80);
    terminal.resize(24, 80);
    let mut resized = Vec::new();
    let resized_commit = commit(&mut runtime, &mut resized);
    assert!(!resized_commit.cleared);
    assert!(resized_commit.full_repaint);
    assert_eq!(count(&resized, CLEAR), 0);
    assert_eq!(count(&resized, HOME), 0);
    terminal.feed(&resized);
    all.extend_from_slice(&resized);
    assert_terminal_matches_screen(&terminal, runtime.screen());

    let restored = runtime.resize(40, 120).unwrap();
    assert!(restored.height_changed);
    assert!(restored.width_changed);
    assert_eq!(restored.content_width, 120);
    terminal.resize(40, 120);
    let mut restored_output = Vec::new();
    let restored_commit = commit(&mut runtime, &mut restored_output);
    assert!(!restored_commit.cleared);
    assert!(restored_commit.full_repaint);
    assert_eq!(count(&restored_output, CLEAR), 0);
    assert_eq!(count(&restored_output, HOME), 0);
    terminal.feed(&restored_output);
    all.extend_from_slice(&restored_output);
    assert_terminal_matches_screen(&terminal, runtime.screen());

    let width = runtime.set_content_width(100).unwrap();
    assert!(width.changed);
    assert_eq!(width.width, 100);
    let mut narrowed = Vec::new();
    let narrowed_commit = commit(&mut runtime, &mut narrowed);
    assert!(!narrowed_commit.cleared);
    assert!(narrowed_commit.full_repaint);
    assert_eq!(count(&narrowed, CLEAR), 0);
    assert_eq!(count(&narrowed, HOME), 0);
    terminal.feed(&narrowed);
    all.extend_from_slice(&narrowed);
    assert_terminal_matches_screen(&terminal, runtime.screen());

    assert_eq!(count(&all, CLEAR), 1);
    assert_eq!(count(&all, HOME), 1);
}

#[test]
fn empty_diff_commits_clear_once_and_emit_no_synchronized_update() {
    let mut runtime = FrameRuntime::new(
        FrameGeometry {
            rows: 4,
            cols: 20,
            content_width: 20,
        },
        LinkMode::Default,
    )
    .unwrap();

    let mut first = Vec::new();
    let first_commit = runtime.commit_empty_frame(&mut first).unwrap();
    assert!(first_commit.committed);
    assert!(first_commit.cleared);
    assert_eq!(first, FIRST_FRAME_CLEAR);
    assert_eq!(count(&first, BEGIN_SYNCHRONIZED_UPDATE), 0);
    assert_eq!(count(&first, END_SYNCHRONIZED_UPDATE), 0);

    let mut second = Vec::new();
    let second_commit = runtime.commit_empty_frame(&mut second).unwrap();
    assert!(second_commit.committed);
    assert!(!second_commit.cleared);
    assert_eq!(second_commit.bytes, 0);
    assert_eq!(second_commit.output, Default::default());
    assert!(second.is_empty());
}

#[test]
fn content_width_change_invalidates_width_caches_and_preserves_top_source() {
    let document = long_document(true);
    let mut runtime = FrameRuntime::from_finished_source(
        &document,
        FrameGeometry {
            rows: 12,
            cols: 80,
            content_width: 80,
        },
        LinkMode::Default,
    )
    .unwrap();
    let mut terminal = TerminalScreen::new(12, 80);
    let mut first = Vec::new();
    commit(&mut runtime, &mut first);
    terminal.feed(&first);
    assert!(runtime.scroll_down(5).unwrap());
    let mut scrolled = Vec::new();
    commit(&mut runtime, &mut scrolled);
    terminal.feed(&scrolled);

    let top_before = runtime.top_source().expect("scrolled top source");
    let tile = TileId(1);
    let height_ticket = runtime
        .viewport()
        .heights()
        .ticket(tile)
        .expect("height ticket");
    runtime
        .viewport_mut()
        .checkpoints_mut()
        .insert(tile, RenderCheckpoint::line_start())
        .unwrap();
    let checkpoint_token = runtime
        .viewport()
        .checkpoints()
        .token(tile)
        .expect("checkpoint token");

    {
        let (document, viewport) = runtime.document_and_viewport_mut();
        let mut budget = unlimited();
        let measured = viewport
            .table_widths_mut()
            .observe_visible_rows(document, TableId(0), 80, 0..1, &mut budget)
            .unwrap();
        assert!(measured.cells_read > 0);
        let stats = viewport
            .table_widths_mut()
            .stats(document, TableId(0), 80)
            .unwrap();
        assert!(stats.rows_read > 0);
    }

    let change = runtime.set_content_width(50).unwrap();
    assert!(change.changed);
    assert_eq!(change.previous_width, 80);
    assert_eq!(change.width, 50);
    assert_eq!(change.top_source, Some(top_before));
    assert_eq!(change.estimated_rows, CONTENT_WIDTH_RESET_ESTIMATE);
    assert_eq!(runtime.geometry().content_width, 50);
    assert_eq!(runtime.top_source(), Some(top_before));
    assert!(!runtime.viewport().has_pending_frame());

    for row in 0..runtime.screen().rows() {
        assert!(
            runtime
                .screen()
                .front_row(row)
                .unwrap()
                .iter()
                .all(|cell| cell.same_visual(&Cell::blank())),
            "width reset must clear diff baseline row {row}"
        );
    }

    let viewport = runtime.viewport_mut();
    assert_eq!(
        viewport.heights().height_state(tile),
        Some(HeightState::Estimated)
    );
    assert_eq!(
        viewport.heights_mut().measure(height_ticket, 1),
        Err(LayoutError::StaleMeasurement)
    );
    assert!(!viewport.checkpoints().validates(checkpoint_token));
    assert!(viewport.checkpoints().exact(tile).is_none());
    {
        let (document, viewport) = runtime.document_and_viewport_mut();
        let stats = viewport
            .table_widths_mut()
            .stats(document, TableId(0), 50)
            .unwrap();
        assert_eq!(stats.rows_read, 0);
        assert_eq!(stats.cells_read, 0);
    }

    let mut narrowed = Vec::new();
    let narrow_commit = commit(&mut runtime, &mut narrowed);
    assert!(!narrow_commit.cleared);
    assert!(narrow_commit.full_repaint);
    terminal.feed(&narrowed);
    assert_eq!(runtime.top_source(), Some(top_before));
    assert!(runtime.viewport().config().width == 50);
    assert_terminal_matches_screen(&terminal, runtime.screen());
}

#[test]
fn height_only_resize_preserves_width_tokens_and_remeasures_screen() {
    let document = long_document(true);
    let mut runtime = FrameRuntime::from_finished_source(
        &document,
        FrameGeometry {
            rows: 10,
            cols: 80,
            content_width: 80,
        },
        LinkMode::Default,
    )
    .unwrap();
    let mut terminal = TerminalScreen::new(10, 80);
    let mut first = Vec::new();
    commit(&mut runtime, &mut first);
    terminal.feed(&first);
    assert!(runtime.scroll_down(3).unwrap());
    let mut scrolled = Vec::new();
    commit(&mut runtime, &mut scrolled);
    terminal.feed(&scrolled);
    let top_before = runtime.top_source().expect("top source");

    let tile = TileId(1);
    let height_ticket = runtime
        .viewport()
        .heights()
        .ticket(tile)
        .expect("height ticket");
    runtime
        .viewport_mut()
        .checkpoints_mut()
        .insert(tile, RenderCheckpoint::line_start())
        .unwrap();
    let checkpoint_token = runtime
        .viewport()
        .checkpoints()
        .token(tile)
        .expect("checkpoint token");
    {
        let (document, viewport) = runtime.document_and_viewport_mut();
        let mut budget = unlimited();
        viewport
            .table_widths_mut()
            .observe_visible_rows(document, TableId(0), 80, 0..1, &mut budget)
            .unwrap();
    }

    let resize = runtime.resize(20, 80).unwrap();
    assert!(resize.height_changed);
    assert!(!resize.width_changed);
    assert_eq!(resize.content_width, 80);
    let viewport = runtime.viewport_mut();
    assert_eq!(viewport.config().width, 80);
    assert!(
        viewport.heights_mut().measure(height_ticket, 1).is_ok(),
        "height-only resize must preserve width measurement tickets"
    );
    assert!(viewport.checkpoints().validates(checkpoint_token));
    {
        let (document, viewport) = runtime.document_and_viewport_mut();
        let stats = viewport
            .table_widths_mut()
            .stats(document, TableId(0), 80)
            .unwrap();
        assert!(stats.rows_read > 0);
    }

    terminal.resize(20, 80);
    let mut resized = Vec::new();
    let resized_commit = commit(&mut runtime, &mut resized);
    assert!(!resized_commit.cleared);
    assert!(resized_commit.full_repaint);
    terminal.feed(&resized);
    assert_eq!(runtime.top_source(), Some(top_before));
    assert_terminal_matches_screen(&terminal, runtime.screen());
}
