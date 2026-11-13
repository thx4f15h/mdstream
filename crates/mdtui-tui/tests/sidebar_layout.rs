mod support;

use std::time::Duration;

use crossterm::event::{KeyCode, KeyModifiers};
use mdtui_tui::app::AppState;
use mdtui_tui::cli::LinkMode;
use mdtui_tui::render::frame::{FrameCommit, FrameGeometry, FrameRuntime};
use mdtui_tui::screen::{Screen, StyleRole};
use mdtui_tui::static_render::render_static;
use mdtui_tui::stream::{IngestResult, StreamEngine};
use mdtui3::{Budget, WorkLimit};
use support::terminal::{FrameOutcome, Vt100Writer, assert_terminal_matches_screen};

const WIDTH: u16 = 120;
const HEIGHT: u16 = 40;

fn geometry(rows: u16, cols: u16) -> FrameGeometry {
    FrameGeometry {
        rows,
        cols,
        content_width: cols,
    }
}

fn unlimited() -> Budget {
    Budget::new(WorkLimit::unlimited())
}

fn commit(runtime: &mut FrameRuntime, output: &mut Vec<u8>) -> FrameCommit {
    let commit = runtime.frame_to(output, &mut unlimited()).expect("frame");
    assert!(commit.committed);
    commit
}

fn commit_due(engine: &mut StreamEngine, writer: &mut Vt100Writer) -> FrameCommit {
    writer.begin_frame();
    match engine.render_due(writer, &mut unlimited(), Duration::ZERO) {
        Ok(Some(commit)) if commit.committed => {
            writer.finish_frame(FrameOutcome::Committed);
            commit
        }
        Ok(_) => {
            writer.finish_frame(FrameOutcome::NoOutput);
            panic!("expected a committed frame");
        }
        Err(error) => {
            writer.finish_frame(FrameOutcome::Failed);
            panic!("unexpected frame error: {error}");
        }
    }
}

fn onscreen_text(screen: &Screen, row: u16, start: u16, end: u16) -> String {
    let mut text = String::new();
    for column in start..end {
        let cell = screen.front_cell(row, column).expect("screen cell");
        if !cell.is_continuation() {
            text.push_str(&cell.text().collect_text());
        }
    }
    text
}

fn find_in_region(screen: &Screen, start: u16, end: u16, needle: &str) -> Option<(u16, u16)> {
    let first = if start == 0 && end < screen.cols() {
        2
    } else {
        0
    };
    for row in first..screen.rows() {
        let text = onscreen_text(screen, row, start, end);
        if let Some(byte) = text.find(needle) {
            let column = start + u16::try_from(text[..byte].chars().count()).unwrap();
            return Some((row, column));
        }
    }
    None
}

fn assert_sidebar_styles_stay_left(screen: &Screen, content_x: u16) {
    for row in 0..screen.rows() {
        for column in 0..screen.cols() {
            let style = screen.front_cell(row, column).unwrap().style();
            if style.contains(StyleRole::SIDEBAR)
                || style.contains(StyleRole::SIDEBAR_CURRENT)
                || style.contains(StyleRole::SIDEBAR_SELECTION)
            {
                assert!(
                    column < content_x,
                    "sidebar style leaked to row {row} column {column}"
                );
            }
        }
    }
}

fn assert_border(screen: &Screen, border: u16) {
    for row in 0..screen.rows() {
        let cell = screen.front_cell(row, border).expect("border cell");
        assert_eq!(cell.text(), "│");
        assert_eq!(cell.style(), StyleRole::TABLE_BORDER);
    }
}

fn assert_body_matches_static(runtime: &FrameRuntime, content_width: u16, content_x: u16) {
    let rows = runtime.screen().rows();
    let expected = render_static(runtime.document(), content_width, rows).expect("static body");
    for row in 0..rows {
        for column in 0..content_width {
            let actual = runtime
                .screen()
                .front_cell(row, content_x + column)
                .expect("body cell");
            let expected = expected.screen.cell(row, column).expect("static body cell");
            assert!(
                actual.same_visual(expected),
                "body mismatch at row {row} column {column}: actual {:?} expected {:?}",
                actual.text(),
                expected.text()
            );
        }
    }
}

fn layout_source() -> &'static str {
    "# Sidebar Alpha deliberately overwide heading that must clip exactly at the body right boundary without entering sidebar columns\n\nBODY_ONLY_TOKEN wraps across the body region without entering the sidebar columns at all.\n\n## Beta\n\n| A | B | C |\n| --- | --- | --- |\n| TABLE_ONLY_VALUE | two | three |\n"
}

