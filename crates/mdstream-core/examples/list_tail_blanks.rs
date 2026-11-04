use mdstream::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new();
    // Only the blank outside the quote can separate the outer sibling.
    doc.append("- outer\n  - > child\n    >\n    \n")?;
    let list = ContainerId(0);
    let inner = ContainerId(2);
    let mut view = FoldView::new(&doc, 1)?;
    let fold = FoldId::Container(list);
    view.set_collapsed(&doc, fold, true)?;
    assert_eq!(
        doc.container(list).unwrap().list_tightness,
        Some(ListTightness::Tight)
    );
    let boundary = doc.source().len();
    let before = doc.work();
    let update = doc.append("- sibling\n")?;
    assert_eq!(update.loosened_lists, [list]);
    assert_eq!(doc.work().list_steps - before.list_steps, 4);
    assert_eq!(
        doc.container(inner).unwrap().list_tightness,
        Some(ListTightness::Tight)
    );
    assert_eq!(doc.container(inner).unwrap().source.end, boundary);
    assert_eq!(doc.container(inner).unwrap().state, SyntaxState::Sealed);
    view.apply(&doc, &update)?;
    let update = doc.finish();
    assert!(update.loosened_lists.is_empty());
    view.apply(&doc, &update)?;
    assert!(view.is_collapsed(fold));

    let mut cursor = doc.container_ancestors(ContainerId(5)).unwrap();
    let mut frames = 0;
    loop {
        frames += 1;
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        let Some((id, node)) = cursor.advance(&mut budget).unwrap() else {
            break;
        };
        println!(
            "{}: {:?}, {:?}, {:?}",
            id.0, node.kind, node.source, node.list_tightness
        );
        assert_eq!(budget.used().bytes, 0);
    }
    assert_eq!(frames, 3);
    println!("{frames} frames, 0 body bytes; parent Loose, closed child Tight, collapse retained");
    Ok(())
}
