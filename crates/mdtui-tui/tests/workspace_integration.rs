use mdtui_tui::cli::LinkMode;
use mdtui_tui::links::LinkTable;
use mdtui_tui::render::first_frame::{FirstFrame, FirstFrameConfig, FirstFrameStatus};
use mdtui_tui::render::input::DisplayInput;
use mdtui_tui::screen::{ContentArea, Screen};
use mdtui_tui::static_render::{render_once_ansi, render_static};
use mdtui3::{Budget, Document, LinkId, Span, TILE_BYTES, TileId, WorkLimit};

fn document(source: &str, finish: bool) -> Document {
    let mut document = Document::new();
    document.append(source).unwrap();
    if finish {
        document.finish();
    }
    document
}

fn first(document: &Document, tile_limit: usize, width: u16) -> FirstFrame {
    let mut first = FirstFrame::new(
        document,
        FirstFrameConfig {
            top_tile: TileId(0),
            tile_limit,
            width,
            row_limit: 4096,
        },
    )
    .unwrap();
    for _ in 0..4096 {
        let mut budget = Budget::new(mdtui_tui::stream::FRAME_WORK);
        let step = first.advance(document, &mut budget).unwrap();
        if matches!(
            step.status,
            FirstFrameStatus::Covered | FirstFrameStatus::EndOfDocument
        ) {
            return first;
        }
    }
    panic!("window did not complete");
}

fn row(screen: &Screen, row: u16) -> String {
    (0..screen.cols())
        .map(|col| screen.cell(row, col).unwrap().text())
        .collect::<String>()
        .trim_end()
        .to_owned()
}

#[test]
fn image_labels_empty_placeholders_and_ids_survive_static_and_streaming_in_every_block() {
    for source in [
        "before ![中文 &amp; alt](image.png) after\n",
        "![](image.png)\n",
        "# ![](image.png)\n",
        "- ![](image.png)\n",
        "> ![](image.png)\n",
        "| image |\n| --- |\n| ![](image.png) |\n",
        "![](one.png)![](two.png)\n",
    ] {
        let document = document(source, false);
        let expected = render_static(&document, 80, 40).unwrap();
        let first = first(&document, document.tile_count(), 80);
        let mut actual = Screen::new(40, 80, ContentArea::new(0, 80)).unwrap();
        first.write_to_screen(&mut actual, 0).unwrap();
        assert_eq!(
            first.lines().len(),
            usize::from(expected.stats.rows),
            "{source:?}"
        );
        let mut links = Vec::new();
        for y in 0..expected.stats.rows {
            assert_eq!(
                row(&actual, y),
                row(&expected.screen, y),
                "{source:?} row {y}"
            );
            for x in 0..80 {
                let cell = actual.cell(y, x).unwrap();
                assert_eq!(
                    cell.link_id(),
                    expected.screen.cell(y, x).unwrap().link_id(),
                    "{source:?}"
                );
                if let Some(id) = cell.link_id() {
                    if !links.contains(&id) {
                        links.push(id);
                    }
                    assert!(cell.source().is_some());
                    assert!(document.link_destination(id).is_some());
                }
            }
        }
        assert_eq!(links.len(), document.links().len(), "{source:?}");
        assert!(!document.is_finished());
        assert_eq!(first.sink_stats().unwrap().document_queries_in_callbacks, 0);
    }
}

#[test]
fn image_osc8_and_no_links_use_the_same_visible_placeholder() {
    let document = document("![](image.png)\n", true);
    let linked = render_once_ansi(&document, 80, 24, LinkMode::Default).unwrap();
    let plain = render_once_ansi(&document, 80, 24, LinkMode::NoOsc8).unwrap();
    assert!(
        linked
            .windows(b"\x1b]8;;image.png\x1b\\".len())
            .any(|s| s == b"\x1b]8;;image.png\x1b\\")
    );
    assert!(linked.windows(7).any(|s| s == b"[image]"));
    assert!(plain.windows(7).any(|s| s == b"[image]"));
    assert!(!plain.windows(4).any(|s| s == b"\x1b]8;"));
}

#[test]
fn table_alignment_places_padding_before_center_and_right_content() {
    let document = document(
        "| L | C | R |\n| :--- | :---: | ---: |\n| long | long | long |\n| x | y | z |\n",
        true,
    );
    let expected = render_static(&document, 80, 24).unwrap();
    assert_eq!(row(&expected.screen, 4), "│ x    │  y   │    z │");
    let first = first(&document, document.tile_count(), 80);
    let mut actual = Screen::new(24, 80, ContentArea::new(0, 80)).unwrap();
    first.write_to_screen(&mut actual, 0).unwrap();
    assert_eq!(row(&actual, 4), row(&expected.screen, 4));
}

