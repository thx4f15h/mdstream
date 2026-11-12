mod support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossterm::event::{KeyCode, KeyModifiers};
use mdtui_tui::app::{AppState, FocusTarget};
use mdtui_tui::cli::LinkMode;
use mdtui_tui::external::{
    ExternalOpenOutcome, ExternalOpenStatus, ExternalOpener, ExternalTarget,
};
use mdtui_tui::fragment::{AnchorBuildStatus, FragmentActivationStatus};
use mdtui_tui::render::frame::{
    BEGIN_SYNCHRONIZED_UPDATE, END_SYNCHRONIZED_UPDATE, FrameGeometry, FrameRuntime,
};
use mdtui_tui::stream::{IngestResult, StreamEngine};
use mdtui3::{Budget, WorkLimit};
use support::terminal::{FrameOutcome, Vt100Writer};

const WIDTH: u16 = 80;
const HEIGHT: u16 = 24;

#[derive(Clone)]
struct FakeOpener {
    calls: Arc<Mutex<Vec<String>>>,
    status: ExternalOpenStatus,
}

impl FakeOpener {
    fn new(status: ExternalOpenStatus) -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            status,
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl ExternalOpener for FakeOpener {
    fn open(&mut self, target: &ExternalTarget) -> ExternalOpenOutcome {
        self.calls.lock().unwrap().push(target.as_str().to_owned());
        ExternalOpenOutcome::new(self.status)
    }
}

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

fn commit_due(engine: &mut StreamEngine, writer: &mut Vt100Writer) {
    writer.begin_frame();
    match engine.render_due(writer, &mut unlimited(), Duration::ZERO) {
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

fn settle(engine: &mut StreamEngine, writer: &mut Vt100Writer) {
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
    if engine.due(Duration::ZERO) {
        commit_due(engine, writer);
    }
}

fn finished_engine(
    source: &str,
    mode: LinkMode,
    opener: FakeOpener,
    writer: &mut Vt100Writer,
) -> StreamEngine {
    let runtime = FrameRuntime::new(geometry(), mode).expect("runtime");
    let mut engine = StreamEngine::with_external_opener(runtime, Box::new(opener));
    assert_eq!(
        engine
            .ingest_data(source.as_bytes(), Duration::ZERO)
            .expect("ingest"),
        IngestResult::FirstFrame
    );
    commit_due(&mut engine, writer);
    assert!(engine.finish_input(Duration::ZERO).expect("finish"));
    settle(&mut engine, writer);
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
            AnchorBuildStatus::PendingDocument => panic!("document is unfinished"),
        }
    }
    panic!("anchor build did not finish");
}

fn focus_link(app: &mut AppState, engine: &mut StreamEngine, writer: &mut Vt100Writer) {
    let outcome = app
        .handle_key(engine, KeyCode::Tab, KeyModifiers::NONE)
        .expect("focus link");
    assert!(outcome.redraw);
    assert_eq!(app.focus(), FocusTarget::Link);
    settle(engine, writer);
}

fn all_output(writer: &Vt100Writer) -> Vec<u8> {
    writer
        .writes()
        .iter()
        .flat_map(|write| write.bytes.iter().copied())
        .collect()
}

fn assert_no_osc8(bytes: &[u8]) {
    assert!(!bytes.windows(4).any(|window| window == b"\x1b]8;"));
}

fn assert_notice_protocol(writer: &Vt100Writer, frame: usize, message: &str) {
    let bytes: Vec<_> = writer
        .frame_writes(frame)
        .iter()
        .flat_map(|write| write.bytes.iter().copied())
        .collect();
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
    assert!(writer.terminal().text().contains(message));
}

#[test]
fn default_and_no_links_never_open_while_open_external_uses_fake_opener() {
    let source = "[web](https://example.com/) and [mail](mailto:user@example.com)\n";
    for mode in [LinkMode::Default, LinkMode::NoOsc8] {
        let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
        let opener = FakeOpener::new(ExternalOpenStatus::Opened);
        let calls = opener.clone();
        let mut engine = finished_engine(source, mode, opener, &mut writer);
        let mut app = AppState::default();
        focus_link(&mut app, &mut engine, &mut writer);
        let outcome = app
            .handle_key(&mut engine, KeyCode::Enter, KeyModifiers::NONE)
            .expect("external activation");
        assert_eq!(
            outcome.fragment_activation.expect("fragment status").status,
            FragmentActivationStatus::External
        );
        assert_eq!(outcome.external_open, None);
        assert!(calls.calls().is_empty());
        assert!(app.history().is_empty());
        assert!(!outcome.redraw);
        let output = all_output(&writer);
        match mode {
            LinkMode::Default => assert!(
                output
                    .windows(b"\x1b]8;;https://example.com/".len())
                    .any(|window| window == b"\x1b]8;;https://example.com/")
            ),
            LinkMode::NoOsc8 => assert_no_osc8(&output),
            LinkMode::OpenExternal => unreachable!(),
        }
    }

    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    let opener = FakeOpener::new(ExternalOpenStatus::Opened);
    let calls = opener.clone();
    let mut engine = finished_engine(source, LinkMode::OpenExternal, opener, &mut writer);
    let mut app = AppState::default();
    focus_link(&mut app, &mut engine, &mut writer);
    let outcome = app
        .handle_key(&mut engine, KeyCode::Enter, KeyModifiers::NONE)
        .expect("open external");
    assert_eq!(calls.calls(), vec!["https://example.com/".to_owned()]);
    assert_eq!(
        outcome.external_open.expect("open outcome").status,
        ExternalOpenStatus::Opened
    );
    assert_eq!(
        outcome.fragment_activation.expect("fragment status").status,
        FragmentActivationStatus::External
    );
    assert!(app.history().is_empty());
    settle(&mut engine, &mut writer);
    let frame = writer.frames().len() - 1;
    assert_notice_protocol(&writer, frame, "已打开外部链接");
}

#[test]
fn mailto_uses_link_url_normalization() {
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    let opener = FakeOpener::new(ExternalOpenStatus::Opened);
    let calls = opener.clone();
    let mut engine = finished_engine(
        "<user@example.com>\n",
        LinkMode::OpenExternal,
        opener,
        &mut writer,
    );
    let mut app = AppState::default();
    focus_link(&mut app, &mut engine, &mut writer);
    app.handle_key(&mut engine, KeyCode::Enter, KeyModifiers::NONE)
        .expect("open mail");
    assert_eq!(calls.calls(), vec!["mailto:user@example.com".to_owned()]);
    assert!(app.history().is_empty());
}

#[test]
fn invalid_and_unsupported_targets_do_not_reach_the_opener() {
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    let opener = FakeOpener::new(ExternalOpenStatus::Opened);
    let calls = opener.clone();
    let mut engine = finished_engine(
        "[bad](ftp://example.com/file)\n",
        LinkMode::OpenExternal,
        opener,
        &mut writer,
    );
    let mut app = AppState::default();
    focus_link(&mut app, &mut engine, &mut writer);
    let outcome = app
        .handle_key(&mut engine, KeyCode::Enter, KeyModifiers::NONE)
        .expect("unsupported external");
    assert_eq!(
        outcome.external_open.expect("open outcome").status,
        ExternalOpenStatus::Unsupported
    );
    assert!(calls.calls().is_empty());
    assert!(app.history().is_empty());
    settle(&mut engine, &mut writer);
    let frame = writer.frames().len() - 1;
    assert_notice_protocol(&writer, frame, "不支持打开该链接");

    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    let opener = FakeOpener::new(ExternalOpenStatus::Opened);
    let calls = opener.clone();
    let mut engine = finished_engine("[empty]()\n", LinkMode::OpenExternal, opener, &mut writer);
    let mut app = AppState::default();
    focus_link(&mut app, &mut engine, &mut writer);
    let outcome = app
        .handle_key(&mut engine, KeyCode::Enter, KeyModifiers::NONE)
        .expect("empty external");
    assert_eq!(
        outcome.fragment_activation.expect("fragment status").status,
        FragmentActivationStatus::Invalid
    );
    assert_eq!(outcome.external_open, None);
    assert!(calls.calls().is_empty());
}

#[test]
fn opener_failure_is_reported_without_internal_history() {
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    let opener = FakeOpener::new(ExternalOpenStatus::Failed);
    let calls = opener.clone();
    let mut engine = finished_engine(
        "[web](https://example.com/)\n",
        LinkMode::OpenExternal,
        opener,
        &mut writer,
    );
    let mut app = AppState::default();
    focus_link(&mut app, &mut engine, &mut writer);
    let outcome = app
        .handle_key(&mut engine, KeyCode::Enter, KeyModifiers::NONE)
        .expect("failed open");
    assert_eq!(calls.calls(), vec!["https://example.com/".to_owned()]);
    assert_eq!(
        outcome.external_open.expect("open outcome").status,
        ExternalOpenStatus::Failed
    );
    assert!(app.history().is_empty());
    settle(&mut engine, &mut writer);
    let frame = writer.frames().len() - 1;
    assert_notice_protocol(&writer, frame, "打开外部链接失败");
}

#[test]
fn no_links_keeps_internal_fragment_focus_and_jump() {
    let source = "[go](#target)\n\n# Target\n\nbody\n";
    let mut writer = Vt100Writer::new(HEIGHT, WIDTH);
    let opener = FakeOpener::new(ExternalOpenStatus::Opened);
    let calls = opener.clone();
    let mut engine = finished_engine(source, LinkMode::NoOsc8, opener, &mut writer);
    build_anchors(&mut engine);
    let mut app = AppState::default();
    focus_link(&mut app, &mut engine, &mut writer);
    let outcome = app
        .handle_key(&mut engine, KeyCode::Enter, KeyModifiers::NONE)
        .expect("internal jump");
    assert_eq!(
        outcome.fragment_activation.expect("fragment status").status,
        FragmentActivationStatus::Moved
    );
    assert_eq!(app.history().len(), 1);
    assert!(calls.calls().is_empty());
    assert_eq!(
        engine.runtime().top_source(),
        Some(engine.document().headings().entries()[0].source.start)
    );
    assert_no_osc8(&all_output(&writer));
}
