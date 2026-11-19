use crossterm::event::{KeyCode, KeyModifiers};
use mdtui_tui::{
    cli::LinkMode,
    render::frame::{FrameGeometry, FrameRuntime},
};
use mdtui3::{Budget, WorkLimit};
fn ready(r: &mut FrameRuntime) {
    for _ in 0..10000 {
        if r.frame_to(&mut vec![], &mut Budget::new(WorkLimit::unlimited()))
            .unwrap()
            .committed
        {
            return;
        }
    }
    panic!("no note frame");
}
fn key(r: &mut FrameRuntime, k: KeyCode) {
    assert!(r.footnote_key(k, KeyModifiers::NONE, true).unwrap());
}
#[test]
fn note_query_keeps_the_256_byte_contract_and_both_copy_modes_reject_more_than_64kib() {
    let cluster = format!("a{}", "\u{301}".repeat(32768));
    let mut r = FrameRuntime::from_finished_source(
        &format!("See [^n].\n\n[^n]: {cluster}\n"),
        FrameGeometry {
            rows: 6,
            cols: 20,
            content_width: 20,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    ready(&mut r);
    key(&mut r, KeyCode::Char('f'));
    ready(&mut r);
    key(&mut r, KeyCode::Enter);
    ready(&mut r);
    key(&mut r, KeyCode::Char('/'));
    for _ in 0..100 {
        key(&mut r, KeyCode::Char('中'));
    }
    key(&mut r, KeyCode::Enter);
    ready(&mut r);
    assert_eq!(r.footnote_reading().unwrap().query().len(), 255);
    key(&mut r, KeyCode::Esc);
    ready(&mut r);
    key(&mut r, KeyCode::Char('v'));
    ready(&mut r);
    for copy in ['y', 'Y'] {
        key(&mut r, KeyCode::Char(copy));
        let mut wire = vec![];
        for _ in 0..10000 {
            r.frame_to(&mut wire, &mut Budget::new(WorkLimit::unlimited()))
                .unwrap();
            if !r.footnote_reading().unwrap().copy_pending() {
                break;
            }
        }
        assert_eq!(
            r.footnote_reading().unwrap().message(),
            "copy too large (64 KiB limit)"
        );
        assert!(!wire.windows(5).any(|w| w == b"\x1b]52;"));
    }
}
