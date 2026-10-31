use mdstream::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new();
    doc.append("> - first\n>   \n")?;
    let list = ContainerId(1);
    assert_eq!(
        doc.container(list).unwrap().list_tightness,
        Some(ListTightness::Tight)
    );
    // A trailing blank is tentative. The next sibling confirms separation.
    let update = doc.append("> - second\n>   - nested\n>     \n>     paragraph\n")?;
    assert_eq!(update.loosened_lists, [list, ContainerId(4)]);
    for &id in &update.loosened_lists {
        let node = doc.container(id).unwrap();
        println!(
            "list {}: {:?}, invalidate {:?}",
            id.0,
            node.list_tightness.unwrap(),
            node.source
        );
    }
    let eof = doc.finish();
    assert!(eof.loosened_lists.is_empty());
    let item = doc.blocks().last().unwrap().container.unwrap();
    let mut cursor = doc.container_ancestors(item).unwrap();
    let mut frames = 0;
    let mut list_count = 0;
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
        if let Some(tightness) = node.list_tightness {
            assert_eq!(node.state, SyntaxState::Sealed);
            println!("ancestor list {}: {tightness:?} (final)", id.0);
            list_count += 1;
        }
        assert!(budget.used().queries <= 1);
        assert!(budget.used().objects <= 1);
    }
    assert_eq!(list_count, 2);
    assert_eq!(frames, 6); // five containers, then exhaustion of the path
    println!("{frames} frames; {list_count} lists; no descendant scan");
    Ok(())
}
