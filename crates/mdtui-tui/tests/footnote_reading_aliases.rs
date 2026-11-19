use crossterm::event::{KeyCode, KeyModifiers};
use mdtui_tui::{
    app::AppState,
    cli::LinkMode,
    render::frame::{FrameGeometry, FrameRuntime},
    stream::StreamEngine,
};
use mdtui3::{Budget, Span, WorkLimit};
fn settle(e: &mut StreamEngine) {
    for _ in 0..200_000 {
        let mut b = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 6,
            tiles: 1,
            recovery: 1,
            dependencies: 1,
        });
        let frame = e.runtime_mut().frame_to(&mut vec![], &mut b).unwrap();
        assert!(b.used().queries <= 1 && b.used().objects <= 1 && b.used().bytes <= 6);
        if frame.committed && !e.runtime().reading().pending() && !e.runtime().has_pending_jump() {
            return;
        }
    }
    panic!("note reading frame did not converge");
}
fn start(source: &str) -> (StreamEngine, AppState) {
    let r = FrameRuntime::from_finished_source(
        source,
        FrameGeometry {
            rows: 8,
            cols: 60,
            content_width: 60,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    let mut e = StreamEngine::new(r);
    for _ in 0..10000 {
        if e.runtime_mut()
            .frame_to(&mut vec![], &mut Budget::new(WorkLimit::unlimited()))
            .unwrap()
            .committed
        {
            break;
        }
    }
    (e, AppState::default())
}
fn key(e: &mut StreamEngine, a: &mut AppState, k: KeyCode) {
    a.handle_key(e, k, KeyModifiers::NONE).unwrap();
    settle(e);
}
fn query(e: &mut StreamEngine, a: &mut AppState, q: &str) {
    let old = e.runtime().reading().query().chars().count();
    a.handle_key(e, KeyCode::Char('/'), KeyModifiers::NONE)
        .unwrap();
    for _ in 0..old {
        a.handle_key(e, KeyCode::Backspace, KeyModifiers::NONE)
            .unwrap();
    }
    for c in q.chars() {
        a.handle_key(e, KeyCode::Char(c), KeyModifiers::NONE)
            .unwrap();
    }
    a.handle_key(e, KeyCode::Enter, KeyModifiers::NONE).unwrap();
    settle(e);
}
fn hit(e: &StreamEngine) -> Span {
    e.runtime()
        .reading()
        .current_match()
        .unwrap_or_else(|| panic!("missing hit: {:?}", e.runtime().reading().status()))
        .source
}
#[test]
fn unrelated_active_reference_does_not_force_a_withdrawn_table_marker_through_shape() {
    let source = format!(
        "$prior\nend$ [^n] {}|b\n-|-\nx|y\n\n[^n]: note\n\nelsewhere[^real]\n\n[^real]: R\n",
        "z".repeat(1100)
    );
    let (mut e, mut a) = start(&source);
    query(&mut e, &mut a, "[^n]");
    assert_eq!(hit(&e), Span::new(12, 16));
    assert!(
        !e.runtime()
            .reading()
            .status()
            .unwrap()
            .contains("match not drawn")
    );
}
#[test]
fn distinct_label_aliases_keep_n_and_n_backwards_order_before_visible_text() {
    let (mut e, mut a) = start("pre[^aaaa] xx a\n\n[^aaaa]: note\n");
    query(&mut e, &mut a, "a");
    assert_eq!(hit(&e), Span::new(5, 6));
    for start in [6, 7, 8, 14] {
        key(&mut e, &mut a, KeyCode::Char('n'));
        assert_eq!(hit(&e), Span::new(start, start + 1));
    }
    key(&mut e, &mut a, KeyCode::Char('N'));
    assert_eq!(hit(&e), Span::new(8, 9));
    key(&mut e, &mut a, KeyCode::Char('N'));
    assert_eq!(hit(&e), Span::new(7, 8));
    query(&mut e, &mut a, "¹");
    assert_eq!(hit(&e), Span::new(3, 10));
    assert!(
        !e.runtime()
            .reading()
            .status()
            .unwrap()
            .contains("match not drawn")
    );
}
#[test]
fn visible_then_alias_then_visible_matches_keep_source_order() {
    let (mut e, mut a) = start("a pre[^a] a\n\n[^a]: note\n");
    query(&mut e, &mut a, "a");
    assert_eq!(hit(&e), Span::new(0, 1));
    for start in [7, 10] {
        key(&mut e, &mut a, KeyCode::Char('n'));
        assert_eq!(hit(&e), Span::new(start, start + 1));
    }
    key(&mut e, &mut a, KeyCode::Char('N'));
    assert_eq!(hit(&e), Span::new(7, 8));
}
#[test]
fn numbered_root_reference_remains_an_actual_focus_object_with_mathematical_notes() {
    let (mut e, _) = start("See [^a].\n\n[^a]: A $\\frac{1}{2}$ [^b]\n[^b]: B $x^2$ [^a]\n");
    assert_eq!(e.screen().front_cell(0, 4).unwrap().text(), "¹");
    assert_eq!(
        e.screen().front_cell(0, 4).unwrap().source(),
        Some(Span::new(4, 8))
    );
    assert!(e.runtime_mut().enter_object_focus(true));
}
#[test]
fn an_ascii_note_atom_crossing_source_tiles_keeps_one_height_owner() {
    let label = "n".repeat(700);
    let source = format!("a[^{label}]b\n\n[^{label}]: body\n");
    let (e, _) = start(&source);
    assert_eq!(e.screen().front_cell(0, 0).unwrap().text(), "a");
    assert_eq!(e.screen().front_cell(0, 1).unwrap().text(), "¹");
    assert_eq!(
        e.screen().front_cell(0, 1).unwrap().source(),
        Some(Span::new(1, 704))
    );
    assert_eq!(e.screen().front_cell(0, 2).unwrap().text(), "b");
    assert_eq!(
        e.runtime().viewport().heights().height(mdtui3::TileId(1)),
        Some(0)
    );
}
#[test]
fn a_number_followed_by_combining_marks_uses_the_complete_shared_cluster() {
    let marks = "\u{301}".repeat(8);
    let source = format!("a[^n]{marks}\n\n[^n]: body\n");
    let (e, _) = start(&source);
    assert_eq!(
        e.screen().front_cell(0, 1).unwrap().text().collect_text(),
        format!("¹{marks}")
    );
    assert_eq!(
        e.screen().front_cell(0, 1).unwrap().source(),
        Some(Span::new(1, 21))
    );
}
