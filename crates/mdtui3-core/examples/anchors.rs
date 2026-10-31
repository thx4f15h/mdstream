use mdtui3::*;

fn index(doc: &Document) -> Result<HeadingAnchors, LayoutError> {
    let mut index = HeadingAnchors::new(doc);
    let mut frames = 0;
    let mut bytes = 0;
    loop {
        frames += 1;
        let mut budget = Budget::new(WorkLimit {
            bytes: 32,
            queries: 4,
            objects: 16,
            ..Default::default()
        });
        let completion = index.advance(doc, &mut budget)?;
        bytes += budget.used().bytes;
        if completion == Completion::EndOfDocument {
            println!(
                "revision {}: {frames} frames, {bytes} charged bytes",
                index.revision()
            );
            return Ok(index);
        }
    }
}

fn resolve(
    doc: &Document,
    index: &HeadingAnchors,
    fragment: &str,
) -> Result<FragmentStep, LayoutError> {
    let mut lookup = index.fragment(fragment)?;
    loop {
        let mut budget = Budget::new(WorkLimit {
            bytes: 6,
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        let result = lookup.advance(doc, &mut budget)?;
        if result != FragmentStep::BudgetExhausted {
            println!("{fragment}: {result:?}");
            return Ok(result);
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new();
    doc.append("# Guide\n## Hello\n## Hello-1\n## Hello\n")?;
    let pending = index(&doc)?;
    assert_eq!(resolve(&doc, &pending, "#中文")?, FragmentStep::Unresolved);
    doc.append("## 中文\nbody\n## Other\n")?;
    doc.finish();
    assert_eq!(
        pending.get(&doc, HeadingId(0)),
        Err(LayoutError::StaleContinuation)
    );
    let anchors = index(&doc)?;
    for id in 0..anchors.indexed_headings() {
        let anchor = anchors.get(&doc, HeadingId(id))?.unwrap();
        println!(
            "{:?}: #{} at {}",
            anchor.target.heading, anchor.slug, anchor.target.source
        );
    }
    assert!(matches!(
        resolve(&doc, &anchors, "#hello-2")?,
        FragmentStep::Resolved(_)
    ));
    let FragmentStep::Resolved(target) = resolve(&doc, &anchors, "#%E4%B8%AD%E6%96%87")? else {
        panic!("late target was not found");
    };
    let mut view = FoldView::new(&doc, 1)?;
    view.set_collapsed(&doc, FoldId::Heading(HeadingId(0)), true)?;
    view.set_collapsed(&doc, FoldId::Heading(HeadingId(5)), true)?;
    let mut navigation = FoldNavigation::new(
        &doc,
        &view,
        &Navigator::new(),
        NavAction::Source(target.source),
        FoldNavigationPolicy::Reveal,
        24,
    )?;
    loop {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        let progress = navigation.advance(&doc, &mut view, &mut budget)?;
        if progress.completion == Completion::Covered {
            assert_eq!(progress.target.unwrap().source, Some(target.source));
            break;
        }
    }
    assert!(view.is_collapsed(FoldId::Heading(HeadingId(5))));
    println!(
        "revealed source {} and preserved the unrelated fold",
        target.source
    );
    Ok(())
}
