use crossterm::event::{KeyCode, KeyModifiers};
use mdtui_tui::{
    cli::LinkMode,
    render::frame::{FrameGeometry, FrameRuntime},
};
use mdtui3::{Budget, WorkLimit};
fn settle(r: &mut FrameRuntime) -> String {
    for _ in 0..100_000 {
        let mut b = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 6,
            tiles: 1,
            recovery: 1,
            dependencies: 1,
        });
        let c = r.frame_to(&mut vec![], &mut b).unwrap();
        if c.committed && !r.reading_pending() && !r.fold_pending() {
            return (0..r.screen().rows() - 1)
                .map(|y| {
                    r.screen()
                        .front_row(y)
                        .unwrap()
                        .iter()
                        .map(|c| c.text().collect_text())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n");
        }
    }
    panic!("finite note math focus");
}
fn key(r: &mut FrameRuntime, k: KeyCode) -> String {
    assert!(r.footnote_key(k, KeyModifiers::NONE, true).unwrap());
    settle(r)
}
#[test]
fn asynchronous_math_focus_changes_the_clipped_note_grid_at_the_same_source_and_top() {
    // All rule glyphs belong to the same frac source atom. Its denominator beta
    // is a distinct, initially clipped witness for the intrinsic horizontal view.
    let source = format!(
        "See [^n].\n\n[^n]: \\[\n    \\frac{{a}}{{{}\\beta{{}}{}}}\n    \\]\n",
        "x".repeat(40),
        "x".repeat(39)
    );
    let mut r = FrameRuntime::from_finished_source(
        &source,
        FrameGeometry {
            rows: 6,
            cols: 16,
            content_width: 16,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    settle(&mut r);
    key(&mut r, KeyCode::Char('f'));
    let initial = key(&mut r, KeyCode::Enter);
    assert!(!initial.contains('β'), "{initial}");
    key(&mut r, KeyCode::Char('/'));
    key(&mut r, KeyCode::Char('─'));
    key(&mut r, KeyCode::Enter);
    let first = r
        .footnote_reading()
        .unwrap()
        .current_match()
        .unwrap()
        .source;
    let first_position = r.surface_position();
    let mut grid = String::new();
    for _ in 0..40 {
        grid = key(&mut r, KeyCode::Char('n'));
    }
    assert_eq!(
        r.footnote_reading()
            .unwrap()
            .current_match()
            .unwrap()
            .source,
        first
    );
    assert_eq!(r.surface_position(), first_position);
    assert!(
        grid.contains('β'),
        "later intrinsic focus must draw current math view: {grid}"
    );
    let mut found = false;
    for y in 0..r.screen().rows() - 1 {
        for x in 0..r.screen().cols() {
            if r.screen()
                .front_cell(y, x)
                .unwrap()
                .text()
                .collect_text()
                .contains('β')
            {
                assert_eq!(r.footnote_source_at(y, x).unwrap().body, first.body);
                found = true;
            }
        }
    }
    assert!(found);
    key(&mut r, KeyCode::Char('/'));
    key(&mut r, KeyCode::Backspace);
    key(&mut r, KeyCode::Char('β'));
    assert!(
        r.footnote_key(KeyCode::Enter, KeyModifiers::NONE, true)
            .unwrap()
    );
    let mut pending = false;
    for _ in 0..100_000 {
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
        if r.footnote_reading().unwrap().current_match().is_some() && r.fold_pending() {
            pending = true;
            break;
        }
    }
    assert!(
        pending,
        "resize must interrupt the actual source-refinement phase"
    );
    r.resize(6, 12).unwrap();
    assert!(settle(&mut r).contains('β'));
}

#[test]
fn unknown_controlword_is_kept_literal_instead_of_accepting_a_known_prefix() {
    let mut r = FrameRuntime::from_finished_source(
        "See [^n].\n\n[^n]: \\(\\betaxxx\\)\n",
        FrameGeometry {
            rows: 6,
            cols: 40,
            content_width: 40,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    settle(&mut r);
    key(&mut r, KeyCode::Char('f'));
    let grid = key(&mut r, KeyCode::Enter);
    assert!(grid.contains("\\betaxxx") && !grid.contains('β'), "{grid}");
}
