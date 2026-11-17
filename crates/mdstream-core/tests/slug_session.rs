use mdstream::*;
use std::{collections::BTreeSet, sync::Arc};
fn limit() -> WorkLimit {
    WorkLimit {
        objects: 1,
        queries: 1,
        bytes: 6,
        ..Default::default()
    }
}
fn settle(d: &Document, s: &mut HeadingSlugSession, tiny: bool) {
    for _ in 0..2_000_000 {
        let mut b = Budget::new(if tiny {
            limit()
        } else {
            WorkLimit::unlimited()
        });
        let c = s.advance(d, &mut b).unwrap();
        if c == Completion::Covered {
            assert!(s.is_current(d));
            return;
        }
        assert_ne!(b.used(), WorkLimit::default(), "stalled at {:?}", s.stats());
    }
    panic!("did not settle: {:?} {:?}", d.source(), s.stats());
}
fn names(d: &Document, s: &HeadingSlugSession) -> Vec<String> {
    (0..d.headings().entries().len())
        .map(|id| {
            let mut slug = s.get(d, HeadingId(id)).unwrap().unwrap();
            let mut out = Vec::new();
            loop {
                let mut b = Budget::new(limit());
                match slug.next_byte(&mut b) {
                    SlugByte::Byte(b) => out.push(b),
                    SlugByte::End => break,
                    SlugByte::BudgetExhausted => assert_ne!(b.used(), WorkLimit::default()),
                }
            }
            String::from_utf8(out).unwrap()
        })
        .collect()
}
fn oracle(d: &Document) -> Vec<String> {
    let mut used = BTreeSet::new();
    d.headings()
        .entries()
        .iter()
        .map(|h| {
            let text: String = d
                .display_runs_in(h.content)
                .unwrap()
                .filter(|r| !r.style.contains(Style::HIDDEN))
                .map(|r| r.text)
                .collect();
            let mut base = String::new();
            for ch in text.chars() {
                if ch.is_alphanumeric() {
                    base.extend(ch.to_lowercase());
                } else if ch == '-' || ch == '_' {
                    base.push(ch);
                } else if ch.is_whitespace() {
                    base.push('-');
                }
            }
            if base.is_empty() {
                base.push_str("section");
            }
            let mut name = base.clone();
            let mut n = 0;
            while !used.insert(name.clone()) {
                n += 1;
                name = format!("{base}-{n}");
            }
            name
        })
        .collect()
}
fn compare(d: &Document, s: &mut HeadingSlugSession, tiny: bool) {
    settle(d, s, tiny);
    assert_eq!(names(d, s), oracle(d), "source {:?}", d.source());
}
fn query(
    d: &Document,
    s: &mut HeadingSlugSession,
    r: &mut CurrentFragment,
) -> Result<FragmentStep, LayoutError> {
    for _ in 0..2_000_000 {
        let mut b = Budget::new(limit());
        match s.advance_fragment(d, r, &mut b) {
            Ok(FragmentStep::BudgetExhausted) => assert_ne!(b.used(), WorkLimit::default()),
            other => return other,
        }
    }
    panic!("query stalled")
}
#[test]
fn every_prefix_current_matches_independent_cold_display_and_global_greedy() {
    let cases = [
        "# foo\n# foo-1\n# foo\n# foo-2\n# foo\n# foo-1\n",
        "# A\t B &amp; İ 中 ` A\r\nB ` ###\n",
        "one &amp; `two`\nthree\n---\n",
        "> title\n> next *word*\n> ---\n\n# root\n",
        "# **[a][x]** [a](#a)\n\n[x]: #a\n",
        "# a [*x*][r]\n\n[r]: /yes\n[r]: /no\n",
        "# ***\n# section\n# section-1\n",
        "# a [x][r]\n\n[r]:\n /x \"title\"\n",
        "# x `a\rb\r\nc`\n",
        "[^x]: # local\n\n# root\n\n[^x]\n",
    ];
    for text in cases {
        let mut d = Document::new();
        let mut s = HeadingSlugSession::new(&mut d);
        for ch in text.chars() {
            let u = d.append(&ch.to_string()).unwrap();
            s.observe(&d, Arc::new(u)).unwrap();
            compare(&d, &mut s, true);
        }
        let u = d.finish();
        s.observe(&d, Arc::new(u)).unwrap();
        compare(&d, &mut s, true);
    }
}
#[test]
fn all_cuts_queued_merged_and_late_subscription() {
    let text = "# foo\n# foo-1\n# [foo][r]\n\n[r]: /yes\n\n> a &amp; ` b `\n> ---\n";
    for cut in 0..=text.len() {
        let mut d = Document::new();
        d.append(&text[..cut]).unwrap();
        let mut s = HeadingSlugSession::new(&mut d);
        let mut u = d.append(&text[cut..]).unwrap();
        let end = d.finish();
        u.merge(end).unwrap();
        s.observe(&d, Arc::new(u)).unwrap();
        compare(&d, &mut s, true);
    }
}
#[test]
fn durable_request_current_owner_invalid_tail_and_paid_cancel() {
    let mut d = Document::new();
    let mut s = HeadingSlugSession::new(&mut d);
    let mut r = s.fragment(Arc::from("#foo")).unwrap();
    assert_eq!(query(&d, &mut s, &mut r), Ok(FragmentStep::Unresolved));
    for text in ["# [foo][r]\n", "\n[r]: /x\n", "# foo\n"] {
        let u = d.append(text).unwrap();
        s.observe(&d, Arc::new(u)).unwrap();
        compare(&d, &mut s, true);
        let q = query(&d, &mut s, &mut r).unwrap();
        if names(&d, &s).iter().any(|n| n == "foo") {
            assert!(matches!(q, FragmentStep::Resolved(_)));
        } else {
            assert_eq!(q, FragmentStep::Unresolved);
        }
    }
    for raw in [
        "#missing%FF",
        "#missing%E2%82",
        "#missing%80",
        "#foo%",
        "#foo%2Z",
    ] {
        let mut r = s.fragment(Arc::from(raw)).unwrap();
        assert_eq!(query(&d, &mut s, &mut r), Err(LayoutError::InvalidRange));
    }
    for raw in ["#FOO", "#f+oo", "#%2566oo"] {
        let mut r = s.fragment(Arc::from(raw)).unwrap();
        assert_eq!(query(&d, &mut s, &mut r), Ok(FragmentStep::Unresolved));
    }
    drop(r);
    settle(&d, &mut s, true);
    assert_eq!(s.stats().requests, 0);
    for _ in 0..1_000_000 {
        let mut b = Budget::new(limit());
        if s.retire(&d, &mut b).unwrap() == Completion::Covered {
            assert_eq!(s.stats().rope_nodes, 0);
            assert_eq!(s.stats().requests, 0);
            return;
        }
    }
    panic!("retire stalled");
}
#[test]
fn wrong_owner_order_and_zero_are_atomic() {
    let mut a = Document::new();
    let mut s = HeadingSlugSession::new(&mut a);
    let b = Document::new();
    let mut z = Budget::new(WorkLimit::default());
    assert_eq!(s.advance(&b, &mut z), Err(LayoutError::WrongDocument));
    let before = s.stats();
    assert_eq!(s.advance(&a, &mut z), Ok(Completion::Covered));
    assert_eq!(s.stats(), before);
    let u = a.append("# a").unwrap();
    assert_eq!(s.advance(&a, &mut z), Err(LayoutError::OutOfOrderUpdate));
    s.observe(&a, Arc::new(u)).unwrap();
    let before = s.stats();
    assert_eq!(s.advance(&a, &mut z), Ok(Completion::BudgetExhausted));
    assert_eq!(s.stats(), before);
    assert_eq!(z.used(), WorkLimit::default());
}