#[test]
fn late_confirmed_email_is_focused_in_source_order_with_stable_ids() {
    let source = format!(
        "{} _foo@bar.com [x](/x) rest",
        "p".repeat(mdtui3::BLOCK_LOOKAHEAD)
    );
    let mut document = document(&source, false);
    assert_eq!(document.links().len(), 1);
    document.append("_ [z](/z)\n").unwrap();
    document.finish();
    let ids: Vec<_> = document
        .links_in(Span::new(0, document.source().len()))
        .map(|(id, _)| id)
        .collect();
    assert_eq!(ids, [LinkId(1), LinkId(0), LinkId(2)]);
    let first = first(&document, document.tile_count(), 80);
    let mut screen = Screen::new(40, 80, ContentArea::new(0, 80)).unwrap();
    first.write_to_screen(&mut screen, 0).unwrap();
    screen.swap_buffers();
    let links = LinkTable::from_screen(&screen, ContentArea::new(0, 80));
    assert_eq!(links.links(), ids);
}

#[test]
fn partial_megabyte_blocks_prepare_only_a_small_source_window() {
    for source in [
        format!("```rust\n{}\n```\n", "code\n".repeat(210_000)),
        format!("- {}\n", "word ".repeat(210_000)),
        format!("{}\n===\n", "title ".repeat(180_000)),
    ] {
        let document = document(&source, true);
        let first = first(&document, 8, 80);
        assert_eq!(first.plan().segments.len(), 8);
        assert!(first.scanner().stats().scanned_bytes <= TILE_BYTES * 8 + 4);
        assert!(!first.lines().is_empty());
        assert!(first.lines().len() < 600);
        let stats = first.sink_stats().unwrap();
        assert_eq!(stats.document_queries_in_callbacks, 0);
        assert!(stats.preparation_source_bytes <= 3 * TILE_BYTES * 8 + 4096);
        assert!(stats.scratch_capacity_cells < 200_000);
    }
}

#[test]
fn empty_image_in_a_clipped_hidden_run_is_not_duplicated() {
    let document = document("prefix ![](image.png) suffix\n", true);
    let source = Span::new(0, document.source().len());
    let whole = DisplayInput::from_document(&document, source)
        .unwrap()
        .visible_text();
    let mut pieces = String::new();
    for at in 0..document.source().len() {
        pieces.push_str(
            &DisplayInput::from_document(&document, Span::new(at, at + 1))
                .unwrap()
                .visible_text(),
        );
    }
    assert_eq!(pieces, whole);
    assert_eq!(whole.matches("[image]").count(), 1);
    let mut budget = Budget::new(WorkLimit::unlimited());
    let mut cursor = document.display_cursor(source).unwrap();
    while !matches!(
        cursor.next(&document, &mut budget).unwrap(),
        mdtui3::DisplayStep::Complete
    ) {}
}

#[test]
fn unicode_clusters_cross_tiles_and_one_scalar_budget_steps_without_splitting_cells() {
    use mdtui_tui::render::viewport::{ViewportConfig, ViewportFrameStatus, ViewportState};
    let limit = WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 6,
        tiles: 1,
        recovery: 1,
        dependencies: 1,
    };
    for cluster in [
        "👩\u{200d}💻",
        "🇨🇳",
        "a\u{301}\u{308}",
        "👨\u{200d}👩\u{200d}👧\u{200d}👦",
    ] {
        let prefix = "p".repeat(TILE_BYTES - cluster.chars().next().unwrap().len_utf8());
        let source = format!("{prefix}{cluster} end\n");
        let expected_document = document(&source, false);
        let expected = render_static(&expected_document, 80, 24).unwrap();
        for cut in prefix.len()..source.len() {
            if !source.is_char_boundary(cut) {
                continue;
            }
            let mut document = document(&source[..cut], false);
            let mut viewport = ViewportState::new(
                &document,
                ViewportConfig {
                    width: 80,
                    rows: 24,
                    ..ViewportConfig::default()
                },
            )
            .unwrap();
            // Start a real frame on the incomplete cluster, then invalidate it
            // through the same append/update path used by the CLI.
            let _ = viewport.frame(&document, &mut Budget::new(limit)).unwrap();
            let update = document.append(&source[cut..]).unwrap();
            viewport.apply_update(&document, &update).unwrap();
            let mut first = FirstFrame::new(
                &document,
                FirstFrameConfig {
                    top_tile: TileId(0),
                    tile_limit: document.tile_count(),
                    width: 80,
                    row_limit: 100,
                },
            )
            .unwrap();
            let mut complete = false;
            for _ in 0..10_000 {
                let mut budget = Budget::new(limit);
                let step = first.advance(&document, &mut budget).unwrap();
                assert!(budget.used().bytes <= limit.bytes);
                assert!(budget.used().objects <= limit.objects);
                if matches!(
                    step.status,
                    FirstFrameStatus::Covered | FirstFrameStatus::EndOfDocument
                ) {
                    complete = true;
                    break;
                }
            }
            assert!(complete);
            let mut actual = Screen::new(24, 80, ContentArea::new(0, 80)).unwrap();
            first.write_to_screen(&mut actual, 0).unwrap();
            for y in 0..expected.stats.rows {
                for x in 0..80 {
                    assert_eq!(
                        actual.cell(y, x),
                        expected.screen.cell(y, x),
                        "{cluster:?} cut={cut} {y}:{x}"
                    );
                }
            }
            let final_frame = viewport
                .frame(&document, &mut Budget::new(WorkLimit::unlimited()))
                .unwrap();
            assert!(matches!(
                final_frame.status,
                ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument
            ));
            assert!(!document.is_finished());
        }
    }
}

