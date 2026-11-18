mod support;

use std::time::Duration;

use crossterm::event::{KeyCode, KeyModifiers};
use mdtui_tui::app::{AppState, FocusTarget, KeyAction};
use mdtui_tui::cli::LinkMode;
use mdtui_tui::fragment::{AnchorBuildStatus, FragmentActivationStatus};
use mdtui_tui::render::frame::{FrameGeometry, FrameRuntime, HistoryRestoreStatus};
use mdtui_tui::stream::{IngestResult, StreamEngine};
use mdtui3::{Budget, WorkLimit};
use support::terminal::{FrameOutcome, Vt100Writer};

const WIDTH: u16 = 60;
const HEIGHT: u16 = 8;

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

fn commit_due(engine: &mut StreamEngine, writer: &mut Vt100Writer, now: Duration) {
    writer.begin_frame();
    match engine.render_due(writer, &mut unlimited(), now) {
        Ok(Some(commit)) if commit.committed => writer.finish_frame(FrameOutcome::Committed),
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
                .expect("calibration")
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

fn started_engine(source: &str, writer: &mut Vt100Writer) -> StreamEngine {
    let mut engine = StreamEngine::new(
        FrameRuntime::new(geometry(HEIGHT, WIDTH), LinkMode::Default).expect("runtime"),
    );
    assert_eq!(
        engine
            .ingest_data(source.as_bytes(), Duration::ZERO)
            .expect("ingest"),
        IngestResult::FirstFrame
    );
    commit_due(&mut engine, writer, Duration::ZERO);
    engine
}

fn finished_engine(source: &str, writer: &mut Vt100Writer) -> StreamEngine {
    let mut engine = started_engine(source, writer);
    assert!(engine.finish_input(Duration::ZERO).expect("finish"));
    settle(&mut engine, writer, Duration::ZERO);
    engine
}

fn open_sidebar(app: &mut AppState, engine: &mut StreamEngine, writer: &mut Vt100Writer) {
    app.handle_key(engine, KeyCode::Char('s'), KeyModifiers::NONE)
        .expect("open sidebar");
    assert!(app.sidebar_visible());
    app.handle_key(engine, KeyCode::Tab, KeyModifiers::NONE)
        .expect("focus sidebar");
    assert_eq!(app.focus(), FocusTarget::Sidebar);
    settle(engine, writer, Duration::ZERO);
}

fn activate_sidebar(app: &mut AppState, engine: &mut StreamEngine) {
    let outcome = app
        .handle_key(engine, KeyCode::Enter, KeyModifiers::NONE)
        .expect("activate sidebar selection");
    assert!(outcome.redraw);
}

fn build_anchors(engine: &mut StreamEngine) {
    for _ in 0..100_000 {
        match engine
            .advance_anchor_index(&mut unlimited())
            .expect("anchor build")
        {
            AnchorBuildStatus::Complete => return,
            AnchorBuildStatus::BudgetExhausted => continue,
            AnchorBuildStatus::PendingDocument => panic!("document was not finished"),
        }
    }
    panic!("anchor index did not complete");
}

fn jump_to_second_heading(app: &mut AppState, engine: &mut StreamEngine, writer: &mut Vt100Writer) {
    open_sidebar(app, engine, writer);
    app.handle_key(engine, KeyCode::Down, KeyModifiers::NONE)
        .expect("select second heading");
    activate_sidebar(app, engine);
    settle(engine, writer, Duration::ZERO);
}

fn focus_link_from_sidebar(
    app: &mut AppState,
    engine: &mut StreamEngine,
    writer: &mut Vt100Writer,
) {
    app.handle_key(engine, KeyCode::Tab, KeyModifiers::NONE)
        .expect("focus link");
    assert_eq!(app.focus(), FocusTarget::Link);
    settle(engine, writer, Duration::ZERO);
}

#[test]
fn sequential_heading_jumps_back_and_forward_to_each_source() {
    let source = "# A\n\na\n\n# B\n\nb\n\n# C\n\nc\n";
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    let mut engine = finished_engine(source, &mut writer);
    let headings = engine.document().headings().entries().to_vec();
    assert_eq!(
        engine
            .document()
            .tile_at(headings[0].source.start)
            .expect("A tile"),
        engine
            .document()
            .tile_at(headings[1].source.start)
            .expect("B tile")
    );
    let mut app = AppState::default();
    open_sidebar(&mut app, &mut engine, &mut writer);

    app.handle_key(&mut engine, KeyCode::Down, KeyModifiers::NONE)
        .expect("select B");
    activate_sidebar(&mut app, &mut engine);
    settle(&mut engine, &mut writer, Duration::ZERO);
    assert_eq!(
        engine.runtime().top_source(),
        Some(headings[1].source.start)
    );
    assert_eq!(app.history().len(), 1);
    assert_eq!(app.history().last_source(), Some(headings[1].source.start));

    app.handle_key(&mut engine, KeyCode::Down, KeyModifiers::NONE)
        .expect("select C");
    activate_sidebar(&mut app, &mut engine);
    settle(&mut engine, &mut writer, Duration::ZERO);
    assert_eq!(
        engine.runtime().top_source(),
        Some(headings[2].source.start)
    );
    assert_eq!(app.history().len(), 2);

    let back = app
        .handle_action(&mut engine, KeyAction::HistoryBack)
        .expect("back");
    assert!(back.redraw);
    settle(&mut engine, &mut writer, Duration::ZERO);
    assert_eq!(
        engine.runtime().top_source(),
        Some(headings[1].source.start)
    );
    assert_eq!(app.history().last_source(), Some(headings[1].source.start));

    let forward = app
        .handle_action(&mut engine, KeyAction::HistoryForward)
        .expect("forward");
    assert!(forward.redraw);
    settle(&mut engine, &mut writer, Duration::ZERO);
    assert_eq!(
        engine.runtime().top_source(),
        Some(headings[2].source.start)
    );
    assert_eq!(app.history().last_source(), Some(headings[2].source.start));
}

#[test]
fn body_scroll_refreshes_current_history_without_growing_it() {
    let mut source = String::from("# A\n\na\n\n# B\n\n");
    for index in 0..20 {
        source.push_str(&format!("line {index:02}\n\n"));
    }
    source.push_str("# C\n\nc\n");
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    let mut engine = finished_engine(&source, &mut writer);
    let headings = engine.document().headings().entries().to_vec();
    let mut app = AppState::default();
    open_sidebar(&mut app, &mut engine, &mut writer);
    app.handle_key(&mut engine, KeyCode::Down, KeyModifiers::NONE)
        .expect("select B");
    activate_sidebar(&mut app, &mut engine);
    settle(&mut engine, &mut writer, Duration::ZERO);
    let before = app.history().current().expect("current history");
    assert_eq!(app.history().len(), 1);
    assert_eq!(before.source, headings[1].source.start);

    app.handle_key(&mut engine, KeyCode::Tab, KeyModifiers::NONE)
        .expect("focus body");
    assert_eq!(app.focus(), FocusTarget::Body);

    let down = app
        .handle_key(&mut engine, KeyCode::Down, KeyModifiers::NONE)
        .expect("scroll body");
    assert!(down.redraw);
    assert_eq!(app.history().len(), 1);
    let after = app.history().current().expect("updated history");
    assert_ne!(after, before);
    assert_eq!(after, engine.navigation_position().expect("body position"));
    settle(&mut engine, &mut writer, Duration::ZERO);
}

#[test]
fn a_new_jump_truncates_the_forward_branch() {
    let source = "# A\n\na\n\n# B\n\nb\n\n# C\n\nc\n\n# D\n\nd\n";
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    let mut engine = finished_engine(source, &mut writer);
    let headings = engine.document().headings().entries().to_vec();
    let mut app = AppState::default();
    open_sidebar(&mut app, &mut engine, &mut writer);

    app.handle_key(&mut engine, KeyCode::Down, KeyModifiers::NONE)
        .expect("select B");
    activate_sidebar(&mut app, &mut engine);
    settle(&mut engine, &mut writer, Duration::ZERO);
    app.handle_key(&mut engine, KeyCode::Down, KeyModifiers::NONE)
        .expect("select C");
    activate_sidebar(&mut app, &mut engine);
    settle(&mut engine, &mut writer, Duration::ZERO);

    app.handle_action(&mut engine, KeyAction::HistoryBack)
        .expect("back to B");
    settle(&mut engine, &mut writer, Duration::ZERO);
    assert_eq!(
        engine.runtime().top_source(),
        Some(headings[1].source.start)
    );
    app.handle_action(&mut engine, KeyAction::HistoryForward)
        .expect("forward to C");
    settle(&mut engine, &mut writer, Duration::ZERO);
    assert_eq!(
        engine.runtime().top_source(),
        Some(headings[2].source.start)
    );

    app.handle_action(&mut engine, KeyAction::HistoryBack)
        .expect("back to B again");
    settle(&mut engine, &mut writer, Duration::ZERO);
    app.handle_key(&mut engine, KeyCode::Down, KeyModifiers::NONE)
        .expect("select D");
    activate_sidebar(&mut app, &mut engine);
    settle(&mut engine, &mut writer, Duration::ZERO);
    assert_eq!(
        engine.runtime().top_source(),
        Some(headings[3].source.start)
    );
    assert_eq!(app.history().len(), 2);
    assert!(!app.history().can_go_forward());

    let forward = app
        .handle_action(&mut engine, KeyAction::HistoryForward)
        .expect("truncated forward is a no-op");
    assert!(!forward.redraw);
    assert_eq!(
        engine.runtime().top_source(),
        Some(headings[3].source.start)
    );
}

#[test]
fn append_and_resize_keep_history_sources_restorable() {
    let mut source = String::from("# A\n\na\n\n# B\n\n");
    for index in 0..12 {
        source.push_str(&format!("beta line {index:02}\n\n"));
    }
    source.push_str("# C\n\nc\n");
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    let mut engine = started_engine(&source, &mut writer);
    let headings = engine.document().headings().entries().to_vec();
    let mut app = AppState::default();
    open_sidebar(&mut app, &mut engine, &mut writer);

    app.handle_key(&mut engine, KeyCode::Down, KeyModifiers::NONE)
        .expect("select B");
    activate_sidebar(&mut app, &mut engine);
    settle(&mut engine, &mut writer, Duration::ZERO);
    app.handle_key(&mut engine, KeyCode::Down, KeyModifiers::NONE)
        .expect("select C");
    activate_sidebar(&mut app, &mut engine);
    settle(&mut engine, &mut writer, Duration::ZERO);
    assert_eq!(
        engine.runtime().top_source(),
        Some(headings[2].source.start)
    );
    assert_eq!(
        app.history().current().expect("C history").source,
        headings[2].source.start
    );

    app.resize(&mut engine, 10, 50).expect("resize");
    settle(&mut engine, &mut writer, Duration::ZERO);
    let now = Duration::from_millis(100);
    assert_eq!(
        engine
            .ingest_data(b"\nappended tail\n", now)
            .expect("append"),
        IngestResult::Applied
    );
    settle(&mut engine, &mut writer, now);

    app.handle_action(&mut engine, KeyAction::HistoryBack)
        .expect("back after append");
    settle(&mut engine, &mut writer, now);
    assert_eq!(
        engine.runtime().top_source(),
        Some(headings[1].source.start)
    );

    app.handle_action(&mut engine, KeyAction::HistoryForward)
        .expect("forward after append");
    settle(&mut engine, &mut writer, now);
    assert_eq!(
        engine.runtime().top_source(),
        Some(headings[2].source.start)
    );
}

#[test]
fn same_source_history_offsets_restore_distinct_rows() {
    let source = "one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen seventeen eighteen nineteen twenty twenty-one twenty-two twenty-three twenty-four twenty-five twenty-six twenty-seven twenty-eight twenty-nine thirty thirty-one thirty-two thirty-three thirty-four\n";
    let mut runtime = FrameRuntime::from_finished_source(
        source,
        FrameGeometry {
            rows: 6,
            cols: 20,
            content_width: 20,
        },
        LinkMode::Default,
    )
    .expect("runtime");
    let mut output = Vec::new();
    runtime
        .frame_to(&mut output, &mut unlimited())
        .expect("initial frame");
    let first = runtime.navigation_position().expect("first position");
    assert!(runtime.scroll_down(1).expect("scroll"));
    let second = runtime.navigation_position().expect("second position");
    assert_eq!(first.source, second.source);
    assert_ne!(first.row_offset, second.row_offset);

    assert!(matches!(
        runtime
            .restore_navigation_position(first, &mut unlimited())
            .expect("restore first"),
        HistoryRestoreStatus::Ready { .. }
    ));
    assert_eq!(runtime.navigation_position(), Some(first));
    assert!(matches!(
        runtime
            .restore_navigation_position(second, &mut unlimited())
            .expect("restore second"),
        HistoryRestoreStatus::Ready { .. }
    ));
    assert_eq!(runtime.navigation_position(), Some(second));

    let extreme = mdtui_tui::history::HistoryPosition {
        source: first.source,
        row_offset: u64::MAX,
    };
    let restored = runtime
        .restore_navigation_position(extreme, &mut unlimited())
        .expect("clamped restore");
    let HistoryRestoreStatus::Ready { position, .. } = restored else {
        panic!("expected a ready clamped restore");
    };
    let anchor = runtime.viewport().top_anchor().expect("clamped anchor");
    let height = runtime
        .viewport()
        .heights()
        .height(anchor.tile)
        .expect("clamped tile height");
    assert_eq!(position.row_offset, height.saturating_sub(1));
}

#[test]
fn external_activation_does_not_change_an_existing_history_entry() {
    let source = "# A\n\na\n\n# B\n\n[web](https://example.com/)\n\nb\n";
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    let mut engine = finished_engine(source, &mut writer);
    let mut app = AppState::default();
    jump_to_second_heading(&mut app, &mut engine, &mut writer);
    focus_link_from_sidebar(&mut app, &mut engine, &mut writer);
    let history = app.history().current();
    let top = engine.runtime().top_source();

    let outcome = app
        .handle_key(&mut engine, KeyCode::Enter, KeyModifiers::NONE)
        .expect("external activation");
    assert_eq!(
        outcome.fragment_activation.expect("status").status,
        FragmentActivationStatus::External
    );
    assert_eq!(app.history().current(), history);
    assert_eq!(engine.runtime().top_source(), top);
}

#[test]
fn pending_fragment_does_not_change_an_existing_history_entry() {
    let source = "# A\n\na\n\n# B\n\n[go](#missing)\n\nb\n";
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    let mut engine = started_engine(source, &mut writer);
    let mut app = AppState::default();
    jump_to_second_heading(&mut app, &mut engine, &mut writer);
    focus_link_from_sidebar(&mut app, &mut engine, &mut writer);
    let history = app.history().current();
    let top = engine.runtime().top_source();
    assert!(!engine.document().is_finished());

    let outcome = app
        .handle_key(&mut engine, KeyCode::Enter, KeyModifiers::NONE)
        .expect("pending activation");
    assert_eq!(
        outcome.fragment_activation.expect("status").status,
        FragmentActivationStatus::PendingDocument
    );
    assert_eq!(app.history().current(), history);
    assert_eq!(engine.runtime().top_source(), top);
    let pending_history = app.history().clone();
    settle(&mut engine, &mut writer, Duration::from_secs(1));
    app.frame_committed(&engine);
    assert_eq!(app.history(), &pending_history);
    engine
        .ingest_data(b"\n# Missing\n\nlate target\n", Duration::from_secs(2))
        .unwrap();
    for i in 0..10 {
        settle(
            &mut engine,
            &mut writer,
            Duration::from_secs(3) + Duration::from_millis(i * 40),
        );
        app.frame_committed(&engine);
    }
    let target = engine.document().headings().entries()[2].source.start;
    assert_eq!(app.history().last_source(), Some(target));
    assert_eq!(app.history().len(), pending_history.len() + 1);
    app.frame_committed(&engine);
    assert_eq!(app.history().len(), pending_history.len() + 1);
    assert!(!engine.document().is_finished());
}

#[test]
fn not_found_fragment_does_not_change_an_existing_history_entry() {
    let source = "# A\n\na\n\n# B\n\n[bad](#missing)\n\nb\n";
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    let mut engine = finished_engine(source, &mut writer);
    build_anchors(&mut engine);
    let mut app = AppState::default();
    jump_to_second_heading(&mut app, &mut engine, &mut writer);
    focus_link_from_sidebar(&mut app, &mut engine, &mut writer);
    let history = app.history().current();
    let top = engine.runtime().top_source();

    let outcome = app
        .handle_key(&mut engine, KeyCode::Enter, KeyModifiers::NONE)
        .expect("not found activation");
    assert_eq!(
        outcome.fragment_activation.expect("status").status,
        FragmentActivationStatus::NotFound
    );
    assert_eq!(app.history().current(), history);
    assert_eq!(engine.runtime().top_source(), top);
}

#[test]
fn current_pre_eof_fragment_uses_the_original_existing_heading_sample() {
    // This original history fixture used to wait solely for EOF. It now has a
    // current target before EOF; keep the sample and verify one successful event.
    let source = "# A\n\na\n\n# B\n\n[go](#b)\n\nb\n";
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    let mut engine = started_engine(source, &mut writer);
    let mut app = AppState::default();
    jump_to_second_heading(&mut app, &mut engine, &mut writer);
    focus_link_from_sidebar(&mut app, &mut engine, &mut writer);
    let target = engine.document().headings().entries()[1].source.start;
    let before = app.history().len();
    let activation = app
        .handle_key(&mut engine, KeyCode::Enter, KeyModifiers::NONE)
        .expect("current preEOF activation")
        .fragment_activation
        .unwrap();
    assert!(activation.status.is_success());
    assert_eq!(activation.target.unwrap().source, target);
    assert!(activation.target.unwrap().provisional);
    assert_eq!(app.history().last_source(), Some(target));
    assert!(app.history().len() <= before + 1);
    let recorded = app.history().clone();
    settle(&mut engine, &mut writer, Duration::ZERO);
    app.frame_committed(&engine);
    assert_eq!(app.history(), &recorded);
    assert!(!engine.document().is_finished());
}

#[test]
fn folded_fragment_sidebar_history_and_directory_choices_use_the_same_view() {
    use mdtui3::{FoldId, HeadingId};
    let source = "[jump](#target)\n\n# Outer\nouter\n\n## Target\nHIT_ONE\n\n## Later\nHIT_TWO\n\n# Unrelated\nNOPE\n";
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    let mut engine = finished_engine(source, &mut writer);
    let mut app = AppState::default();
    app.handle_key(&mut engine, KeyCode::Char('z'), KeyModifiers::NONE)
        .unwrap();
    settle_folds(&mut engine, &mut writer);
    for key in [KeyCode::Right, KeyCode::Down, KeyCode::Down, KeyCode::Enter] {
        app.handle_key(&mut engine, key, KeyModifiers::NONE)
            .unwrap();
    }
    settle_folds(&mut engine, &mut writer);
    app.handle_key(&mut engine, KeyCode::Esc, KeyModifiers::NONE)
        .unwrap();
    settle_folds(&mut engine, &mut writer);
    assert!(
        engine
            .runtime()
            .viewport()
            .folds()
            .is_collapsed(FoldId::Heading(HeadingId(0)))
    );
    build_anchors(&mut engine);
    app.handle_key(&mut engine, KeyCode::Tab, KeyModifiers::NONE)
        .unwrap();
    assert_eq!(app.focus(), FocusTarget::Link);
    let activated = app
        .handle_key(&mut engine, KeyCode::Enter, KeyModifiers::NONE)
        .unwrap();
    assert!(activated.fragment_activation.unwrap().status.is_success());
    settle_folds(&mut engine, &mut writer);
    assert_eq!(
        engine.runtime().top_source(),
        Some(source.find("## Target").unwrap())
    );
    assert!(
        !engine
            .runtime()
            .viewport()
            .folds()
            .is_collapsed(FoldId::Heading(HeadingId(0)))
    );
    assert!(
        engine
            .runtime()
            .viewport()
            .folds()
            .is_collapsed(FoldId::Heading(HeadingId(1)))
    );
    assert!(
        engine
            .runtime()
            .viewport()
            .folds()
            .is_collapsed(FoldId::Heading(HeadingId(3)))
    );
    let history = app.history().len();
    assert_eq!(history, 1);
    // No visible link remains at this top; normalize to Body before opening tree.
    app.handle_key(&mut engine, KeyCode::Esc, KeyModifiers::NONE)
        .unwrap();
    assert_eq!(app.focus(), FocusTarget::Body);
    open_sidebar(&mut app, &mut engine, &mut writer);
    app.handle_key(&mut engine, KeyCode::Home, KeyModifiers::NONE)
        .unwrap();
    settle_folds(&mut engine, &mut writer);
    app.handle_key(&mut engine, KeyCode::Left, KeyModifiers::NONE)
        .unwrap();
    settle_folds(&mut engine, &mut writer);
    assert!(
        engine
            .runtime()
            .directory()
            .unwrap()
            .collapsed(HeadingId(0))
    );
    assert!(
        !engine
            .runtime()
            .viewport()
            .folds()
            .is_collapsed(FoldId::Heading(HeadingId(0)))
    );
    app.handle_key(&mut engine, KeyCode::End, KeyModifiers::NONE)
        .unwrap();
    activate_sidebar(&mut app, &mut engine);
    settle_folds(&mut engine, &mut writer);
    assert_eq!(
        engine.runtime().top_source(),
        Some(source.find("# Unrelated").unwrap())
    );
    assert_eq!(app.history().len(), history + 1);
    assert!(
        engine
            .runtime()
            .directory()
            .unwrap()
            .collapsed(HeadingId(0))
    );
    app.handle_action(&mut engine, KeyAction::HistoryBack)
        .unwrap();
    settle_folds(&mut engine, &mut writer);
    app.frame_committed(&engine);
    assert_eq!(
        engine.runtime().top_source(),
        Some(source.find("## Target").unwrap())
    );
    app.handle_action(&mut engine, KeyAction::HistoryForward)
        .unwrap();
    settle_folds(&mut engine, &mut writer);
    app.frame_committed(&engine);
    assert_eq!(
        engine.runtime().top_source(),
        Some(source.find("# Unrelated").unwrap())
    );
    assert!(
        engine
            .runtime()
            .directory()
            .unwrap()
            .collapsed(HeadingId(0))
    );
    // Re-activating the current exact target cannot append another history entry.
    let before = app.history().clone();
    app.handle_key(&mut engine, KeyCode::Enter, KeyModifiers::NONE)
        .unwrap();
    settle_folds(&mut engine, &mut writer);
    app.frame_committed(&engine);
    assert_eq!(app.history(), &before);
}

fn settle_folds(engine: &mut StreamEngine, writer: &mut Vt100Writer) {
    for _ in 0..10000 {
        settle(engine, writer, Duration::ZERO);
        if !engine.runtime().fold_pending() && !engine.runtime().has_pending_jump() {
            return;
        }
    }
    panic!("folded navigation did not settle");
}
