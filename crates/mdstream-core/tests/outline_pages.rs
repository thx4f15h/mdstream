use mdstream::{
    Budget, Document, HeadingId, HeadingSearch, HeadingSearchStep, LayoutError, WorkLimit,
};

fn parse(s: &str) -> Document {
    let mut d = Document::new();
    d.append(s).unwrap();
    d.finish();
    d
}
fn micro() -> WorkLimit {
    WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 6,
        ..Default::default()
    }
}

#[test]
fn first_match_pages_equal_first_occurrence_oracle_with_micro_budget() {
    let d = parse("# aaa\n\n## a&amp;b\n\n> # 汉字\n>\n> ## ![alt](img)\n\n# aaa\n");
    for level in 0..=6 {
        for query in ["aa", "&", "汉字", "alt", "missing"] {
            let mut reference = HeadingSearch::new(&d, query, level).unwrap();
            let mut all = Vec::new();
            reference
                .advance(&d, &mut Budget::new(WorkLimit::unlimited()), &mut all)
                .unwrap();
            all.dedup_by_key(|hit| hit.heading);
            let mut search = HeadingSearch::new(&d, query, level).unwrap();
            let mut actual = Vec::new();
            for _ in 0..20000 {
                let mut b = Budget::new(micro());
                match search.next_match(&d, &mut b).unwrap() {
                    HeadingSearchStep::Match(hit) => actual.push(hit),
                    HeadingSearchStep::Complete => break,
                    HeadingSearchStep::BudgetExhausted => {}
                }
                assert!(b.used().queries <= 1 && b.used().objects <= 1 && b.used().bytes <= 6);
            }
            assert_eq!(actual, all);
            assert!(search.scratch_capacity_bytes() < 8192);
        }
    }
}

#[test]
fn first_match_does_not_scan_million_overlapping_occurrences_or_read_body() {
    let d = parse(&format!(
        "# {}\n\n{}\n\n# aa\n",
        "a".repeat(1_000_000),
        "b".repeat(1_000_000)
    ));
    let mut search = HeadingSearch::new(&d, "aa", 6).unwrap();
    let mut b = Budget::new(WorkLimit::unlimited());
    assert!(
        matches!(search.next_match(&d,&mut b).unwrap(),HeadingSearchStep::Match(hit) if hit.heading==HeadingId(0))
    );
    let mut second = Budget::new(WorkLimit::unlimited());
    assert!(
        matches!(search.next_match(&d,&mut second).unwrap(),HeadingSearchStep::Match(hit) if hit.heading==HeadingId(1))
    );
    assert!(b.used().bytes < 8192 && second.used().bytes < 64);
    assert!(search.scratch_capacity_bytes() < 8192);
}

#[test]
fn outline_parent_preserves_scope_and_chains_beyond_six_levels() {
    let mut source = String::from("# Root\n\n");
    for depth in 1..=1000 {
        source.push_str(&format!("{} # H{depth}\n", ">".repeat(depth)));
    }
    let d = parse(&source);
    assert_eq!(d.headings().entries().len(), 1001);
    for i in 1..=1000 {
        assert_eq!(d.headings().parent(HeadingId(i)), None);
        assert_eq!(
            d.headings().outline_parent(HeadingId(i)),
            Some(HeadingId(i - 1))
        );
    }
}

