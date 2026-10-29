use mdtui3::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new();
    doc.append("# Guide\n## a**b**&amp;c\nbody-only\n## ` ab&c `\n### ab&c\n")?;
    doc.finish();
    let mut view = FoldView::new(&doc, 1)?;
    view.set_collapsed(&doc, FoldId::Heading(HeadingId(0)), true)?;

    let mut search = HeadingSearch::new(&doc, "ab&c", 2)?;
    let mut matches = Vec::new();
    let mut first = None;
    let mut frames = 0;
    loop {
        frames += 1;
        let mut budget = Budget::new(WorkLimit {
            bytes: 16,
            queries: 2,
            objects: 2,
            ..Default::default()
        });
        let completion = search.advance(&doc, &mut budget, &mut matches)?;
        for hit in matches.drain(..) {
            first.get_or_insert(hit.source.start);
            println!(
                "{:?}: title bytes {:?}, source {:?} = {:?}",
                hit.heading,
                hit.display,
                hit.source.range(),
                doc.text(hit.source).unwrap()
            );
        }
        if completion == Completion::EndOfDocument {
            break;
        }
    }
    println!("searched revision {} in {frames} frames", search.revision());
    assert!(view.is_collapsed(FoldId::Heading(HeadingId(0))));

    if let Some(source) = first {
        let mut navigation = FoldNavigation::new(
            &doc,
            &view,
            &Navigator::new(),
            NavAction::Source(source),
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
                println!(
                    "revealed first result at source {:?}",
                    progress.target.unwrap().source
                );
                break;
            }
        }
    }
    Ok(())
}
