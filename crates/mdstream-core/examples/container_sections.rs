use mdstream::*;

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
        heading_levels: [true; 6],
        table: true,
        ..Default::default()
    });
    let update = doc.append("> # Guide\n> - ## Item\n>   a|b\n>   -|-\n>   first|row\n")?;
    view.apply(&doc, &update)?;
    let guide = FoldId::Heading(HeadingId(0));
    let item = FoldId::Heading(HeadingId(1));
    let table = FoldId::Table(TableId(0));
    assert_eq!(doc.fold_candidate(table).unwrap().parent, Some(item));
    assert_eq!(
        doc.fold_candidate(FoldId::Container(ContainerId(1)))
            .unwrap()
            .parent,
        Some(guide)
    );
    let before = view.range_visits();
    let update = doc.append(">   next|row\n")?;
    view.apply(&doc, &update)?;
    println!(
        "append: {} range visits (shared section tail + table)",
        view.range_visits() - before
    );
    let source = doc.source().find("next").unwrap();
    view.set_preview(&doc, Some(guide), &mut frame())?;
    assert!(
        matches!(view.hidden_at(&doc, source, &mut frame())?, FoldLookup::Hidden(c) if c.id == item)
    );
    view.set_preview(&doc, None, &mut frame())?;
    let mut update = doc.append("> - ## Sibling\n>   retained\nout\n")?;
    update.merge(doc.finish())?;
    view.apply(&doc, &update)?;
    assert!(!doc.fold_candidate(item).unwrap().open);
    let mut task = FoldNavigation::new(
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
        let mut budget = frame();
        let progress = task.advance(&doc, &mut view, &mut budget)?;
        assert_eq!(budget.used().bytes, 0);
        if progress.completion == Completion::Covered {
            assert_eq!(progress.target.unwrap().source, Some(source));
            break;
        }
    }
    assert!(view.is_collapsed(FoldId::Heading(HeadingId(2))));
    println!(
        "revealed guide/item/table in {frames} frames; source {source}; sibling remains collapsed"
    );
    Ok(())
}
