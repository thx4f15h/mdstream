use mdtui3::{Budget, Completion, Document, HeadingAnchors, HeadingId, WorkLimit};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new();
    for chunk in [
        "# Guide\n",
        "> ## Topic\n",
        "> - Detail\n",
        ">   ===\n",
        "> ## Next\n",
        "outside\n",
    ] {
        let update = doc.append(chunk)?;
        println!(
            "headings={:?}, closed containers={:?}",
            update.new_headings, update.closed_containers
        );
    }
    doc.finish();
    let index = doc.headings();
    for (i, heading) in index.entries().iter().enumerate() {
        let id = HeadingId(i);
        println!(
            "{id:?}: scope={:?}, parent={:?}, section={:?}",
            heading.container,
            index.parent(id),
            index.section(id, doc.source().len())
        );
    }
    assert_eq!(index.parent(HeadingId(2)), None);
    assert_eq!(index.next_sibling(HeadingId(1)), Some(HeadingId(3)));
    assert_eq!(
        index.current(doc.source().find("outside").unwrap()),
        Some(HeadingId(0))
    );
    let mut anchors = HeadingAnchors::new(&doc);
    let mut frames = 0;
    loop {
        frames += 1;
        let mut budget = Budget::new(WorkLimit {
            bytes: 6,
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        if anchors.advance(&doc, &mut budget)? == Completion::EndOfDocument {
            break;
        }
    }
    for i in 0..index.entries().len() {
        println!(
            "#{}, source={}",
            anchors.get(&doc, HeadingId(i))?.unwrap().slug,
            index.entries()[i].source.start
        );
    }
    println!(
        "{} headings, {frames} budgeted anchor frames",
        index.entries().len()
    );
    Ok(())
}
