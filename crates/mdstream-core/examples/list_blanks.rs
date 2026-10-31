use mdstream::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new();
    doc.append("- first\n  - child\n")?;
    let list = ContainerId(0);
    let nested = ContainerId(2);
    let mut view = FoldView::new(&doc, 1)?;
    let fold = FoldId::Container(list);
    view.set_collapsed(&doc, fold, true)?;
    let blank = doc.append("\n\n")?;
    assert!(blank.closed_containers.is_empty());
    assert!(blank.loosened_lists.is_empty());
    view.apply(&doc, &blank)?;
    let continued = doc.append("    second paragraph\n")?;
    assert_eq!(continued.loosened_lists, [nested]);
    view.apply(&doc, &continued)?;
    let sibling = doc.append("\n- sibling\n\n")?;
    assert_eq!(sibling.loosened_lists, [list]);
    assert_eq!(sibling.new_containers, 4..5);
    view.apply(&doc, &sibling)?;
    let end = doc.finish();
    assert!(end.loosened_lists.is_empty());
    view.apply(&doc, &end)?;
    assert!(view.is_collapsed(fold));
    assert_eq!(doc.container(list).unwrap().source.end, doc.source().len());

    let mut ancestors = doc.container_ancestors(ContainerId(4)).unwrap();
    let mut frames = 0;
    loop {
        frames += 1;
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        let Some((id, node)) = ancestors.advance(&mut budget).unwrap() else {
            break;
        };
        println!(
            "{}: {:?}, {:?}, {:?}",
            id.0, node.kind, node.source, node.list_tightness
        );
        assert_eq!(budget.used().bytes, 0);
    }
    assert_eq!(frames, 3);
    println!("{frames} frames, 0 body bytes; list identity and collapsed choice retained");
    Ok(())
}