#[test]
fn stale_and_wrong_document_reject_before_output_or_budget() {
    let mut d = Document::new();
    d.append("# a\n").unwrap();
    let mut search = HeadingSearch::new(&d, "a", 6).unwrap();
    let other = parse("# a\n");
    let mut b = Budget::new(micro());
    assert_eq!(
        search.next_match(&other, &mut b),
        Err(LayoutError::WrongDocument)
    );
    assert_eq!(b.used(), WorkLimit::default());
    d.append("\n").unwrap();
    assert_eq!(
        search.next_match(&d, &mut b),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(b.used(), WorkLimit::default());
}

#[test]
fn from_heading_skips_preceding_giant_title_and_out_of_arena_is_complete() {
    let d = parse(&format!("# {}\n\n# target\n", "x".repeat(1_000_000)));
    let mut search = HeadingSearch::from_heading(&d, "target", 6, HeadingId(1)).unwrap();
    let mut b = Budget::new(WorkLimit::unlimited());
    assert!(
        matches!(search.next_match(&d,&mut b).unwrap(),HeadingSearchStep::Match(hit) if hit.heading==HeadingId(1))
    );
    assert!(b.used().bytes < 64);
    let mut search = HeadingSearch::from_heading(&d, "target", 6, HeadingId(2)).unwrap();
    let mut zero = Budget::new(WorkLimit::default());
    assert_eq!(
        search.next_match(&d, &mut zero).unwrap(),
        HeadingSearchStep::Complete
    );
    assert_eq!(zero.used(), WorkLimit::default());
}

#[test]
fn conditional_scope_first_matches_follow_late_definitions_and_parent_suppression() {
    let mut d = Document::new();
    d.append("# *foo [bar* baz]\n\n# *foo [bar* [kid] baz][outer]\n\n[outer]: /u\n")
        .unwrap();
    for defined in [false, true] {
        let mut resolver = d.link_resolver();
        for (query, expected) in [
            ("*foo", if defined { vec![0] } else { vec![1] }),
            ("bar*", if defined { vec![0] } else { vec![1] }),
            ("bar kid baz", if defined { vec![1] } else { vec![] }),
        ] {
            let mut search = HeadingSearch::new(&d, query, 6).unwrap();
            let before = resolver.style_work();
            let mut zero = Budget::new(WorkLimit::default());
            search.swap_link_resolver(&mut resolver).unwrap();
            assert_eq!(
                search.next_match(&d, &mut zero).unwrap(),
                HeadingSearchStep::BudgetExhausted
            );
            search.swap_link_resolver(&mut resolver).unwrap();
            assert_eq!(resolver.style_work(), before);
            let mut ids = Vec::new();
            let mut complete = false;
            for _ in 0..20_000 {
                let mut b = Budget::new(micro());
                search.swap_link_resolver(&mut resolver).unwrap();
                let step = search.next_match(&d, &mut b).unwrap();
                search.swap_link_resolver(&mut resolver).unwrap();
                assert!(b.used().queries <= 1 && b.used().objects <= 1 && b.used().bytes <= 6);
                match step {
                    HeadingSearchStep::Match(hit) => {
                        if query == "*foo" {
                            let start = d.headings().get(hit.heading).unwrap().content.start;
                            assert_eq!(hit.source, mdstream::Span::new(start, start + 4));
                        }
                        ids.push(hit.heading.0);
                    }
                    HeadingSearchStep::Complete => {
                        complete = true;
                        break;
                    }
                    HeadingSearchStep::BudgetExhausted => {}
                }
            }
            assert!(complete, "{defined} {query}: first-match pages must finish");
            assert_eq!(ids, expected, "{defined} {query}");
        }
        assert!(
            resolver.style_work().events < 64,
            "one shared scope evaluation"
        );
        if !defined {
            let mut stale = HeadingSearch::new(&d, "foo", 6).unwrap();
            d.append("[bar* baz]: /v\n[kid]: /k\n").unwrap();
            let mut b = Budget::new(micro());
            assert_eq!(
                stale.next_match(&d, &mut b),
                Err(LayoutError::StaleContinuation)
            );
            assert_eq!(b.used(), WorkLimit::default());
        }
    }
}

#[test]
fn conditional_first_hit_skips_remaining_giant_title_with_shared_resolver() {
    let d = parse(&format!("# *{} [r]\n\n# aa\n", "a".repeat(1_000_000)));
    let mut search = HeadingSearch::new(&d, "aa", 6).unwrap();
    let mut resolver = d.link_resolver();
    let mut paid = Vec::new();
    for id in 0..2 {
        let mut b = Budget::new(WorkLimit::unlimited());
        search.swap_link_resolver(&mut resolver).unwrap();
        let hit = search.next_match(&d, &mut b).unwrap();
        search.swap_link_resolver(&mut resolver).unwrap();
        assert!(matches!(hit, HeadingSearchStep::Match(hit) if hit.heading == HeadingId(id)));
        paid.push(b.used().bytes);
    }
    assert!(paid[0] < 8192 && paid[1] < 64, "{paid:?}");
    assert!(resolver.style_work().events > 0 && resolver.style_work().events <= 8);
    assert!(search.scratch_capacity_bytes() < 8192);
    eprintln!(
        "conditional heading first hits: paid_bytes={paid:?} styles={:?}",
        resolver.style_work()
    );
}
