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
fn visible_copy_of_indexed_source_aliases_keeps_all_projected_characters() {
    let cases = [
        ("![](界)".to_owned(), "[image]".to_owned(), "!"),
        ("&fjlig;界".to_owned(), "fj".to_owned(), "&fjlig;"),
        ("```\n\t界\n```".to_owned(), " ".to_owned(), "\t"),
    ];
    let mut copied = Vec::new();
    for (source, expected, raw) in cases {
        let mut engine = engine(&source, 24, 8);
        let mut app = AppState::default();
        key(&mut app, &mut engine, KeyCode::Char('v'));
        key(&mut app, &mut engine, KeyCode::Char('y'));
        copied.push((clipboard(&finish_task(&mut engine)), expected.into_bytes()));
        key(&mut app, &mut engine, KeyCode::Char('Y'));
        assert_eq!(clipboard(&finish_task(&mut engine)), raw.as_bytes());
    }
    for (actual, expected) in copied {
        assert_eq!(actual, expected);
    }
    let marks = "\u{301}".repeat(500);
    let raw = format!("&fjlig;{marks}");
    let mut engine = engine(&format!("{raw}界"), 24, 8);
    let mut app = AppState::default();
    key(&mut app, &mut engine, KeyCode::Char('v'));
    key(&mut app, &mut engine, KeyCode::Right);
    assert_eq!(
        engine.runtime().reading().selection.range(),
        Some(Span::new(0, raw.len()))
    );
    key(&mut app, &mut engine, KeyCode::Char('y'));
    assert_eq!(
        clipboard(&finish_task(&mut engine)),
        format!("fj{marks}").as_bytes()
    );
    key(&mut app, &mut engine, KeyCode::Char('Y'));
    assert_eq!(clipboard(&finish_task(&mut engine)), raw.as_bytes());
}

#[test]
fn giant_selection_copy_search_resize_sidebar_and_history_use_one_complete_cluster() {
    use mdtui_tui::history::HistoryPosition;
    let raw = format!("a&#x301;{}", "\u{301}".repeat(5000));
    let visible = format!("a{}", "\u{301}".repeat(5001));
    let source = format!("{raw}!\n\n# Destination\n\ntail\n");
    let mut engine = engine(&source, 48, 8);
    let mut app = AppState::default();
    let span = Span::new(0, raw.len());
    key(&mut app, &mut engine, KeyCode::Char('v'));
    assert_eq!(engine.runtime().reading().selection.range(), Some(span));
    paint(&mut engine);
    assert!(
        engine
            .screen()
            .front_cell(0, 0)
            .unwrap()
            .style()
            .contains(StyleRole::SELECTION)
    );
    key(&mut app, &mut engine, KeyCode::Char('y'));
    assert_eq!(clipboard(&finish_task(&mut engine)), visible.as_bytes());
    key(&mut app, &mut engine, KeyCode::Char('Y'));
    assert_eq!(clipboard(&finish_task(&mut engine)), raw.as_bytes());
    app.handle_key(&mut engine, KeyCode::Right, KeyModifiers::SHIFT)
        .unwrap();
    assert_eq!(
        engine.runtime().reading().selection.range(),
        Some(Span::new(0, raw.len() + 1))
    );
    engine.resize(10, 120).unwrap();
    key(&mut app, &mut engine, KeyCode::Char('s'));
    assert!(engine.runtime().sidebar_visible());
    for _ in 0..1000 {
        paint(&mut engine);
        if !engine.runtime().sidebar_pending() {
            break;
        }
    }
    assert_eq!(
        engine.runtime().reading().selection.range(),
        Some(Span::new(0, raw.len() + 1))
    );
    key(&mut app, &mut engine, KeyCode::Char('s'));
    key(&mut app, &mut engine, KeyCode::Esc);
    query(&mut app, &mut engine, "\u{301}\u{301}");
    finish_task(&mut engine);
    let first = engine
        .runtime()
        .reading()
        .current_match()
        .unwrap()
        .display
        .clone();
    paint(&mut engine);
    key(&mut app, &mut engine, KeyCode::Char('n'));
    finish_task(&mut engine);
    assert!(
        engine
            .runtime()
            .reading()
            .current_match()
            .unwrap()
            .display
            .start
            > first.start
    );
    key(&mut app, &mut engine, KeyCode::Char('N'));
    finish_task(&mut engine);
    assert_eq!(
        engine.runtime().reading().current_match().unwrap().display,
        first
    );
    paint(&mut engine);
    assert!(
        engine
            .screen()
            .front_cell(0, 0)
            .unwrap()
            .style()
            .contains(StyleRole::SEARCH)
    );
    key(&mut app, &mut engine, KeyCode::Esc);
    engine
        .restore_navigation_position(HistoryPosition {
            source: raw.len() / 2,
            row_offset: 0,
        })
        .unwrap();
    paint(&mut engine);
    assert_eq!(
        engine.screen().front_cell(0, 0).unwrap().source(),
        Some(span)
    );
    assert_eq!(
        engine.screen().front_cell(0, 0).unwrap().text().len(),
        visible.len()
    );
    // A directory activation and real Alt history must return through the
    // same refiner, retaining the full source envelope rather than its cut.
    key(&mut app, &mut engine, KeyCode::Char('s'));
    assert!(engine.runtime().sidebar_visible());
    for _ in 0..1000 {
        paint(&mut engine);
        if !engine.runtime().sidebar_pending() {
            break;
        }
    }
    key(&mut app, &mut engine, KeyCode::Tab);
    assert_eq!(app.focus(), mdtui_tui::app::FocusTarget::Sidebar);
    key(&mut app, &mut engine, KeyCode::End);
    assert!(engine.runtime().sidebar_selection().is_some());
    key(&mut app, &mut engine, KeyCode::Enter);
    paint(&mut engine);
    app.frame_committed(&engine);
    assert!(!app.history().is_empty());
    app.handle_key(&mut engine, KeyCode::Left, KeyModifiers::ALT)
        .unwrap();
    paint(&mut engine);
    assert!(
        (0..engine.screen().cols()).any(|col| engine
            .screen()
            .front_cell(0, col)
            .unwrap()
            .text()
            .len()
            == visible.len())
    );
}

