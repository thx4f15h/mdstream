use mdstream::*;
#[test]
fn paid_cut_change_keeps_identity_choice_preview_and_rejects_non_paragraph() {
    let mut doc = Document::new();
    doc.append("ab\r\ncdef\n").unwrap();
    doc.finish();
    let mut view = FoldView::<()>::with_state(&doc, 4).unwrap();
    let mut registration =
        SourceFoldRegistration::paragraph_overlay(&doc, &view, BlockId(0), 0).unwrap();
    let FoldId::Source(id) = registration
        .advance(&doc, &mut view, &mut Budget::new(WorkLimit::unlimited()))
        .unwrap()
        .id
        .unwrap()
    else {
        panic!()
    };
    view.set_collapsed(&doc, FoldId::Source(id), true).unwrap();
    let before = view.version();
    assert_eq!(
        view.set_paragraph_fold_cut(&doc, id, 4, &mut Budget::new(WorkLimit::default())),
        Ok(None)
    );
    assert_eq!(view.version(), before);
    assert_eq!(
        view.set_paragraph_fold_cut(&doc, id, 3, &mut Budget::new(WorkLimit::unlimited())),
        Err(LayoutError::InvalidRange)
    );
    let mut one = Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        ..Default::default()
    });
    view.set_paragraph_fold_cut(&doc, id, 4, &mut one)
        .unwrap()
        .unwrap();
    assert_eq!(one.used().queries, 1);
    assert_eq!(one.used().bytes, 0);
    let c = view
        .fold_candidate(&doc, FoldId::Source(id))
        .unwrap()
        .unwrap();
    assert_eq!(c.hidden, Span::new(4, 9));
    assert!(view.is_collapsed(c.id));
    view.set_preview(&doc, Some(c.id), &mut Budget::new(WorkLimit::unlimited()))
        .unwrap();
    view.set_paragraph_fold_cut(&doc, id, 0, &mut Budget::new(WorkLimit::unlimited()))
        .unwrap()
        .unwrap();
    assert_eq!(view.preview(), Some(c.id));
    assert!(view.is_collapsed(c.id));
    assert_eq!(
        view.hidden_at(&doc, 0, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap(),
        FoldLookup::Visible
    );
    view.set_preview(&doc, None, &mut Budget::new(WorkLimit::unlimited()))
        .unwrap();
    assert!(matches!(
        view.hidden_at(&doc, 0, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap(),
        FoldLookup::Hidden(_)
    ));
    assert_eq!(view.source_fold_count(), 1);
}
