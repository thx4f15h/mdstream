use crossterm::event::{KeyCode, KeyModifiers};
use mdtui_tui::{
    app::AppState,
    cli::LinkMode,
    reading::MAX_QUERY_BYTES,
    render::frame::{FrameGeometry, FrameRuntime},
    screen::StyleRole,
    stream::StreamEngine,
};
use mdtui3::{Budget, Span, WorkLimit};

fn engine(source: &str, width: u16, rows: u16) -> StreamEngine {
    let runtime = FrameRuntime::from_finished_source(
        source,
        FrameGeometry {
            rows,
            cols: width,
            content_width: width,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    let mut engine = StreamEngine::new(runtime);
    paint(&mut engine);
    engine
}
fn paint(engine: &mut StreamEngine) {
    for _ in 0..1000 {
        let commit = engine
            .runtime_mut()
            .frame_to(&mut Vec::new(), &mut Budget::new(WorkLimit::unlimited()))
            .unwrap();
        if commit.committed
            && engine
                .runtime()
                .reading()
                .selection
                .pending_motion()
                .is_none()
        {
            return;
        }
    }
    panic!("no frame");
}
fn key(app: &mut AppState, engine: &mut StreamEngine, code: KeyCode) {
    assert!(
        !app.handle_key(engine, code, KeyModifiers::NONE)
            .unwrap()
            .quit
    );
}
fn query(app: &mut AppState, engine: &mut StreamEngine, text: &str) {
    key(app, engine, KeyCode::Char('/'));
    for ch in text.chars() {
        key(app, engine, KeyCode::Char(ch));
    }
    key(app, engine, KeyCode::Enter);
}
fn tiny() -> Budget {
    Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 6,
        tiles: 1,
        ..WorkLimit::default()
    })
}
fn finish_task(engine: &mut StreamEngine) -> Vec<u8> {
    let mut output = Vec::new();
    for _ in 0..200000 {
        engine
            .runtime_mut()
            .advance_reading(&mut output, &mut tiny());
        if !engine.runtime().reading().pending() {
            return output;
        }
    }
    panic!("task does not make progress");
}
fn clipboard(bytes: &[u8]) -> Vec<u8> {
    let prefix = b"\x1b]52;c;";
    let start = bytes
        .windows(prefix.len())
        .position(|part| part == prefix)
        .expect("actual OSC52 output")
        + prefix.len();
    let encoded = &bytes[start..start + bytes[start..].iter().position(|byte| *byte == 7).unwrap()];
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut decoded = Vec::new();
    let mut bits = 0_u32;
    let mut count = 0;
    for byte in encoded.iter().take_while(|byte| **byte != b'=') {
        bits = (bits << 6)
            | alphabet
                .iter()
                .position(|candidate| candidate == byte)
                .unwrap() as u32;
        count += 6;
        if count >= 8 {
            count -= 8;
            decoded.push((bits >> count) as u8);
        }
    }
    decoded
}

#[test]
fn exact_display_and_source_clipboards_and_selection_do_not_reanchor_body() {
    let mut engine = engine("a **b&amp;c** d\r\nnext\n", 40, 8);
    let mut app = AppState::default();
    let top = engine.runtime().top_source();
    key(&mut app, &mut engine, KeyCode::Char('v'));
    key(&mut app, &mut engine, KeyCode::Char(' '));
    for _ in 0..6 {
        key(&mut app, &mut engine, KeyCode::Right);
    }
    assert_eq!(
        engine.runtime().reading().selection.range(),
        Some(Span::new(0, 15))
    );
    paint(&mut engine);
    assert_eq!(engine.runtime().top_source(), top);
    assert!(
        engine
            .screen()
            .front_cell(0, 3)
            .unwrap()
            .style()
            .contains(StyleRole::SELECTION)
    );
    key(&mut app, &mut engine, KeyCode::Char('Y'));
    assert_eq!(clipboard(&finish_task(&mut engine)), b"a **b&amp;c** d");
    key(&mut app, &mut engine, KeyCode::Char('y'));
    assert_eq!(clipboard(&finish_task(&mut engine)), b"a b&c d");
    assert_eq!(engine.runtime().top_source(), top);
}

#[test]
fn q_is_query_text_enter_submits_utf8_pop_and_256_byte_limit() {
    let mut engine = engine("q 界\n", 40, 8);
    let mut app = AppState::default();
    key(&mut app, &mut engine, KeyCode::Char('/'));
    key(&mut app, &mut engine, KeyCode::Char('q'));
    key(&mut app, &mut engine, KeyCode::Char('界'));
    key(&mut app, &mut engine, KeyCode::Backspace);
    assert_eq!(engine.runtime().reading().input(), Some("q"));
    assert!(engine.runtime().reading().current_match().is_none());
    for _ in 0..MAX_QUERY_BYTES {
        key(&mut app, &mut engine, KeyCode::Char('x'));
    }
    assert_eq!(
        engine.runtime().reading().input().unwrap().len(),
        MAX_QUERY_BYTES
    );
    assert!(
        app.handle_key(&mut engine, KeyCode::Char('c'), KeyModifiers::CONTROL)
            .unwrap()
            .quit
    );
    key(&mut app, &mut engine, KeyCode::Esc);
    assert!(
        app.handle_key(&mut engine, KeyCode::Char('q'), KeyModifiers::NONE)
            .unwrap()
            .quit
    );
    query(&mut app, &mut engine, "q");
    finish_task(&mut engine);
    assert_eq!(
        engine.runtime().reading().current_match().unwrap().source,
        Span::new(0, 1)
    );
}

