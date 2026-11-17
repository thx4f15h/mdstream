mod support;

use std::time::Duration;

use crossterm::event::{KeyCode, KeyModifiers};
use mdtui_tui::app::{AppState, FocusTarget};
use mdtui_tui::cli::LinkMode;
use mdtui_tui::fragment::{AnchorBuildStatus, FragmentActivationStatus};
use mdtui_tui::render::frame::{
    BEGIN_SYNCHRONIZED_UPDATE, END_SYNCHRONIZED_UPDATE, FrameGeometry, FrameRuntime,
};
use mdtui_tui::stream::{IngestResult, StreamEngine};
use mdtui3::{Budget, LinkId, WorkLimit};
use support::terminal::{FrameOutcome, Vt100Writer, assert_terminal_matches_screen};

const WIDTH: u16 = 80;
const HEIGHT: u16 = 24;

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
        Ok(Some(commit)) if commit.committed => {
            writer.finish_frame(FrameOutcome::Committed);
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

fn first_link(engine: &StreamEngine) -> LinkId {
    engine
        .link_table()
        .expect("link table")
        .first()
        .expect("visible link")
}

fn focus_link(app: &mut AppState, engine: &mut StreamEngine, writer: &mut Vt100Writer) {
    let entered = app
        .handle_key(engine, KeyCode::Tab, KeyModifiers::NONE)
        .expect("enter link focus");
    assert!(entered.redraw);
    assert_eq!(app.focus(), FocusTarget::Link);
    settle(engine, writer, Duration::ZERO);
}

fn activate(
    app: &mut AppState,
    engine: &mut StreamEngine,
) -> mdtui_tui::fragment::FragmentActivation {
    app.handle_key(engine, KeyCode::Enter, KeyModifiers::NONE)
        .expect("activate")
        .fragment_activation
        .expect("fragment activation status")
}

fn assert_incremental_frame(bytes: &[u8]) {
    assert!(!bytes.windows(4).any(|window| window == b"\x1b[2J"));
    assert!(!bytes.windows(3).any(|window| window == b"\x1b[H"));
    assert!(!bytes.windows(3).any(|window| window == b"\x1b[K"));
    assert_eq!(
        bytes
            .windows(BEGIN_SYNCHRONIZED_UPDATE.len())
            .filter(|window| *window == BEGIN_SYNCHRONIZED_UPDATE)
            .count(),
        1
    );
    assert_eq!(
        bytes
            .windows(END_SYNCHRONIZED_UPDATE.len())
            .filter(|window| *window == END_SYNCHRONIZED_UPDATE)
            .count(),
        1
    );
}

#[test]
fn streaming_fragment_is_pending_then_eof_index_enables_the_jump() {
    let source = "[jump](#target)\n\n# Target\n\ntarget body\n";
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    let mut engine = started_engine(source, &mut writer);
    let mut app = AppState::default();
    focus_link(&mut app, &mut engine, &mut writer);
    let link = first_link(&engine);
    assert!(!engine.document().is_finished());
    build_anchors(&mut engine);
    assert!(engine.anchor_index_complete());
    let activation = activate(&mut app, &mut engine);
    assert_eq!(activation.status, FragmentActivationStatus::Moved);
    assert!(activation.target.unwrap().provisional);
    let target = engine.document().headings().entries()[0].source.start;
    assert_eq!(engine.runtime().top_source(), Some(target));
    assert_eq!(app.history().len(), 1);
    assert_eq!(app.history().last_source(), Some(target));
    settle(&mut engine, &mut writer, Duration::ZERO);
    app.frame_committed(&engine);
    assert_eq!(app.history().len(), 1);
    assert!(engine.finish_input(Duration::ZERO).unwrap());
    settle(&mut engine, &mut writer, Duration::ZERO);
    build_anchors(&mut engine);
    assert!(engine.anchor_index_complete());
    assert_eq!(engine.runtime().top_source(), Some(target));
    assert_eq!(app.history().len(), 1);
    assert_terminal_matches_screen(writer.terminal(), engine.runtime().screen());
    let _ = link;
}

#[test]
fn anchor_index_is_incremental_and_pending_activation_does_not_restart_it() {
    let mut source = String::from("[jump](#target)\n\n# Target\n\n");
    for index in 0..80 {
        source.push_str(&format!("# Heading {index:02}\n\n"));
    }
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    let mut engine = finished_engine(&source, &mut writer);
    let mut app = AppState::default();
    focus_link(&mut app, &mut engine, &mut writer);
    let link = first_link(&engine);
    let top = engine.runtime().top_source();
    let tiny = WorkLimit {
        queries: 1,
        tiles: 1,
        bytes: 8,
        objects: 1,
        recovery: 1,
        ..WorkLimit::default()
    };

    let mut saw_budget = false;
    let mut partial = 0;
    for _ in 0..200_000 {
        let mut budget = Budget::new(tiny);
        match engine
            .advance_anchor_index(&mut budget)
            .expect("anchor step")
        {
            AnchorBuildStatus::BudgetExhausted => saw_budget = true,
            AnchorBuildStatus::Complete => break,
            AnchorBuildStatus::PendingDocument => panic!("finished document reported pending"),
        }
        partial = engine.anchor_indexed_headings().unwrap_or(0);
        if partial > 0 {
            break;
        }
    }
    assert!(saw_budget);
    assert!(partial > 0);
    assert!(!engine.anchor_index_complete());

    // Activating now pays resumable guard validation before checking the
    // partial anchor index. Keep the same selection task across micro frames.
    let mut pending = None;
    for _ in 0..20_000 {
        let mut budget = Budget::new(tiny);
        let activation = engine
            .activate_focused_link_with_budget(&mut budget)
            .expect("pending activation");
        assert!(budget.used().queries <= tiny.queries);
        assert!(budget.used().objects <= tiny.objects);
        if activation.status != FragmentActivationStatus::BudgetExhausted {
            pending = Some(activation);
            break;
        }
    }
    let pending = pending.expect("guard validation progresses");
    assert_eq!(pending.status, FragmentActivationStatus::PendingIndex);
    assert_eq!(engine.notice(), Some("正在建立锚点索引"));
    assert_eq!(engine.runtime().top_source(), top);
    assert!(app.history().is_empty());
    assert_eq!(engine.link_focus(), Some(link));
    // The accepted maintenance path now continues the same request in the
    // render poll. It may finish the index and jump before another Enter.
    let _ = app
        .handle_key(&mut engine, KeyCode::Enter, KeyModifiers::NONE)
        .unwrap();
    settle(&mut engine, &mut writer, Duration::ZERO);
    app.frame_committed(&engine);
    build_anchors(&mut engine);
    assert!(engine.anchor_index_complete());
    let total_headings = engine.document().headings().entries().len();
    assert_eq!(engine.anchor_indexed_headings(), Some(total_headings));
    let target = engine.document().headings().entries()[0].source.start;
    assert_eq!(engine.runtime().top_source(), Some(target));
    assert_eq!(app.history().len(), 1);
    assert_eq!(app.history().last_source(), Some(target));
    app.frame_committed(&engine);
    assert_eq!(app.history().len(), 1);
    assert!(!engine.finish_input(Duration::ZERO).unwrap());
}

#[test]
fn decoded_fragment_forms_match_core_slugs_percent_utf8_and_duplicate_names() {
    let cases = [
        ("[go](#a\\_b)\n\n# a_b\n\nbody\n", Some(0)),
        ("[go](#&#x61;)\n\n# a\n\nbody\n", Some(0)),
        ("[go](#%E4%B8%AD)\n\n# 中\n\nbody\n", Some(0)),
        ("[go](#foo-1)\n\n# Foo\n\n# Foo\n\nbody\n", Some(1)),
        ("[go](#)\n\nbody\n", None),
    ];
    for (source, heading_index) in cases {
        let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
        let mut engine = finished_engine(source, &mut writer);
        build_anchors(&mut engine);
        let mut app = AppState::default();
        focus_link(&mut app, &mut engine, &mut writer);
        let activation = activate(&mut app, &mut engine);
        assert_eq!(
            activation.status,
            FragmentActivationStatus::Moved,
            "{source:?}"
        );
        let expected_target = if let Some(index) = heading_index {
            engine.document().headings().entries()[index].source.start
        } else {
            0
        };
        let expected_top = if heading_index.is_some() {
            expected_target
        } else {
            engine.document().next_visible(0).unwrap_or(0)
        };
        assert_eq!(
            activation.target.expect("anchor target").source,
            expected_target,
            "{source:?}"
        );
        assert_eq!(
            engine.runtime().top_source(),
            Some(expected_top),
            "{source:?}"
        );
        assert_eq!(app.history().len(), 1, "{source:?}");
        assert_eq!(
            app.history().last_source(),
            Some(expected_target),
            "{source:?}"
        );
    }
}

#[test]
fn successful_fragment_updates_sidebar_current_and_only_success_writes_history() {
    let mut source = String::from("# Start\n\n[go](#target)\n\n");
    for index in 0..50 {
        source.push_str(&format!("paragraph {index:02}\n\n"));
    }
    source.push_str("# Target\n\ntarget body\n");
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    let mut engine = finished_engine(&source, &mut writer);
    build_anchors(&mut engine);
    let mut app = AppState::default();
    app.handle_key(&mut engine, KeyCode::Char('s'), KeyModifiers::NONE)
        .expect("open sidebar");
    settle(&mut engine, &mut writer, Duration::ZERO);
    app.handle_key(&mut engine, KeyCode::Tab, KeyModifiers::NONE)
        .expect("body to sidebar");
    app.handle_key(&mut engine, KeyCode::Tab, KeyModifiers::NONE)
        .expect("sidebar to link");
    assert_eq!(app.focus(), FocusTarget::Link);
    settle(&mut engine, &mut writer, Duration::ZERO);
    let top = engine.runtime().top_source();
    let target = engine.document().headings().entries()[1];

    let activation = activate(&mut app, &mut engine);
    assert_eq!(activation.status, FragmentActivationStatus::Moved);
    assert!(activation.moved);
    assert_ne!(engine.runtime().top_source(), top);
    assert_eq!(engine.runtime().top_source(), Some(target.source.start));
    assert_eq!(app.history().len(), 1);
    assert_eq!(app.history().last_source(), Some(target.source.start));
    settle(&mut engine, &mut writer, Duration::ZERO);
    assert_eq!(
        engine.runtime().sidebar_current_heading(),
        Some(mdtui3::HeadingId(1))
    );
    assert_terminal_matches_screen(writer.terminal(), engine.runtime().screen());

    let frame = writer.frames().len() - 1;
    let bytes: Vec<_> = writer
        .frame_writes(frame)
        .iter()
        .flat_map(|write| write.bytes.iter().copied())
        .collect();
    assert_incremental_frame(&bytes);
}

#[test]
fn not_found_invalid_and_tiny_budget_leave_position_and_history_unchanged() {
    for source in [
        "[go](#A)\n\n# a\n\nbody\n",
        "[go](#%zz)\n\n# a\n\nbody\n",
        "[go]()\n\n# a\n\nbody\n",
    ] {
        let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
        let mut engine = finished_engine(source, &mut writer);
        build_anchors(&mut engine);
        let mut app = AppState::default();
        focus_link(&mut app, &mut engine, &mut writer);
        let top = engine.runtime().top_source();
        let activation = activate(&mut app, &mut engine);
        assert!(matches!(
            activation.status,
            FragmentActivationStatus::NotFound | FragmentActivationStatus::Invalid
        ));
        assert!(!activation.moved);
        assert_eq!(engine.notice(), Some(activation.status.message()));
        settle(&mut engine, &mut writer, Duration::ZERO);
        assert!(
            writer
                .terminal()
                .text()
                .contains(activation.status.message())
        );
        assert_eq!(engine.runtime().top_source(), top);
        assert!(app.history().is_empty());
    }

    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    let mut engine = finished_engine("[go](#target)\n\n# Target\n", &mut writer);
    build_anchors(&mut engine);
    let mut app = AppState::default();
    focus_link(&mut app, &mut engine, &mut writer);
    let top = engine.runtime().top_source();
    let mut tiny = Budget::new(WorkLimit {
        queries: 1,
        tiles: 1,
        bytes: 1,
        objects: 1,
        recovery: 1,
        ..WorkLimit::default()
    });
    let activation = engine
        .activate_focused_link_with_budget(&mut tiny)
        .expect("constrained activation");
    assert_eq!(activation.status, FragmentActivationStatus::BudgetExhausted);
    assert!(!activation.moved);
    assert_eq!(engine.notice(), Some(activation.status.message()));
    assert_eq!(engine.runtime().top_source(), top);
    assert!(app.history().is_empty());
    // A later render frame is now authorized to finish this same request.
    settle(&mut engine, &mut writer, Duration::ZERO);
    assert_eq!(
        engine.runtime().top_source(),
        Some(engine.document().headings().entries()[0].source.start)
    );
    assert_terminal_matches_screen(writer.terminal(), engine.runtime().screen());
}

#[test]
fn unfinished_heading_source_jump_does_not_need_an_anchor_index() {
    let source = "# First\n\nfirst body\n\n# Second\n\nsecond body\n";
    let mut document = mdtui3::Utf8Document::new();
    document.append(source.as_bytes()).unwrap();
    let mut runtime =
        FrameRuntime::from_utf8_document(document, geometry(HEIGHT, WIDTH), LinkMode::Default)
            .unwrap();
    assert!(!runtime.document().is_finished());
    assert!(!runtime.anchor_index_started());
    assert!(runtime.set_sidebar_visible(true).unwrap().visible);
    let mut output = Vec::new();
    runtime.frame_to(&mut output, &mut unlimited()).unwrap();
    assert!(runtime.sidebar_selection_end());
    let target = runtime.document().headings().entries()[1].source.start;
    assert!(
        runtime
            .activate_sidebar_selection(&mut unlimited())
            .expect("source jump")
    );
    assert_eq!(runtime.top_source(), Some(target));
    assert!(!runtime.anchor_index_started());
    assert!(!runtime.document().is_finished());
}

#[test]
fn missing_pre_eof_request_resumes_on_input_without_another_enter_and_commits_once() {
    let source = format!("[jump](#later)\n\n# First\n\n{}", "old body\n\n".repeat(40));
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    let mut engine = started_engine(&source, &mut writer);
    let mut app = AppState::default();
    focus_link(&mut app, &mut engine, &mut writer);
    build_anchors(&mut engine);
    let top = engine.runtime().top_source();
    let pending = activate(&mut app, &mut engine);
    assert_eq!(pending.status, FragmentActivationStatus::PendingDocument);
    assert_eq!(engine.runtime().top_source(), top);
    assert!(app.history().is_empty());
    settle(&mut engine, &mut writer, Duration::ZERO);
    app.frame_committed(&engine);
    assert!(app.history().is_empty());
    assert!(writer.terminal().text().contains("等待标题出现"));
    assert!(
        engine
            .runtime()
            .screen()
            .front_cell(HEIGHT - 1, 0)
            .unwrap()
            .style()
            .contains(mdtui_tui::screen::StyleRole::META)
    );
    engine
        .ingest_data(b"\n# Later\n\nLATE_TARGET\n", Duration::from_secs(1))
        .unwrap();
    assert!(!engine.document().is_finished());
    for i in 0..100 {
        let now = Duration::from_secs(2) + Duration::from_millis(i * 40);
        settle(&mut engine, &mut writer, now);
        app.frame_committed(&engine);
        if app.history().len() == 1 {
            break;
        }
    }
    let target = engine.document().headings().entries()[1].source.start;
    assert_eq!(engine.runtime().top_source(), Some(target));
    assert_eq!(app.history().last_source(), Some(target));
    assert!(
        app.last_fragment_activation()
            .unwrap()
            .target
            .unwrap()
            .provisional
    );
    app.frame_committed(&engine);
    assert_eq!(app.history().len(), 1);
    assert!(writer.terminal().text().contains("LATE_TARGET"));
    assert_terminal_matches_screen(writer.terminal(), engine.runtime().screen());
}
#[test]
fn mouse_activation_and_history_keep_source_after_current_slug_owner_changes() {
    let source =
        "[start](#foo)\n\n# [foo][r]\n\nliteral\n\n# foo\n\n[to bar](#bar)\n\n# bar\n\nlast\n";
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    let mut engine = started_engine(source, &mut writer);
    let mut app = AppState::default();
    let h1 = engine.document().headings().entries()[1].source.start;
    let h2 = engine.document().headings().entries()[2].source.start;
    // Resolve an actual published front cell through the application's mouse
    // route. This uses the same guard/decoder/current request as Enter.
    let outcome = app.click_link(&mut engine, 0, 0).unwrap();
    assert!(outcome.fragment_activation.unwrap().status.is_success());
    assert_eq!(engine.runtime().top_source(), Some(h1));
    settle(&mut engine, &mut writer, Duration::ZERO);
    app.frame_committed(&engine);
    app.handle_key(&mut engine, KeyCode::Esc, KeyModifiers::NONE)
        .unwrap();
    focus_link(&mut app, &mut engine, &mut writer);
    assert_eq!(
        activate(&mut app, &mut engine).status,
        FragmentActivationStatus::Moved
    );
    settle(&mut engine, &mut writer, Duration::ZERO);
    app.frame_committed(&engine);
    assert_eq!(engine.runtime().top_source(), Some(h2));
    assert_eq!(app.history().len(), 2);
    engine
        .ingest_data(b"\n[r]: /winning\n", Duration::from_secs(1))
        .unwrap();
    build_anchors(&mut engine);
    app.handle_key(&mut engine, KeyCode::Left, KeyModifiers::ALT)
        .unwrap();
    for i in 0..10 {
        settle(
            &mut engine,
            &mut writer,
            Duration::from_secs(2) + Duration::from_millis(i * 40),
        );
        app.frame_committed(&engine);
    }
    assert_eq!(engine.runtime().top_source(), Some(h1));
    assert_eq!(app.history().last_source(), Some(h1));
    app.handle_key(&mut engine, KeyCode::Right, KeyModifiers::ALT)
        .unwrap();
    settle(&mut engine, &mut writer, Duration::from_secs(3));
    app.frame_committed(&engine);
    assert_eq!(engine.runtime().top_source(), Some(h2));
    assert_eq!(app.history().len(), 2);
}

#[test]
fn late_binding_revalidates_owner_during_source_refinement_and_resize_keeps_index() {
    let source = format!(
        "[start](#foo)\n\n# [foo][r]\n\nliteral\n\n{}# foo\n\nOLD_OWNER\n",
        "body\n\n".repeat(60)
    );
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    let mut engine = started_engine(&source, &mut writer);
    build_anchors(&mut engine);
    let mut app = AppState::default();
    focus_link(&mut app, &mut engine, &mut writer);
    let top = engine.runtime().top_source();
    let micro = WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 6,
        tiles: 1,
        recovery: 1,
        dependencies: 1,
    };
    for _ in 0..10000 {
        let result = engine
            .activate_focused_link_with_budget(&mut Budget::new(micro))
            .unwrap();
        assert!(!result.status.is_success());
        if engine.runtime().fragment_refinement_pending() {
            break;
        }
    }
    assert!(engine.runtime().fragment_refinement_pending());
    assert_eq!(engine.runtime().top_source(), top);
    let before = engine.runtime().slug_session_stats();
    engine
        .ingest_data(b"\n[r]: /winner\n", Duration::from_secs(1))
        .unwrap();
    let target = engine.document().headings().entries()[0].source.start;
    let result = engine
        .activate_focused_link_with_budget(&mut unlimited())
        .unwrap();
    assert!(result.status.is_success());
    assert_eq!(result.target.unwrap().source, target);
    assert_eq!(engine.runtime().top_source(), Some(target));
    app.frame_committed(&engine);
    assert_eq!(app.history().last_source(), Some(target));
    assert_eq!(app.history().len(), 1);
    let after = engine.runtime().slug_session_stats();
    assert_eq!(
        after.work.source_bytes - before.work.source_bytes,
        b"\n[r]: /winner\n".len() as u64
    );
    app.resize(&mut engine, 30, 100).unwrap();
    assert!(engine.anchor_index_complete());
    assert_eq!(engine.runtime().slug_session_stats().work, after.work);
    app.frame_committed(&engine);
    assert_eq!(app.history().len(), 1);
}

