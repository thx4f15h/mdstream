#[path = "support/performance.rs"]
mod performance;
#[path = "support/snapshot.rs"]
mod snapshot;

use mdtui3::*;
use performance::*;
use std::{mem::size_of, time::Duration};

fn depth(n: usize) -> u64 {
    u64::from(usize::BITS - n.max(1).leading_zeros()) + 1
}

#[test]
fn matrix_workloads_are_reproducible_and_csv_counters_are_stable() {
    let mut fingerprint = 0xcbf29ce484222325_u64;
    for case in CASES {
        let a = measure(case, 4096, 5);
        let b = measure(case, 4096, 5);
        assert_eq!(a.work, b.work, "{case}");
        assert_eq!(a.frames, b.frames, "{case}");
        assert_eq!(a.capacity, b.capacity, "{case}");
        let untimed = |r: &Report| {
            r.columns()
                .into_iter()
                .filter(|(k, _)| !k.ends_with("_us") && !k.ends_with("_ms") && k != "parse_mib_s")
                .collect::<Vec<_>>()
        };
        assert_eq!(untimed(&a), untimed(&b));
        for byte in a.fingerprint.to_le_bytes() {
            fingerprint = (fingerprint ^ u64::from(byte)).wrapping_mul(0x100000001b3);
        }
        let columns = a.columns();
        let mut keys = std::collections::BTreeSet::new();
        for (key, value) in columns {
            assert!(keys.insert(key));
            assert!(!value.contains([',', '\n']));
        }
    }
    assert_eq!(fingerprint, 8_476_765_057_420_165_672); // FNV of the 17 source fingerprints.
}

#[test]
fn append_schedules_and_unicode_byte_splits_preserve_complete_semantics() {
    for case in CASES {
        let w = workload(case, 4096);
        let actual = parse(case, &w);
        let mut whole = Document::new();
        whole.append(&format!("{}{}", w.source, w.suffix)).unwrap();
        whole.finish();
        assert_eq!(
            snapshot::snapshot(&actual.doc),
            snapshot::snapshot(&whole),
            "{case}"
        );
        assert_eq!(
            actual.calls,
            w.source.len().div_ceil(w.chunk) + usize::from(!w.suffix.is_empty())
        );
    }
    let a = measure("unicode_1", 4096, 5);
    let b = measure("unicode_257", 4096, 5);
    assert_eq!(a.work, b.work);
    assert_eq!(a.frames, b.frames);
    assert!(a.frames.iter().any(|f| f.bytes > f.scalars));
}

#[test]
fn matrix_parse_counts_scale_with_bytes_and_logarithmic_index_depth() {
    for size in [4096, 16_384, 65_536] {
        for case in CASES {
            let r = measure(case, size, 3);
            let n = r.bytes as u64;
            assert_eq!(r.work.input_bytes, n, "{case}");
            assert!(r.work.inline_bytes <= n, "{case}");
            assert!(r.work.classified_bytes <= n, "{case}");
            assert!(r.work.table_bytes <= n, "{case}");
            assert!(r.work.table_header_bytes <= n, "{case}");
            assert!(r.work.table_candidate_bytes <= n, "{case}");
            assert!(r.work.delimiter_steps <= 2 * n, "{case}");
            // Fixed syntax has <= one range write per input byte. Each write
            // descends/returns through at most four paths of the sparse tree.
            assert!(r.work.tree_visits <= 4 * n * depth(r.bytes), "{case}");
            assert!(r.capacity.annotation.nodes <= 4 * r.bytes, "{case}");
            if case.starts_with("append_") || case.starts_with("one_byte_") {
                assert_eq!(r.work.inline_bytes, n);
                assert_eq!(r.work.classified_bytes, 1024);
                assert_eq!(r.work.tree_visits, 1);
                // Geometric arena growth retains logarithmic prefix-path nodes.
                assert!(r.capacity.annotation.nodes as u64 <= 2 * depth(r.bytes));
            }
        }
    }
}

#[test]
fn sparse_wide_table_storage_tracks_physical_cells_not_the_rectangle() {
    for size in [4096, 65_536, 1_048_576] {
        let r = measure("wide_sparse", size, 5);
        assert_eq!(r.capacity.physical_cells, r.columns + r.rows);
        assert!(r.capacity.table_cell_bytes <= 4 * (r.columns + r.rows) * size_of::<TableCell>());
        assert!(r.capacity.table_column_bytes <= 2 * r.columns * size_of::<Column>());
        assert!(r.capacity.physical_cells * 8 < r.columns * r.rows);
        let w = workload("wide_sparse", size);
        let parsed = parse("wide_sparse", &w);
        let table = &parsed.doc.tables()[0];
        assert_eq!(table.columns.len(), w.columns);
        assert_eq!(table.rows().len(), w.rows + 1);
        assert_eq!(
            table
                .cell(w.rows, 0)
                .map(|c| parsed.doc.text(c.source).unwrap()),
            Some("x")
        );
        assert!(table.cell(w.rows, w.columns - 1).is_none());
    }
}

