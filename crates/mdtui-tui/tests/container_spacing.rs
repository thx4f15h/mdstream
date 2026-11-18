use mdtui_tui::render::first_frame::{FirstFrame, FirstFrameConfig, FirstFrameStatus};
use mdtui3::{Budget, Document, TileId, WorkLimit};

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}
fn layout(doc: &Document) -> FirstFrame {
    let mut first = FirstFrame::new(
        doc,
        FirstFrameConfig {
            top_tile: TileId(0),
            tile_limit: doc.tile_count().max(1),
            width: 40,
            row_limit: 1000,
        },
    )
    .unwrap();
    for _ in 0..10000 {
        let step = first
            .advance(doc, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap();
        if matches!(
            step.status,
            FirstFrameStatus::Covered | FirstFrameStatus::EndOfDocument
        ) {
            return first;
        }
    }
    panic!("layout did not finish");
}
fn grid(first: &FirstFrame) -> Vec<String> {
    first
        .lines()
        .iter()
        .map(|line| {
            line.cells
                .iter()
                .map(|c| c.text.collect_text())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect()
}
#[test]
fn handwritten_reading_grids() {
    for (source, expected) in [
        ("a\n\nb\n", vec!["a", "", "b"]),
        ("- a\n- b\n", vec!["• a", "• b"]),
        ("- a\n\n- b\n", vec!["• a", "", "• b"]),
        (
            "- a\n\n  b\n  c\n- d\n",
            vec!["• a", "", "  b c", "", "• d"],
        ),
        ("> a\n>\n> b\n", vec!["│ a", "", "│ b"]),
        ("> a\n> b\n", vec!["│ a b"]),
        (
            "- a\n  - b\n  - c\n- d\n",
            vec!["• a", "  • b", "  • c", "• d"],
        ),
        (
            "- a\n  > b\n  >\n  > c\n- d\n",
            vec!["• a", "", "│ b", "", "│ c", "", "• d"],
        ),
    ] {
        let doc = parse(source);
        let first = layout(&doc);
        assert_eq!(grid(&first), expected, "source {source:?}");
        assert_eq!(first.heights().total_rows(), first.lines().len() as u128);
        for line in first.lines() {
            if line.cells.is_empty() {
                assert!(line.source.is_none());
            }
        }
    }
}

fn micro() -> Budget {
    Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 6,
        tiles: 1,
        recovery: 1,
        dependencies: 1,
    })
}
fn complete(first: &mut FirstFrame, doc: &Document) -> usize {
    for frame in 0..100_000 {
        let mut budget = micro();
        let step = first.advance(doc, &mut budget).unwrap();
        assert!(
            budget.used().queries <= 1 && budget.used().objects <= 1 && budget.used().bytes <= 6
        );
        if matches!(
            step.status,
            FirstFrameStatus::Covered | FirstFrameStatus::EndOfDocument
        ) {
            return frame + 1;
        }
    }
    panic!("micro layout stalled: {:?}", first.scanner().stats());
}
#[test]
fn zero_and_micro_budget_preserve_the_same_independent_grid() {
    let doc = parse("- a\n\n  b\n  c\n- d\n");
    let mut first = FirstFrame::new(
        &doc,
        FirstFrameConfig {
            top_tile: TileId(0),
            tile_limit: 4,
            width: 40,
            row_limit: 100,
        },
    )
    .unwrap();
    let mut zero = Budget::new(WorkLimit::default());
    first.advance(&doc, &mut zero).unwrap();
    assert_eq!(zero.used(), WorkLimit::default());
    assert_eq!(first.rows(), 0);
    let frames = complete(&mut first, &doc);
    assert_eq!(grid(&first), ["• a", "", "  b c", "", "• d"]);
    eprintln!(
        "micro frames={frames} scanner={:?} sink={:?}",
        first.scanner().stats(),
        first.sink_stats()
    );
}
#[test]
fn loose_outer_list_does_not_loosen_its_tight_nested_list() {
    let doc = parse("- a\n\n  - b\n  - c\n- d\n");
    assert_eq!(
        grid(&layout(&doc)),
        ["• a", "", "  • b", "  • c", "", "• d"]
    );
}
#[test]
fn cross_tile_separator_has_one_owner_and_refinement_selects_real_content() {
    use mdtui_tui::render::refine::{RefineConfig, RefineProgress, SourceRefiner};
    let source = format!("- {}\n\n- b\n", "a".repeat(252));
    let doc = parse(&source);
    let first = layout(&doc);
    assert_eq!(doc.block_at(256), Some(mdtui3::BlockId(2)));
    assert_eq!(first.heights().height(TileId(0)), Some(7));
    assert_eq!(first.heights().height(TileId(1)), Some(2));
    assert_eq!(grid(&first)[7..], ["", "• b"]);
    let mut local = FirstFrame::new(
        &doc,
        FirstFrameConfig {
            top_tile: TileId(1),
            tile_limit: 1,
            width: 40,
            row_limit: 100,
        },
    )
    .unwrap();
    complete(&mut local, &doc);
    assert_eq!(grid(&local), ["", "• b"]);
    assert_eq!(local.heights().height(TileId(1)), Some(2));
    let mut owner = first.into_layout_parts();
    let mut refine = SourceRefiner::new(
        &doc,
        owner.heights(),
        &owner,
        258,
        RefineConfig {
            width: 40,
            ..Default::default()
        },
    )
    .unwrap();
    for _ in 0..10000 {
        match refine
            .advance_owned(&doc, &mut owner, &mut micro())
            .unwrap()
        {
            RefineProgress::Ready(target) => {
                assert_eq!(target.source, 258);
                assert_eq!(target.row, 8);
                assert_eq!(owner.heights().row_of(target.anchor), Some(8));
                return;
            }
            RefineProgress::NotFound => panic!("real b must be found"),
            _ => {}
        }
    }
    panic!("refinement stalled");
}

fn view_config(rows: u16) -> mdtui_tui::render::viewport::ViewportConfig {
    mdtui_tui::render::viewport::ViewportConfig {
        width: 40,
        rows,
        max_tiles: 32,
        overscan_rows: 0,
        ..Default::default()
    }
}
fn ready(
    view: &mut mdtui_tui::render::viewport::ViewportState,
    doc: &Document,
) -> mdtui_tui::render::viewport::ViewportFrame {
    use mdtui_tui::render::viewport::ViewportFrameStatus;
    for _ in 0..10000 {
        let frame = view
            .frame(doc, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap();
        if matches!(
            frame.status,
            ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument
        ) {
            return frame;
        }
    }
    panic!("viewport stalled");
}
fn view_grid(frame: &mdtui_tui::render::viewport::ViewportFrame) -> Vec<String> {
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
#[test]
fn late_loose_expires_only_affected_heights_and_old_items_reflow_lazily() {
    use mdtui_tui::render::viewport::ViewportState;
    let mut doc = Document::new();
    doc.enable_display_shape_effects().unwrap();
    doc.append(&format!("{}\n\n- a\n- b\n", "p".repeat(300)))
        .unwrap();
    let first = layout(&doc);
    let mut view = ViewportState::from_first_frame(&doc, first, view_config(20)).unwrap();
    let before = view.heights().height(TileId(0));
    let before_state = view.heights().height_state(TileId(0));
    let update = doc.append("\n- c\n").unwrap();
    view.apply_update(&doc, &update).unwrap();
    assert_eq!(view.heights().height(TileId(0)), before);
    assert_eq!(view.heights().height_state(TileId(0)), before_state);
    assert_eq!(
        view.heights().height_state(TileId(1)),
        Some(mdtui3::HeightState::Expired)
    );
    let frame = ready(&mut view, &doc);
    let text = view_grid(&frame);
    assert_eq!(&text[8..], ["• a", "", "• b", "", "• c"]);
    assert_eq!(view.heights().height(TileId(0)), before);
}
#[test]
fn code_math_and_table_boundaries_use_the_same_spacing_stream() {
    let source = "> a\n> ```\n> c\n> ```\n> $$\n> x\n> $$\n> | h |\n> | - |\n> | v |\n\nout\n";
    let doc = parse(source);
    let first = layout(&doc);
    assert_eq!(
        grid(&first),
        [
            "│ a",
            "",
            "  c",
            "",
            "│ x",
            "",
            "┌───┐",
            "│ h │",
            "╞═══╡",
            "│ v │",
            "└───┘",
            "",
            "out"
        ]
    );
    assert_eq!(first.sink_stats().unwrap().document_queries_in_callbacks, 0);
}
#[test]
fn viewport_scroll_end_resize_and_fold_keep_the_handwritten_sequence() {
    use mdtui_tui::render::viewport::ViewportState;
    use mdtui3::{ContainerKind, FoldId};
    let doc = parse("- a\n\n  b\n- c\n\nout\n\ntail\n");
    let first = layout(&doc);
    let expected = ["• a", "", "  b", "", "• c", "out", "", "tail"];
    assert_eq!(grid(&first), expected);
    let mut view = ViewportState::from_first_frame(&doc, first, view_config(4)).unwrap();
    assert_eq!(view_grid(&ready(&mut view, &doc)), expected[..4]);
    view.scroll_down(&doc, 2).unwrap();
    assert_eq!(view_grid(&ready(&mut view, &doc)), expected[2..6]);
    view.seek_end(&doc, false).unwrap();
    assert_eq!(view_grid(&ready(&mut view, &doc)), expected[4..]);
    view.set_content_width(&doc, 30).unwrap();
    assert_eq!(view_grid(&ready(&mut view, &doc)), expected[4..]);
    view.cancel_end();
    view.home(&doc).unwrap();
    let list = (0..doc.container_count())
        .map(mdtui3::ContainerId)
        .find(|id| matches!(doc.container(*id).unwrap().kind, ContainerKind::List { .. }))
        .unwrap();
    view.folds_mut()
        .set_collapsed(&doc, FoldId::Container(list), true)
        .unwrap();
    view.fold_changed(&doc);
    let folded = view_grid(&ready(&mut view, &doc));
    assert!(folded[0].starts_with("[+ container"), "{folded:?}");
    assert_eq!(&folded[1..], ["out", "", "tail"]);
    view.folds_mut()
        .set_collapsed(&doc, FoldId::Container(list), false)
        .unwrap();
    view.fold_changed(&doc);
    view.cancel_end();
    view.home(&doc).unwrap();
    assert_eq!(view_grid(&ready(&mut view, &doc)), expected[..4]);
}

#[test]
fn deep_paths_resume_per_node_without_copying_ancestor_paths_per_leaf() {
    let prefix = "> ".repeat(600);
    let doc = parse(&format!("{prefix}a\n{prefix}\n{prefix}b\n"));
    let before = doc.work();
    let mut first = FirstFrame::new(
        &doc,
        FirstFrameConfig {
            top_tile: TileId(0),
            tile_limit: doc.tile_count(),
            width: 40,
            row_limit: 20,
        },
    )
    .unwrap();
    let frames = complete(&mut first, &doc);
    assert_eq!(grid(&first), ["│ a", "", "│ b"]);
    assert_eq!(doc.work(), before);
    assert!(first.sink_stats().unwrap().container_nodes_read <= 605);
    eprintln!(
        "deep micro frames={frames} scanner={:?} sink={:?}",
        first.scanner().stats(),
        first.sink_stats()
    );
}
#[test]
fn million_hidden_body_and_old_item_history_are_not_spacing_input() {
    use mdtui_tui::render::viewport::ViewportState;
    use mdtui3::{ContainerId, FoldId};
    let mut doc = Document::new();
    doc.enable_display_shape_effects().unwrap();
    doc.append(&format!("- {}\n\nout\n\ntail\n", "a".repeat(1_000_000)))
        .unwrap();
    doc.finish();
    let before = doc.work();
    let mut view = ViewportState::new(&doc, view_config(8)).unwrap();
    view.folds_mut()
        .set_collapsed(&doc, FoldId::Container(ContainerId(0)), true)
        .unwrap();
    view.fold_changed(&doc);
    let frame = ready(&mut view, &doc);
    let text = view_grid(&frame);
    assert!(text[0].starts_with("[+ container"), "{text:?}");
    assert_eq!(&text[1..], ["out", "", "tail"]);
    assert!(frame.stats.layout.preparation_source_bytes < 1024);
    assert_eq!(doc.work(), before);
    eprintln!(
        "million hidden COLD (includes existing canonical shape bootstrap) stats={:?}",
        frame.stats
    );
    view.set_content_width(&doc, 39).unwrap();
    let warm = ready(&mut view, &doc);
    assert_eq!(view_grid(&warm)[1..], ["out", "", "tail"]);
    assert!(warm.stats.layout_budget.bytes < 1024);
    assert!(warm.stats.layout.spacing_metadata_steps < 20);
    eprintln!("million hidden WARM resize stats={:?}", warm.stats);

    let mut doc = Document::new();
    doc.enable_display_shape_effects().unwrap();
    doc.append(&"- old\n".repeat(4096)).unwrap();
    doc.append("\n- last\n").unwrap();
    doc.finish();
    let at = doc.source().rfind("- last").unwrap();
    let mut first = FirstFrame::new(
        &doc,
        FirstFrameConfig {
            top_tile: doc.tile_at(at).unwrap(),
            tile_limit: 2,
            width: 40,
            row_limit: 100,
        },
    )
    .unwrap();
    complete(&mut first, &doc);
    assert_eq!(grid(&first).last().map(String::as_str), Some("• last"));
    assert!(first.sink_stats().unwrap().container_nodes_read < 160);
    assert!(first.scanner().stats().scanned_bytes < 1024);
    eprintln!(
        "old history scanner={:?} sink={:?}",
        first.scanner().stats(),
        first.sink_stats()
    );
}

#[test]
fn real_selection_copy_and_search_skip_spacing_rows_and_keep_original_source() {
    use crossterm::event::{KeyCode, KeyModifiers};
    use mdtui_tui::{
        cli::LinkMode,
        render::frame::{FrameGeometry, FrameRuntime},
    };
    fn paint(runtime: &mut FrameRuntime) {
        for _ in 0..10000 {
            if runtime
                .frame_to(&mut Vec::new(), &mut Budget::new(WorkLimit::unlimited()))
                .unwrap()
                .committed
            {
                return;
            }
        }
        panic!("runtime stalled");
    }
    let mut runtime = FrameRuntime::from_finished_source(
        "- a\n\n  b\n- c\n",
        FrameGeometry {
            rows: 12,
            cols: 40,
            content_width: 40,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    paint(&mut runtime);
    for code in [KeyCode::Char('v'), KeyCode::Char(' '), KeyCode::Down] {
        runtime.reading_key(code, KeyModifiers::NONE, true).unwrap();
        paint(&mut runtime);
    }
    assert_eq!(
        runtime.reading().selection.range(),
        Some(mdtui3::Span::new(2, 8))
    );
    for (code, expected) in [
        (KeyCode::Char('y'), b"\x1b]52;c;YQpi\x07".as_slice()),
        (KeyCode::Char('Y'), b"\x1b]52;c;YQoKICBi\x07".as_slice()),
    ] {
        runtime.reading_key(code, KeyModifiers::NONE, true).unwrap();
        let mut bytes = Vec::new();
        for _ in 0..10000 {
            runtime.advance_reading(&mut bytes, &mut micro());
            if !runtime.reading().pending() {
                break;
            }
        }
        assert!(
            bytes.windows(expected.len()).any(|w| w == expected),
            "wire={bytes:?}"
        );
    }
    for code in [
        KeyCode::Esc,
        KeyCode::Char('/'),
        KeyCode::Char('b'),
        KeyCode::Enter,
    ] {
        runtime.reading_key(code, KeyModifiers::NONE, true).unwrap();
    }
    for _ in 0..10000 {
        runtime.advance_reading(&mut Vec::new(), &mut micro());
        if !runtime.reading().pending() {
            break;
        }
    }
    paint(&mut runtime);
    assert_eq!(
        runtime.reading().current_match().unwrap().source,
        mdtui3::Span::new(7, 8)
    );
    assert!(
        runtime
            .screen()
            .front_cell(0, 2)
            .unwrap()
            .style()
            .contains(mdtui_tui::screen::StyleRole::SEARCH)
    );
}

#[test]
fn quote_continuation_window_keeps_quote_context_without_replaying_earlier_leaves() {
    let source = format!("{}>\n> last\n", "> one two\n".repeat(80));
    let doc = parse(&source);
    let mut first = FirstFrame::new(
        &doc,
        FirstFrameConfig {
            top_tile: TileId(1),
            tile_limit: 1,
            width: 40,
            row_limit: 100,
        },
    )
    .unwrap();
    complete(&mut first, &doc);
    assert!(!first.lines().is_empty());
    for line in first.lines() {
        assert_eq!(line.cells[0].text, "│");
        assert!(line.source.unwrap().source.start >= 256);
    }
    assert!(first.scanner().stats().scanned_bytes < 512);
}

#[test]
fn folded_overscan_and_loose_paragraph_target_share_the_stable_admission_path() {
    use mdtui_tui::render::{
        refine::{RefineConfig, RefineProgress, RefineStatus, SourceRefiner},
        viewport::ViewportState,
    };
    use mdtui3::{FoldId, HeadingId};
    let source = format!(
        "# pre\n{}# Target\n- x\n\n  y\n- z\n\nFINAL\n",
        "- a\n\n  b\n- c\n\n".repeat(80)
    );
    let mut doc = Document::new();
    doc.enable_display_shape_effects().unwrap();
    doc.append(&source).unwrap();
    doc.finish();
    let mut view = ViewportState::new(
        &doc,
        mdtui_tui::render::viewport::ViewportConfig {
            width: 40,
            rows: 4,
            max_tiles: 4,
            overscan_rows: 4,
            row_slack: 4,
            ..Default::default()
        },
    )
    .unwrap();
    view.folds_mut()
        .set_collapsed(&doc, FoldId::Heading(HeadingId(0)), true)
        .unwrap();
    view.fold_changed(&doc);
    ready(&mut view, &doc);
    let source = source.rfind("  y").unwrap() + 2;
    let target = {
        let owner = view.layout_owner_mut();
        let mut refiner = SourceRefiner::new(
            &doc,
            owner.heights(),
            owner,
            source,
            RefineConfig {
                width: 40,
                max_tiles: 4,
                row_slack: 4,
                ..Default::default()
            },
        )
        .unwrap();
        let mut result = None;
        for _ in 0..20000 {
            match refiner.advance_owned(&doc, owner, &mut micro()).unwrap() {
                RefineProgress::Ready(target) => {
                    result = Some(target);
                    break;
                }
                RefineProgress::NotFound => panic!("visible y must be a real target"),
                _ => {}
            }
        }
        result.expect("micro source refinement must continue")
    };
    assert_eq!(target.source, source);
    view.set_top_from_refinement(
        &doc,
        target.anchor,
        target.line_start,
        source,
        target.status == RefineStatus::Exact,
    )
    .unwrap();
    assert_eq!(
        view_grid(&ready(&mut view, &doc)),
        ["  y", "", "• z", "FINAL"]
    );
    view.set_content_width(&doc, 20).unwrap();
    assert_eq!(
        view_grid(&ready(&mut view, &doc)),
        ["  y", "", "• z", "FINAL"]
    );
    assert!(view.folds().is_collapsed(FoldId::Heading(HeadingId(0))));
    view.seek_end(&doc, false).unwrap();
    assert_eq!(
        view_grid(&ready(&mut view, &doc)),
        ["  y", "", "• z", "FINAL"]
    );
}

#[test]
fn indexed_cold_adoption_keeps_hidden_million_body_out_of_spacing_preparation() {
    use mdtui_tui::{
        cli::LinkMode,
        render::frame::{FrameGeometry, FrameRuntime},
    };
    use mdtui3::{ContainerId, FoldId, Utf8Document};
    let source = format!("- {}\n\nout\n\ntail\n", "a".repeat(1_000_000));
    let input_start = std::time::Instant::now();
    let mut input = Utf8Document::new();
    input.append(source.as_bytes()).unwrap();
    input.finish();
    let input_elapsed = input_start.elapsed();
    let input_work = input.document().work();
    let source_stats = input.document().display_source_stats();
    let mut runtime = FrameRuntime::from_utf8_document(
        input,
        FrameGeometry {
            rows: 8,
            cols: 40,
            content_width: 40,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    {
        let (doc, view) = runtime.document_and_viewport_mut();
        view.folds_mut()
            .set_collapsed(doc, FoldId::Container(ContainerId(0)), true)
            .unwrap();
        view.fold_changed(doc);
    }
    let mut wire = Vec::new();
    let prepare_start = std::time::Instant::now();
    let mut total = WorkLimit::default();
    let mut last = None;
    for _ in 0..1000 {
        let mut budget = Budget::new(WorkLimit::unlimited());
        let commit = runtime.frame_to(&mut wire, &mut budget).unwrap();
        total.bytes += budget.used().bytes;
        total.queries += budget.used().queries;
        total.objects += budget.used().objects;
        if commit.committed {
            last = Some(commit.metrics);
            break;
        }
    }
    let metrics = last.expect("indexed adoption must complete");
    let prepare_elapsed = prepare_start.elapsed();
    let shape = runtime.viewport().shared_links().shape().unwrap();
    assert!(total.bytes < 4096, "{total:?}");
    assert!(
        shape.work().source_decode_bytes < 1024,
        "{:?}",
        shape.work()
    );
    assert!(metrics.layout.spacing_metadata_steps < 30);
    assert!(metrics.layout.container_nodes_read < 10);
    assert_eq!(runtime.document().work(), input_work);
    assert_eq!(
        runtime.document().display_source_stats().work.decoded_bytes,
        source_stats.work.decoded_bytes
    );
    let mut parser = vt100::Parser::new(8.try_into().unwrap(), 40.try_into().unwrap(), 0);
    parser.process(&wire);
    let text = parser.screen().contents();
    let lines: Vec<_> = text.lines().collect();
    assert!(lines[0].starts_with("[+ container"), "{text}");
    assert_eq!(&lines[1..], ["out", "", "tail"]);
    assert!(wire.len() < 16 * 1024);
    eprintln!(
        "indexed cold input_elapsed={input_elapsed:?} source_stats={source_stats:?} prepare_elapsed={prepare_elapsed:?} budget={total:?} shape_work={:?} shape_capacity={} output_bytes={} layout={:?}",
        shape.work(),
        shape.capacity_bytes(),
        wire.len(),
        metrics.layout
    );
}
