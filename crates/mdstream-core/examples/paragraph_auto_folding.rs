//! Apply a source-byte policy only to changed paragraphs, sharing each frame's budget.
use mdstream::*;

fn frame() -> Budget {
    Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        ..Default::default()
    })
}
fn apply(doc: &Document, view: &mut FoldView, update: &Update) -> Result<usize, LayoutError> {
    view.apply(doc, update)?;
    let mut task = ParagraphAutoFold::for_update(doc, view, update)?;
    let mut frames = 0;
    loop {
        frames += 1;
        let mut budget = frame();
        let progress = task.advance(doc, view, &mut budget)?;
        assert_eq!(budget.used().bytes, 0);
        if progress.completion == Completion::Covered {
            return Ok(frames);
        }
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 1)?;
    view.set_defaults(FoldDefaults {
        paragraphs: Some(ParagraphFoldPolicy {
            min_bytes: 1024,
            preview_bytes: 9,
        }),
        ..Default::default()
    });
    let update = doc.append("> preview\n")?;
    apply(&doc, &mut view, &update)?;
    assert_eq!(view.source_fold_count(), 0);
    let update = doc.append(&format!("> {}", "x".repeat(1_000_000)))?;
    let frames = apply(&doc, &mut view, &update)?;
    let candidate = view.source_fold_at(&doc, 0)?.unwrap();
    assert!(view.is_collapsed(candidate.id));
    view.set_collapsed(&doc, candidate.id, false)?;
    let update = doc.append(" tail\n>\n> another short paragraph\n")?;
    apply(&doc, &mut view, &update)?;
    assert_eq!(view.source_fold_at(&doc, 0)?.unwrap().id, candidate.id);
    assert!(!view.is_collapsed(candidate.id));
    println!(
        "automatic registration: {frames} frames, zero body reads; explicit expansion survives closure; {} registration",
        view.source_fold_count()
    );
    Ok(())
}