#[test]
fn delayed_closure_of_megabyte_payloads_does_not_rescan_old_text() {
    for size in [4096, 1_048_576] {
        for (case, style) in [
            ("late_emphasis", Style::EMPHASIS),
            ("late_code", Style::CODE),
            ("late_math", Style::MATH),
        ] {
            let w = workload(case, size);
            let parsed = parse(case, &w);
            assert_eq!(parsed.closure[0], 2);
            assert_eq!(parsed.closure[1], 2);
            assert!(parsed.closure[2] <= 4);
            assert!(parsed.closure[3] <= 8 * depth(parsed.doc.source().len()));
            let run = parsed
                .doc
                .runs_in(Span::new(size / 2, size / 2 + 1))
                .unwrap()
                .next()
                .unwrap();
            assert!(run.style.contains(style), "{case}");
        }
        let w = workload("late_references", size);
        let parsed = parse("late_references", &w);
        assert_eq!(parsed.doc.links().len(), size.div_ceil(6));
        assert_eq!(parsed.doc.link_destination(LinkId(0)), Some("/destination"));
        assert_eq!(parsed.closure[4], 3); // One label intern, independent of old uses.
        assert!(parsed.closure[1] <= 2);
        assert!(parsed.closure[3] <= 4 * depth(parsed.doc.source().len()));
    }
}

#[test]
fn shared_budget_bounds_both_queries_recovery_consumption_and_height_writes() {
    for size in [4096, 65_536] {
        for case in CASES {
            let r = measure(case, size, 9);
            assert!(r.frames.iter().any(|f| f.bytes > 0), "{case}");
            for f in &r.frames {
                for (used, limit) in budget_values(f.used)
                    .into_iter()
                    .zip(budget_values(FRAME_LIMIT))
                {
                    assert!(used <= limit, "{case}: {f:?}");
                }
                assert!(f.bytes <= f.used.bytes);
                assert!(f.objects <= f.used.objects);
                assert!(f.measurements + f.corrections <= f.used.tiles);
                assert!(
                    f.height_visits
                        <= (f.measurements as u64 + f.corrections as u64) * 4 * depth(r.tiles)
                            + f.reset_visits,
                    "{case}: {f:?}"
                );
            }
            assert!(r.capacity.scratch_bytes <= FRAME_LIMIT.tiles * size_of::<VisibleTile>());
        }
    }
}

#[test]
fn repeated_resize_is_one_visit_and_corrections_follow_one_logarithmic_path() {
    for size in [4096, 1_048_576] {
        for case in ["resize", "height_corrections"] {
            let r = measure(case, size, 65);
            assert!(r.frames.iter().all(|f| f.measurements > 0));
            for f in &r.frames {
                if case == "resize" {
                    assert_eq!(f.resets, 1);
                    assert_eq!(f.reset_visits, 1);
                    assert_eq!(f.corrections, 0);
                } else {
                    assert_eq!(f.corrections, 1);
                    assert!(f.correction_visits <= 2 * depth(r.tiles));
                    assert_eq!(f.resets, 0);
                }
            }
            let twice = measure(case, size, 130);
            // A fixed eight-anchor working set reuses the same sparse nodes.
            assert_eq!(r.capacity.height, twice.capacity.height);
            assert_eq!(r.capacity.dependency, twice.capacity.dependency);
        }
    }
}

#[test]
fn fixed_window_plain_text_work_is_independent_of_offscreen_size() {
    let a = measure("append_whole", 65_536, 9);
    let b = measure("append_whole", 1_048_576, 9);
    for (a, b) in a.frames.iter().zip(&b.frames) {
        assert_eq!(a.used, b.used);
        assert_eq!(a.bytes, b.bytes);
        assert_eq!(a.measurements, b.measurements);
        assert_eq!(a.scalars, b.scalars);
    }
}

#[test]
fn nearest_rank_quantiles_use_all_samples_and_handle_small_sets() {
    let samples: Vec<_> = (1..=100).rev().map(Duration::from_nanos).collect();
    for p in [1, 50, 95, 99, 100] {
        assert_eq!(percentile(&samples, p), Duration::from_nanos(p as u64));
    }
    assert_eq!(percentile(&samples[..3], 50), Duration::from_nanos(99));
    assert_eq!(percentile(&samples[..1], 95), Duration::from_nanos(100));
}
