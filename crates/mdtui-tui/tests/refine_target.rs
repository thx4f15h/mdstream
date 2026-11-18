use mdtui_tui::render::{
    checkpoint::RenderCheckpointStore,
    first_frame::{FirstFrame, FirstFrameConfig, FirstFrameStatus},
    refine::{RefineConfig, RefineProgress, RefineStatus, RefinedTarget, SourceRefiner},
    viewport::{ViewportConfig, ViewportFrame, ViewportFrameStatus, ViewportState},
};
use mdtui3::{Budget, Document, FoldId, HeadingId, TileId, WorkLimit};

fn unlimited() -> Budget {
    Budget::new(WorkLimit::unlimited())
}
fn micro() -> WorkLimit {
    WorkLimit {
        queries: 32,
        objects: 32,
        bytes: 256,
        tiles: 1,
        recovery: 1,
        dependencies: 32,
    }
}
fn paid(used: WorkLimit, limit: WorkLimit) {
    assert!(used.queries <= limit.queries);
    assert!(used.objects <= limit.objects);
    assert!(used.bytes <= limit.bytes);
    assert!(used.tiles <= limit.tiles);
    assert!(used.recovery <= limit.recovery);
    assert!(used.dependencies <= limit.dependencies);
}
fn document(source: &str) -> Document {
    let mut document = Document::new();
    document.enable_display_shape_effects().unwrap();
    document.append(source).unwrap();
    document.finish();
    document
}
fn original() -> String {
    format!(
        "# Start\n{}\n# Tail\nREADY\n",
        "abcdefghi\n\n".repeat(32768 / 11)
    )
}
fn ready(view: &mut ViewportState, document: &Document) -> ViewportFrame {
    for _ in 0..100_000 {
        let mut zero = Budget::new(WorkLimit::default());
        view.frame(document, &mut zero).unwrap();
        assert_eq!(zero.used(), WorkLimit::default());
        let mut budget = Budget::new(micro());
        let frame = view.frame(document, &mut budget).unwrap();
        paid(budget.used(), micro());
        assert!(frame.stats.planned_tiles <= view.config().max_tiles);
        if matches!(
            frame.status,
            ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument
        ) {
            return frame;
        }
    }
    panic!("viewport did not settle");
}
fn viewport(document: &Document, config: RefineConfig) -> ViewportState {
    let mut view = ViewportState::new(
        document,
        ViewportConfig {
            width: 40,
            rows: 10,
            max_tiles: config.max_tiles,
            row_slack: config.row_slack,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        view.shared_links()
            .prepare_shape(document, &mut unlimited())
            .unwrap()
    );
    ready(&mut view, document);
    view.set_content_width(document, config.width).unwrap();
    ready(&mut view, document);
    view
}
fn refine(
    document: &Document,
    owner: &mut RenderCheckpointStore,
    source: usize,
    config: RefineConfig,
) -> RefinedTarget {
    let mut refiner = SourceRefiner::new(document, owner.heights(), owner, source, config).unwrap();
    let mut paused = false;
    for _ in 0..100_000 {
        let mut zero = Budget::new(WorkLimit::default());
        assert!(matches!(
            refiner.advance_owned(document, owner, &mut zero).unwrap(),
            RefineProgress::PendingScan | RefineProgress::BudgetExhausted
        ));
        assert_eq!(zero.used(), WorkLimit::default());
        let mut budget = Budget::new(micro());
        let progress = refiner.advance_owned(document, owner, &mut budget).unwrap();
        paid(budget.used(), micro());
        match progress {
            RefineProgress::Ready(target) => {
                assert!(paused);
                assert_eq!(target.source, source);
                assert_eq!(owner.heights().row_of(target.anchor), Some(target.row));
                assert!(target.plan_end.0 - target.plan_start.0 <= config.max_tiles);
                assert!(target.rendered_tiles <= config.max_tiles);
                return target;
            }
            RefineProgress::NotFound => panic!("visible source {source} was lost"),
            _ => paused = true,
        }
    }
    panic!("source refinement did not settle");
}
fn jump(view: &mut ViewportState, document: &Document, target: RefinedTarget) -> ViewportFrame {
    view.set_top_from_refinement(
        document,
        target.anchor,
        target.line_start,
        target.source,
        target.status != RefineStatus::Approximate,
    )
    .unwrap();
    ready(view, document)
}
fn first(frame: &ViewportFrame) -> String {
    frame.lines[0]
        .cells
        .iter()
        .map(|cell| cell.text.collect_text())
        .collect::<String>()
        .trim_end()
        .to_owned()
}

#[test]
fn original_tail_marker_content_and_resize_fit_eight_one_and_default_tiles() {
    let document = document(&original());
    // Handwritten source oracle: # is the final byte of tile127; Tail starts
    // two bytes later in tile128. No production locator computes this oracle.
    assert_eq!(document.source().len(), 32780);
    assert_eq!(document.tile_count(), 129);
    assert_eq!(&document.source()[32767..32774], "# Tail\n");
    for max_tiles in [8, 1, RefineConfig::default().max_tiles] {
        for width in [40, 20] {
            let config = RefineConfig {
                width,
                max_tiles,
                ..Default::default()
            };
            let mut view = viewport(&document, config);
            for source in [32767, 32768, 32769, 32772] {
                let target = refine(&document, view.layout_owner_mut(), source, config);
                assert_eq!(target.line_start, 32767);
                assert!(target.plan_start.0 <= 128 && target.plan_end.0 > 128);
                let frame = jump(&mut view, &document, target);
                assert_eq!(first(&frame), "Tail");
                assert_eq!(view.top_source(&document), Some(32767));
            }
        }
    }
}

#[test]
fn original_measured_rows_have_an_independent_paragraph_count_oracle() {
    let document = document(&original());
    let mut first = FirstFrame::new(
        &document,
        FirstFrameConfig {
            top_tile: TileId(0),
            tile_limit: document.tile_count(),
            width: 20,
            row_limit: 10000,
        },
    )
    .unwrap();
    loop {
        if matches!(
            first.advance(&document, &mut unlimited()).unwrap().status,
            FirstFrameStatus::Covered | FirstFrameStatus::EndOfDocument
        ) {
            break;
        }
    }
    let mut owner = first.into_layout_parts();
    let config = RefineConfig {
        width: 20,
        max_tiles: 8,
        row_slack: 4,
        ..Default::default()
    };
    let target = refine(&document, &mut owner, 32767, config);
    // Start occupies row0. Each of2978 one-line paragraphs follows a blank
    // separator, then Tail follows one more separator: 2*2978+2 =5958.
    assert_eq!(target.row, 5958);
    assert_eq!(target.line_start, 32767);
}

#[test]
fn indexed_heading_content_survives_varied_prefixes_and_fold_projection() {
    let prefixes = [
        "a".repeat(32768),
        "abcdefghi\n\n".repeat(2978),
        format!("a | b\n--- | ---\n{}", "one | two\n".repeat(400)),
        "> quote\n>\n".repeat(400),
        "[hidden]: /target\n\n".repeat(400),
    ];
    for prefix in prefixes {
        let mut source = format!("# Start\n{prefix}\n\n");
        source.push_str(&"\n".repeat((255 + 256 - source.len() % 256) % 256));
        let at = source.len();
        source.push_str("# Tail\nREADY\n");
        assert_eq!(at % 256, 255);
        let document = document(&source);
        let config = RefineConfig {
            width: 20,
            max_tiles: 8,
            row_slack: 4,
            ..Default::default()
        };
        let mut view = viewport(&document, config);
        let target = refine(&document, view.layout_owner_mut(), at, config);
        assert_eq!(first(&jump(&mut view, &document, target)), "Tail");
        // A distant PH owns the hidden prefix while Tail's marker/content
        // straddle their original raw tiles. Reuse this same owner throughout.
        view.folds_mut()
            .set_collapsed(&document, FoldId::Heading(HeadingId(0)), true)
            .unwrap();
        view.fold_changed(&document);
        ready(&mut view, &document);
        let target = refine(&document, view.layout_owner_mut(), at, config);
        assert_eq!(first(&jump(&mut view, &document, target)), "Tail");
        assert_eq!(view.top_source(&document), Some(at));
    }
}

#[test]
fn an_actual_nonrendering_definition_still_returns_not_found() {
    let source = format!(
        "# Start\n{}\n[only]: /invisible\n",
        "abcdefghi\n\n".repeat(2978)
    );
    let at = source.find("[only]").unwrap();
    let document = document(&source);
    let config = RefineConfig {
        width: 20,
        max_tiles: 8,
        row_slack: 4,
        ..Default::default()
    };
    let mut view = viewport(&document, config);
    let before = view.top_anchor();
    let mut refiner =
        SourceRefiner::new(&document, view.heights(), view.checkpoints(), at, config).unwrap();
    loop {
        match refiner
            .advance_owned(
                &document,
                view.layout_owner_mut(),
                &mut Budget::new(micro()),
            )
            .unwrap()
        {
            RefineProgress::NotFound => break,
            RefineProgress::Ready(target) => panic!("definition acquired a row: {target:?}"),
            _ => {}
        }
    }
    assert_eq!(view.top_anchor(), before);
}

#[test]
fn six_byte_paid_replay_resumes_without_losing_the_target_or_prefix_heights() {
    let document = document(&original());
    let mut first = FirstFrame::new(
        &document,
        FirstFrameConfig {
            top_tile: TileId(0),
            tile_limit: document.tile_count(),
            width: 20,
            row_limit: 10000,
        },
    )
    .unwrap();
    loop {
        if matches!(
            first.advance(&document, &mut unlimited()).unwrap().status,
            FirstFrameStatus::Covered | FirstFrameStatus::EndOfDocument
        ) {
            break;
        }
    }
    let links = first.scanner().shared_links();
    let mut owner = first.into_layout_parts();
    let prefix_height = owner.heights().height(TileId(0));
    owner.invalidate(&document, 128..129).unwrap();
    let mut refiner = SourceRefiner::new(
        &document,
        owner.heights(),
        &owner,
        32767,
        RefineConfig {
            width: 20,
            max_tiles: 8,
            row_slack: 4,
            ..Default::default()
        },
    )
    .unwrap();
    refiner.set_links(links);
    let limit = WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 6,
        tiles: 1,
        recovery: 1,
        dependencies: 1,
    };
    let mut recovery = 0;
    for _ in 0..100_000 {
        let mut zero = Budget::new(WorkLimit::default());
        assert!(matches!(
            refiner
                .advance_owned(&document, &mut owner, &mut zero)
                .unwrap(),
            RefineProgress::PendingScan | RefineProgress::BudgetExhausted
        ));
        assert_eq!(zero.used(), WorkLimit::default());
        let mut budget = Budget::new(limit);
        let result = refiner
            .advance_owned(&document, &mut owner, &mut budget)
            .unwrap();
        paid(budget.used(), limit);
        recovery += budget.used().recovery;
        match result {
            RefineProgress::Ready(target) => {
                assert!(matches!(target.status, RefineStatus::ReplayFrom { .. }));
                assert!(recovery > 0, "ReplayFrom must have paid real recovery work");
                assert_eq!(target.row, 5958);
                assert_eq!(target.line_start, 32767);
                assert_eq!(owner.heights().height(TileId(0)), prefix_height);
                return;
            }
            RefineProgress::NotFound => panic!("paid replay lost Tail"),
            _ => {}
        }
    }
    panic!("six-byte replay did not resume");
}

