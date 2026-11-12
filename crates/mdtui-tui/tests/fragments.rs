mod support;

use std::time::Duration;

use crossterm::event::{KeyCode, KeyModifiers};
use mdtui_tui::app::{AppState, FocusTarget};
use mdtui_tui::cli::LinkMode;
use mdtui_tui::fragment::{AnchorBuildStatus, FragmentActivationStatus};
use mdtui_tui::render::frame::{
    BEGIN_SYNCHRONIZED_UPDATE, END_SYNCHRONIZED_UPDATE, FrameGeometry, FrameRuntime,
};
use mdtui_tui::stream::{ANCHOR_POLL_INTERVAL, IngestResult, StreamEngine};
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
    let top = engine.runtime().top_source();
    assert!(!engine.document().is_finished());
    assert_eq!(
        engine
            .advance_anchor_index(&mut unlimited())
            .expect("pre-finish anchor step"),
        AnchorBuildStatus::PendingDocument
    );
    assert!(!engine.anchor_index_started());

    let pending = activate(&mut app, &mut engine);
    assert_eq!(pending.status, FragmentActivationStatus::PendingDocument);
    assert_eq!(pending.status.message(), "待文档完成");
    assert_eq!(
        app.last_fragment_activation()
            .map(|activation| activation.status),
        Some(FragmentActivationStatus::PendingDocument)
    );
    assert_eq!(engine.runtime().top_source(), top);
    assert!(app.history().is_empty());
    assert!(!engine.anchor_index_started());
    assert_eq!(engine.link_focus(), Some(link));
    assert_eq!(engine.notice(), Some("待文档完成"));
    assert!(engine.due(Duration::ZERO));
    settle(&mut engine, &mut writer, Duration::ZERO);
    assert!(writer.terminal().text().contains("待文档完成"));
    assert_eq!(engine.runtime().top_source(), top);
    assert!(app.history().is_empty());
    let notice_frame = writer.frames().len() - 1;
    let notice_bytes: Vec<_> = writer
        .frame_writes(notice_frame)
        .iter()
        .flat_map(|write| write.bytes.iter().copied())
        .collect();
    assert_incremental_frame(&notice_bytes);
    assert!(
        engine
            .runtime()
            .screen()
            .front_cell(HEIGHT - 1, 0)
            .expect("notice cell")
            .style()
            .contains(mdtui_tui::screen::StyleRole::META)
    );

    assert!(engine.finish_input(Duration::ZERO).expect("finish"));
    settle(&mut engine, &mut writer, Duration::ZERO);
    assert!(engine.anchor_build_pending());
    assert!(engine.poll_timeout(Duration::ZERO) <= ANCHOR_POLL_INTERVAL);
    assert!(!engine.anchor_index_complete());
    build_anchors(&mut engine);
    assert!(engine.anchor_index_complete());
    assert!(!engine.anchor_build_pending());
    assert_eq!(engine.link_focus(), Some(link));

    let activation = activate(&mut app, &mut engine);
    assert_eq!(activation.status, FragmentActivationStatus::Moved);
    assert!(activation.moved);
    let target = engine.document().headings().entries()[0].source.start;
    assert_eq!(activation.target.expect("target").source, target);
    assert_eq!(engine.runtime().top_source(), Some(target));
    assert_eq!(app.history().len(), 1);
    assert_eq!(app.history().last_source(), Some(target));
    assert_eq!(engine.notice(), None);
    settle(&mut engine, &mut writer, Duration::ZERO);
    assert!(!writer.terminal().text().contains("待文档完成"));

    let frame = writer.frames().len() - 1;
    let bytes: Vec<_> = writer
        .frame_writes(frame)
        .iter()
        .flat_map(|write| write.bytes.iter().copied())
        .collect();
    assert_incremental_frame(&bytes);
    assert_terminal_matches_screen(writer.terminal(), engine.runtime().screen());
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
    settle(&mut engine, &mut writer, Duration::ZERO);
    assert!(writer.terminal().text().contains("正在建立锚点索引"));
    assert_eq!(engine.anchor_indexed_headings(), Some(partial));
    assert_eq!(engine.runtime().top_source(), top);
    assert!(app.history().is_empty());
    assert_eq!(engine.link_focus(), Some(link));

    let mut resumed = partial;
    for _ in 0..200_000 {
        let mut budget = Budget::new(tiny);
        if engine
            .advance_anchor_index(&mut budget)
            .expect("resume anchor build")
            == AnchorBuildStatus::Complete
        {
            break;
        }
        let current = engine.anchor_indexed_headings().unwrap_or(0);
        assert!(current >= resumed);
        resumed = current;
    }
    assert!(engine.anchor_index_complete());
    let total_headings = engine.document().headings().entries().len();
    assert_eq!(engine.anchor_indexed_headings(), Some(total_headings));
    assert_eq!(
        engine
            .advance_anchor_index(&mut unlimited())
            .expect("completed index is idempotent"),
        AnchorBuildStatus::Complete
    );
    assert_eq!(engine.anchor_indexed_headings(), Some(total_headings));
    assert!(
        !engine
            .finish_input(Duration::ZERO)
            .expect("repeated finish")
    );
    assert_eq!(engine.anchor_indexed_headings(), Some(total_headings));
    let target = engine.document().headings().entries()[0].source.start;
    let activation = activate(&mut app, &mut engine);
    assert_eq!(activation.status, FragmentActivationStatus::Moved);
    assert_eq!(engine.notice(), None);
    assert_eq!(engine.anchor_indexed_headings(), Some(total_headings));
    assert_eq!(engine.runtime().top_source(), Some(target));
    assert_eq!(app.history().len(), 1);
    assert_eq!(app.history().last_source(), Some(target));
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
