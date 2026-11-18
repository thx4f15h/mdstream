use crossterm::event::{KeyCode, KeyModifiers};
use mdtui_tui::{
    cli::LinkMode,
    render::frame::{FrameGeometry, FrameRuntime},
};
use mdtui3::{Budget, FoldId, HeadingId, WorkLimit};
fn draw(runtime: &mut FrameRuntime, parser: &mut vt100::Parser) -> String {
    let mut bytes = Vec::new();
    for _ in 0..10000 {
        let commit = runtime
            .frame_to(&mut bytes, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap();
        if commit.committed && !runtime.fold_pending() {
            parser.process(&bytes);
            return parser.screen().contents();
        }
    }
    panic!("frame did not settle")
}
#[test]
fn real_keyboard_collapses_and_restores_heading_body() {
    let mut runtime = FrameRuntime::from_finished_source(
        "# A\nSECRET\n\n# B\npublic\n",
        FrameGeometry {
            rows: 24,
            cols: 80,
            content_width: 80,
        },
        LinkMode::Default,
    )
    .unwrap();
    let mut parser = vt100::Parser::new(24.try_into().unwrap(), 80.try_into().unwrap(), 0);
    assert!(draw(&mut runtime, &mut parser).contains("SECRET"));
    runtime
        .reading_key(KeyCode::Char('z'), KeyModifiers::NONE, true)
        .unwrap();
    draw(&mut runtime, &mut parser);
    assert!(runtime.fold_menu_key(KeyCode::Enter));
    let folded = draw(&mut runtime, &mut parser);
    assert!(
        runtime
            .viewport()
            .folds()
            .is_collapsed(FoldId::Heading(HeadingId(0)))
    );
    assert!(!folded.contains("SECRET"), "{folded}");
    assert!(folded.contains("[+ heading"), "{folded}");
    assert!(folded.contains("public"), "{folded}");
    assert!(runtime.fold_menu_key(KeyCode::Enter));
    assert!(draw(&mut runtime, &mut parser).contains("SECRET"));
}

fn ready_view(
    view: &mut mdtui_tui::render::viewport::ViewportState,
    doc: &mdtui3::Document,
) -> mdtui_tui::render::viewport::ViewportFrame {
    for _ in 0..100_000 {
        let frame = view
            .frame(doc, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap();
        if matches!(
            frame.status,
            mdtui_tui::render::viewport::ViewportFrameStatus::Ready
                | mdtui_tui::render::viewport::ViewportFrameStatus::EndOfDocument
        ) {
            return frame;
        }
    }
    panic!("view did not converge")
}
fn body(frame: &mdtui_tui::render::viewport::ViewportFrame) -> String {
    frame
        .lines
        .iter()
        .map(|line| {
            line.cells
                .iter()
                .map(|cell| cell.text.collect_text())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
fn register(
    view: &mut mdtui3::FoldView<mdtui_tui::render::checkpoint::RenderCheckpoint>,
    doc: &mdtui3::Document,
    span: mdtui3::Span,
) -> FoldId {
    let mut task = mdtui3::SourceFoldRegistration::overlapping(doc, view, span).unwrap();
    let progress = task
        .advance(doc, view, &mut Budget::new(WorkLimit::unlimited()))
        .unwrap();
    let id = progress.id.unwrap();
    view.set_collapsed(doc, id, true).unwrap();
    id
}
#[test]
fn production_source_fragments_skip_nested_crossing_same_start_and_adjacent_ranges() {
    use mdtui_tui::render::viewport::{ViewportConfig, ViewportState};
    use mdtui3::{Document, Span};
    let mut doc = Document::new();
    doc.append("abCDEFGHIJKLmnOPQRSTuv\n").unwrap();
    doc.finish();
    let mut view = ViewportState::new(&doc, ViewportConfig::default()).unwrap();
    let a = register(view.folds_mut(), &doc, Span::new(2, 8));
    let b = register(view.folds_mut(), &doc, Span::new(5, 12));
    let _same = register(view.folds_mut(), &doc, Span::new(2, 6));
    let c = register(view.folds_mut(), &doc, Span::new(14, 20));
    let _nested = register(view.folds_mut(), &doc, Span::new(15, 18));
    view.fold_changed(&doc);
    let text = body(&ready_view(&mut view, &doc));
    assert!(!text.contains("CDE"), "{text}");
    assert!(!text.contains("OPQ"), "{text}");
    assert!(text.contains("ab"));
    assert!(text.contains("mn"));
    assert!(text.contains("uv"));
    assert_eq!(text.matches("[+ source").count(), 2, "{text}");
    view.folds_mut().set_collapsed(&doc, a, false).unwrap();
    view.folds_mut().set_collapsed(&doc, b, false).unwrap();
    view.folds_mut().set_collapsed(&doc, c, false).unwrap();
    view.fold_changed(&doc);
    let text = body(&ready_view(&mut view, &doc));
    assert_eq!(text.matches("[+ source").count(), 2, "{text}");
}
#[test]
fn fixed_table_rows_have_one_placeholder_and_preserve_header_alignment_and_tail() {
    use mdtui_tui::render::viewport::{ViewportConfig, ViewportState};
    use mdtui3::{Document, TableId};
    let mut doc = Document::new();
    doc.append("| A | B |\n| :--- | ---: |\n| ONE | TWO |\n| SECRET | HIDE |\n| LAST | TAIL |\n")
        .unwrap();
    doc.finish();
    let mut view = ViewportState::new(&doc, ViewportConfig::default()).unwrap();
    let id = view
        .folds_mut()
        .insert_table_rows(&doc, TableId(0), 2..3)
        .unwrap();
    view.folds_mut().set_collapsed(&doc, id, true).unwrap();
    view.fold_changed(&doc);
    let text = body(&ready_view(&mut view, &doc));
    assert!(text.contains("A"));
    assert!(text.contains("ONE"), "{text}");
    assert!(text.contains("LAST"), "{text}");
    assert!(!text.contains("SECRET"), "{text}");
    assert!(!text.contains("HIDE"), "{text}");
    assert_eq!(text.matches("[+ rows").count(), 1, "{text}");
    assert_eq!(text.matches('┌').count(), 1, "{text}");
    assert_eq!(text.matches('└').count(), 1, "{text}");
}
#[test]
fn resizing_during_real_compaction_preserves_the_fold_owner_and_registered_range() {
    use mdtui_tui::render::{
        checkpoint::{CheckpointMaintenancePhase, RenderCheckpoint},
        viewport::{ViewportConfig, ViewportState},
    };
    use mdtui3::{Document, Span, TileId};
    let mut doc = Document::new();
    doc.append(&format!("# H\n{}\n", "secret ".repeat(1000)))
        .unwrap();
    doc.finish();
    let mut view = ViewportState::new(&doc, ViewportConfig::default()).unwrap();
    let id = register(view.folds_mut(), &doc, Span::new(8, 2000));
    for i in 0..doc.tile_count() {
        view.checkpoints_mut()
            .insert(TileId(i), RenderCheckpoint::line_mid(i, 0))
            .unwrap();
    }
    let token = view.checkpoints().token(TileId(0)).unwrap();
    let mut entered = false;
    for _ in 0..10000 {
        let progress = view
            .maintain_checkpoints(
                &doc,
                &mut Budget::new(WorkLimit {
                    queries: 1,
                    objects: 1,
                    ..Default::default()
                }),
            )
            .unwrap();
        if progress.phase == Some(CheckpointMaintenancePhase::Compact) {
            entered = true;
            break;
        }
    }
    assert!(entered);
    let _rows = view.heights().total_rows();
    view.set_content_width(&doc, 40).unwrap();
    assert!(!view.checkpoints().validates(token));
    for _ in 0..10000 {
        if view
            .maintain_checkpoints(
                &doc,
                &mut Budget::new(WorkLimit {
                    queries: 1,
                    objects: 1,
                    ..Default::default()
                }),
            )
            .unwrap()
            .phase
            .is_none()
        {
            break;
        }
    }
    assert!(!view.checkpoints().maintenance_in_progress());
    assert!(view.folds().is_collapsed(id));
    assert!(view.folds().fold_candidate(&doc, id).unwrap().is_some());
    let text = body(&ready_view(&mut view, &doc));
    assert!(text.contains("[+ source"), "{text}");
}
#[test]
fn panned_table_with_a_million_hidden_cell_bytes_reads_only_visible_cell_windows() {
    use mdtui_tui::render::viewport::{ViewportConfig, ViewportState};
    use mdtui3::{Document, TableId};
    let mut doc = Document::new();
    doc.enable_display_shape_effects().unwrap();
    doc.append(&format!(
        "| HIDDEN | VISIBLE |\n| --- | --- |\n| {} | TAIL |\n",
        "secret".repeat(170_000)
    ))
    .unwrap();
    doc.finish();
    let hidden = doc.table(TableId(0)).unwrap().cell(1, 0).unwrap().source;
    let mut view = ViewportState::new(&doc, ViewportConfig::default()).unwrap();
    let links = view.shared_links();
    assert!(
        links
            .prepare_shape(&doc, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap()
    );
    let shape = links.shape().unwrap();
    let before = shape.work();
    register(view.folds_mut(), &doc, hidden);
    view.fold_changed(&doc);
    view.set_table_first_column(&doc, TableId(0), 1).unwrap();
    let frame = ready_view(&mut view, &doc);
    assert!(
        frame.stats.layout_budget.bytes < 16_384,
        "{:?}",
        frame.stats
    );
    assert_eq!(shape.work().source_decode_bytes, before.source_decode_bytes);
    let text = body(&frame);
    assert!(text.contains("VISIBLE"), "{text}");
    assert!(text.contains("TAIL"), "{text}");
    assert!(!text.contains("secret"));
    assert_eq!(view.table_widths().first_column(TableId(0)), 1);
}

#[test]
fn code_summary_is_paid_decoded_bounded_and_preserves_the_single_placeholder_source() {
    use mdtui3::{BlockId, CodeInfoStep, Document};
    let mut doc = Document::new();
    doc.append("``` r\\+&ouml;\nSECRET\n```\n").unwrap();
    doc.finish();
    let mut cursor = doc.code_info_prefix(BlockId(0), 256).unwrap();
    let mut info = String::new();
    for _ in 0..1000 {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 6,
            ..Default::default()
        });
        match cursor.next(&doc, &mut budget).unwrap() {
            CodeInfoStep::Atom { utf8, len, .. } => {
                info.push_str(std::str::from_utf8(&utf8[..usize::from(len)]).unwrap())
            }
            CodeInfoStep::Complete => break,
            CodeInfoStep::BudgetExhausted => {}
        }
    }
    assert_eq!(info, "r+ö");
    let mut runtime = FrameRuntime::from_finished_source(
        "``` r\\+&ouml;\nSECRET\n```\n",
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
    runtime.open_fold_menu(false);
    draw(&mut runtime, &mut parser);
    runtime.fold_menu_key(KeyCode::Enter);
    draw(&mut runtime, &mut parser);
    // Node page: toggle/open/close/parent/preview/end-preview/summary.
    for _ in 0..6 {
        runtime.fold_menu_key(KeyCode::Down);
    }
    runtime.fold_menu_key(KeyCode::Enter);
    let grid = draw(&mut runtime, &mut parser);
    assert!(grid.contains("r+ö"), "{grid}");
    assert!(!grid.contains("SECRET"), "{grid}");
    assert_eq!(grid.matches("[+ code").count(), 1, "{grid}");
    let mut mapped = 0;
    for row in 0..22 {
        for col in 0..80 {
            if runtime
                .screen()
                .front_cell(row, col)
                .unwrap()
                .source()
                .is_some()
            {
                mapped += 1;
            }
        }
    }
    assert!(mapped > 4);
}

#[test]
fn placeholder_selection_maps_the_full_hidden_crlf_source_and_visible_copy_contains_one_label() {
    let source = "# A\r\nSECRET\r\n\r\n# B\r\nbody\r\n";
    let mut runtime = FrameRuntime::from_finished_source(
        source,
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
    runtime.open_fold_menu(false);
    draw(&mut runtime, &mut parser);
    runtime.fold_menu_key(KeyCode::Enter);
    draw(&mut runtime, &mut parser);
    runtime.fold_menu_key(KeyCode::Esc);
    draw(&mut runtime, &mut parser);
    runtime
        .reading_key(KeyCode::Char('v'), KeyModifiers::NONE, true)
        .unwrap();
    runtime
        .reading_key(KeyCode::Down, KeyModifiers::NONE, true)
        .unwrap();
    let expected = mdtui3::Span::new(source.find("SECRET").unwrap(), source.find("# B").unwrap());
    assert_eq!(runtime.reading().selection.range(), Some(expected));
    runtime
        .reading_key(KeyCode::Char('y'), KeyModifiers::NONE, true)
        .unwrap();
    let mut wire = Vec::new();
    for _ in 0..10000 {
        runtime.advance_reading(
            &mut wire,
            &mut Budget::new(WorkLimit {
                queries: 1,
                objects: 1,
                bytes: 6,
                tiles: 1,
                dependencies: 1,
                recovery: 1,
            }),
        );
        if !runtime.reading().copy_pending() {
            break;
        }
    }
    assert!(!runtime.reading().copy_pending());
    assert!(wire.starts_with(b"\x1b]52;c;"));
    // The OSC52 payload must contain the placeholder, never its summary/body.
    let data = std::str::from_utf8(&wire[7..wire.len() - 2]).unwrap();
    let decode = |text: &str| {
        let mut out = Vec::new();
        let mut acc = 0u32;
        let mut bits = 0;
        for ch in text.bytes().filter(|b| *b != b'=') {
            let n = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
                .iter()
                .position(|b| *b == ch)
                .unwrap() as u32;
            acc = (acc << 6) | n;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((acc >> bits) as u8);
            }
        }
        out
    };
    let copied = decode(data);
    let copied = String::from_utf8(copied).unwrap();
    assert_eq!(copied.matches("[+ heading").count(), 1, "{copied}");
    assert!(!copied.contains("SECRET"));
    runtime
        .reading_key(KeyCode::Char('Y'), KeyModifiers::NONE, true)
        .unwrap();
    wire.clear();
    for _ in 0..10000 {
        runtime.advance_reading(&mut wire, &mut Budget::new(WorkLimit::unlimited()));
        if !runtime.reading().copy_pending() {
            break;
        }
    }
    assert_eq!(
        decode(std::str::from_utf8(&wire[7..wire.len() - 2]).unwrap()),
        b"SECRET\r\n\r\n"
    );
}

#[test]
fn resize_preserves_a_same_tile_source_target_and_a_collapsed_placeholder_top() {
    let source = "# First\nfirst\n\n# Second\nsecond\n\n# Third\nthird\n";
    let mut runtime = FrameRuntime::from_finished_source(
        source,
        FrameGeometry {
            rows: 8,
            cols: 80,
            content_width: 80,
        },
        LinkMode::Default,
    )
    .unwrap();
    let mut parser = vt100::Parser::new(8.try_into().unwrap(), 80.try_into().unwrap(), 0);
    draw(&mut runtime, &mut parser);
    let second = source.find("# Second").unwrap();
    for _ in 0..10000 {
        let result = runtime
            .restore_navigation_position(
                mdtui_tui::history::HistoryPosition {
                    source: second,
                    row_offset: 0,
                },
                &mut Budget::new(WorkLimit::unlimited()),
            )
            .unwrap();
        if result.is_ready() {
            break;
        }
    }
    draw(&mut runtime, &mut parser);
    assert_eq!(runtime.top_source(), Some(second));
    runtime.resize(8, 100).unwrap();
    parser.set_size(8.try_into().unwrap(), 100.try_into().unwrap());
    let grid = draw(&mut runtime, &mut parser);
    assert_eq!(runtime.top_source(), Some(second), "{grid}");
    assert!(grid.lines().next().unwrap().contains("Second"), "{grid}");
    runtime.open_fold_menu(false);
    draw(&mut runtime, &mut parser);
    runtime.fold_menu_key(KeyCode::Enter);
    draw(&mut runtime, &mut parser);
    runtime.fold_menu_key(KeyCode::Esc);
    draw(&mut runtime, &mut parser);
    assert!(
        runtime
            .viewport()
            .folds()
            .is_collapsed(FoldId::Heading(HeadingId(1)))
    );
    runtime.resize(8, 40).unwrap();
    parser.set_size(8.try_into().unwrap(), 40.try_into().unwrap());
    let grid = draw(&mut runtime, &mut parser);
    assert_eq!(runtime.top_source(), Some(second), "{grid}");
    assert!(!grid.contains("second"), "{grid}");
}

#[test]
fn projected_end_skips_a_million_hidden_unicode_tail_and_fixed_cut_survives_append_resize() {
    use mdtui_tui::render::viewport::{ViewportConfig, ViewportState};
    use mdtui3::{Document, Span};
    let mut doc = Document::new();
    doc.enable_display_shape_effects().unwrap();
    let prefix = "# H\nαβ";
    doc.append(&format!("{prefix}{}", "secret ".repeat(150_000)))
        .unwrap();
    let mut view = ViewportState::new(
        &doc,
        ViewportConfig {
            rows: 5,
            ..Default::default()
        },
    )
    .unwrap();
    let start = "# H\nα".len();
    let id = register(view.folds_mut(), &doc, Span::new(start, doc.source().len()));
    view.fold_changed(&doc);
    view.seek_end(&doc, false).unwrap();
    let frame = ready_view(&mut view, &doc);
    let text = body(&frame);
    assert!(text.contains("[+ source"), "{text}");
    assert!(!text.contains("secret"));
    assert!(frame.stats.planned_tiles <= 8, "{:?}", frame.stats);
    let cut = doc.source().len();
    let update = doc.append("\n\n# New\nNEW_TAIL\n").unwrap();
    view.apply_update(&doc, &update).unwrap();
    view.set_content_width(&doc, 40).unwrap();
    let text = body(&ready_view(&mut view, &doc));
    assert!(!text.contains("NEW_TAIL"), "{text}");
    assert!(view.folds().is_collapsed(id));
    assert_eq!(
        view.folds()
            .fold_candidate(&doc, id)
            .unwrap()
            .unwrap()
            .source
            .end,
        cut
    );
    view.seek_end(&doc, true).unwrap();
    assert!(body(&ready_view(&mut view, &doc)).contains("NEW_TAIL"));
}

#[test]
fn real_menu_deep_batch_zero_micro_cancel_and_restart_preserve_paid_choices() {
    let source = format!("{}body\n", "> ".repeat(600));
    let mut runtime = FrameRuntime::from_finished_source(
        &source,
        FrameGeometry {
            rows: 8,
            cols: 80,
            content_width: 80,
        },
        LinkMode::Default,
    )
    .unwrap();
    let mut parser = vt100::Parser::new(8.try_into().unwrap(), 80.try_into().unwrap(), 0);
    draw(&mut runtime, &mut parser);
    runtime.open_fold_menu(false);
    draw(&mut runtime, &mut parser);
    runtime.fold_menu_key(KeyCode::Right); // document: focus, open, collapse
    runtime.fold_menu_key(KeyCode::Down);
    runtime.fold_menu_key(KeyCode::Down);
    runtime.fold_menu_key(KeyCode::Enter);
    let old = runtime.viewport().folds().version();
    let mut wire = Vec::new();
    runtime
        .frame_to(&mut wire, &mut Budget::new(WorkLimit::default()))
        .unwrap();
    assert_eq!(runtime.viewport().folds().version(), old);
    for _ in 0..100 {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 6,
            tiles: 1,
            recovery: 1,
            dependencies: 1,
        });
        runtime.frame_to(&mut wire, &mut budget).unwrap();
        assert!(
            budget.used().queries <= 1 && budget.used().objects <= 1 && budget.used().bytes <= 6
        );
    }
    assert!(runtime.fold_pending());
    let partial = runtime.viewport().folds().version();
    assert!(partial > old);
    assert!(runtime.fold_menu_key(KeyCode::Esc));
    assert!(!runtime.fold_pending());
    assert_eq!(runtime.viewport().folds().version(), partial);
    runtime.open_fold_menu(false);
    draw(&mut runtime, &mut parser);
    runtime.fold_menu_key(KeyCode::Right);
    runtime.fold_menu_key(KeyCode::Down);
    runtime.fold_menu_key(KeyCode::Down);
    runtime.fold_menu_key(KeyCode::Enter);
    let mut steps = 0;
    while runtime.fold_pending() && steps < 10000 {
        runtime
            .frame_to(
                &mut wire,
                &mut Budget::new(WorkLimit {
                    queries: 1,
                    objects: 1,
                    bytes: 6,
                    tiles: 1,
                    recovery: 1,
                    dependencies: 1,
                }),
            )
            .unwrap();
        steps += 1;
    }
    assert!(!runtime.fold_pending(), "{steps}");
    assert!((500..10000).contains(&steps), "{steps}");
    runtime.fold_menu_key(KeyCode::Esc);
    let grid = draw(&mut runtime, &mut parser);
    assert!(!grid.contains("body"), "{grid}");
    assert_eq!(
        runtime
            .fold_hits()
            .iter()
            .filter(|hit| hit.row == runtime.fold_hits()[0].row)
            .count(),
        1
    );
    let hit = runtime.fold_hits()[0];
    assert_eq!(hit.epoch, runtime.viewport().folds().version());
    assert!(hit.x1 > hit.x0 && hit.hidden_envelope.end == source.len());
}

fn menu_action(
    runtime: &mut FrameRuntime,
    parser: &mut vt100::Parser,
    page: usize,
    action: usize,
) -> String {
    runtime.open_fold_menu(false);
    draw(runtime, parser);
    for _ in 0..page {
        runtime.fold_menu_key(KeyCode::Right);
    }
    for _ in 0..action {
        runtime.fold_menu_key(KeyCode::Down);
    }
    runtime.fold_menu_key(KeyCode::Enter);
    draw(runtime, parser)
}

#[test]
fn two_real_views_new_only_default_and_preview_restore_across_append_and_resize() {
    let geometry = FrameGeometry {
        rows: 12,
        cols: 80,
        content_width: 80,
    };
    let mut a = FrameRuntime::new(geometry, LinkMode::Default).unwrap();
    let mut b = FrameRuntime::new(geometry, LinkMode::Default).unwrap();
    let mut pa = vt100::Parser::new(12.try_into().unwrap(), 80.try_into().unwrap(), 0);
    let mut pb = vt100::Parser::new(12.try_into().unwrap(), 80.try_into().unwrap(), 0);
    append(&mut a, b"# Old\nOLD_BODY\n\n");
    append(&mut b, b"# Old\nOLD_BODY\n\n");
    draw(&mut a, &mut pa);
    draw(&mut b, &mut pb);
    menu_action(&mut a, &mut pa, 5, 0); // h1 defaults
    a.fold_menu_key(KeyCode::Esc);
    append(&mut a, b"# New\nNEW_BODY\n\n");
    append(&mut b, b"# New\nNEW_BODY\n\n");
    let ga = draw(&mut a, &mut pa);
    let gb = draw(&mut b, &mut pb);
    assert!(ga.contains("OLD_BODY") && !ga.contains("NEW_BODY"), "{ga}");
    assert!(gb.contains("OLD_BODY") && gb.contains("NEW_BODY"), "{gb}");
    assert!(
        !a.viewport()
            .folds()
            .is_collapsed(FoldId::Heading(HeadingId(0)))
    );
    assert!(
        a.viewport()
            .folds()
            .is_collapsed(FoldId::Heading(HeadingId(1)))
    );
    // Collapse the old heading, preview it, then change its live range and width.
    menu_action(&mut a, &mut pa, 0, 0);
    assert!(!draw(&mut a, &mut pa).contains("OLD_BODY"));
    a.fold_menu_key(KeyCode::Down);
    a.fold_menu_key(KeyCode::Down);
    a.fold_menu_key(KeyCode::Down);
    a.fold_menu_key(KeyCode::Down); // preview
    a.fold_menu_key(KeyCode::Enter);
    assert!(draw(&mut a, &mut pa).contains("OLD_BODY"));
    a.fold_menu_key(KeyCode::Esc); // close menu; preview persists
    append(&mut a, b"APPENDED\n");
    a.resize(12, 40).unwrap();
    pa.set_size(12.try_into().unwrap(), 40.try_into().unwrap());
    assert!(draw(&mut a, &mut pa).contains("OLD_BODY"));
    a.reading_key(KeyCode::Esc, KeyModifiers::NONE, true)
        .unwrap();
    let ga = draw(&mut a, &mut pa);
    assert!(!ga.contains("OLD_BODY") && !ga.contains("NEW_BODY"), "{ga}");
    assert!(a.viewport().folds().preview().is_none());
    assert!(
        a.viewport()
            .folds()
            .is_collapsed(FoldId::Heading(HeadingId(0)))
    );
    assert!(draw(&mut b, &mut pb).contains("OLD_BODY"));
}

#[test]
fn actual_source_selection_creates_same_start_ranges_then_enumerates_and_removes_exact_id() {
    let mut runtime = FrameRuntime::from_finished_source(
        "abcdefghij\n",
        FrameGeometry {
            rows: 12,
            cols: 80,
            content_width: 80,
        },
        LinkMode::Default,
    )
    .unwrap();
    let mut parser = vt100::Parser::new(12.try_into().unwrap(), 80.try_into().unwrap(), 0);
    draw(&mut runtime, &mut parser);
    runtime
        .reading_key(KeyCode::Char('v'), KeyModifiers::NONE, true)
        .unwrap();
    runtime
        .reading_key(KeyCode::Char(' '), KeyModifiers::NONE, true)
        .unwrap();
    for _ in 0..5 {
        runtime
            .reading_key(KeyCode::Right, KeyModifiers::NONE, true)
            .unwrap();
    }
    assert_eq!(
        runtime.reading().selection.range(),
        Some(mdtui3::Span::new(0, 6))
    );
    menu_action(&mut runtime, &mut parser, 2, 3); // overlay
    runtime.fold_menu_key(KeyCode::Enter); // identical source, separate ID
    draw(&mut runtime, &mut parser);
    runtime.fold_menu_key(KeyCode::Home);
    runtime.fold_menu_key(KeyCode::Enter); // enumerate first overlapping candidate
    draw(&mut runtime, &mut parser);
    runtime.fold_menu_key(KeyCode::Enter); // enumerate second
    let selected = draw(&mut runtime, &mut parser);
    let mut cursor = mdtui3::SourceFoldCandidates::new(
        runtime.document(),
        runtime.viewport().folds(),
        mdtui3::Span::new(0, 6),
    )
    .unwrap();
    let mut ids = Vec::new();
    cursor
        .advance(
            runtime.document(),
            runtime.viewport().folds(),
            &mut Budget::new(WorkLimit::unlimited()),
            &mut ids,
        )
        .unwrap();
    assert_eq!(ids.len(), 2, "{selected}");
    // Range page index 7 removes the currently selected registration, preserving
    // the first ID even though both have the same endpoints.
    for _ in 0..7 {
        runtime.fold_menu_key(KeyCode::Down);
    }
    runtime.fold_menu_key(KeyCode::Enter);
    draw(&mut runtime, &mut parser);
    assert!(
        runtime
            .viewport()
            .folds()
            .fold_candidate(runtime.document(), ids[0].id)
            .unwrap()
            .is_some()
    );
    assert!(
        runtime
            .viewport()
            .folds()
            .fold_candidate(runtime.document(), ids[1].id)
            .unwrap()
            .is_none()
    );
    assert!(runtime.viewport().folds().is_collapsed(ids[0].id));
}

fn append(runtime: &mut FrameRuntime, bytes: &[u8]) {
    if let Some(update) = runtime.append_bytes(bytes).unwrap() {
        runtime.apply_update(&update).unwrap();
    }
}

#[test]
fn all_seven_production_placeholder_families_have_source_and_single_owner_height() {
    use mdtui_tui::render::viewport::{ViewportConfig, ViewportState};
    use mdtui3::{BlockId, ContainerId, Document, Span, TableId, TileId};
    for (source, family, built_in, expected_rows) in [
        (
            "# H\nSECRET\n",
            "heading",
            Some(FoldId::Heading(HeadingId(0))),
            2,
        ),
        (
            "``` rust\nSECRET\n```\n",
            "code",
            Some(FoldId::Code(BlockId(0))),
            1,
        ),
        (
            "$$\nSECRET\n$$\n",
            "math",
            Some(FoldId::Math(BlockId(0))),
            1,
        ),
        (
            "> SECRET\n",
            "container",
            Some(FoldId::Container(ContainerId(0))),
            1,
        ),
        (
            "| SECRET |\n| --- |\n",
            "table",
            Some(FoldId::Table(TableId(0))),
            1,
        ),
        ("SECRET\n", "source", None, 1),
        ("| H |\n| --- |\n| SECRET |\n", "rows", None, 5),
    ] {
        let mut doc = Document::new();
        doc.append(source).unwrap();
        doc.finish();
        let mut view = ViewportState::new(&doc, ViewportConfig::default()).unwrap();
        let id = if let Some(id) = built_in {
            id
        } else if family == "rows" {
            view.folds_mut()
                .insert_table_rows(&doc, TableId(0), 1..2)
                .unwrap()
        } else {
            register(view.folds_mut(), &doc, Span::new(0, source.len()))
        };
        view.folds_mut().set_collapsed(&doc, id, true).unwrap();
        let candidate = view.folds().fold_candidate(&doc, id).unwrap().unwrap();
        view.fold_changed(&doc);
        let frame = ready_view(&mut view, &doc);
        let text = body(&frame);
        assert!(text.contains(&format!("[+ {family}")), "{family}: {text}");
        assert!(!text.contains("SECRET"), "{family}: {text}");
        assert_eq!(text.matches("[+").count(), 1, "{family}: {text}");
        let mapped = frame
            .lines
            .iter()
            .flat_map(|line| &line.cells)
            .filter(|cell| cell.source == Some(candidate.hidden))
            .count();
        assert!(mapped > 1, "{family}: {text}");
        assert_eq!(
            view.heights().height(TileId(0)),
            Some(expected_rows),
            "{family}: {text}"
        );
        assert_eq!(
            frame.lines.len(),
            expected_rows as usize,
            "{family}: {text}"
        );
    }
}

#[test]
fn logical_full_text_search_reveals_hidden_hit_without_resetting_unrelated_choices() {
    let source = "# A\nHIDDEN_HIT\n\n# B\nOTHER_SECRET\n";
    let mut runtime = FrameRuntime::from_finished_source(
        source,
        FrameGeometry {
            rows: 12,
            cols: 80,
            content_width: 80,
        },
        LinkMode::Default,
    )
    .unwrap();
    let mut parser = vt100::Parser::new(12.try_into().unwrap(), 80.try_into().unwrap(), 0);
    draw(&mut runtime, &mut parser);
    menu_action(&mut runtime, &mut parser, 1, 2); // collapse all
    runtime.fold_menu_key(KeyCode::Esc);
    assert!(!draw(&mut runtime, &mut parser).contains("HIDDEN_HIT"));
    runtime
        .reading_key(KeyCode::Char('/'), KeyModifiers::NONE, true)
        .unwrap();
    for ch in "HIDDEN_HIT".chars() {
        runtime
            .reading_key(KeyCode::Char(ch), KeyModifiers::NONE, true)
            .unwrap();
    }
    runtime
        .reading_key(KeyCode::Enter, KeyModifiers::NONE, true)
        .unwrap();
    for _ in 0..10000 {
        runtime.advance_reading(
            &mut Vec::new(),
            &mut Budget::new(WorkLimit {
                queries: 1,
                objects: 1,
                bytes: 6,
                tiles: 1,
                dependencies: 1,
                recovery: 1,
            }),
        );
        runtime
            .frame_to(
                &mut Vec::new(),
                &mut Budget::new(WorkLimit {
                    queries: 1,
                    objects: 1,
                    bytes: 6,
                    tiles: 1,
                    dependencies: 1,
                    recovery: 1,
                }),
            )
            .unwrap();
        if !runtime.has_pending_jump()
            && runtime.reading().current_match().is_some()
            && !runtime.reading().pending()
        {
            break;
        }
    }
    let grid = draw(&mut runtime, &mut parser);
    assert!(grid.contains("HIDDEN_HIT"), "{grid}");
    assert!(
        !runtime
            .viewport()
            .folds()
            .is_collapsed(FoldId::Heading(HeadingId(0)))
    );
    assert!(
        runtime
            .viewport()
            .folds()
            .is_collapsed(FoldId::Heading(HeadingId(1)))
    );
    assert!(!grid.contains("OTHER_SECRET"), "{grid}");
}

#[test]
fn explicit_source_cuts_are_real_payload_barriers_for_clusters_and_current_objects() {
    use mdtui_tui::render::viewport::{ViewportConfig, ViewportState};
    use mdtui3::{Document, Span};
    for source in [
        format!("a{}TAIL\n", "\u{301}".repeat(10000)),
        "🇦🇧🇨🇩🇪🇫TAIL\n".to_owned(),
        "x &NotEqualTilde; TAIL\n".to_owned(),
        "`x SECRET y` TAIL\n".to_owned(),
        "![x SECRET y](https://example.test) TAIL\n".to_owned(),
        "$x SECRET y$ TAIL\n".to_owned(),
        "x[^SECRET] TAIL\n\n[^SECRET]: note\n".to_owned(),
    ] {
        let mut doc = Document::new();
        doc.enable_display_shape_effects().unwrap();
        doc.append(&source).unwrap();
        doc.finish();
        let hidden = if let Some(start) = source.find("SECRET") {
            Span::new(start, start + 6)
        } else if source.starts_with('a') {
            Span::new(3, source.find("TAIL").unwrap())
        } else if source.starts_with('🇦') {
            Span::new(4, 20)
        } else {
            let start = source.find("NotEqual").unwrap();
            Span::new(start, start + 8)
        };
        let mut view = ViewportState::new(&doc, ViewportConfig::default()).unwrap();
        let links = view.shared_links();
        assert!(
            links
                .prepare_shape(&doc, &mut Budget::new(WorkLimit::unlimited()))
                .unwrap()
        );
        let shape = links.shape().unwrap();
        let before = shape.work();
        register(view.folds_mut(), &doc, hidden);
        view.fold_changed(&doc);
        let frame = ready_view(&mut view, &doc);
        let text = body(&frame);
        assert!(!text.contains("SECRET"), "{text}");
        assert!(text.contains("[+ source"), "{text}");
        assert!(text.contains("TAIL"), "{text}");
        for cell in frame.lines.iter().flat_map(|line| &line.cells) {
            if let Some(span) = cell.source {
                assert!(
                    span == hidden || span.end <= hidden.start || span.start >= hidden.end,
                    "cross-boundary cell {span:?}, hidden {hidden:?}: {text}"
                );
            }
        }
        assert_eq!(
            before.source_decode_bytes,
            shape.work().source_decode_bytes,
            "hidden fragment caused source decode"
        );
    }
}

#[test]
fn independent_byte_mask_union_placeholder_grid_and_height_oracle_across_tiles() {
    use mdtui_tui::render::viewport::{ViewportConfig, ViewportState};
    use mdtui3::{Document, Span, TileId};
    for seed in 0..32usize {
        let source: String = (0..900)
            .map(|n| char::from(b'a' + (n % 26) as u8))
            .collect();
        let mut doc = Document::new();
        doc.append(&source).unwrap();
        doc.finish();
        let mut view = ViewportState::new(
            &doc,
            ViewportConfig {
                width: 4096,
                rows: 20,
                ..Default::default()
            },
        )
        .unwrap();
        let mut ranges = vec![
            Span::new(252, 258),
            Span::new(254, 270),
            Span::new(270, 280),
            Span::new(252, 256),
        ];
        for n in 0..12 {
            let start = (seed * 37 + n * 71) % 880;
            ranges.push(Span::new(
                start,
                (start + 3 + (seed * 7 + n * 11) % 45).min(900),
            ));
        }
        let mut hidden = vec![false; source.len()];
        for span in &ranges {
            hidden[span.range()].fill(true);
            register(view.folds_mut(), &doc, *span);
        }
        ranges.sort_by_key(|s| (s.start, s.end));
        let mut unions: Vec<Span> = Vec::new();
        for span in ranges {
            if let Some(last) = unions.last_mut().filter(|last| span.start < last.end) {
                last.end = last.end.max(span.end);
            } else {
                unions.push(span);
            }
        }
        view.fold_changed(&doc);
        let frame = ready_view(&mut view, &doc);
        let mut seen = vec![false; source.len()];
        let mut placeholders = std::collections::BTreeSet::new();
        for cell in frame.lines.iter().flat_map(|line| &line.cells) {
            if let Some(span) = cell.source {
                if let Some(union) = unions.iter().find(|union| **union == span) {
                    placeholders.insert((union.start, union.end));
                } else {
                    assert_eq!(span.len(), 1, "{seed}: {span:?}");
                    assert!(!hidden[span.start], "hidden source cell {seed}: {span:?}");
                    assert_eq!(cell.text.collect_text(), &source[span.range()]);
                    assert!(!seen[span.start], "duplicate source cell {seed}: {span:?}");
                    seen[span.start] = true;
                }
            }
        }
        assert_eq!(
            seen,
            hidden.iter().map(|h| !h).collect::<Vec<_>>(),
            "seed {seed}: {}",
            body(&frame)
        );
        assert_eq!(
            placeholders,
            unions.iter().map(|u| (u.start, u.end)).collect()
        );
        assert_eq!(frame.lines.len(), 1, "seed {seed}");
        let rows: u64 = (0..doc.tile_count())
            .map(|n| view.heights().height(TileId(n)).unwrap())
            .sum();
        assert_eq!(rows, 1, "seed {seed}");
    }
}

#[test]
fn micro_history_restore_continues_automatically_and_escape_retains_revealed_choices() {
    use mdtui_tui::{history::HistoryPosition, render::frame::HistoryRestoreStatus};
    use mdtui3::ContainerId;
    let source = format!("{}# Target\n", "> ".repeat(600));
    let mut runtime = FrameRuntime::from_finished_source(
        &source,
        FrameGeometry {
            rows: 12,
            cols: 80,
            content_width: 80,
        },
        LinkMode::Default,
    )
    .unwrap();
    let mut parser = vt100::Parser::new(12.try_into().unwrap(), 80.try_into().unwrap(), 0);
    draw(&mut runtime, &mut parser);
    menu_action(&mut runtime, &mut parser, 1, 2);
    runtime.fold_menu_key(KeyCode::Esc);
    draw(&mut runtime, &mut parser);
    let target = HistoryPosition {
        source: source.find("Target").unwrap(),
        row_offset: 0,
    };
    let tiny = || {
        Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 6,
            tiles: 1,
            recovery: 1,
            dependencies: 1,
        })
    };
    assert_eq!(
        runtime
            .restore_navigation_position(target, &mut tiny())
            .unwrap(),
        HistoryRestoreStatus::BudgetExhausted
    );
    for _ in 0..100 {
        runtime.frame_to(&mut Vec::new(), &mut tiny()).unwrap();
    }
    assert!(runtime.has_pending_jump());
    let opened = (0..600)
        .filter(|&n| {
            !runtime
                .viewport()
                .folds()
                .is_collapsed(FoldId::Container(ContainerId(n)))
        })
        .count();
    assert!(opened > 0 && opened < 600, "{opened}");
    runtime
        .reading_key(KeyCode::Esc, KeyModifiers::NONE, true)
        .unwrap();
    assert!(!runtime.has_pending_jump());
    assert!(runtime.history_restore_result().is_none());
    assert_eq!(
        (0..600)
            .filter(|&n| !runtime
                .viewport()
                .folds()
                .is_collapsed(FoldId::Container(ContainerId(n))))
            .count(),
        opened
    );
    runtime
        .restore_navigation_position(target, &mut tiny())
        .unwrap();
    let mut frames = 0;
    while runtime.has_pending_jump() && frames < 20000 {
        runtime.frame_to(&mut Vec::new(), &mut tiny()).unwrap();
        frames += 1;
    }
    assert!(!runtime.has_pending_jump(), "{frames}");
    assert!(
        runtime
            .history_restore_result()
            .is_some_and(|(_, status)| status.is_ready())
    );
    assert!(draw(&mut runtime, &mut parser).contains("Target"));
}

#[test]
fn folded_clipped_table_search_and_hidden_definition_targets_terminate_without_false_hits() {
    use mdtui_tui::{history::HistoryPosition, render::frame::HistoryRestoreStatus};
    use mdtui3::Span;
    let source = format!("| H |\n| --- |\n| {}HIT |\n| OTHER |\n", "a".repeat(8000));
    let mut runtime = FrameRuntime::from_finished_source(
        &source,
        FrameGeometry {
            rows: 12,
            cols: 40,
            content_width: 40,
        },
        LinkMode::Default,
    )
    .unwrap();
    let mut parser = vt100::Parser::new(12.try_into().unwrap(), 40.try_into().unwrap(), 0);
    draw(&mut runtime, &mut parser);
    {
        let (doc, view) = runtime.document_and_viewport_mut();
        let hit = source.find("HIT").unwrap();
        register(view.folds_mut(), doc, Span::new(hit, hit + 3));
        let other = source.find("OTHER").unwrap();
        register(view.folds_mut(), doc, Span::new(other, other + 5));
        view.fold_changed(doc);
    }
    runtime
        .reading_key(KeyCode::Char('/'), KeyModifiers::NONE, true)
        .unwrap();
    for ch in "HIT".chars() {
        runtime
            .reading_key(KeyCode::Char(ch), KeyModifiers::NONE, true)
            .unwrap();
    }
    runtime
        .reading_key(KeyCode::Enter, KeyModifiers::NONE, true)
        .unwrap();
    let mut settled = false;
    for _ in 0..10000 {
        runtime.advance_reading(
            &mut Vec::new(),
            &mut Budget::new(mdtui_tui::stream::FRAME_WORK),
        );
        runtime
            .frame_to(
                &mut Vec::new(),
                &mut Budget::new(mdtui_tui::stream::FRAME_WORK),
            )
            .unwrap();
        if !runtime.has_pending_jump() && !runtime.reading().pending() {
            settled = true;
            break;
        }
    }
    assert!(settled);
    draw(&mut runtime, &mut parser);
    assert!(runtime.reading().current_match().is_some());
    assert!(
        runtime
            .reading()
            .status()
            .unwrap()
            .contains("match not drawn")
    );
    let mut runtime = FrameRuntime::from_finished_source(
        "[name]: /hidden\n",
        FrameGeometry {
            rows: 8,
            cols: 40,
            content_width: 40,
        },
        LinkMode::Default,
    )
    .unwrap();
    let (doc, view) = runtime.document_and_viewport_mut();
    register(view.folds_mut(), doc, Span::new(0, doc.source().len()));
    view.fold_changed(doc);
    let mut result = HistoryRestoreStatus::BudgetExhausted;
    for _ in 0..10000 {
        result = runtime
            .restore_navigation_position(
                HistoryPosition {
                    source: 2,
                    row_offset: 0,
                },
                &mut Budget::new(WorkLimit {
                    queries: 1,
                    objects: 1,
                    bytes: 6,
                    tiles: 1,
                    recovery: 1,
                    dependencies: 1,
                }),
            )
            .unwrap();
        if result != HistoryRestoreStatus::BudgetExhausted {
            break;
        }
    }
    assert_eq!(result, HistoryRestoreStatus::NotFound);
    assert!(!runtime.has_pending_jump());
}

#[test]
fn actual_grouped_commands_cover_subtree_siblings_strict_levels_focus_and_task_filters() {
    use mdtui3::ContainerId;
    let source = "# A\na\n\n## Child\nchild\n\n### Grand\ngrand\n\n# B\nb\n\n## Peer\npeer\n";
    let mut runtime = FrameRuntime::from_finished_source(
        source,
        FrameGeometry {
            rows: 24,
            cols: 100,
            content_width: 100,
        },
        LinkMode::Default,
    )
    .unwrap();
    let mut parser = vt100::Parser::new(24.try_into().unwrap(), 100.try_into().unwrap(), 0);
    draw(&mut runtime, &mut parser);
    let state = |runtime: &FrameRuntime| {
        (0..5)
            .map(|n| {
                runtime
                    .viewport()
                    .folds()
                    .is_collapsed(FoldId::Heading(HeadingId(n)))
            })
            .collect::<Vec<_>>()
    };
    menu_action(&mut runtime, &mut parser, 0, 8); // collapse subtree A
    assert_eq!(state(&runtime), [true, true, true, false, false]);
    menu_action(&mut runtime, &mut parser, 0, 9); // siblings exclude target and descendants
    assert_eq!(state(&runtime), [true, true, true, false, false]);
    menu_action(&mut runtime, &mut parser, 0, 7); // open subtree
    assert_eq!(state(&runtime), [false, false, false, false, false]);
    menu_action(&mut runtime, &mut parser, 3, 3); // only h2 collapse
    assert_eq!(state(&runtime), [false, true, false, false, true]);
    menu_action(&mut runtime, &mut parser, 1, 0); // focus A
    assert_eq!(state(&runtime), [false, true, true, true, true]);
    menu_action(&mut runtime, &mut parser, 0, 10); // collapse siblings, preserve target
    assert_eq!(state(&runtime), [false, true, true, true, true]);
    menu_action(&mut runtime, &mut parser, 1, 1); // all open
    assert_eq!(state(&runtime), [false, false, false, false, false]);
    let mut runtime = FrameRuntime::from_finished_source(
        "> quote\n\n- [x] DONE\n- [ ] TODO\n",
        FrameGeometry {
            rows: 24,
            cols: 100,
            content_width: 100,
        },
        LinkMode::Default,
    )
    .unwrap();
    let mut parser = vt100::Parser::new(24.try_into().unwrap(), 100.try_into().unwrap(), 0);
    draw(&mut runtime, &mut parser);
    menu_action(&mut runtime, &mut parser, 4, 7); // completed tasks collapse
    for n in 0..runtime.document().container_count() {
        let id = ContainerId(n);
        assert_eq!(
            runtime
                .viewport()
                .folds()
                .is_collapsed(FoldId::Container(id)),
            runtime
                .viewport()
                .folds()
                .task_checked(runtime.document(), id)
                .unwrap()
                == Some(true)
        );
    }
    menu_action(&mut runtime, &mut parser, 4, 9); // incomplete tasks collapse
    for n in 0..runtime.document().container_count() {
        let id = ContainerId(n);
        assert_eq!(
            runtime
                .viewport()
                .folds()
                .is_collapsed(FoldId::Container(id)),
            runtime
                .viewport()
                .folds()
                .task_checked(runtime.document(), id)
                .unwrap()
                .is_some()
        );
    }
    let text = menu_action(&mut runtime, &mut parser, 4, 1); // quotes collapse too
    assert!(!text.contains("quote") || text.contains("quotes: collapse"));
}

#[test]
fn actual_selection_strict_live_eof_seal_and_fixed_rows_keep_their_original_identity() {
    let geometry = FrameGeometry {
        rows: 16,
        cols: 80,
        content_width: 80,
    };
    let mut runtime =
        FrameRuntime::from_finished_source("abcdef\n\n", geometry, LinkMode::Default).unwrap();
    let mut parser = vt100::Parser::new(16.try_into().unwrap(), 80.try_into().unwrap(), 0);
    draw(&mut runtime, &mut parser);
    runtime
        .reading_key(KeyCode::Char('v'), KeyModifiers::NONE, true)
        .unwrap();
    runtime
        .reading_key(KeyCode::Char(' '), KeyModifiers::NONE, true)
        .unwrap();
    runtime
        .reading_key(KeyCode::Right, KeyModifiers::NONE, true)
        .unwrap();
    runtime
        .reading_key(KeyCode::Right, KeyModifiers::NONE, true)
        .unwrap();
    let strict = menu_action(&mut runtime, &mut parser, 2, 4);
    assert!(strict.contains("[+ source"), "{strict}");
    let mut runtime = FrameRuntime::new(geometry, LinkMode::Default).unwrap();
    append(&mut runtime, b"abcdef\n");
    let mut parser = vt100::Parser::new(16.try_into().unwrap(), 80.try_into().unwrap(), 0);
    draw(&mut runtime, &mut parser);
    runtime
        .reading_key(KeyCode::Char('v'), KeyModifiers::NONE, true)
        .unwrap();
    menu_action(&mut runtime, &mut parser, 2, 5); // stream from selected source
    runtime.fold_menu_key(KeyCode::Esc);
    append(&mut runtime, b"GROWING\n");
    let text = draw(&mut runtime, &mut parser);
    assert!(!text.contains("GROWING"), "{text}");
    menu_action(&mut runtime, &mut parser, 2, 0); // enumerate same ID
    for _ in 0..6 {
        runtime.fold_menu_key(KeyCode::Down);
    }
    runtime.fold_menu_key(KeyCode::Enter); // seal
    draw(&mut runtime, &mut parser);
    runtime.fold_menu_key(KeyCode::Esc);
    append(&mut runtime, b"AFTER_SEAL\n");
    assert!(draw(&mut runtime, &mut parser).contains("AFTER_SEAL"));
    let mut runtime = FrameRuntime::new(geometry, LinkMode::Default).unwrap();
    append(&mut runtime, b"| H |\n| --- |\n| SECRET |\n");
    let mut parser = vt100::Parser::new(16.try_into().unwrap(), 80.try_into().unwrap(), 0);
    draw(&mut runtime, &mut parser);
    runtime
        .reading_key(KeyCode::Char('v'), KeyModifiers::NONE, true)
        .unwrap();
    runtime
        .reading_key(KeyCode::Down, KeyModifiers::NONE, true)
        .unwrap();
    let text = menu_action(&mut runtime, &mut parser, 2, 8); // selected fixed rows
    assert!(text.contains("[+ rows"), "{text}");
    runtime.fold_menu_key(KeyCode::Esc);
    append(&mut runtime, b"| NEW_ROW |\n");
    let text = draw(&mut runtime, &mut parser);
    assert!(
        text.contains("NEW_ROW") && !text.contains("SECRET"),
        "{text}"
    );
}

#[test]
fn hidden_body_render_counters_and_owned_capacities_are_bounded() {
    let mib = std::env::var("MDTUI_026_FOLD_MEASURE_MIB")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(1);
    assert!((1..=64).contains(&mib));
    let started = std::time::Instant::now();
    let source = format!("# H\n{}\n", "x".repeat(mib * 1024 * 1024));
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
    runtime.open_fold_menu(false);
    draw(&mut runtime, &mut parser);
    let shape = runtime.viewport().shared_links().shape().unwrap();
    let shape_before = shape.work();
    let core_before = runtime.document().work();
    let links_before = runtime.viewport().shared_links().stats();
    runtime.fold_menu_key(KeyCode::Enter);
    let render_start = std::time::Instant::now();
    let mut total = WorkLimit::default();
    let mut wire = Vec::new();
    let mut peak_tiles = 0;
    let mut frames = 0;
    loop {
        let mut budget = Budget::new(mdtui_tui::stream::FRAME_WORK);
        let commit = runtime.frame_to(&mut wire, &mut budget).unwrap();
        let used = budget.used();
        total.queries += used.queries;
        total.objects += used.objects;
        total.bytes += used.bytes;
        total.tiles += used.tiles;
        total.dependencies += used.dependencies;
        total.recovery += used.recovery;
        peak_tiles = peak_tiles.max(commit.metrics.planned_tiles);
        frames += 1;
        assert!(frames < 10000);
        if commit.committed && !runtime.fold_pending() {
            break;
        }
    }
    parser.process(&wire);
    assert!(parser.screen().contents().contains("[+ heading"));
    assert_eq!(
        shape.work().source_decode_bytes,
        shape_before.source_decode_bytes
    );
    assert_eq!(
        runtime.document().work().input_bytes,
        core_before.input_bytes
    );
    assert_eq!(
        shape.work().classified_scalars,
        shape_before.classified_scalars
    );
    assert_eq!(
        shape.work().input_node_visits,
        shape_before.input_node_visits
    );
    let visible_source_bytes = total.bytes.saturating_sub(wire.len());
    assert!(
        visible_source_bytes <= 16,
        "only the four-byte visible heading may be probed: {total:?}, wire {}",
        wire.len()
    );
    assert!(
        peak_tiles <= 3 && total.recovery == 0,
        "{peak_tiles} {total:?}"
    );
    let links = runtime.viewport().shared_links().stats();
    assert!(links.presentation_entries <= links_before.presentation_entries + 1);
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let memory: Vec<_> = status
        .lines()
        .filter(|line| line.starts_with("VmRSS:") || line.starts_with("VmHWM:"))
        .collect();
    eprintln!(
        "026 hidden measurement: source={} frames={frames} layout={:?} total={:?} wire={} budget={total:?} planned_tiles={peak_tiles} shape_capacity={} checkpoint={:?} resolver_before={links_before:?} resolver_after={links:?} memory={memory:?}",
        source.len(),
        render_start.elapsed(),
        started.elapsed(),
        wire.len(),
        shape.capacity_bytes(),
        runtime.viewport().checkpoints().arena_stats()
    );
    runtime.fold_menu_key(KeyCode::Esc);
    draw(&mut runtime, &mut parser);
    runtime
        .reading_key(KeyCode::Char('v'), KeyModifiers::NONE, true)
        .unwrap();
    runtime
        .reading_key(KeyCode::Down, KeyModifiers::NONE, true)
        .unwrap();
    assert!(runtime.reading().selection.range().unwrap().len() > 64 * 1024);
    runtime
        .reading_key(KeyCode::Char('y'), KeyModifiers::NONE, true)
        .unwrap();
    let mut copy = Vec::new();
    for _ in 0..1000 {
        runtime.advance_reading(&mut copy, &mut Budget::new(mdtui_tui::stream::FRAME_WORK));
        if !runtime.reading().copy_pending() {
            break;
        }
    }
    assert!(!runtime.reading().copy_pending());
    assert!(copy.starts_with(b"\x1b]52;c;") && copy.len() < 256);
    runtime
        .reading_key(KeyCode::Char('Y'), KeyModifiers::NONE, true)
        .unwrap();
    assert!(!runtime.reading().copy_pending());
    assert!(runtime.reading().message().contains("64 KiB"));
    eprintln!(
        "026 hidden visible copy: packet={} raw Y refused",
        copy.len()
    );
}

#[test]
fn folded_search_next_previous_reveals_each_target_and_retains_opened_paths() {
    let source = "# A\nHIT\n\n# B\nHIT\n\n# C\nNEVER\n";
    let mut runtime = FrameRuntime::from_finished_source(
        source,
        FrameGeometry {
            rows: 12,
            cols: 80,
            content_width: 80,
        },
        LinkMode::Default,
    )
    .unwrap();
    let mut parser = vt100::Parser::new(12.try_into().unwrap(), 80.try_into().unwrap(), 0);
    draw(&mut runtime, &mut parser);
    menu_action(&mut runtime, &mut parser, 1, 2);
    runtime.fold_menu_key(KeyCode::Esc);
    runtime
        .reading_key(KeyCode::Char('/'), KeyModifiers::NONE, true)
        .unwrap();
    for ch in "HIT".chars() {
        runtime
            .reading_key(KeyCode::Char(ch), KeyModifiers::NONE, true)
            .unwrap();
    }
    runtime
        .reading_key(KeyCode::Enter, KeyModifiers::NONE, true)
        .unwrap();
    for (key, expected) in [
        (None, source.find("HIT").unwrap()),
        (Some('n'), source.rfind("HIT").unwrap()),
        (Some('N'), source.find("HIT").unwrap()),
    ] {
        if let Some(key) = key {
            runtime
                .reading_key(KeyCode::Char(key), KeyModifiers::NONE, true)
                .unwrap();
        }
        let mut ready = false;
        for _ in 0..10000 {
            runtime.advance_reading(
                &mut Vec::new(),
                &mut Budget::new(mdtui_tui::stream::FRAME_WORK),
            );
            let mut bytes = Vec::new();
            runtime
                .frame_to(&mut bytes, &mut Budget::new(mdtui_tui::stream::FRAME_WORK))
                .unwrap();
            parser.process(&bytes);
            if !runtime.has_pending_jump() && !runtime.reading().pending() {
                ready = true;
                break;
            }
        }
        assert!(ready);
        assert_eq!(
            runtime.reading().current_match().unwrap().source.start,
            expected
        );
        let text = draw(&mut runtime, &mut parser);
        assert!(text.lines().next().unwrap().contains("HIT"), "{text}");
        assert!(
            runtime
                .viewport()
                .folds()
                .is_collapsed(FoldId::Heading(HeadingId(2)))
        );
    }
    assert!(
        !runtime
            .viewport()
            .folds()
            .is_collapsed(FoldId::Heading(HeadingId(0)))
    );
    assert!(
        !runtime
            .viewport()
            .folds()
            .is_collapsed(FoldId::Heading(HeadingId(1)))
    );
}

#[test]
fn automatic_paragraph_backlog_keeps_both_updates_and_setext_retyping_removes_old_source_fold() {
    let geometry = FrameGeometry {
        rows: 24,
        cols: 80,
        content_width: 80,
    };
    let mut runtime = FrameRuntime::new(geometry, LinkMode::Default).unwrap();
    let mut parser = vt100::Parser::new(24.try_into().unwrap(), 80.try_into().unwrap(), 0);
    draw(&mut runtime, &mut parser);
    menu_action(&mut runtime, &mut parser, 5, 15);
    runtime.fold_menu_key(KeyCode::Esc);
    append(&mut runtime, format!("{}\n\n", "a".repeat(5000)).as_bytes());
    append(&mut runtime, format!("{}\n\n", "b".repeat(5000)).as_bytes());
    let text = draw(&mut runtime, &mut parser);
    assert_eq!(text.matches("[+ source").count(), 2, "{text}");
    let mut runtime = FrameRuntime::new(geometry, LinkMode::Default).unwrap();
    let mut parser = vt100::Parser::new(24.try_into().unwrap(), 80.try_into().unwrap(), 0);
    draw(&mut runtime, &mut parser);
    menu_action(&mut runtime, &mut parser, 5, 15);
    runtime.fold_menu_key(KeyCode::Esc);
    append(&mut runtime, format!("{}\n", "a".repeat(5000)).as_bytes());
    assert!(draw(&mut runtime, &mut parser).contains("[+ source"));
    append(&mut runtime, b"---\n");
    let text = draw(&mut runtime, &mut parser);
    assert!(!text.contains("[+ source"), "{text}");
    assert_eq!(runtime.document().headings().entries().len(), 1);
}

#[test]
fn hundred_thousand_column_partial_table_pan_prepares_only_selected_cells_and_real_row_owners() {
    use mdtui_tui::render::viewport::{ViewportConfig, ViewportState};
    use mdtui3::{Document, TableId};
    let columns = 100_000;
    let source = format!(
        "{}|LAST_HEADER|\n{}|\n{}|LAST_VISIBLE|\n{}|HIDDEN_ROW|\n{}|AFTER_ROW|\n",
        "|h".repeat(columns - 1),
        "|---".repeat(columns),
        "|a".repeat(columns - 1),
        "|b".repeat(columns - 1),
        "|c".repeat(columns - 1)
    );
    let mut doc = Document::new();
    doc.enable_display_shape_effects().unwrap();
    doc.append(&source).unwrap();
    doc.finish();
    let mut view = ViewportState::new(&doc, ViewportConfig::default()).unwrap();
    let links = view.shared_links();
    assert!(
        links
            .prepare_shape(&doc, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap()
    );
    let shape = links.shape().unwrap();
    let before = shape.work();
    let range = view
        .folds_mut()
        .insert_table_rows(&doc, TableId(0), 2..3)
        .unwrap();
    view.folds_mut().set_collapsed(&doc, range, true).unwrap();
    view.fold_changed(&doc);
    view.set_table_first_column(&doc, TableId(0), columns - 1)
        .unwrap();
    let frame = ready_view(&mut view, &doc);
    let text = body(&frame);
    assert!(
        text.contains("LAST_HEADER") && text.contains("LAST_VISIBLE") && text.contains("AFTER_ROW"),
        "{text}"
    );
    assert!(!text.contains("HIDDEN_ROW"), "{text}");
    assert_eq!(text.matches("[+ rows").count(), 1, "{text}");
    assert_eq!(text.matches('┌').count(), 1, "{text}");
    assert_eq!(text.matches('└').count(), 1, "{text}");
    assert!(
        frame.stats.planned_tiles <= 6 && frame.stats.layout_budget.bytes < 16384,
        "{:?}",
        frame.stats
    );
    assert_eq!(shape.work().source_decode_bytes, before.source_decode_bytes);
    assert!(view.table_widths().work().column_metadata_reads <= 4);
    let expected = doc.table(TableId(0)).unwrap().row(3).unwrap().source.start;
    let tile = doc.tile_at(expected).unwrap();
    assert!(view.heights().height(tile).unwrap() > 0);
}

#[test]
fn two_production_views_of_one_document_keep_choices_defaults_ranges_and_heights_independent() {
    use mdtui_tui::render::viewport::{ViewportConfig, ViewportState};
    use mdtui3::{Document, Span};
    let mut doc = Document::new();
    doc.enable_display_shape_effects().unwrap();
    doc.append("# Old\nOLD_BODY\n\n").unwrap();
    let mut a = ViewportState::new(&doc, ViewportConfig::default()).unwrap();
    let mut b = ViewportState::new(&doc, ViewportConfig::default()).unwrap();
    ready_view(&mut a, &doc);
    ready_view(&mut b, &doc);
    let range = register(a.folds_mut(), &doc, Span::new(6, 14));
    a.fold_changed(&doc);
    let mut defaults = a.folds().defaults();
    defaults.heading_levels[0] = true;
    a.folds_mut().set_defaults(defaults);
    let update = doc.append("# New\nNEW_BODY\n").unwrap();
    a.apply_update(&doc, &update).unwrap();
    b.apply_update(&doc, &update).unwrap();
    let ga = body(&ready_view(&mut a, &doc));
    let gb = body(&ready_view(&mut b, &doc));
    assert!(ga.contains("[+ source") && !ga.contains("NEW_BODY"), "{ga}");
    assert!(gb.contains("OLD_BODY") && gb.contains("NEW_BODY"), "{gb}");
    assert!(a.folds().fold_candidate(&doc, range).unwrap().is_some());
    assert!(b.folds().fold_candidate(&doc, range).unwrap().is_none());
    a.set_content_width(&doc, 40).unwrap();
    assert!(a.folds().is_collapsed(range));
    assert_eq!(b.config().width, 80);
    assert!(body(&ready_view(&mut b, &doc)).contains("OLD_BODY"));
}

#[test]
fn whole_table_summary_uses_two_indexed_rows_and_copy_keeps_only_one_minimal_placeholder() {
    let source =
        "| HEAD_A | HEAD_B |\n| --- | --- |\n| BODY_ONE | VALUE |\n| NEVER_SUMMARY | HIDDEN |\n";
    let mut runtime = FrameRuntime::from_finished_source(
        source,
        FrameGeometry {
            rows: 16,
            cols: 80,
            content_width: 80,
        },
        LinkMode::Default,
    )
    .unwrap();
    let mut parser = vt100::Parser::new(16.try_into().unwrap(), 80.try_into().unwrap(), 0);
    draw(&mut runtime, &mut parser);
    menu_action(&mut runtime, &mut parser, 0, 0);
    for _ in 0..6 {
        runtime.fold_menu_key(KeyCode::Down);
    }
    runtime.fold_menu_key(KeyCode::Enter);
    let grid = draw(&mut runtime, &mut parser);
    assert!(
        grid.contains("HEAD_A") && grid.contains("BODY_ONE"),
        "{grid}"
    );
    assert!(!grid.contains("NEVER_SUMMARY"), "{grid}");
    assert_eq!(
        runtime.viewport().heights().height(mdtui3::TileId(0)),
        Some(3)
    );
    runtime.fold_menu_key(KeyCode::Esc);
    draw(&mut runtime, &mut parser);
    runtime
        .reading_key(KeyCode::Char('v'), KeyModifiers::NONE, true)
        .unwrap();
    runtime
        .reading_key(KeyCode::Char('y'), KeyModifiers::NONE, true)
        .unwrap();
    let mut wire = Vec::new();
    for _ in 0..10000 {
        runtime.advance_reading(
            &mut wire,
            &mut Budget::new(WorkLimit {
                queries: 1,
                objects: 1,
                bytes: 6,
                tiles: 1,
                recovery: 1,
                dependencies: 1,
            }),
        );
        if !runtime.reading().copy_pending() {
            break;
        }
    }
    assert!(!runtime.reading().copy_pending());
    assert!(wire.starts_with(b"\x1b]52;c;"));
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let (mut bits, mut count) = (0u32, 0usize);
    let mut decoded = Vec::new();
    for byte in wire[7..wire.len() - 2].iter().take_while(|b| **b != b'=') {
        bits = (bits << 6) | alphabet.iter().position(|b| b == byte).unwrap() as u32;
        count += 6;
        if count >= 8 {
            count -= 8;
            decoded.push((bits >> count) as u8);
        }
    }
    let text = String::from_utf8(decoded).unwrap();
    assert_eq!(text.matches("[+ table").count(), 1, "{text}");
    assert!(
        !text.contains("HEAD_A") && !text.contains("BODY_ONE"),
        "{text}"
    );
    runtime
        .reading_key(KeyCode::Char('Y'), KeyModifiers::NONE, true)
        .unwrap();
    assert!(runtime.reading().copy_pending());
}

#[test]
fn inline_placeholder_row_belongs_to_its_global_start_and_survives_repeated_frame_and_resize() {
    use mdtui_tui::render::viewport::{ViewportConfig, ViewportState};
    use mdtui3::{Document, Span};
    let source = format!("a{}TAIL\n", "x".repeat(100_000));
    let mut doc = Document::new();
    doc.enable_display_shape_effects().unwrap();
    doc.append(&source).unwrap();
    doc.finish();
    let mut view = ViewportState::new(&doc, ViewportConfig::default()).unwrap();
    let hidden = Span::new(1, 100_001);
    register(view.folds_mut(), &doc, hidden);
    view.fold_changed(&doc);
    let frame = ready_view(&mut view, &doc);
    let text = body(&frame);
    assert!(
        text.contains("[+ source") && text.contains("TAIL"),
        "{text}"
    );
    let owner = doc.tile_at(hidden.start).unwrap();
    assert_eq!(
        view.heights().height(owner),
        Some(1),
        "PH row must belong to global start, not a distant suffix"
    );
    assert_eq!(body(&ready_view(&mut view, &doc)), text);
    view.set_content_width(&doc, 60).unwrap();
    let text = body(&ready_view(&mut view, &doc));
    assert!(
        text.contains("[+ source") && text.contains("TAIL"),
        "{text}"
    );
    assert_eq!(view.heights().height(owner), Some(1));
}

#[test]
fn live_source_range_from_a_real_unclassified_tail_has_a_drawn_placeholder() {
    let mut runtime = FrameRuntime::new(
        FrameGeometry {
            rows: 12,
            cols: 80,
            content_width: 80,
        },
        LinkMode::Default,
    )
    .unwrap();
    append(&mut runtime, b"pending tail without LF");
    let mut parser = vt100::Parser::new(12.try_into().unwrap(), 80.try_into().unwrap(), 0);
    let before = draw(&mut runtime, &mut parser);
    assert!(before.contains("pending"), "{before}");
    runtime
        .reading_key(KeyCode::Char('v'), KeyModifiers::NONE, true)
        .unwrap();
    let text = menu_action(&mut runtime, &mut parser, 2, 5);
    assert!(text.contains("[+ source"), "{text}");
    assert!(!text.contains("without LF"), "{text}");
}

#[test]
fn a_visible_giant_cluster_before_a_fold_does_not_hide_the_following_placeholder() {
    use mdtui_tui::render::viewport::{ViewportConfig, ViewportState};
    use mdtui3::{Document, Span};
    let prefix = format!("a{}", "\u{301}".repeat(50000));
    let source = format!("{prefix}{}TAIL\n", "x".repeat(100000));
    let mut doc = Document::new();
    doc.enable_display_shape_effects().unwrap();
    doc.append(&source).unwrap();
    doc.finish();
    let mut view = ViewportState::new(&doc, ViewportConfig::default()).unwrap();
    let links = view.shared_links();
    assert!(
        links
            .prepare_shape(&doc, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap()
    );
    let shape = links.shape().unwrap();
    let before = shape.work();
    register(
        view.folds_mut(),
        &doc,
        Span::new(prefix.len(), prefix.len() + 100000),
    );
    view.fold_changed(&doc);
    let frame = ready_view(&mut view, &doc);
    let text = body(&frame);
    assert!(
        text.contains("[+ source") && text.contains("TAIL"),
        "length {}, suffix {:?}",
        text.len(),
        &text[text.len().saturating_sub(100)..]
    );
    assert_eq!(frame.lines[0].cells[0].text.len(), prefix.len());
    assert!(frame.stats.planned_tiles <= 5, "{:?}", frame.stats);
    assert_eq!(shape.work().source_decode_bytes, before.source_decode_bytes);
}

#[test]
fn a_visible_suffix_jump_recovers_its_folded_logical_row_without_replaying_the_hidden_gap() {
    use mdtui_tui::{history::HistoryPosition, render::frame::HistoryRestoreStatus};
    use mdtui3::Span;
    let source = format!("a{}TAIL\n", "x".repeat(100000));
    let mut runtime = FrameRuntime::from_finished_source(
        &source,
        FrameGeometry {
            rows: 12,
            cols: 80,
            content_width: 80,
        },
        LinkMode::Default,
    )
    .unwrap();
    let mut parser = vt100::Parser::new(12.try_into().unwrap(), 80.try_into().unwrap(), 0);
    let id;
    {
        let (doc, view) = runtime.document_and_viewport_mut();
        id = register(view.folds_mut(), doc, Span::new(1, 100001));
        view.fold_changed(doc);
    }
    draw(&mut runtime, &mut parser);
    let result = runtime
        .restore_navigation_position(
            HistoryPosition {
                source: 100001,
                row_offset: 0,
            },
            &mut Budget::new(WorkLimit::unlimited()),
        )
        .unwrap();
    assert!(matches!(result, HistoryRestoreStatus::Ready { .. }));
    assert_eq!(runtime.top_source(), Some(0));
    assert!(runtime.viewport().folds().is_collapsed(id));
    let text = draw(&mut runtime, &mut parser);
    assert!(
        text.contains("[+ source") && text.contains("TAIL"),
        "{text}"
    );
}

#[test]
fn inline_code_placeholder_has_one_global_start_row_through_resize() {
    use mdtui_tui::render::viewport::{ViewportConfig, ViewportState};
    use mdtui3::{Document, Span};
    let source = format!("```rust\na{}TAIL\n```\n", "x".repeat(100000));
    let mut doc = Document::new();
    doc.enable_display_shape_effects().unwrap();
    doc.append(&source).unwrap();
    doc.finish();
    let mut view = ViewportState::new(&doc, ViewportConfig::default()).unwrap();
    register(view.folds_mut(), &doc, Span::new(9, 100009));
    view.fold_changed(&doc);
    let frame = ready_view(&mut view, &doc);
    let text = body(&frame);
    assert!(
        text.contains("[+ source") && text.contains("TAIL"),
        "{text}"
    );
    assert_eq!(
        view.heights().height(mdtui3::TileId(0)),
        Some(2),
        "one code label plus one PH body row"
    );
    assert_eq!(body(&ready_view(&mut view, &doc)), text);
    view.set_content_width(&doc, 60).unwrap();
    let text = body(&ready_view(&mut view, &doc));
    assert!(
        text.contains("[+ source") && text.contains("TAIL"),
        "{text}"
    );
    assert_eq!(view.heights().height(mdtui3::TileId(0)), Some(2));
}

#[test]
fn end_at_a_table_rows_placeholder_draws_the_bottom_border_once() {
    use mdtui_tui::render::viewport::{ViewportConfig, ViewportState};
    use mdtui3::{Document, TableId};
    let source = format!(
        "| H | V |\n| --- | --- |\n| A | B |\n| {} | SECRET |\n",
        "x".repeat(100000)
    );
    let mut doc = Document::new();
    doc.enable_display_shape_effects().unwrap();
    doc.append(&source).unwrap();
    doc.finish();
    let mut view = ViewportState::new(
        &doc,
        ViewportConfig {
            rows: 2,
            ..Default::default()
        },
    )
    .unwrap();
    let id = view
        .folds_mut()
        .insert_table_rows(&doc, TableId(0), 2..3)
        .unwrap();
    view.folds_mut().set_collapsed(&doc, id, true).unwrap();
    view.fold_changed(&doc);
    view.seek_end(&doc, true).unwrap();
    let frame = ready_view(&mut view, &doc);
    let text = body(&frame);
    assert!(text.contains("[+ rows"), "{text}");
    assert_eq!(text.matches('└').count(), 1, "{text}");
    assert_eq!(text.lines().count(), 2, "{text}");
    assert!(!text.contains("SECRET"));
    assert!(frame.stats.planned_tiles <= 8, "{:?}", frame.stats);
}

#[test]
fn a_frame_starting_exactly_at_a_table_rows_placeholder_keeps_its_bottom_border() {
    use mdtui_tui::render::viewport::{ViewportConfig, ViewportState};
    use mdtui3::{Anchor, Document, TableId};
    let mut source = "| H | V |\n| --- | --- |\n".to_owned();
    source.push_str(&format!(
        "| {} | B |\n",
        "a".repeat(256 * 100 - source.len() - 9)
    ));
    assert_eq!(source.len(), 256 * 100);
    source.push_str(&format!("| {} | SECRET |\n", "x".repeat(100000)));
    let mut doc = Document::new();
    doc.enable_display_shape_effects().unwrap();
    doc.append(&source).unwrap();
    doc.finish();
    let mut view = ViewportState::new(
        &doc,
        ViewportConfig {
            rows: 2,
            ..Default::default()
        },
    )
    .unwrap();
    let id = view
        .folds_mut()
        .insert_table_rows(&doc, TableId(0), 2..3)
        .unwrap();
    view.folds_mut().set_collapsed(&doc, id, true).unwrap();
    view.fold_changed(&doc);
    let start = doc.table(TableId(0)).unwrap().row(2).unwrap().source.start;
    view.set_top_from_refinement(
        &doc,
        Anchor {
            tile: doc.tile_at(start).unwrap(),
            offset: 0,
        },
        start,
        start,
        true,
    )
    .unwrap();
    let frame = ready_view(&mut view, &doc);
    let text = body(&frame);
    assert!(text.contains("[+ rows"), "{text}");
    assert_eq!(text.matches('└').count(), 1, "{text}");
    assert_eq!(text.lines().count(), 2, "{text}");
}

#[test]
fn indexed_fold_frontier_finishes_with_one_query_one_object_six_bytes() {
    use mdtui_tui::render::viewport::{ViewportConfig, ViewportFrameStatus, ViewportState};
    use mdtui3::{Document, Span};
    let prefix = format!("a{}", "\u{301}".repeat(50000));
    let source = format!("{prefix}{}TAIL\n", "x".repeat(100000));
    let mut doc = Document::new();
    doc.enable_display_shape_effects().unwrap();
    doc.append(&source).unwrap();
    doc.finish();
    let mut view = ViewportState::new(&doc, ViewportConfig::default()).unwrap();
    assert!(
        view.shared_links()
            .prepare_shape(&doc, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap()
    );
    register(
        view.folds_mut(),
        &doc,
        Span::new(prefix.len(), prefix.len() + 100000),
    );
    view.fold_changed(&doc);
    let mut zero = Budget::new(WorkLimit::default());
    assert_eq!(
        view.frame(&doc, &mut zero).unwrap().status,
        ViewportFrameStatus::BudgetExhausted
    );
    assert_eq!(zero.used(), WorkLimit::default());
    let limit = WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 6,
        tiles: 1,
        recovery: 1,
        dependencies: 1,
    };
    for _ in 0..50000 {
        let mut budget = Budget::new(limit);
        let frame = view.frame(&doc, &mut budget).unwrap();
        let used = budget.used();
        assert!(
            used.queries <= 1
                && used.objects <= 1
                && used.bytes <= 6
                && used.tiles <= 1
                && used.recovery <= 1
        );
        if frame.status == ViewportFrameStatus::Ready
            || frame.status == ViewportFrameStatus::EndOfDocument
        {
            let text = body(&frame);
            assert!(
                text.contains("[+ source") && text.contains("TAIL"),
                "length {}",
                text.len()
            );
            assert!(frame.stats.planned_tiles <= 5, "{:?}", frame.stats);
            return;
        }
    }
    panic!("one-query fragment frontier did not finish");
}

#[test]
fn summary_publication_rejects_resize_revision_fold_and_foreign_view_before_zero_budget() {
    use mdtui_tui::{
        fold_ui::FoldUi,
        render::{checkpoint::RenderCheckpoint, objects::SharedLinks},
    };
    use mdtui3::{BlockId, Document, FoldView};
    for mutation in 0..5 {
        let mut doc = Document::new();
        doc.append("```rust\nbody\n```\n").unwrap();
        let mut view = FoldView::<RenderCheckpoint>::with_state(&doc, 1).unwrap();
        let id = FoldId::Code(BlockId(0));
        view.set_collapsed(&doc, id, true).unwrap();
        let links = SharedLinks::new(&doc);
        let mut ui = FoldUi::default();
        ui.open(0, None, Some(id));
        for _ in 0..6 {
            ui.key(KeyCode::Down);
        }
        ui.key(KeyCode::Enter);
        for _ in 0..2 {
            ui.advance(
                &doc,
                &mut view,
                &links,
                80,
                &mut Budget::new(WorkLimit::unlimited()),
            );
        }
        assert!(
            ui.pending(),
            "summary must await separately paid publication"
        );
        let expected = match mutation {
            0 => {
                view.reset(1).unwrap();
                "StaleContinuation"
            }
            1 => {
                view.set_collapsed(&doc, id, false).unwrap();
                "StaleContinuation"
            }
            2 => {
                let update = doc.append("after\n").unwrap();
                view.apply(&doc, &update).unwrap();
                "StaleContinuation"
            }
            3 => {
                view = FoldView::with_state(&doc, 1).unwrap();
                view.set_collapsed(&doc, id, true).unwrap();
                "WrongView"
            }
            4 => {
                doc = Document::new();
                doc.append("other\n").unwrap();
                view = FoldView::with_state(&doc, 1).unwrap();
                "WrongDocument"
            }
            _ => unreachable!(),
        };
        let mut zero = Budget::new(WorkLimit::default());
        ui.advance(&doc, &mut view, &links, 40, &mut zero);
        assert!(
            !ui.pending(),
            "stale publication survived mutation {mutation}: {:?}",
            ui.status(80)
        );
        assert_eq!(zero.used(), WorkLimit::default());
        assert!(
            ui.status(80).unwrap().contains(expected),
            "mutation {mutation}: {:?}",
            ui.status(80)
        );
    }
}