#[test]
fn unicode_byte_cuts_header_rollback_same_tile_and_late_code() {
    let cases = [
        "# A&fjlig; **B**\n# A-1\n# A\n# A\n中文 &amp; X\n===\n# [Late][id]\n\n[id]: /target\n",
        "> *old [x* z]\n> ---\n\n[x* z]: /u\n",
        "-  - a|b\n\t中🙂 &amp;\n\n# last\n",
        "> a `old\n> close` | b\n> ---|---\n> end|cell\n\n# root\n",
        "[old\nend](u '&amp;') | b\nwrong\n===\n",
        "> a `old\n> close` | b\n> wrong\n> ---\n",
        "# !!!\n# section\n# A\n# A-1\n# A\n# A-1\n# A\n#\n# 中文 É İ e\u{301} 𐐀 🙂\n",
    ];
    for text in cases {
        for cut in 0..=text.len() {
            let mut d = Utf8Document::new();
            let mut s = d.heading_slug_session();
            let a = d.append(&text.as_bytes()[..cut]).unwrap();
            if a.revision != a.base_revision {
                s.observe(d.document(), Arc::new(a)).unwrap();
            }
            compare(d.document(), &mut s, true);
            let a = d.append(&text.as_bytes()[cut..]).unwrap();
            if a.revision != a.base_revision {
                s.observe(d.document(), Arc::new(a)).unwrap();
            }
            compare(d.document(), &mut s, true);
            let u = d.finish();
            s.observe(d.document(), Arc::new(u)).unwrap();
            compare(d.document(), &mut s, true);
        }
    }
}
#[test]
fn paused_work_continues_across_updates_and_cancellation_has_no_retained_query_capacity() {
    let mut d = Document::new();
    let mut s = HeadingSlugSession::new(&mut d);
    let mut r = s
        .fragment(Arc::from(format!("#{}", "a".repeat(8192))))
        .unwrap();
    // Register a long absent endpoint once, then grow toward it under continual
    // input. No repeated decode or old query-prefix comparison is permitted.
    assert_eq!(query(&d, &mut s, &mut r), Ok(FragmentStep::Unresolved));
    let registered = s.stats();
    assert_eq!(registered.work.query_bytes, 8192);
    for text in ["# ", "aaaa", "aaaa", "aaaa", "aaaa"] {
        let u = d.append(text).unwrap();
        s.observe(&d, Arc::new(u)).unwrap();
        let mut b = Budget::new(limit());
        let _ = s.advance_fragment(&d, &mut r, &mut b).unwrap();
    }
    settle(&d, &mut s, true);
    assert_eq!(query(&d, &mut s, &mut r), Ok(FragmentStep::Unresolved));
    assert_eq!(s.stats().work.query_bytes, 8192);
    let before_cancel = s.stats();
    drop(r);
    settle(&d, &mut s, true);
    let retired = s.stats();
    assert_eq!(retired.requests, 0);
    assert!(
        retired.owned_capacity_bytes < before_cancel.owned_capacity_bytes,
        "{before_cancel:?} {retired:?}"
    );
}
#[test]
fn collision_cascade_changes_current_request_and_duplicates_are_first_winning() {
    let mut d = Document::new();
    let mut s = HeadingSlugSession::new(&mut d);
    let u = d
        .append("# [foo][r]\n# foo\n# foo-1\n# foo\n# foo-2\n# foo\n\n")
        .unwrap();
    s.observe(&d, Arc::new(u)).unwrap();
    compare(&d, &mut s, true);
    let mut r = s.fragment(Arc::from("#foo")).unwrap();
    assert!(matches!(
        query(&d, &mut s, &mut r),
        Ok(FragmentStep::Resolved(AnchorTarget {
            heading: Some(HeadingId(1)),
            ..
        }))
    ));
    let u = d.append("[r]: /yes\n").unwrap();
    s.observe(&d, Arc::new(u)).unwrap();
    compare(&d, &mut s, true);
    assert!(matches!(
        query(&d, &mut s, &mut r),
        Ok(FragmentStep::Resolved(AnchorTarget {
            heading: Some(HeadingId(0)),
            ..
        }))
    ));
    let old = names(&d, &s);
    let u = d.append("[r]: /ignored\n").unwrap();
    s.observe(&d, Arc::new(u)).unwrap();
    compare(&d, &mut s, true);
    assert_eq!(names(&d, &s), old);
}

