use crossterm::event::{KeyCode, KeyModifiers};
use mdtui_tui::{
    cli::LinkMode,
    render::{
        first_frame::{FirstFrame, FirstFrameConfig, FirstFrameStatus},
        frame::{FrameGeometry, FrameRuntime},
        viewport::{ViewportConfig, ViewportFrameStatus, ViewportState},
    },
};
use mdtui3::{Budget, Document, TileId, WorkLimit};
fn budget() -> Budget {
    Budget::new(WorkLimit::unlimited())
}
fn first(d: &Document) -> FirstFrame {
    let mut f = FirstFrame::new(
        d,
        FirstFrameConfig {
            top_tile: TileId(0),
            tile_limit: d.tile_count().max(1),
            width: 40,
            row_limit: 100000,
        },
    )
    .unwrap();
    for _ in 0..10000 {
        if matches!(
            f.advance(d, &mut budget()).unwrap().status,
            FirstFrameStatus::Covered | FirstFrameStatus::EndOfDocument
        ) {
            return f;
        }
    }
    panic!("no first frame")
}
fn ready(v: &mut ViewportState, d: &Document) -> Vec<String> {
    for _ in 0..10000 {
        let f = v.frame(d, &mut budget()).unwrap();
        if matches!(
            f.status,
            ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument
        ) {
            return f
                .lines
                .iter()
                .map(|l| l.cells.iter().map(|c| c.text.collect_text()).collect())
                .collect();
        }
    }
    panic!("no viewport")
}
#[test]
fn actual_producer_code_lines_whole_middle_and_end() {
    let source = (0..900)
        .map(|i| format!("    row{i:04}\r\n\r\n"))
        .collect::<String>()
        + "    last\n";
    let mut d = Document::new();
    d.append(&source).unwrap();
    d.finish();
    assert_eq!(d.blocks().len(), 1);
    let before = d.work();
    let f = first(&d);
    let whole = f
        .lines()
        .iter()
        .map(|l| {
            l.cells
                .iter()
                .map(|c| c.text.collect_text())
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    assert_eq!(whole.len(), 1801);
    for (i, line) in whole.iter().enumerate() {
        let expected = if i == 1800 {
            "  last".to_owned()
        } else if i % 2 == 0 {
            format!("  row{:04}", i / 2)
        } else {
            String::new()
        };
        assert_eq!(line, &format!("{expected:<40}"));
    }
    let mut v = ViewportState::from_first_frame(
        &d,
        f,
        ViewportConfig {
            width: 40,
            rows: 5,
            overscan_rows: 2,
            max_tiles: 64,
            max_replay_bytes: 256 * 1024,
            row_slack: 32,
        },
    )
    .unwrap();
    assert_eq!(ready(&mut v, &d), whole[..5]);
    v.scroll_down(&d, 700).unwrap();
    assert_eq!(ready(&mut v, &d), whole[700..705]);
    v.end(&d).unwrap();
    assert_eq!(ready(&mut v, &d), whole[1796..]);
    assert_eq!(before, d.work());
}
fn append(r: &mut FrameRuntime, bytes: &[u8]) {
    let update = r.append_bytes(bytes).unwrap().unwrap();
    r.apply_update(&update).unwrap();
}
fn paint(r: &mut FrameRuntime) {
    for _ in 0..10000 {
        if r.frame_to(&mut Vec::new(), &mut budget())
            .unwrap()
            .committed
        {
            return;
        }
    }
    panic!("no frame")
}
fn screen(r: &FrameRuntime) -> String {
    (0..7)
        .map(|y| {
            (0..40)
                .map(|x| r.screen().front_cell(y, x).unwrap().text().collect_text())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
#[test]
fn pending_quote_blanks_are_withheld_through_end_follow_and_eof() {
    let mut r = FrameRuntime::new(
        FrameGeometry {
            rows: 8,
            cols: 40,
            content_width: 40,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    append(&mut r, b">     a\n");
    paint(&mut r);
    let before = screen(&r);
    append(&mut r, b">\n>\t\n");
    assert!(r.document().indented_code_pending().is_some());
    r.seek_end(true).unwrap();
    paint(&mut r);
    assert_eq!(screen(&r), before);
    assert!(!screen(&r).contains('>'));
    append(&mut r, b">     b\n");
    paint(&mut r);
    assert!(r.document().indented_code_pending().is_none());
    let f = first(r.document());
    assert_eq!(f.lines().len(), 4);
    assert!(screen(&r).contains('b'));
    append(&mut r, b">\n>\n");
    paint(&mut r);
    let before = screen(&r);
    let update = r.finish_input().unwrap().unwrap();
    r.apply_update(&update).unwrap();
    paint(&mut r);
    assert_eq!(screen(&r), before);
    assert!(r.document().indented_code_pending().is_none());
}
fn clipboard(r: &mut FrameRuntime, key: char) -> Vec<u8> {
    r.reading_key(KeyCode::Char(key), KeyModifiers::NONE, true)
        .unwrap();
    let mut bytes = Vec::new();
    for _ in 0..200000 {
        let mut b = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 6,
            tiles: 1,
            recovery: 1,
            dependencies: 1,
        });
        r.advance_reading(&mut bytes, &mut b);
        if !r.reading().pending() {
            break;
        }
    }
    let start = bytes
        .windows(7)
        .position(|b| b == b"\x1b]52;c;")
        .expect("OSC52")
        + 7;
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let mut bits = 0u32;
    let mut count = 0;
    for &b in &bytes[start..] {
        if b == 7 || b == b'=' {
            break;
        }
        bits = (bits << 6) | alphabet.iter().position(|&c| c == b).unwrap() as u32;
        count += 6;
        if count >= 8 {
            count -= 8;
            out.push((bits >> count) as u8)
        }
    }
    out
}
#[test]
fn actual_source_and_visible_clipboards_keep_internal_blank() {
    let mut r = FrameRuntime::from_finished_source(
        "    a\r\n\r\n    b\n",
        FrameGeometry {
            rows: 8,
            cols: 40,
            content_width: 40,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    paint(&mut r);
    for key in [
        KeyCode::Char('v'),
        KeyCode::Char(' '),
        KeyCode::Down,
        KeyCode::Down,
    ] {
        r.reading_key(key, KeyModifiers::NONE, true).unwrap();
        paint(&mut r)
    }
    assert_eq!(
        r.reading().selection.range(),
        Some(mdtui3::Span::new(4, 14))
    );
    assert_eq!(clipboard(&mut r, 'Y'), b"a\r\n\r\n    b");
    assert_eq!(clipboard(&mut r, 'y'), b"a\n\nb");
}
