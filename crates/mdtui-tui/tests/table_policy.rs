use mdtui_tui::render::table_widths::{TableWidthCache, TableWidthPolicy};
use mdtui3::{BlockKind, Budget, Document, TableId, WorkLimit};

fn parse(source: &str) -> (Document, TableId) {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    let id = doc
        .blocks()
        .iter()
        .find_map(|b| match b.kind {
            BlockKind::Table { id } => Some(id),
            _ => None,
        })
        .unwrap();
    (doc, id)
}

#[test]
fn policy_validates_and_clamps_real_display_widths() {
    assert!(TableWidthPolicy::new(8, Some(4)).is_err());
    assert!(TableWidthPolicy::new(0, Some(0)).is_err());
    assert!(TableWidthPolicy::new(1001, None).is_err());
    let (doc, id) = parse("|a|b|\n|---|---|\n|中&amp;文|![x](i)|\n");
    let mut cache = TableWidthCache::new(&doc);
    cache.set_policy(TableWidthPolicy::new(3, Some(4)).unwrap());
    let result = cache
        .advance(&doc, id, 80, &mut Budget::new(WorkLimit::unlimited()))
        .unwrap();
    assert!(result.stable_for_width);
    assert_eq!(cache.snapshot(&doc, id).unwrap().widths, [4, 3]);
}

#[test]
fn million_byte_cell_precision_is_resumable_and_bounded() {
    let (doc, id) = parse(&format!(
        "|a|b|\n|---|---|\n|{}|中|\n",
        "x".repeat(1_000_000)
    ));
    let mut cache = TableWidthCache::new(&doc);
    let mut frames = 0;
    let mut bytes = 0;
    let mut peak_capacity = 0;
    loop {
        let mut budget = Budget::new(mdtui_tui::stream::FRAME_WORK);
        let result = cache.advance(&doc, id, 80, &mut budget).unwrap();
        bytes += budget.used().bytes;
        frames += 1;
        assert!(frames < 100, "precision made no progress");
        peak_capacity = peak_capacity.max(cache.memory_stats().tracked_capacity_bytes);
        assert!(peak_capacity < 256 * 1024);
        if result.stable_for_width {
            break;
        }
    }
    assert_eq!(cache.snapshot(&doc, id).unwrap().widths[0], 1_000_000);
    assert!((1_000_000..3_000_000).contains(&bytes), "bytes={bytes}");
    println!(
        "million precision: frames={frames} paid_bytes={bytes} peak_tracked_capacity={peak_capacity} work={:?}",
        cache.work()
    );
}

#[test]
fn last_column_metadata_is_a_window_not_a_prefix() {
    let (doc, id) = parse(&format!(
        "{}|\n{}|\n{}|\n",
        "|h".repeat(100_000),
        "|---".repeat(100_000),
        "|x".repeat(100_000)
    ));
    let mut cache = TableWidthCache::new(&doc);
    assert!(cache.set_first_column(&doc, id, 99_998).unwrap());
    assert_eq!(cache.work().column_metadata_reads, 0);
    let snap = cache
        .snapshot_budgeted(&doc, id, 80, &mut Budget::new(WorkLimit::unlimited()))
        .unwrap()
        .unwrap();
    assert_eq!(snap.first_column, 99_998);
    assert_eq!(snap.widths.len(), 2);
    assert_eq!(cache.work().column_metadata_reads, 2);
    println!(
        "last-column metadata: work={:?} memory={:?}",
        cache.work(),
        cache.memory_stats()
    );
}

fn budget(bytes: usize) -> Budget {
    Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        bytes,
        dependencies: 1,
        tiles: 1,
        recovery: 1,
    })
}

fn settle(
    cache: &mut TableWidthCache,
    doc: &Document,
    id: TableId,
    columns: usize,
    bytes: usize,
) -> usize {
    for frame in 0..200_000 {
        let mut work = budget(bytes);
        let result = cache.advance(doc, id, columns, &mut work).unwrap();
        assert!(work.used().queries <= 1 && work.used().objects <= 1 && work.used().bytes <= bytes);
        if result.stable_for_width {
            return frame + 1;
        }
    }
    panic!("measurement stalled");
}