#[test]
fn late_bound_link_only_fragment_heading_remains_visible_after_home_jump_and_resize() {
    let source = format!(
        "[jump](#foo)\n\n# [foo][r]\n\nFIRST TARGET\n\n{}# foo\n\nSECOND TARGET\n",
        "filler paragraph\n\n".repeat(80)
    );
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    let mut engine = started_engine(&source, &mut writer);
    let mut app = AppState::default();
    focus_link(&mut app, &mut engine, &mut writer);
    build_anchors(&mut engine);
    assert!(activate(&mut app, &mut engine).status.is_success());
    settle(&mut engine, &mut writer, Duration::ZERO);
    app.frame_committed(&engine);
    assert!(writer.terminal().text().contains("SECOND TARGET"));
    engine
        .ingest_data(b"\n[r]: /yes\n", Duration::from_secs(1))
        .unwrap();
    app.handle_key(&mut engine, KeyCode::Home, KeyModifiers::NONE)
        .unwrap();
    settle(&mut engine, &mut writer, Duration::from_secs(2));
    assert!(
        writer
            .terminal()
            .text()
            .lines()
            .any(|line| line.trim() == "foo"),
        "{}",
        writer.terminal().text()
    );
    app.handle_key(&mut engine, KeyCode::Esc, KeyModifiers::NONE)
        .unwrap();
    focus_link(&mut app, &mut engine, &mut writer);
    build_anchors(&mut engine);
    let result = activate(&mut app, &mut engine);
    assert_eq!(result.target.unwrap().heading, Some(mdtui3::HeadingId(0)));
    settle(&mut engine, &mut writer, Duration::from_secs(3));
    assert!(
        writer
            .terminal()
            .text()
            .lines()
            .any(|line| line.trim() == "foo")
    );
    app.resize(&mut engine, HEIGHT, 100).unwrap();
    let mut wide = Vt100Writer::new(HEIGHT, 100);
    settle(&mut engine, &mut wide, Duration::from_secs(4));
    assert!(
        wide.terminal()
            .text()
            .lines()
            .any(|line| line.trim() == "foo")
    );
    assert_terminal_matches_screen(wide.terminal(), engine.runtime().screen());
}

