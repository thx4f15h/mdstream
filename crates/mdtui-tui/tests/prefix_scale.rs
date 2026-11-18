use crossterm::event::KeyCode;
use mdtui_tui::{
    cli::LinkMode,
    render::{
        frame::{FrameGeometry, FrameRuntime},
        objects::SharedLinks,
        prefix::DisplayPrefix,
    },
    stream::FRAME_WORK,
};
use mdtui3::{Budget, Document, Span, WorkLimit};
use std::time::Instant;
fn prepare(source: &str, rows: usize, width: u16, expected: usize, micro: bool) {
    let input_started = Instant::now();
    let mut d = Document::new();
    d.append(source).unwrap();
    d.finish();
    let input_time = input_started.elapsed();
    let links = SharedLinks::new(&d);
    let mut task = DisplayPrefix::new(&d, Span::new(0, source.len()), rows, width).unwrap();
    let mut zero = Budget::new(WorkLimit::default());
    assert_eq!(task.advance(&d, &links, &mut zero).unwrap(), None);
    assert_eq!(zero.used(), WorkLimit::default());
    let started = Instant::now();
    let mut calls = 0;
    let mut charges = WorkLimit::default();
    let mut cut = None;
    for _ in 0..200_000 {
        calls += 1;
        let mut b = Budget::new(if micro {
            WorkLimit {
                queries: 1,
                objects: 1,
                bytes: 6,
                tiles: 1,
                recovery: 1,
                dependencies: 1,
            }
        } else {
            FRAME_WORK
        });
        let before = task.prepared_cells();
        cut = task.advance(&d, &links, &mut b).unwrap();
        if micro {
            assert!(task.prepared_cells() - before <= 1);
        }
        let used = b.used();
        charges.queries += used.queries;
        charges.objects += used.objects;
        charges.bytes += used.bytes;
        if cut.is_some() {
            break;
        }
        assert_ne!(used, WorkLimit::default());
    }
    assert_eq!(cut, Some(expected));
    let shape = links.shape().unwrap().work();
    assert_eq!(shape.source_decode_bytes, 0);
    eprintln!(
        "prefix-scale source={} rows={rows} width={width} micro={micro} input={input_time:?} query={:?} calls={calls} charges={charges:?} prepared_steps={} quanta={} display_phase_charged_bytes={} raw={:?} shape={shape:?}",
        source.len(),
        started.elapsed(),
        task.prepared_cells(),
        task.probes(),
        task.source_work(),
        d.display_source().stats()
    );
}
fn frame(r: &mut FrameRuntime) -> (usize, usize) {
    let mut bytes = 0;
    for calls in 1..10_000 {
        let mut wire = Vec::new();
        let commit = r.frame_to(&mut wire, &mut Budget::new(FRAME_WORK)).unwrap();
        bytes += wire.len();
        if commit.committed && !r.fold_pending() {
            return (calls, bytes);
        }
    }
    panic!("normal frame budget did not finish")
}
#[test]
fn fixed_one_row_blank_run_scale_separates_input_query_preparation_and_real_frames() {
    for n in [16, 64, 1024, 16384] {
        let source = format!("a{}z\n", " ".repeat(n));
        prepare(&source, 1, 4, 4, true);
        prepare(&source, 1, 4, 4, false);
        let list = format!("- a{}z\n", " ".repeat(n));
        prepare(&list, 1, 6, list.len(), true);
        let mut r = FrameRuntime::from_finished_source(
            &source,
            FrameGeometry {
                rows: 8,
                cols: 40,
                content_width: 4,
            },
            LinkMode::Default,
        )
        .unwrap();
        let initial = Instant::now();
        let initial_cost = frame(&mut r);
        let initial_elapsed = initial.elapsed();
        r.open_fold_menu(false);
        frame(&mut r);
        r.fold_menu_key(KeyCode::Right);
        r.fold_menu_key(KeyCode::Right);
        r.fold_menu_key(KeyCode::Home);
        for _ in 0..13 {
            r.fold_menu_key(KeyCode::Down);
        }
        r.fold_menu_key(KeyCode::Enter);
        let started = Instant::now();
        let (frames, wire) = frame(&mut r);
        let c = r
            .viewport()
            .folds()
            .source_fold_at(r.document(), 0)
            .unwrap()
            .unwrap();
        assert_eq!(c.hidden, Span::new(4, source.len()));
        assert_eq!(r.viewport().heights().total_rows(), 2);
        eprintln!(
            "actual-menu blank_run={n} initial_calls_bytes={initial_cost:?} initial_elapsed={initial_elapsed:?} prefix_frames={frames} prefix_wire={wire} prefix_elapsed={:?}",
            started.elapsed()
        );
    }
}
#[test]
fn a_giant_single_egc_keeps_shared_source_and_reports_real_output_cost() {
    let prefix = format!("a{}", "\u{301}".repeat(100_000));
    let source = format!("{prefix}z\n");
    prepare(&source, 1, 1, prefix.len(), true);
    prepare(&source, 1, 1, prefix.len(), false);
    let mut r = FrameRuntime::from_finished_source(
        &source,
        FrameGeometry {
            rows: 4,
            cols: 20,
            content_width: 1,
        },
        LinkMode::Default,
    )
    .unwrap();
    let started = Instant::now();
    let (frames, wire) = frame(&mut r);
    assert!(
        wire >= prefix.len(),
        "whole visible EGC payload must actually be sent"
    );
    eprintln!(
        "giant-egc actual_output_frames={frames} wire={wire} elapsed={:?} rss={:?}",
        started.elapsed(),
        std::fs::read_to_string("/proc/self/status")
            .unwrap()
            .lines()
            .filter(|l| l.starts_with("VmHWM:") || l.starts_with("VmRSS:"))
            .collect::<Vec<_>>()
    );
}
