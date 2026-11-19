use crossterm::event::{KeyCode, KeyModifiers};
use mdtui_tui::{
    app::AppState,
    cli::LinkMode,
    render::frame::{FrameGeometry, FrameRuntime},
    stream::{LoadEvent, Reader, StreamEngine},
};
use mdtui3::{Budget, WorkLimit};
use std::{
    collections::VecDeque,
    io::{self, Read},
    time::Duration,
};
fn one() -> Budget {
    Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 6,
        tiles: 1,
        recovery: 1,
        dependencies: 1,
    })
}
fn settle(e: &mut StreamEngine, a: &mut AppState) {
    for _ in 0..10000 {
        if e.calibration_pending() {
            e.advance_calibration(&mut Budget::new(WorkLimit::unlimited()))
                .unwrap();
        }
        if !e.due(Duration::ZERO) {
            return;
        }
        if let Some(c) = e
            .render_due(
                &mut vec![],
                &mut Budget::new(WorkLimit::unlimited()),
                Duration::ZERO,
            )
            .unwrap()
            && c.committed
        {
            a.frame_committed(e);
        }
    }
    panic!("engine protocol did not settle");
}
struct Chunks(VecDeque<Vec<u8>>);
impl Read for Chunks {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let Some(chunk) = self.0.pop_front() else {
            return Ok(0);
        };
        out[..chunk.len()].copy_from_slice(&chunk);
        Ok(chunk.len())
    }
}
#[test]
fn queued_utf8_suffix_and_eof_are_unconsumed_until_note_seed_finishes_and_controls_remain_live() {
    let runtime = FrameRuntime::new(
        FrameGeometry {
            rows: 8,
            cols: 40,
            content_width: 40,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    let mut e = StreamEngine::new(runtime);
    let mut a = AppState::default();
    let mut prefix = b"See [^n].\n\n[^n]: **one**\n    tail".to_vec();
    prefix.extend_from_slice(&[0xf0, 0x9f]);
    e.ingest_data(&prefix, Duration::ZERO).unwrap();
    settle(&mut e, &mut a);
    a.handle_key(&mut e, KeyCode::Char('f'), KeyModifiers::NONE)
        .unwrap();
    settle(&mut e, &mut a);
    a.handle_key(&mut e, KeyCode::Enter, KeyModifiers::NONE)
        .unwrap();
    e.runtime_mut().frame_to(&mut vec![], &mut one()).unwrap();
    assert!(!e.runtime().input_ready() && e.runtime().fold_busy());
    let suffix = vec![0x98, 0x80, b'\n'];
    let mut reader = Reader::spawn(Chunks(VecDeque::from([suffix.clone()]))).unwrap();
    let before = e.document().source().to_owned();
    let metrics = e.metrics().read_bytes;
    // Same readiness gate as the real terminal loop, before Reader::try_recv.
    assert!(e.ingest_data(&suffix, Duration::ZERO).is_err());
    assert!(e.finish_input(Duration::ZERO).is_err());
    assert!(e.runtime_mut().finish_input().is_err());
    assert_eq!(e.document().source(), before);
    assert_eq!(e.metrics().read_bytes, metrics);
    assert!(!e.input_finished() && !e.document().is_input_sealed());
    assert!(
        a.handle_key(&mut e, KeyCode::Char('q'), KeyModifiers::NONE)
            .unwrap()
            .quit
    );
    e.resize(7, 12).unwrap();
    assert!(!e.runtime().input_ready());
    let mut initialized = false;
    for _ in 0..10000 {
        if e.runtime_mut().initialize(&mut one()).unwrap() {
            initialized = true;
            break;
        }
    }
    assert!(initialized && e.runtime().input_ready());
    let LoadEvent::Data(bytes) = reader.recv_timeout(Duration::from_secs(2)).unwrap() else {
        panic!()
    };
    assert_eq!(bytes, suffix);
    e.ingest_data(&bytes, Duration::ZERO).unwrap();
    assert!(matches!(
        reader.recv_timeout(Duration::from_secs(2)).unwrap(),
        LoadEvent::Eof
    ));
    e.finish_input(Duration::ZERO).unwrap();
    reader.close().unwrap();
    settle(&mut e, &mut a);
    let mut expected = prefix;
    expected.extend_from_slice(&suffix);
    assert_eq!(e.document().source().as_bytes(), expected);
    assert_eq!(e.metrics().read_bytes, expected.len());
    assert!(e.input_finished());
    assert_eq!(e.screen().cols(), 12);
    assert_eq!(e.screen().rows(), 7);
    let grid = (0..6)
        .map(|y| {
            e.screen()
                .front_row(y)
                .unwrap()
                .iter()
                .map(|c| c.text().collect_text())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(grid.contains("tail😀"), "{grid}");
    assert!(!grid.contains('�'));
    assert_eq!(
        e.runtime()
            .footnote_stats()
            .unwrap()
            .shape
            .source_decode_bytes,
        0
    );
}