#[derive(Default)]
struct ClipboardShortWriter {
    wire: Vec<u8>,
    calls: usize,
    max_request: usize,
}
impl std::io::Write for ClipboardShortWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.calls += 1;
        self.max_request = self.max_request.max(bytes.len());
        if self.calls.is_multiple_of(3) {
            return Err(std::io::ErrorKind::WouldBlock.into());
        }
        let n = bytes.len().min(2);
        self.wire.extend_from_slice(&bytes[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn giant_osc52_resumes_short_writes_and_eagain_and_cancel_closes_partial_packet() {
    let cluster = format!("a{}", "\u{301}".repeat(5000));
    let mut engine = engine(&format!("{cluster}\n"), 40, 8);
    let mut app = AppState::default();
    key(&mut app, &mut engine, KeyCode::Char('v'));
    key(&mut app, &mut engine, KeyCode::Char('y'));
    let mut writer = ClipboardShortWriter::default();
    for _ in 0..200_000 {
        engine
            .runtime_mut()
            .advance_reading(&mut writer, &mut tiny());
        if !engine.runtime().reading().pending() {
            break;
        }
    }
    assert!(!engine.runtime().reading().pending());
    assert_eq!(clipboard(&writer.wire), cluster.as_bytes());
    assert!(
        writer.max_request > 6,
        "OSC52 uses its bounded prepaid atomic packet credit"
    );
    key(&mut app, &mut engine, KeyCode::Char('y'));
    let mut partial = ClipboardShortWriter::default();
    for _ in 0..200_000 {
        engine
            .runtime_mut()
            .advance_reading(&mut partial, &mut tiny());
        if !partial.wire.is_empty() {
            break;
        }
    }
    assert!(!partial.wire.is_empty());
    assert!(!partial.wire.contains(&7));
    key(&mut app, &mut engine, KeyCode::Esc);
    for _ in 0..100 {
        engine
            .runtime_mut()
            .advance_reading(&mut partial, &mut tiny());
        if !engine.runtime().reading().copy_pending() {
            break;
        }
    }
    assert!(!engine.runtime().reading().copy_pending());
    assert!(partial.wire.ends_with(b"\x18\x1b\\"));
    assert!(!partial.wire.contains(&7));
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
        assert_eq!(
            (cell.text().collect_text(), cell.source()),
            (text.to_owned(), Some(span))
        );
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
            assert_eq!(
                cell.text().collect_text().as_bytes(),
                &b"abcdefghij"[at % 10..at % 10 + 1]
            );
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
            engine
                .screen()
                .front_cell(1, 2)
                .unwrap()
                .text()
                .collect_text(),
            engine.screen().front_cell(1, 2).unwrap().source()
        ),
        ("b".to_owned(), Some(Span::new(6, 7)))
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
                found.push((cell.text().collect_text(), cell.source()));
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
        .map(|col| {
            engine
                .screen()
                .front_cell(0, col)
                .unwrap()
                .text()
                .collect_text()
        })
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
                (!cell.is_continuation() && cell.source().is_some())
                    .then(|| cell.text().collect_text())
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
                        .then(|| (cell.text().collect_text(), cell.source()))
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
                (!cell.is_continuation() && cell.source().is_some())
                    .then(|| cell.text().collect_text())
            })
            .collect();
        assert_eq!(drawn, expected_sources);
        let row: String = (0..40)
            .map(|col| {
                engine
                    .screen()
                    .front_cell(0, col)
                    .unwrap()
                    .text()
                    .collect_text()
            })
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

