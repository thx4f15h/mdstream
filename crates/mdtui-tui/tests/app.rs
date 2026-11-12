mod support;

use std::time::Duration;

use crossterm::event::{KeyCode, KeyModifiers};
use mdtui_tui::app::{AppState, FocusTarget, KeyAction, content_width, sidebar_width};
use mdtui_tui::cli::LinkMode;
use mdtui_tui::render::checkpoint::RenderCheckpoint;
use mdtui_tui::render::frame::{FrameCommit, FrameGeometry, FrameRuntime};
use mdtui_tui::stream::{IngestResult, MAX_BATCH_DELAY, StreamEngine};
use mdtui3::{Budget, HeightState, LayoutError, TileId, WorkLimit};
use support::terminal::{FrameOutcome, Vt100Writer, assert_terminal_matches_screen};

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

fn settle_after_width_change(engine: &mut StreamEngine, writer: &mut Vt100Writer) {
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
    commit_due(engine, writer, Duration::ZERO);
}

fn long_source() -> String {
    let mut source = String::from("# TOP_MARK\n\n");
    for index in 0..160 {
        source.push_str(&format!(
            "paragraph {index:03} abcdefghijklmnopqrstuvwxyz\n\n"
        ));
    }
    source.push_str("BOTTOM_MARK\n");
    source
}

