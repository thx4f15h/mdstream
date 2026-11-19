use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind};
use mdtui_tui::image::input::{GraphicsReply, Input, Parser};
use std::time::{Duration, Instant};
fn events(p: &mut Parser, b: &[u8]) -> Vec<Input> {
    let now = Instant::now();
    b.iter().filter_map(|v| p.feed(*v, now)).collect()
}
fn key(c: KeyCode, m: KeyModifiers) -> Input {
    Input::Key(KeyEvent::new(c, m))
}
#[test]
fn split_keys_unicode_modifiers_escape_and_resize_protocol() {
    let mut p = Parser::default();
    for (bytes, expected) in [
        (
            b"\x1b[1;7D".as_slice(),
            key(KeyCode::Left, KeyModifiers::ALT | KeyModifiers::CONTROL),
        ),
        (b"\x1b[Z", key(KeyCode::BackTab, KeyModifiers::SHIFT)),
        (b"\x1b[[E", key(KeyCode::F(5), KeyModifiers::NONE)),
        (b"\x1bOP", key(KeyCode::F(1), KeyModifiers::NONE)),
        (b"\x1b[5~", key(KeyCode::PageUp, KeyModifiers::NONE)),
        (
            "\x1bé".as_bytes(),
            key(KeyCode::Char('é'), KeyModifiers::ALT),
        ),
        (
            "界".as_bytes(),
            key(KeyCode::Char('界'), KeyModifiers::NONE),
        ),
        ("É".as_bytes(), key(KeyCode::Char('É'), KeyModifiers::SHIFT)),
        (b"Y", key(KeyCode::Char('Y'), KeyModifiers::SHIFT)),
        (b"\x03", key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        (b"\x08", key(KeyCode::Char('h'), KeyModifiers::CONTROL)),
        (b"\x1c", key(KeyCode::Char('4'), KeyModifiers::CONTROL)),
        (b"\n", key(KeyCode::Char('j'), KeyModifiers::CONTROL)),
        (b"\r", key(KeyCode::Enter, KeyModifiers::NONE)),
        (b"\t", key(KeyCode::Tab, KeyModifiers::NONE)),
        (b"\x7f", key(KeyCode::Backspace, KeyModifiers::NONE)),
    ] {
        assert_eq!(events(&mut p, bytes), vec![expected], "{bytes:?}");
    }
    let now = Instant::now();
    assert_eq!(p.feed(27, now), None);
    assert_eq!(p.expire(now + Duration::from_millis(19)), None);
    assert_eq!(
        p.expire(now + Duration::from_millis(21)),
        Some(key(KeyCode::Esc, KeyModifiers::NONE))
    );
    assert_eq!(
        events(&mut p, b"\x1b[6;16;8t"),
        vec![Input::Cell(mdtui_tui::image::worker::CellPixels {
            width: 8,
            height: 16
        })]
    );
    assert!(p.pending_capacity() <= 512);
}
#[test]
fn replies_paste_and_oversized_apc_never_turn_into_application_keys() {
    let mut p = Parser::default();
    assert_eq!(
        events(&mut p, b"\x1b_Gi=31;OK\x1b\\q"),
        vec![
            Input::Graphics(GraphicsReply {
                image: 31,
                placement: None,
                ok: true,
                missing: false
            }),
            key(KeyCode::Char('q'), KeyModifiers::NONE)
        ]
    );
    assert_eq!(
        events(&mut p, b"\x1b_Gi=92,p=3;ENOENT: gone\x1b\\"),
        vec![Input::Graphics(GraphicsReply {
            image: 92,
            placement: Some(3),
            ok: false,
            missing: true
        })]
    );
    assert_eq!(events(&mut p, b"\x1b[?1;2c"), vec![Input::Device]);
    assert_eq!(
        events(&mut p, b"\x1b[200~qq\x03\x1b[201~z"),
        vec![Input::Ignored, key(KeyCode::Char('z'), KeyModifiers::NONE)]
    );
    let mut bad = b"\x1b_G".to_vec();
    bad.extend(std::iter::repeat_n(b'q', 2000));
    bad.extend(b"\x1b\\q");
    let e = events(&mut p, &bad);
    assert_eq!(
        e,
        vec![Input::Ignored, key(KeyCode::Char('q'), KeyModifiers::NONE)]
    );
    assert_eq!(p.pending_capacity(), 512);
    // Invalid UTF8 and malformed CSI are bounded and do not panic.
    assert_eq!(
        events(&mut p, b"\xff\x1b[999999999999999999999999999999Aq").last(),
        Some(&key(KeyCode::Char('q'), KeyModifiers::NONE))
    );
}
#[test]
fn mouse_and_existing_csi_u_character_events() {
    let mut p = Parser::default();
    let e = events(&mut p, b"\x1b[<28;4;5M");
    let Input::Mouse(m) = e[0] else { panic!() };
    assert_eq!((m.column, m.row), (3, 4));
    assert_eq!(
        m.modifiers,
        KeyModifiers::SHIFT | KeyModifiers::ALT | KeyModifiers::CONTROL
    );
    assert!(matches!(m.kind, MouseEventKind::Down(_)));
    let e = events(&mut p, b"\x1b[<35;4;5M");
    assert!(matches!(e[0],Input::Mouse(m) if m.kind==MouseEventKind::Moved));
    let e = events(&mut p, b"\x1b[<66;4;5M");
    assert!(matches!(e[0],Input::Mouse(m) if m.kind==MouseEventKind::ScrollLeft));
    assert_eq!(
        events(&mut p, b"\x1b[99;5u"),
        vec![key(KeyCode::Char('c'), KeyModifiers::CONTROL)]
    );
    assert_eq!(
        events(&mut p, b"\x1b[97:65;2u"),
        vec![key(KeyCode::Char('A'), KeyModifiers::NONE)]
    );
    assert_eq!(
        events(&mut p, b"\x1b[113;1:3u"),
        vec![Input::Key(KeyEvent::new_with_kind(
            KeyCode::Char('q'),
            KeyModifiers::NONE,
            KeyEventKind::Release
        ))]
    );
}
