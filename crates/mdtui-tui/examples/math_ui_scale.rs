//! Actual native scanner/sink/height/End measurements. Run through heavy-gate.sh
//! for million-sized inputs; RSS includes the shared native shape index.
use mdtui_tui::render::viewport::{ViewportConfig, ViewportFrameStatus, ViewportState};
use mdtui3::{Budget, Document, MathWindowStep, Span, TileId, WorkLimit};
use std::time::Instant;
fn budget() -> Budget {
    Budget::new(WorkLimit {
        queries: 256,
        objects: 256,
        bytes: 1536,
        tiles: 8,
        recovery: 64,
        dependencies: 64,
    })
}
fn main() {
    let n = std::env::args()
        .nth(1)
        .map_or(1024, |s| s.parse::<usize>().unwrap());
    let source = format!("\\[\n{}\n\\]\n\nTAIL\n", "x".repeat(n));
    let input_start = Instant::now();
    let mut document = Document::new();
    document.append(&source).unwrap();
    document.finish();
    let input_ns = input_start.elapsed().as_nanos();
    let input_work = document.work();
    let mut window = document.math_window(Span::new(0, source.len())).unwrap();
    let MathWindowStep::Object(meta) = window.advance(&document, &mut budget()).unwrap() else {
        panic!("native object")
    };
    let mut viewport = ViewportState::new(
        &document,
        ViewportConfig {
            width: 12,
            rows: 4,
            overscan_rows: 0,
            max_tiles: 8,
            max_replay_bytes: 8192,
            row_slack: 4,
        },
    )
    .unwrap();
    let mut zero = Budget::new(WorkLimit::default());
    viewport.frame(&document, &mut zero).unwrap();
    assert_eq!(zero.used(), WorkLimit::default());
    assert_eq!(viewport.shared_links().math().retained_owners(), 0);
    let mut calls = 0;
    let mut max_ns = 0;
    let mut queries = 0;
    let mut bytes = 0;
    let start = Instant::now();
    let initial = loop {
        let tick = Instant::now();
        let mut b = budget();
        let frame = viewport.frame(&document, &mut b).unwrap();
        calls += 1;
        max_ns = max_ns.max(tick.elapsed().as_nanos());
        queries += b.used().queries;
        bytes += b.used().bytes;
        assert!(b.used().queries <= 256 && b.used().objects <= 256 && b.used().bytes <= 1536);
        if matches!(
            frame.status,
            ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument
        ) {
            break frame;
        }
        assert!(calls < n * 100 + 10000, "finite positive progress");
    };
    let prepare_ns = start.elapsed().as_nanos();
    let before = viewport.shared_links().math().stats(meta.id).unwrap();
    let mut end_calls = 0;
    let mut end_queries = 0;
    let mut end_bytes = 0;
    let end_start = Instant::now();
    viewport.seek_end(&document, false).unwrap();
    let tail = loop {
        let mut b = budget();
        let frame = viewport.frame(&document, &mut b).unwrap();
        end_calls += 1;
        end_queries += b.used().queries;
        end_bytes += b.used().bytes;
        if matches!(
            frame.status,
            ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument
        ) {
            break frame;
        }
        assert!(end_calls < n * 10 + 10000);
    };
    let end_ns = end_start.elapsed().as_nanos();
    let text: Vec<_> = tail
        .lines
        .iter()
        .map(|l| {
            l.cells
                .iter()
                .map(|c| c.text.collect_text())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect();
    assert!(
        text.iter().any(|line| line == "TAIL"),
        "actual tail {text:?}"
    );
    let after = viewport.shared_links().math().stats(meta.id).unwrap();
    assert_eq!(
        before.source_bytes, after.source_bytes,
        "cached End never rereads TeX input"
    );
    if document.tile(TileId(0)).unwrap().end <= meta.source.end {
        assert_eq!(viewport.heights().height(TileId(0)), Some(1));
    } else {
        assert!(viewport.heights().height(TileId(0)).unwrap() >= 1);
    }
    for tile in 1..document.tile_count().saturating_sub(1) {
        assert_eq!(viewport.heights().height(TileId(tile)), Some(0));
    }
    let capacity: usize = after
        .syntax_storage
        .into_iter()
        .chain(after.layout_storage)
        .chain(after.shaping_storage)
        .chain([after.payload])
        .map(|s| s.capacity_bytes)
        .sum();
    println!(
        "n={n} input_ns={input_ns} prepare_ns={prepare_ns} calls={calls} queries={queries} bytes={bytes} max_frame_ns={max_ns} initial_math_callbacks={} end_ns={end_ns} end_calls={end_calls} end_queries={end_queries} end_bytes={end_bytes} end_work={:?} cached_tex_source_delta={} owned_math_record_page_capacity={capacity} math_index_payload={} document_fixed_bytes={} tail={text:?}",
        initial.stats.layout.math_object_callbacks,
        viewport.end_work(),
        after.source_bytes - before.source_bytes,
        after.index_payload_bytes,
        std::mem::size_of::<Document>()
    );
    println!("actual_core_input_work={input_work:?}");
    println!(
        "native_shape_work={:?}",
        viewport.shared_links().shape().unwrap().work()
    );
    println!("math_work={after:?}");
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    for line in status
        .lines()
        .filter(|s| s.starts_with("VmRSS:") || s.starts_with("VmHWM:"))
    {
        println!("{line}");
    }
}
