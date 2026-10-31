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
    let mut other = FoldView::new(&doc, 1)?;
    let completed = ContainerFoldFilter {
        completed_tasks: true,
        ..Default::default()
    };
    view.set_defaults(FoldDefaults {
        containers: completed,
        ..Default::default()
    });
    let update = doc.append("- [x] finished\n- [ ] review\n")?;
    view.apply(&doc, &update)?;
    other.apply(&doc, &update)?;
    let finished = FoldId::Container(ContainerId(1));
    let review = FoldId::Container(ContainerId(2));
    assert!(view.is_collapsed(finished));
    assert!(!view.is_collapsed(review));
    assert!(!other.is_collapsed(finished));

    view.set_preview(&doc, Some(review), &mut frame())?;
    view.set_task_checked(&doc, ContainerId(2), true)?;
    assert_eq!(other.task_checked(&doc, ContainerId(2))?, Some(false));
    assert!(!view.is_collapsed(review)); // State changes preserve fold intent.
    let mut batch = FoldBatch::new(
        &doc,
        &view,
        FoldBatchAction::Containers {
            filter: completed,
            collapsed: true,
        },
    )?;
    let mut frames = 0;
    loop {
        frames += 1;
        let mut budget = frame();
        let progress = batch.advance(&doc, &mut view, &mut budget)?;
        assert_eq!(budget.used().bytes, 0);
        if progress.completion == Completion::Covered {
            assert_eq!(progress.visited, 3);
            assert_eq!(progress.changed, 1);
            break;
        }
    }
    assert!(view.is_collapsed(review)); // Saved choice, still temporarily visible.
    assert_eq!(view.preview(), Some(review));
    view.set_preview(&doc, None, &mut frame())?;

    let target = doc.source().find("review").unwrap();
    let mut nav = FoldNavigation::new(
        &doc,
        &view,
        &Navigator::new(),
        NavAction::Source(target),
        FoldNavigationPolicy::Reveal,
        10,
    )?;
    let mut nav_frames = 0;
    loop {
        nav_frames += 1;
        let progress = nav.advance(&doc, &mut view, &mut frame())?;
        if progress.completion == Completion::Covered {
            assert_eq!(progress.target.unwrap().source, Some(target));
            break;
        }
    }
    let mut update = doc.append("- [x] newly finished\n")?;
    update.merge(doc.finish())?;
    view.apply(&doc, &update)?;
    other.apply(&doc, &update)?;
    assert!(!view.is_collapsed(review)); // Explicit navigation wins over defaults.
    assert!(view.is_collapsed(FoldId::Container(ContainerId(3))));
    assert!(doc.source().contains("[ ] review"));
    println!(
        "filtered 3 containers in {frames} frames / 0 source bytes; navigation: {nav_frames} frames"
    );
    println!("task override stays in one view; new completed item still starts collapsed");
    Ok(())
}