#[test]
fn deep_container_preparation_resumes_and_counts_only_metadata_nodes() {
    let source = format!("{} {}\n", ">".repeat(900), "body ".repeat(220_000));
    let document = document(&source, true);
    assert!(document.container_count() >= 900);
    let mut first = FirstFrame::new(
        &document,
        FirstFrameConfig {
            top_tile: TileId(0),
            tile_limit: 8,
            width: 80,
            row_limit: 1000,
        },
    )
    .unwrap();
    let mut calls = 0;
    loop {
        calls += 1;
        assert!(calls < 1000);
        let limit = WorkLimit {
            queries: 16,
            objects: 16,
            bytes: 4096,
            tiles: 16,
            recovery: 16,
            dependencies: 16,
        };
        let mut budget = Budget::new(limit);
        let step = first.advance(&document, &mut budget).unwrap();
        assert!(budget.used().queries <= limit.queries);
        assert!(budget.used().objects <= limit.objects);
        if matches!(
            step.status,
            FirstFrameStatus::Covered | FirstFrameStatus::EndOfDocument
        ) {
            break;
        }
    }
    assert!(calls > 50);
    let stats = first.sink_stats().unwrap();
    assert_eq!(stats.container_nodes_read, 900);
    assert_eq!(stats.preparation_source_bytes, 0);
    assert_eq!(stats.scratch_capacity_cells, 0);
    assert_eq!(stats.document_queries_in_callbacks, 0);
    assert!(!first.lines().is_empty());
}

#[test]
fn oversized_bare_targets_cost_no_destination_reads_or_copies_in_repeated_diff_frames() {
    use mdtui_tui::external::{ExternalTarget, ExternalTargetError};
    use mdtui_tui::render::diff::diff_screen;
    use mdtui_tui::render::output::{OutputOptions, diff_to_vec};
    use mdtui_tui::screen::{Cell, StyleRole};
    for source in [
        format!("https://e.test/{}\n", "a".repeat(1_000_000)),
        format!("{}@example.com\n", "a".repeat(1_000_000)),
    ] {
        let document = document(&source, true);
        assert_eq!(document.links().len(), 1);
        assert_eq!(
            ExternalTarget::from_document(&document, LinkId(0)),
            Err(ExternalTargetError::TooLong)
        );
        for _ in 0..4 {
            let mut screen = Screen::new(24, 80, ContentArea::new(0, 80)).unwrap();
            for y in 0..4 {
                for x in 0..12 {
                    let style = if x % 2 == 0 {
                        StyleRole::LINK
                    } else {
                        StyleRole::LINK.with(StyleRole::STRONG)
                    };
                    screen
                        .write_cell(
                            y,
                            x,
                            Cell::new("x", style, 1, Some(LinkId(0)), Some(Span::new(0, 1)))
                                .unwrap(),
                        )
                        .unwrap();
                }
            }
            let diff = diff_screen(&screen);
            let (bytes, stats) =
                diff_to_vec(&document, &screen, &diff, OutputOptions::default()).unwrap();
            assert_eq!(stats.destination_source_bytes, 0);
            assert_eq!(stats.oversized_destinations, 1);
            assert_eq!(stats.osc8_links, 0);
            assert!(stats.segments > 4);
            assert!(bytes.len() < 4096);
            assert!(!bytes.windows(4).any(|s| s == b"\x1b]8;"));
            screen.swap_buffers();
            assert_eq!(
                LinkTable::from_screen(&screen, ContentArea::new(0, 80)).links(),
                [LinkId(0)]
            );
        }
    }
}

#[test]
fn bounded_targets_are_prepared_once_per_frame_and_www_opens_with_a_scheme() {
    use mdtui_tui::external::ExternalTarget;
    use mdtui_tui::render::diff::diff_screen;
    use mdtui_tui::render::link::encode_link;
    use mdtui_tui::render::output::{OutputOptions, diff_to_vec};
    use mdtui_tui::screen::{Cell, StyleRole};
    let document = document(
        "www.example.com user@example.com [decoded](https://e.test/a\\&b?x=1&amp;y=2)\n",
        true,
    );
    let raw = document.link_destination(LinkId(0)).unwrap();
    assert_eq!(
        ExternalTarget::from_document(&document, LinkId(0))
            .unwrap()
            .as_str(),
        "http://www.example.com"
    );
    assert_eq!(
        ExternalTarget::from_document(&document, LinkId(1))
            .unwrap()
            .as_str(),
        "mailto:user@example.com"
    );
    assert_eq!(
        ExternalTarget::from_document(&document, LinkId(2))
            .unwrap()
            .as_str(),
        "https://e.test/a&b?x=1&y=2"
    );
    assert!(
        encode_link(
            &document,
            LinkId(0),
            "www",
            StyleRole::LINK,
            LinkMode::Default,
            true
        )
        .unwrap()
        .contains("\x1b]8;;http://www.example.com\x1b\\")
    );
    let mut screen = Screen::new(4, 20, ContentArea::new(0, 20)).unwrap();
    for y in 0..4 {
        screen
            .write_cell(
                y,
                0,
                Cell::new("x", StyleRole::LINK, 1, Some(LinkId(0)), None).unwrap(),
            )
            .unwrap();
    }
    let (_, stats) = diff_to_vec(
        &document,
        &screen,
        &diff_screen(&screen),
        OutputOptions::default(),
    )
    .unwrap();
    assert_eq!(stats.destination_source_bytes, raw.len());
    assert_eq!(stats.osc8_links, 4);
}

