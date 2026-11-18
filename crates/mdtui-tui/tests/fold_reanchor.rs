use crossterm::event::{KeyCode, KeyModifiers};
use mdtui_tui::{
    cli::LinkMode,
    render::frame::{FrameGeometry, FrameRuntime},
    stream::FRAME_WORK,
};
use mdtui3::{Budget, WorkLimit};

fn tick(runtime: &mut FrameRuntime, parser: &mut vt100::Parser, limit: WorkLimit) -> bool {
    let mut budget = Budget::new(limit);
    let mut bytes = Vec::new();
    runtime.advance_reading(&mut bytes, &mut budget);
    let commit = runtime.frame_to(&mut bytes, &mut budget).unwrap();
    let used = budget.used();
    assert!(
        used.queries <= limit.queries
            && used.objects <= limit.objects
            && used.bytes <= limit.bytes
            && used.tiles <= limit.tiles
            && used.recovery <= limit.recovery
            && used.dependencies <= limit.dependencies
    );
    parser.process(&bytes);
    commit.committed
}

fn draw(runtime: &mut FrameRuntime, parser: &mut vt100::Parser) -> String {
    for _ in 0..100_000 {
        if tick(runtime, parser, FRAME_WORK)
            && !runtime.fold_pending()
            && !runtime.reading().pending()
            && !runtime.has_pending_jump()
        {
            return parser.screen().contents();
        }
    }
    panic!("reanchor did not settle: {}", parser.screen().contents());
}

fn key(runtime: &mut FrameRuntime, code: KeyCode) {
    assert!(runtime.reading_key(code, KeyModifiers::NONE, true).unwrap());
}

fn query(runtime: &mut FrameRuntime, parser: &mut vt100::Parser, text: &str) {
    key(runtime, KeyCode::Char('/'));
    for ch in text.chars() {
        key(runtime, KeyCode::Char(ch));
    }
    key(runtime, KeyCode::Enter);
    draw(runtime, parser);
    assert!(runtime.reading().current_match().is_some());
}

