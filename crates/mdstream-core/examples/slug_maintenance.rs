//! Deterministic production scale probe; no cold rebuilding on the runtime path.
use mdstream::*;
use std::{sync::Arc, time::Instant};
fn budget() -> Budget {
    Budget::new(WorkLimit::unlimited())
}
fn settle(d: &Document, s: &mut HeadingSlugSession) {
    assert_eq!(s.advance(d, &mut budget()).unwrap(), Completion::Covered);
}
fn append(d: &mut Document, s: &mut HeadingSlugSession, text: &str) {
    let u = d.append(text).unwrap();
    if u.revision != u.base_revision {
        s.observe(d, Arc::new(u)).unwrap();
    }
    settle(d, s);
}
fn lookup(d: &Document, s: &mut HeadingSlugSession, r: &mut CurrentFragment) -> FragmentStep {
    s.advance_fragment(d, r, &mut budget()).unwrap()
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let mode = args.get(1).map_or("growth", String::as_str);
    let n = args.get(2).map_or(4096, |s| s.parse::<usize>().unwrap());
    let start = Instant::now();
    let mut d = Document::new();
    let mut s = HeadingSlugSession::new(&mut d);
    let before;
    let mut growth_comparisons = 0;
    match mode {
        "growth" => {
            append(&mut d, &mut s, &format!("# {}\n# ", "a".repeat(n)));
            before = s.stats().work;
            for _ in 0..n {
                append(&mut d, &mut s, "a");
            }
            growth_comparisons = s.stats().work.compared_bytes - before.compared_bytes;
            assert_eq!(growth_comparisons, n as u64);
            let mut r = s
                .fragment(Arc::from(format!("#{}-1", "a".repeat(n))))
                .unwrap();
            assert!(matches!(
                lookup(&d, &mut s, &mut r),
                FragmentStep::Resolved(AnchorTarget {
                    heading: Some(HeadingId(1)),
                    ..
                })
            ));
            assert_eq!(s.stats().work.source_bytes - before.source_bytes, n as u64);
            // Query insertion compares a bounded numeric tail after the shared
            // canonical endpoint; the independently backed growth itself is n.
        }
        "history" => {
            let mut text = String::new();
            for i in 0..n {
                use std::fmt::Write;
                writeln!(text, "# history-{i}").unwrap();
            }
            text.push('\n');
            append(&mut d, &mut s, &text);
            before = s.stats().work;
            for _ in 0..1000 {
                append(&mut d, &mut s, "x");
            }
            assert!(s.stats().work.heading_reads - before.heading_reads <= 1000);
            assert_eq!(s.stats().work.renamed, before.renamed);
        }
        "cascade" => {
            append(
                &mut d,
                &mut s,
                &format!("# [foo][r]\n{}\n", "# foo\n".repeat(n)),
            );
            before = s.stats().work;
            let mut r = s.fragment(Arc::from("#foo")).unwrap();
            assert!(matches!(
                lookup(&d, &mut s, &mut r),
                FragmentStep::Resolved(AnchorTarget {
                    heading: Some(HeadingId(1)),
                    ..
                })
            ));
            append(&mut d, &mut s, "[r]: /yes\n");
            assert!(matches!(
                lookup(&d, &mut s, &mut r),
                FragmentStep::Resolved(AnchorTarget {
                    heading: Some(HeadingId(0)),
                    ..
                })
            ));
            assert_eq!(s.stats().work.renamed - before.renamed, (n + 1) as u64);
        }
        "setext" => {
            append(&mut d, &mut s, &"a".repeat(n));
            before = s.stats().work;
            append(&mut d, &mut s, "\n---\n");
            assert_eq!(s.indexed_headings(), 1);
            assert_eq!(s.stats().work.source_bytes - before.source_bytes, 5);
        }
        "queries" => {
            let mut r = s
                .fragment(Arc::from(format!("#{}", "a".repeat(n))))
                .unwrap();
            assert_eq!(lookup(&d, &mut s, &mut r), FragmentStep::Unresolved);
            before = s.stats().work;
            append(&mut d, &mut s, "# ");
            for _ in 0..n {
                append(&mut d, &mut s, "a");
                let _ = lookup(&d, &mut s, &mut r);
            }
            assert!(matches!(
                lookup(&d, &mut s, &mut r),
                FragmentStep::Resolved(_)
            ));
            assert_eq!(s.stats().work.query_bytes - before.query_bytes, 0);
            drop(r);
            settle(&d, &mut s);
            assert_eq!(s.stats().requests, 0);
        }
        _ => panic!("growth/history/cascade/setext/queries"),
    }
    let input_work = d.work();
    let cpu = std::fs::read_to_string("/proc/self/stat")
        .ok()
        .and_then(|s| {
            let (_, tail) = s.rsplit_once(')')?;
            let words: Vec<_> = tail.split_whitespace().collect();
            Some((
                words.get(11)?.parse::<u64>().ok()?,
                words.get(12)?.parse::<u64>().ok()?,
            ))
        })
        .unwrap_or((0, 0));
    let stats = s.stats();
    let work = stats.work;
    let elapsed = start.elapsed().as_secs_f64();
    let rss = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines().find_map(|line| {
                line.strip_prefix("VmHWM:")
                    .and_then(|s| s.split_whitespace().next())
                    .and_then(|s| s.parse::<usize>().ok())
            })
        })
        .unwrap_or(0);
    assert_eq!(s.retire(&d, &mut budget()).unwrap(), Completion::Covered);
    let retired = s.stats();
    assert_eq!(retired.rope_nodes, 0);
    assert_eq!(retired.trie_nodes, 0);
    assert_eq!(retired.numeric_nodes, 0);
    println!(
        "{{\"mode\":\"{mode}\",\"n\":{n},\"seconds\":{elapsed},\"growth_comparisons\":{growth_comparisons},\"rss_peak_kib\":{rss},\"cpu_user_ticks\":{},\"cpu_system_ticks\":{},\"input_tree_visits\":{},\"input_link_index_steps\":{},\"changed_bases\":{},\"source_bytes\":{},\"delta_source_bytes\":{},\"normalized_bytes\":{},\"compared_bytes\":{},\"delta_compared_bytes\":{},\"trie_steps\":{},\"rope_steps\":{},\"numeric_steps\":{},\"projection_steps\":{},\"renamed\":{},\"delta_renamed\":{},\"heading_reads\":{},\"delta_heading_reads\":{},\"query_bytes\":{},\"owned_capacity_bytes\":{},\"directory_entries\":{},\"retired\":{},\"closed_owned_capacity_bytes\":{}}}",
        cpu.0,
        cpu.1,
        input_work.tree_visits,
        input_work.link_index_steps,
        work.changed_bases,
        work.source_bytes,
        work.source_bytes - before.source_bytes,
        work.normalized_bytes,
        work.compared_bytes,
        work.compared_bytes - before.compared_bytes,
        work.trie_steps,
        work.rope_steps,
        work.numeric_steps,
        work.projection_steps,
        work.renamed,
        work.renamed - before.renamed,
        work.heading_reads,
        work.heading_reads - before.heading_reads,
        work.query_bytes,
        stats.owned_capacity_bytes,
        stats.directory_entries,
        work.retired,
        retired.owned_capacity_bytes
    );
}