#[test]
fn viewport_window_edges_keep_complete_clusters_and_ri_parity_with_budgeted_context() {
    use unicode_segmentation::UnicodeSegmentation;
    for unit in [
        "👩\u{200d}💻",
        "🇨🇳",
        "a\u{301}\u{308}",
        "👨\u{200d}👩\u{200d}👧\u{200d}👦",
    ] {
        let source = format!("p{} end\n", unit.repeat(3000));
        let document = document(&source, true);
        let start = TileId(30);
        let end = TileId(32);
        let requested = Span::new(
            document.tile(start).unwrap().start,
            document.tile(end).unwrap().start,
        );
        let expected: Vec<_> = source
            .grapheme_indices(true)
            .filter(|(at, text)| *at < requested.end && at + text.len() > requested.start)
            .map(|(at, text)| (text.to_owned(), Span::new(at, at + text.len())))
            .collect();
        let mut first = FirstFrame::new(
            &document,
            FirstFrameConfig {
                top_tile: start,
                tile_limit: 2,
                width: 80,
                row_limit: 1000,
            },
        )
        .unwrap();
        let limit = WorkLimit {
            queries: 2,
            objects: 2,
            bytes: 32,
            tiles: 2,
            dependencies: 2,
            recovery: 2,
        };
        let mut done = false;
        for _ in 0..10_000 {
            let mut budget = Budget::new(limit);
            let step = first.advance(&document, &mut budget).unwrap();
            assert!(budget.used().bytes <= limit.bytes);
            assert!(budget.used().queries <= limit.queries);
            if matches!(
                step.status,
                FirstFrameStatus::Covered | FirstFrameStatus::EndOfDocument
            ) {
                done = true;
                break;
            }
        }
        assert!(done, "{unit:?}");
        let actual: Vec<_> = first
            .lines()
            .iter()
            .flat_map(|line| line.cells.iter())
            .map(|cell| (cell.text.clone(), cell.source.unwrap()))
            .collect();
        assert_eq!(actual, expected, "{unit:?} source window {requested:?}");
        assert!(first.scanner().stats().edge_source_bytes > 0);
        assert_eq!(first.scanner().stats().omitted_edge_clusters, 0);
        assert!(first.scanner().stats().scanned_bytes <= requested.len() + unit.len() * 2);
        assert_eq!(first.sink_stats().unwrap().preparation_source_bytes, 0);
    }
}

#[test]
fn sparse_long_table_snapshot_has_no_rows_times_columns_allocation() {
    use mdtui_tui::render::table_widths::TableWidthCache;
    let columns = 128;
    let mut source = format!(
        "|{}|\n|{}|\n",
        vec!["h"; columns].join("|"),
        vec!["---"; columns].join("|")
    );
    source.push_str(&"| x |\n".repeat(30_000));
    let document = document(&source, true);
    assert_eq!(document.tables().len(), 1);
    let mut cache = TableWidthCache::new(&document);
    let _ = cache.snapshot(&document, mdtui3::TableId(0)).unwrap();
    let before = cache.memory_stats();
    assert_eq!(before.observed_rows, 0);
    assert!(before.tracked_capacity_bytes < columns * 512, "{before:?}");
    let mut budget = Budget::new(WorkLimit {
        queries: 4,
        objects: 4,
        bytes: 128,
        ..WorkLimit::default()
    });
    cache
        .observe_visible_rows(
            &document,
            mdtui3::TableId(0),
            80,
            15_000..15_004,
            &mut budget,
        )
        .unwrap();
    let after = cache.memory_stats();
    assert!(after.observed_rows <= 4);
    assert!(after.tracked_capacity_bytes < columns * 512, "{after:?}");
}

#[test]
fn actual_runtime_reconciles_scrolled_source_for_core_heading_navigation() {
    use mdtui_tui::render::frame::{FrameGeometry, FrameRuntime};
    let source = format!("# first\n\n{}\n\n# second\n", "body ".repeat(120));
    let mut runtime = FrameRuntime::from_finished_source(
        &source,
        FrameGeometry {
            rows: 8,
            cols: 40,
            content_width: 40,
        },
        LinkMode::Default,
    )
    .unwrap();
    for _ in 0..100 {
        if runtime
            .frame_to(&mut Vec::new(), &mut Budget::new(WorkLimit::unlimited()))
            .unwrap()
            .committed
        {
            break;
        }
    }
    assert!(runtime.scroll_down(3).unwrap());
    for _ in 0..100 {
        if runtime
            .frame_to(&mut Vec::new(), &mut Budget::new(WorkLimit::unlimited()))
            .unwrap()
            .committed
        {
            break;
        }
    }
    let actual = runtime.top_source().unwrap();
    assert!(actual > source.find("body").unwrap());
    let target = runtime
        .navigator()
        .clone()
        .navigate(
            runtime.document(),
            runtime.viewport().heights(),
            mdtui3::NavAction::PreviousHeading { max_level: 6 },
            8,
        )
        .unwrap();
    assert_eq!(target.source, Some(0));
}

