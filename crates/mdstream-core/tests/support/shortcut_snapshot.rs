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
        let complementary_emphasis = doc.source() == "[a][*b*&amp;\\!]" && (4..7).contains(&start);
        if complementary_emphasis {
            // The inactive full suffix now has a real classified emphasis
            // pair. Keep this older scalar ledger's field projection, after
            // independently asserting the exact new marker/payload semantics.
            assert_eq!(raw.style.contains(Style::HIDDEN), matches!(start, 4 | 6));
            assert_eq!(raw.emphasis_depth, usize::from(start == 5));
            assert_eq!(
                raw.style,
                if start == 5 {
                    Style::EMPHASIS
                } else {
                    Style::HIDDEN
                }
            );
            assert_eq!(display.style, raw.style);
            raw.style = Style::default();
            raw.emphasis_depth = 0;
            display.style = Style::default();
            display.emphasis_depth = 0;
        }
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
        // This predecessor ledger predates ordered identities. Preserve its
        // exact field projection; ordered paths have independent assertions.
        result.push_str(&format!("RenderRun {{ source: {:?}, text: {:?}, block: {:?}, style: {:?}, emphasis_depth: {}, strong_depth: {}, link: {:?} }} DisplayRun {{ source: {:?}, replacement: {:?}, text: {:?}, block: {:?}, style: {:?}, emphasis_depth: {}, strong_depth: {}, link: {:?} }}\n",
            raw.source,raw.text,raw.block,raw.style,raw.emphasis_depth,raw.strong_depth,raw.link,
            display.source,display.replacement,display.text,display.block,display.style,display.emphasis_depth,display.strong_depth,display.link));
    }
    result
}
