use crossterm::event::{KeyCode, KeyModifiers};
use mdtui_tui::{
    app::{AppState, KeyAction},
    cli::LinkMode,
    external::{ExternalOpenOutcome, ExternalOpenStatus, ExternalOpener, ExternalTarget},
    history::SurfacePosition,
    render::frame::{FrameGeometry, FrameRuntime},
    stream::StreamEngine,
};
use mdtui3::{Budget, WorkLimit};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

struct Opener(Arc<Mutex<Vec<String>>>);
impl ExternalOpener for Opener {
    fn open(&mut self, target: &ExternalTarget) -> ExternalOpenOutcome {
        self.0.lock().unwrap().push(target.as_str().to_owned());
        ExternalOpenOutcome::new(ExternalOpenStatus::Opened)
    }
}
fn settle(e: &mut StreamEngine, a: &mut AppState) {
    let mut wire = vec![];
    for _ in 0..10000 {
        if e.calibration_pending() {
            e.advance_calibration(&mut Budget::new(WorkLimit::unlimited()))
                .unwrap();
        }
        if e.due(Duration::ZERO) {
            if let Some(commit) = e
                .render_due(
                    &mut wire,
                    &mut Budget::new(WorkLimit::unlimited()),
                    Duration::ZERO,
                )
                .unwrap()
                && commit.committed
            {
                a.frame_committed(e);
            }
        } else {
            return;
        }
    }
    panic!("navigation did not settle");
}
fn started(source: &str, mode: LinkMode) -> (StreamEngine, AppState, Arc<Mutex<Vec<String>>>) {
    let r = FrameRuntime::new(
        FrameGeometry {
            rows: 8,
            cols: 40,
            content_width: 40,
        },
        mode,
    )
    .unwrap();
    let calls = Arc::new(Mutex::new(vec![]));
    let mut e = StreamEngine::with_external_opener(r, Box::new(Opener(calls.clone())));
    let mut a = AppState::default();
    e.ingest_data(source.as_bytes(), Duration::ZERO).unwrap();
    settle(&mut e, &mut a);
    e.finish_input(Duration::ZERO).unwrap();
    settle(&mut e, &mut a);
    (e, a, calls)
}
fn key(e: &mut StreamEngine, a: &mut AppState, code: KeyCode) {
    a.handle_key(e, code, KeyModifiers::NONE).unwrap();
    settle(e, a);
}
fn line(e: &StreamEngine) -> String {
    e.runtime()
        .screen()
        .front_row(0)
        .unwrap()
        .iter()
        .map(|c| c.text().collect_text())
        .collect::<String>()
        .trim_end()
        .to_owned()
}

#[test]
fn host_and_body_navigation_history_commit_only_after_actual_wire_acknowledgement() {
    let (mut e, mut a, _) = started(
        "See [^a].\n\n[^a]: A [^b]\n[^b]: B [^a]\n",
        LinkMode::NoOsc8,
    );
    assert!(a.history().is_empty());
    key(&mut e, &mut a, KeyCode::Tab);
    a.handle_key(&mut e, KeyCode::Enter, KeyModifiers::NONE)
        .unwrap();
    let mut wire = vec![];
    assert!(
        e.render_due(
            &mut wire,
            &mut Budget::new(WorkLimit::default()),
            Duration::ZERO
        )
        .unwrap()
        .is_none()
    );
    assert!(a.history().is_empty());
    settle(&mut e, &mut a);
    assert_eq!(line(&e), "A ²");
    assert_eq!(a.history().len(), 2);
    assert!(matches!(
        a.history().current_surface(),
        Some(SurfacePosition::Body { .. })
    ));
    key(&mut e, &mut a, KeyCode::Tab);
    key(&mut e, &mut a, KeyCode::Enter);
    assert_eq!(line(&e), "B ¹");
    assert_eq!(a.history().len(), 3);
    a.handle_action(&mut e, KeyAction::HistoryBack).unwrap();
    assert_eq!(a.history().len(), 3);
    settle(&mut e, &mut a);
    assert_eq!(line(&e), "A ²");
    a.handle_action(&mut e, KeyAction::HistoryBack).unwrap();
    settle(&mut e, &mut a);
    assert_eq!(line(&e), "See ¹.");
    assert!(matches!(
        a.history().current_surface(),
        Some(SurfacePosition::Root { .. })
    ));
    a.handle_action(&mut e, KeyAction::HistoryForward).unwrap();
    settle(&mut e, &mut a);
    assert_eq!(line(&e), "A ²");
    assert_eq!(a.history().len(), 3);
}

#[test]
fn body_external_callback_uses_decoded_host_target_and_default_mode_does_not_open() {
    for mode in [LinkMode::NoOsc8, LinkMode::OpenExternal] {
        let (mut e, mut a, calls) = started(
            "See [^n].\n\n[^n]: [L][r]\n# outside\n\n[r]: https://example.test/a&amp;b \"Host title\"\n",
            mode,
        );
        key(&mut e, &mut a, KeyCode::Tab);
        key(&mut e, &mut a, KeyCode::Enter);
        assert_eq!(line(&e), "L");
        key(&mut e, &mut a, KeyCode::Tab);
        let history = a.history().clone();
        key(&mut e, &mut a, KeyCode::Enter);
        assert_eq!(
            *calls.lock().unwrap(),
            if mode == LinkMode::OpenExternal {
                vec!["https://example.test/a&b".to_owned()]
            } else {
                vec![]
            }
        );
        assert_eq!(a.history(), &history);
    }
}

