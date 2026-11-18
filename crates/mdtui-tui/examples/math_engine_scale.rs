//! Reproducible native-object/query counters, independent of final UI glue.
use mdtui_tui::render::math::{MathEngine, MathProgress, PaintStep, Rect};
use mdtui3::{Budget, Document, MathWindowStep, Span, WorkLimit};
use std::time::Instant;

fn tiny() -> Budget {
    Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 6,
        ..WorkLimit::default()
    })
}
fn main() {
    let mut args = std::env::args().skip(1);
    let kind = args.next().unwrap_or_else(|| "wide".to_owned());
    let n = args.next().map_or(1024, |s| s.parse().expect("size"));
    let tex = match kind.as_str() {
        "wide" => "x".repeat(n),
        "cluster" => format!("a{}", "\u{301}".repeat(n)),
        "unknown" => format!("\\{}", "a".repeat(n)),
        "deep" => format!("{}x{}", "\\sqrt{".repeat(n), "}".repeat(n)),
        "matrix" => format!(
            "\\begin{{matrix}}{}x&x\\end{{matrix}}",
            "x&x\\\\".repeat(n.saturating_sub(1))
        ),
        _ => panic!("wide|unknown|deep|matrix|cluster"),
    };
    let mut d = Document::new();
    d.append(&format!("\\({tex}\\)\n")).unwrap();
    d.finish();
    let mut w = d.math_window(Span::new(0, d.source().len())).unwrap();
    let MathWindowStep::Object(meta) = w.advance(&d, &mut tiny()).unwrap() else {
        panic!("native object")
    };
    let mut e = MathEngine::new(&d, meta).unwrap();
    assert_eq!(e.stats().payload.pages, 0);
    assert_eq!(
        e.advance(&d, &mut Budget::new(WorkLimit::default()))
            .unwrap(),
        MathProgress::BudgetExhausted
    );
    let start = Instant::now();
    let mut calls = 0;
    let mut max_ns = 0;
    let mut charged = 0;
    loop {
        let frame = Instant::now();
        let mut b = tiny();
        let p = e.advance(&d, &mut b).unwrap();
        calls += 1;
        charged += b.used().queries;
        max_ns = max_ns.max(frame.elapsed().as_nanos());
        assert!(b.used().queries <= 1 && b.used().objects <= 1 && b.used().bytes <= 6);
        if p == MathProgress::Ready {
            break;
        }
        assert_eq!(p, MathProgress::BudgetExhausted);
    }
    let prep_ns = start.elapsed().as_nanos();
    let s = e.stats();
    let m = e.metrics().unwrap();
    let page_capacity: usize = s
        .syntax_storage
        .into_iter()
        .chain(s.layout_storage)
        .chain(s.shaping_storage)
        .chain([s.payload])
        .map(|s| s.capacity_bytes)
        .sum();
    let mut paint = e
        .paint(Rect {
            x: m.width.saturating_sub(8),
            y: m.height.saturating_sub(2),
            width: m.width.clamp(1, 8),
            height: m.height.min(2),
        })
        .unwrap();
    let mut paint_calls = 0;
    let mut paint_queries = 0;
    let mut paint_bytes = 0;
    let mut ink_bytes = 0;
    let mut ink_calls = 0;
    let mut output_cells = 0;
    loop {
        let mut b = tiny();
        let step = paint.advance(&e, &d, &mut b).unwrap();
        paint_calls += 1;
        paint_queries += b.used().queries;
        paint_bytes += b.used().bytes;
        if let PaintStep::Cell(cell) = step {
            output_cells += 1;
            let mut ink = e.ink_cursor(cell.ink).unwrap();
            let mut out = [0; 4];
            loop {
                let mut b = tiny();
                ink_calls += 1;
                match ink.advance(&e, &d, &mut b, &mut out).unwrap() {
                    mdtui_tui::render::math::InkReadStep::Bytes(n) => ink_bytes += n,
                    mdtui_tui::render::math::InkReadStep::Complete => break,
                    mdtui_tui::render::math::InkReadStep::BudgetExhausted => {}
                }
            }
        }
        if step == PaintStep::Complete {
            break;
        }
    }
    let p = paint.work();
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let rss = status.lines().find(|l| l.starts_with("VmRSS:")).unwrap();
    let hwm = status.lines().find(|l| l.starts_with("VmHWM:")).unwrap();
    println!(
        "kind,n,tex_bytes,calls,charged_queries,prepare_ns,max_frame_ns,input_source_bytes,payload_bytes,tex_bytes_consumed,ast_nodes,ast_edges,layout_boxes,layout_edges,index_entries,owned_page_capacity_bytes,paint_steps,paint_output_bytes,paint_seeks,paint_ancestry,width,height"
    );
    println!(
        "{kind},{n},{},{calls},{charged},{prep_ns},{max_ns},{},{},{},{},{},{},{},{},{page_capacity},{},{},{},{},{},{}",
        tex.len(),
        s.source_bytes,
        s.payload_bytes,
        s.syntax.bytes,
        s.syntax.nodes,
        s.syntax.edges,
        s.layout.boxes,
        s.layout.edges,
        s.index_entries,
        p.steps,
        p.output_bytes,
        p.seeks,
        p.ancestry,
        m.width,
        m.height
    );
    let directory_payload: usize = s
        .syntax_storage
        .into_iter()
        .chain(s.layout_storage)
        .chain(s.shaping_storage)
        .chain([s.payload])
        .map(|s| s.directory_payload_bytes)
        .sum();
    let max_page_allocation = s
        .syntax_storage
        .into_iter()
        .chain(s.layout_storage)
        .chain(s.shaping_storage)
        .chain([s.payload])
        .map(|s| s.max_page_bytes)
        .max()
        .unwrap();
    println!(
        "directory_key_value_payload_bytes={directory_payload} index_key_value_payload_bytes={} max_fixed_record_page_bytes={max_page_allocation} paint_calls={paint_calls} paint_queries={paint_queries} paint_bytes={paint_bytes} paint_validation_steps={}",
        s.index_payload_bytes, p.validation_steps
    );
    println!(
        "shaping_steps={} shaping_bytes={} shaping_scalars={} shaping_joins={} output_cells={output_cells} ink_output_bytes={ink_bytes} ink_calls={ink_calls} document_fixed_bytes={}",
        s.shaping.steps,
        s.shaping.bytes,
        s.shaping.scalars,
        s.shaping.joins,
        std::mem::size_of::<Document>()
    );
    println!("{rss}\n{hwm}");
    let mut retirement = e.into_retirement();
    let start = Instant::now();
    while !retirement.advance(&mut tiny()) {}
    let s = retirement.stats();
    assert_eq!(s.index_entries, 0);
    assert_eq!(s.payload.pages, 0);
    assert!(
        s.syntax_storage
            .into_iter()
            .chain(s.layout_storage)
            .chain(s.shaping_storage)
            .all(|s| s.pages == 0)
    );
    println!(
        "retire_steps={} retire_ns={} old_payload_records_relocated=0; BTree headers/slack excluded from owned_page_capacity",
        retirement.pub_steps,
        start.elapsed().as_nanos()
    );
}
