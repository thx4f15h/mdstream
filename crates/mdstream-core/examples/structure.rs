use mdstream::{BlockKind, Document, HeadingId, HeightIndex, NavAction, Navigator, Span};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new();
    let mut heights = HeightIndex::default();
    let source = "# Tasks\n\n- [ ] implement renderer\n- [x] index headings\n\n## Measurements\n\n| Name | Value | State |\n| :--- | ---: | :---: |\n| CPU | **42** | ready |\n\n![diagram](diagram.png)\n";
    let update = doc.append(source)?;
    heights.apply(&doc, &update)?;
    let update = doc.finish();
    heights.apply(&doc, &update)?;
    let mut navigator = Navigator::new();
    let target = navigator
        .navigate(&doc, &heights, NavAction::Heading(HeadingId(1)), 24)
        .unwrap();
    println!("heading jump: {target:?}");
    for heading in doc.headings().entries() {
        println!(
            "h{} {:?}",
            heading.level,
            doc.text(heading.content).unwrap()
        );
    }
    for block in doc.blocks() {
        if let BlockKind::ListItem {
            task: Some(task), ..
        } = block.kind
        {
            println!("task checked={} marker={:?}", task.checked, task.source);
        }
    }
    for table in doc.tables() {
        println!("columns: {:?}", table.columns);
        for row in table.rows().iter().take(2) {
            println!(
                "row: {:?}",
                row.cells
                    .iter()
                    .map(|cell| doc.text(cell.source).unwrap())
                    .collect::<Vec<_>>()
            );
        }
    }
    for (id, info) in doc.links_in(Span::new(0, doc.source().len())) {
        println!(
            "{:?}: {:?} -> {:?}",
            info.kind,
            doc.text(info.label),
            doc.link_destination(id)
        );
    }
    Ok(())
}
