use crossterm::event::{KeyCode as K, KeyModifiers as M};
use mdtui_tui::{
    app::AppState,
    cli::LinkMode,
    history::HistoryPosition,
    render::{
        frame::{FrameGeometry, FrameRuntime, HistoryRestoreStatus},
        refine::{RefineConfig, RefineProgress, RefineStatus, SourceRefiner},
        viewport::{ViewportConfig, ViewportFrame, ViewportFrameStatus, ViewportState},
    },
    stream::{FRAME_WORK, StreamEngine},
};
use mdtui3::{Anchor, Budget, Document, FoldId, HeadingId, TILE_BYTES, TileId, WorkLimit};

const ORIGINAL: &str =
    "# Stream\n- a\n- b\n\n- c\n\n  d\n\n> q\n>\n> r\n\nFINAL\n\n# Tail\nREADY\n";

fn micro() -> WorkLimit {
    WorkLimit {
        queries: 4,
        objects: 4,
        bytes: 64,
        tiles: 1,
        recovery: 1,
        dependencies: 4,
    }
}
fn document(source: &str) -> Document {
    let mut doc = Document::new();
    doc.enable_display_shape_effects().unwrap();
    doc.append(source).unwrap();
    doc.finish();
    doc
}
fn viewport(doc: &Document) -> ViewportState {
    let mut view = ViewportState::new(
        doc,
        ViewportConfig {
            width: 40,
            rows: 10,
            max_tiles: 8,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        view.shared_links()
            .prepare_shape(doc, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap()
    );
    ready(&mut view, doc);
    view
}
fn ready(view: &mut ViewportState, doc: &Document) -> ViewportFrame {
    for _ in 0..100_000 {
        let mut zero = Budget::new(WorkLimit::default());
        view.frame(doc, &mut zero).unwrap();
        assert_eq!(zero.used(), WorkLimit::default());
        let mut budget = Budget::new(micro());
        let frame = view.frame(doc, &mut budget).unwrap();
        let used = budget.used();
        assert!(
            used.queries <= 4
                && used.objects <= 4
                && used.bytes <= 64
                && used.tiles <= 1
                && used.recovery <= 1
                && used.dependencies <= 4
        );
        if matches!(
            frame.status,
            ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument
        ) {
            return frame;
        }
    }
    panic!("viewport did not settle");
}
fn grid(frame: &ViewportFrame) -> Vec<String> {
    frame
        .lines
        .iter()
        .map(|l| {
            l.cells
                .iter()
                .map(|c| c.text.collect_text())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect()
}
fn jump(view: &mut ViewportState, doc: &Document, source: usize) {
    let mut refiner = SourceRefiner::new(
        doc,
        view.heights(),
        view.checkpoints(),
        source,
        RefineConfig {
            width: view.config().width,
            max_tiles: 8,
            ..Default::default()
        },
    )
    .unwrap();
    refiner.set_links(view.shared_links());
    for _ in 0..100_000 {
        match refiner
            .advance_owned(doc, view.layout_owner_mut(), &mut Budget::new(micro()))
            .unwrap()
        {
            RefineProgress::Ready(t) => {
                view.set_top_from_refinement(
                    doc,
                    t.anchor,
                    t.line_start,
                    t.source,
                    t.status != RefineStatus::Approximate,
                )
                .unwrap();
                return;
            }
            RefineProgress::NotFound => panic!("missing source {source}"),
            _ => {}
        }
    }
    panic!("refiner did not settle");
}
fn fold(view: &mut ViewportState, doc: &Document, id: usize, collapsed: bool) -> ViewportFrame {
    view.folds_mut()
        .set_collapsed(doc, FoldId::Heading(HeadingId(id)), collapsed)
        .unwrap();
    view.fold_changed(doc);
    ready(view, doc)
}

#[test]
fn source_jump_after_resize_or_previous_batch_starts_the_next_fold_at_its_target() {
    for cross_tile in [false, true] {
        for resize in [false, true] {
            let padding = if cross_tile {
                "padding\n\n".repeat(TILE_BYTES / 9 + 8)
            } else {
                String::new()
            };
            let text = format!("# Start\n{padding}\n# Tail\nREADY\n");
            let source = text.find("# Tail").unwrap();
            let doc = document(&text);
            assert_eq!(doc.tile_at(source) != Some(TileId(0)), cross_tile);
            let mut view = viewport(&doc);
            if resize {
                view.set_content_width(&doc, 20).unwrap();
                ready(&mut view, &doc);
            } else {
                fold(&mut view, &doc, 0, true);
                fold(&mut view, &doc, 0, false);
            }
            jump(&mut view, &doc, source);
            assert_eq!(grid(&ready(&mut view, &doc)), ["Tail", "", "READY"]);
            assert_eq!(view.top_source(&doc), Some(source));
            let frame = fold(&mut view, &doc, 1, true);
            assert_eq!(
                grid(&frame)[0],
                "Tail",
                "cross={cross_tile} resize={resize}: {:?}",
                grid(&frame)
            );
            assert!(grid(&frame)[1].starts_with("[+ heading 1"));
            assert_eq!(view.top_source(&doc), Some(source));
            assert_eq!(frame.lines[0].source.unwrap().source.start, source);
        }
    }
}

#[test]
fn home_supersedes_a_pending_same_tile_resize_refiner() {
    let text = "# Start\nFIRST\n\n# Tail\nLAST\n";
    let doc = document(text);
    let mut view = viewport(&doc);
    jump(&mut view, &doc, text.find("# Tail").unwrap());
    ready(&mut view, &doc);
    view.set_content_width(&doc, 20).unwrap();
    view.frame(&doc, &mut Budget::new(micro())).unwrap();
    view.home(&doc).unwrap();
    let frame = ready(&mut view, &doc);
    assert_eq!(grid(&frame)[0], "Start", "{:?}", grid(&frame));
    assert_eq!(view.top_source(&doc), Some(0));
    assert_eq!(grid(&fold(&mut view, &doc, 1, true))[0], "Start");
}

#[test]
fn physical_row_navigation_starts_a_new_fold_anchor() {
    for action in ["down", "up", "page_down", "page_up", "home", "end"] {
        let text = format!(
            "# Start\n{}\n# Tail\nREADY\n",
            (0..40)
                .map(|n| format!("line {n:02}  \n"))
                .collect::<String>()
        );
        let doc = document(&text);
        let mut view = viewport(&doc);
        jump(&mut view, &doc, text.find("line 20").unwrap());
        ready(&mut view, &doc);
        view.set_content_width(&doc, 20).unwrap();
        ready(&mut view, &doc);
        match action {
            "down" => view.down(&doc),
            "up" => view.up(&doc),
            "page_down" => view.page_down(&doc),
            "page_up" => view.page_up(&doc),
            "home" => view.home(&doc),
            "end" => view.end(&doc),
            _ => unreachable!(),
        }
        .unwrap();
        let before = ready(&mut view, &doc);
        assert!(
            !grid(&before)[0].is_empty(),
            "{action}: {:?}",
            grid(&before)
        );
        let source = view.top_source(&doc);
        let after = fold(&mut view, &doc, 1, true);
        assert_eq!(
            view.top_source(&doc),
            source,
            "{action}: {:?} -> {:?}",
            grid(&before),
            grid(&after)
        );
        assert_eq!(grid(&after)[0], grid(&before)[0], "{action}");
    }
}

#[test]
fn same_batch_reprojection_and_resize_retain_the_original_exact_source() {
    let text = format!(
        "# Start\nFIRST\n\n# Body\n{}SECRET\n\nNEIGHBOUR\n",
        "padding\n\n".repeat(TILE_BYTES / 9 + 8)
    );
    let doc = document(&text);
    let source = text.find("SECRET").unwrap();
    let mut view = viewport(&doc);
    jump(&mut view, &doc, source);
    assert_eq!(grid(&ready(&mut view, &doc))[0], "SECRET");
    assert!(grid(&fold(&mut view, &doc, 1, true))[0].starts_with("[+ heading 1"));
    view.set_content_width(&doc, 20).unwrap();
    ready(&mut view, &doc);
    let frame = fold(&mut view, &doc, 1, false);
    assert_eq!(grid(&frame)[0], "SECRET", "{:?}", grid(&frame));
    assert_eq!(view.top_source(&doc), Some(source));
}

#[test]
fn rejected_navigation_cannot_discard_a_foreign_or_invalid_batch_anchor() {
    let text = format!(
        "# Start\nFIRST\n\n# Body\n{}SECRET😃\n",
        "padding\n\n".repeat(TILE_BYTES / 9 + 8)
    );
    for invalid in [
        "foreign_down",
        "foreign_up",
        "foreign_refined",
        "foreign_fold",
        "tile",
        "offset",
        "source",
        "line_start",
        "utf8",
        "width",
        "same_width",
        "zero_rows",
    ] {
        let doc = document(&text);
        let foreign = document(&text);
        let source = text.find("SECRET").unwrap();
        let mut view = viewport(&doc);
        jump(&mut view, &doc, source);
        ready(&mut view, &doc);
        fold(&mut view, &doc, 1, true);
        let anchor = view.top_anchor().unwrap();
        let top = view.top_source(&doc).unwrap();
        match invalid {
            "foreign_down" => {
                assert!(view.down(&foreign).is_err());
            }
            "foreign_up" => {
                assert!(view.up(&foreign).is_err());
            }
            "foreign_refined" => {
                assert!(
                    view.set_top_from_refinement(&foreign, anchor, top, top, true)
                        .is_err()
                );
            }
            "foreign_fold" => view.fold_changed(&foreign),
            "tile" => {
                assert!(
                    view.set_top_from_refinement(
                        &doc,
                        Anchor {
                            tile: TileId(doc.tile_count()),
                            offset: 0
                        },
                        top,
                        top,
                        true
                    )
                    .is_err()
                );
            }
            "offset" => {
                assert!(
                    view.set_top_from_refinement(
                        &doc,
                        Anchor {
                            offset: u64::MAX,
                            ..anchor
                        },
                        top,
                        top,
                        true
                    )
                    .is_err()
                );
            }
            "line_start" => {
                assert!(
                    view.set_top_from_refinement(&doc, anchor, usize::MAX, top, true)
                        .is_err()
                );
            }
            "utf8" => {
                assert!(
                    view.set_top_from_refinement(&doc, anchor, top, source + 7, true)
                        .is_err()
                );
            }
            "source" => {
                assert!(
                    view.set_top_from_refinement(&doc, anchor, usize::MAX, usize::MAX, true)
                        .is_err()
                );
            }
            "width" => {
                assert!(view.set_content_width(&doc, 0).is_err());
            }
            "same_width" => {
                assert!(!view.set_content_width(&doc, 40).unwrap().changed);
            }
            "zero_rows" => {
                assert!(!view.scroll_down(&doc, 0).unwrap());
            }
            _ => unreachable!(),
        }
        assert_eq!(view.top_source(&doc), Some(top), "{invalid}");
        let frame = fold(&mut view, &doc, 1, false);
        assert_eq!(grid(&frame)[0], "SECRET😃", "{invalid}: {:?}", grid(&frame));
        assert_eq!(view.top_source(&doc), Some(source), "{invalid}");
    }
}

#[test]
fn accepted_noop_source_navigation_replaces_the_old_batch_anchor() {
    let text = format!(
        "# Body\n{}SECRET\n",
        "padding\n\n".repeat(TILE_BYTES / 9 + 8)
    );
    let doc = document(&text);
    let mut view = viewport(&doc);
    jump(&mut view, &doc, text.find("SECRET").unwrap());
    ready(&mut view, &doc);
    fold(&mut view, &doc, 0, true);
    let anchor = view.top_anchor().unwrap();
    let source = view.top_source(&doc).unwrap();
    assert_ne!(source, text.find("SECRET").unwrap());
    assert!(
        !view
            .set_top_from_refinement(&doc, anchor, source, source, true)
            .unwrap()
    );
    let frame = fold(&mut view, &doc, 0, false);
    assert_eq!(grid(&frame)[0], "padding");
    assert_eq!(view.top_source(&doc), Some(source));
}

fn runtime(text: &str) -> FrameRuntime {
    FrameRuntime::from_finished_source(
        text,
        FrameGeometry {
            rows: 24,
            cols: 60,
            content_width: 60,
        },
        LinkMode::Default,
    )
    .unwrap()
}
fn draw(runtime: &mut FrameRuntime, parser: &mut vt100::Parser) -> String {
    for _ in 0..100_000 {
        let mut bytes = Vec::new();
        let mut budget = Budget::new(FRAME_WORK);
        runtime.advance_reading(&mut bytes, &mut budget);
        let committed = runtime.frame_to(&mut bytes, &mut budget).unwrap().committed;
        parser.process(&bytes);
        if committed
            && !runtime.has_pending_jump()
            && !runtime.fold_pending()
            && !runtime.reading().pending()
            && !runtime.sidebar_pending()
        {
            return parser.screen().contents();
        }
    }
    panic!("runtime did not settle: {}", parser.screen().contents());
}
fn collapse_tail(runtime: &mut FrameRuntime, parser: &mut vt100::Parser, source: usize) {
    assert_eq!(runtime.top_source(), Some(source));
    let (doc, view) = runtime.document_and_viewport_mut();
    view.folds_mut()
        .set_collapsed(doc, FoldId::Heading(HeadingId(1)), true)
        .unwrap();
    view.fold_changed(doc);
    let frame = draw(runtime, parser);
    assert!(frame.lines().next().unwrap().contains("Tail"), "{frame}");
    assert!(frame.contains("[+ heading 1"), "{frame}");
    assert_eq!(runtime.top_source(), Some(source));
}

#[test]
fn original_directory_keys_after_end_resize_and_home_keep_tail_when_folded() {
    let mut e = StreamEngine::new(runtime(ORIGINAL));
    e.finish_input(std::time::Duration::ZERO).unwrap();
    let mut app = AppState::default();
    let mut parser = vt100::Parser::new(24.try_into().unwrap(), 60.try_into().unwrap(), 0);
    draw(e.runtime_mut(), &mut parser);
    app.handle_key(&mut e, K::End, M::NONE).unwrap();
    draw(e.runtime_mut(), &mut parser);
    e.resize(10, 40).unwrap();
    parser.set_size(10.try_into().unwrap(), 40.try_into().unwrap());
    for _ in 0..1000 {
        e.advance_calibration(&mut Budget::new(FRAME_WORK)).unwrap();
        let mut bytes = Vec::new();
        e.render_due(
            &mut bytes,
            &mut Budget::new(FRAME_WORK),
            std::time::Duration::ZERO,
        )
        .unwrap();
        parser.process(&bytes);
        if !e.calibration_pending() {
            break;
        }
    }
    assert!(!e.calibration_pending());
    draw(e.runtime_mut(), &mut parser);
    for k in [K::Home, K::Char('s'), K::Tab, K::Down, K::Enter] {
        app.handle_key(&mut e, k, M::NONE).unwrap();
        draw(e.runtime_mut(), &mut parser);
    }
    let before = draw(e.runtime_mut(), &mut parser);
    assert!(
        before.lines().next().unwrap().trim_end().ends_with("Tail"),
        "{before}"
    );
    assert!(before.contains("READY"), "{before}");
    assert_eq!(
        e.runtime().top_source(),
        Some(ORIGINAL.find("# Tail").unwrap())
    );
    for k in [K::Char('z'), K::Enter, K::Esc] {
        app.handle_key(&mut e, k, M::NONE).unwrap();
        draw(e.runtime_mut(), &mut parser);
    }
    let after = draw(e.runtime_mut(), &mut parser);
    assert!(
        after.lines().next().unwrap().trim_end().ends_with("Tail"),
        "{after}"
    );
    assert!(after.contains("[+ heading 1"), "{after}");
    assert_eq!(
        e.runtime().top_source(),
        Some(ORIGINAL.find("# Tail").unwrap())
    );
}

#[test]
fn history_fragment_and_search_reveal_replace_resize_anchor() {
    let text = "[jump](#tail)\n\n# Start\nFIRST\n\n## Tail\nREADY\n";
    let source = text.find("## Tail").unwrap();
    for entry in ["history", "fragment", "search"] {
        let mut r = runtime(text);
        let mut parser = vt100::Parser::new(24.try_into().unwrap(), 60.try_into().unwrap(), 0);
        draw(&mut r, &mut parser);
        r.resize(10, 40).unwrap();
        parser.set_size(10.try_into().unwrap(), 40.try_into().unwrap());
        draw(&mut r, &mut parser);
        // Reveal must use the existing FoldView; the source jump then becomes
        // the starting point of the next explicit fold.
        let (doc, view) = r.document_and_viewport_mut();
        view.folds_mut()
            .set_collapsed(doc, FoldId::Heading(HeadingId(0)), true)
            .unwrap();
        view.fold_changed(doc);
        draw(&mut r, &mut parser);
        match entry {
            "history" => {
                assert!(matches!(
                    r.restore_navigation_position(
                        HistoryPosition {
                            source,
                            row_offset: 0
                        },
                        &mut Budget::new(FRAME_WORK)
                    )
                    .unwrap(),
                    HistoryRestoreStatus::Ready { .. } | HistoryRestoreStatus::BudgetExhausted
                ));
            }
            "fragment" => {
                while !r.anchor_index_complete() {
                    r.advance_anchor_index(&mut Budget::new(FRAME_WORK))
                        .unwrap();
                }
                assert!(r.enter_link_focus(true).is_some());
                r.activate_focused_link(&mut Budget::new(FRAME_WORK))
                    .unwrap();
            }
            "search" => {
                for k in [
                    K::Char('/'),
                    K::Char('T'),
                    K::Char('a'),
                    K::Char('i'),
                    K::Char('l'),
                    K::Enter,
                ] {
                    r.reading_key(k, M::NONE, true).unwrap();
                }
            }
            _ => unreachable!(),
        }
        let frame = draw(&mut r, &mut parser);
        assert!(frame.contains("READY"), "{entry}: {frame}");
        assert!(!r.viewport().folds().has_collapsed(), "{entry}");
        collapse_tail(&mut r, &mut parser, source);
    }
}
