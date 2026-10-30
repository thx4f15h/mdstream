use mdtui3::*;

fn frame() -> Budget {
    Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        ..Default::default()
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 1)?;
    view.set_defaults(FoldDefaults {
        container_depth: Some(2), // Root List = 1, Item = 2, nested Quote = 3.
        ..Default::default()
    });
    let first = FoldId::Container(ContainerId(1));
    let child = FoldId::Container(ContainerId(2));
    let second = FoldId::Container(ContainerId(3));
    let update = doc.append("- first\n")?;
    view.apply(&doc, &update)?;
    assert!(view.is_collapsed(first));
    view.set_preview(&doc, Some(first), &mut frame())?;
    let update = doc.append("  > child\n")?;
    view.apply(&doc, &update)?;
    assert!(view.is_collapsed(child));
    let source = doc.source().find("child").unwrap();
    assert!(
        matches!(view.hidden_at(&doc, source, &mut frame())?, FoldLookup::Hidden(c) if c.id == child)
    );
    view.set_collapsed(&doc, first, false)?; // Explicit choice also wins during preview.
    view.set_preview(&doc, None, &mut frame())?;
    let update = doc.append("- second\n")?;
    view.apply(&doc, &update)?;
    assert!(!view.is_collapsed(first));
    assert!(view.is_collapsed(second));

    let mut task = FoldBatch::new(&doc, &view, FoldBatchAction::All { collapsed: false })?;
    let mut frames = 0;
    loop {
        frames += 1;
        if task.advance(&doc, &mut view, &mut frame())?.completion == Completion::Covered {
            break;
        }
    }
    let mut update = doc.append("- third\n")?;
    update.merge(doc.finish())?;
    view.apply(&doc, &update)?;
    assert!(!view.is_collapsed(second));
    assert!(view.is_collapsed(FoldId::Container(ContainerId(4))));
    println!(
        "expanded existing nodes in {frames} budgeted frames; new item still starts collapsed"
    );
    println!(
        "initialized {} container metadata entries",
        view.default_visits()
    );
    Ok(())
}