fn initialized_engine(source: &str, writer: &mut Vt100Writer) -> StreamEngine {
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

fn started_engine(source: &str, writer: &mut Vt100Writer) -> StreamEngine {
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
    engine
}

#[test]
fn page_home_and_end_change_the_viewport_and_request_a_frame() {
    let source = long_source();
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    let mut engine = initialized_engine(&source, &mut writer);
    let mut app = AppState::default();
    let top = engine.runtime().top_source();

    let page = app
        .handle_action(&mut engine, KeyAction::PageDown)
        .expect("page down");
    assert!(page.redraw);
    assert!(engine.due(Duration::ZERO));
    assert_ne!(engine.runtime().top_source(), top);
    commit_due(&mut engine, &mut writer, Duration::ZERO);
    let paged_top = engine.runtime().top_source();

    let end = app.handle_action(&mut engine, KeyAction::End).expect("end");
    assert!(end.redraw);
    assert!(engine.runtime().top_source() > paged_top);
    commit_due(&mut engine, &mut writer, Duration::ZERO);

    let home = app
        .handle_action(&mut engine, KeyAction::Home)
        .expect("home");
    assert!(home.redraw);
    assert_eq!(engine.runtime().top_source(), top);
}

#[test]
fn sidebar_toggle_preserves_top_source_and_invalidates_width_tokens() {
    let source = long_source();
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    let mut engine = initialized_engine(&source, &mut writer);
    let mut app = AppState::default();

    engine.runtime_mut().scroll_down(5).expect("scroll");
    engine.request_render();
    commit_due(&mut engine, &mut writer, Duration::ZERO);
    let top = engine.runtime().top_source().expect("top source");

    let tile = TileId(1);
    let ticket = engine
        .runtime()
        .viewport()
        .heights()
        .ticket(tile)
        .expect("height ticket");
    engine
        .runtime_mut()
        .viewport_mut()
        .checkpoints_mut()
        .insert(tile, RenderCheckpoint::line_start())
        .expect("checkpoint insert");
    let checkpoint = engine
        .runtime()
        .viewport()
        .checkpoints()
        .token(tile)
        .expect("checkpoint token");

    let opened = app
        .handle_key(&mut engine, KeyCode::Char('s'), KeyModifiers::NONE)
        .expect("open sidebar");
    assert!(opened.redraw);
    assert!(app.sidebar_visible());
    assert_eq!(
        engine.runtime().geometry().content_width,
        content_width(WIDTH, true)
    );
    assert_eq!(engine.runtime().geometry().content_width, 84);
    assert_eq!(engine.runtime().top_source(), Some(top));
    assert_eq!(
        engine.runtime().viewport().heights().height_state(tile),
        Some(HeightState::Estimated)
    );
    assert_eq!(
        engine
            .runtime_mut()
            .viewport_mut()
            .heights_mut()
            .measure(ticket, 1),
        Err(LayoutError::StaleMeasurement)
    );
    assert!(
        !engine
            .runtime()
            .viewport()
            .checkpoints()
            .validates(checkpoint)
    );
    settle_after_width_change(&mut engine, &mut writer);
    assert_terminal_matches_screen(writer.terminal(), engine.runtime().screen());
    assert_eq!(engine.runtime().top_source(), Some(top));

    let ticket = engine
        .runtime()
        .viewport()
        .heights()
        .ticket(tile)
        .expect("height ticket after reflow");
    engine
        .runtime_mut()
        .viewport_mut()
        .checkpoints_mut()
        .insert(tile, RenderCheckpoint::line_start())
        .expect("checkpoint insert after reflow");
    let checkpoint = engine
        .runtime()
        .viewport()
        .checkpoints()
        .token(tile)
        .expect("checkpoint token after reflow");

    let closed = app
        .handle_key(&mut engine, KeyCode::Char('s'), KeyModifiers::NONE)
        .expect("close sidebar");
    assert!(closed.redraw);
    assert!(!app.sidebar_visible());
    assert_eq!(engine.runtime().geometry().content_width, WIDTH);
    assert_eq!(engine.runtime().top_source(), Some(top));
    assert_eq!(
        engine
            .runtime_mut()
            .viewport_mut()
            .heights_mut()
            .measure(ticket, 1),
        Err(LayoutError::StaleMeasurement)
    );
    assert!(
        !engine
            .runtime()
            .viewport()
            .checkpoints()
            .validates(checkpoint)
    );
}

#[test]
fn focus_cycle_activation_and_empty_history_are_stable() {
    let source = long_source();
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    let mut engine = initialized_engine(&source, &mut writer);
    let mut app = AppState::default();
    let top = engine.runtime().top_source();

    let no_links = app
        .handle_key(&mut engine, KeyCode::Tab, KeyModifiers::NONE)
        .expect("tab");
    assert!(!no_links.redraw);
    assert_eq!(app.focus(), FocusTarget::Body);
    assert_eq!(engine.link_focus(), None);
    let activation = app
        .handle_key(&mut engine, KeyCode::Enter, KeyModifiers::NONE)
        .expect("activate");
    assert_eq!(activation.activated, Some(FocusTarget::Body));
    assert!(!activation.redraw);

    app.handle_key(&mut engine, KeyCode::BackTab, KeyModifiers::NONE)
        .expect("backtab");
    assert_eq!(app.focus(), FocusTarget::Body);
    app.handle_key(&mut engine, KeyCode::Char('s'), KeyModifiers::NONE)
        .expect("open sidebar");
    assert!(app.sidebar_visible());
    app.handle_key(&mut engine, KeyCode::Tab, KeyModifiers::NONE)
        .expect("body to sidebar");
    assert_eq!(app.focus(), FocusTarget::Sidebar);
    app.handle_key(&mut engine, KeyCode::Char('s'), KeyModifiers::NONE)
        .expect("close sidebar");
    assert_eq!(app.focus(), FocusTarget::Body);

    for action in [KeyAction::HistoryBack, KeyAction::HistoryForward] {
        let outcome = app
            .handle_action(&mut engine, action)
            .expect("history no-op");
        assert!(!outcome.quit);
        assert!(!outcome.redraw);
        assert_eq!(outcome.activated, None);
        assert!(app.history().is_empty());
        assert_eq!(engine.runtime().top_source(), top);
    }

    let enter = app
        .handle_key(&mut engine, KeyCode::Enter, KeyModifiers::NONE)
        .expect("enter on body");
    assert_eq!(enter.activated, Some(FocusTarget::Body));
}

#[test]
fn narrow_terminal_keeps_sidebar_hidden_and_focus_stable() {
    assert_eq!(sidebar_width(39), None);
    let four_twenty = content_width(120, true);
    assert_eq!(four_twenty, 84);
    assert!(sidebar_width(40).is_some());
    assert!(sidebar_width(160).is_some());

    let mut engine = StreamEngine::new(
        FrameRuntime::new(
            FrameGeometry {
                rows: 24,
                cols: 30,
                content_width: 30,
            },
            LinkMode::Default,
        )
        .unwrap(),
    );
    let mut app = AppState::default();
    app.handle_key(&mut engine, KeyCode::Tab, KeyModifiers::NONE)
        .expect("tab to link");
    assert_eq!(app.focus(), FocusTarget::Body);
    assert_eq!(engine.link_focus(), None);
    let outcome = app
        .handle_key(&mut engine, KeyCode::Char('s'), KeyModifiers::NONE)
        .expect("narrow sidebar toggle");
    assert!(!outcome.redraw);
    assert!(!app.sidebar_visible());
    assert_eq!(engine.runtime().geometry().content_width, 30);
    assert_eq!(app.focus(), FocusTarget::Body);
}

#[test]
fn sidebar_reflow_frame_keeps_the_committed_screen_in_sync() {
    let source = long_source();
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    let mut engine = initialized_engine(&source, &mut writer);
    let mut app = AppState::default();
    let first_frame = writer.frames().len();

    app.handle_key(&mut engine, KeyCode::Char('s'), KeyModifiers::NONE)
        .expect("open sidebar");
    settle_after_width_change(&mut engine, &mut writer);

    assert_eq!(writer.frames().len(), first_frame + 1);
    assert!(writer.frames().last().unwrap().outcome == Some(FrameOutcome::Committed));
    assert_terminal_matches_screen(writer.terminal(), engine.runtime().screen());
    let last: Vec<u8> = writer
        .frame_writes(writer.frames().len() - 1)
        .iter()
        .flat_map(|write| write.bytes.iter().copied())
        .collect();
    assert!(!last.windows(4).any(|window| window == b"\x1b[2J"));
    assert!(!last.windows(3).any(|window| window == b"\x1b[H"));
}

#[test]
fn down_and_up_are_boundary_safe_and_nonblocking() {
    let source = long_source();
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    let mut engine = started_engine(&source, &mut writer);
    let mut app = AppState::default();

    let down = app
        .handle_key(&mut engine, KeyCode::Down, KeyModifiers::NONE)
        .expect("down");
    assert!(down.redraw);
    assert!(engine.due(Duration::ZERO));
    commit_due(&mut engine, &mut writer, Duration::ZERO);

    let up = app
        .handle_key(&mut engine, KeyCode::Up, KeyModifiers::NONE)
        .expect("up");
    assert!(up.redraw);
    assert!(engine.due(Duration::ZERO));

    let now = Duration::from_millis(1);
    let updated = engine
        .ingest_data(b"\nreader still advances\n", now)
        .expect("reader update after keys");
    assert_eq!(updated, IngestResult::Applied);
    engine
        .ingest_data(b"", now + MAX_BATCH_DELAY)
        .expect("empty reader update");
}
