use mdtui3::*;

fn frame() -> Budget {
    Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 8,
        ..Default::default()
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 1)?;
    view.set_defaults(FoldDefaults {
        code: true,
        ..Default::default()
    });
    let update = doc.append("> # Guide\n> - ```rust\n>   let value = 1;\n")?;
    view.apply(&doc, &update)?;
    let code = FoldId::Code(BlockId(1));
    let candidate = doc.fold_candidate(code).unwrap();
    assert!(candidate.open && view.is_collapsed(code));
    assert_eq!(candidate.parent, Some(FoldId::Container(ContainerId(2))));
    let mut preview = DisplayExcerpt::new(
        &doc,
        candidate.source,
        ExcerptLimits {
            source_bytes: 128,
            display_bytes: 40,
            lines: 1,
        },
    )?;
    let mut frames = 0;
    let mut bytes = 0;
    let mut text = String::new();
    loop {
        frames += 1;
        let mut budget = frame();
        let step = preview.next(&doc, &mut budget)?;
        bytes += budget.used().bytes;
        match step {
            ExcerptStep::Run(run) => text.push_str(run.text),
            ExcerptStep::Complete(_) => break,
            ExcerptStep::BudgetExhausted => (),
        }
    }
    assert_eq!(text, "let value = 1;\n");
    println!("preview: {text:?}, {frames} frames / {bytes} charged bytes");
    let source = doc.source().find("value").unwrap();
    let mut update = doc.append(">   let value = 2;\n>   ```\n>       indented\n> - sibling\n")?;
    update.merge(doc.finish())?;
    view.apply(&doc, &update)?;
    assert!(!doc.fold_candidate(code).unwrap().open);
    let indented = FoldId::Code(BlockId(2));
    assert!(view.is_collapsed(code) && view.is_collapsed(indented));
    view.set_collapsed(&doc, FoldId::Heading(HeadingId(0)), true)?;
    view.set_collapsed(&doc, FoldId::Container(ContainerId(2)), true)?;
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
        let mut budget = frame();
        let progress = navigation.advance(&doc, &mut view, &mut budget)?;
        assert_eq!(budget.used().bytes, 0);
        if progress.completion == Completion::Covered {
            assert_eq!(progress.target.unwrap().source, Some(source));
            break;
        }
    }
    assert!(!view.is_collapsed(code));
    assert!(view.is_collapsed(indented));
    println!("revealed source {source} in {frames} frames; indented sibling stays collapsed");
    Ok(())
}
