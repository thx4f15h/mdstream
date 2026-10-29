use mdtui3::{Document, HeightIndex};
use std::{hint::black_box, time::Instant};

fn main() {
    println!(
        "case,bytes,parse_ms,inline_bytes,delimiter_steps,tree_visits,annotation_nodes,annotation_capacity_bytes,annotation_allocations,height_capacity_bytes,viewport_median_us,viewport_p95_us,query_visits"
    );
    for size in [65_536, 131_072, 262_144, 524_288] {
        for case in [
            "one_byte_appends",
            "one_byte_reused",
            "dense_markup",
            "deep_openers",
            "late_references",
        ] {
            let source = match case {
                "one_byte_appends" | "one_byte_reused" => "x".repeat(size),
                "dense_markup" => "*a* **b** ~~c~~ [d](e) `f`\n".repeat(size / 27),
                "deep_openers" => format!("*a {}z*\n", "_b ".repeat(size / 3)),
                _ => format!("{}\n\n[key]: /destination\n", "[key] ".repeat(size / 6)),
            };
            let mut doc = Document::new();
            let start = Instant::now();
            if case == "one_byte_appends" {
                for _ in 0..size {
                    black_box(doc.append("x").unwrap());
                }
            } else if case == "one_byte_reused" {
                let mut update = mdtui3::Update::default();
                for _ in 0..size {
                    doc.append_into("x", &mut update).unwrap();
                    black_box(&update);
                }
            } else {
                black_box(doc.append(&source).unwrap());
            }
            black_box(doc.finish());
            let elapsed = start.elapsed();
            let mut heights = HeightIndex::default();
            heights.rebuild(&doc).unwrap();
            let top = (doc.tile_count() / 2) as u128;
            let mut query_visits = 0;
            let mut samples = Vec::with_capacity(31);
            // Warm the queried region once, then report distribution rather than
            // treating one cold query as a stable benchmark or a test threshold.
            for iteration in 0..32 {
                let start = Instant::now();
                query_visits = 0;
                for tile in heights.viewport(top..top + 24, 2, 32) {
                    let mut runs = doc.runs(tile.tile).unwrap();
                    for run in runs.by_ref() {
                        black_box(run);
                    }
                    query_visits += runs.annotation_visits();
                }
                if iteration > 0 {
                    samples.push(start.elapsed().as_secs_f64() * 1_000_000.0);
                }
            }
            samples.sort_by(f64::total_cmp);
            let work = doc.work();
            println!(
                "{case},{},{:.3},{},{},{},{},{},{},{},{:.3},{:.3},{}",
                source.len(),
                elapsed.as_secs_f64() * 1000.0,
                work.inline_bytes,
                work.delimiter_steps,
                work.tree_visits,
                doc.annotation_node_count(),
                doc.annotation_arena_stats().capacity_bytes,
                doc.annotation_arena_stats().allocation_calls,
                heights.arena_stats().capacity_bytes,
                samples[15],
                samples[29],
                query_visits
            );
        }
    }
}
