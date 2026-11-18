use mdtui_tui::render::{objects::SharedLinks, prefix::SafePrefixCut};
use mdtui3::{Budget, Document, Span, WorkLimit};
fn cut(source: &str, proposed: usize) -> usize {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    let links = SharedLinks::new(&doc);
    let mut task = SafePrefixCut::new(&doc, Span::new(0, source.len()), proposed).unwrap();
    let mut zero = Budget::new(WorkLimit::default());
    assert_eq!(task.advance(&doc, &links, &mut zero).unwrap(), None);
    assert_eq!(zero.used(), WorkLimit::default());
    for _ in 0..100_000 {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 6,
            ..Default::default()
        });
        if let Some(cut) = task.advance(&doc, &links, &mut budget).unwrap() {
            return cut;
        }
    }
    panic!("safe cut made no progress")
}
#[test]
fn handwritten_complete_display_atoms_and_native_objects() {
    assert_eq!(cut("ae\u{301}z\n", 2), 1);
    assert_eq!(cut("A&amp;B\n", 3), 1);
    assert_eq!(cut("a![longalt](url)z\n", 5), 1);
    assert_eq!(cut("a[label](url)z\n", 5), 1);
    assert_eq!(cut("a $x+y$ z\n", 5), 2);
    assert_eq!(cut("$x$\u{301}z\n", 3), 3);
    assert_eq!(cut("a $\\frac{b}{c}$ z\n", 8), 2);
    // A literal unresolved candidate has no native object to preserve.
    assert_eq!(cut("a[abc][missing]z\n", 4), 4);
}
#[test]
fn replacement_atom_and_grapheme_share_the_boundary() {
    assert_eq!(cut("a&NotEqualTilde;z\n", 5), 1);
    assert_eq!(cut("a\\*\u{301}z\n", 3), 1);
}
fn logical(source: &str, lines: usize) -> usize {
    use mdtui_tui::render::prefix::LogicalPrefix;
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    let links = SharedLinks::new(&doc);
    let mut task = LogicalPrefix::new(&doc, Span::new(0, source.len()), lines).unwrap();
    for _ in 0..100_000 {
        if let Some(cut) = task
            .advance(
                &doc,
                &links,
                &mut Budget::new(WorkLimit {
                    queries: 1,
                    objects: 1,
                    bytes: 6,
                    ..Default::default()
                }),
            )
            .unwrap()
        {
            return cut;
        }
    }
    panic!("logical prefix made no progress")
}
#[test]
fn handwritten_logical_lines_are_display_lfs_not_source_bytes_or_terminal_wraps() {
    assert_eq!(logical("abcdefghi\n", 0), 0);
    assert_eq!(logical("abcdefghi\n", 1), 10);
    assert_eq!(logical("ab\r\ncd\nef\n", 1), 4);
    assert_eq!(logical("ab\r\ncd\nef\n", 2), 7);
    assert_eq!(logical("ab&#10;cd\n", 1), 7);
    assert_eq!(logical("`ab\ncd` z\n", 1), 10);
    assert_eq!(logical("a $b\nc$ z\n", 1), 2);
}
fn display(source: &str, rows: usize, width: u16) -> usize {
    use mdtui_tui::render::prefix::DisplayPrefix;
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    let links = SharedLinks::new(&doc);
    let mut task = DisplayPrefix::new(&doc, Span::new(0, source.len()), rows, width).unwrap();
    let mut zero = Budget::new(WorkLimit::default());
    assert_eq!(
        task.advance(&doc, &links, &mut zero).unwrap(),
        (rows == 0).then_some(0)
    );
    assert_eq!(zero.used(), WorkLimit::default());
    for _ in 0..200_000 {
        if let Some(cut) = task
            .advance(
                &doc,
                &links,
                &mut Budget::new(WorkLimit {
                    queries: 1,
                    objects: 1,
                    bytes: 6,
                    tiles: 1,
                    recovery: 1,
                    dependencies: 1,
                }),
            )
            .unwrap()
        {
            return cut;
        }
    }
    panic!("real display prefix made no progress")
}
#[test]
fn handwritten_display_rows_use_real_layout_and_complete_atoms() {
    assert_eq!(display("abcdefghi\n", 0, 4), 0);
    assert_eq!(display("abcdefghi\n", 1, 4), 4);
    assert_eq!(display("abcdefghi\n", 2, 4), 8);
    assert_eq!(display("abcdefghi\n", 3, 4), 10);
    assert_eq!(display("A&amp;BCDE\n", 1, 4), 8);
    assert_eq!(display("A\\*BCDE\n", 1, 4), 5);
    assert_eq!(display("a界e\u{301}z\n", 1, 4), 7);
    assert_eq!(display("a![longalt](url)z\n", 1, 4), 1);
    assert_eq!(display("a $\\frac{b}{c}$ z\n", 1, 20), 2);
    assert_eq!(display("abc&fjlig;z\n", 1, 4), 3);
    assert_eq!(display("a\\*\u{301}z\n", 1, 1), 1);
}
#[test]
fn prefix_tasks_reject_foreign_owner_document_and_stale_revision_before_zero_work() {
    use mdtui_tui::render::prefix::{DisplayPrefix, LogicalPrefix};
    let mut d = Document::new();
    d.append("ae\u{301}xyz\n").unwrap();
    let source = Span::new(0, d.source().len());
    let a = SharedLinks::new(&d);
    let b = SharedLinks::new(&d);
    let mut safe = SafePrefixCut::new(&d, source, 2).unwrap();
    let mut logical = LogicalPrefix::new(&d, source, 1).unwrap();
    let mut display = DisplayPrefix::new(&d, source, 1, 2).unwrap();
    assert_eq!(
        safe.advance(&d, &a, &mut Budget::new(WorkLimit::default()))
            .unwrap(),
        None
    );
    assert_eq!(
        logical
            .advance(&d, &a, &mut Budget::new(WorkLimit::default()))
            .unwrap(),
        None
    );
    assert_eq!(
        display
            .advance(&d, &a, &mut Budget::new(WorkLimit::default()))
            .unwrap(),
        None
    );
    let mut zero = Budget::new(WorkLimit::default());
    assert_eq!(
        safe.advance(&d, &b, &mut zero),
        Err(mdtui3::LayoutError::WrongDocument)
    );
    assert_eq!(
        logical.advance(&d, &b, &mut zero),
        Err(mdtui3::LayoutError::WrongDocument)
    );
    assert!(matches!(
        display.advance(&d, &b, &mut zero),
        Err(mdtui_tui::render::first_frame::FirstFrameError::Layout(
            mdtui3::LayoutError::WrongDocument
        ))
    ));
    d.append("tail\n").unwrap();
    assert_eq!(
        safe.advance(&d, &a, &mut zero),
        Err(mdtui3::LayoutError::StaleContinuation)
    );
    assert_eq!(
        logical.advance(&d, &a, &mut zero),
        Err(mdtui3::LayoutError::StaleContinuation)
    );
    assert_eq!(zero.used(), WorkLimit::default());
}
#[test]
fn inline_and_two_dimensional_objects_and_cross_tile_egc_have_handwritten_prefixes() {
    assert_eq!(display("abcd $x$ rest\n", 1, 4), 4);
    assert_eq!(display("a $\\frac{b}{c}$ z\n", 3, 20), 18);
    assert_eq!(display("a\tb界c\n", 1, 4), 3);
    let source = format!("a{}z\n", "\u{301}".repeat(5000));
    assert_eq!(display(&source, 1, 1), 10001);
}
#[test]
fn long_internal_and_trailing_whitespace_preparation_is_paid_per_atom() {
    use mdtui_tui::render::prefix::DisplayPrefix;
    for suffix in ["z\n", "\n"] {
        let source = format!("a{}{suffix}", " ".repeat(1024));
        let mut doc = Document::new();
        doc.append(&source).unwrap();
        doc.finish();
        let links = SharedLinks::new(&doc);
        let mut task = DisplayPrefix::new(&doc, Span::new(0, source.len()), 1, 4).unwrap();
        let mut completed = None;
        let mut calls = 0;
        for _ in 0..200_000 {
            calls += 1;
            let before = task.prepared_cells();
            let mut budget = Budget::new(WorkLimit {
                queries: 1,
                objects: 1,
                bytes: 6,
                tiles: 1,
                recovery: 1,
                dependencies: 1,
            });
            completed = task.advance(&doc, &links, &mut budget).unwrap();
            assert!(
                task.prepared_cells() - before <= 1,
                "a one-object call prepared a batch"
            );
            if completed.is_some() {
                break;
            }
            assert_ne!(
                budget.used(),
                WorkLimit::default(),
                "zero-cost stalled call at {calls}, prepared {}, quanta {}",
                task.prepared_cells(),
                task.probes()
            );
        }
        eprintln!(
            "whitespace suffix={suffix:?} calls={calls} atoms={} quanta={} charged_bytes={} shape={:?}",
            task.prepared_cells(),
            task.probes(),
            task.source_work(),
            links.shape().map(|s| s.work())
        );
        assert_eq!(
            completed,
            Some(if suffix == "z\n" { 4 } else { source.len() })
        );
    }
}
#[test]
fn a_native_object_resets_the_following_cluster_context() {
    assert_eq!(display("$x$\u{301}z\n", 1, 1), 5);
}
#[test]
fn the_giant_cluster_after_native_math_crosses_source_quanta_whole() {
    let source = format!("$x${}z\n", "\u{301}".repeat(5000));
    assert_eq!(display(&source, 1, 1), 10003);
}

#[test]
fn handwritten_quote_and_list_body_rows_use_production_prefixes_and_word_wraps() {
    assert_eq!(display("> abcdef\n> ghij\n", 1, 6), 6);
    assert_eq!(display("> abcdef\n> ghij\n", 2, 6), 12);
    assert_eq!(display("> abcdef\n> ghij\n", 3, 6), 16);
    assert_eq!(display("- abc de fg\n", 1, 6), 5);
    assert_eq!(display("- abc de fg\n", 2, 6), 8);
    assert_eq!(display("- abcdefghij\n", 1, 6), 6);
    assert_eq!(display("- abcdefghij\n", 2, 6), 10);
    assert_eq!(display("- 甲乙a。\n", 1, 8), 9);
    assert_eq!(display("- a $\\frac{b}{c}$ z\n", 1, 20), 4);
}

#[test]
fn empty_images_keep_the_existing_placeholder_and_whole_source_atom_cut() {
    assert_eq!(display("A ![](url) Z\n", 1, 4), 2);
    assert_eq!(display("A ![\\(\\)](url) Z\n", 1, 4), 2);
    assert_eq!(display("A ![\\(\\)](url) Z\n", 1, 24), 17);
}
