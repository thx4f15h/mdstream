use std::time::Duration;

use crossterm::event::{KeyCode, KeyModifiers};
use mdtui_tui::{
    app::AppState,
    cli::LinkMode,
    render::frame::{FrameGeometry, FrameRuntime},
    stream::{ANCHOR_WORK, FRAME_WORK, IDLE_POLL_INTERVAL, StreamEngine},
};
use mdtui3::{Budget, WorkLimit};

fn runtime() -> FrameRuntime {
    FrameRuntime::new(
        FrameGeometry {
            rows: 12,
            cols: 40,
            content_width: 40,
        },
        LinkMode::Default,
    )
    .unwrap()
}

fn settle(engine: &mut StreamEngine, output: &mut Vec<u8>) {
    for _ in 0..1000 {
        if engine.anchor_build_pending() {
            engine
                .advance_anchor_index(&mut Budget::new(FRAME_WORK))
                .unwrap();
        }
        if !engine.due(Duration::from_secs(1)) {
            return;
        }
        engine
            .render_due(output, &mut Budget::new(FRAME_WORK), Duration::from_secs(1))
            .unwrap();
    }
    panic!("empty input left a frame permanently due");
}

fn empty_preview_ui() -> mdtui_tui::fold_ui::FoldUi {
    use mdtui_tui::{
        fold_ui::FoldUi,
        render::{checkpoint::RenderCheckpoint, objects::SharedLinks},
    };
    use mdtui3::{Document, FoldView};

    let doc = Document::new();
    let mut view = FoldView::<RenderCheckpoint>::with_state(&doc, 1).unwrap();
    let links = SharedLinks::new(&doc);
    let mut ui = FoldUi::default();
    ui.open(0, None, None);
    ui.advance(&doc, &mut view, &links, 40, &mut Budget::new(FRAME_WORK));
    for _ in 0..4 {
        assert!(ui.key(KeyCode::Down));
    }
    assert!(ui.key(KeyCode::Enter));
    ui.advance(&doc, &mut view, &links, 40, &mut Budget::new(FRAME_WORK));
    assert!(!ui.pending());
    ui
}

#[test]
fn empty_preview_status_keeps_the_target_notice_and_escape_action_at_20_and_40_columns() {
    let ui = empty_preview_ui();
    assert_eq!(ui.status(20).as_deref(), Some("No target Esc=resume"));
    for width in [40, 120] {
        assert_eq!(
            ui.status(width).as_deref(),
            Some("No fold target yet; Esc to resume input")
        );
    }
}

#[test]
fn navigating_after_empty_preview_feedback_shows_the_selected_menu_action() {
    for (key, page, label) in [
        (KeyCode::Up, "node", "parent candidate"),
        (KeyCode::Down, "node", "end preview"),
        (KeyCode::Home, "node", "toggle"),
        (KeyCode::End, "node", "collapse siblings"),
        (
            KeyCode::Left,
            "new defaults",
            "default toggle h1 (only new)",
        ),
        (KeyCode::Right, "document", "keep current path"),
    ] {
        let mut ui = empty_preview_ui();
        assert!(ui.key(key));
        let wide = ui.status(120).unwrap();
        assert!(
            wide.contains(&format!("{page}: {label}")),
            "{key:?}: {wide}"
        );
        for width in [20, 40, 120] {
            let status = ui.status(width).unwrap();
            assert!(
                !status.contains("No target") && !status.contains("No fold target"),
                "{key:?}, {width}: {status}"
            );
        }
    }
}

#[test]
fn empty_preview_grid_has_a_complete_notice_at_20_40_and_120_columns() {
    for (width, notice) in [
        (20, "No target Esc=resume"),
        (40, "No fold target yet; Esc to resume input"),
        (120, "No fold target yet; Esc to resume input"),
    ] {
        let mut engine = StreamEngine::new(runtime());
        engine.resize(12, width).unwrap();
        let mut app = AppState::default();
        let mut output = Vec::new();
        app.handle_key(&mut engine, KeyCode::Char('z'), KeyModifiers::NONE)
            .unwrap();
        settle(&mut engine, &mut output);
        for _ in 0..4 {
            app.handle_key(&mut engine, KeyCode::Down, KeyModifiers::NONE)
                .unwrap();
        }
        app.handle_key(&mut engine, KeyCode::Enter, KeyModifiers::NONE)
            .unwrap();
        settle(&mut engine, &mut output);
        assert_eq!(engine.metrics().read_bytes, 0);
        assert!(!engine.runtime().fold_pending());
        let mut parser = vt100::Parser::new(12.try_into().unwrap(), width.try_into().unwrap(), 0);
        parser.process(&output);
        for row in 0..12 {
            let actual = (0..width)
                .map(|col| {
                    parser
                        .screen()
                        .cell(row, col)
                        .unwrap()
                        .contents()
                        .chars()
                        .next()
                        .unwrap_or(' ')
                })
                .collect::<String>();
            assert_eq!(
                actual.trim_end(),
                if row == 11 { notice } else { "" },
                "width {width}, row {row}"
            );
        }
        app.handle_key(&mut engine, KeyCode::Esc, KeyModifiers::NONE)
            .unwrap();
        settle(&mut engine, &mut output);
        assert!(!engine.runtime().fold_busy());
        engine.ingest_data(b"READY\n", Duration::ZERO).unwrap();
        settle(&mut engine, &mut output);
        parser.process(&output);
        assert!(parser.screen().contents().contains("READY"));
    }
}

