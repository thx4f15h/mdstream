use std::io;

use crossterm::event::{KeyCode, KeyModifiers};
use mdtui_tui::{
    app::{AppState, KeyAction},
    cli::LinkMode,
    render::frame::{FrameGeometry, FrameRuntime},
    stream::{FRAME_WORK, StreamEngine},
};
use mdtui3::{Budget, WorkLimit};

fn engine(source: &str, width: u16, rows: u16) -> StreamEngine {
    let mut runtime = FrameRuntime::new(
        FrameGeometry {
            rows,
            cols: width,
            content_width: width,
        },
        LinkMode::Default,
    )
    .unwrap();
    let update = runtime.append_bytes(source.as_bytes()).unwrap().unwrap();
    runtime.apply_update(&update).unwrap();
    StreamEngine::new(runtime)
}
fn frame(engine: &mut StreamEngine) -> usize {
    // Runtime entry permits tests to inspect each paid frame separately from
    // the engine's input throttling and first-frame policy.
    for step in 1..=256 {
        let mut budget = Budget::new(FRAME_WORK);
        let commit = engine
            .runtime_mut()
            .frame_to(&mut io::sink(), &mut budget)
            .unwrap();
        assert!(budget.used().queries <= FRAME_WORK.queries);
        assert!(budget.used().bytes <= FRAME_WORK.bytes);
        assert!(budget.used().objects <= FRAME_WORK.objects);
        assert!(budget.used().tiles <= FRAME_WORK.tiles);
        assert!(budget.used().recovery <= FRAME_WORK.recovery);
        assert!(budget.used().dependencies <= FRAME_WORK.dependencies);
        assert!(commit.metrics.planned_tiles <= FRAME_WORK.tiles);
        assert!(commit.metrics.layout.rows <= 8192);
        assert!(commit.metrics.layout.segments_consumed <= 4096);
        assert!(commit.metrics.layout.events_consumed <= 4096);
        assert_eq!(commit.metrics.layout.document_queries_in_callbacks, 0);
        if commit.committed {
            return step;
        }
    }
    panic!("tail did not settle");
}
fn append(engine: &mut StreamEngine, source: &str) {
    if let Some(update) = engine
        .runtime_mut()
        .append_bytes(source.as_bytes())
        .unwrap()
    {
        engine.runtime_mut().apply_update(&update).unwrap();
    }
}
fn text(engine: &StreamEngine) -> String {
    let screen = engine.screen();
    (0..screen.rows())
        .map(|r| {
            (0..screen.cols())
                .map(|c| screen.front_cell(r, c).unwrap().text())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
fn lines(engine: &StreamEngine) -> Vec<String> {
    text(engine)
        .lines()
        .map(|s| s.trim_end().to_owned())
        .collect()
}
fn end(app: &mut AppState, engine: &mut StreamEngine) {
    app.handle_key(engine, KeyCode::End, KeyModifiers::NONE)
        .unwrap();
    frame(engine);
}

#[test]
fn follow_appends_same_tile_cross_tile_and_unicode_without_history() {
    let mut engine = engine("old\n\nbase\n", 12, 5);
    let mut app = AppState::default();
    frame(&mut engine);
    end(&mut app, &mut engine);
    assert_eq!(&lines(&engine)[..4], ["old", "", "base", ""]);
    for suffix in ["\none\n", "\n中文🙂abcdefghi", "jklmnop", "\n\nTAIL\n"] {
        append(&mut engine, suffix);
        frame(&mut engine);
        assert!(text(&engine).contains("FOLLOW"));
        assert!(app.history().is_empty());
    }
    assert!(text(&engine).contains("TAIL"));
    let source = engine.runtime().document().source().rfind("TAIL").unwrap();
    assert!((0..4).any(|r| {
        engine
            .screen()
            .front_line_source(r)
            .is_some_and(|s| s.source.start == source)
    }));
    append(
        &mut engine,
        &format!("\n{}\n\nCROSS_TILE\n", "new ".repeat(150)),
    );
    frame(&mut engine);
    assert!(text(&engine).contains("CROSS_TILE"));
    for ch in "\n逐字追加🙂\n\nFINAL\n".chars() {
        append(&mut engine, &ch.to_string());
        frame(&mut engine);
    }
    assert!(text(&engine).contains("FINAL"));
    assert!(app.history().is_empty());
}

#[test]
fn once_freezes_pending_source_cut_and_reenter_end_follows() {
    let mut engine = engine(&format!("{}\n\nCUT\n", "old\n\n".repeat(100)), 20, 5);
    let mut app = AppState::default();
    end(&mut app, &mut engine);
    app.handle_key(&mut engine, KeyCode::End, KeyModifiers::CONTROL)
        .unwrap();
    append(&mut engine, "\nNEW_MUST_WAIT\n");
    frame(&mut engine);
    assert!(!engine.runtime().viewport().following());
    assert!(text(&engine).contains("CUT"));
    assert!(!text(&engine).contains("NEW_MUST_WAIT"));
    let old = engine.runtime().top_source();
    append(&mut engine, "\nANOTHER_NEW\n");
    frame(&mut engine);
    assert_eq!(engine.runtime().top_source(), old);
    end(&mut app, &mut engine);
    assert!(text(&engine).contains("ANOTHER_NEW"));
}

#[test]
fn up_page_home_history_and_sidebar_navigation_exit_follow() {
    for action in [
        KeyAction::LineUp,
        KeyAction::PageUp,
        KeyAction::Home,
        KeyAction::LineDown,
        KeyAction::PageDown,
        KeyAction::HistoryBack,
        KeyAction::HistoryForward,
    ] {
        let mut engine = engine(
            &format!("# Top\n\n{}\n# Last\n\nTAIL\n", "body\n\n".repeat(80)),
            60,
            8,
        );
        let mut app = AppState::default();
        end(&mut app, &mut engine);
        app.handle_action(&mut engine, action).unwrap();
        frame(&mut engine);
        assert!(!engine.runtime().viewport().following(), "{action:?}");
        let old = engine.runtime().top_source();
        append(
            &mut engine,
            &format!("\n{}\nAPPEND\n", "new\n\n".repeat(40)),
        );
        frame(&mut engine);
        assert_eq!(engine.runtime().top_source(), old, "{action:?}");
    }
    let mut engine = engine("# Top\n\nbody\n\n# Last\n\ntail\n", 80, 8);
    let mut app = AppState::default();
    end(&mut app, &mut engine);
    app.handle_action(&mut engine, KeyAction::ToggleSidebar)
        .unwrap();
    frame(&mut engine);
    let focus_outcome = app
        .handle_action(&mut engine, KeyAction::FocusNext)
        .unwrap();
    assert!(
        !engine.runtime().viewport().following(),
        "focus {:?} outcome {:?} sidebar {}",
        app.focus(),
        focus_outcome,
        app.sidebar_visible()
    );
    app.handle_action(&mut engine, KeyAction::End).unwrap();
    assert_eq!(
        engine.runtime().sidebar_selection(),
        Some(mdtui3::HeadingId(1))
    );
    app.handle_action(&mut engine, KeyAction::Activate).unwrap();
    frame(&mut engine);
    assert_eq!(engine.runtime().top_source(), Some(13));
}

#[test]
fn pending_long_line_late_code_and_resize_keep_actual_tail() {
    let mut engine = engine(&format!("before\n\n{}", "界🙂word ".repeat(200)), 20, 7);
    let mut app = AppState::default();
    end(&mut app, &mut engine);
    append(&mut engine, "ENDLINE");
    frame(&mut engine);
    assert!(lines(&engine)[4].ends_with("ENDL"));
    assert_eq!(lines(&engine)[5], "INE");
    let endline = engine
        .runtime()
        .document()
        .source()
        .rfind("ENDLINE")
        .unwrap();
    assert_eq!(
        engine.screen().front_line_source(5).unwrap().source.start,
        endline + 4
    );
    append(&mut engine, "\n\n```rust\ncode tail\n```");
    frame(&mut engine);
    assert!(text(&engine).contains("code tail"));
    app.resize(&mut engine, 10, 12).unwrap();
    frame(&mut engine);
    assert!(text(&engine).contains("code tail"));
    assert!(engine.runtime().viewport().following());
    let update = engine.runtime_mut().finish_input().unwrap().unwrap();
    engine.runtime_mut().apply_update(&update).unwrap();
    frame(&mut engine);
    assert!(text(&engine).contains("code tail"));
    assert!(text(&engine).contains("FOLLOW"));
}

#[test]
fn empty_hidden_tail_noops_and_zero_budget() {
    for initial in ["", "[a]: /a\n[b]: /b\n", "```rust\n```\n"] {
        let mut engine = if initial.is_empty() {
            StreamEngine::new(
                FrameRuntime::new(
                    FrameGeometry {
                        rows: 5,
                        cols: 20,
                        content_width: 20,
                    },
                    LinkMode::Default,
                )
                .unwrap(),
            )
        } else {
            engine(initial, 20, 5)
        };
        let mut app = AppState::default();
        app.handle_action(&mut engine, KeyAction::End).unwrap();
        let mut budget = Budget::new(WorkLimit::default());
        let commit = engine
            .runtime_mut()
            .frame_to(&mut io::sink(), &mut budget)
            .unwrap();
        assert!(!commit.committed);
        assert_eq!(budget.used(), WorkLimit::default());
        frame(&mut engine);
        let before = text(&engine);
        if initial.starts_with('[') {
            assert_eq!(&lines(&engine)[..4], ["", "", "", ""]);
        }
        if initial.starts_with('`') {
            assert!(before.contains("rust"));
        }
        for _ in 0..10 {
            assert!(
                !app.handle_action(&mut engine, KeyAction::End)
                    .unwrap()
                    .redraw
            );
            assert!(
                !app.handle_key(&mut engine, KeyCode::Char('z'), KeyModifiers::NONE)
                    .unwrap()
                    .redraw
            );
            assert!(engine.runtime_mut().append_bytes(b"").unwrap().is_none());
        }
        frame(&mut engine);
        assert_eq!(text(&engine), before);
        append(&mut engine, "\nVISIBLE\n");
        frame(&mut engine);
        assert!(text(&engine).contains("VISIBLE"));
    }
}

#[test]
fn hidden_definitions_and_huge_empty_code_info_skip_prefix_layout() {
    for suffix in [
        format!("\n\n{}", "[same]: /url\n".repeat(40_000)),
        format!("\n\n```rust {}\n```\n", "x".repeat(1024 * 1024)),
    ] {
        let mut engine = engine(&format!("VISIBLE_BEFORE\n{suffix}"), 40, 6);
        let mut app = AppState::default();
        end(&mut app, &mut engine);
        assert!(
            text(&engine).contains("VISIBLE_BEFORE"),
            "{}",
            text(&engine)
        );
        let work = engine.runtime().viewport().end_work();
        assert!(work.source_bytes < 32 * 1024, "{work:?}");
        assert!(work.planned_tiles < 32, "{work:?}");
        if suffix.contains("```rust") {
            assert!(text(&engine).contains("rust"));
        }
    }
}

#[test]
fn repeated_update_preserves_pending_tail_and_bounded_resume() {
    use mdtui_tui::render::viewport::{ViewportConfig, ViewportFrameStatus, ViewportState};
    use mdtui3::Document;
    let mut doc = Document::new();
    let mut view = ViewportState::new(
        &doc,
        ViewportConfig {
            width: 10,
            rows: 5,
            ..ViewportConfig::default()
        },
    )
    .unwrap();
    let update = doc
        .append(&format!("{}\n\nFINAL_TAIL\n", "line ".repeat(1000)))
        .unwrap();
    view.apply_update(&doc, &update).unwrap();
    view.seek_end(&doc, true).unwrap();
    let micro = WorkLimit {
        queries: 32,
        tiles: 1,
        bytes: 32,
        objects: 4,
        recovery: 1,
        dependencies: 4,
    };
    for _ in 0..5 {
        let f = view.frame(&doc, &mut Budget::new(micro)).unwrap();
        assert_eq!(f.status, ViewportFrameStatus::BudgetExhausted);
        assert!(!view.apply_update(&doc, &update).unwrap().discarded_render);
    }
    let mut ready = None;
    for _ in 0..64 {
        let f = view.frame(&doc, &mut Budget::new(FRAME_WORK)).unwrap();
        if f.status == ViewportFrameStatus::EndOfDocument {
            ready = Some(f);
            break;
        }
    }
    let ready = ready.unwrap();
    let last = ready.lines.last().unwrap();
    assert_eq!(
        last.cells
            .iter()
            .map(|c| c.text.as_str())
            .collect::<String>(),
        "FINAL_TAIL"
    );
    assert_eq!(
        last.source.unwrap().source.start,
        doc.source().rfind("FINAL_TAIL").unwrap()
    );
    let noop = doc.append("").unwrap();
    view.apply_update(&doc, &noop).unwrap();
    assert_eq!(
        view.frame(&doc, &mut Budget::new(WorkLimit::default()))
            .unwrap()
            .lines,
        ready.lines
    );
}

#[test]
fn sixty_four_mib_tail_updates_read_only_bounded_windows_and_retain_checkpoints() {
    let source = format!("{}\n\nLARGE_TAIL\n", "a".repeat(64 * 1024 * 1024));
    let mut engine = engine(&source, 80, 24);
    let mut app = AppState::default();
    let parser_work = engine.runtime().document().work();
    assert!(parser_work.input_bytes >= 64 * 1024 * 1024);
    app.handle_action(&mut engine, KeyAction::End).unwrap();
    let initial_steps = frame(&mut engine);
    assert!(text(&engine).contains("LARGE_TAIL"));
    let initial = engine.runtime().viewport().end_work();
    assert!(initial.source_bytes < 32 * 1024, "{initial:?}");
    assert!(initial.planned_tiles <= 16, "{initial:?}");
    assert_eq!(
        engine.runtime().document().work(),
        parser_work,
        "query re-parsed source"
    );
    let mut peak_entries = 0;
    let mut peak_nodes = 0;
    for n in 0..64 {
        let marker = format!("\nnew-{n:03}\n");
        append(&mut engine, &marker);
        frame(&mut engine);
        assert!(text(&engine).contains(marker.trim()));
        let work = engine.runtime().viewport().end_work();
        assert!(work.source_bytes < 32 * 1024, "{work:?}");
        let view = engine.runtime().viewport();
        assert!(work.checkpoint_entries <= 16, "{work:?}");
        assert!(work.checkpoint_nodes < 256, "{work:?}");
        peak_entries = peak_entries.max(view.checkpoints().len());
        peak_nodes = peak_nodes.max(view.checkpoints().arena_stats().nodes);
    }
    assert!(peak_entries < 128, "{peak_entries}");
    assert!(peak_nodes < 512, "{peak_nodes}");
    eprintln!(
        "64MiB end: steps={initial_steps} {initial:?}; updates=64 checkpoint_peak_entries={peak_entries} checkpoint_peak_nodes={peak_nodes}; input_work={parser_work:?}; content_index={:?}",
        engine.runtime().document().content_block_index_stats()
    );
}

fn assert_static_tail(engine: &StreamEngine) {
    let width = engine.runtime().geometry().content_width;
    let want = engine.runtime().viewport().viewport_rows();
    let expected =
        mdtui_tui::static_render::render_static(engine.runtime().document(), width, 512).unwrap();
    let start = expected.stats.rows.saturating_sub(want);
    let count = expected.stats.rows.min(want);
    for row in 0..count {
        for col in 0..width {
            assert_eq!(
                engine.screen().front_cell(row, col),
                expected.screen.cell(start + row, col),
                "row {row} col {col} expected_total={} source={:?} actual={} ",
                expected.stats.rows,
                engine.runtime().document().source(),
                text(engine)
            );
        }
        assert_eq!(
            engine.screen().front_line_source(row),
            expected.screen.line_source(start + row),
            "row {row} source"
        );
    }
}

#[test]
fn late_setext_table_reference_and_quote_heights_reselect_real_last_screen() {
    for (first, chunks) in [
        ("title\n", vec!["===\n", "\nlast heading body\n"]),
        (
            "| a | b |\n",
            vec![
                "| --- | --- |\n",
                "| one | two |\n",
                "| wide long cell | x |\n",
            ],
        ),
        (
            "before [label][x]\n",
            vec!["\n[x]: /url\n", "\nlast ref body\n"],
        ),
        (
            "> ```rust\n> code\n",
            vec!["> tail\n", "> ```\n", "\nlast quote body\n"],
        ),
    ] {
        let mut engine = engine(&format!("{}\n{first}", "old\n\n".repeat(12)), 32, 6);
        let mut app = AppState::default();
        end(&mut app, &mut engine);
        for chunk in chunks {
            append(&mut engine, chunk);
            frame(&mut engine);
            assert_static_tail(&engine);
        }
        let update = engine.runtime_mut().finish_input().unwrap().unwrap();
        engine.runtime_mut().apply_update(&update).unwrap();
        frame(&mut engine);
        assert_static_tail(&engine);
    }
}

#[test]
fn stream_engine_batches_pause_finish_and_resize_keep_follow_intention() {
    use std::time::Duration;
    fn drain(engine: &mut StreamEngine, now: Duration) {
        for _ in 0..256 {
            let mut budget = Budget::new(FRAME_WORK);
            if engine.calibration_pending() {
                engine.advance_calibration(&mut budget).unwrap();
            }
            let _ = engine
                .render_due(&mut io::sink(), &mut budget, now)
                .unwrap();
            if !engine.due(now) && !engine.calibration_pending() {
                return;
            }
        }
        panic!("stream did not drain");
    }
    let mut engine = StreamEngine::new(
        FrameRuntime::new(
            FrameGeometry {
                rows: 6,
                cols: 30,
                content_width: 30,
            },
            LinkMode::Default,
        )
        .unwrap(),
    );
    engine
        .ingest_data(b"first\n\nbase\n", Duration::ZERO)
        .unwrap();
    drain(&mut engine, Duration::from_secs(1));
    let mut app = AppState::default();
    app.handle_action(&mut engine, KeyAction::End).unwrap();
    drain(&mut engine, Duration::from_secs(2));
    for _ in 0..64 {
        engine
            .ingest_data(b"\nnew batch\n", Duration::from_secs(2))
            .unwrap();
    }
    engine
        .ingest_data(b"\nSTREAM_LAST\n", Duration::from_secs(2))
        .unwrap();
    assert!(engine.pending_updates() > 0);
    drain(&mut engine, Duration::from_secs(3));
    assert!(text(&engine).contains("STREAM_LAST"));
    let saved = text(&engine);
    assert!(!engine.due(Duration::from_secs(20)));
    assert_eq!(text(&engine), saved);
    app.resize(&mut engine, 8, 20).unwrap();
    drain(&mut engine, Duration::from_secs(21));
    assert!(text(&engine).contains("STREAM_LAST"));
    engine.finish_input(Duration::from_secs(22)).unwrap();
    drain(&mut engine, Duration::from_secs(22));
    assert!(text(&engine).contains("STREAM_LAST"));
    assert!(engine.runtime().viewport().following());
    assert!(!engine.finish_input(Duration::from_secs(23)).unwrap());
    assert!(
        engine
            .ingest_data(b"forbidden", Duration::from_secs(23))
            .is_err()
    );
    assert!(app.history().is_empty());
}

#[test]
fn one_query_one_object_six_byte_frames_converge_to_cells_and_source() {
    use mdtui_tui::render::viewport::{ViewportConfig, ViewportFrameStatus, ViewportState};
    use mdtui3::Document;
    let cases = [
        String::new(),
        format!("{}TAIL_MICRO\n", "ordinary long paragraph ".repeat(300)),
        format!("visible\n\n{}", "[same]: /url\n".repeat(1000)),
        "one\n\ntwo\n\nthree\n\nLAST_MICRO\n".to_owned(),
    ];
    for source in cases {
        let mut doc = Document::new();
        doc.append(&source).unwrap();
        doc.finish();
        let mut view = ViewportState::new(
            &doc,
            ViewportConfig {
                width: 16,
                rows: 4,
                ..ViewportConfig::default()
            },
        )
        .unwrap();
        view.seek_end(&doc, true).unwrap();
        let mut zero = Budget::new(WorkLimit::default());
        assert_eq!(
            view.frame(&doc, &mut zero).unwrap().status,
            ViewportFrameStatus::BudgetExhausted
        );
        assert_eq!(zero.used(), WorkLimit::default());
        assert_eq!(view.end_work().windows, 0);
        let micro = WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 6,
            tiles: 1,
            recovery: 1,
            dependencies: 1,
        };
        let mut ready = None;
        for step in 0..100_000 {
            let mut budget = Budget::new(micro);
            let frame = view.frame(&doc, &mut budget).unwrap();
            assert!(
                budget.used().queries <= 1
                    && budget.used().objects <= 1
                    && budget.used().bytes <= 6
            );
            if frame.status == ViewportFrameStatus::EndOfDocument {
                ready = Some((step, frame));
                break;
            }
        }
        let (steps, frame) = ready.unwrap_or_else(|| {
            panic!(
                "one-query tail stuck for {} bytes; {:?}",
                source.len(),
                view.end_work()
            )
        });
        if source.is_empty() {
            assert!(frame.lines.is_empty());
        } else {
            let last = frame.lines.last().unwrap();
            let text = last
                .cells
                .iter()
                .map(|c| c.text.as_str())
                .collect::<String>();
            let expected = if source.contains("TAIL_MICRO") {
                "TAIL_MICRO"
            } else if source.contains("LAST_MICRO") {
                "LAST_MICRO"
            } else {
                "visible"
            };
            assert_eq!(text, expected);
            assert_eq!(
                last.source.unwrap().source.start,
                doc.source().rfind(expected).unwrap()
            );
        }
        eprintln!(
            "micro tail bytes={} steps={} work={:?}",
            source.len(),
            steps,
            view.end_work()
        );
    }
}

#[test]
fn empty_controls_headings_and_code_labels_keep_existing_cell_semantics() {
    for tail in [
        "-\n",
        "- [ ]\n",
        "#\n",
        "```\n```\n",
        "```rust\n```\n",
        "- ![](image.png)\n",
    ] {
        let mut engine = engine(&format!("earlier visible\n\n{tail}"), 32, 6);
        let update = engine.runtime_mut().finish_input().unwrap().unwrap();
        engine.runtime_mut().apply_update(&update).unwrap();
        let mut app = AppState::default();
        end(&mut app, &mut engine);
        assert_static_tail(&engine);
        assert!(text(&engine).contains("earlier visible"), "tail {tail:?}");
        if tail.contains("rust") {
            assert!(text(&engine).contains("rust"));
        }
        if tail.contains("image.png") {
            assert!(text(&engine).contains("[image]"));
        }
        // Empty markers intentionally emit no control in the established style.
        if tail == "-\n" || tail == "- [ ]\n" {
            assert!(!text(&engine).contains('☐'));
        }
    }
}
