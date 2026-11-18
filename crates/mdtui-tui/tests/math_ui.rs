//! Handwritten display expectations through the real scanner, sink and height pass.
use mdtui_tui::render::first_frame::{FirstFrame, FirstFrameConfig, FirstFrameStatus};
use mdtui3::{Budget, Document, TileId, WorkLimit};
fn render(source: &str, width: u16) -> FirstFrame {
    let mut d = Document::new();
    d.append(source).unwrap();
    d.finish();
    let mut first = FirstFrame::new(
        &d,
        FirstFrameConfig {
            top_tile: TileId(0),
            tile_limit: d.tile_count().max(1),
            width,
            row_limit: 24,
        },
    )
    .unwrap();
    for _ in 0..source.len() * 100 + 1000 {
        let mut b = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 6,
            tiles: 1,
            recovery: 1,
            dependencies: 1,
        });
        let step = first.advance(&d, &mut b).unwrap();
        assert!(b.used().queries <= 1 && b.used().objects <= 1 && b.used().bytes <= 6);
        if matches!(
            step.status,
            FirstFrameStatus::Covered | FirstFrameStatus::EndOfDocument
        ) {
            return first;
        }
    }
    panic!(
        "frame stalled: scan={} rows={} attempts={} scanstats={:?} sink={:?}",
        first.scan_complete(),
        first.rows(),
        first.layout_attempts(),
        first.scanner().stats(),
        first.sink_stats()
    );
}
fn grid(f: &FirstFrame) -> Vec<String> {
    f.lines()
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
fn native_math_callbacks_and_baseline_inline_rows() {
    let f = render("A \\(\\frac{a}{b}\\) Z\n", 20);
    assert_eq!(grid(&f), ["   a", "A ─── Z", "   b"]);
    assert_eq!(f.rows(), 3);
    assert_eq!(f.sink_stats().unwrap().math_object_callbacks, 1);
    assert_eq!(f.sink_stats().unwrap().document_queries_in_callbacks, 0);
}
#[test]
fn display_math_unicode_and_atomic_softwrap() {
    let f = render("\\[\n\\frac{中}{\\alpha{}́}\n\\]\n", 10);
    assert_eq!(grid(&f), [" 中", "────", " ά"]);
    assert_eq!(f.rows(), 3);
    let f = render("ab \\(\\frac{x}{y}\\)z\n", 4);
    assert_eq!(grid(&f), ["ab", " x", "───z", " y"]);
}
#[test]
fn table_cell_fraction_sets_row_height_and_preserves_native_columns() {
    let f = render("| a | b |\n| - | - |\n| \\(\\frac{x}{y}\\) | z |\n", 20);
    assert_eq!(
        grid(&f),
        [
            "┌─────┬───┐",
            "│ a   │ b │",
            "╞═════╪═══╡",
            "│  x  │   │",
            "│ ─── │ z │",
            "│  y  │   │",
            "└─────┴───┘"
        ]
    );
    assert_eq!(f.rows(), 7);
    assert_eq!(f.sink_stats().unwrap().math_object_callbacks, 1);
}
#[test]
fn a_viewport_inside_a_tall_matrix_queries_real_owner_rows() {
    use mdtui_tui::render::viewport::{ViewportConfig, ViewportFrameStatus, ViewportState};
    let source = format!(
        "\\[\n\\begin{{matrix}}{}\\end{{matrix}}\n\\]\n",
        (0..40)
            .map(|i| format!("r{i}"))
            .collect::<Vec<_>>()
            .join("\\\\")
    );
    let mut d = Document::new();
    d.append(&source).unwrap();
    d.finish();
    let mut state = ViewportState::new(
        &d,
        ViewportConfig {
            width: 10,
            rows: 4,
            overscan_rows: 0,
            max_tiles: 8,
            max_replay_bytes: 8192,
            row_slack: 4,
        },
    )
    .unwrap();
    for _ in 0..10000 {
        let f = state
            .frame(&d, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap();
        if matches!(
            f.status,
            ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument
        ) {
            break;
        }
    }
    state.scroll_down(&d, 50).unwrap();
    for _ in 0..10000 {
        let f = state
            .frame(&d, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap();
        if matches!(
            f.status,
            ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument
        ) {
            let rows: Vec<_> = f
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
                .collect();
            assert_eq!(rows, ["r25", "", "r26", ""]);
            return;
        }
    }
    panic!("viewport stalled");
}
#[test]
fn empty_objects_keep_their_owner_height_and_actual_callback() {
    for source in ["\\(\\)\n", "\\[\n\\]\n", "$$\n$$\n"] {
        let f = render(source, 8);
        assert_eq!(f.rows(), 1, "{source:?}");
        assert_eq!(grid(&f), [""]);
        assert_eq!(f.sink_stats().unwrap().math_object_callbacks, 1);
    }
}

#[test]
fn end_inside_a_tall_matrix_uses_the_intrinsic_bottom_rows() {
    use mdtui_tui::render::viewport::{ViewportConfig, ViewportFrameStatus, ViewportState};
    let source = format!(
        "\\[\n\\begin{{matrix}}{}\\end{{matrix}}\n\\]\n",
        (0..40)
            .map(|i| format!("r{i}"))
            .collect::<Vec<_>>()
            .join("\\\\")
    );
    let mut d = Document::new();
    d.append(&source).unwrap();
    d.finish();
    let mut state = ViewportState::new(
        &d,
        ViewportConfig {
            width: 10,
            rows: 4,
            overscan_rows: 0,
            max_tiles: 8,
            max_replay_bytes: 8192,
            row_slack: 4,
        },
    )
    .unwrap();
    state.seek_end(&d, false).unwrap();
    for _ in 0..10000 {
        let f = state
            .frame(&d, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap();
        if matches!(
            f.status,
            ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument
        ) {
            let rows: Vec<_> = f
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
                .collect();
            assert_eq!(rows, ["", "r38", "", "r39"]);
            return;
        }
    }
    panic!("End stalled");
}
#[test]
fn end_after_a_cross_tile_object_keeps_the_real_tail_and_zero_continuations() {
    use mdtui_tui::render::viewport::{ViewportConfig, ViewportFrameStatus, ViewportState};
    let source = format!("\\[\n{}\n\\]\n\nTAIL\n", "x".repeat(20_000));
    let mut d = Document::new();
    d.append(&source).unwrap();
    d.finish();
    let mut state = ViewportState::new(
        &d,
        ViewportConfig {
            width: 10,
            rows: 4,
            overscan_rows: 0,
            max_tiles: 8,
            max_replay_bytes: 8192,
            row_slack: 4,
        },
    )
    .unwrap();
    state.seek_end(&d, false).unwrap();
    for _ in 0..10000 {
        let f = state
            .frame(&d, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap();
        if matches!(
            f.status,
            ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument
        ) {
            let rows: Vec<_> = f
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
                .collect();
            assert!(rows.iter().any(|r| r == "TAIL"), "{rows:?}");
            for tile in 1..d.tile_count() - 1 {
                assert_eq!(state.heights().height(TileId(tile)), Some(0));
            }
            return;
        }
    }
    panic!("cross-tile End stalled");
}
#[test]
fn container_and_heading_math_keep_native_prefix_roles() {
    assert_eq!(
        grid(&render("> \\(\\frac{a}{b}\\)\n", 12)),
        ["│  a", "│ ───", "│  b"]
    );
    assert_eq!(grid(&render("# \\(x^2\\)\n", 12)), [" 2", "x"]);
    assert_eq!(
        grid(&render("- \\(\\frac{a}{b}\\)\n", 12)),
        ["   a", "• ───", "   b"]
    );
}
#[test]
fn mathematical_link_labels_keep_the_canonical_link_and_outer_style() {
    let source = "**[\\(x^2\\)](/target)**\n";
    let f = render(source, 20);
    assert_eq!(grid(&f), [" 2", "x"]);
    let cells: Vec<_> = f
        .lines()
        .iter()
        .flat_map(|l| &l.cells)
        .filter(|c| c.source.is_some())
        .collect();
    assert!(!cells.is_empty());
    for cell in cells {
        assert_eq!(cell.link, Some(mdtui3::LinkId(0)));
        assert!(cell.style.contains(mdtui_tui::screen::StyleRole::STRONG));
    }
}
#[test]
fn empty_mathematical_image_alt_keeps_the_existing_canonical_placeholder() {
    let f = render("A ![\\(\\)](/img) Z\n", 24);
    assert_eq!(grid(&f), ["A [image] Z"]);
}

#[test]
fn table_precision_pan_resize_and_end_use_mathematical_dimensions() {
    use mdtui_tui::render::viewport::{ViewportConfig, ViewportFrameStatus, ViewportState};
    let source = "| a | b | c |\n| - | - | - |\n| \\(\\frac{x}{y}\\) | \\(\\alpha\\) | TAIL |\n";
    let mut d = Document::new();
    d.append(source).unwrap();
    d.finish();
    let mut v = ViewportState::new(
        &d,
        ViewportConfig {
            width: 24,
            rows: 12,
            overscan_rows: 0,
            max_tiles: 8,
            max_replay_bytes: 8192,
            row_slack: 4,
        },
    )
    .unwrap();
    let id = mdtui3::TableId(0);
    let mut stable = false;
    for _ in 0..20000 {
        let mut b = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 6,
            ..Default::default()
        });
        let step = v.advance_table_widths(&d, id, 24, &mut b).unwrap();
        assert!(b.used().queries <= 1 && b.used().objects <= 1 && b.used().bytes <= 6);
        if step.stable_for_width {
            stable = true;
            break;
        }
    }
    assert!(stable);
    assert_eq!(
        v.table_widths_mut().snapshot(&d, id).unwrap().widths,
        [3, 1, 4]
    );
    fn rows(v: &mut ViewportState, d: &Document) -> Vec<String> {
        for _ in 0..1000 {
            let f = v
                .frame(d, &mut Budget::new(WorkLimit::unlimited()))
                .unwrap();
            if matches!(
                f.status,
                ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument
            ) {
                return f
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
                    .collect();
            }
        }
        panic!("mathematical table frame stalled");
    }
    let grid = rows(&mut v, &d);
    assert!(
        grid.iter().any(|r| r.contains("│ ─── │ α │ TAIL │")),
        "{grid:?}"
    );
    v.set_content_width(&d, 10).unwrap();
    v.set_table_first_column(&d, id, 1).unwrap();
    let grid = rows(&mut v, &d);
    assert!(grid.iter().any(|r| r.contains('α')), "{grid:?}");
    assert!(!grid.iter().any(|r| r.contains('─') && r.contains('x')));
    v.set_content_width(&d, 24).unwrap();
    v.seek_end(&d, false).unwrap();
    let grid = rows(&mut v, &d);
    assert!(grid.iter().any(|r| r.contains("TAIL")), "{grid:?}");
}

#[test]
fn a_cross_tile_mathematical_table_cell_owns_height_and_end_without_phantom_rows() {
    use mdtui_tui::render::viewport::{ViewportConfig, ViewportFrameStatus, ViewportState};
    let source = format!(
        "| a | b |\n| - | - |\n| \\({}\\) | R |\n",
        "x".repeat(20_000)
    );
    let mut d = Document::new();
    d.append(&source).unwrap();
    d.finish();
    let mut v = ViewportState::new(
        &d,
        ViewportConfig {
            width: 20,
            rows: 4,
            overscan_rows: 0,
            max_tiles: 8,
            max_replay_bytes: 8192,
            row_slack: 4,
        },
    )
    .unwrap();
    v.seek_end(&d, false).unwrap();
    for _ in 0..10000 {
        let f = v
            .frame(&d, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap();
        if matches!(
            f.status,
            ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument
        ) {
            let rows: Vec<_> = f
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
                .collect();
            assert!(
                rows.last().is_some_and(|s| s.starts_with('└')),
                "actual table bottom {rows:?}"
            );
            assert!(
                rows.iter().any(|s| s.contains('x')),
                "actual math cell {rows:?}"
            );
            assert_eq!(v.heights().height(TileId(0)), Some(5));
            for tile in 1..d.tile_count() {
                assert_eq!(
                    v.heights().height(TileId(tile)),
                    Some(0),
                    "continuation {tile}"
                );
            }
            return;
        }
    }
    panic!("table End stalled");
}

#[test]
fn folding_projects_before_math_preparation_and_restores_the_same_owner() {
    use mdtui_tui::render::viewport::{ViewportConfig, ViewportFrameStatus, ViewportState};
    use mdtui3::{FoldId, HeadingId, MathObjectId, Span};
    let mut d = Document::new();
    d.append("# A\n\\[\n\\frac{SECRET}{b}\n\\]\n\n# B\npublic\n")
        .unwrap();
    d.finish();
    let mut state = ViewportState::new(&d, ViewportConfig::default()).unwrap();
    state
        .folds_mut()
        .set_collapsed(&d, FoldId::Heading(HeadingId(0)), true)
        .unwrap();
    state.fold_changed(&d);
    let mut window = d.math_window(Span::new(0, d.source().len())).unwrap();
    let id = loop {
        if let mdtui3::MathWindowStep::Object(m) = window
            .advance(&d, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap()
        {
            break m.id;
        }
    };
    assert!(matches!(id, MathObjectId::Block(_)));
    for collapsed in [true, false] {
        let mut ready = false;
        for _ in 0..100000 {
            let mut b = Budget::new(WorkLimit {
                queries: 1,
                objects: 1,
                bytes: 6,
                tiles: 1,
                ..Default::default()
            });
            let frame = state.frame(&d, &mut b).unwrap();
            assert!(b.used().queries <= 1 && b.used().objects <= 1 && b.used().bytes <= 6);
            if matches!(
                frame.status,
                ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument
            ) {
                let text = frame
                    .lines
                    .iter()
                    .flat_map(|l| &l.cells)
                    .map(|c| c.text.collect_text())
                    .collect::<String>();
                assert_eq!(text.contains("SECRET"), !collapsed, "{text}");
                assert!(text.contains("public"), "{text}");
                if collapsed {
                    assert!(
                        state.shared_links().math().stats(id).is_none(),
                        "hidden TeX must not be prepared"
                    );
                }
                ready = true;
                break;
            }
        }
        assert!(ready);
        state
            .folds_mut()
            .set_collapsed(&d, FoldId::Heading(HeadingId(0)), false)
            .unwrap();
        state.fold_changed(&d);
    }
}

#[test]
fn a_source_fold_inside_math_stops_payload_at_the_semantic_fragment_boundary() {
    use mdtui_tui::render::viewport::{ViewportConfig, ViewportFrameStatus, ViewportState};
    use mdtui3::Span;
    let source = "before \\(a+SECRET+b\\) after\n";
    let mut d = Document::new();
    d.append(source).unwrap();
    d.finish();
    let mut state = ViewportState::new(&d, ViewportConfig::default()).unwrap();
    let at = source.find("SECRET").unwrap();
    let view = state.folds_mut();
    let mut registration =
        mdtui3::SourceFoldRegistration::overlapping(&d, view, Span::new(at, at + 6)).unwrap();
    let id = registration
        .advance(&d, view, &mut Budget::new(WorkLimit::unlimited()))
        .unwrap()
        .id
        .unwrap();
    view.set_collapsed(&d, id, true).unwrap();
    state.fold_changed(&d);
    for _ in 0..10000 {
        let frame = state
            .frame(&d, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap();
        if matches!(
            frame.status,
            ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument
        ) {
            let text = frame
                .lines
                .iter()
                .flat_map(|l| &l.cells)
                .map(|c| c.text.collect_text())
                .collect::<String>();
            assert!(!text.contains("SECRET"), "{text}");
            assert!(
                text.contains("before") && text.contains("after") && text.contains("[+ source"),
                "{text}"
            );
            return;
        }
    }
    panic!("source fold did not finish");
}