fn flat_layout_source() -> &'static str {
    "# Sidebar Alpha deliberately overwide heading that must clip exactly at the body right boundary without entering sidebar columns\n\nBODY_ONLY_TOKEN wraps across the body region without entering the sidebar columns at all.\n\n# Beta\n\n| A | B | C |\n| --- | --- | --- |\n| TABLE_ONLY_VALUE | two | three |\n"
}

#[test]
fn sidebar_layout_widths_and_narrow_hiding_are_exact() {
    for (cols, expected) in [
        (20, None),
        (40, Some((20, 20))),
        (80, Some((24, 56))),
        (120, Some((36, 84))),
        (160, Some((40, 120))),
    ] {
        let mut runtime = FrameRuntime::new(geometry(24, cols), LinkMode::Default).unwrap();
        let layout = runtime.set_sidebar_visible(true).unwrap();
        assert_eq!(layout.visible, expected.is_some(), "cols {cols}");
        if let Some((sidebar, body)) = expected {
            assert_eq!(layout.sidebar_width, sidebar, "cols {cols}");
            assert_eq!(layout.content_x, sidebar, "cols {cols}");
            assert_eq!(layout.content_width, body, "cols {cols}");
            assert_eq!(runtime.content_x(), sidebar);
            assert_eq!(runtime.geometry().content_width, body);
        } else {
            assert_eq!(layout.sidebar_width, 0, "cols {cols}");
            assert_eq!(layout.content_x, 0, "cols {cols}");
            assert_eq!(layout.content_width, cols, "cols {cols}");
        }
    }

    let mut runtime = FrameRuntime::new(geometry(24, 120), LinkMode::Default).unwrap();
    runtime.set_content_width(100).unwrap();
    assert!(!runtime.sidebar_visible());
    assert_eq!(runtime.content_x(), 0);
    assert_eq!(runtime.geometry().content_width, 100);
}

#[test]
fn sidebar_and_body_share_a_frame_without_overlap() {
    let source = flat_layout_source();
    let mut runtime =
        FrameRuntime::from_finished_source(source, geometry(HEIGHT, WIDTH), LinkMode::Default)
            .unwrap();
    let layout = runtime.set_sidebar_visible(true).unwrap();
    assert!(layout.visible);
    let mut output = Vec::new();
    commit(&mut runtime, &mut output);

    let screen = runtime.screen();
    assert_border(screen, layout.content_x - 1);
    assert_sidebar_styles_stay_left(screen, layout.content_x);
    assert_eq!(
        runtime.sidebar_current_heading(),
        Some(mdtui3::HeadingId(0))
    );
    assert!(find_in_region(screen, 0, layout.content_x, "Sidebar Alpha").is_some());
    assert!(find_in_region(screen, 0, layout.content_x, "BODY_ONLY_TOKEN").is_none());
    assert!(find_in_region(screen, layout.content_x, screen.cols(), "BODY_ONLY_TOKEN").is_some());
    let (row, column) = find_in_region(screen, 0, layout.content_x, "Sidebar Alpha").unwrap();
    assert!(
        screen
            .front_cell(row, column)
            .unwrap()
            .style()
            .contains(StyleRole::SIDEBAR_CURRENT)
    );
    assert_body_matches_static(&runtime, layout.content_width, layout.content_x);

    let mut terminal = support::terminal::TerminalScreen::new(HEIGHT, WIDTH);
    terminal.feed(&output);
    assert_terminal_matches_screen(&terminal, runtime.screen());
}

#[test]
fn body_wrap_and_table_prefix_follow_body_width_at_80_and_120() {
    for cols in [80, 120] {
        let mut runtime = FrameRuntime::from_finished_source(
            layout_source(),
            geometry(HEIGHT, cols),
            LinkMode::Default,
        )
        .unwrap();
        let layout = runtime.set_sidebar_visible(true).unwrap();
        assert!(layout.visible);
        let mut output = Vec::new();
        commit(&mut runtime, &mut output);
        assert_body_matches_static(&runtime, layout.content_width, layout.content_x);
        assert!(
            find_in_region(runtime.screen(), layout.content_x, cols, "TABLE_ONLY_VALUE").is_some()
        );
        assert!(
            find_in_region(runtime.screen(), 0, layout.content_x, "TABLE_ONLY_VALUE").is_none()
        );
        assert_border(runtime.screen(), layout.content_x - 1);
    }
}

