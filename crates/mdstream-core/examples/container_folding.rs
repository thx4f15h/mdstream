use mdstream::*;

fn frame() -> Budget {
    Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 16,
        ..Default::default()
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new();
    doc.append("# Guide\n> - a|b\n>   -|-\n>   first|row\n")?;
    let mut view = FoldView::new(&doc, 1)?;
    let quote = FoldId::Container(ContainerId(0));
    let item = FoldId::Container(ContainerId(2));
    let table = FoldId::Table(TableId(0));
    for id in [quote, item, table] {
        view.set_collapsed(&doc, id, true)?;
    }
    let before = view.range_visits();
    let update = doc.append(">   next|row\n")?;
    view.apply(&doc, &update)?;
    println!(
        "append: {} range visits (one shared container tail + one table)",
        view.range_visits() - before
    );
    view.set_preview(&doc, Some(quote), &mut frame())?;
    let source = doc.source().find("next").unwrap();
    assert!(
        matches!(view.hidden_at(&doc, source, &mut frame())?, FoldLookup::Hidden(c) if c.id == item)
    );
    let mut update = doc.append("> - sibling\n# End\n")?;
    update.merge(doc.finish())?;
    view.apply(&doc, &update)?;
    view.set_preview(&doc, None, &mut frame())?;
    let mut budget = frame();
    let candidate = doc.fold_candidate(quote).unwrap();
    println!(
        "quote summary: {:?}",
        candidate.summary_text(&doc, 16, &mut budget)
    );
    let anchor = FoldAnchor::new(&doc, source)?;
    assert_eq!(
        anchor
            .resolve(&doc, &view, 10, &mut frame())?
            .unwrap()
            .hidden_by,
        Some(quote)
    );
    let mut navigation = FoldNavigation::new(
        &doc,
        &view,
        &Navigator::new(),
        NavAction::Source(source),
        FoldNavigationPolicy::Reveal,
        10,
    )?;
    let mut frames = 0;
    loop {
        frames += 1;
        let progress = navigation.advance(&doc, &mut view, &mut frame())?;
        if progress.completion == Completion::Covered {
            assert_eq!(progress.target.unwrap().source, Some(source));
            break;
        }
    }
    assert_eq!(
        anchor
            .resolve(&doc, &view, 10, &mut frame())?
            .unwrap()
            .hidden_by,
        None
    );
    println!("revealed quote/item/table in {frames} budgeted frames; source {source}");
    Ok(())
}
