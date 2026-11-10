use mdstream::{Document, Span, Style};

// The existing bit-only projections deliberately reject simultaneous starts.
// Assert public text/depth observations separately, without inferring nesting order.
pub fn check(number: usize, doc: &Document) {
    let expected: &[(&str, usize, usize)] = match number {
        413 => &[("foo", 1, 1), (" bar", 1, 0), ("\n", 0, 0)],
        416 => &[("foo", 0, 0), ("bar", 1, 1), ("baz\n", 0, 0)],
        430 => &[("foo", 1, 1), (" bar", 0, 1), ("\n", 0, 0)],
        467 => &[("foo", 1, 1), ("\n", 0, 0)],
        468 => &[("foo", 1, 2), ("\n", 0, 0)],
        _ => panic!("unreviewed observation {number}"),
    };
    let actual: Vec<_> = doc
        .display_runs_in(Span::new(0, doc.source().len()))
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .flat_map(|r| {
            r.text
                .chars()
                .map(move |c| (c, r.emphasis_depth, r.strong_depth))
        })
        .collect();
    let expected: Vec<_> = expected
        .iter()
        .flat_map(|&(s, e, d)| s.chars().map(move |c| (c, e, d)))
        .collect();
    assert_eq!(actual, expected, "example {number}");
}
