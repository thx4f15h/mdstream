use mdstream::{Budget, Completion, DisplaySearch, Document, Span, WorkLimit};

#[test]
fn single_hit_steps_share_resolver_overlap_and_resume_utf8() {
    let mut doc = Document::new();
    doc.append("x **&amp;界a**a &amp;界aaa\n").unwrap();
    doc.finish();
    let mut resolver = doc.link_resolver();
    let mut search = DisplaySearch::new(&doc, "&界a").unwrap();
    let mut zero = Budget::new(WorkLimit::default());
    assert_eq!(
        search
            .next_with_resolver(&doc, &mut resolver, &mut zero)
            .unwrap(),
        (Completion::BudgetExhausted, None)
    );
    assert_eq!(search.source_position(), 0);
    let mut hits = Vec::new();
    for _ in 0..10000 {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 6,
            tiles: 1,
            ..WorkLimit::default()
        });
        let (done, hit) = search
            .next_with_resolver(&doc, &mut resolver, &mut budget)
            .unwrap();
        if let Some(hit) = hit {
            hits.push((hit.display, hit.source));
        }
        if done == Completion::EndOfDocument {
            break;
        }
    }
    assert_eq!(
        hits,
        vec![(2..7, Span::new(4, 13)), (9..14, Span::new(17, 26))]
    );
    let mut overlap = DisplaySearch::new(&doc, "aa").unwrap();
    let mut count = 0;
    loop {
        let (done, hit) = overlap
            .next_with_resolver(
                &doc,
                &mut resolver,
                &mut Budget::new(WorkLimit::unlimited()),
            )
            .unwrap();
        count += usize::from(hit.is_some());
        if done == Completion::EndOfDocument {
            break;
        }
    }
    assert_eq!(count, 3);
}

#[test]
fn revision_rejects_single_hit_step_before_budget_or_resolver_changes() {
    let mut doc = Document::new();
    doc.append("[x][r]").unwrap();
    let mut search = DisplaySearch::new(&doc, "x").unwrap();
    let mut resolver = doc.link_resolver();
    doc.append("\n\n[r]: /ok\n").unwrap();
    let mut budget = Budget::new(WorkLimit::unlimited());
    assert_eq!(
        search
            .next_with_resolver(&doc, &mut resolver, &mut budget)
            .unwrap_err(),
        mdstream::LayoutError::StaleContinuation
    );
    assert_eq!(budget.used(), WorkLimit::default());
}

#[test]
fn every_utf8_input_cut_and_micro_frame_matches_independent_static_hits() {
    let source = "x **&amp;界a**a &amp;界aaa\n";
    for cut in 0..=source.len() {
        let mut stream = mdstream::Utf8Document::new();
        stream.append(&source.as_bytes()[..cut]).unwrap();
        stream.append(&source.as_bytes()[cut..]).unwrap();
        stream.finish();
        let doc = stream.document();
        let mut search = DisplaySearch::new(doc, "&界a").unwrap();
        let mut resolver = doc.link_resolver();
        let mut hits = Vec::new();
        for _ in 0..10000 {
            let (done, hit) = search
                .next_with_resolver(
                    doc,
                    &mut resolver,
                    &mut Budget::new(WorkLimit {
                        queries: 1,
                        objects: 1,
                        bytes: 6,
                        ..WorkLimit::default()
                    }),
                )
                .unwrap();
            if let Some(hit) = hit {
                hits.push((hit.display, hit.source));
            }
            if done == Completion::EndOfDocument {
                break;
            }
        }
        assert_eq!(
            hits,
            vec![(2..7, Span::new(4, 13)), (9..14, Span::new(17, 26))],
            "cut={cut}"
        );
    }
}
