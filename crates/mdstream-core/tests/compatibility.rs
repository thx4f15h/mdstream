use mdstream::*;
include!("fixtures/commonmark_autolinks.rs");
include!("fixtures/commonmark_display.rs");

#[test]
fn pinned_commonmark_display_text_cases() {
    assert_eq!(DISPLAY_CASES.len(), 24);
    for &(number, markdown, expected) in DISPLAY_CASES {
        let mut doc = Document::new();
        doc.append(markdown).unwrap();
        doc.finish();
        let visible: String = doc
            .display_runs_in(Span::new(0, markdown.len()))
            .unwrap()
            .filter(|r| !r.style.contains(Style::HIDDEN))
            .map(|r| r.text)
            .collect();
        assert_eq!(visible.trim_end_matches('\n'), expected, "example {number}");
    }
    eprintln!(
        "CommonMark 0.31.2: 24 selected display-text cases supported; HTML, attributes and full code fallback are not scored"
    );
}

#[test]
fn pinned_commonmark_autolinks_report_supported_and_deliberate_differences() {
    let mut supported = 0;
    let mut differences = 0;
    for &(number, markdown, expected_url, expected_text, agrees) in AUTOLINKS {
        let mut doc = Document::new();
        doc.append(markdown).unwrap();
        doc.finish();
        let gfm_url = match number {
            608 => Some("https://foo.bar"),
            611 => Some("https://example.com"),
            612 => Some("mailto:foo@bar.example.com"),
            _ => expected_url,
        };
        assert_eq!(
            doc.link_url(LinkId(0)).as_deref(),
            gfm_url,
            "example {number}"
        );
        let visible: String = doc
            .runs_in(Span::new(0, markdown.len()))
            .unwrap()
            .filter(|run| !run.style.contains(Style::HIDDEN))
            .map(|run| run.text)
            .collect();
        if matches!(number, 608 | 611 | 612) {
            assert_ne!(gfm_url, expected_url);
            assert_eq!(visible.trim_end_matches('\n'), expected_text);
            differences += 1;
        } else if agrees {
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
    assert_eq!((supported, differences), (15, 4));
    eprintln!(
        "CommonMark 0.31.2 angle autolinks: {supported} supported, {differences} deliberate difference; other sections not scored"
    );
}
