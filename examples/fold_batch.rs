//! Progressive commands and defaults for newly discovered streaming candidates.
use mdtui3::*;

fn run(doc: &Document, view: &mut FoldView, action: FoldBatchAction) -> Result<(), LayoutError> {
    let mut task = FoldBatch::new(doc, view, action)?;
    let mut frames = 0;
    loop {
        let mut budget = Budget::new(WorkLimit {
            queries: 8,
            objects: 8,
            ..Default::default()
        });
        let progress = task.advance(doc, view, &mut budget)?;
        frames += 1;
        if progress.completion == Completion::Covered {
            println!(
                "{action:?}: frames={frames}, visited={}, changed={}",
                progress.visited, progress.changed
            );
            return Ok(());
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 1)?;
    view.set_defaults(FoldDefaults {
        heading_levels: [false, true, true, true, true, true],
        code: true,
    });
    let update = doc.append(&format!("# Guide\n{}", "## Details\nbody\n".repeat(100)))?;
    view.apply(&doc, &update)?;
    run(&doc, &mut view, FoldBatchAction::All { collapsed: false })?;

    // A one-shot expansion preserves defaults for later incoming sections.
    let update = doc.append("## New details\nnew body\n```rs\nnew code\n```\n")?;
    view.apply(&doc, &update)?;
    let update = doc.finish();
    view.apply(&doc, &update)?;
    println!(
        "Old child collapsed={}, new child collapsed={}",
        view.is_collapsed(FoldId::Heading(HeadingId(1))),
        view.is_collapsed(FoldId::Heading(HeadingId(101)))
    );
    run(
        &doc,
        &mut view,
        FoldBatchAction::Siblings {
            target: FoldId::Heading(HeadingId(101)),
            collapsed: true,
        },
    )?;
    run(
        &doc,
        &mut view,
        FoldBatchAction::AtLevel {
            level: 2,
            collapsed: true,
        },
    )?;
    run(
        &doc,
        &mut view,
        FoldBatchAction::Subtree {
            root: FoldId::Heading(HeadingId(101)),
            collapsed: false,
        },
    )?;
    run(
        &doc,
        &mut view,
        FoldBatchAction::Focus {
            target: FoldId::Heading(HeadingId(1)),
        },
    )?;
    Ok(())
}
