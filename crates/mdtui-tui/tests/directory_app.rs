use crossterm::event::{KeyCode as K, KeyModifiers as M};
use mdtui_tui::app::{AppState, FocusTarget};
use mdtui_tui::cli::LinkMode;
use mdtui_tui::render::frame::{FrameGeometry, FrameRuntime};
use mdtui_tui::stream::StreamEngine;
use mdtui3::{Budget, HeadingId, TableId, WorkLimit};

fn setup(source: &str) -> (AppState, StreamEngine) {
    let runtime = FrameRuntime::from_finished_source(
        source,
        FrameGeometry {
            rows: 12,
            cols: 120,
            content_width: 120,
        },
        LinkMode::Default,
    )
    .unwrap();
    let mut engine = StreamEngine::new(runtime);
    let app = AppState::default();
    paint(&mut engine);
    (app, engine)
}
fn paint(engine: &mut StreamEngine) {
    for _ in 0..10000 {
        let frame = engine
            .runtime_mut()
            .frame_to(&mut Vec::new(), &mut Budget::new(WorkLimit::unlimited()))
            .unwrap();
        if frame.committed
            && !engine.runtime().sidebar_pending()
            && !engine.runtime().has_pending_jump()
        {
            return;
        }
    }
    panic!("pending");
}
fn key(app: &mut AppState, engine: &mut StreamEngine, k: K, m: M) -> bool {
    let result = app.handle_key(engine, k, m).unwrap();
    assert!(!result.quit);
    paint(engine);
    app.frame_committed(engine);
    result.redraw
}
fn sidebar(app: &mut AppState, engine: &mut StreamEngine) {
    key(app, engine, K::Char('s'), M::NONE);
    key(app, engine, K::Tab, M::NONE);
    assert_eq!(app.focus(), FocusTarget::Sidebar);
}
#[test]
fn sidebar_input_routes_q_unicode_delete_limit_cancel_and_commit_without_typing_scans() {
    let (mut app, mut engine) = setup("# q汉\n\n## Child\n\n# Next\n");
    sidebar(&mut app, &mut engine);
    key(&mut app, &mut engine, K::Char('/'), M::NONE);
    let before = engine.runtime().directory().unwrap().work();
    let bytes = engine.runtime().sidebar_stats().unwrap().source_bytes_read;
    for c in ['q', '汉', 's', '0'] {
        assert!(
            !app.handle_key(&mut engine, K::Char(c), M::NONE)
                .unwrap()
                .quit
        );
    }
    assert_eq!(
        engine.runtime().directory().unwrap().editing(),
        Some("q汉s0")
    );
    app.handle_key(&mut engine, K::Delete, M::NONE).unwrap();
    app.handle_key(&mut engine, K::Backspace, M::NONE).unwrap();
    assert_eq!(engine.runtime().directory().unwrap().editing(), Some("q汉"));
    app.handle_key(&mut engine, K::Tab, M::NONE).unwrap();
    assert_eq!(app.focus(), FocusTarget::Body);
    app.handle_key(&mut engine, K::BackTab, M::NONE).unwrap();
    assert_eq!(app.focus(), FocusTarget::Sidebar);
    app.handle_key(&mut engine, K::Left, M::ALT).unwrap();
    assert_eq!(engine.runtime().directory().unwrap().editing(), Some("q汉"));

    assert_eq!(engine.runtime().directory().unwrap().work(), before);
    assert_eq!(
        engine.runtime().sidebar_stats().unwrap().source_bytes_read,
        bytes
    );
    key(&mut app, &mut engine, K::Enter, M::NONE);
    assert_eq!(engine.runtime().directory().unwrap().query(), "q汉");
    assert_eq!(
        engine
            .runtime()
            .directory()
            .unwrap()
            .entries()
            .map(|e| e.id.0)
            .collect::<Vec<_>>(),
        [0]
    );
    key(&mut app, &mut engine, K::Char('/'), M::NONE);
    key(&mut app, &mut engine, K::Char('x'), M::NONE);
    key(&mut app, &mut engine, K::Esc, M::NONE);
    assert_eq!(engine.runtime().directory().unwrap().query(), "q汉");
    key(&mut app, &mut engine, K::Esc, M::NONE);
    assert_eq!(engine.runtime().directory().unwrap().query(), "");
    key(&mut app, &mut engine, K::Char('/'), M::NONE);
    assert!(
        app.handle_key(&mut engine, K::Char('c'), M::CONTROL)
            .unwrap()
            .quit
    );
}
#[test]
fn tree_keys_preserve_table_pan_and_history_routes() {
    let (mut app, mut engine) = setup(
        "# Root\n\n| A | B | C | D | E | F | G | H | I | J | K | L |\n| - | - | - | - | - | - | - | - | - | - | - | - |\n| x | y | z | 4 | 5 | 6 | 7 | 8 | 9 | 0 | 1 | 2 |\n\n## Child\n\n# End\n",
    );
    key(&mut app, &mut engine, K::Right, M::NONE);
    let pan = engine
        .runtime()
        .viewport()
        .table_widths()
        .first_column(TableId(0));
    assert_eq!(pan, 1);
    sidebar(&mut app, &mut engine);
    let pan = engine
        .runtime()
        .viewport()
        .table_widths()
        .first_column(TableId(0));
    key(&mut app, &mut engine, K::Left, M::NONE);
    assert!(
        engine
            .runtime()
            .directory()
            .unwrap()
            .collapsed(HeadingId(0))
    );
    key(&mut app, &mut engine, K::Right, M::CONTROL);
    assert_eq!(
        engine
            .runtime()
            .viewport()
            .table_widths()
            .first_column(TableId(0)),
        pan
    );
    key(&mut app, &mut engine, K::Right, M::NONE);
    key(&mut app, &mut engine, K::Right, M::NONE);
    assert_eq!(engine.runtime().sidebar_selection(), Some(HeadingId(1)));
    // History seeds its first target by the existing application contract.
    key(&mut app, &mut engine, K::End, M::NONE);
    key(&mut app, &mut engine, K::Enter, M::NONE);
    key(&mut app, &mut engine, K::Home, M::NONE);
    key(&mut app, &mut engine, K::Enter, M::NONE);
    key(&mut app, &mut engine, K::Right, M::NONE);
    let source = engine
        .document()
        .headings()
        .get(HeadingId(1))
        .unwrap()
        .source
        .start;
    key(&mut app, &mut engine, K::Enter, M::NONE);
    assert_eq!(engine.runtime().top_source(), Some(source));
    let history = app.history().len();
    assert!(history >= 2);
    key(&mut app, &mut engine, K::Left, M::ALT);
    assert_ne!(engine.runtime().top_source(), Some(source));
    key(&mut app, &mut engine, K::Right, M::ALT);
    assert_eq!(engine.runtime().top_source(), Some(source));
    assert_eq!(app.history().len(), history);
}
#[test]
fn breadcrumb_tracks_body_current_with_no_source_or_link_hit_and_resize_keeps_choices() {
    let (mut app, mut engine) =
        setup("# Outer\n\n## Inner\n\n> # Deep\n>\n> body\n\n# Last\n\ntrailer\n");
    sidebar(&mut app, &mut engine);
    key(&mut app, &mut engine, K::End, M::NONE);
    assert_eq!(engine.runtime().sidebar_selection(), Some(HeadingId(3)));
    assert_eq!(
        engine.runtime().directory().unwrap().current(),
        Some(HeadingId(0))
    );
    assert!(
        engine
            .runtime()
            .directory()
            .unwrap()
            .breadcrumb_text()
            .contains("Outer")
    );
    for row in 0..2 {
        for column in 0..engine.runtime().content_x() - 1 {
            let cell = engine.runtime().screen().front_cell(row, column).unwrap();
            assert_eq!(cell.source(), None);
            assert_eq!(cell.link_id(), None);
        }
    }
    key(&mut app, &mut engine, K::Home, M::NONE);
    key(&mut app, &mut engine, K::Left, M::NONE);
    let cut = engine.runtime().top_source();
    app.resize(&mut engine, 24, 80).unwrap();
    paint(&mut engine);
    assert_eq!(engine.runtime().top_source(), cut);
    assert!(
        engine
            .runtime()
            .directory()
            .unwrap()
            .collapsed(HeadingId(0))
    );
    assert_eq!(
        engine
            .runtime()
            .directory()
            .unwrap()
            .entries()
            .map(|e| e.id.0)
            .collect::<Vec<_>>(),
        [0, 3]
    );
    key(&mut app, &mut engine, K::Char('s'), M::NONE);
    key(&mut app, &mut engine, K::Char('s'), M::NONE);
    assert!(
        engine
            .runtime()
            .directory()
            .unwrap()
            .collapsed(HeadingId(0))
    );
}
#[test]
fn queued_end_enter_and_micro_frames_finish_the_exact_jump() {
    let (mut app, mut engine) = setup("# A\n\n## B\n\n# C\n");
    sidebar(&mut app, &mut engine);
    app.handle_key(&mut engine, K::End, M::NONE).unwrap();
    let micro = WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 6,
        tiles: 1,
        recovery: 1,
        dependencies: 1,
    };
    engine
        .runtime_mut()
        .activate_sidebar_selection(&mut Budget::new(micro))
        .unwrap();
    assert!(engine.runtime().has_pending_jump());
    let mut frames = 0;
    while engine.runtime().has_pending_jump() {
        let mut b = Budget::new(micro);
        engine
            .runtime_mut()
            .frame_to(&mut Vec::new(), &mut b)
            .unwrap();
        frames += 1;
        assert!(frames < 10000);
        assert!(b.used().queries <= 1 && b.used().objects <= 1 && b.used().bytes <= 6);
    }
    assert_eq!(engine.runtime().sidebar_selection(), Some(HeadingId(2)));
    assert_eq!(
        engine.runtime().top_source(),
        Some(
            engine
                .document()
                .headings()
                .get(HeadingId(2))
                .unwrap()
                .source
                .start
        )
    );
}

#[test]
fn cancelling_a_pending_sidebar_selection_never_seeds_jump_history() {
    let mut source = String::from("# Root\n\n");
    for depth in 1..=1000 {
        source.push_str(&format!("{} # H{depth}\n", ">".repeat(depth)));
    }
    let (mut app, mut engine) = setup(&source);
    sidebar(&mut app, &mut engine);
    let before = engine.runtime().top_source();
    app.handle_key(&mut engine, K::End, M::NONE).unwrap();
    app.handle_key(&mut engine, K::Enter, M::NONE).unwrap();
    assert!(engine.runtime().has_pending_jump());
    app.handle_key(&mut engine, K::Char('1'), M::NONE).unwrap();
    assert!(!engine.runtime().has_pending_jump());
    paint(&mut engine);
    app.frame_committed(&engine);
    assert_eq!(engine.runtime().top_source(), before);
    assert!(app.history().is_empty());
}
