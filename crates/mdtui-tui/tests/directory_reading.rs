use crossterm::event::{KeyCode as K, KeyModifiers as M};
use mdtui_tui::{
    app::{AppState, FocusTarget},
    cli::LinkMode,
    render::frame::{FrameGeometry, FrameRuntime},
    stream::{FRAME_WORK, StreamEngine},
};
use mdtui3::{Budget, HeadingId};

fn setup(text: &str) -> (AppState, StreamEngine) {
    let mut engine = StreamEngine::new(
        FrameRuntime::from_finished_source(
            text,
            FrameGeometry {
                rows: 16,
                cols: 120,
                content_width: 120,
            },
            LinkMode::NoOsc8,
        )
        .unwrap(),
    );
    paint(&mut engine);
    let mut app = AppState::default();
    key(&mut app, &mut engine, K::Char('s'));
    paint(&mut engine);
    (app, engine)
}
fn paint(engine: &mut StreamEngine) -> Vec<u8> {
    let mut out = Vec::new();
    for _ in 0..20000 {
        let c = engine
            .runtime_mut()
            .frame_to(&mut out, &mut Budget::new(FRAME_WORK))
            .unwrap();
        if c.committed
            && !engine.runtime().has_pending_jump()
            && !engine.runtime().reading().pending()
            && !engine.runtime().sidebar_pending()
        {
            return out;
        }
    }
    panic!("combined frame did not settle");
}
fn key(app: &mut AppState, e: &mut StreamEngine, k: K) {
    assert!(!app.handle_key(e, k, M::NONE).unwrap().quit);
}
fn type_text(app: &mut AppState, e: &mut StreamEngine, s: &str) {
    for ch in s.chars() {
        key(app, e, K::Char(ch));
    }
}
#[test]
fn focus_owns_each_query_and_paused_drafts_never_capture_the_other_input() {
    let (mut app, mut e) = setup("intro qbody\n\n# qSide\n\nbody\n");
    key(&mut app, &mut e, K::Char('/'));
    type_text(&mut app, &mut e, "qbody");
    key(&mut app, &mut e, K::Tab);
    assert_eq!(app.focus(), FocusTarget::Sidebar);
    key(&mut app, &mut e, K::Char('/'));
    type_text(&mut app, &mut e, "qSide");
    assert_eq!(e.runtime().reading().input(), Some("qbody"));
    assert_eq!(e.runtime().directory().unwrap().editing(), Some("qSide"));
    key(&mut app, &mut e, K::Enter);
    paint(&mut e);
    assert_eq!(e.runtime().directory().unwrap().query(), "qSide");
    assert_eq!(e.runtime().reading().query(), "");
    key(&mut app, &mut e, K::BackTab);
    assert_eq!(app.focus(), FocusTarget::Body);
    key(&mut app, &mut e, K::Enter);
    paint(&mut e);
    assert_eq!(e.runtime().reading().query(), "qbody");
    assert_eq!(e.runtime().directory().unwrap().query(), "qSide");
    key(&mut app, &mut e, K::Tab);
    let before = e.runtime().reading().current_match().cloned();
    key(&mut app, &mut e, K::Char('n'));
    key(&mut app, &mut e, K::Char('N'));
    key(&mut app, &mut e, K::Esc);
    paint(&mut e);
    assert_eq!(e.runtime().reading().current_match(), before.as_ref());
    assert_eq!(e.runtime().reading().query(), "qbody");
    assert_eq!(e.runtime().directory().unwrap().query(), "");
    key(&mut app, &mut e, K::BackTab);
    key(&mut app, &mut e, K::Esc);
    paint(&mut e);
    assert_eq!(e.runtime().reading().query(), "");
}
#[test]
fn ctrl_c_exits_either_focused_editor_while_q_stays_literal() {
    let (mut app, mut e) = setup("# qSide\n\nqbody\n");
    key(&mut app, &mut e, K::Char('/'));
    key(&mut app, &mut e, K::Char('q'));
    assert_eq!(e.runtime().reading().input(), Some("q"));
    assert!(
        app.handle_key(&mut e, K::Char('c'), M::CONTROL)
            .unwrap()
            .quit
    );
    key(&mut app, &mut e, K::Tab);
    key(&mut app, &mut e, K::Char('/'));
    key(&mut app, &mut e, K::Char('q'));
    assert_eq!(e.runtime().directory().unwrap().editing(), Some("q"));
    assert_eq!(e.runtime().reading().input(), Some("q"));
    assert!(
        app.handle_key(&mut e, K::Char('c'), M::CONTROL)
            .unwrap()
            .quit
    );
}
#[test]
fn visible_and_exact_source_copy_with_sidebar_keep_current_and_selection_distinct() {
    let (mut app, mut e) = setup("a **b&amp;c** d\n\n# Heading\n");
    let selected = e.runtime().sidebar_selection();
    let cut = e.runtime().top_source();
    key(&mut app, &mut e, K::Char('v'));
    key(&mut app, &mut e, K::Char(' '));
    for _ in 0..4 {
        key(&mut app, &mut e, K::Right);
    }
    key(&mut app, &mut e, K::Char('y'));
    let visible = paint(&mut e);
    assert!(
        visible
            .windows(b"\x1b]52;c;YSBiJmM=\x07".len())
            .any(|x| x == b"\x1b]52;c;YSBiJmM=\x07")
    );
    key(&mut app, &mut e, K::Char('Y'));
    let source = paint(&mut e);
    assert!(
        source
            .windows(b"\x1b]52;c;YSAqKmImYW1wO2M=\x07".len())
            .any(|x| x == b"\x1b]52;c;YSAqKmImYW1wO2M=\x07")
    );
    assert_eq!(e.runtime().sidebar_selection(), selected);
    assert_eq!(e.runtime().top_source(), cut);
    key(&mut app, &mut e, K::Tab);
    key(&mut app, &mut e, K::Char('/'));
    type_text(&mut app, &mut e, "Heading");
    key(&mut app, &mut e, K::Enter);
    paint(&mut e);
    assert!(e.runtime().reading().selection.range().is_some());
    assert_eq!(
        e.runtime().directory().unwrap().selection(),
        Some(HeadingId(0))
    );
}
#[test]
fn body_n_and_n_previous_do_not_change_sidebar_name_query_or_selected_heading() {
    let (mut app, mut e) = setup("# Head\n\nneedle one needle two\n\n# Other\n");
    key(&mut app, &mut e, K::Tab);
    key(&mut app, &mut e, K::Char('/'));
    type_text(&mut app, &mut e, "Head");
    key(&mut app, &mut e, K::Enter);
    paint(&mut e);
    let selected = e.runtime().sidebar_selection();
    key(&mut app, &mut e, K::BackTab);
    key(&mut app, &mut e, K::Char('/'));
    type_text(&mut app, &mut e, "needle");
    key(&mut app, &mut e, K::Enter);
    paint(&mut e);
    let first = e.runtime().reading().current_match().unwrap().clone();
    key(&mut app, &mut e, K::Char('n'));
    paint(&mut e);
    assert!(e.runtime().reading().current_match().unwrap().source.start > first.source.start);
    key(&mut app, &mut e, K::Char('N'));
    paint(&mut e);
    assert_eq!(e.runtime().reading().current_match(), Some(&first));
    assert_eq!(e.runtime().directory().unwrap().query(), "Head");
    assert_eq!(e.runtime().sidebar_selection(), selected);
}