// This naive oracle deliberately owns the full display in tests. It is not the
// production width/cache/geometry implementation and has no provisional hints.
fn model_width(text: &str) -> usize {
    use unicode_segmentation::UnicodeSegmentation;
    use unicode_width::UnicodeWidthStr;
    text.graphemes(true)
        .map(|g| UnicodeWidthStr::width(g).min(2))
        .sum()
}

#[test]
fn real_semantics_match_handwritten_display_oracle_at_scalar_budgets() {
    let cases = [
        ("中文", "中文"),
        ("e\u{301}", "e\u{301}"),
        ("👩&#x200d;💻", "👩\u{200d}💻"),
        ("🇺**🇸**", "🇺🇸"),
        ("&amp;&lt;", "&<"),
        (r"a\*b", "a*b"),
        ("` a  b `", "a  b"),
        ("**abc**", "abc"),
        ("https://a.test/z", "https://a.test/z"),
        ("[x](https://a.test/long/destination)", "x"),
        ("![中](image)", "中"),
        ("![](image)", "[image]"),
        ("![a ![b](inner) c](outer)", "a b c"),
        ("![![](inner)](outer)", "[image]"),
        ("![x][absent]", "![x][absent]"),
        ("[![y](inner)](outer)", "y"),
    ];
    for (source, displayed) in cases {
        let (doc, id) = parse(&format!("|h|\n|---|\n|{source}|\n"));
        let mut cache = TableWidthCache::new(&doc);
        let frames = settle(&mut cache, &doc, id, 1000, 6);
        assert!(frames > 1);
        assert_eq!(
            cache.snapshot(&doc, id).unwrap().widths,
            [model_width(displayed).max(1)],
            "source={source:?}, displayed={displayed:?}"
        );
    }
}

