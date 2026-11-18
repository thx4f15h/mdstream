use mdstream::{BlockId, Document, Span};
#[test]
fn indexed_logical_paragraph_metadata_retains_exact_leaf_identity_and_bounds() {
    let source = "- a\n  b\n\n  c\n- d\n\nout\n";
    for cut in 0..=source.len() {
        let mut doc = Document::new();
        doc.append(&source[..cut]).unwrap();
        doc.append(&source[cut..]).unwrap();
        doc.finish();
        let work = doc.work();
        for offset in [0, 2, 4, 6, 7] {
            let p = doc.paragraph_at(offset).unwrap();
            assert_eq!(p.block, BlockId(0));
            assert_eq!(p.source, Span::new(0, 8));
            assert!(!p.open);
        }
        assert!(doc.paragraph_at(8).is_none());
        let c = source.find("  c").unwrap();
        let p = doc.paragraph_at(c + 2).unwrap();
        assert_eq!(p.source, Span::new(c, c + 4));
        assert_ne!(p.block, BlockId(0));
        assert_eq!(p.container, doc.block(BlockId(0)).unwrap().container);
        assert!(doc.paragraph_at(source.len()).is_none());
        assert_eq!(doc.work(), work);
    }
}
#[test]
fn growing_and_late_setext_use_current_metadata_without_rescanning_body() {
    let mut doc = Document::new();
    doc.append("> a\n> b\n").unwrap();
    let p = doc.paragraph_at(6).unwrap();
    assert_eq!(p.source, Span::new(0, 8));
    assert!(p.open);
    doc.append("> ---\n").unwrap();
    assert!(doc.paragraph_at(2).is_none());
}

#[test]
fn late_table_and_loose_metadata_keep_exact_paragraph_boundaries() {
    let mut doc = Document::new();
    doc.append("> h | x\n").unwrap();
    let header = doc.paragraph_at(3).unwrap();
    assert!(header.open);
    doc.append("> - | -\n").unwrap();
    assert!(doc.paragraph_at(3).is_none());
    doc.finish();
    assert!(doc.paragraph_at(3).is_none());

    let mut doc = Document::new();
    doc.append("- a\n- b\n").unwrap();
    let first = doc.paragraph_at(2).unwrap();
    let second = doc.paragraph_at(6).unwrap();
    assert_eq!(first.source, Span::new(0, 4));
    assert_eq!(second.source, Span::new(4, 8));
    doc.append("\n- c\n").unwrap();
    assert_eq!(doc.paragraph_at(2).unwrap(), first);
    assert_eq!(doc.paragraph_at(6).unwrap().source, second.source);
    assert!(doc.paragraph_at(8).is_none());
    assert!(doc.paragraph_at(usize::MAX).is_none());
    let work = doc.work();
    for _ in 0..100 {
        assert_eq!(doc.paragraph_at(2).unwrap().block, first.block);
    }
    assert_eq!(doc.work(), work);
}
