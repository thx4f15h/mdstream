use mdtui3::*;
include!("fixtures/commonmark_autolinks.rs");

#[test]
fn pinned_commonmark_autolinks_report_supported_and_deliberate_differences() {
    let mut supported = 0;
    let mut differences = 0;
    for &(number, markdown, expected_url, expected_text, agrees) in AUTOLINKS {
        let mut doc = Document::new();
        doc.append(markdown).unwrap();
        doc.finish();
        assert_eq!(
            doc.link_url(LinkId(0)).as_deref(),
            expected_url,
            "example {number}"
        );
        let visible: String = doc
            .runs_in(Span::new(0, markdown.len()))
            .unwrap()
            .filter(|run| !run.style.contains(Style::HIDDEN))
            .map(|run| run.text)
            .collect();
        if agrees {
            assert_eq!(
                visible.trim_end_matches('\n'),
                expected_text,
                "example {number}"
            );
            supported += 1;
        } else {
            assert_ne!(
                visible.trim_end_matches('\n'),
                expected_text,
                "review changed dialect for example {number}"
            );
            differences += 1;
        }
    }
    assert_eq!((supported, differences), (18, 1));
    eprintln!(
        "CommonMark 0.31.2 angle autolinks: {supported} supported, {differences} deliberate difference; other sections not scored"
    );
}