#[test]
fn giant_cell_keeps_provisional_screen_and_key_redraw_available_during_width_calibration() {
    use mdtui_tui::render::frame::{FrameGeometry, FrameRuntime};
    use mdtui_tui::stream::{FRAME_WORK, StreamEngine};
    use std::time::Duration;
    let mut engine = StreamEngine::new(
        FrameRuntime::new(
            FrameGeometry {
                rows: 24,
                cols: 80,
                content_width: 80,
            },
            LinkMode::Default,
        )
        .unwrap(),
    );
    let source = format!("| h |\n| --- |\n| {} |\n", "x".repeat(1_000_000));
    for bytes in source.as_bytes().chunks(32 * 1024) {
        engine.ingest_data(bytes, Duration::ZERO).unwrap();
        if !engine.first_frame_done() {
            for _ in 0..10 {
                engine
                    .render_due(
                        &mut Vec::new(),
                        &mut Budget::new(FRAME_WORK),
                        Duration::from_millis(100),
                    )
                    .unwrap();
                if engine.first_frame_done() {
                    break;
                }
            }
        }
    }
    assert!(engine.first_frame_done());
    engine.finish_input(Duration::from_secs(1)).unwrap();
    let mut budget = Budget::new(FRAME_WORK);
    engine.advance_calibration(&mut budget).unwrap();
    assert!(budget.used().bytes <= FRAME_WORK.bytes);
    assert!(engine.calibration_pending());
    engine.request_render();
    assert!(engine.due(Duration::from_secs(2)));
    let mut committed = false;
    for _ in 0..100 {
        if engine
            .render_due(
                &mut Vec::new(),
                &mut Budget::new(FRAME_WORK),
                Duration::from_secs(2),
            )
            .unwrap()
            .is_some_and(|commit| commit.committed)
        {
            committed = true;
            break;
        }
    }
    assert!(committed);
    for _ in 0..100 {
        if engine
            .advance_calibration(&mut Budget::new(FRAME_WORK))
            .unwrap()
            .is_some()
        {
            break;
        }
    }
    assert!(
        engine
            .render_due(
                &mut Vec::new(),
                &mut Budget::new(FRAME_WORK),
                Duration::from_secs(3)
            )
            .unwrap()
            .is_some_and(|c| c.committed)
    );
    assert!(
        !engine.calibration_pending(),
        "giant cell precision must finish"
    );
    let id = mdtui3::TableId(0);
    let runtime = engine.runtime_mut();
    let (document, viewport) = runtime.document_and_viewport_mut();
    assert_eq!(
        viewport
            .table_widths_mut()
            .snapshot(document, id)
            .unwrap()
            .widths[0],
        1_000_000
    );
    let first_row: String = (0..80)
        .map(|x| engine.screen().front_cell(0, x).unwrap().text())
        .collect();
    assert!(first_row.contains('┌'));
}

#[test]
fn semantic_zwj_and_hidden_markers_survive_real_viewport_edge_scroll_and_checkpoint_recovery() {
    use mdtui_tui::render::viewport::{
        ViewportConfig, ViewportFrame, ViewportFrameStatus, ViewportState,
    };
    fn ready(state: &mut ViewportState, document: &Document) -> ViewportFrame {
        for _ in 0..1000 {
            let mut budget = Budget::new(mdtui_tui::stream::FRAME_WORK);
            let frame = state.frame(document, &mut budget).unwrap();
            assert!(budget.used().bytes <= mdtui_tui::stream::FRAME_WORK.bytes);
            if matches!(
                frame.status,
                ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument
            ) {
                return frame;
            }
        }
        panic!("semantic edge frame did not complete");
    }
    for unit in [
        "👩&#x200d;💻",
        "👩**\u{200d}💻**",
        "a&#x301;",
        "a**\u{301}**",
    ] {
        let first_scalar = unit.chars().next().unwrap().len_utf8();
        let prefix = "p".repeat(TILE_BYTES * 30 - first_scalar);
        let source = format!("{prefix}{unit} {}\n", "x".repeat(8000));
        let document = document(&source, true);
        let wanted = if unit.starts_with('a') {
            "a\u{301}"
        } else {
            "👩\u{200d}💻"
        };
        let expected = render_static(&document, 80, 256).unwrap();
        let expected_cell = (0..expected.stats.rows)
            .flat_map(|y| (0..80).map(move |x| (y, x)))
            .filter_map(|(y, x)| expected.screen.cell(y, x))
            .find(|cell| cell.text() == wanted)
            .unwrap_or_else(|| {
                panic!("core did not produce the expected visible cluster for {unit:?}")
            });
        let requested = document.tile(TileId(30)).unwrap().start;
        let mut state = ViewportState::new(
            &document,
            ViewportConfig {
                width: 80,
                rows: 3,
                max_tiles: 1,
                overscan_rows: 0,
                ..ViewportConfig::default()
            },
        )
        .unwrap();
        state
            .set_top_from_refinement(
                &document,
                mdtui3::Anchor {
                    tile: TileId(30),
                    offset: 0,
                },
                requested,
                requested,
                false,
            )
            .unwrap();
        let first = ready(&mut state, &document);
        let cell = &first.lines[0].cells[0];
        assert_eq!(cell.text, wanted, "{unit:?}");
        assert_eq!(cell.width, expected_cell.width());
        assert_eq!(cell.source, expected_cell.source());
        assert!(state.scroll_down(&document, 1).unwrap());
        let _ = ready(&mut state, &document);
        assert!(state.scroll_up(&document, 1).unwrap());
        let restored = ready(&mut state, &document);
        assert_eq!(restored.lines[0].cells[0], *cell);
    }
}

