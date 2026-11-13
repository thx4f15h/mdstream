use mdstream::{Budget, Document, LayoutError, Span, StyleMarkerStep, WorkLimit};
fn collect(d: &Document, source: Span, reverse: bool) -> Vec<mdstream::ResolvedStyleSpan> {
    let mut resolver = d.link_resolver();
    let mut cursor = if reverse {
        d.style_markers_reverse(source)
    } else {
        d.style_markers(source)
    }
    .unwrap();
    let mut result = Vec::new();
    for _ in 0..1_000_000 {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 6,
            ..Default::default()
        });
        match cursor.advance(d, &mut resolver, &mut budget).unwrap() {
            StyleMarkerStep::Span(s) => {
                assert!(d.text(s.source).is_some());
                result.push(s)
            }
            StyleMarkerStep::BudgetExhausted => {}
            StyleMarkerStep::Complete => return result,
        }
        assert!(budget.used().queries <= 1 && budget.used().objects <= 1);
    }
    panic!("native marker cursor did not finish")
}
#[test]
fn reverse_native_spans_exactly_match_forward_snapshots_across_utf8_scopes_and_conditionals() {
    for source in [
        "a **界e\u{301}** z\n\n*second*\n",
        "*a [x* z]\n\n",
        "[a **b][r]\n\n",
        "no styles 界\n\n",
        "**x**\n\n[hidden]: /url\n\nnext\n",
    ] {
        let mut d = Document::new();
        d.append(source).unwrap();
        for suffix in ["", "\n[x* z]: /u\n[r]: /v\n"] {
            d.append(suffix).unwrap();
            for source in [
                Span::new(0, d.source().len()),
                Span::new(1, d.source().len()),
                Span::new(0, 0),
            ] {
                let forward = collect(&d, source, false);
                let mut reverse = collect(&d, source, true);
                reverse.reverse();
                assert_eq!(reverse, forward, "{:?} {source:?}", d.source());
            }
        }
    }
}
#[test]
fn reverse_marker_ownership_precedes_zero_budget_and_completion() {
    let mut a = Document::new();
    a.append("界").unwrap();
    let mut b = Document::new();
    b.append("abc").unwrap();
    let mut cursor = a.style_markers_reverse(Span::new(0, 0)).unwrap();
    let mut resolver = b.link_resolver();
    let mut zero = Budget::new(WorkLimit::default());
    assert_eq!(
        cursor.advance(&b, &mut resolver, &mut zero),
        Err(LayoutError::WrongDocument)
    );
    assert_eq!(zero.used(), WorkLimit::default());
    let mut resolver = a.link_resolver();
    assert_eq!(
        cursor.advance(&a, &mut resolver, &mut zero).unwrap(),
        StyleMarkerStep::Complete
    );
}
