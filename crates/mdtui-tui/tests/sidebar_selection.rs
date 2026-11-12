mod support;

use std::time::Duration;

use crossterm::event::{KeyCode, KeyModifiers};
use mdtui_tui::app::{AppState, FocusTarget};
use mdtui_tui::cli::LinkMode;
use mdtui_tui::render::frame::{FrameCommit, FrameGeometry, FrameRuntime};
use mdtui_tui::screen::StyleRole;
use mdtui_tui::stream::{IngestResult, MAX_BATCH_DELAY, StreamEngine};
use mdtui3::{Budget, HeadingId, WorkLimit};
use support::terminal::{FrameOutcome, Vt100Writer};

const WIDTH: u16 = 120;
const HEIGHT: u16 = 40;

fn geometry() -> FrameGeometry {
    FrameGeometry {
        rows: HEIGHT,
        cols: WIDTH,
        content_width: WIDTH,
    }
}

fn unlimited() -> Budget {
    Budget::new(WorkLimit::unlimited())
}

fn commit_due(engine: &mut StreamEngine, writer: &mut Vt100Writer, now: Duration) -> FrameCommit {
    writer.begin_frame();
    match engine.render_due(writer, &mut unlimited(), now) {
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

fn settle(engine: &mut StreamEngine, writer: &mut Vt100Writer, now: Duration) {
    if engine.calibration_pending() {
        for _ in 0..100_000 {
            if engine
                .advance_calibration(&mut unlimited())
                .expect("calibration must succeed")
                .is_some()
            {
                break;
            }
        }
    }
    if engine.due(now) {
        commit_due(engine, writer, now);
    }
}

fn finished_engine(source: &str, writer: &mut Vt100Writer) -> StreamEngine {
    let mut engine = StreamEngine::new(
        FrameRuntime::new(geometry(), LinkMode::Default).expect("runtime must initialize"),
    );
    assert_eq!(
        engine
            .ingest_data(source.as_bytes(), Duration::ZERO)
            .unwrap(),
        IngestResult::FirstFrame
    );
    commit_due(&mut engine, writer, Duration::ZERO);
    assert!(engine.finish_input(Duration::ZERO).unwrap());
    for _ in 0..100_000 {
        if engine
            .advance_calibration(&mut unlimited())
            .expect("calibration must succeed")
            .is_some()
        {
            break;
        }
    }
    commit_due(&mut engine, writer, Duration::ZERO);
    engine
}

fn open_sidebar(app: &mut AppState, engine: &mut StreamEngine, writer: &mut Vt100Writer) {
    let outcome = app
        .handle_key(engine, KeyCode::Char('s'), KeyModifiers::NONE)
        .expect("open sidebar");
    assert!(outcome.redraw);
    settle(engine, writer, Duration::ZERO);
    if engine.runtime().sidebar_pending() {
        engine.request_render();
        commit_due(engine, writer, Duration::ZERO);
    }
}

fn many_heading_source() -> String {
    let mut source = String::new();
    for index in 0..100 {
        source.push_str(&format!("# Heading {index:03}\n\nbody {index}\n\n"));
    }
    source.push_str(&"x".repeat(200_000));
    source
}

fn sidebar_style(engine: &StreamEngine, text: &str) -> StyleRole {
    let screen = engine.runtime().screen();
    let sidebar_width = engine.runtime().content_x();
    let mut rows = Vec::new();
    for row in 0..screen.rows() {
        let mut line = String::new();
        for column in 0..sidebar_width {
            let cell = screen.front_cell(row, column).expect("sidebar cell");
            if !cell.is_continuation() {
                line.push_str(cell.text());
            }
        }
        rows.push(line.clone());
        if let Some(byte) = line.find(text) {
            let column = u16::try_from(line[..byte].chars().count()).expect("column");
            return screen
                .front_cell(row, column)
                .expect("heading cell")
                .style();
        }
    }
    panic!("sidebar text {text:?} was not visible in {rows:?}");
}

#[test]
fn sidebar_page_selection_and_window_move_without_body_scroll() {
    let source = many_heading_source();
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    let mut engine = finished_engine(&source, &mut writer);
    let mut app = AppState::default();
    open_sidebar(&mut app, &mut engine, &mut writer);
    assert_eq!(engine.runtime().content_x(), 36);
    let body_top = engine.runtime().top_source();
    assert_eq!(engine.runtime().sidebar_selection(), Some(HeadingId(0)));
    assert_eq!(engine.runtime().sidebar_window_start(), Some(0));

    app.handle_key(&mut engine, KeyCode::Tab, KeyModifiers::NONE)
        .unwrap();
    assert_eq!(app.focus(), FocusTarget::Sidebar);
    let page = app
        .handle_key(&mut engine, KeyCode::PageDown, KeyModifiers::NONE)
        .unwrap();
    assert!(page.redraw);
    settle(&mut engine, &mut writer, Duration::ZERO);
    assert_eq!(engine.runtime().sidebar_selection(), Some(HeadingId(40)));
    assert_eq!(engine.runtime().sidebar_window_start(), Some(21));
    assert_eq!(engine.runtime().top_source(), body_top);

    let second = app
        .handle_key(&mut engine, KeyCode::PageDown, KeyModifiers::NONE)
        .unwrap();
    assert!(second.redraw);
    settle(&mut engine, &mut writer, Duration::ZERO);
    assert_eq!(engine.runtime().sidebar_selection(), Some(HeadingId(80)));
    assert_eq!(engine.runtime().sidebar_window_start(), Some(61));
    assert_eq!(engine.runtime().top_source(), body_top);

    app.handle_key(&mut engine, KeyCode::BackTab, KeyModifiers::NONE)
        .unwrap();
    assert_eq!(app.focus(), FocusTarget::Body);
    let body_page = app
        .handle_key(&mut engine, KeyCode::End, KeyModifiers::NONE)
        .unwrap();
    assert!(body_page.redraw);
    settle(&mut engine, &mut writer, Duration::ZERO);
    assert!(engine.runtime().top_source() > body_top);
    assert_eq!(engine.runtime().sidebar_selection(), Some(HeadingId(80)));
    assert_eq!(engine.runtime().sidebar_window_start(), Some(61));
    assert_ne!(
        engine.runtime().sidebar_current_heading(),
        Some(HeadingId(80))
    );
}

#[test]
fn sidebar_home_end_and_boundaries_only_move_the_sidebar_selection() {
    let source = many_heading_source();
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    let mut engine = finished_engine(&source, &mut writer);
    let mut app = AppState::default();
    open_sidebar(&mut app, &mut engine, &mut writer);
    app.handle_key(&mut engine, KeyCode::Tab, KeyModifiers::NONE)
        .unwrap();
    let body_top = engine.runtime().top_source();

    let end = app
        .handle_key(&mut engine, KeyCode::End, KeyModifiers::NONE)
        .unwrap();
    assert!(end.redraw);
    settle(&mut engine, &mut writer, Duration::ZERO);
    assert_eq!(engine.runtime().sidebar_selection(), Some(HeadingId(99)));
    assert_eq!(engine.runtime().sidebar_window_start(), Some(62));
    assert_eq!(engine.runtime().top_source(), body_top);

    let boundary = app
        .handle_key(&mut engine, KeyCode::Down, KeyModifiers::NONE)
        .unwrap();
    assert!(!boundary.redraw);
    assert_eq!(engine.runtime().sidebar_selection(), Some(HeadingId(99)));

    let home = app
        .handle_key(&mut engine, KeyCode::Home, KeyModifiers::NONE)
        .unwrap();
    assert!(home.redraw);
    settle(&mut engine, &mut writer, Duration::ZERO);
    assert_eq!(engine.runtime().sidebar_selection(), Some(HeadingId(0)));
    assert_eq!(engine.runtime().sidebar_window_start(), Some(0));
    assert_eq!(engine.runtime().top_source(), body_top);

    let boundary = app
        .handle_key(&mut engine, KeyCode::Up, KeyModifiers::NONE)
        .unwrap();
    assert!(!boundary.redraw);
    assert_eq!(engine.runtime().sidebar_selection(), Some(HeadingId(0)));
}

#[test]
fn current_and_selection_have_distinct_styles_and_independent_windows() {
    let mut source = String::new();
    for index in 0..100 {
        source.push_str(&format!("# Heading {index:03}\n\nbody {index:03}\n\n"));
    }
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    let mut engine = finished_engine(&source, &mut writer);
    let mut app = AppState::default();
    open_sidebar(&mut app, &mut engine, &mut writer);

    assert_eq!(engine.runtime().body_current_heading(), Some(HeadingId(0)));
    assert_eq!(
        engine.runtime().sidebar_current_heading(),
        Some(HeadingId(0))
    );
    assert_eq!(engine.runtime().sidebar_selection(), Some(HeadingId(0)));
    let initial = sidebar_style(&engine, "Heading 000");
    assert!(initial.contains(StyleRole::SIDEBAR_CURRENT));
    assert!(initial.contains(StyleRole::SIDEBAR_SELECTION));

    app.handle_key(&mut engine, KeyCode::Tab, KeyModifiers::NONE)
        .expect("focus sidebar");
    assert_eq!(app.focus(), FocusTarget::Sidebar);
    app.handle_key(&mut engine, KeyCode::Down, KeyModifiers::NONE)
        .expect("select next heading");
    settle(&mut engine, &mut writer, Duration::ZERO);
    assert_eq!(engine.runtime().body_current_heading(), Some(HeadingId(0)));
    assert_eq!(
        engine.runtime().sidebar_current_heading(),
        Some(HeadingId(0))
    );
    assert_eq!(engine.runtime().sidebar_selection(), Some(HeadingId(1)));
    assert_eq!(engine.runtime().sidebar_window_start(), Some(0));
    let current = sidebar_style(&engine, "Heading 000");
    assert!(current.contains(StyleRole::SIDEBAR_CURRENT));
    assert!(!current.contains(StyleRole::SIDEBAR_SELECTION));
    let selected = sidebar_style(&engine, "Heading 001");
    assert!(!selected.contains(StyleRole::SIDEBAR_CURRENT));
    assert!(selected.contains(StyleRole::SIDEBAR_SELECTION));

    app.handle_key(&mut engine, KeyCode::BackTab, KeyModifiers::NONE)
        .expect("focus body");
    assert_eq!(app.focus(), FocusTarget::Body);
    app.handle_key(&mut engine, KeyCode::End, KeyModifiers::NONE)
        .expect("scroll body to end");
    settle(&mut engine, &mut writer, Duration::ZERO);
    let scrolled_current = engine.runtime().body_current_heading();
    assert!(scrolled_current.is_some());
    assert_ne!(scrolled_current, Some(HeadingId(1)));
    assert_eq!(engine.runtime().sidebar_current_heading(), None);
    assert_eq!(engine.runtime().sidebar_selection(), Some(HeadingId(1)));
    assert_eq!(engine.runtime().sidebar_window_start(), Some(0));
    let no_fallback = sidebar_style(&engine, "Heading 001");
    assert!(!no_fallback.contains(StyleRole::SIDEBAR_CURRENT));
    assert!(no_fallback.contains(StyleRole::SIDEBAR_SELECTION));

    let body_top = engine.runtime().top_source();
    app.handle_key(&mut engine, KeyCode::Tab, KeyModifiers::NONE)
        .expect("focus sidebar after body scroll");
    app.handle_key(&mut engine, KeyCode::PageDown, KeyModifiers::NONE)
        .expect("move sidebar selection");
    settle(&mut engine, &mut writer, Duration::ZERO);
    assert_eq!(engine.runtime().sidebar_selection(), Some(HeadingId(41)));
    assert_eq!(engine.runtime().sidebar_window_start(), Some(22));
    assert_eq!(engine.runtime().body_current_heading(), scrolled_current);
    assert_eq!(engine.runtime().sidebar_current_heading(), None);
    assert_eq!(engine.runtime().top_source(), body_top);
    let selected = sidebar_style(&engine, "Heading 041");
    assert!(!selected.contains(StyleRole::SIDEBAR_CURRENT));
    assert!(selected.contains(StyleRole::SIDEBAR_SELECTION));
}

#[test]
fn selection_survives_toggle_resize_and_append_without_becoming_current() {
    let mut source = String::new();
    for index in 0..20 {
        source.push_str(&format!(
            "# Heading {index:03}\n\nbody {index:03}\n\nfiller {index:03}\n\n"
        ));
    }
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    let mut engine = StreamEngine::new(
        FrameRuntime::new(geometry(), LinkMode::Default).expect("runtime must initialize"),
    );
    assert_eq!(
        engine
            .ingest_data(source.as_bytes(), Duration::ZERO)
            .expect("ingest"),
        IngestResult::FirstFrame
    );
    commit_due(&mut engine, &mut writer, Duration::ZERO);
    let mut app = AppState::default();
    open_sidebar(&mut app, &mut engine, &mut writer);
    app.handle_key(&mut engine, KeyCode::Tab, KeyModifiers::NONE)
        .expect("focus sidebar");
    app.handle_key(&mut engine, KeyCode::PageDown, KeyModifiers::NONE)
        .expect("select last heading");
    settle(&mut engine, &mut writer, Duration::ZERO);
    assert_eq!(engine.runtime().sidebar_selection(), Some(HeadingId(19)));
    assert_eq!(engine.runtime().body_current_heading(), Some(HeadingId(0)));

    app.handle_key(&mut engine, KeyCode::Char('s'), KeyModifiers::NONE)
        .expect("close sidebar");
    settle(&mut engine, &mut writer, Duration::ZERO);
    assert!(!app.sidebar_visible());
    assert_eq!(engine.runtime().sidebar_selection(), Some(HeadingId(19)));
    app.handle_key(&mut engine, KeyCode::Char('s'), KeyModifiers::NONE)
        .expect("reopen sidebar");
    settle(&mut engine, &mut writer, Duration::ZERO);
    assert!(app.sidebar_visible());
    assert_eq!(engine.runtime().sidebar_selection(), Some(HeadingId(19)));
    assert_eq!(engine.runtime().body_current_heading(), Some(HeadingId(0)));

    app.resize(&mut engine, 24, 80).expect("resize");
    settle(&mut engine, &mut writer, Duration::ZERO);
    assert_eq!(engine.runtime().sidebar_selection(), Some(HeadingId(19)));
    assert_eq!(engine.runtime().body_current_heading(), Some(HeadingId(0)));

    let now = Duration::from_millis(100);
    assert_eq!(
        engine
            .ingest_data(b"\n# Heading 020\n\nbody 020\n", now)
            .expect("append heading"),
        IngestResult::Applied
    );
    settle(&mut engine, &mut writer, now + MAX_BATCH_DELAY);
    assert_eq!(engine.runtime().sidebar_selection(), Some(HeadingId(19)));
    assert_eq!(engine.runtime().body_current_heading(), Some(HeadingId(0)));

    assert_eq!(engine.runtime().body_current_heading(), Some(HeadingId(0)));
    assert_eq!(
        engine.runtime().sidebar_current_heading(),
        Some(HeadingId(0))
    );
    assert_eq!(engine.runtime().sidebar_selection(), Some(HeadingId(19)));
    let current = sidebar_style(&engine, "Heading 000");
    assert!(current.contains(StyleRole::SIDEBAR_CURRENT));
    assert!(!current.contains(StyleRole::SIDEBAR_SELECTION));
    let selected = sidebar_style(&engine, "Heading 019");
    assert!(!selected.contains(StyleRole::SIDEBAR_CURRENT));
    assert!(selected.contains(StyleRole::SIDEBAR_SELECTION));
}

#[test]
fn enter_activates_selected_heading_source_and_preserves_selection() {
    let source =
        "# Alpha\n\nalpha body\n\n# Beta\n\nbeta body\n\nbeta tail\n\n# Gamma\n\ngamma body\n";
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    let mut engine = finished_engine(source, &mut writer);
    let mut app = AppState::default();
    open_sidebar(&mut app, &mut engine, &mut writer);
    app.handle_key(&mut engine, KeyCode::Tab, KeyModifiers::NONE)
        .unwrap();
    app.handle_key(&mut engine, KeyCode::Down, KeyModifiers::NONE)
        .unwrap();
    settle(&mut engine, &mut writer, Duration::ZERO);
    let beta = engine.document().headings().entries()[1];
    assert_eq!(engine.runtime().sidebar_selection(), Some(HeadingId(1)));

    let enter = app
        .handle_key(&mut engine, KeyCode::Enter, KeyModifiers::NONE)
        .unwrap();
    assert!(enter.redraw);
    settle(&mut engine, &mut writer, Duration::ZERO);
    assert_eq!(engine.runtime().top_source(), Some(beta.source.start));
    assert_eq!(engine.runtime().sidebar_selection(), Some(HeadingId(1)));
    assert_eq!(
        engine.runtime().sidebar_current_heading(),
        Some(HeadingId(1))
    );
    assert_eq!(engine.runtime().sidebar_window_start(), Some(0));
}

#[test]
fn enter_with_no_heading_selection_is_a_noop() {
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    let mut engine = finished_engine("plain body without headings\n", &mut writer);
    let mut app = AppState::default();
    open_sidebar(&mut app, &mut engine, &mut writer);
    app.handle_key(&mut engine, KeyCode::Tab, KeyModifiers::NONE)
        .unwrap();
    assert_eq!(app.focus(), FocusTarget::Sidebar);
    let top = engine.runtime().top_source();
    let enter = app
        .handle_key(&mut engine, KeyCode::Enter, KeyModifiers::NONE)
        .unwrap();
    assert!(!enter.redraw);
    assert_eq!(engine.runtime().top_source(), top);
    assert_eq!(engine.runtime().sidebar_selection(), None);

    for key in [KeyCode::Home, KeyCode::End, KeyCode::Up, KeyCode::PageDown] {
        let outcome = app
            .handle_key(&mut engine, key, KeyModifiers::NONE)
            .expect("empty sidebar navigation");
        assert!(!outcome.redraw);
        assert_eq!(engine.runtime().top_source(), top);
        assert_eq!(engine.runtime().sidebar_selection(), None);
    }
}

#[test]
fn heading_activation_moves_body_and_keeps_the_sidebar_window() {
    let mut source = String::new();
    for index in 0..20 {
        source.push_str(&format!(
            "# Heading {index:02}\n\nbody paragraph {index:02}\n\n"
        ));
    }
    let mut runtime = FrameRuntime::from_finished_source(
        &source,
        FrameGeometry {
            rows: 8,
            cols: WIDTH,
            content_width: WIDTH,
        },
        LinkMode::Default,
    )
    .unwrap();
    assert!(runtime.set_sidebar_visible(true).unwrap().visible);
    let mut output = Vec::new();
    runtime
        .frame_to(&mut output, &mut unlimited())
        .expect("initial sidebar frame");
    assert!(runtime.sidebar_move_selection(10));
    runtime
        .frame_to(&mut output, &mut unlimited())
        .expect("selection frame");
    assert_eq!(runtime.sidebar_selection(), Some(HeadingId(10)));
    assert_eq!(runtime.sidebar_window_start(), Some(7));
    let target = runtime.document().headings().entries()[10];

    assert!(
        runtime
            .activate_sidebar_selection(&mut unlimited())
            .expect("activate heading")
    );
    assert_eq!(runtime.top_source(), Some(target.source.start));
    assert_eq!(runtime.sidebar_selection(), Some(HeadingId(10)));
    assert_eq!(runtime.sidebar_window_start(), Some(7));
    assert!(!runtime.viewport().top_position().is_approximate());

    runtime
        .frame_to(&mut output, &mut unlimited())
        .expect("activated frame");
    assert_eq!(runtime.sidebar_current_heading(), Some(HeadingId(10)));
    assert_eq!(runtime.sidebar_selection(), Some(HeadingId(10)));
    assert_eq!(runtime.sidebar_window_start(), Some(7));
}

#[test]
fn failed_heading_activation_does_not_move_and_a_retry_succeeds() {
    let mut source = String::from("# Alpha\n\nalpha body\n\n");
    for index in 0..200 {
        source.push_str(&format!("filler paragraph {index:03}\n\n"));
    }
    source.push_str("# Destination\n\ndestination body\n");
    let mut runtime =
        FrameRuntime::from_finished_source(&source, geometry(), LinkMode::Default).unwrap();
    assert!(runtime.set_sidebar_visible(true).unwrap().visible);
    let mut output = Vec::new();
    runtime
        .frame_to(&mut output, &mut unlimited())
        .expect("initial sidebar frame");
    assert!(runtime.sidebar_selection_end());
    runtime
        .frame_to(&mut output, &mut unlimited())
        .expect("paid selection frame");
    let selected = runtime.sidebar_selection();
    let top = runtime.top_source();
    let target = runtime
        .document()
        .headings()
        .get(selected.expect("destination selected"))
        .copied()
        .expect("destination heading");
    let mut tiny = Budget::new(WorkLimit {
        queries: 1,
        tiles: 1,
        objects: 1,
        bytes: 8,
        recovery: 1,
        ..WorkLimit::default()
    });
    assert!(
        !runtime
            .activate_sidebar_selection(&mut tiny)
            .expect("constrained activation")
    );
    assert_eq!(runtime.top_source(), top);
    assert_eq!(runtime.sidebar_selection(), selected);

    assert!(
        runtime
            .activate_sidebar_selection(&mut unlimited())
            .expect("retry activation")
    );
    assert_eq!(runtime.top_source(), Some(target.source.start));
    assert_eq!(runtime.sidebar_selection(), selected);
}

#[test]
fn runtime_heading_update_reuses_visible_entries_and_adds_new_heading() {
    let source = "# Alpha\n\nalpha body\n\n# Beta\n\nbeta body\n";
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    let mut engine = StreamEngine::new(
        FrameRuntime::new(geometry(), LinkMode::Default).expect("runtime must initialize"),
    );
    let mut app = AppState::default();
    assert_eq!(
        engine
            .ingest_data(source.as_bytes(), Duration::ZERO)
            .unwrap(),
        IngestResult::FirstFrame
    );
    commit_due(&mut engine, &mut writer, Duration::ZERO);
    open_sidebar(&mut app, &mut engine, &mut writer);
    let before = engine.runtime().sidebar_stats().expect("sidebar stats");

    let at = Duration::from_millis(100);
    assert_eq!(
        engine
            .ingest_data(b"\nunrelated paragraph\n\n", at)
            .unwrap(),
        IngestResult::Applied
    );
    let after_paragraph = engine.runtime().sidebar_stats().expect("sidebar stats");
    assert_eq!(after_paragraph.excerpts_created, before.excerpts_created);
    assert_eq!(after_paragraph.source_bytes_read, before.source_bytes_read);
    assert_eq!(after_paragraph.reused_entries, before.reused_entries);
    assert_eq!(after_paragraph.rebuilt_entries, before.rebuilt_entries);
    settle(&mut engine, &mut writer, at + MAX_BATCH_DELAY);
    assert!(engine.runtime().sidebar_stats().unwrap().reused_entries > before.reused_entries);

    let at = at + MAX_BATCH_DELAY;
    assert_eq!(
        engine
            .ingest_data(b"\n# Gamma\n\ngamma body\n", at)
            .unwrap(),
        IngestResult::Applied
    );
    let after_heading = engine.runtime().sidebar_stats().expect("sidebar stats");
    assert_eq!(after_heading.excerpts_created, before.excerpts_created);
    assert_eq!(after_heading.source_bytes_read, before.source_bytes_read);
    assert_eq!(after_heading.rebuilt_entries, before.rebuilt_entries);
    assert_eq!(engine.runtime().sidebar_selection(), Some(HeadingId(0)));
    settle(&mut engine, &mut writer, at + MAX_BATCH_DELAY);
    let after_frame = engine.runtime().sidebar_stats().expect("sidebar stats");
    assert_eq!(after_frame.rebuilt_entries, before.rebuilt_entries + 1);
    assert_eq!(after_frame.excerpts_created, before.excerpts_created + 1);
    assert!(engine.runtime().sidebar_window_start().is_some());
}
