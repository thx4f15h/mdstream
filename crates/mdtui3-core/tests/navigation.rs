use mdtui3::{Document, HeadingId, HeightIndex, NavAction, Navigator, Span, TileId};

#[test]
fn headings_index_levels_source_ranges_and_setext() {
    let source = "# Top\n\n## Child ##\n\nbody\n===\n\n### Deep\n```\n# not a heading\n```\n";
    let mut doc = Document::new();
    let update = doc.append(source).unwrap();
    assert_eq!(update.new_headings, 0..4);
    doc.finish();
    let index = doc.headings();
    assert_eq!(index.entries().len(), 4);
    assert_eq!(index.at_level(1), [HeadingId(0), HeadingId(2)]);
    assert!(index.at_level(0).is_empty());
    assert!(index.at_level(7).is_empty());
    assert_eq!(
        doc.text(index.get(HeadingId(1)).unwrap().content)
            .unwrap()
            .trim(),
        "Child"
    );
    assert_eq!(
        doc.text(index.get(HeadingId(2)).unwrap().content),
        Some("body")
    );
    assert_eq!(index.next(0, 1), Some(HeadingId(2)));
    assert_eq!(index.next(0, 2), Some(HeadingId(1)));
    assert_eq!(index.previous(source.len(), 2), Some(HeadingId(2)));
    assert_eq!(index.next(source.len(), 6), None);
}

#[test]
fn heading_navigation_distinguishes_multiple_titles_in_one_tile() {
    let mut doc = Document::new();
    doc.append("# one\n## two\n### three\n").unwrap();
    let mut heights = HeightIndex::default();
    heights.rebuild(&doc).unwrap();
    let mut nav = Navigator::new();
    assert_eq!(
        nav.navigate(&doc, &heights, NavAction::Heading(HeadingId(0)), 24)
            .unwrap()
            .source,
        Some(0)
    );
    let two = nav
        .navigate(&doc, &heights, NavAction::NextHeading { max_level: 6 }, 24)
        .unwrap();
    assert_eq!(two.source, Some(6));
    assert_eq!(two.top, 0);
    let three = nav
        .navigate(&doc, &heights, NavAction::NextHeading { max_level: 6 }, 24)
        .unwrap();
    assert_eq!(three.source, Some(13));
    assert_eq!(
        nav.navigate(
            &doc,
            &heights,
            NavAction::PreviousHeading { max_level: 1 },
            24
        )
        .unwrap()
        .source,
        Some(0)
    );
    assert!(
        nav.navigate(&doc, &heights, NavAction::Heading(HeadingId(99)), 24)
            .is_none()
    );
}

#[test]
fn paging_lines_home_end_and_anchor_corrections() {
    let mut doc = Document::new();
    doc.append(&"x".repeat(256 * 100)).unwrap();
    let mut heights = HeightIndex::default();
    heights.rebuild(&doc).unwrap();
    let mut nav = Navigator::new();
    for (action, expected) in [
        (NavAction::LineUp, 0),
        (NavAction::LineDown, 1),
        (NavAction::PageDown, 25),
        (NavAction::PageUp, 1),
        (NavAction::End, 76),
        (NavAction::LineDown, 76),
        (NavAction::Home, 0),
    ] {
        assert_eq!(
            nav.navigate(&doc, &heights, action, 24).unwrap().top,
            expected
        );
    }
    nav.navigate(&doc, &heights, NavAction::PageDown, 24)
        .unwrap();
    heights
        .measure(heights.ticket(TileId(0)).unwrap(), 11)
        .unwrap();
    assert_eq!(nav.top(&heights), 34);
    heights.suppress(25..100).unwrap();
    assert_eq!(
        nav.navigate(&doc, &heights, NavAction::End, 24)
            .unwrap()
            .top,
        11
    );
    let empty = Document::new();
    assert_eq!(
        Navigator::new()
            .navigate(&empty, &HeightIndex::default(), NavAction::End, 0)
            .unwrap()
            .anchor,
        None
    );
}

#[test]
fn giant_setext_heading_index_has_no_title_copy_or_rescan() {
    let mut doc = Document::new();
    doc.append(&format!("{}\n", "x".repeat(1_000_000))).unwrap();
    let before = doc.work();
    doc.append("===\n").unwrap();
    assert_eq!(doc.headings().entries()[0].content, Span::new(0, 1_000_000));
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert!(doc.work().tree_visits - before.tree_visits < 200);
}

#[test]
fn long_atx_heading_content_extends_across_chunks() {
    let mut doc = Document::new();
    doc.append(&format!("# {}", "a".repeat(5000))).unwrap();
    doc.append("b\r").unwrap();
    doc.append("\n").unwrap();
    let h = &doc.headings().entries()[0];
    assert_eq!(doc.text(h.content).unwrap().len(), 5001);
    assert_eq!(h.source.end, doc.source().len());
}

#[test]
fn rendered_source_anchor_makes_heading_jumps_exact_after_line_scroll() {
    let mut doc = Document::new();
    doc.append("# a\n# b\n# c\n").unwrap();
    let mut heights = HeightIndex::default();
    heights.rebuild(&doc).unwrap();
    heights
        .measure(heights.ticket(TileId(0)).unwrap(), 3)
        .unwrap();
    let mut nav = Navigator::new();
    assert_eq!(
        nav.navigate(&doc, &heights, NavAction::LineDown, 1)
            .unwrap()
            .top,
        1
    );
    nav.reconcile_source(&heights, 1, 1, 4);
    assert_eq!(
        nav.navigate(&doc, &heights, NavAction::NextHeading { max_level: 6 }, 1)
            .unwrap()
            .source,
        Some(8)
    );
}