#[test]
fn pre_input_redraw_settles_and_accepts_first_bytes_after_escape() {
    for keys in [vec![KeyCode::Esc], vec![KeyCode::Char('z'), KeyCode::Esc]] {
        let mut engine = StreamEngine::new(runtime());
        let mut app = AppState::default();
        let mut output = Vec::new();
        for key in keys {
            app.handle_key(&mut engine, key, KeyModifiers::NONE)
                .unwrap();
        }
        // A zero budget cannot publish, consume input or invent work credit.
        let mut zero = Budget::new(WorkLimit::default());
        assert!(
            engine
                .render_due(&mut output, &mut zero, Duration::ZERO)
                .unwrap()
                .is_none()
        );
        assert_eq!(zero.used(), WorkLimit::default());
        assert!(output.is_empty());
        assert_eq!(engine.metrics().read_bytes, 0);
        settle(&mut engine, &mut output);
        assert!(!engine.runtime().fold_busy());
        assert_eq!(
            engine.poll_timeout(Duration::from_secs(1)),
            IDLE_POLL_INTERVAL
        );
        assert_eq!(engine.metrics().read_bytes, 0);
        engine.ingest_data(b"READY\n", Duration::ZERO).unwrap();
        settle(&mut engine, &mut output);
        let mut parser = vt100::Parser::new(12.try_into().unwrap(), 40.try_into().unwrap(), 0);
        parser.process(&output);
        assert!(parser.screen().contents().contains("READY"));
        assert_eq!(engine.metrics().read_bytes, 6);
        engine.ingest_data(b"NEXT\n", Duration::ZERO).unwrap();
        settle(&mut engine, &mut output);
        engine.finish_input(Duration::ZERO).unwrap();
        for _ in 0..1000 {
            if !engine.calibration_pending() {
                break;
            }
            engine
                .advance_calibration(&mut Budget::new(FRAME_WORK))
                .unwrap();
            if engine.due(Duration::from_secs(1)) {
                settle(&mut engine, &mut output);
            }
        }
        assert!(!engine.calibration_pending());
        settle(&mut engine, &mut output);
        parser.process(&output);
        assert!(parser.screen().contents().contains("NEXT"));
        assert!(engine.input_finished());
        assert!(
            app.handle_key(&mut engine, KeyCode::Char('q'), KeyModifiers::NONE)
                .unwrap()
                .quit
        );
    }
}

#[test]
fn empty_menu_uses_frame_runtime_and_resumes_after_every_page() {
    for page in 0..6 {
        let mut engine = StreamEngine::new(runtime());
        let mut app = AppState::default();
        let mut output = Vec::new();
        let opened = app
            .handle_key(&mut engine, KeyCode::Char('z'), KeyModifiers::NONE)
            .unwrap();
        assert!(opened.redraw);
        assert!(engine.runtime().fold_busy());
        settle(&mut engine, &mut output);
        for _ in 0..page {
            app.handle_key(&mut engine, KeyCode::Right, KeyModifiers::NONE)
                .unwrap();
            settle(&mut engine, &mut output);
        }
        let mut parser = vt100::Parser::new(12.try_into().unwrap(), 40.try_into().unwrap(), 0);
        parser.process(&output);
        assert!(parser.screen().contents().contains("Enter Esc"));
        app.handle_key(&mut engine, KeyCode::Esc, KeyModifiers::NONE)
            .unwrap();
        settle(&mut engine, &mut output);
        assert!(!engine.runtime().fold_busy());
        engine.ingest_data(b"READY\n", Duration::ZERO).unwrap();
        settle(&mut engine, &mut output);
        parser.process(&output);
        assert!(parser.screen().contents().contains("READY"));
    }
}

