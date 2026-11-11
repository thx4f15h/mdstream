//! Keep a stable root cut while the document streams, then seal the same ID.
use mdstream::*;

fn complete(
    doc: &Document,
    view: &mut FoldView,
    mut task: SourceFoldRegistration,
) -> Result<FoldId, LayoutError> {
    loop {
        let mut frame = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        if let Some(id) = task.advance(doc, view, &mut frame)?.id {
            return Ok(id);
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new();
    doc.append("lead text\n\n")?;
    let mut view = FoldView::new(&doc, 2)?;
    let task = SourceFoldRegistration::tail(&doc, &view, 5)?;
    let id = complete(&doc, &mut view, task)?;
    view.set_collapsed(&doc, id, true)?;
    let old_eof = FoldAnchor::new(&doc, doc.source().len())?;
    let update = doc.append("```rs\nhidden code\n```\n\n")?;
    view.apply(&doc, &update)?;
    let candidate = view.fold_candidate(&doc, id)?.unwrap();
    assert_eq!(candidate.hidden, Span::new(5, doc.source().len()));
    let mut frame = Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        ..Default::default()
    });
    assert_eq!(
        old_eof
            .resolve(&doc, &view, 10, &mut frame)?
            .unwrap()
            .hidden_by,
        Some(id)
    );
    let FoldId::Source(source_id) = id else {
        unreachable!()
    };
    let task = SourceFoldRegistration::seal_tail(&doc, &view, source_id)?;
    assert_eq!(complete(&doc, &mut view, task)?, id);
    let update = doc.append("visible after seal\n")?;
    view.apply(&doc, &update)?;
    let sealed = view.fold_candidate(&doc, id)?.unwrap();
    assert_eq!(sealed.hidden, candidate.hidden);
    assert!(!sealed.open);
    println!("same ID: {id:?}; sealed source: {:?}", sealed.source);
    Ok(())
}