// Preserve the accepted026 counterexample's complete reading interaction,
// including the entity/emphasis selection, both copies, and inline math focus.
fn original_flow(cancel: bool) {
    let source = format!(
        "{}\n\na **b&amp;c** d\n\n$x+y$\n\n$$\nBLOCK_BODY\n$$\n\nREADY_END\n",
        "abcdefghij".repeat(8192)
    );
    let mut runtime = FrameRuntime::from_finished_source(
        &source,
        FrameGeometry {
            rows: 24,
            cols: 80,
            content_width: 80,
        },
        LinkMode::Default,
    )
    .unwrap();
    let mut parser = vt100::Parser::new(24.try_into().unwrap(), 80.try_into().unwrap(), 0);
    draw(&mut runtime, &mut parser);
    runtime.seek_end(true).unwrap();
    assert!(draw(&mut runtime, &mut parser).contains("READY_END"));
    for (text, start, ends) in [
        ("b&c", 81922, vec![81924, 81927, 81932, 81933, 81936, 81937]),
        ("x+y", 81940, vec![81942, 81943]),
    ] {
        query(&mut runtime, &mut parser, text);
        key(&mut runtime, KeyCode::Char('v'));
        key(&mut runtime, KeyCode::Char(' '));
        draw(&mut runtime, &mut parser);
        assert_eq!(
            runtime.reading().selection.range().unwrap(),
            mdtui3::Span::new(start, start + 1)
        );
        for end in ends {
            key(&mut runtime, KeyCode::Right);
            draw(&mut runtime, &mut parser);
            assert_eq!(
                runtime.reading().selection.range().unwrap(),
                mdtui3::Span::new(start, end)
            );
        }
        for copy in ['y', 'Y'] {
            key(&mut runtime, KeyCode::Char(copy));
            draw(&mut runtime, &mut parser);
            assert!(
                runtime
                    .reading()
                    .message()
                    .contains("clipboard request sent")
            );
        }
        key(&mut runtime, KeyCode::Esc);
        draw(&mut runtime, &mut parser);
    }
    let original_source = runtime.top_source();
    eprintln!(
        "before cancel={cancel}: source={original_source:?}, top={:?}",
        runtime.viewport().top_position()
    );
    key(&mut runtime, KeyCode::Char('z'));
    draw(&mut runtime, &mut parser);
    for code in [KeyCode::Right, KeyCode::Down, KeyCode::Down] {
        assert!(runtime.fold_menu_key(code));
        draw(&mut runtime, &mut parser);
    }
    let version = runtime.viewport().folds().version();
    assert!(runtime.fold_menu_key(KeyCode::Enter));
    if cancel {
        let limit = WorkLimit {
            queries: 8,
            objects: 8,
            ..FRAME_WORK
        };
        for _ in 0..10000 {
            tick(&mut runtime, &mut parser, limit);
            if runtime.viewport().folds().version() != version {
                break;
            }
        }
        assert_ne!(runtime.viewport().folds().version(), version);
        assert!(
            runtime.fold_pending(),
            "must cancel an actually pending batch"
        );
    } else {
        draw(&mut runtime, &mut parser);
    }
    let applied_version = runtime.viewport().folds().version();
    assert!(runtime.fold_menu_key(KeyCode::Esc));
    assert_eq!(runtime.viewport().folds().version(), applied_version);
    let closed = draw(&mut runtime, &mut parser);
    eprintln!(
        "after: source={:?}, top={:?}\n{closed}",
        runtime.top_source(),
        runtime.viewport().top_position()
    );
    assert!(
        closed.contains("[+ math") && closed.contains("READY_END"),
        "cancel={cancel}\n{closed}"
    );
    assert_eq!(runtime.top_source(), original_source);
    // Reopen through the real document menu without any End recovery.
    key(&mut runtime, KeyCode::Char('z'));
    draw(&mut runtime, &mut parser);
    for code in [KeyCode::Right, KeyCode::Down, KeyCode::Enter] {
        assert!(runtime.fold_menu_key(code));
        draw(&mut runtime, &mut parser);
    }
    assert!(runtime.fold_menu_key(KeyCode::Esc));
    let expanded = draw(&mut runtime, &mut parser);
    assert!(
        expanded.contains("BLOCK") && expanded.contains("READY_END"),
        "{expanded}"
    );
    runtime.scroll_down(1).unwrap();
    let down = draw(&mut runtime, &mut parser);
    assert!(
        down.contains("READY_END"),
        "down left the source tail:\n{down}"
    );
    runtime.scroll_up(1).unwrap();
    draw(&mut runtime, &mut parser);
    // The CLI performs EOF calibration through StreamEngine after geometry
    // changes; keep that lifecycle in the geometry regression as well.
    let mut engine = mdtui_tui::stream::StreamEngine::new(runtime);
    engine.finish_input(std::time::Duration::ZERO).unwrap();
    let mut resized = |engine: &mut mdtui_tui::stream::StreamEngine, rows: u16, cols: u16| {
        engine.resize(rows, cols).unwrap();
        parser.set_size(rows.try_into().unwrap(), cols.try_into().unwrap());
        for _ in 0..100_000 {
            let mut budget = Budget::new(FRAME_WORK);
            engine.advance_calibration(&mut budget).unwrap();
            let mut bytes = Vec::new();
            let committed = engine
                .render_due(&mut bytes, &mut budget, std::time::Duration::ZERO)
                .unwrap();
            parser.process(&bytes);
            if committed.is_some_and(|c| c.committed) && !engine.calibration_pending() {
                return parser.screen().contents();
            }
        }
        panic!("resized stream did not settle");
    };
    resized(&mut engine, 3, 20);
    let restored = resized(&mut engine, 24, 80);
    assert!(
        restored.contains("BLOCK") && restored.contains("READY_END"),
        "{restored}"
    );
}

#[test]
fn document_batch_reanchors_after_complete() {
    original_flow(false);
}

#[test]
fn document_batch_reanchors_after_partial_cancel() {
    original_flow(true);
}