#[test]
fn shared_current_session_keeps_conditional_recovery_across_frames_and_growth() {
    let source = format!("# [x][r] {}", "**a** ".repeat(512));
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    let mut engine = started_engine(&source, &mut writer);
    build_anchors(&mut engine);
    let before = engine.runtime().slug_session_stats();
    engine.ingest_data(b"z", Duration::from_secs(1)).unwrap();
    let micro = WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 6,
        ..Default::default()
    };
    for _ in 0..10000 {
        if engine
            .advance_anchor_index(&mut Budget::new(micro))
            .unwrap()
            == AnchorBuildStatus::Complete
        {
            break;
        }
    }
    assert!(engine.anchor_index_complete());
    let after = engine.runtime().slug_session_stats();
    assert!(after.work.metadata_steps - before.work.metadata_steps < 64);
    assert!(after.canonical_style_work.events - before.canonical_style_work.events < 16);
}

#[test]
fn late_pre_eof_target_resize_and_history_restore_the_current_source() {
    let source = format!(
        "[jump](#target)\n\n# Early\n\n{}",
        "filler paragraph\n\n".repeat(80)
    );
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    let mut engine = started_engine(&source, &mut writer);
    let mut app = AppState::default();
    focus_link(&mut app, &mut engine, &mut writer);
    build_anchors(&mut engine);
    assert_eq!(
        activate(&mut app, &mut engine).status,
        FragmentActivationStatus::PendingDocument
    );
    engine
        .ingest_data(
            b"\n# [Target](#)\n\nLATE_RESIZE_TARGET\n",
            Duration::from_secs(1),
        )
        .unwrap();
    for i in 0..10 {
        settle(
            &mut engine,
            &mut writer,
            Duration::from_secs(2) + Duration::from_millis(i * 40),
        );
        app.frame_committed(&engine);
    }
    let target = engine.document().headings().entries()[1].source.start;
    assert_eq!(engine.runtime().top_source(), Some(target));
    assert!(writer.terminal().text().contains("LATE_RESIZE_TARGET"));
    let work = engine.runtime().slug_session_stats().work;
    app.resize(&mut engine, HEIGHT, 100).unwrap();
    let mut wide = Vt100Writer::new(HEIGHT, 100);
    for i in 0..10 {
        settle(
            &mut engine,
            &mut wide,
            Duration::from_secs(3) + Duration::from_millis(i * 40),
        );
        app.frame_committed(&engine);
    }
    assert_eq!(engine.runtime().top_source(), Some(target));
    assert!(
        wide.terminal().text().contains("LATE_RESIZE_TARGET"),
        "{}",
        wide.terminal().text()
    );
    assert_eq!(engine.runtime().slug_session_stats().work, work);
    assert_eq!(app.history().len(), 1);
    assert!(!engine.document().is_finished());
    let home = app
        .click_link(&mut engine, 0, 0)
        .unwrap()
        .fragment_activation
        .unwrap();
    assert!(home.status.is_success());
    assert_eq!(home.target.unwrap().source, 0);
    // Raw fragment target stays zero; the exact displayed line starts at the
    // first visible link-label byte, after its opening bracket.
    let start = engine.document().link_info(LinkId(0)).unwrap().label.start;
    settle(&mut engine, &mut wide, Duration::from_secs(4));
    app.frame_committed(&engine);
    assert_eq!(app.history().len(), 2);
    assert_eq!(engine.runtime().top_source(), Some(start));
    app.handle_key(&mut engine, KeyCode::Left, KeyModifiers::ALT)
        .unwrap();
    settle(&mut engine, &mut wide, Duration::from_secs(5));
    app.frame_committed(&engine);
    assert_eq!(engine.runtime().top_source(), Some(target));
    assert!(wide.terminal().text().contains("LATE_RESIZE_TARGET"));
    app.handle_key(&mut engine, KeyCode::Right, KeyModifiers::ALT)
        .unwrap();
    settle(&mut engine, &mut wide, Duration::from_secs(6));
    app.frame_committed(&engine);
    assert_eq!(engine.runtime().top_source(), Some(start));
    assert_terminal_matches_screen(wide.terminal(), engine.runtime().screen());
}