#[test]
fn overlong_edge_cluster_has_one_source_mapped_local_height_placeholder() {
    let source = format!("a{}\n", "\u{301}".repeat(100_000));
    let document = document(&source, true);
    let start = TileId(100);
    let end = TileId(102);
    let requested = Span::new(
        document.tile(start).unwrap().start,
        document.tile(end).unwrap().start,
    );
    let mut first = FirstFrame::new(
        &document,
        FirstFrameConfig {
            top_tile: start,
            tile_limit: 2,
            width: 80,
            row_limit: 24,
        },
    )
    .unwrap();
    let mut done = false;
    for _ in 0..1000 {
        let mut budget = Budget::new(mdtui_tui::stream::FRAME_WORK);
        let step = first.advance(&document, &mut budget).unwrap();
        if matches!(
            step.status,
            FirstFrameStatus::Covered | FirstFrameStatus::EndOfDocument
        ) {
            done = true;
            break;
        }
    }
    assert!(done);
    assert_eq!(first.lines().len(), 1);
    assert_eq!(first.lines()[0].cells.len(), 1);
    let cell = &first.lines()[0].cells[0];
    assert_eq!(cell.text, "…");
    assert_eq!(cell.source, Some(requested));
    assert_eq!(cell.width, 1);
    assert_eq!(first.sink_stats().unwrap().edge_placeholders, 1);
    assert_eq!(
        first.heights().height_state(TileId(101)),
        Some(mdtui3::HeightState::Local)
    );
    assert_eq!(first.heights().height(TileId(101)), Some(1));
    assert!(first.scanner().stats().edge_source_bytes < 40_000);
    assert!(first.scanner().stats().display_source_work < 40_000);
}

#[test]
fn runtime_heading_jump_resumes_across_frames_and_preserves_the_raw_heading_source() {
    use mdtui_tui::render::frame::{FrameGeometry, FrameRuntime};
    let source = format!(
        "# first\n\n{}\n\n## second\n\ntrailer\n",
        "**x** ".repeat(300)
    );
    let mut runtime = FrameRuntime::from_finished_source(
        &source,
        FrameGeometry {
            rows: 3,
            cols: 80,
            content_width: 80,
        },
        LinkMode::Default,
    )
    .unwrap();
    runtime.set_sidebar_visible(true).unwrap();
    runtime
        .frame_to(&mut Vec::new(), &mut Budget::new(WorkLimit::unlimited()))
        .unwrap();
    assert!(runtime.sidebar_selection_end());
    let limit = WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 6,
        tiles: 1,
        recovery: 1,
        dependencies: 1,
    };
    assert!(
        !runtime
            .activate_sidebar_selection(&mut Budget::new(limit))
            .unwrap()
    );
    assert!(runtime.has_pending_jump());
    let mut done = false;
    for _ in 0..10_000 {
        let mut budget = Budget::new(limit);
        let commit = runtime.frame_to(&mut Vec::new(), &mut budget).unwrap();
        assert!(budget.used().bytes <= limit.bytes);
        if commit.committed {
            done = true;
            break;
        }
    }
    assert!(done);
    assert!(!runtime.has_pending_jump());
    let heading = runtime
        .document()
        .headings()
        .get(mdtui3::HeadingId(1))
        .unwrap();
    assert_eq!(runtime.top_source(), Some(heading.source.start));
    let previous = runtime
        .navigator()
        .clone()
        .navigate(
            runtime.document(),
            runtime.viewport().heights(),
            mdtui3::NavAction::PreviousHeading { max_level: 6 },
            3,
        )
        .unwrap();
    assert_eq!(previous.source, Some(0));
}