fn ready(
    view: &mut mdtui_tui::render::viewport::ViewportState,
    doc: &Document,
) -> mdtui_tui::render::viewport::ViewportFrame {
    use mdtui_tui::render::viewport::ViewportFrameStatus;
    for _ in 0..5000 {
        let frame = view
            .frame(doc, &mut Budget::new(mdtui_tui::stream::FRAME_WORK))
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
fn line_text(line: &mdtui_tui::render::sink::SinkLine) -> String {
    line.cells
        .iter()
        .map(|cell| cell.text.as_str())
        .collect::<String>()
        .trim_end()
        .to_owned()
}

#[test]
fn cell_boundaries_reset_unicode_but_display_runs_inside_one_cell_join() {
    use mdtui_tui::render::viewport::{ViewportConfig, ViewportState};
    let (doc, id) = parse("|h|h|h|h|h|\n|---|---|---|---|---|\n|🇺|🇸|👩&#x200d;|💻|👩&#x200d;💻|\n");
    let mut cache = TableWidthCache::new(&doc);
    settle(&mut cache, &doc, id, 80, 6);
    let expected = [
        model_width("🇺"),
        model_width("🇸"),
        model_width("👩\u{200d}"),
        2,
        2,
    ];
    assert_eq!(cache.snapshot(&doc, id).unwrap().widths, expected);
    let mut view = ViewportState::new(
        &doc,
        ViewportConfig {
            rows: 10,
            width: 80,
            ..ViewportConfig::default()
        },
    )
    .unwrap();
    let frame = ready(&mut view, &doc);
    let cells: Vec<_> = frame
        .lines
        .iter()
        .flat_map(|line| &line.cells)
        .filter(|cell| {
            cell.text.contains('🇺')
                || cell.text.contains('🇸')
                || cell.text.contains('👩')
                || cell.text.contains('💻')
        })
        .collect();
    assert_eq!(
        cells
            .iter()
            .map(|cell| cell.text.as_str())
            .collect::<Vec<_>>(),
        ["🇺", "🇸", "👩\u{200d}", "💻", "👩\u{200d}💻"]
    );
    for cell in cells {
        assert!(cell.source.is_some());
    }
    view.set_table_first_column(&doc, id, 3).unwrap();
    let panned = ready(&mut view, &doc);
    assert!(line_text(&panned.lines[3]).contains("💻"));
    assert!(!line_text(&panned.lines[3]).contains('🇺'));
}

#[test]
fn far_column_layout_reads_indexed_window_and_keeps_original_row_source_and_hits() {
    use mdtui_tui::render::viewport::{ViewportConfig, ViewportState};
    use mdtui3::Span;
    let source = format!(
        "{}|[![TAIL](inner)](https://outer.test)|\n{}|---:|\n|x|\n",
        "|h".repeat(99_999),
        "|---".repeat(99_999)
    );
    let (doc, id) = parse(&source);
    let mut view = ViewportState::new(
        &doc,
        ViewportConfig {
            rows: 8,
            width: 40,
            max_tiles: 8,
            ..ViewportConfig::default()
        },
    )
    .unwrap();
    view.set_table_policy(&doc, TableWidthPolicy::new(3, Some(6)).unwrap())
        .unwrap();
    view.set_table_first_column(&doc, id, 99_998).unwrap();
    let old = view.heights().ticket(mdtui3::TileId(0)).unwrap();
    let frame = ready(&mut view, &doc);
    assert!(line_text(&frame.lines[1]).contains("TAIL"));
    assert_eq!(
        frame.lines[1].source.unwrap().source,
        doc.table(id).unwrap().rows()[0].source
    );
    let tail = frame.lines[1]
        .cells
        .iter()
        .find(|cell| cell.text == "T")
        .unwrap();
    assert!(tail.source.unwrap().start > 100_000);
    assert!(tail.link.is_some());
    assert_eq!(frame.stats.layout.document_queries_in_callbacks, 0);
    assert!(
        frame.stats.layout_budget.bytes < 2048,
        "{:?}",
        frame.stats.layout_budget
    );
    assert_eq!(view.table_widths().memory_stats().columns, 2);
    assert_eq!(view.table_widths().work().column_metadata_reads, 2);
    view.set_table_first_column(&doc, id, 99_999).unwrap();
    assert!(view.heights_mut().measure(old, 1).is_err());
    let final_frame = ready(&mut view, &doc);
    assert!(line_text(&final_frame.lines[1]).contains("TAIL"));
    assert_eq!(final_frame.lines[1].source.unwrap().source.start, 0);
    println!(
        "far-column frame: work={:?} budget={:?} memory={:?}",
        view.table_widths().work(),
        frame.stats.layout_budget,
        view.table_widths().memory_stats()
    );
    assert_eq!(
        doc.text(Span::new(
            tail.source.unwrap().start,
            tail.source.unwrap().end
        ))
        .unwrap(),
        "T"
    );
}

#[test]
fn pan_policy_resize_and_append_preserve_identity_and_invalidate_tickets() {
    use mdtui_tui::render::viewport::{ViewportConfig, ViewportState};
    let mut doc = Document::new();
    doc.append("|a|b|c|d|\n|---|:---:|---:|---|\n|aa|bb|cc|dd|\n")
        .unwrap();
    let id = TableId(0);
    let mut view = ViewportState::new(
        &doc,
        ViewportConfig {
            rows: 8,
            width: 20,
            ..ViewportConfig::default()
        },
    )
    .unwrap();
    view.set_table_policy(&doc, TableWidthPolicy::new(3, Some(3)).unwrap())
        .unwrap();
    ready(&mut view, &doc);
    let ticket = view.heights().ticket(mdtui3::TileId(0)).unwrap();
    view.set_table_first_column(&doc, id, 2).unwrap();
    assert!(view.heights_mut().measure(ticket, 4).is_err());
    let panned = ready(&mut view, &doc);
    assert_eq!(line_text(&panned.lines[3]), "│  cc │ dd  │");
    view.set_content_width(&doc, 30).unwrap();
    assert_eq!(view.table_widths().first_column(id), 2);
    let update = doc.append("|z|z|new|z|\n").unwrap();
    view.apply_update(&doc, &update).unwrap();
    assert_eq!(view.table_widths().first_column(id), 2);
    assert!(line_text(&ready(&mut view, &doc).lines[1]).contains('c'));
    assert!(view.set_table_first_column(&doc, id, usize::MAX).unwrap());
    assert_eq!(view.table_widths().first_column(id), 3);
    assert!(!view.set_table_first_column(&doc, id, usize::MAX).unwrap());
}

#[test]
fn long_precision_does_not_starve_other_visible_columns() {
    let (doc, id) = parse(&format!(
        "|a|b|\n|---|---|\n|{}|中|\n",
        "x".repeat(1_000_000)
    ));
    let mut cache = TableWidthCache::new(&doc);
    cache.set_policy(TableWidthPolicy::new(1, Some(8)).unwrap());
    for _ in 0..2 {
        cache
            .advance(
                &doc,
                id,
                80,
                &mut Budget::new(mdtui_tui::stream::FRAME_WORK),
            )
            .unwrap();
    }
    let snapshot = cache.snapshot(&doc, id).unwrap();
    assert!(!snapshot.exact[0]);
    assert!(
        snapshot.exact[1],
        "short columns must progress while a long cell is pending"
    );
}

#[test]
fn pending_precision_survives_unrelated_appends_without_replaying_old_prefix() {
    let mut doc = Document::new();
    doc.append(&format!(
        "|a|b|\n|---|---|\n|{}|z|\n",
        "x".repeat(1_000_000)
    ))
    .unwrap();
    let id = TableId(0);
    let mut cache = TableWidthCache::new(&doc);
    for _ in 0..30 {
        cache
            .advance(
                &doc,
                id,
                80,
                &mut Budget::new(WorkLimit {
                    bytes: 16_384,
                    queries: 4096,
                    objects: 4096,
                    dependencies: 4096,
                    ..WorkLimit::default()
                }),
            )
            .unwrap();
        let update = doc.append("|z|z|\n").unwrap();
        cache.apply_update(&doc, &update).unwrap();
    }
    for _ in 0..100 {
        if cache
            .advance(
                &doc,
                id,
                80,
                &mut Budget::new(mdtui_tui::stream::FRAME_WORK),
            )
            .unwrap()
            .stable_for_width
        {
            break;
        }
    }
    assert_eq!(cache.snapshot(&doc, id).unwrap().widths[0], 1_000_000);
    assert!(
        cache.work().measurement_source_bytes < 1_200_000,
        "{:?}",
        cache.work()
    );
}

#[test]
fn cli_policy_boundaries_and_pan_keys_are_explicit() {
    use crossterm::event::{KeyCode, KeyModifiers};
    use mdtui_tui::{
        app::{KeyAction, action_for_key},
        cli::{self, Outcome},
    };
    let parse = |args: &[&str]| cli::parse(args.iter().map(std::ffi::OsString::from));
    for args in [
        vec!["--table-min-width", "0", "--table-max-width", "1", "a.md"],
        vec![
            "--table-min-width",
            "1000",
            "--table-max-width",
            "1000",
            "a.md",
        ],
    ] {
        assert!(matches!(parse(&args).unwrap(), Outcome::Run(_)));
    }
    for args in [
        vec!["--table-min-width", "1001", "a.md"],
        vec!["--table-max-width", "0", "a.md"],
        vec!["--table-min-width", "9", "--table-max-width", "8", "a.md"],
        vec!["--table-min-width", "x", "a.md"],
        vec!["--once", "--table-max-width", "8", "a.md"],
        vec!["--table-max-width", "8", "--table-max-width", "9", "a.md"],
    ] {
        assert!(parse(&args).is_err(), "{args:?}");
    }
    assert_eq!(
        action_for_key(KeyCode::Left, KeyModifiers::NONE),
        KeyAction::PanTable(-1)
    );
    assert_eq!(
        action_for_key(KeyCode::Right, KeyModifiers::CONTROL),
        KeyAction::PanTable(8)
    );
    assert_eq!(
        action_for_key(KeyCode::Left, KeyModifiers::ALT),
        KeyAction::HistoryBack
    );
}

#[test]
fn table_window_keeps_deep_row_offsets_when_a_paragraph_shares_its_tile() {
    use mdtui_tui::render::viewport::{ViewportConfig, ViewportState};
    let source = format!(
        "p\n\n|h|\n|---|\n{}",
        (0..200).map(|i| format!("|{i}|\n")).collect::<String>()
    );
    let (doc, _) = parse(&source);
    let mut view = ViewportState::new(
        &doc,
        ViewportConfig {
            rows: 4,
            width: 40,
            max_tiles: 4,
            ..ViewportConfig::default()
        },
    )
    .unwrap();
    ready(&mut view, &doc);
    assert!(view.scroll_down(&doc, 38).unwrap());
    let frame = ready(&mut view, &doc);
    assert_eq!(frame.rows, 4);
    assert!(
        line_text(&frame.lines[0]).contains("33"),
        "{:?}",
        frame.lines.iter().map(line_text).collect::<Vec<_>>()
    );
}

#[test]
fn independent_full_table_model_covers_bounds_narrow_windows_and_alignment() {
    use mdtui_tui::render::table::table_geometry_with_widths;
    for columns in 1..=8 {
        let raw = [
            "中",
            "&amp;",
            "e\u{301}",
            "**abcd**",
            "👩&#x200d;💻",
            "![](i)",
            "![z](i)",
            "🇺🇸",
        ];
        let display = [
            "中",
            "&",
            "e\u{301}",
            "abcd",
            "👩\u{200d}💻",
            "[image]",
            "z",
            "🇺🇸",
        ];
        let (doc, id) = parse(&format!(
            "{}|\n{}|\n|{}|\n",
            "|h".repeat(columns),
            "|---:".repeat(columns),
            raw[..columns].join("|")
        ));
        for (min, max) in [
            (0, None),
            (0, Some(1)),
            (3, Some(3)),
            (5, Some(8)),
            (1000, Some(1000)),
        ] {
            let mut cache = TableWidthCache::new(&doc);
            cache.set_policy(TableWidthPolicy::new(min, max).unwrap());
            cache
                .advance(&doc, id, 20_000, &mut Budget::new(WorkLimit::unlimited()))
                .unwrap();
            let snapshot = cache
                .snapshot_budgeted(&doc, id, 20_000, &mut Budget::new(WorkLimit::unlimited()))
                .unwrap()
                .unwrap();
            let expected: Vec<_> = display[..columns]
                .iter()
                .map(|s| {
                    model_width(s)
                        .max(1)
                        .max(min)
                        .min(max.unwrap_or(usize::MAX))
                })
                .collect();
            assert_eq!(snapshot.widths, expected);
            let table = doc.table(id).unwrap();
            for width in [1, 2, 3, 4, 5, 8, 12, 20, 40, 80] {
                let mut used = 1;
                let mut visible = 0;
                for natural in &expected {
                    if used + natural + 3 > width {
                        break;
                    }
                    used += natural + 3;
                    visible += 1;
                }
                visible = visible.max(1);
                let geometry = table_geometry_with_widths(
                    &doc,
                    table.block,
                    doc.block(table.block).unwrap(),
                    width,
                    &snapshot.widths,
                    0..table.rows().len(),
                    false,
                )
                .unwrap();
                assert_eq!(geometry.visible_columns, visible);
                assert_eq!(
                    geometry.full_width,
                    expected.iter().sum::<usize>() + 3 * columns + 1
                );
                assert!(
                    geometry
                        .columns
                        .iter()
                        .all(|c| c.alignment == mdtui3::Alignment::Right)
                );
            }
        }
    }
}

#[test]
fn million_byte_image_alt_and_local_giant_cluster_have_explicit_width_truth() {
    for (source, truth) in [
        (format!("![{}](i)", "x".repeat(1_000_000)), 1_000_000),
        (format!("a{}", "\u{301}".repeat(5000)), 1),
    ] {
        let (doc, id) = parse(&format!("|h|\n|---|\n|{source}|\n"));
        let mut cache = TableWidthCache::new(&doc);
        for _ in 0..100 {
            if cache
                .advance(
                    &doc,
                    id,
                    80,
                    &mut Budget::new(mdtui_tui::stream::FRAME_WORK),
                )
                .unwrap()
                .stable_for_width
            {
                break;
            }
            assert!(cache.memory_stats().tracked_capacity_bytes < 128 * 1024);
        }
        assert_eq!(cache.snapshot(&doc, id).unwrap().widths, [truth]);
        if truth == 1 {
            use mdtui_tui::render::viewport::{ViewportConfig, ViewportState};
            let mut view = ViewportState::new(
                &doc,
                ViewportConfig {
                    rows: 10,
                    width: 80,
                    max_tiles: 8,
                    ..ViewportConfig::default()
                },
            )
            .unwrap();
            let frame = ready(&mut view, &doc);
            // The body begins outside the initial tile plan; jump by its indexed row source.
            let source = doc.table(id).unwrap().rows()[1].source.start;
            view.set_top_from_refinement(
                &doc,
                mdtui3::Anchor {
                    tile: doc.tile_at(source).unwrap(),
                    offset: 0,
                },
                source,
                source,
                false,
            )
            .unwrap();
            let body = ready(&mut view, &doc);
            assert!(
                body.lines.iter().any(|l| line_text(l).contains('…')),
                "{:?} {:?}",
                frame.lines,
                body.lines
            );
        }
    }
}

#[test]
fn late_definition_discards_stale_precision_and_remeasures_real_display() {
    let mut doc = Document::new();
    doc.append(&format!(
        "|h|\n|---|\n|{}![y][late]|\n",
        "x".repeat(400_000)
    ))
    .unwrap();
    let id = TableId(0);
    let mut cache = TableWidthCache::new(&doc);
    cache
        .advance(
            &doc,
            id,
            80,
            &mut Budget::new(mdtui_tui::stream::FRAME_WORK),
        )
        .unwrap();
    let update = doc.append("\n[late]: image\n").unwrap();
    assert!(cache.advance(&doc, id, 80, &mut budget(6)).is_err());
    cache.apply_update(&doc, &update).unwrap();
    let eof = doc.finish();
    cache.apply_update(&doc, &eof).unwrap();
    for _ in 0..100 {
        if cache
            .advance(
                &doc,
                id,
                80,
                &mut Budget::new(mdtui_tui::stream::FRAME_WORK),
            )
            .unwrap()
            .stable_for_width
        {
            break;
        }
    }
    assert_eq!(cache.snapshot(&doc, id).unwrap().widths, [400_001]);
}

#[test]
fn app_pans_first_painted_table_with_large_steps_boundaries_and_sidebar_noop() {
    use crossterm::event::{KeyCode, KeyModifiers};
    use mdtui_tui::{
        app::AppState,
        cli::LinkMode,
        render::frame::{FrameGeometry, FrameRuntime},
        stream::StreamEngine,
    };
    use std::time::Duration;
    let source = format!(
        "# H\n\n{}|\n{}|\n{}|\n\n|second|other|\n|---|---|\n|one|two|\n",
        "|h".repeat(12),
        "|---".repeat(12),
        "|x".repeat(12)
    );
    let mut engine = StreamEngine::new(
        FrameRuntime::new(
            FrameGeometry {
                rows: 20,
                cols: 80,
                content_width: 80,
            },
            LinkMode::Default,
        )
        .unwrap(),
    );
    engine
        .set_table_policy(TableWidthPolicy::new(3, Some(3)).unwrap())
        .unwrap();
    assert!(
        !engine.due(Duration::ZERO),
        "initial table settings must permit input reads"
    );
    engine
        .ingest_data(source.as_bytes(), Duration::ZERO)
        .unwrap();
    engine.finish_input(Duration::ZERO).unwrap();
    fn draw(engine: &mut StreamEngine) {
        for _ in 0..1000 {
            if engine.calibration_pending() {
                engine
                    .advance_calibration(&mut Budget::new(mdtui_tui::stream::FRAME_WORK))
                    .unwrap();
            }
            if engine
                .render_due(
                    &mut Vec::new(),
                    &mut Budget::new(mdtui_tui::stream::FRAME_WORK),
                    Duration::from_secs(100),
                )
                .unwrap()
                .is_some_and(|c| c.committed)
            {
                return;
            }
        }
        panic!("app draw stalled");
    }
    draw(&mut engine);
    let mut app = AppState::default();
    assert!(
        app.handle_key(&mut engine, KeyCode::Right, KeyModifiers::NONE)
            .unwrap()
            .redraw
    );
    draw(&mut engine);
    assert_eq!(
        engine
            .runtime()
            .viewport()
            .table_widths()
            .first_column(TableId(0)),
        1
    );
    assert_eq!(
        engine
            .runtime()
            .viewport()
            .table_widths()
            .first_column(TableId(1)),
        0
    );
    app.handle_key(&mut engine, KeyCode::Right, KeyModifiers::CONTROL)
        .unwrap();
    draw(&mut engine);
    assert_eq!(
        engine
            .runtime()
            .viewport()
            .table_widths()
            .first_column(TableId(0)),
        9
    );
    app.handle_key(&mut engine, KeyCode::Right, KeyModifiers::CONTROL)
        .unwrap();
    draw(&mut engine);
    assert_eq!(
        engine
            .runtime()
            .viewport()
            .table_widths()
            .first_column(TableId(0)),
        11
    );
    assert!(
        !app.handle_key(&mut engine, KeyCode::Right, KeyModifiers::NONE)
            .unwrap()
            .redraw
    );
    app.handle_key(&mut engine, KeyCode::Char('s'), KeyModifiers::NONE)
        .unwrap();
    draw(&mut engine);
    app.handle_key(&mut engine, KeyCode::Tab, KeyModifiers::NONE)
        .unwrap();
    assert!(
        !app.handle_key(&mut engine, KeyCode::Left, KeyModifiers::NONE)
            .unwrap()
            .redraw
    );
    engine
        .runtime_mut()
        .viewport_mut()
        .table_widths_mut()
        .retain_tables(&[]);
    assert_eq!(
        engine
            .runtime()
            .viewport()
            .table_widths()
            .first_column(TableId(0)),
        0
    );
}

#[test]
fn cancelling_width_measurement_keeps_one_memo_for_paid_node_retirement() {
    use mdtui_tui::render::objects::SharedLinks;
    let (doc, id) = parse(&format!(
        "|a|b|\n|---|---|\n|[![alt](/img) {}](/outer)|z|\n",
        (0..1024).map(|n| format!("[r{n}] ")).collect::<String>()
    ));
    let links = SharedLinks::new(&doc);
    let mut cache = TableWidthCache::new(&doc);
    cache.set_links(links.clone());
    for _ in 0..1000 {
        cache
            .advance(
                &doc,
                id,
                80,
                &mut Budget::new(WorkLimit {
                    queries: 128,
                    objects: 128,
                    bytes: 4096,
                    dependencies: 128,
                    ..WorkLimit::default()
                }),
            )
            .unwrap();
        if links.stats().memo_entries > 32 {
            break;
        }
    }
    let before = links.stats();
    assert!(before.memo_entries > 32);
    cache.set_first_column(&doc, id, 1).unwrap();
    assert_eq!(links.stats(), before);
    assert!(
        !links
            .prepare(&doc, &mut Budget::new(WorkLimit::default()))
            .unwrap()
    );
    assert_eq!(links.stats(), before);
    for _ in 0..20_000 {
        let old = links.stats();
        let done = links.prepare(&doc, &mut budget(6)).unwrap();
        let new = links.stats();
        assert!(
            new.resolution.retired_entries - old.resolution.retired_entries
                + new.retired_presentations
                - old.retired_presentations
                <= 1
        );
        if done {
            break;
        }
    }
    assert_eq!(links.stats().memo_entries, 0);
    assert_eq!(links.stats().presentation_entries, 0);
    println!(
        "width cancellation: before={before:?} after={:?}",
        links.stats()
    );
}

#[test]
fn configured_truncation_never_skips_an_unfitting_wide_prefix_to_show_later_tokens() {
    use mdtui_tui::render::viewport::{ViewportConfig, ViewportState};
    let (doc, _) = parse("|h|\n|---|\n|中a|\n");
    let mut view = ViewportState::new(
        &doc,
        ViewportConfig {
            rows: 8,
            width: 20,
            ..ViewportConfig::default()
        },
    )
    .unwrap();
    view.set_table_policy(&doc, TableWidthPolicy::new(0, Some(1)).unwrap())
        .unwrap();
    let frame = ready(&mut view, &doc);
    assert_eq!(line_text(&frame.lines[3]), "│   │");
}

#[test]
fn table_status_covers_the_entire_row_without_invisible_source_or_image_hits() {
    use mdtui_tui::{
        cli::LinkMode,
        render::frame::{FrameGeometry, FrameRuntime},
    };
    let mut runtime = FrameRuntime::new(
        FrameGeometry {
            rows: 8,
            cols: 80,
            content_width: 80,
        },
        LinkMode::Default,
    )
    .unwrap();
    runtime
        .set_table_policy(TableWidthPolicy::new(3, Some(3)).unwrap())
        .unwrap();
    let row = format!(
        "|{}|\n",
        (0..12)
            .map(|_| "![img](https://image.test)")
            .collect::<Vec<_>>()
            .join("|")
    );
    let source = format!(
        "{}|\n{}|\n{}",
        "|h".repeat(12),
        "|---".repeat(12),
        row.repeat(20)
    );
    let update = runtime.append_bytes(source.as_bytes()).unwrap().unwrap();
    runtime.apply_update(&update).unwrap();
    for _ in 0..100 {
        if runtime
            .frame_to(
                &mut Vec::new(),
                &mut Budget::new(mdtui_tui::stream::FRAME_WORK),
            )
            .unwrap()
            .committed
        {
            break;
        }
    }
    let message: String = (0..80)
        .map(|x| runtime.screen().front_cell(7, x).unwrap().text())
        .collect();
    assert!(message.starts_with("table 1 cols 1-12 / 12"), "{message}");
    assert!(
        (0..80).all(|x| runtime
            .screen()
            .front_cell(7, x)
            .unwrap()
            .link_id()
            .is_none()),
        "status retained image hits"
    );
    assert!(
        (0..80).all(|x| runtime
            .screen()
            .front_cell(7, x)
            .unwrap()
            .source()
            .is_none()),
        "status retained body source"
    );
}

#[test]
fn repeated_resize_resumes_precision_inside_the_retained_column_window() {
    use mdtui_tui::render::viewport::{ViewportConfig, ViewportState};
    let (doc, id) = parse(&format!(
        "|a|b|\n|---|---|\n|{}|z|\n",
        "x".repeat(1_000_000)
    ));
    let mut view = ViewportState::new(&doc, ViewportConfig::default()).unwrap();
    for step in 0..30 {
        view.advance_table_widths(
            &doc,
            id,
            view.config().width as usize,
            &mut Budget::new(WorkLimit {
                bytes: 16_384,
                queries: 4096,
                objects: 4096,
                dependencies: 4096,
                ..WorkLimit::default()
            }),
        )
        .unwrap();
        view.set_content_width(&doc, 20 + step % 2).unwrap();
    }
    for _ in 0..100 {
        if view
            .advance_table_widths(
                &doc,
                id,
                20,
                &mut Budget::new(mdtui_tui::stream::FRAME_WORK),
            )
            .unwrap()
            .stable_for_width
        {
            break;
        }
    }
    assert_eq!(
        view.table_widths_mut().snapshot(&doc, id).unwrap().widths[0],
        1_000_000
    );
    assert!(view.table_widths().work().measurement_source_bytes < 1_200_000);
    println!("resize precision: {:?}", view.table_widths().work());
}