#[test]
fn non_heading_reference_history_does_not_create_heading_display_dependencies() {
    let mut d = Document::new();
    let mut s = HeadingSlugSession::new(&mut d);
    let u = d
        .append(&format!("# stable\n\n{}", "[body][r]\n\n".repeat(20000)))
        .unwrap();
    s.observe(&d, Arc::new(u)).unwrap();
    settle(&d, &mut s, false);
    let old = s.stats();
    let u = d.append("[r]: /late\n").unwrap();
    s.observe(&d, Arc::new(u)).unwrap();
    settle(&d, &mut s, true);
    let new = s.stats();
    assert_eq!(new.work.metadata_steps, old.work.metadata_steps);
    assert_eq!(new.work.renamed, old.work.renamed);
    assert_eq!(new.work.source_bytes - old.work.source_bytes, 11);
    assert_eq!(names(&d, &s), vec!["stable"]);
}

#[test]
fn active_input_returns_current_pre_eof_results_without_resetting_the_request() {
    let mut d = Document::new();
    let mut s = HeadingSlugSession::new(&mut d);
    let u = d.append("# stable\n\n# ").unwrap();
    s.observe(&d, Arc::new(u)).unwrap();
    settle(&d, &mut s, true);
    let mut r = s.fragment(Arc::from("#stable")).unwrap();
    for _ in 0..2048 {
        let u = d.append("a").unwrap();
        s.observe(&d, Arc::new(u)).unwrap();
        let mut ready = false;
        for _ in 0..8 {
            let mut b = Budget::new(WorkLimit {
                queries: 128,
                objects: 512,
                bytes: 1024,
                ..Default::default()
            });
            if let FragmentStep::Resolved(t) = s.advance_fragment(&d, &mut r, &mut b).unwrap() {
                assert_eq!(t.heading, Some(HeadingId(0)));
                assert!(t.provisional);
                ready = true;
                break;
            }
        }
        assert!(
            ready,
            "maintenance restarted or could not keep up with paid bounded input"
        );
    }
    assert!(!d.is_finished());
    assert_eq!(s.stats().work.query_bytes, 6);
}

