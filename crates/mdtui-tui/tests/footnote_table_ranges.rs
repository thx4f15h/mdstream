use crossterm::event::{KeyCode, KeyModifiers};
use mdtui_tui::{
    cli::LinkMode,
    render::frame::{FrameGeometry, FrameRuntime},
};
use mdtui3::{Budget, WorkLimit};
fn ready(r: &mut FrameRuntime) -> String {
    for _ in 0..100_000 {
        let mut b = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 6,
            tiles: 1,
            recovery: 1,
            dependencies: 1,
        });
        let frame = r.frame_to(&mut vec![], &mut b).unwrap();
        assert!(b.used().queries <= 1 && b.used().objects <= 1 && b.used().bytes <= 6);
        if frame.committed && !r.fold_pending() {
            return (0..r.screen().rows())
                .map(|i| {
                    r.screen()
                        .front_row(i)
                        .unwrap()
                        .iter()
                        .map(|c| c.text().collect_text())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n");
        }
    }
    panic!(
        "note row microbudget stalled: owner={:?} fold={} jump={} reading={} stats={:?}",
        r.footnote_surface(),
        r.fold_pending(),
        r.has_pending_jump(),
        r.reading_pending(),
        r.footnote_stats()
    );
}
fn key(r: &mut FrameRuntime, k: KeyCode) -> String {
    assert!(r.footnote_key(k, KeyModifiers::NONE, true).unwrap());
    ready(r)
}
fn rows(r: &mut FrameRuntime, index: usize) -> String {
    key(r, KeyCode::Char('z'));
    key(r, KeyCode::Char('t'));
    for _ in 0..index {
        key(r, KeyCode::Down);
    }
    key(r, KeyCode::Enter)
}
fn identity(grid: &str) -> String {
    grid.split_whitespace()
        .find(|s| s.starts_with('R') && s.chars().nth(1).is_some_and(|c| c.is_ascii_digit()))
        .expect(grid)
        .chars()
        .take_while(|c| *c == 'R' || c.is_ascii_digit())
        .collect()
}
#[test]
fn real_note_row_ids_expansion_and_private_new_only_policy_survive_root_and_other_note() {
    let mut r = FrameRuntime::new(
        FrameGeometry {
            rows: 24,
            cols: 100,
            content_width: 100,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    let u = r.append_bytes(b"See [^a] and [^b].\n\n[^b]: B\n[^a]: | H |\n    | - |\n    | OLD_ONE |\n    | OLD_TWO |\n    | OLD_THREE |\n").unwrap().unwrap();
    r.apply_update(&u).unwrap();
    ready(&mut r);
    key(&mut r, KeyCode::Char('f'));
    key(&mut r, KeyCode::Enter);
    let a = r.footnote_surface().unwrap();
    let id = identity(&rows(&mut r, 1));
    // The normal Open action still targets this actual stable row registration.
    key(&mut r, KeyCode::Left);
    key(&mut r, KeyCode::Down);
    key(&mut r, KeyCode::Enter);
    key(&mut r, KeyCode::Esc);
    let grid = ready(&mut r);
    assert!(
        grid.contains("OLD_TWO") && grid.contains("OLD_THREE"),
        "{grid}"
    );
    rows(&mut r, 6);
    key(&mut r, KeyCode::Esc);
    // A->root->A keeps the one active owner and logical registration.
    key(&mut r, KeyCode::Char('b'));
    key(&mut r, KeyCode::Enter);
    assert!(r.footnote_surface().is_none());
    key(&mut r, KeyCode::Char('f'));
    key(&mut r, KeyCode::Home);
    key(&mut r, KeyCode::Enter);
    assert_eq!(r.footnote_surface(), Some(a));
    assert_eq!(identity(&rows(&mut r, 15)), id);
    key(&mut r, KeyCode::Esc);
    // A->B retires A's actual renderer. New A input is parsed only by its native parser.
    key(&mut r, KeyCode::Char('f'));
    key(&mut r, KeyCode::Down);
    key(&mut r, KeyCode::Enter);
    assert_ne!(r.footnote_surface(), Some(a));
    let before = r.footnote_stats().unwrap();
    let u = r
        .append_bytes(b"\n    next\n\n    | U |\n    | - |\n    | NEW_ONE |\n    | NEW_TWO |\n")
        .unwrap()
        .unwrap();
    r.apply_update(&u).unwrap();
    ready(&mut r);
    key(&mut r, KeyCode::Char('f'));
    key(&mut r, KeyCode::Home);
    let grid = key(&mut r, KeyCode::Enter);
    assert_eq!(r.footnote_surface(), Some(a));
    assert!(
        grid.contains("OLD_TWO") && grid.contains("OLD_THREE") && grid.contains("NEW_ONE"),
        "{grid}"
    );
    // A still-provisional physical line is not an indexed row and stays visible.
    assert!(grid.contains("NEW_TWO"), "{grid}");
    r.document()
        .footnote_body(a.symbol())
        .unwrap()
        .with_renderer(a, r.document().revision(), |native| {
            assert_eq!(native.table(mdtui3::TableId(1)).unwrap().rows().len(), 2);
        })
        .unwrap();
    let u = r.append_bytes(b"    \n").unwrap().unwrap();
    r.apply_update(&u).unwrap();
    let grid = ready(&mut r);
    assert!(
        grid.contains("OLD_THREE") && grid.contains("NEW_ONE") && !grid.contains("NEW_TWO"),
        "{grid}"
    );
    let status = rows(&mut r, 15);
    assert_eq!(identity(&status), id);
    assert!(status.contains(&format!("{id}C")), "{status}");
    key(&mut r, KeyCode::Esc);
    let after = r.footnote_stats().unwrap();
    assert_eq!(after.note_creations, 3);
    assert!(after.note_retirements > before.note_retirements);
    // Sealing the host preserves the same native view and displayed row choices.
    let u = r.finish_input().unwrap().unwrap();
    r.apply_update(&u).unwrap();
    let grid = ready(&mut r);
    assert!(
        grid.contains("OLD_THREE") && grid.contains("NEW_ONE") && !grid.contains("NEW_TWO"),
        "{grid}"
    );
}

#[test]
fn input_between_paid_note_construction_and_seed_completion_keeps_a_real_native_index() {
    let mut r = FrameRuntime::new(
        FrameGeometry {
            rows: 8,
            cols: 40,
            content_width: 40,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    let u = r
        .append_bytes(b"See [^n].\n\n[^n]: **one**\n    tail")
        .unwrap()
        .unwrap();
    r.apply_update(&u).unwrap();
    for _ in 0..100 {
        if r.frame_to(&mut vec![], &mut Budget::new(WorkLimit::unlimited()))
            .unwrap()
            .committed
        {
            break;
        }
    }
    r.footnote_key(KeyCode::Char('f'), KeyModifiers::NONE, true)
        .unwrap();
    r.frame_to(&mut vec![], &mut Budget::new(WorkLimit::unlimited()))
        .unwrap();
    r.footnote_key(KeyCode::Enter, KeyModifiers::NONE, true)
        .unwrap();
    r.frame_to(
        &mut vec![],
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
    let len = r.document().source().len();
    assert!(r.fold_busy());
    assert!(matches!(
        r.append_bytes(b"X"),
        Err(mdtui_tui::render::frame::FrameError::InitializationPending)
    ));
    assert_eq!(r.document().source().len(), len);
    let mut initialized = false;
    for _ in 0..10_000 {
        if r.initialize(&mut Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 6,
            tiles: 1,
            recovery: 1,
            dependencies: 1,
        }))
        .unwrap()
        {
            initialized = true;
            break;
        }
    }
    assert!(initialized);
    let u = r.append_bytes(b"X").unwrap().unwrap();
    r.apply_update(&u).unwrap();
    // The outer line is admitted at its physical LF; retry bytes are preserved.
    let u = r.append_bytes(b"\n").unwrap().unwrap();
    r.apply_update(&u).unwrap();
    let grid = ready(&mut r);
    assert!(grid.contains("tailX"), "{grid}");
    assert_eq!(r.footnote_stats().unwrap().shape.source_decode_bytes, 0);
}
