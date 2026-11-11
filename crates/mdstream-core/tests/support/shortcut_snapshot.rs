use mdstream::{Document, Span, Style};

#[path = "snapshot.rs"]
mod frozen;

// Compare every structural/payload field and every original scalar mapping.
// Only these frozen predecessor marker bytes may lose HIDDEN. This comparison
// does not render Markdown, choose references or change an expected fixture.
pub fn describe(doc: &Document, markers: &[(usize, usize)], new_behavior: bool) -> String {
    let mut snapshot = frozen::snapshot(doc);
    if markers.is_empty() {
        return format!("{snapshot:?}");
    }
    snapshot.runs.clear();
    snapshot.display.clear();
    snapshot.depths.clear();
    let mut result = format!("{snapshot:?}\n");
    for (start, c) in doc.source().char_indices() {
        let span = Span::new(start, start + c.len_utf8());
        let marker = markers.iter().any(|&(a, b)| a <= start && start < b);
        let mut raw = doc.runs_in(span).unwrap().next().unwrap();
        let mut display = doc.display_runs_in(span).unwrap().next().unwrap();
        if marker && new_behavior {
            assert!(
                !raw.style.contains(Style::HIDDEN),
                "{span:?} in {:?}",
                doc.source()
            );
            assert!(!display.style.contains(Style::HIDDEN));
            raw.style = raw.style.union(Style::HIDDEN);
            display.style = display.style.union(Style::HIDDEN);
        }
        result.push_str(&format!("{raw:?} {display:?}\n"));
    }
    result
}