#[test]
fn one_byte_growth_does_not_replay_unchanged_heading_metadata() {
    for n in [512, 2048] {
        let mut d = Document::new();
        let mut s = HeadingSlugSession::new(&mut d);
        let u = d.append(&format!("# {}", "**a** ".repeat(n))).unwrap();
        s.observe(&d, Arc::new(u)).unwrap();
        settle(&d, &mut s, false);
        let old = s.stats();
        let u = d.append("z").unwrap();
        println!(
            "DELTA n={n} headings={:?} changes={:?} effects={:?}",
            u.changed_headings,
            u.changes,
            u.display_shape.events()
        );
        s.observe(&d, Arc::new(u)).unwrap();
        settle(&d, &mut s, false);
        let new = s.stats();
        println!(
            "growth n={n} metadata={} projection={}",
            new.work.metadata_steps - old.work.metadata_steps,
            new.work.projection_steps - old.work.projection_steps
        );
        assert!(
            new.work.metadata_steps - old.work.metadata_steps <= 64,
            "{old:?} {new:?}"
        );
    }
}
#[test]
fn paused_lane_retains_source_and_index_capacity_in_diagnostics() {
    let mut d = Document::new();
    let mut s = HeadingSlugSession::new(&mut d);
    let u = d.append(&format!("# {}\n", "a".repeat(10000))).unwrap();
    s.observe(&d, Arc::new(u)).unwrap();
    settle(&d, &mut s, false);
    let old = s.stats();
    let u = d.append("z").unwrap();
    s.observe(&d, Arc::new(u)).unwrap();
    let mut saw_lane = false;
    for _ in 0..512 {
        let mut b = Budget::new(limit());
        let result = s.advance(&d, &mut b).unwrap();
        let now = s.stats();
        if now.continuation_bytes != 0 {
            saw_lane = true;
            assert!(
                now.owned_capacity_bytes >= old.owned_capacity_bytes,
                "{old:?} {now:?}"
            );
            assert!(now.trie_nodes >= old.trie_nodes);
            assert!(now.numeric_nodes >= old.numeric_nodes);
        }
        if result == Completion::Covered {
            break;
        }
    }
    assert!(saw_lane);
}