#[test]
fn current_heading_highlight_tracks_the_body_top() {
    let mut source = String::from("# Alpha\n\nalpha body\n\n");
    for index in 0..30 {
        source.push_str(&format!("alpha paragraph {index}\n\n"));
    }
    source.push_str("# Beta\n\nbeta body\n");
    for index in 0..30 {
        source.push_str(&format!("beta paragraph {index}\n\n"));
    }
    let mut runtime =
        FrameRuntime::from_finished_source(&source, geometry(HEIGHT, WIDTH), LinkMode::Default)
            .unwrap();
    let layout = runtime.set_sidebar_visible(true).unwrap();
    let mut output = Vec::new();
    commit(&mut runtime, &mut output);
    assert_eq!(
        runtime.sidebar_current_heading(),
        Some(mdtui3::HeadingId(0))
    );
    let (row, column) = find_in_region(runtime.screen(), 0, layout.content_x, "Alpha").unwrap();
    assert!(
        runtime
            .screen()
            .front_cell(row, column)
            .unwrap()
            .style()
            .contains(StyleRole::SIDEBAR_CURRENT)
    );

    assert!(runtime.scroll_down(u64::MAX).unwrap());
    commit(&mut runtime, &mut output);
    assert_eq!(
        runtime.sidebar_current_heading(),
        Some(mdtui3::HeadingId(1))
    );
    let (row, column) = find_in_region(runtime.screen(), 0, layout.content_x, "Beta").unwrap();
    assert!(
        runtime
            .screen()
            .front_cell(row, column)
            .unwrap()
            .style()
            .contains(StyleRole::SIDEBAR_CURRENT)
    );
}

#[test]
fn empty_heading_set_renders_only_the_sidebar_border() {
    let mut runtime = FrameRuntime::from_finished_source(
        "BODY_ONLY no headings here\n",
        geometry(HEIGHT, WIDTH),
        LinkMode::Default,
    )
    .unwrap();
    let layout = runtime.set_sidebar_visible(true).unwrap();
    let mut output = Vec::new();
    commit(&mut runtime, &mut output);
    assert_eq!(runtime.sidebar_current_heading(), None);
    assert_border(runtime.screen(), layout.content_x - 1);
    assert_sidebar_styles_stay_left(runtime.screen(), layout.content_x);
    assert!(find_in_region(runtime.screen(), 0, layout.content_x, "BODY_ONLY").is_none());
    assert!(find_in_region(runtime.screen(), layout.content_x, WIDTH, "BODY_ONLY").is_some());
}

#[test]
fn sidebar_indentation_follows_heading_level() {
    let mut runtime = FrameRuntime::from_finished_source(
        "# One\n\n## Two\n\n### Three\n",
        geometry(HEIGHT, WIDTH),
        LinkMode::Default,
    )
    .unwrap();
    let layout = runtime.set_sidebar_visible(true).unwrap();
    let mut output = Vec::new();
    commit(&mut runtime, &mut output);
    for (text, expected_column) in [("One", 2), ("Two", 4), ("Three", 6)] {
        let (row, column) = find_in_region(runtime.screen(), 0, layout.content_x, text).unwrap();
        assert_eq!(column, expected_column, "heading {text}");
        assert!(row < HEIGHT);
    }
}

#[test]
fn real_sidebar_key_toggle_renders_and_clears_the_sidebar() {
    let source = flat_layout_source();
    let mut engine =
        StreamEngine::new(FrameRuntime::new(geometry(HEIGHT, WIDTH), LinkMode::Default).unwrap());
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    assert_eq!(
        engine
            .ingest_data(source.as_bytes(), Duration::ZERO)
            .unwrap(),
        IngestResult::FirstFrame
    );
    commit_due(&mut engine, &mut writer);
    let top = engine.runtime().top_source();

    let mut app = AppState::default();
    let opened = app
        .handle_key(&mut engine, KeyCode::Char('s'), KeyModifiers::NONE)
        .unwrap();
    assert!(opened.redraw);
    assert!(engine.runtime().sidebar_visible());
    assert_eq!(engine.runtime().content_x(), 36);
    commit_due(&mut engine, &mut writer);
    assert!(find_in_region(engine.runtime().screen(), 0, 36, "Sidebar Alpha").is_some());
    assert_eq!(engine.runtime().top_source(), top);
    assert_terminal_matches_screen(writer.terminal(), engine.runtime().screen());

    let closed = app
        .handle_key(&mut engine, KeyCode::Char('s'), KeyModifiers::NONE)
        .unwrap();
    assert!(closed.redraw);
    assert!(!engine.runtime().sidebar_visible());
    assert_eq!(engine.runtime().content_x(), 0);
    commit_due(&mut engine, &mut writer);
    assert_sidebar_styles_stay_left(engine.runtime().screen(), 0);
    assert_eq!(engine.runtime().top_source(), top);
    assert_terminal_matches_screen(writer.terminal(), engine.runtime().screen());
}