#[test]
fn frame_link_source_and_osc8_payload_quotas_preserve_all_application_link_ids() {
    use mdtui_tui::render::diff::diff_screen;
    use mdtui_tui::render::output::{
        MAX_FRAME_DESTINATION_SOURCE_BYTES, MAX_FRAME_OSC8_TARGET_BYTES, OutputOptions, diff_to_vec,
    };
    use mdtui_tui::screen::{Cell, StyleRole};
    let source: String = (0..30)
        .map(|n| format!("[x](https://e.test/{}?id={n})\n", "a".repeat(3000)))
        .collect();
    let document = document(&source, true);
    assert_eq!(document.links().len(), 30);
    let mut screen = Screen::new(40, 80, ContentArea::new(0, 80)).unwrap();
    for y in 0..30 {
        screen
            .write_cell(
                y,
                0,
                Cell::new("x", StyleRole::LINK, 1, Some(LinkId(usize::from(y))), None).unwrap(),
            )
            .unwrap();
    }
    let (bytes, stats) = diff_to_vec(
        &document,
        &screen,
        &diff_screen(&screen),
        OutputOptions::default(),
    )
    .unwrap();
    assert!(stats.destination_source_bytes <= MAX_FRAME_DESTINATION_SOURCE_BYTES);
    assert!(stats.osc8_target_bytes <= MAX_FRAME_OSC8_TARGET_BYTES);
    assert!(stats.deferred_link_segments > 0);
    assert!(bytes.len() < MAX_FRAME_OSC8_TARGET_BYTES + 4096);
    assert_eq!(
        bytes
            .windows(b"\x1b]8;;\x1b\\".len())
            .filter(|s| *s == b"\x1b]8;;\x1b\\")
            .count(),
        stats.osc8_links
    );
    screen.swap_buffers();
    assert_eq!(
        LinkTable::from_screen(&screen, ContentArea::new(0, 80)).len(),
        30
    );
}

#[test]
fn distant_ri_lookbehind_limit_never_guesses_flag_parity() {
    let document = document(&format!("p{}\n", "🇨🇳".repeat(20_000)), true);
    let mut first = FirstFrame::new(
        &document,
        FirstFrameConfig {
            top_tile: TileId(100),
            tile_limit: 2,
            width: 80,
            row_limit: 24,
        },
    )
    .unwrap();
    let mut done = false;
    for _ in 0..1000 {
        let mut budget = Budget::new(mdtui_tui::stream::FRAME_WORK);
        let step = first.advance(&document, &mut budget).unwrap();
        if matches!(
            step.status,
            FirstFrameStatus::Covered | FirstFrameStatus::EndOfDocument
        ) {
            done = true;
            break;
        }
    }
    assert!(done);
    assert!(first.scanner().stats().context_limited_edges > 0);
    assert!(first.scanner().stats().edge_source_bytes < 40_000);
    assert_eq!(first.lines().len(), 1);
    assert_eq!(first.lines()[0].cells[0].text, "…");
    assert_eq!(first.sink_stats().unwrap().edge_placeholders, 1);
    assert_eq!(
        first.heights().height_state(TileId(101)),
        Some(mdtui3::HeightState::Local)
    );
}

#[test]
fn append_revisions_keep_table_cache_capacity_tied_to_visible_tables() {
    use mdtui_tui::render::viewport::{ViewportConfig, ViewportFrameStatus, ViewportState};
    let mut document = Document::new();
    let mut state = ViewportState::new(
        &document,
        ViewportConfig {
            width: 80,
            rows: 4,
            max_tiles: 4,
            overscan_rows: 0,
            ..ViewportConfig::default()
        },
    )
    .unwrap();
    for n in 0..1000 {
        let update = document
            .append(&format!("| h |\n| --- |\n| value-{n} |\n\n"))
            .unwrap();
        state.apply_update(&document, &update).unwrap();
        if n % 40 != 0 {
            continue;
        }
        for _ in 0..100 {
            let frame = state
                .frame(&document, &mut Budget::new(mdtui_tui::stream::FRAME_WORK))
                .unwrap();
            if matches!(
                frame.status,
                ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument
            ) {
                break;
            }
        }
        let memory = state.table_widths().memory_stats();
        assert!(memory.tables < 50, "{n}: {memory:?}");
        assert!(memory.tracked_capacity_bytes < 64 * 1024, "{n}: {memory:?}");
    }
    assert_eq!(document.tables().len(), 1000);
}

