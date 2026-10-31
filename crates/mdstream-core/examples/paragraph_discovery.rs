//! Discover a paragraph from a window inside its body, then fold its live tail.
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
    doc.append("earlier paragraph\n\n> preview\n")?;
    doc.append(&format!("> {}", "x".repeat(1_000_000)))?;
    let mut view = FoldView::new(&doc, 1)?;
    let mut discovery = ParagraphFoldCandidates::new(&doc, Span::new(500_000, 500_001), 1000)?;
    let mut budget = frame();
    let mut out = Vec::new();
    assert_eq!(
        discovery.advance(&doc, &mut budget, &mut out)?,
        Completion::Covered
    );
    let candidate = out[0];
    assert!(candidate.source.start < 500_000 && candidate.open);
    assert_eq!(budget.used().bytes, 0);
    let mut task = if candidate.container.is_some() {
        SourceFoldRegistration::container_paragraph(&doc, &view, candidate.block, 9)?
    } else if candidate.open {
        SourceFoldRegistration::open_paragraph(&doc, &view, candidate.block, 9)?
    } else {
        SourceFoldRegistration::paragraph(&doc, &view, candidate.block, 9)?
    };
    // Discovery used this frame. Registration must resume on a later budget.
    assert_eq!(
        task.advance(&doc, &mut view, &mut budget)?.completion,
        Completion::BudgetExhausted
    );
    let mut frames = 1;
    let id = loop {
        frames += 1;
        if let Some(id) = task.advance(&doc, &mut view, &mut frame())?.id {
            break id;
        }
    };
    view.set_collapsed(&doc, id, true)?;
    let start = doc.source().len();
    let update = doc.append("tail")?;
    view.apply(&doc, &update)?;
    // New revision: re-query just the appended window, finding the same first leaf.
    let mut discovery =
        ParagraphFoldCandidates::new(&doc, Span::new(start, doc.source().len()), 1000)?;
    discovery.advance(&doc, &mut frame(), &mut out)?;
    assert_eq!(out[0].block, candidate.block);
    assert_eq!(
        view.fold_candidate(&doc, id)?.unwrap().source,
        out[0].source
    );
    println!(
        "discovery: 1 query/object, 0 body bytes; registration: {frames} total frames; stable {:?}",
        candidate.block
    );
    Ok(())
}
