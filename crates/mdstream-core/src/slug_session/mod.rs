//! Maintained logical heading slugs. No screen-layout or Unicode raster policy.
mod index;
mod numbers;
mod pages;
mod projection;
mod rope;
mod runtime;
mod session;
mod tape;
pub use runtime::SlugWork;
pub use session::{CurrentFragment, CurrentSlug, HeadingSlugSession, SlugByte, SlugSessionStats};

#[cfg(test)]
mod tests {
    use super::{
        rope::{Memory, RAW, Rope, Small},
        runtime::{Cx, boxed},
    };
    use crate::{Budget, WorkLimit};

    #[test]
    fn native_rope_microbudget_preserves_shared_prefix_and_paid_retirement() {
        let cx = Cx::default();
        let mut memory = Memory::new(cx.clone());
        for _ in 0..4096 {
            memory.push(RAW, b'a');
        }
        let mut job = boxed(&cx, async move {
            let mut rope = Rope::Empty;
            for i in 0..4096 {
                let part = memory.span(RAW, i, 1);
                rope = memory.join(rope, part).await;
            }
            assert_eq!(memory.len(rope), 4096);
            assert!(matches!(rope, Rope::Span { .. }));
            let old = memory.cut(rope, 0, 4095).await;
            assert!(memory.shared_prefix(old, rope).await);
            let suffix = Rope::Small(Small::from_bytes(b"-1"));
            let copy = memory.retain(rope).await;
            let name = memory.join(copy, suffix).await;
            assert_eq!(memory.lcp(rope, name).await, 4096);
            assert_eq!(memory.byte(name, 4097).await, b'1');
            memory.release(old).await;
            memory.release(name).await;
            memory.release(rope).await;
            memory.retire().await;
            assert_eq!(memory.owned().3, 0);
            memory
        });
        let before = cx.work();
        let mut zero = Budget::new(WorkLimit::default());
        assert!(cx.poll(job.as_mut(), &mut zero).is_pending());
        assert_eq!(cx.work(), before);
        for _ in 0..100_000 {
            let mut b = Budget::new(WorkLimit {
                objects: 1,
                queries: 1,
                bytes: 6,
                ..Default::default()
            });
            if cx.poll(job.as_mut(), &mut b).is_ready() {
                return;
            }
        }
        panic!("rope did not progress");
    }
}

#[cfg(test)]
mod index_tests {
    use super::{
        index::Index,
        rope::Rope,
        runtime::{Cx, Op, boxed},
    };
    use crate::{Budget, WorkLimit};
    use std::{collections::BTreeSet, task::Poll};
    #[test]
    fn native_independent_competitor_growth_preserves_root_counterexample() {
        let n = std::env::var("MDSTREAM_SLUG_SCALE")
            .ok()
            .map_or(4096, |s| s.parse::<usize>().unwrap());
        let cx = Cx::default();
        let mut index = Index::new(cx.clone());
        let work = cx.clone();
        let mut job = boxed(&cx, async move {
            let fixed = index.mem.new_buffer();
            let growing = index.mem.new_buffer();
            for _ in 0..n {
                work.charge(Op::Emit).await;
                index.mem.push(fixed, b'a');
            }
            let rope = index.mem.span(fixed, 0, n);
            index.set(0, rope).await;
            index.maintain().await;
            let before = work.work();
            for i in 1..=n {
                work.charge(Op::Emit).await;
                index.mem.push(growing, b'a');
                let rope = index.mem.span(growing, 0, i);
                index.set(1, rope).await;
                index.maintain().await;
                index.mem.retire().await;
            }
            let w = work.work();
            assert_eq!(w.compared_bytes - before.compared_bytes, n as u64);
            assert_eq!(index.entries[&1].ordinal, 1);
            assert_eq!(w.normalized_bytes - before.normalized_bytes, n as u64);
            let leased = index.mem.new_buffer();
            for _ in 0..n {
                work.charge(Op::Copy).await;
                index.mem.push(leased, b'z');
            }
            let rope = index.mem.span(leased, 0, n);
            let node = index.register(rope).await;
            assert_eq!(index.current(node), None);
            index.unregister(node).await;
            index.mem.drop_buffer_owner(leased);
            index.mem.retire().await;
            println!(
                "{{\"native_competitor_n\":{n},\"growth_compared_bytes\":{},\"growth_normalized_bytes\":{},\"trie_steps\":{},\"rope_steps\":{},\"renamed\":{}}}",
                w.compared_bytes - before.compared_bytes,
                w.normalized_bytes - before.normalized_bytes,
                w.trie_steps,
                w.rope_steps,
                w.renamed
            );
            index.close().await;
            index.mem.drop_buffer_owner(fixed);
            index.mem.drop_buffer_owner(growing);
            index.mem.retire().await;
            assert_eq!(work.stores()[0][2], 0);
            assert_eq!(work.stores()[1][2], 0);
            assert_eq!(work.stores()[2][2], 0);
        });
        let tiny = n <= 4096;
        loop {
            let mut b = Budget::new(if tiny {
                WorkLimit {
                    queries: 1,
                    objects: 1,
                    bytes: 6,
                    ..Default::default()
                }
            } else {
                WorkLimit::unlimited()
            });
            if let Poll::Ready(()) = cx.poll(job.as_mut(), &mut b) {
                break;
            }
            assert_ne!(b.used(), WorkLimit::default());
        }
    }
    #[test]
    fn arbitrary_changed_bases_and_query_order_match_independent_global_greedy() {
        let cx = Cx::default();
        let work = cx.clone();
        let mut job = boxed(&cx, async move {
            let mut index = Index::new(work.clone());
            let mut bases = vec![String::new(); 32];
            let mut random = 17u64;
            for round in 0..400 {
                random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
                let id = if round < 32 {
                    round
                } else {
                    (random as usize) % 32
                };
                let values = [
                    "foo", "foo-1", "foo-2", "foo-1-1", "section", "a-10", "a", "a-1", "foo-01",
                    "foo-0", "foo-999", "a-2",
                ];
                let text = values[(random >> 32) as usize % values.len()];
                bases[id] = text.into();
                let b = index.mem.new_buffer();
                for byte in text.bytes() {
                    work.charge(Op::Emit).await;
                    index.mem.push(b, byte);
                }
                let rope = index.mem.span(b, 0, text.len());
                index.set(id, rope).await;
                index.mem.drop_buffer_owner(b);
                index.maintain().await;
                let mut occupied = BTreeSet::new();
                for (id, base) in bases.iter().enumerate().filter(|(_, b)| !b.is_empty()) {
                    let mut ordinal = 0;
                    let mut name = base.clone();
                    while !occupied.insert(name.clone()) {
                        ordinal += 1;
                        name = format!("{base}-{ordinal}");
                    }
                    assert_eq!(
                        index.entries[&id].ordinal, ordinal,
                        "round {round} id {id} bases {bases:?}"
                    );
                    let b = index.mem.new_buffer();
                    for byte in name.bytes() {
                        work.charge(Op::Copy).await;
                        index.mem.push(b, byte);
                    }
                    let query = index.mem.span(b, 0, name.len());
                    let leased = index.register(query).await;
                    assert_eq!(index.current(leased), Some(id));
                    index.unregister(leased).await;
                    index.mem.drop_buffer_owner(b);
                }
                index.mem.retire().await;
            }
            index.close().await;
            index.mem.retire().await;
            let _ = Rope::Empty;
        });
        assert!(
            cx.poll(job.as_mut(), &mut Budget::new(WorkLimit::unlimited()))
                .is_ready()
        );
    }
}