#[test]
fn unresolved_images_remain_literal_until_a_late_definition_invalidates_layout() {
    use mdtui_tui::render::viewport::{ViewportConfig, ViewportFrameStatus, ViewportState};
    for raw in [
        "![][missing]",
        "![alt][missing]",
        "![alt](broken",
        "![](broken",
    ] {
        let doc = document(&format!("{raw}\n"), false);
        let static_frame = render_static(&doc, 80, 24).unwrap();
        let first = first(&doc, doc.tile_count(), 80);
        let mut actual = Screen::new(24, 80, ContentArea::new(0, 80)).unwrap();
        first.write_to_screen(&mut actual, 0).unwrap();
        assert_eq!(row(&actual, 0), raw);
        assert_eq!(row(&static_frame.screen, 0), raw);
        for x in 0..80 {
            assert_eq!(actual.cell(0, x).unwrap().link_id(), None, "{raw}");
        }
    }
    let empty = document("![]()\n", false);
    assert_eq!(
        row(&render_static(&empty, 80, 24).unwrap().screen, 0),
        "[image]"
    );
    for label in ["", "alt"] {
        let mut doc = document(&format!("![{label}][pic]\n"), false);
        let mut viewport = ViewportState::new(&doc, ViewportConfig::default()).unwrap();
        let before = viewport
            .frame(&doc, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap();
        let ticket = viewport.heights().ticket(TileId(0)).unwrap();
        let update = doc.append("\n[pic]: image.png\n").unwrap();
        viewport.apply_update(&doc, &update).unwrap();
        assert!(!before.matches(&doc));
        assert_eq!(
            viewport.heights_mut().measure(ticket, 1),
            Err(mdtui3::LayoutError::StaleMeasurement)
        );
        let frame = viewport
            .frame(&doc, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap();
        assert!(matches!(
            frame.status,
            ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument
        ));
        let text: String = frame
            .lines
            .iter()
            .flat_map(|line| &line.cells)
            .map(|cell| cell.text.as_str())
            .collect();
        assert_eq!(text, if label.is_empty() { "[image]" } else { label });
        assert!(
            frame
                .lines
                .iter()
                .flat_map(|line| &line.cells)
                .all(|cell| cell.link == Some(LinkId(0)) && cell.source.is_some())
        );
        assert!(!doc.is_finished());
    }
}

#[test]
fn million_byte_fence_line_is_raw_before_lf_then_bounded_label_and_body_without_eof() {
    use mdtui_tui::render::viewport::{
        ViewportConfig, ViewportFrame, ViewportFrameStatus, ViewportState,
    };
    fn ready(state: &mut ViewportState, doc: &Document) -> ViewportFrame {
        for _ in 0..256 {
            let mut budget = Budget::new(mdtui_tui::stream::FRAME_WORK);
            let frame = state.frame(doc, &mut budget).unwrap();
            assert!(budget.used().bytes <= mdtui_tui::stream::FRAME_WORK.bytes);
            if matches!(
                frame.status,
                ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument
            ) {
                return frame;
            }
        }
        panic!("fence viewport failed to converge");
    }
    for marker_length in [3, 1_000_003] {
        let raw = format!("{}{}", "`".repeat(marker_length), "rust".repeat(250_000));
        let mut doc = document(&raw, false);
        let mut state = ViewportState::new(
            &doc,
            ViewportConfig {
                width: 80,
                rows: 24,
                max_tiles: 16,
                ..ViewportConfig::default()
            },
        )
        .unwrap();
        let frame = ready(&mut state, &doc);
        assert_eq!(frame.rows, 24);
        assert!(
            frame
                .lines
                .iter()
                .flat_map(|line| &line.cells)
                .all(|cell| !cell.text.contains('…'))
        );
        assert_eq!(frame.lines[0].cells[0].text, "`");
        let old_ticket = state.heights().ticket(TileId(0)).unwrap();
        let update = doc.append("\nbody one\nbody two\n").unwrap();
        assert!(update.dirty.iter().any(|s| s.start == 0));
        state.apply_update(&doc, &update).unwrap();
        assert_eq!(
            state.heights_mut().measure(old_ticket, 1),
            Err(mdtui3::LayoutError::StaleMeasurement)
        );
        let frame = ready(&mut state, &doc);
        let lines: Vec<String> = frame
            .lines
            .iter()
            .map(|line| line.cells.iter().map(|cell| cell.text.as_str()).collect())
            .collect();
        assert!(
            lines.iter().any(|line| line.contains("body one")),
            "{lines:?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("body two")),
            "{lines:?}"
        );
        assert!(lines[0].starts_with("[rust"), "{lines:?}");
        assert!(!lines.iter().any(|line| line.contains("```")));
        assert!(frame.stats.layout.preparation_source_bytes <= 4096);
        assert!(frame.stats.planned_tiles <= 16);
        assert!(!doc.is_finished());
    }
}

#[test]
fn confirmed_million_marker_without_info_or_body_is_an_empty_bounded_viewport() {
    use mdtui_tui::render::viewport::{ViewportConfig, ViewportFrameStatus, ViewportState};
    let doc = document(&format!("{}\n", "~".repeat(1_000_003)), false);
    let mut state = ViewportState::new(
        &doc,
        ViewportConfig {
            max_tiles: 8,
            ..ViewportConfig::default()
        },
    )
    .unwrap();
    for _ in 0..32 {
        let frame = state
            .frame(&doc, &mut Budget::new(mdtui_tui::stream::FRAME_WORK))
            .unwrap();
        if frame.status == ViewportFrameStatus::Ready {
            assert_eq!(frame.rows, 0);
            assert!(!doc.is_finished());
            return;
        }
    }
    panic!("empty code did not settle");
}

#[test]
fn failed_long_fence_info_is_rendered_as_a_table_header_without_implicit_finish() {
    let raw = format!(
        "```{}` | second\n--- | ---\nbody | cell\n",
        "a".repeat(1_000_000)
    );
    let doc = document(&raw, false);
    assert_eq!(doc.tables().len(), 1);
    let first = first(&doc, 8, 80);
    assert!(
        first
            .lines()
            .iter()
            .flat_map(|line| &line.cells)
            .any(|cell| cell.text == "`")
    );
    // The cell prefix has 4 KiB grapheme context after the 4-byte/scalar
    // terminal prefix; the indexed second cell is read directly, 1 MiB away.
    assert_eq!(
        first.scanner().stats().scanned_bytes,
        80 * 4 + 4096 + "second".len()
    );
    assert!(first.sink_stats().unwrap().preparation_source_bytes <= 4096);
    assert!(!doc.is_finished());
}
