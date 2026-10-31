//! Streaming recognition and bounded display consumption of a long separator.
use mdtui3::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new();
    doc.append("> - Name|Value\n>   |")?;
    for _ in 0..1000 {
        doc.append(&"-".repeat(1000))?;
        assert!(doc.pending().len() <= BLOCK_LOOKAHEAD + 3);
    }
    assert!(doc.tables().is_empty());
    let before = doc.work();
    let update = doc.append("|:--:|\n>   one|**中**\n")?;
    assert_eq!(update.new_tables, 0..1);
    assert!(doc.work().inline_bytes - before.inline_bytes < 40);
    doc.finish();
    let mut preview = TablePreview::new(
        &doc,
        TableId(0),
        TablePreviewLimits {
            rows: 2,
            source_bytes: 64,
            display_bytes: 64,
        },
    )?;
    let mut frames = 0;
    let mut charged = 0;
    loop {
        frames += 1;
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 8,
            ..Default::default()
        });
        let step = preview.next(&doc, &mut budget)?;
        charged += budget.used().bytes;
        match step {
            TablePreviewStep::Run { row, run } => println!("row {row}: {:?}", run.text),
            TablePreviewStep::BudgetExhausted => {}
            TablePreviewStep::Complete(end) => {
                assert_eq!(end, TablePreviewEnd::RowsEnd);
                // Paid read windows can overlap across frames.
                assert!(charged <= 128);
                break;
            }
        }
    }
    println!(
        "separator probes={}, preview frames={frames}, charged bytes={charged}",
        doc.work().table_candidate_bytes
    );
    Ok(())
}
