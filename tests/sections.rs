use mdtui3::*;

#[test]
fn skipped_levels_have_distinct_siblings_parents_and_exact_level_navigation() {
    let mut doc = Document::new();
    doc.append("# A\n### B\n#### C\n## D\n### E\n# F\n")
        .unwrap();
    let h = doc.headings();
    assert_eq!(h.parent(HeadingId(1)), Some(HeadingId(0)));
    assert_eq!(h.parent(HeadingId(3)), Some(HeadingId(0)));
    assert_eq!(h.next_sibling(HeadingId(1)), Some(HeadingId(3)));
    assert_eq!(h.previous_sibling(HeadingId(3)), Some(HeadingId(1)));
    assert_eq!(h.next_sibling(HeadingId(3)), None);
    assert_eq!(h.first_child(HeadingId(0)), Some(HeadingId(1)));
    assert_eq!(h.section_end(HeadingId(0)), Some(HeadingId(5)));
    assert_eq!(
        h.next_at_level(h.get(HeadingId(1)).unwrap().source.start, 3),
        Some(HeadingId(4))
    );
    assert_eq!(
        h.section(HeadingId(5), doc.source().len()).unwrap().end,
        doc.source().len()
    );
}