#[test]
fn raw_clipboard_waits_for_a_true_giant_boundary_and_never_cancels_pure_extend() {
    let prefix = format!("a{}", "\u{301}".repeat(1000));
    let mut runtime = FrameRuntime::new(
        FrameGeometry {
            rows: 6,
            cols: 40,
            content_width: 40,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    let update = runtime.append_bytes(prefix.as_bytes()).unwrap().unwrap();
    runtime.apply_update(&update).unwrap();
    let mut engine = StreamEngine::new(runtime);
    paint(&mut engine);
    let mut app = AppState::default();
    key(&mut app, &mut engine, KeyCode::Char('v'));
    paint(&mut engine); // Explicit selection style change may repaint once.
    key(&mut app, &mut engine, KeyCode::Char('Y'));
    let mut wire = Vec::new();
    for _ in 0..32 {
        engine
            .runtime_mut()
            .frame_to(&mut wire, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap();
    }
    assert!(
        wire.is_empty(),
        "OSC52/status must wait outside the open EGC"
    );
    assert!(engine.runtime().reading().copy_pending());
    let marks = "\u{301}".repeat(128);
    let update = engine
        .runtime_mut()
        .append_bytes(marks.as_bytes())
        .unwrap()
        .unwrap();
    engine.runtime_mut().apply_update(&update).unwrap();
    for _ in 0..1000 {
        if engine
            .runtime_mut()
            .frame_to(&mut wire, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap()
            .committed
        {
            break;
        }
    }
    assert_eq!(wire, marks.as_bytes());
    let update = engine
        .runtime_mut()
        .append_bytes(b"X\n\n")
        .unwrap()
        .unwrap();
    engine.runtime_mut().apply_update(&update).unwrap();
    for _ in 0..100_000 {
        engine
            .runtime_mut()
            .frame_to(&mut wire, &mut tiny())
            .unwrap();
        if !engine.runtime().reading().copy_pending() {
            break;
        }
    }
    assert!(!engine.runtime().reading().copy_pending());
    assert_eq!(clipboard(&wire), prefix.as_bytes());
    assert_eq!(wire.iter().filter(|&&b| b == 0xcc).count(), 128);
    assert!(
        !wire.contains(&24),
        "copy did not cancel/repaint the existing glyph"
    );
}

#[test]
fn native_cold_reading_y_y_keeps_selection_direction_and_the_same_math_fold_view() {
    fn cold_engine(source: &str) -> StreamEngine {
        let mut document = mdtui3::Utf8Document::new();
        document.append(source.as_bytes()).unwrap();
        document.finish();
        let runtime = FrameRuntime::from_utf8_document(
            document,
            FrameGeometry {
                rows: 8,
                cols: 64,
                content_width: 64,
            },
            LinkMode::NoOsc8,
        )
        .unwrap();
        let mut engine = StreamEngine::new(runtime);
        paint(&mut engine);
        engine
    }
    fn settle_fold(engine: &mut StreamEngine) {
        for _ in 0..1000 {
            paint(engine);
            if !engine.runtime().fold_pending() {
                return;
            }
        }
        panic!("fold operation did not settle");
    }
    for reverse in [false, true] {
        let source = "a **b&amp;c** d\n";
        let mut engine = cold_engine(source);
        let mut app = AppState::default();
        let input = engine.runtime().document().display_source_stats();
        let shape = engine.runtime().viewport().shared_links().shape().unwrap();
        assert_eq!(shape.work().source_decode_bytes, 0);
        key(&mut app, &mut engine, KeyCode::Char('v'));
        if reverse {
            for _ in 0..6 {
                key(&mut app, &mut engine, KeyCode::Right);
            }
        }
        key(&mut app, &mut engine, KeyCode::Char(' '));
        for _ in 0..6 {
            key(
                &mut app,
                &mut engine,
                if reverse {
                    KeyCode::Left
                } else {
                    KeyCode::Right
                },
            );
        }
        assert_eq!(
            engine.runtime().reading().selection.range(),
            Some(Span::new(0, 15))
        );
        key(&mut app, &mut engine, KeyCode::Char('y'));
        assert_eq!(clipboard(&finish_task(&mut engine)), b"a b&c d");
        key(&mut app, &mut engine, KeyCode::Char('Y'));
        assert_eq!(clipboard(&finish_task(&mut engine)), b"a **b&amp;c** d");
        assert_eq!(engine.runtime().document().display_source_stats(), input);
    }
    let mut engine = cold_engine("$x+y$\n");
    let mut app = AppState::default();
    key(&mut app, &mut engine, KeyCode::Char('v'));
    key(&mut app, &mut engine, KeyCode::Char(' '));
    for _ in 0..2 {
        key(&mut app, &mut engine, KeyCode::Right);
    }
    key(&mut app, &mut engine, KeyCode::Char('y'));
    assert_eq!(clipboard(&finish_task(&mut engine)), b"x+y");
    let source = "$$\n\\sin+x\n$$\n\ntail &amp; 界\n";
    let mut engine = cold_engine(source);
    let id = mdtui3::FoldId::Math(mdtui3::BlockId(0));
    let candidate = engine.runtime().document().fold_candidate(id).unwrap();
    // The accepted native math consumer paints command aliases and copies
    // their whole source atom before the same FoldView is collapsed.
    let expanded = (0..8)
        .map(|row| {
            (0..64)
                .map(|col| {
                    engine
                        .screen()
                        .front_cell(row, col)
                        .unwrap()
                        .text()
                        .collect_text()
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(expanded.contains("tail & 界"), "{expanded}");
    assert!(expanded.contains("sin+x"), "{expanded}");
    key(&mut app, &mut engine, KeyCode::Char('v'));
    assert_eq!(
        engine.runtime().reading().selection.range(),
        Some(Span::new(3, 7))
    );
    key(&mut app, &mut engine, KeyCode::Char('y'));
    assert_eq!(clipboard(&finish_task(&mut engine)), b"sin");
    key(&mut app, &mut engine, KeyCode::Char('Y'));
    assert_eq!(clipboard(&finish_task(&mut engine)), br"\sin");
    key(&mut app, &mut engine, KeyCode::Esc);
    engine.runtime_mut().open_fold_menu(false);
    settle_fold(&mut engine);
    engine.runtime_mut().fold_menu_key(KeyCode::Right);
    engine.runtime_mut().fold_menu_key(KeyCode::Down);
    engine.runtime_mut().fold_menu_key(KeyCode::Down);
    assert!(engine.runtime_mut().fold_menu_key(KeyCode::Enter));
    settle_fold(&mut engine);
    assert!(engine.runtime().viewport().folds().is_collapsed(id));
    assert!(engine.runtime_mut().fold_menu_key(KeyCode::Esc));
    paint(&mut engine);
    key(&mut app, &mut engine, KeyCode::Home);
    paint(&mut engine);
    let version = engine.runtime().viewport().folds().version();
    key(&mut app, &mut engine, KeyCode::Char('v'));
    assert_eq!(
        engine.runtime().reading().selection.range(),
        Some(candidate.hidden)
    );
    key(&mut app, &mut engine, KeyCode::Char('y'));
    assert_eq!(
        clipboard(&finish_task(&mut engine)),
        format!(
            "[+ math 0 {}..{}]",
            candidate.source.start, candidate.source.end
        )
        .as_bytes()
    );
    key(&mut app, &mut engine, KeyCode::Char('Y'));
    assert_eq!(
        clipboard(&finish_task(&mut engine)),
        &source.as_bytes()[candidate.hidden.range()]
    );
    assert_eq!(engine.runtime().viewport().folds().version(), version);
    assert_eq!(
        engine.runtime().reading().selection.range(),
        Some(candidate.hidden)
    );
}

#[test]
fn actual_math_reading_search_and_copy_keep_post_tex_atom_root_source() {
    let source = "\\(\\alpha{}́\\)\n\nplain tail\n";
    let mut engine = engine(source, 24, 8);
    let mut app = AppState::default();
    key(&mut app, &mut engine, KeyCode::Char('v'));
    assert_eq!(
        engine.runtime().reading().selection.range(),
        Some(Span::new(2, 12))
    );
    key(&mut app, &mut engine, KeyCode::Char('y'));
    assert_eq!(clipboard(&finish_task(&mut engine)), "ά".as_bytes());
    key(&mut app, &mut engine, KeyCode::Char('Y'));
    assert_eq!(clipboard(&finish_task(&mut engine)), "\\alpha{}́".as_bytes());
    query(&mut app, &mut engine, "ά");
    finish_task(&mut engine);
    assert_eq!(
        engine.runtime().reading().current_match().unwrap().source,
        Span::new(2, 12)
    );
    let old = engine.runtime().reading().query().chars().count();
    key(&mut app, &mut engine, KeyCode::Char('/'));
    for _ in 0..old {
        key(&mut app, &mut engine, KeyCode::Backspace);
    }
    for ch in "alpha".chars() {
        key(&mut app, &mut engine, KeyCode::Char(ch));
    }
    key(&mut app, &mut engine, KeyCode::Enter);
    finish_task(&mut engine);
    assert!(engine.runtime().reading().current_match().is_none());
    let old = engine.runtime().reading().query().chars().count();
    key(&mut app, &mut engine, KeyCode::Char('/'));
    for _ in 0..old {
        key(&mut app, &mut engine, KeyCode::Backspace);
    }
    for ch in "tail".chars() {
        key(&mut app, &mut engine, KeyCode::Char(ch));
    }
    key(&mut app, &mut engine, KeyCode::Enter);
    finish_task(&mut engine);
    let hit = engine.runtime().reading().current_match().unwrap();
    assert_eq!(&source[hit.source.range()], "tail");
}
#[test]
fn one_command_source_alias_copies_every_visible_operator_glyph() {
    for (tex, visible) in [("\\sin", "sin"), ("\\unknown", "\\unknown")] {
        let mut engine = engine(&format!("\\({tex}\\)\n"), 24, 8);
        let mut app = AppState::default();
        key(&mut app, &mut engine, KeyCode::Char('v'));
        assert_eq!(
            engine.runtime().reading().selection.range(),
            Some(Span::new(2, 2 + tex.len()))
        );
        key(&mut app, &mut engine, KeyCode::Char('y'));
        assert_eq!(clipboard(&finish_task(&mut engine)), visible.as_bytes());
        key(&mut app, &mut engine, KeyCode::Char('Y'));
        assert_eq!(clipboard(&finish_task(&mut engine)), tex.as_bytes());
    }
}

#[test]
fn rendered_math_search_reveals_the_body_glyph_after_source_jump_and_resize() {
    let matrix = (0..40)
        .map(|i| {
            if i == 35 {
                r"\alpha".to_owned()
            } else {
                format!("r{i}")
            }
        })
        .collect::<Vec<_>>()
        .join(r"\\");
    for source in [
        format!("\\[\n\\begin{{matrix}}{matrix}\\end{{matrix}}\n\\]\n"),
        format!("\\[\n\\frac{{\\alpha}}{{{}}}\n\\]\n", "x".repeat(80)),
        format!("| a | b |\n| - | - |\n| \\(\\begin{{matrix}}{matrix}\\end{{matrix}}\\) | z |\n"),
        format!(
            "| a | b |\n| - | - |\n| \\({}\\alpha\\) | z |\n",
            "x".repeat(20_000)
        ),
    ] {
        let mut engine = engine(&source, 20, 6);
        let mut app = AppState::default();
        query(&mut app, &mut engine, "α");
        // This added wide case emits over 20k glyph descriptors. Keep the
        // original small-task helper's 200k assertion unchanged.
        let mut completed = false;
        for _ in 0..source.len() * 64 + 20_000 {
            let mut budget = tiny();
            engine
                .runtime_mut()
                .advance_reading(&mut Vec::new(), &mut budget);
            assert!(
                budget.used().queries <= 1
                    && budget.used().objects <= 1
                    && budget.used().bytes <= 6
            );
            if !engine.runtime().reading().pending() {
                completed = true;
                break;
            }
        }
        assert!(completed, "finite wide mathematical search");
        paint(&mut engine);
        fn revealed(engine: &StreamEngine) -> bool {
            let s = engine.screen();
            (0..s.rows() - 1).any(|row| {
                (0..s.cols()).any(|column| {
                    let cell = s.front_cell(row, column).unwrap();
                    cell.style().contains(StyleRole::SEARCH)
                        && cell.text().collect_text().contains('α')
                })
            })
        }
        assert!(
            revealed(&engine),
            "mathematical match must be drawn in the body, not only its footer"
        );
        let at = source.find(r"\alpha").unwrap();
        assert_eq!(
            engine.runtime().reading().current_match().unwrap().source,
            Span::new(at, at + 6)
        );
        assert_eq!(
            engine.runtime().top_source(),
            Some(at),
            "source focus before resize: {source:?}"
        );
        engine.runtime_mut().resize(6, 12).unwrap();
        paint(&mut engine);
        assert!(
            engine.runtime().reading().current_match().is_none(),
            "width invalidates the rendered display offset"
        );
        assert!(
            (0..5).any(|r| (0..12).any(|c| engine
                .screen()
                .front_cell(r, c)
                .unwrap()
                .text()
                .collect_text()
                .contains('α'))),
            "resize focus failed: source={source:?} top={:?} body={:?}",
            engine.runtime().top_source(),
            (0..5)
                .map(|r| (0..12)
                    .map(|c| engine
                        .screen()
                        .front_cell(r, c)
                        .unwrap()
                        .text()
                        .collect_text())
                    .collect::<String>())
                .collect::<Vec<_>>()
        );
        key(&mut app, &mut engine, KeyCode::Char('n'));
        // This added wide case emits over 20k glyph descriptors. Keep the
        // original small-task helper's 200k assertion unchanged.
        let mut completed = false;
        for _ in 0..source.len() * 64 + 20_000 {
            let mut budget = tiny();
            engine
                .runtime_mut()
                .advance_reading(&mut Vec::new(), &mut budget);
            assert!(
                budget.used().queries <= 1
                    && budget.used().objects <= 1
                    && budget.used().bytes <= 6
            );
            if !engine.runtime().reading().pending() {
                completed = true;
                break;
            }
        }
        assert!(completed, "finite wide mathematical search");
        paint(&mut engine);
        assert!(
            revealed(&engine),
            "fresh search failed: source={source:?} top={:?} match={:?} body={:?}",
            engine.runtime().top_source(),
            engine.runtime().reading().current_match(),
            (0..5)
                .map(|r| (0..12)
                    .map(|c| engine
                        .screen()
                        .front_cell(r, c)
                        .unwrap()
                        .text()
                        .collect_text())
                    .collect::<String>())
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn native_display_search_keeps_its_width_independent_match() {
    let mut engine = engine("before alpha after\n", 20, 6);
    let mut app = AppState::default();
    query(&mut app, &mut engine, "alpha");
    finish_task(&mut engine);
    paint(&mut engine);
    let original = engine.runtime().reading().current_match().unwrap().source;
    engine.runtime_mut().resize(6, 12).unwrap();
    paint(&mut engine);
    assert_eq!(
        engine.runtime().reading().current_match().unwrap().source,
        original
    );
}

#[test]
fn one_column_operator_search_and_visible_copy_keep_the_full_source_atom() {
    let mut engine = engine("\\(\\sin\\)\n", 1, 4);
    let mut app = AppState::default();
    query(&mut app, &mut engine, "n");
    finish_task(&mut engine);
    paint(&mut engine);
    assert_eq!(
        engine
            .screen()
            .front_cell(0, 0)
            .unwrap()
            .text()
            .collect_text(),
        "n"
    );
    assert_eq!(
        engine.runtime().reading().current_match().unwrap().source,
        Span::new(2, 6)
    );
    key(&mut app, &mut engine, KeyCode::Char('v'));
    key(&mut app, &mut engine, KeyCode::Char('y'));
    assert_eq!(clipboard(&finish_task(&mut engine)), b"sin");
    key(&mut app, &mut engine, KeyCode::Char('Y'));
    assert_eq!(clipboard(&finish_task(&mut engine)), br"\sin");
}

#[test]
fn mathematical_search_reveals_a_folded_owner_and_retains_other_folds() {
    use mdtui3::{FoldId, HeadingId};
    let source = "# A\n\\[\n\\frac{\\alpha}{b}\n\\]\n\n# B\nHIDDEN\n";
    let mut engine = engine(source, 20, 8);
    {
        let (doc, view) = engine.runtime_mut().document_and_viewport_mut();
        for i in 0..2 {
            view.folds_mut()
                .set_collapsed(doc, FoldId::Heading(HeadingId(i)), true)
                .unwrap();
        }
        view.fold_changed(doc);
    }
    paint(&mut engine);
    let mut app = AppState::default();
    query(&mut app, &mut engine, "α");
    finish_task(&mut engine);
    paint(&mut engine);
    let at = source.find("\\alpha").unwrap();
    assert_eq!(
        engine.runtime().reading().current_match().unwrap().source,
        Span::new(at, at + 6)
    );
    assert!(
        !engine
            .runtime()
            .viewport()
            .folds()
            .is_collapsed(FoldId::Heading(HeadingId(0)))
    );
    assert!(
        engine
            .runtime()
            .viewport()
            .folds()
            .is_collapsed(FoldId::Heading(HeadingId(1)))
    );
    assert!((0..7).any(|r| (0..20).any(|c| {
        engine
            .screen()
            .front_cell(r, c)
            .unwrap()
            .text()
            .collect_text()
            .contains('α')
    })));
}

#[test]
fn math_query_created_after_resize_before_the_next_frame_uses_the_new_width() {
    let mut engine = engine("\\(\\alpha\\)\n", 80, 24);
    let mut app = AppState::default();
    engine.runtime_mut().resize(3, 20).unwrap();
    query(&mut app, &mut engine, "α");
    finish_task(&mut engine);
    assert_eq!(
        engine.runtime().reading().current_match().unwrap().source,
        Span::new(2, 8)
    );
}

#[test]
fn frame_scheduler_searches_a_wide_math_table_after_follow_end_and_resize() {
    let source = format!(
        "| a | b |\n| - | - |\n| \\({}\\xi\\) | z |\n\nMATH_END\n",
        "x".repeat(20_000)
    );
    let mut engine = engine(&source, 80, 24);
    let mut app = AppState::default();
    key(&mut app, &mut engine, KeyCode::End);
    paint(&mut engine);
    engine.runtime_mut().resize(3, 20).unwrap();
    paint(&mut engine);
    query(&mut app, &mut engine, "ξ");
    for i in 0..10000 {
        engine
            .runtime_mut()
            .frame_to(
                &mut Vec::new(),
                &mut Budget::new(mdtui_tui::stream::FRAME_WORK),
            )
            .unwrap();
        if !engine.runtime().reading().pending() && !engine.runtime().has_pending_jump() {
            break;
        }
        assert!(
            i < 9999,
            "stalled: {}",
            engine.runtime().reading().message()
        );
    }
    assert!(
        engine.runtime().reading().current_match().is_some(),
        "message={} query={}",
        engine.runtime().reading().message(),
        engine.runtime().reading().query()
    );
    paint(&mut engine);
    assert!(
        (0..2).any(|r| (0..20).any(|c| engine
            .screen()
            .front_cell(r, c)
            .unwrap()
            .text()
            .collect_text()
            .contains('ξ'))),
        "top={:?} following={} body={:?}",
        engine.runtime().top_source(),
        engine.runtime().viewport().following(),
        (0..3)
            .map(|r| (0..20)
                .map(|c| engine
                    .screen()
                    .front_cell(r, c)
                    .unwrap()
                    .text()
                    .collect_text())
                .collect::<String>())
            .collect::<Vec<_>>()
    );
    let at = source.find(r"\xi").unwrap();
    assert_eq!(engine.runtime().top_source(), Some(at));
    engine.runtime_mut().resize(24, 80).unwrap();
    paint(&mut engine);
    assert_eq!(engine.runtime().top_source(), Some(at));
    assert!(
        (0..23).any(|r| (0..80).any(|c| engine
            .screen()
            .front_cell(r, c)
            .unwrap()
            .text()
            .collect_text()
            .contains('ξ'))),
        "source anchor must survive wide resize; top={:?}",
        engine.runtime().top_source()
    );
}

#[test]
fn eof_table_width_calibration_keeps_mathematical_source_focus_after_follow_search_and_resize() {
    use std::time::Duration;
    fn settle(engine: &mut StreamEngine, app: &mut AppState) {
        for _ in 0..20000 {
            let mut budget = Budget::new(mdtui_tui::stream::FRAME_WORK);
            if engine.calibration_pending() {
                engine.advance_calibration(&mut budget).unwrap();
            }
            let commit = engine
                .render_due(&mut Vec::new(), &mut budget, Duration::from_secs(1))
                .unwrap();
            if commit.as_ref().is_some_and(|c| c.committed) {
                app.frame_committed(engine);
            }
            if !engine.calibration_pending()
                && !engine.runtime().reading().pending()
                && !engine.runtime().has_pending_jump()
                && !engine.wire_pending()
                && commit.is_some_and(|c| c.committed)
            {
                return;
            }
        }
        panic!("stream did not settle");
    }
    let matrix = (0..40)
        .map(|i| format!("r{i}"))
        .collect::<Vec<_>>()
        .join(r"\\");
    let source = format!(
        "\\[\n\\frac{{\\alpha}}{{{}}}\n\\]\n\n\\[\n\\begin{{matrix}}{matrix}\\end{{matrix}}\n\\]\n\n| a | b |\n| - | - |\n| \\({}\\xi\\) | z |\n\nMATH_END\n",
        "x".repeat(100),
        "x".repeat(20_000)
    );
    let mut engine = engine(&source, 80, 24);
    let mut app = AppState::default();
    engine.finish_input(Duration::ZERO).unwrap();
    settle(&mut engine, &mut app);
    query(&mut app, &mut engine, "r35");
    settle(&mut engine, &mut app);
    key(&mut app, &mut engine, KeyCode::End);
    settle(&mut engine, &mut app);
    app.resize(&mut engine, 3, 20).unwrap();
    settle(&mut engine, &mut app);
    key(&mut app, &mut engine, KeyCode::Char('/'));
    for _ in 0..3 {
        key(&mut app, &mut engine, KeyCode::Backspace);
    }
    key(&mut app, &mut engine, KeyCode::Char('ξ'));
    key(&mut app, &mut engine, KeyCode::Enter);
    settle(&mut engine, &mut app);
    let at = source.find(r"\xi").unwrap();
    assert_eq!(engine.runtime().top_source(), Some(at));
    app.resize(&mut engine, 24, 80).unwrap();
    settle(&mut engine, &mut app);
    assert_eq!(
        engine.runtime().top_source(),
        Some(at),
        "EOF calibration retains the whole command source"
    );
    assert!((0..23).any(|r| (0..80).any(|c| {
        engine
            .screen()
            .front_cell(r, c)
            .unwrap()
            .text()
            .collect_text()
            .contains('ξ')
    })));
}

#[test]
fn an_unrelated_math_object_preserves_ordinary_hit_source_and_resize_lifetime() {
    for prefix in [
        format!("# {}HIDDEN_HIT\n\n", "x".repeat(80)),
        format!("```\n{}HIDDEN_HIT\n```\n\n", "x".repeat(80)),
        "ordinary HIDDEN_HIT text\n\n".to_owned(),
    ] {
        for suffix in ["", "\\(x\\)\n"] {
            let source = format!("{prefix}{suffix}");
            let mut engine = engine(&source, 20, 6);
            let mut app = AppState::default();
            query(&mut app, &mut engine, "HIDDEN_HIT");
            finish_task(&mut engine);
            paint(&mut engine);
            let at = source.find("HIDDEN_HIT").unwrap();
            let expected = Span::new(at, at + 10);
            assert_eq!(
                engine.runtime().reading().current_match().unwrap().source,
                expected
            );
            engine.runtime_mut().resize(6, 12).unwrap();
            paint(&mut engine);
            assert_eq!(
                engine.runtime().reading().current_match().unwrap().source,
                expected
            );
        }
    }
}

#[test]
fn cold_native_prefix_then_live_suffix_keeps_entity_source_range_and_copy() {
    let prefix = format!("{}\n\n", "ab".repeat(16384));
    let suffix = "tail &amp; 界\n";
    let mut document = mdtui3::Utf8Document::new();
    document.append(prefix.as_bytes()).unwrap();
    let runtime = FrameRuntime::from_utf8_document(
        document,
        FrameGeometry {
            rows: 8,
            cols: 64,
            content_width: 64,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    let mut engine = StreamEngine::new(runtime);
    paint(&mut engine);
    let shape = engine.runtime().viewport().shared_links().shape().unwrap();
    assert_eq!(shape.work().source_decode_bytes, 0);
    assert_eq!(shape.work().source_indexed_bytes, prefix.len());
    let update = engine
        .runtime_mut()
        .append_bytes(suffix.as_bytes())
        .unwrap()
        .unwrap();
    engine.runtime_mut().apply_update(&update).unwrap();
    let update = engine.runtime_mut().finish_input().unwrap().unwrap();
    engine.runtime_mut().apply_update(&update).unwrap();
    paint(&mut engine);
    let mut app = AppState::default();
    query(&mut app, &mut engine, "&");
    finish_task(&mut engine);
    paint(&mut engine);
    let selected = Span::new(prefix.len() + 5, prefix.len() + 10);
    assert_eq!(
        engine.runtime().reading().current_match().unwrap().source,
        selected
    );
    key(&mut app, &mut engine, KeyCode::Char('v'));
    for _ in 0..5 {
        key(&mut app, &mut engine, KeyCode::Right);
    }
    assert_eq!(engine.runtime().reading().selection.range(), Some(selected));
    key(&mut app, &mut engine, KeyCode::Char('y'));
    assert_eq!(clipboard(&finish_task(&mut engine)), b"&");
    key(&mut app, &mut engine, KeyCode::Char('Y'));
    assert_eq!(clipboard(&finish_task(&mut engine)), b"&amp;");
    assert_eq!(shape.work().source_decode_bytes, suffix.len());
    assert_eq!(shape.work().source_indexed_bytes, prefix.len());
}