#[test]
fn conditional_scope_growth_continues_canonical_style_events_and_only_new_markers() {
    for n in [512, 2048] {
        let mut d = Document::new();
        let mut s = HeadingSlugSession::new(&mut d);
        let u = d
            .append(&format!("# [x][r] {}", "**a** ".repeat(n)))
            .unwrap();
        s.observe(&d, Arc::new(u)).unwrap();
        settle(&d, &mut s, false);
        let old = s.stats();
        let u = d.append("z").unwrap();
        s.observe(&d, Arc::new(u)).unwrap();
        settle(&d, &mut s, false);
        let now = s.stats();
        assert_eq!(names(&d, &s), oracle(&d));
        println!(
            "conditional n={n} metadata={} events={} index={}",
            now.work.metadata_steps - old.work.metadata_steps,
            now.canonical_style_work.events - old.canonical_style_work.events,
            now.canonical_style_work.index_steps - old.canonical_style_work.index_steps
        );
        assert!(now.work.metadata_steps - old.work.metadata_steps < 64);
        assert!(now.canonical_style_work.events - old.canonical_style_work.events < 16);
        assert!(now.canonical_style_work.index_steps - old.canonical_style_work.index_steps < 64);
        for _ in 0..1000000 {
            let mut b = Budget::new(limit());
            if s.retire(&d, &mut b).unwrap() == Completion::Covered {
                break;
            }
        }
        assert_eq!(s.stats().resolver_entries, 0);
    }
}

#[test]
fn incremental_canonical_pairing_matches_fresh_binding_and_nested_style_oracle() {
    for text in [
        "# *prior [x* z] suffix*\n\n[x* z]: /u\n",
        "# _[a_ z][r] more_\n\n[r]: /u\n",
        "# **[a *b** c*](x) end\n",
        "# **a [*b*][r] c** d\n\n[r]: /u\n",
        "# _[x][r] _a _b_ c_ d_ tail\n\n[r]: /u\n",
    ] {
        let mut d = Document::new();
        let mut s = HeadingSlugSession::new(&mut d);
        // The parser publishes long open ATX scopes before LF; force that path
        // so each late delimiter really extends an existing current owner.
        let (prefix, rest) = text.split_at(2);
        let first = format!("{prefix}{} ", "q".repeat(1100));
        let u = d.append(&first).unwrap();
        s.observe(&d, Arc::new(u)).unwrap();
        compare(&d, &mut s, true);
        for ch in rest.chars() {
            let u = d.append(&ch.to_string()).unwrap();
            s.observe(&d, Arc::new(u)).unwrap();
            compare(&d, &mut s, true);
        }
        let u = d.finish();
        s.observe(&d, Arc::new(u)).unwrap();
        compare(&d, &mut s, true);
    }
}

#[test]
fn no_slug_change_and_registered_owner_do_not_accumulate_retired_rope_versions() {
    let mut d = Document::new();
    let mut s = HeadingSlugSession::new(&mut d);
    let u = d.append("# a &amp; b\n\n").unwrap();
    s.observe(&d, Arc::new(u)).unwrap();
    settle(&d, &mut s, true);
    let old = s.stats();
    for _ in 0..1000 {
        let u = d.append("z").unwrap();
        s.observe(&d, Arc::new(u)).unwrap();
        settle(&d, &mut s, true);
        assert_eq!(s.stats().pending_retirement, 0);
    }
    let now = s.stats();
    assert_eq!(now.work.renamed, old.work.renamed);
    assert!(now.rope_nodes < old.rope_nodes + 200);
    let mut r = s.fragment(Arc::from("#a--b")).unwrap();
    assert!(matches!(
        query(&d, &mut s, &mut r),
        Ok(FragmentStep::Resolved(_))
    ));
    assert_eq!(s.stats().pending_retirement, 0);
}