#[test]
fn overlapping_multibyte_forward_forward_back_forward_and_boundaries() {
    let mut engine = engine("界界界界\n", 40, 8);
    let mut app = AppState::default();
    query(&mut app, &mut engine, "界界");
    finish_task(&mut engine);
    assert_eq!(
        engine.runtime().reading().current_match().unwrap().display,
        0..6
    );
    key(&mut app, &mut engine, KeyCode::Char('n'));
    finish_task(&mut engine);
    assert_eq!(
        engine.runtime().reading().current_match().unwrap().display,
        3..9
    );
    key(&mut app, &mut engine, KeyCode::Char('N'));
    finish_task(&mut engine);
    assert_eq!(
        engine.runtime().reading().current_match().unwrap().display,
        0..6
    );
    key(&mut app, &mut engine, KeyCode::Char('n'));
    finish_task(&mut engine);
    assert_eq!(
        engine.runtime().reading().current_match().unwrap().display,
        3..9
    );
    key(&mut app, &mut engine, KeyCode::Char('n'));
    finish_task(&mut engine);
    assert_eq!(
        engine.runtime().reading().current_match().unwrap().display,
        6..12
    );
    key(&mut app, &mut engine, KeyCode::Char('n'));
    finish_task(&mut engine);
    assert_eq!(engine.runtime().reading().message(), "end of matches");
    key(&mut app, &mut engine, KeyCode::Char('N'));
    finish_task(&mut engine);
    assert_eq!(
        engine.runtime().reading().current_match().unwrap().display,
        3..9
    );
}

#[test]
fn actual_unicode_cells_whole_grapheme_source_mapping_and_continuations() {
    let source = "界e\u{301}👩‍💻🇨🇳!\n";
    let mut engine = engine(source, 40, 8);
    let mut app = AppState::default();
    for (col, text, span) in [
        (0, "界", Span::new(0, 3)),
        (2, "e\u{301}", Span::new(3, 6)),
        (3, "👩‍💻", Span::new(6, 17)),
        (5, "🇨🇳", Span::new(17, 25)),
        (7, "!", Span::new(25, 26)),
    ] {
        let cell = engine.screen().front_cell(0, col).unwrap();
        assert_eq!((cell.text(), cell.source()), (text, Some(span)));
    }
    key(&mut app, &mut engine, KeyCode::Char('v'));
    for expected in [
        Span::new(3, 6),
        Span::new(6, 17),
        Span::new(17, 25),
        Span::new(25, 26),
    ] {
        key(&mut app, &mut engine, KeyCode::Right);
        assert_eq!(engine.runtime().reading().selection.range(), Some(expected));
    }
}