#[test]
fn default_runtime_directory_fragment_search_and_history_reach_the_same_grid() {
    use crossterm::event::{KeyCode, KeyModifiers};
    use mdtui_tui::{
        cli::LinkMode,
        history::HistoryPosition,
        render::frame::{FrameGeometry, FrameRuntime, HistoryRestoreStatus},
        stream::FRAME_WORK,
    };
    fn draw(runtime: &mut FrameRuntime, parser: &mut vt100::Parser) -> String {
        for _ in 0..100_000 {
            let mut bytes = Vec::new();
            let mut budget = Budget::new(FRAME_WORK);
            runtime.advance_reading(&mut bytes, &mut budget);
            let committed = runtime.frame_to(&mut bytes, &mut budget).unwrap().committed;
            paid(budget.used(), FRAME_WORK);
            parser.process(&bytes);
            if committed
                && !runtime.has_pending_jump()
                && !runtime.reading().pending()
                && !runtime.sidebar_pending()
            {
                return parser.screen().contents();
            }
        }
        panic!(
            "runtime navigation did not settle: {}",
            parser.screen().contents()
        );
    }
    let mut source = format!("[jump](#tail)\n\n# Start\n{}", "abcdefghi\n\n".repeat(2978));
    source.push_str(&"\n".repeat((255 + 256 - source.len() % 256) % 256));
    let at = source.len();
    source.push_str("# Tail\nREADY\n");
    assert_eq!(at % 256, 255);
    for entry in ["directory", "fragment", "search", "history"] {
        let mut runtime = FrameRuntime::from_finished_source(
            &source,
            FrameGeometry {
                rows: 10,
                cols: 40,
                content_width: 40,
            },
            LinkMode::Default,
        )
        .unwrap();
        let mut parser = vt100::Parser::new(10.try_into().unwrap(), 40.try_into().unwrap(), 0);
        draw(&mut runtime, &mut parser);
        runtime.set_content_width(20).unwrap();
        draw(&mut runtime, &mut parser);
        match entry {
            "directory" => {
                runtime.set_sidebar_visible(true).unwrap();
                runtime.ensure_sidebar_selection();
                runtime.sidebar_selection_end();
                runtime
                    .activate_sidebar_selection(&mut Budget::new(FRAME_WORK))
                    .unwrap();
            }
            "fragment" => {
                while !runtime.anchor_index_complete() {
                    runtime
                        .advance_anchor_index(&mut Budget::new(FRAME_WORK))
                        .unwrap();
                }
                assert!(runtime.enter_link_focus(true).is_some());
                runtime
                    .activate_focused_link(&mut Budget::new(FRAME_WORK))
                    .unwrap();
            }
            "search" => {
                for key in [
                    KeyCode::Char('/'),
                    KeyCode::Char('T'),
                    KeyCode::Char('a'),
                    KeyCode::Char('i'),
                    KeyCode::Char('l'),
                    KeyCode::Enter,
                ] {
                    runtime.reading_key(key, KeyModifiers::NONE, true).unwrap();
                }
            }
            "history" => {
                assert!(matches!(
                    runtime
                        .restore_navigation_position(
                            HistoryPosition {
                                source: at,
                                row_offset: 0
                            },
                            &mut Budget::new(FRAME_WORK)
                        )
                        .unwrap(),
                    HistoryRestoreStatus::Ready { .. } | HistoryRestoreStatus::BudgetExhausted
                ));
            }
            _ => unreachable!(),
        }
        let grid = draw(&mut runtime, &mut parser);
        assert!(
            grid.lines().next().unwrap().trim_end().ends_with("Tail"),
            "{entry}: {grid}"
        );
        assert!(grid.contains("READY"), "{entry}: {grid}");
        assert_eq!(runtime.top_source(), Some(at), "{entry}");
    }
}