#[test]
fn explicit_sidebar_jump_wins_over_an_unfinished_body_search() {
    let text = format!(
        "intro\n\n# Target\n\n{}\n\n# Final\n\nneedle\n",
        "long body ".repeat(100_000)
    );
    let (mut app, mut e) = setup(&text);
    key(&mut app, &mut e, K::Char('/'));
    type_text(&mut app, &mut e, "needle");
    key(&mut app, &mut e, K::Enter);
    assert!(e.runtime().reading().pending());
    key(&mut app, &mut e, K::Tab);
    key(&mut app, &mut e, K::Home);
    key(&mut app, &mut e, K::Enter);
    paint(&mut e);
    app.frame_committed(&e);
    let target = e
        .document()
        .headings()
        .get(HeadingId(0))
        .unwrap()
        .source
        .start;
    assert_eq!(e.runtime().top_source(), Some(target));
    assert_eq!(e.runtime().reading().query(), "needle");
    assert_eq!(e.runtime().sidebar_selection(), Some(HeadingId(0)));
    for _ in 0..3 {
        paint(&mut e);
        assert_eq!(e.runtime().top_source(), Some(target));
    }
    key(&mut app, &mut e, K::End);
    key(&mut app, &mut e, K::Enter);
    paint(&mut e);
    app.frame_committed(&e);
    assert!(app.history().len() >= 2);
    key(&mut app, &mut e, K::BackTab);
    key(&mut app, &mut e, K::Char('/'));
    key(&mut app, &mut e, K::Char('q'));
    app.handle_key(&mut e, K::Left, M::ALT).unwrap();
    paint(&mut e);
    assert_eq!(e.runtime().top_source(), Some(target));
    assert_eq!(e.runtime().reading().input(), Some("needleq"));
    key(&mut app, &mut e, K::Esc);
    key(&mut app, &mut e, K::Char('n'));
    paint(&mut e);
    assert_eq!(
        e.runtime().reading().current_match().unwrap().source.start,
        e.document().source().rfind("needle").unwrap()
    );
    assert_eq!(e.runtime().sidebar_selection(), Some(HeadingId(1)));
}

#[test]
fn starting_body_edit_cancels_queued_directory_activation_without_history() {
    let mut text = String::from("# Root\n\n");
    for depth in 1..=1000 {
        text.push_str(&format!("{} # H{depth}\n", ">".repeat(depth)));
    }
    let (mut app, mut e) = setup(&text);
    key(&mut app, &mut e, K::Tab);
    key(&mut app, &mut e, K::End);
    key(&mut app, &mut e, K::Enter);
    assert!(e.runtime().has_pending_jump());
    key(&mut app, &mut e, K::BackTab);
    key(&mut app, &mut e, K::Char('/'));
    key(&mut app, &mut e, K::Char('q'));
    paint(&mut e);
    app.frame_committed(&e);
    assert_eq!(e.runtime().top_source(), Some(0));
    assert_eq!(e.runtime().reading().input(), Some("q"));
    assert!(app.history().is_empty());
}