#[test]
fn selection_survives_resize_append_late_definition_and_eof_search_goes_stale() {
    let mut runtime = FrameRuntime::new(
        FrameGeometry {
            rows: 8,
            cols: 40,
            content_width: 40,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    let update = runtime.append_bytes(b"[label][r]\n\nq\n").unwrap().unwrap();
    runtime.apply_update(&update).unwrap();
    let mut engine = StreamEngine::new(runtime);
    paint(&mut engine);
    let mut app = AppState::default();
    key(&mut app, &mut engine, KeyCode::Char('v'));
    let range = engine.runtime().reading().selection.range();
    query(&mut app, &mut engine, "q");
    finish_task(&mut engine);
    let update = engine
        .runtime_mut()
        .append_bytes(b"\n[r]: /target\n")
        .unwrap()
        .unwrap();
    engine.runtime_mut().apply_update(&update).unwrap();
    assert!(engine.runtime().reading().stale());
    assert!(engine.runtime().reading().current_match().is_none());
    engine.resize(12, 24).unwrap();
    assert_eq!(engine.runtime().reading().selection.range(), range);
    let update = engine.runtime_mut().finish_input().unwrap().unwrap();
    engine.runtime_mut().apply_update(&update).unwrap();
    paint(&mut engine);
    assert_eq!(engine.runtime().reading().selection.range(), range);
    key(&mut app, &mut engine, KeyCode::Char('/'));
    key(&mut app, &mut engine, KeyCode::Enter);
    finish_task(&mut engine);
    assert!(!engine.runtime().reading().stale());
    assert!(engine.runtime().reading().current_match().is_some());
}

#[test]
fn extending_across_screen_scrolls_to_real_cells_and_keeps_fixed_source_anchor() {
    let mut engine = engine("a\n\nb\n\nc\n\nd\n\ne\n\nf\n", 20, 5);
    let mut app = AppState::default();
    key(&mut app, &mut engine, KeyCode::Char('v'));
    key(&mut app, &mut engine, KeyCode::Char(' '));
    paint(&mut engine);
    for end in [4, 7, 10, 13, 16] {
        key(&mut app, &mut engine, KeyCode::Right);
        paint(&mut engine);
        assert_eq!(
            engine.runtime().reading().selection.range(),
            Some(Span::new(0, end))
        );
        assert!(!engine.runtime().reading().pending());
    }
    key(&mut app, &mut engine, KeyCode::Char('Y'));
    assert_eq!(
        clipboard(&finish_task(&mut engine)),
        b"a\n\nb\n\nc\n\nd\n\ne\n\nf"
    );
    key(&mut app, &mut engine, KeyCode::Char('y'));
    assert_eq!(clipboard(&finish_task(&mut engine)), b"a\nb\nc\nd\ne\nf");
    engine.resize(8, 30).unwrap();
    paint(&mut engine);
    assert_eq!(
        engine.runtime().reading().selection.range(),
        Some(Span::new(0, 16))
    );
}

#[test]
fn actual_shared_tiny_frames_advance_search_and_copy_and_report_wire_bytes() {
    let mut engine = engine("a **b&amp;c** d\n\nq\n", 40, 8);
    let mut app = AppState::default();
    query(&mut app, &mut engine, "q");
    for _ in 0..10000 {
        let _ = engine
            .runtime_mut()
            .frame_to(&mut Vec::new(), &mut tiny())
            .unwrap();
        if !engine.runtime().reading().pending() && !engine.runtime().has_pending_jump() {
            break;
        }
    }
    assert_eq!(
        engine.runtime().reading().current_match().unwrap().source,
        Span::new(17, 18)
    );
    assert!(!engine.runtime().has_pending_jump());
    key(&mut app, &mut engine, KeyCode::Esc);
    key(&mut app, &mut engine, KeyCode::Home);
    paint(&mut engine);
    key(&mut app, &mut engine, KeyCode::Char('v'));
    key(&mut app, &mut engine, KeyCode::Char('Y'));
    let mut wire = Vec::new();
    let mut total = 0;
    for _ in 0..10000 {
        let commit = engine
            .runtime_mut()
            .frame_to(&mut wire, &mut tiny())
            .unwrap();
        total += commit.metrics.clipboard_bytes;
        if !engine.runtime().reading().pending() {
            break;
        }
    }
    assert_eq!(clipboard(&wire), b"a");
    assert_eq!(total, 12);
    assert_eq!(engine.runtime().reading().clipboard_bytes(), 12);
}

#[test]
fn small_selection_in_multimegabyte_paragraph_is_logical_and_cut_independent() {
    use mdtui_tui::history::HistoryPosition;
    let source = "abcdefghij".repeat(220_000);
    let mut engine = engine(&source, 23, 8);
    let mut app = AppState::default();
    for cut in [2_000_007, 2_000_038] {
        engine
            .restore_navigation_position(HistoryPosition {
                source: cut,
                row_offset: 0,
            })
            .unwrap();
        paint(&mut engine);
        let first = engine
            .screen()
            .front_cell(0, 0)
            .unwrap()
            .source()
            .unwrap()
            .start;
        // An independent ASCII oracle: at every actual source-backed cell,
        // the visible byte is exactly the corresponding repeated digit slot.
        for col in 0..23 {
            let cell = engine.screen().front_cell(0, col).unwrap();
            let at = cell.source().unwrap().start;
            assert_eq!(cell.text().as_bytes(), &b"abcdefghij"[at % 10..at % 10 + 1]);
        }
        key(&mut app, &mut engine, KeyCode::Esc);
        key(&mut app, &mut engine, KeyCode::Char('v'));
        key(&mut app, &mut engine, KeyCode::Char(' '));
        for _ in 0..40 {
            key(&mut app, &mut engine, KeyCode::Right);
            paint(&mut engine);
        }
        assert_eq!(
            engine.runtime().reading().selection.range(),
            Some(Span::new(first, first + 41))
        );
        key(&mut app, &mut engine, KeyCode::Char('y'));
        let text = clipboard(&finish_task(&mut engine));
        let expected: Vec<_> = (first..first + 41)
            .map(|at| b"abcdefghij"[at % 10])
            .collect();
        assert_eq!(text, expected);
        assert!(!text.contains(&b'\n'));
        let range = engine.runtime().reading().selection.range();
        engine.resize(9, 31).unwrap();
        paint(&mut engine);
        assert_eq!(engine.runtime().reading().selection.range(), range);
        key(&mut app, &mut engine, KeyCode::Char('y'));
        assert_eq!(clipboard(&finish_task(&mut engine)), expected);
        engine.resize(8, 23).unwrap();
        paint(&mut engine);
    }
    assert!(app.history().len() <= 1);
}

#[test]
fn editing_cancels_pending_query_without_moving_and_explicit_resubmit_restarts() {
    let mut engine = engine("a".repeat(20000).as_str(), 40, 8);
    let mut app = AppState::default();
    let top = engine.runtime().top_source();
    query(&mut app, &mut engine, "no");
    assert!(engine.runtime().reading().pending());
    key(&mut app, &mut engine, KeyCode::Char('/'));
    key(&mut app, &mut engine, KeyCode::Backspace);
    key(&mut app, &mut engine, KeyCode::Backspace);
    key(&mut app, &mut engine, KeyCode::Char('a'));
    assert!(!engine.runtime().reading().pending());
    paint(&mut engine);
    assert_eq!(engine.runtime().top_source(), top);
    key(&mut app, &mut engine, KeyCode::Enter);
    finish_task(&mut engine);
    assert_eq!(
        engine.runtime().reading().current_match().unwrap().source,
        Span::new(0, 1)
    );
    key(&mut app, &mut engine, KeyCode::Esc);
    assert!(engine.runtime().reading().current_match().is_none());
    assert!(!engine.runtime().reading().pending());
}

#[test]
fn stream_metrics_count_actual_clipboard_output_even_during_pending_navigation() {
    use std::time::Duration;
    let runtime = FrameRuntime::new(
        FrameGeometry {
            rows: 8,
            cols: 40,
            content_width: 40,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    let mut engine = StreamEngine::new(runtime);
    engine
        .ingest_data(b"a **b&amp;c** d\n\nq\n", Duration::ZERO)
        .unwrap();
    for _ in 0..100 {
        engine
            .render_due(
                &mut Vec::new(),
                &mut Budget::new(WorkLimit::unlimited()),
                Duration::ZERO,
            )
            .unwrap();
        if engine.first_frame_done() {
            break;
        }
    }
    assert!(engine.first_frame_done());
    let mut app = AppState::default();
    key(&mut app, &mut engine, KeyCode::Char('v'));
    key(&mut app, &mut engine, KeyCode::Char(' '));
    for _ in 0..6 {
        key(&mut app, &mut engine, KeyCode::Right);
    }
    key(&mut app, &mut engine, KeyCode::Char('Y'));
    query(&mut app, &mut engine, "q");
    let before = engine.metrics().output_bytes;
    let mut wire = Vec::new();
    for _ in 0..10000 {
        engine
            .render_due(&mut wire, &mut tiny(), Duration::ZERO)
            .unwrap();
        if !engine.runtime().reading().pending() && !engine.runtime().has_pending_jump() {
            break;
        }
    }
    assert_eq!(clipboard(&wire), b"a **b&amp;c** d");
    assert_eq!(engine.metrics().output_bytes - before, wire.len());
}

#[test]
fn panned_table_copy_uses_current_column_and_width_policy_with_exact_raw_envelope() {
    use mdtui_tui::render::table_widths::TableWidthPolicy;
    let source = "|aaaa|bbbb|cccc|dddd|\n|---|---|---|---|\n|AAAA|BBBB|CCCC|DDDD|\n";
    let mut engine = engine(source, 20, 10);
    let mut app = AppState::default();
    engine
        .runtime_mut()
        .set_table_policy(TableWidthPolicy::new(3, Some(3)).unwrap())
        .unwrap();
    paint(&mut engine);
    key(&mut app, &mut engine, KeyCode::Right);
    paint(&mut engine);
    assert_eq!(
        engine
            .runtime()
            .viewport()
            .table_widths()
            .first_column(mdtui3::TableId(0)),
        1
    );
    assert_eq!(
        (
            engine.screen().front_cell(1, 2).unwrap().text(),
            engine.screen().front_cell(1, 2).unwrap().source()
        ),
        ("b", Some(Span::new(6, 7)))
    );
    key(&mut app, &mut engine, KeyCode::Char('v'));
    key(&mut app, &mut engine, KeyCode::Char(' '));
    for _ in 0..8 {
        key(&mut app, &mut engine, KeyCode::Right);
        paint(&mut engine);
    }
    assert_eq!(
        engine.runtime().reading().selection.range(),
        Some(Span::new(6, 19))
    );
    key(&mut app, &mut engine, KeyCode::Char('y'));
    assert_eq!(clipboard(&finish_task(&mut engine)), b"bbb\tccc\tddd");
    key(&mut app, &mut engine, KeyCode::Char('Y'));
    assert_eq!(clipboard(&finish_task(&mut engine)), b"bbbb|cccc|ddd");
    for _ in 0..9 {
        key(&mut app, &mut engine, KeyCode::Right);
        paint(&mut engine);
    }
    assert_eq!(
        engine.runtime().reading().selection.range(),
        Some(Span::new(6, 59))
    );
    key(&mut app, &mut engine, KeyCode::Char('y'));
    assert_eq!(
        clipboard(&finish_task(&mut engine)),
        b"bbb\tccc\tddd\nBBB\tCCC\tDDD"
    );
    key(&mut app, &mut engine, KeyCode::Char('Y'));
    assert_eq!(
        clipboard(&finish_task(&mut engine)),
        b"bbbb|cccc|dddd|\n|---|---|---|---|\n|AAAA|BBBB|CCCC|DDD"
    );
    engine
        .runtime_mut()
        .set_table_policy(TableWidthPolicy::new(0, Some(2)).unwrap())
        .unwrap();
    paint(&mut engine);
    key(&mut app, &mut engine, KeyCode::Char('y'));
    assert_eq!(
        clipboard(&finish_task(&mut engine)),
        b"bb\tcc\tdd\nBB\tCC\tDD"
    );
    engine.resize(12, 30).unwrap();
    paint(&mut engine);
    assert_eq!(
        engine.runtime().reading().selection.range(),
        Some(Span::new(6, 59))
    );
    key(&mut app, &mut engine, KeyCode::Char('y'));
    assert_eq!(
        clipboard(&finish_task(&mut engine)),
        b"bb\tcc\tdd\nBB\tCC\tDD"
    );
}

#[test]
fn submitted_search_reveals_the_indexed_table_column_and_highlights_real_cells() {
    use mdtui_tui::render::table_widths::TableWidthPolicy;
    let mut engine = engine(
        "|aaaa|bbbb|cccc|dddd|\n|---|---|---|---|\n|AAAA|BBBB|CCCC|DDDD|\n",
        20,
        10,
    );
    let mut app = AppState::default();
    engine
        .runtime_mut()
        .set_table_policy(TableWidthPolicy::new(3, Some(3)).unwrap())
        .unwrap();
    paint(&mut engine);
    query(&mut app, &mut engine, "DD");
    for _ in 0..10000 {
        let commit = engine
            .runtime_mut()
            .frame_to(&mut Vec::new(), &mut tiny())
            .unwrap();
        if commit.committed
            && !engine.runtime().reading().pending()
            && !engine.runtime().has_pending_jump()
        {
            break;
        }
    }
    assert_eq!(
        engine.runtime().reading().current_match().unwrap().source,
        Span::new(56, 58)
    );
    assert_eq!(
        engine
            .runtime()
            .viewport()
            .table_widths()
            .first_column(mdtui3::TableId(0)),
        3
    );
    let mut found = Vec::new();
    for row in 0..10 {
        for col in 0..20 {
            let cell = engine.screen().front_cell(row, col).unwrap();
            if cell.style().contains(StyleRole::SEARCH) {
                found.push((cell.text().to_owned(), cell.source()));
            }
        }
    }
    assert_eq!(
        found,
        vec![
            ("D".to_owned(), Some(Span::new(56, 57))),
            ("D".to_owned(), Some(Span::new(57, 58)))
        ]
    );
    assert!(app.history().is_empty());
}

#[test]
fn append_cancels_visible_copy_but_not_immutable_source_copy_or_selection() {
    let mut runtime = FrameRuntime::new(
        FrameGeometry {
            rows: 8,
            cols: 40,
            content_width: 40,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    let update = runtime.append_bytes(b"abc\n").unwrap().unwrap();
    runtime.apply_update(&update).unwrap();
    let mut engine = StreamEngine::new(runtime);
    let mut app = AppState::default();
    paint(&mut engine);
    key(&mut app, &mut engine, KeyCode::Char('v'));
    key(&mut app, &mut engine, KeyCode::Char(' '));
    key(&mut app, &mut engine, KeyCode::Right);
    key(&mut app, &mut engine, KeyCode::Right);
    key(&mut app, &mut engine, KeyCode::Char('Y'));
    let update = engine
        .runtime_mut()
        .append_bytes(b"more\n")
        .unwrap()
        .unwrap();
    engine.runtime_mut().apply_update(&update).unwrap();
    assert_eq!(clipboard(&finish_task(&mut engine)), b"abc");
    key(&mut app, &mut engine, KeyCode::Char('y'));
    let update = engine
        .runtime_mut()
        .append_bytes(b"next\n")
        .unwrap()
        .unwrap();
    engine.runtime_mut().apply_update(&update).unwrap();
    assert!(!engine.runtime().reading().pending());
    assert_eq!(
        engine.runtime().reading().message(),
        "copy cancelled: content changed"
    );
    assert_eq!(
        engine.runtime().reading().selection.range(),
        Some(Span::new(0, 3))
    );
    assert!(finish_task(&mut engine).is_empty());
}

#[test]
fn a_match_beyond_configured_cell_clip_navigates_to_content_and_reports_clipped() {
    use mdtui_tui::render::table_widths::TableWidthPolicy;
    let mut engine = engine("|abcX|\n|----|\n", 20, 10);
    let mut app = AppState::default();
    engine
        .runtime_mut()
        .set_table_policy(TableWidthPolicy::new(3, Some(3)).unwrap())
        .unwrap();
    paint(&mut engine);
    query(&mut app, &mut engine, "X");
    finish_task(&mut engine);
    paint(&mut engine);
    assert_eq!(
        engine.runtime().reading().current_match().unwrap().source,
        Span::new(4, 5)
    );
    let text: String = (0..20)
        .map(|col| engine.screen().front_cell(0, col).unwrap().text())
        .collect();
    assert!(text.contains("abc"), "{text:?}");
    assert!(
        engine
            .runtime()
            .reading()
            .status()
            .unwrap()
            .contains("match not drawn")
    );
}

#[test]
fn logical_image_alt_nested_alt_and_terminal_empty_placeholder_have_explicit_search_truth() {
    for (source, query_text, expected_text, expected_source, expected_hit) in [
        ("![A &amp; 界](img)\n", "&", "A & 界", Span::new(4, 9), true),
        (
            "![a ![b](img) c](outer)\n",
            "b",
            "a b c",
            Span::new(6, 7),
            true,
        ),
        ("![](img)\n", "[image]", "[image]", Span::new(0, 1), false),
    ] {
        let mut engine = engine(source, 40, 8);
        let mut app = AppState::default();
        let drawn: String = (0..40)
            .filter_map(|col| {
                let cell = engine.screen().front_cell(0, col).unwrap();
                (!cell.is_continuation() && cell.source().is_some()).then(|| cell.text().to_owned())
            })
            .collect();
        assert_eq!(drawn, expected_text);
        if !expected_hit {
            assert_eq!(
                engine.screen().front_cell(0, 0).unwrap().source(),
                Some(expected_source)
            );
        }
        query(&mut app, &mut engine, query_text);
        let mut complete = false;
        for _ in 0..10000 {
            let commit = engine
                .runtime_mut()
                .frame_to(&mut Vec::new(), &mut tiny())
                .unwrap();
            if commit.committed
                && !engine.runtime().reading().pending()
                && !engine.runtime().has_pending_jump()
            {
                complete = true;
                break;
            }
        }
        assert!(complete, "unresolved logical target for {source:?}");
        if expected_hit {
            assert_eq!(
                engine.runtime().reading().current_match().unwrap().source,
                expected_source
            );
            let highlighted: Vec<_> = (0..8)
                .flat_map(|row| (0..40).map(move |col| (row, col)))
                .filter_map(|(row, col)| {
                    let cell = engine.screen().front_cell(row, col).unwrap();
                    (cell.style().contains(StyleRole::SEARCH) && !cell.is_continuation())
                        .then(|| (cell.text().to_owned(), cell.source()))
                })
                .collect();
            assert_eq!(
                highlighted,
                vec![(query_text.to_owned(), Some(expected_source))]
            );
        } else {
            assert!(engine.runtime().reading().current_match().is_none());
            assert_eq!(engine.runtime().reading().message(), "no match");
            assert_eq!(engine.screen().front_cell(0, 0).unwrap().text(), "[");
        }
    }
}

#[test]
fn omitted_heading_and_list_links_complete_without_fake_highlight_or_bottom_navigation() {
    // Heading literal spaces retain source; the list's normalized word gap
    // is a generated cell, so its source-backed characters concatenate.
    for (source, expected_row, expected_sources) in [
        (
            "# before [secret](url) after\n",
            "before  after",
            "before  after",
        ),
        (
            "- before [secret](url) after\n",
            "• before after",
            "beforeafter",
        ),
    ] {
        let mut engine = engine(source, 40, 8);
        let mut app = AppState::default();
        let drawn: String = (0..40)
            .filter_map(|col| {
                let cell = engine.screen().front_cell(0, col).unwrap();
                (!cell.is_continuation() && cell.source().is_some()).then(|| cell.text().to_owned())
            })
            .collect();
        assert_eq!(drawn, expected_sources);
        let row: String = (0..40)
            .map(|col| engine.screen().front_cell(0, col).unwrap().text())
            .collect();
        assert_eq!(row.trim_end(), expected_row);
        query(&mut app, &mut engine, "secret");
        let mut complete = false;
        for _ in 0..10000 {
            let commit = engine
                .runtime_mut()
                .frame_to(&mut Vec::new(), &mut tiny())
                .unwrap();
            if commit.committed
                && !engine.runtime().reading().pending()
                && !engine.runtime().has_pending_jump()
            {
                complete = true;
                break;
            }
        }
        assert!(complete);
        assert_eq!(
            engine.runtime().reading().current_match().unwrap().source,
            Span::new(10, 16)
        );
        assert!(
            engine
                .runtime()
                .reading()
                .status()
                .unwrap()
                .contains("match not drawn")
        );
        for row in 0..8 {
            for col in 0..40 {
                assert!(
                    !engine
                        .screen()
                        .front_cell(row, col)
                        .unwrap()
                        .style()
                        .contains(StyleRole::SEARCH)
                );
            }
        }
        assert!(
            engine
                .runtime()
                .top_source()
                .is_some_and(|source| source <= 2)
        );
    }
    let mut engine = engine("# [secret](url)\n", 40, 8);
    let mut app = AppState::default();
    query(&mut app, &mut engine, "secret");
    let mut complete = false;
    for _ in 0..10000 {
        let commit = engine
            .runtime_mut()
            .frame_to(&mut Vec::new(), &mut tiny())
            .unwrap();
        if commit.committed
            && !engine.runtime().reading().pending()
            && !engine.runtime().has_pending_jump()
        {
            complete = true;
            break;
        }
    }
    assert!(complete);
    assert_eq!(
        engine.runtime().reading().current_match().unwrap().source,
        Span::new(3, 9)
    );
    assert!(
        engine
            .runtime()
            .reading()
            .status()
            .unwrap()
            .contains("match not drawn")
    );
}

fn combo_limit() -> WorkLimit {
    WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 6,
        tiles: 1,
        recovery: 1,
        dependencies: 1,
    }
}
fn settle_combo_frame(engine: &mut StreamEngine) {
    for _ in 0..200_000 {
        let mut budget = Budget::new(combo_limit());
        let commit = engine
            .runtime_mut()
            .frame_to(&mut Vec::new(), &mut budget)
            .unwrap();
        let used = budget.used();
        assert!(used.queries <= 1 && used.objects <= 1 && used.bytes <= 6);
        assert!(used.tiles <= 1 && used.recovery <= 1 && used.dependencies <= 1);
        if commit.committed
            && !engine.runtime().reading().pending()
            && !engine.runtime().has_pending_jump()
        {
            return;
        }
    }
    panic!("combined frame did not settle");
}
fn finish_combo_copy(engine: &mut StreamEngine) -> Vec<u8> {
    let mut wire = Vec::new();
    for _ in 0..200_000 {
        let mut budget = Budget::new(combo_limit());
        engine.runtime_mut().advance_reading(&mut wire, &mut budget);
        let used = budget.used();
        assert!(used.queries <= 1 && used.objects <= 1 && used.bytes <= 6);
        assert!(used.tiles <= 1 && used.recovery <= 1 && used.dependencies <= 1);
        if !engine.runtime().reading().pending() {
            return clipboard(&wire);
        }
        assert!(wire.is_empty());
    }
    panic!("combined copy did not settle");
}
fn replace_query(app: &mut AppState, engine: &mut StreamEngine, old_chars: usize, text: &str) {
    key(app, engine, KeyCode::Char('/'));
    for _ in 0..old_chars {
        key(app, engine, KeyCode::Backspace);
    }
    for c in text.chars() {
        key(app, engine, KeyCode::Char(c));
    }
    key(app, engine, KeyCode::Enter);
}

#[test]
fn ordered_adjacent_styles_and_entity_selection_keep_exact_cell_sources_and_copy_bytes() {
    let mut engine = engine("*a*_b_ &fjlig;\n", 40, 8);
    let mut app = AppState::default();
    key(&mut app, &mut engine, KeyCode::Char('v'));
    assert_eq!(
        engine.runtime().reading().selection.range(),
        Some(Span::new(1, 2))
    );
    key(&mut app, &mut engine, KeyCode::Right);
    assert_eq!(
        engine.runtime().reading().selection.range(),
        Some(Span::new(4, 5))
    );
    key(&mut app, &mut engine, KeyCode::Right);
    key(&mut app, &mut engine, KeyCode::Right);
    assert_eq!(
        engine.runtime().reading().selection.range(),
        Some(Span::new(7, 14))
    );
    key(&mut app, &mut engine, KeyCode::Char('Y'));
    assert_eq!(finish_combo_copy(&mut engine), b"&fjlig;");
    key(&mut app, &mut engine, KeyCode::Char('y'));
    assert_eq!(finish_combo_copy(&mut engine), b"fj");
    query(&mut app, &mut engine, "j");
    settle_combo_frame(&mut engine);
    assert_eq!(
        engine.runtime().reading().current_match().unwrap().source,
        Span::new(7, 14)
    );
    assert!(
        !engine
            .runtime()
            .reading()
            .status()
            .unwrap()
            .contains("match not drawn")
    );
}

#[test]
fn late_conditional_scope_definition_reprojects_real_selection_search_and_copy() {
    let mut runtime = FrameRuntime::new(
        FrameGeometry {
            rows: 8,
            cols: 40,
            content_width: 40,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    let update = runtime
        .append_bytes(format!("*{} [x* z]\n\n", "a".repeat(20_000)).as_bytes())
        .unwrap()
        .unwrap();
    runtime.apply_update(&update).unwrap();
    let mut engine = StreamEngine::new(runtime);
    paint(&mut engine);
    let mut app = AppState::default();
    key(&mut app, &mut engine, KeyCode::Char('v'));
    let selected = Some(Span::new(1, 2));
    assert_eq!(engine.runtime().reading().selection.range(), selected);
    query(&mut app, &mut engine, "aaa");
    settle_combo_frame(&mut engine);
    assert_eq!(
        engine.runtime().reading().current_match().unwrap().source,
        Span::new(1, 4)
    );
    let update = engine
        .runtime_mut()
        .append_bytes(b"[x* z]: /u\n")
        .unwrap()
        .unwrap();
    engine.runtime_mut().apply_update(&update).unwrap();
    assert!(engine.runtime().reading().stale());
    assert!(engine.runtime().reading().current_match().is_none());
    settle_combo_frame(&mut engine);
    assert_eq!(engine.runtime().reading().selection.range(), selected);
    assert_eq!(engine.screen().front_cell(0, 0).unwrap().text(), "*");
    assert_eq!(
        engine.screen().front_cell(0, 0).unwrap().source(),
        Some(Span::new(0, 1))
    );
    assert_eq!(
        engine.screen().front_cell(0, 1).unwrap().source(),
        Some(Span::new(1, 2))
    );
    key(&mut app, &mut engine, KeyCode::Char('Y'));
    assert_eq!(finish_combo_copy(&mut engine), b"a");
    key(&mut app, &mut engine, KeyCode::Char('y'));
    assert_eq!(finish_combo_copy(&mut engine), b"a");
    replace_query(&mut app, &mut engine, 3, "*aaa");
    settle_combo_frame(&mut engine);
    assert_eq!(
        engine.runtime().reading().current_match().unwrap().source,
        Span::new(0, 4)
    );
    assert_eq!(engine.runtime().reading().selection.range(), selected);
}

#[test]
fn general_header_confirmation_preserves_source_selection_and_resubmits_fresh_cell_search() {
    let mut runtime = FrameRuntime::new(
        FrameGeometry {
            rows: 8,
            cols: 60,
            content_width: 60,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    let update = runtime
        .append_bytes(format!("*prior\nend*x* {}", "z".repeat(1100)).as_bytes())
        .unwrap()
        .unwrap();
    runtime.apply_update(&update).unwrap();
    let mut engine = StreamEngine::new(runtime);
    paint(&mut engine);
    let mut app = AppState::default();
    query(&mut app, &mut engine, "end");
    settle_combo_frame(&mut engine);
    assert_eq!(
        engine.runtime().reading().current_match().unwrap().source,
        Span::new(7, 10)
    );
    key(&mut app, &mut engine, KeyCode::Char('v'));
    let selected = engine.runtime().reading().selection.range();
    assert!(selected.is_some());
    let update = engine
        .runtime_mut()
        .append_bytes(b"|b\n---|---\n")
        .unwrap()
        .unwrap();
    engine.runtime_mut().apply_update(&update).unwrap();
    assert_eq!(engine.runtime().document().tables().len(), 1);
    assert!(engine.runtime().reading().stale());
    settle_combo_frame(&mut engine);
    assert_eq!(engine.runtime().reading().selection.range(), selected);
    replace_query(&mut app, &mut engine, 3, "endx");
    settle_combo_frame(&mut engine);
    assert_eq!(
        engine.runtime().reading().current_match().unwrap().source,
        Span::new(7, 12)
    );
    assert!(
        !engine
            .runtime()
            .reading()
            .status()
            .unwrap()
            .contains("match not drawn")
    );
    assert_eq!(engine.runtime().reading().selection.range(), selected);
}

#[test]
fn header_footnote_active_order_distinguishes_drawn_literal_and_hidden_core_matches() {
    for (old, body, definitions, query_text, drawn) in [
        ("$prior", "end$ [^n]", "[^n]: note\n", "[^n]", true),
        (
            "`prior",
            "[^early]A`tail[^later]B",
            "[^early]: E\n[^later]: L\n",
            "early",
            false,
        ),
    ] {
        let source = format!(
            "{old}\n{body} {}|b\n-|-\nx|y\n\n{definitions}",
            "z".repeat(1100)
        );
        let mut engine = engine(&source, 60, 8);
        let mut app = AppState::default();
        query(&mut app, &mut engine, query_text);
        settle_combo_frame(&mut engine);
        let start = source.find(query_text).unwrap();
        assert_eq!(
            engine.runtime().reading().current_match().unwrap().source,
            Span::new(start, start + query_text.len())
        );
        assert_eq!(
            !engine
                .runtime()
                .reading()
                .status()
                .unwrap()
                .contains("match not drawn"),
            drawn
        );
        assert!(!engine.runtime().has_pending_jump());
    }
}