#[test]
fn empty_frame_runtime_advances_menu_with_bounded_budget() {
    let mut runtime = runtime();
    runtime.open_fold_menu(false);
    let mut output = Vec::new();
    let mut zero = Budget::new(WorkLimit::default());
    assert!(!runtime.frame_to(&mut output, &mut zero).unwrap().committed);
    assert!(runtime.fold_pending());
    assert!(runtime.fold_menu_key(KeyCode::Esc));
    assert!(!runtime.fold_busy());
    assert!(
        runtime
            .frame_to(&mut output, &mut Budget::new(FRAME_WORK))
            .unwrap()
            .committed
    );
}

#[test]
fn empty_input_background_preparation_settles_with_the_terminal_budget() {
    let mut engine = StreamEngine::new(runtime());
    for _ in 0..1000 {
        if !engine.anchor_build_pending() {
            assert!(!engine.due(Duration::ZERO));
            assert_eq!(engine.poll_timeout(Duration::ZERO), IDLE_POLL_INTERVAL);
            return;
        }
        engine
            .advance_anchor_index(&mut Budget::new(ANCHOR_WORK))
            .unwrap();
    }
    panic!(
        "empty background preparation did not settle: {:?}",
        engine.runtime().slug_session_stats()
    );
}

#[test]
fn pre_input_menu_micro_frames_cancel_pending_work_and_keep_new_only_default() {
    let mut engine = StreamEngine::new(runtime());
    engine.resize(12, 120).unwrap();
    let mut app = AppState::default();
    let mut output = Vec::new();
    app.handle_key(&mut engine, KeyCode::Char('z'), KeyModifiers::NONE)
        .unwrap();
    let micro = WorkLimit {
        bytes: 6,
        queries: 2,
        objects: 2,
        ..FRAME_WORK
    };
    for _ in 0..10_000 {
        if !engine.due(Duration::ZERO) {
            break;
        }
        let mut budget = Budget::new(micro);
        engine
            .render_due(&mut output, &mut budget, Duration::ZERO)
            .unwrap();
        assert!(budget.used().bytes <= 6);
        assert!(budget.used().queries <= 2);
        assert!(budget.used().objects <= 2);
    }
    assert!(!engine.due(Duration::ZERO));
    assert!(engine.runtime().fold_busy());
    // Preview without a target follows the existing visible error path.
    for _ in 0..4 {
        app.handle_key(&mut engine, KeyCode::Down, KeyModifiers::NONE)
            .unwrap();
    }
    app.handle_key(&mut engine, KeyCode::Enter, KeyModifiers::NONE)
        .unwrap();
    settle(&mut engine, &mut output);
    let mut parser = vt100::Parser::new(12.try_into().unwrap(), 120.try_into().unwrap(), 0);
    parser.process(&output);
    assert!(
        parser
            .screen()
            .contents()
            .contains("No fold target yet; Esc to resume input"),
        "{}",
        parser.screen().contents()
    );
    // Page 5's first command changes the h1 default for future nodes.
    app.handle_key(&mut engine, KeyCode::Left, KeyModifiers::NONE)
        .unwrap();
    app.handle_key(&mut engine, KeyCode::Enter, KeyModifiers::NONE)
        .unwrap();
    settle(&mut engine, &mut output);
    assert!(
        engine
            .runtime()
            .viewport()
            .folds()
            .defaults()
            .heading_levels[0]
    );
    // Queue another toggle, deny its budget, then cancel the actual pending op.
    app.handle_key(&mut engine, KeyCode::Enter, KeyModifiers::NONE)
        .unwrap();
    assert!(engine.runtime().fold_pending());
    engine
        .render_due(
            &mut output,
            &mut Budget::new(WorkLimit::default()),
            Duration::ZERO,
        )
        .unwrap();
    assert!(engine.runtime().fold_pending());
    assert!(
        app.handle_key(&mut engine, KeyCode::Char('q'), KeyModifiers::NONE)
            .unwrap()
            .quit
    );
    app.handle_key(&mut engine, KeyCode::Esc, KeyModifiers::NONE)
        .unwrap();
    settle(&mut engine, &mut output);
    assert!(!engine.runtime().fold_busy());
    assert!(
        engine
            .runtime()
            .viewport()
            .folds()
            .defaults()
            .heading_levels[0]
    );
    engine
        .ingest_data(b"# A\nSECRET\n\n# B\nTAIL\n", Duration::ZERO)
        .unwrap();
    settle(&mut engine, &mut output);
    parser.process(&output);
    let text = parser.screen().contents();
    assert!(text.contains("[+ heading"), "{text}");
    assert!(!text.contains("SECRET"), "{text}");
}