#[test]
fn bounded_projection_at_long_prefix_source_and_table_targets_survives_resize_and_follow() {
    use mdtui_tui::render::{
        refine::{RefineConfig, RefineProgress, SourceRefiner},
        viewport::{ViewportConfig, ViewportFrameStatus, ViewportState},
    };
    use mdtui3::{Completion, Document, SourceFoldRegistration, Span, TableId};

    for (repeats, unicode, table) in [
        (256, false, false),
        (8192, false, false),
        (32768, false, false),
        (4096, true, false),
        (8192, false, true),
    ] {
        let prefix = if unicode {
            "中e\u{301}👩‍🚀 ab "
        } else {
            "abcdefghij"
        }
        .repeat(repeats);
        let tail = if table {
            "\n\n| H | V |\n| --- | --- |\n| KEEP | YES |\n| SECRET | HIDE |\n\nNEIGHBOUR\n"
        } else {
            "\n\nSECRET **entity&amp;强调** e\u{301}\n\nNEIGHBOUR\n"
        };
        let mut doc = Document::new();
        doc.enable_display_shape_effects().unwrap();
        doc.append(&format!("{prefix}{tail}")).unwrap();
        doc.finish();
        let source = prefix.len() + tail.find("SECRET").unwrap();
        let mut view = ViewportState::new(
            &doc,
            ViewportConfig {
                max_tiles: 8,
                rows: 12,
                row_slack: 4,
                ..ViewportConfig::default()
            },
        )
        .unwrap();
        let links = view.shared_links();
        assert!(
            links
                .prepare_shape(&doc, &mut Budget::new(WorkLimit::unlimited()))
                .unwrap()
        );
        let mut refiner = SourceRefiner::new(
            &doc,
            view.heights(),
            view.checkpoints(),
            source,
            RefineConfig {
                max_tiles: 8,
                row_slack: 4,
                ..RefineConfig::default()
            },
        )
        .unwrap();
        refiner.set_links(links.clone());
        let target = loop {
            match refiner
                .advance_owned(&doc, view.layout_owner_mut(), &mut Budget::new(FRAME_WORK))
                .unwrap()
            {
                RefineProgress::Ready(target) => break target,
                RefineProgress::PendingScan | RefineProgress::BudgetExhausted => {}
                RefineProgress::NotFound => panic!("missing target"),
            }
        };
        view.set_top_from_refinement(&doc, target.anchor, target.line_start, target.source, false)
            .unwrap();
        let id = if table {
            view.folds_mut()
                .insert_table_rows(&doc, TableId(0), 2..3)
                .unwrap()
        } else {
            let mut task = SourceFoldRegistration::overlapping(
                &doc,
                view.folds(),
                Span::new(source, source + "SECRET".len()),
            )
            .unwrap();
            let progress = task
                .advance(
                    &doc,
                    view.folds_mut(),
                    &mut Budget::new(WorkLimit::unlimited()),
                )
                .unwrap();
            assert_eq!(progress.completion, Completion::Covered);
            progress.id.unwrap()
        };
        view.folds_mut().set_collapsed(&doc, id, true).unwrap();
        view.fold_changed(&doc);
        let mut work_bytes = 0;
        let mut work_tiles = 0;
        let mut frames = 0;
        let mut ready = |view: &mut ViewportState| {
            for _ in 0..100_000 {
                // A zero slice cannot lose the pending source anchor.
                let mut zero = Budget::new(WorkLimit::default());
                view.frame(&doc, &mut zero).unwrap();
                assert_eq!(zero.used(), WorkLimit::default());
                let limit = WorkLimit {
                    queries: 32,
                    objects: 32,
                    tiles: 1,
                    bytes: 256,
                    recovery: 1,
                    dependencies: 32,
                };
                let mut budget = Budget::new(limit);
                let frame = view.frame(&doc, &mut budget).unwrap();
                frames += 1;
                work_bytes += budget.used().bytes;
                work_tiles += budget.used().tiles;
                assert!(
                    budget.used().bytes <= 256
                        && budget.used().tiles <= 1
                        && budget.used().queries <= 32
                );
                if matches!(
                    frame.status,
                    ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument
                ) {
                    assert!(frame.stats.planned_tiles <= 8, "{:?}", frame.stats);
                    return frame
                        .lines
                        .iter()
                        .map(|line| {
                            line.cells
                                .iter()
                                .map(|c| c.text.collect_text())
                                .collect::<String>()
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                }
            }
            panic!("bounded reanchor did not settle");
        };
        let folded = ready(&mut view);
        let ph = if table { "[+ rows" } else { "[+ source" };
        assert!(
            folded.contains(ph) && folded.contains("NEIGHBOUR"),
            "{folded}"
        );
        assert!(!folded.contains("SECRET"), "{folded}");
        view.set_content_width(&doc, 60).unwrap();
        assert!(ready(&mut view).contains(ph));
        view.folds_mut().set_collapsed(&doc, id, false).unwrap();
        view.fold_changed(&doc);
        assert!(ready(&mut view).contains("SECRET"));
        // FOLLOW is still explicit and can be paused; folding never sends End.
        view.seek_end(&doc, true).unwrap();
        assert!(ready(&mut view).contains("NEIGHBOUR"));
        assert!(view.following());
        assert!(view.pause_following());
        eprintln!(
            "036 bounded prefix_bytes={} unicode={unicode} table={table} frames={frames} charged_bytes={work_bytes} charged_tiles={work_tiles} max_tiles=8",
            prefix.len()
        );
    }
}
