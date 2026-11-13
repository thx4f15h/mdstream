use mdtui_tui::render::{
    objects::SharedLinks,
    shape::{
        ShapeCluster, ShapeQuery, ShapeSeed, ShapeSelect, ShapeWindow, ShapeWindowStep, SharedShape,
    },
};
use mdtui3::{Budget, Document, LayoutError, Span, WorkLimit};
fn all() -> Budget {
    Budget::new(WorkLimit::unlimited())
}
fn setup(text: &str) -> (Document, SharedShape, SharedShape, SharedLinks) {
    let mut d = Document::new();
    d.enable_display_shape_effects().unwrap();
    let first = SharedShape::new(&d).unwrap();
    let second = SharedShape::new(&d).unwrap();
    let update = d.append(text).unwrap();
    first.apply(&d, &update).unwrap();
    second.apply(&d, &update).unwrap();
    let links = SharedLinks::new(&d);
    links.set_shape(first.clone()).unwrap();
    (d, first, second, links)
}
#[test]
fn seed_checks_document_and_first_index_before_source_mutation_or_budget() {
    let mut a = Document::new();
    a.append("abc").unwrap();
    let b = Document::new();
    let sb = SharedShape::new(&b).unwrap();
    let mut seed = ShapeSeed::new(&a);
    for limit in [
        WorkLimit::default(),
        WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 6,
            ..Default::default()
        },
        WorkLimit::unlimited(),
    ] {
        let work = sb.work();
        let mut budget = Budget::new(limit);
        assert_eq!(
            seed.advance(&sb, &a, &mut budget),
            Err(LayoutError::WrongDocument)
        );
        assert_eq!(budget.used(), WorkLimit::default());
        assert_eq!(sb.source_len(), 0);
        assert_eq!(sb.work(), work);
    }
    // One document can still own two distinct current indexes. A partially
    // advanced seed must not transfer its source/undo cursor to the second.
    let mut d = Document::new();
    d.append("abc").unwrap();
    let one = SharedShape::from_current(&d);
    let two = SharedShape::from_current(&d);
    let mut seed = ShapeSeed::new(&d);
    let mut budget = Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 1,
        ..Default::default()
    });
    assert!(!seed.advance(&one, &d, &mut budget).unwrap());
    let before = two.work();
    let mut zero = Budget::new(WorkLimit::default());
    assert_eq!(
        seed.advance(&two, &d, &mut zero),
        Err(LayoutError::WrongDocument)
    );
    assert_eq!(two.work(), before);
    assert_eq!(two.source_len(), 0);
    assert!(seed.advance(&one.clone(), &d, &mut all()).unwrap());
    assert_eq!(one.source_len(), 3);
    assert_eq!(
        seed.advance(&two, &d, &mut all()),
        Err(LayoutError::WrongDocument)
    );
}
#[test]
fn all_public_queries_reject_foreign_or_replaced_context_before_cached_results() {
    let (a, sa, sa2, la) = setup("a\u{301}");
    let (b, sb, _, lb) = setup("abc");
    let la2 = SharedLinks::new(&a);
    la2.set_shape(sa2.clone()).unwrap();
    let span = Span::new(0, 3);
    assert_eq!(a.revision(), b.revision());
    let mut query = ShapeQuery::graphemes(&sa, span);
    let mut select = ShapeSelect::new(&sa, span, 0);
    let mut cluster = ShapeCluster::new(&sa, span, 0);
    let mut window = ShapeWindow::new(&sa, span, span).unwrap();
    let mut empty = ShapeWindow::new(&sa, Span::new(0, 0), span).unwrap();
    for cached in [false, true] {
        for limit in [
            WorkLimit::default(),
            WorkLimit {
                queries: 1,
                objects: 1,
                bytes: 6,
                ..Default::default()
            },
        ] {
            for (shape, doc, links) in [
                (&sb, &b, &lb),
                (&sa, &b, &lb),
                (&sa, &a, &lb),
                (&sa2, &a, &la),
                (&sa2, &a, &la2),
                (&sa, &a, &la2),
            ] {
                let before_a = sa.work();
                let before_b = sb.work();
                let before_2 = sa2.work();
                let mut zero = Budget::new(limit);
                assert_eq!(
                    query.advance(shape, doc, links, &mut zero),
                    Err(LayoutError::WrongDocument),
                    "cached={cached}"
                );
                assert_eq!(
                    select.advance(shape, doc, links, &mut zero),
                    Err(LayoutError::WrongDocument)
                );
                assert!(matches!(
                    cluster.advance(shape, doc, links, &mut zero),
                    Err(LayoutError::WrongDocument)
                ));
                assert!(matches!(
                    window.advance(shape, doc, links, &mut zero),
                    Err(LayoutError::WrongDocument)
                ));
                assert!(matches!(
                    empty.advance(shape, doc, links, &mut zero),
                    Err(LayoutError::WrongDocument)
                ));
                assert_eq!(zero.used(), WorkLimit::default());
                assert_eq!(sa.work(), before_a);
                assert_eq!(sb.work(), before_b);
                assert_eq!(sa2.work(), before_2);
            }
        }
        assert_eq!(
            query.advance(&sa.clone(), &a, &la, &mut all()).unwrap(),
            Some(1)
        );
        assert!(
            select
                .advance(&sa.clone(), &a, &la, &mut all())
                .unwrap()
                .is_some()
        );
        assert!(
            cluster
                .advance(&sa.clone(), &a, &la, &mut all())
                .unwrap()
                .is_some()
        );
        if !cached {
            assert!(matches!(
                window.advance(&sa.clone(), &a, &la, &mut all()).unwrap(),
                ShapeWindowStep::Glyph(_)
            ));
        }
        assert!(matches!(
            window.advance(&sa.clone(), &a, &la, &mut all()).unwrap(),
            ShapeWindowStep::Complete
        ));
        assert!(matches!(
            empty.advance(&sa.clone(), &a, &la, &mut all()).unwrap(),
            ShapeWindowStep::Complete
        ));
    }
}
#[test]
fn shared_links_cannot_adopt_foreign_shape_replace_current_index_or_reset_a_seed() {
    let (a, sa, sa2, la) = setup("a\u{301}");
    let (b, sb, _, _) = setup("abc");
    assert_eq!(la.set_shape(sb), Err(LayoutError::WrongDocument));
    assert_eq!(la.set_shape(sa2), Err(LayoutError::WrongDocument));
    la.set_shape(sa.clone()).unwrap();
    let before = sa.work();
    let mut zero = Budget::new(WorkLimit::default());
    assert_eq!(
        la.prepare_shape(&b, &mut zero),
        Err(LayoutError::WrongDocument)
    );
    assert_eq!(la.prepare(&b, &mut zero), Err(LayoutError::WrongDocument));
    assert_eq!(zero.used(), WorkLimit::default());
    assert_eq!(sa.work(), before);
    let adoption = SharedLinks::new(&a);
    assert!(!adoption.prepare_shape(&a, &mut zero).unwrap());
    let adopted = adoption.shape().unwrap();
    assert!(!adoption.shape_ready());
    adoption.set_shape(adopted.clone()).unwrap();
    assert!(
        !adoption.shape_ready(),
        "setting the same index must not discard its pending seed"
    );
    assert!(adoption.prepare_shape(&a, &mut all()).unwrap());
    assert_eq!(adopted.work().source_decode_bytes, a.source().len());
    // Even the public native-resolver callback cannot disguise a foreign
    // resolver behind the old SharedLinks document identity.
    la.with_resolver(|r| r.reset(&b)).unwrap();
    let mut q = ShapeQuery::graphemes(&sa, Span::new(0, 3));
    assert_eq!(
        q.advance(&sa, &a, &la, &mut zero),
        Err(LayoutError::WrongDocument)
    );
    assert_eq!(
        la.prepare_shape(&a, &mut zero),
        Err(LayoutError::WrongDocument)
    );
    assert_eq!(zero.used(), WorkLimit::default());
}