#[test]
fn note_fragment_targets_its_own_heading_and_directory_returns_to_root_surface() {
    let (mut e, mut a, _) = started(
        "See [^n].\n\n# root\n\n[^n]: [go](#inside)\n\n    # Inside\n\n    note text\n",
        LinkMode::NoOsc8,
    );
    key(&mut e, &mut a, KeyCode::Tab);
    key(&mut e, &mut a, KeyCode::Enter);
    assert_eq!(line(&e), "go");
    key(&mut e, &mut a, KeyCode::Tab);
    key(&mut e, &mut a, KeyCode::Enter);
    assert_eq!(line(&e), "Inside");
    assert!(e.runtime().footnote_surface().is_some());
    key(&mut e, &mut a, KeyCode::Char('s'));
    assert_eq!(a.focus(), mdtui_tui::app::FocusTarget::Sidebar);
    key(&mut e, &mut a, KeyCode::Enter);
    assert!(e.runtime().footnote_surface().is_none());
    assert!(matches!(
        a.history().current_surface(),
        Some(SurfacePosition::Root { .. })
    ));
}

#[test]
fn no_op_self_reference_and_cancelled_pending_move_preserve_history() {
    let (mut e, mut a, _) = started("See [^a].\n\n[^a]: A [^a]\n", LinkMode::NoOsc8);
    key(&mut e, &mut a, KeyCode::Tab);
    key(&mut e, &mut a, KeyCode::Enter);
    key(&mut e, &mut a, KeyCode::Tab);
    let history = a.history().clone();
    key(&mut e, &mut a, KeyCode::Enter);
    assert_eq!(a.history(), &history);
    key(&mut e, &mut a, KeyCode::Char('b'));
    assert_eq!(line(&e), "See ¹.");
    let history = a.history().clone();
    key(&mut e, &mut a, KeyCode::Tab);
    a.handle_key(&mut e, KeyCode::Enter, KeyModifiers::NONE)
        .unwrap();
    a.handle_key(&mut e, KeyCode::Esc, KeyModifiers::NONE)
        .unwrap();
    settle(&mut e, &mut a);
    assert_eq!(a.history(), &history);
    assert_eq!(line(&e), "See ¹.");
}

#[test]
fn host_finish_and_late_note_definition_keep_the_active_surface_and_local_missing_state() {
    let r = FrameRuntime::new(
        FrameGeometry {
            rows: 8,
            cols: 40,
            content_width: 40,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    let mut e = StreamEngine::new(r);
    let mut a = AppState::default();
    e.ingest_data(b"See [^a].\n\n[^a]: A [^b]\n# root\n", Duration::ZERO)
        .unwrap();
    settle(&mut e, &mut a);
    key(&mut e, &mut a, KeyCode::Tab);
    key(&mut e, &mut a, KeyCode::Enter);
    assert_eq!(line(&e), "A ²");
    let surface = e.runtime().footnote_surface();
    key(&mut e, &mut a, KeyCode::Tab);
    let status = e
        .runtime()
        .screen()
        .front_row(7)
        .unwrap()
        .iter()
        .map(|c| c.text().collect_text())
        .collect::<String>();
    assert!(status.contains("Unresolved"), "{status}");
    let work = e.document().footnote_body_work();
    e.ingest_data(b"\n[^b]: B\n# end\n", Duration::from_millis(100))
        .unwrap();
    e.request_render();
    settle(&mut e, &mut a);
    assert_eq!(e.runtime().footnote_surface(), surface);
    assert_eq!(line(&e), "A ²");
    let status = e
        .runtime()
        .screen()
        .front_row(7)
        .unwrap()
        .iter()
        .map(|c| c.text().collect_text())
        .collect::<String>();
    assert!(status.contains("Defined"), "{status}");
    assert!(e.document().footnote_body_work().source_bytes > work.source_bytes);
    e.finish_input(Duration::ZERO).unwrap();
    settle(&mut e, &mut a);
    assert_eq!(e.runtime().footnote_surface(), surface);
    assert_eq!(line(&e), "A ²");
    key(&mut e, &mut a, KeyCode::Enter);
    assert_eq!(line(&e), "B");
}

#[test]
fn a_body_link_is_one_focus_object_and_click_dispatches_the_qualified_target() {
    let (mut e, mut a, calls) = started(
        "See [^n].\n\n[^n]: [longlabel](https://one.test/) [second](https://two.test/)\n",
        LinkMode::OpenExternal,
    );
    a.click_link(&mut e, 0, 4).unwrap();
    settle(&mut e, &mut a);
    assert_eq!(line(&e), "longlabel second");
    key(&mut e, &mut a, KeyCode::Tab);
    key(&mut e, &mut a, KeyCode::Tab);
    key(&mut e, &mut a, KeyCode::Enter);
    assert_eq!(*calls.lock().unwrap(), vec!["https://two.test/".to_owned()]);
    a.click_link(&mut e, 0, 3).unwrap();
    settle(&mut e, &mut a);
    assert_eq!(
        *calls.lock().unwrap(),
        vec![
            "https://two.test/".to_owned(),
            "https://one.test/".to_owned()
        ]
    );
}
