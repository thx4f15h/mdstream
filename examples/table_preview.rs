//! Read first-row summaries while the entire table remains collapsed.
use mdtui3::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new();
    doc.append("| Name | Value |\n| - | - |\n| first | **世界** |\n")?;
    doc.append(&format!("| unread | {} |\n", "x".repeat(1_000_000)))?;
    let id = FoldId::Table(TableId(0));
    let mut view = FoldView::new(&doc, 1)?;
    view.set_collapsed(&doc, id, true)?;
    let update = doc.append("| appended | row |\n")?;
    view.apply(&doc, &update)?;
    let mut preview = TablePreview::new(
        &doc,
        TableId(0),
        TablePreviewLimits {
            rows: 2,
            source_bytes: 128,
            display_bytes: 64,
        },
    )?;
    let mut frames = 0;
    let mut bytes = 0;
    let end = 'frames: loop {
        frames += 1;
        let mut budget = Budget::new(WorkLimit {
            queries: 2,
            objects: 2,
            bytes: 24,
            ..Default::default()
        });
        loop {
            match preview.next(&doc, &mut budget)? {
                TablePreviewStep::Run { row, run } => println!("row {row}: {:?}", run.text),
                TablePreviewStep::BudgetExhausted => break,
                TablePreviewStep::Complete(end) => {
                    bytes += budget.used().bytes;
                    break 'frames end;
                }
            }
        }
        bytes += budget.used().bytes;
    };
    assert!(view.is_collapsed(id));
    assert_eq!(end, TablePreviewEnd::RowLimit);
    println!("end={end:?}, frames={frames}, charged bytes={bytes}, fold unchanged");
    Ok(())
}
