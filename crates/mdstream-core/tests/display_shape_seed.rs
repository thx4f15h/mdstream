use mdstream::{Budget, Completion, Document, LayoutError, Span, WorkLimit};
fn micro() -> Budget {
    Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        ..Default::default()
    })
}
#[test]
fn public_rebind_helpers_reject_a_foreign_cursor_before_any_mutation() {
    let mut a = Document::new();
    a.append("[a](/a)\n").unwrap();
    let mut b = Document::new();
    b.append("[b](/b)\n").unwrap();
    assert_eq!(a.revision(), b.revision());
    let resolver = b.link_resolver();
    let mut activity = a.link_activity(mdstream::LinkId(0)).unwrap();
    let mut window = a.link_window(Span::new(0, a.source().len()));
    let mut markers = a.style_markers(Span::new(0, a.source().len())).unwrap();
    assert_eq!(
        activity.rebind_with(&b, &resolver),
        Err(LayoutError::WrongDocument)
    );
    assert_eq!(
        window.rebind_with(&b, &resolver),
        Err(LayoutError::WrongDocument)
    );
    assert_eq!(
        markers.rebind_with(&b, &resolver),
        Err(LayoutError::WrongDocument)
    );
    let resolver = a.link_resolver();
    activity.rebind_with(&a, &resolver).unwrap();
    window.rebind_with(&a, &resolver).unwrap();
    markers.rebind_with(&a, &resolver).unwrap();
}
#[test]
fn partially_gathered_real_checkpoint_is_retired_one_paid_node_at_a_time() {
    let mut d = Document::new();
    d.append(&format!("{}\n", "*x* [r] ".repeat(4000))).unwrap();
    d.append(&"*y* [s] ".repeat(4000)).unwrap();
    let mut seed = d.display_shape_seed();
    for _ in 0..10_000 {
        seed.advance(&d, &mut micro()).unwrap().ok();
        if seed.work().undo_copies > 8 {
            break;
        }
    }
    let before = seed.work();
    assert!(
        before.undo_copies > 8,
        "fixture must gather a real open checkpoint: {before:?}"
    );
    let capacity = seed.capacity_bytes();
    assert!(capacity > 8 * 128);
    assert_eq!(
        seed.retire(&mut Budget::new(WorkLimit::default())),
        Completion::BudgetExhausted
    );
    assert_eq!(seed.work(), before);
    let mut retired = 0;
    loop {
        let old = seed.work();
        let mut b = micro();
        let done = seed.retire(&mut b);
        let now = seed.work();
        assert!(now.retired_nodes - old.retired_nodes <= 1);
        assert!(b.used().objects <= 1);
        retired += now.retired_nodes - old.retired_nodes;
        if done == Completion::Covered {
            break;
        }
    }
    assert!(retired >= before.undo_copies);
    assert!(seed.capacity_bytes() < capacity);
}
