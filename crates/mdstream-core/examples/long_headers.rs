//! Delayed header semantics and a bounded preview of a million-byte header.
use mdstream::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new();
    doc.append("> - |**name**|")?;
    for _ in 0..1000 {
        doc.append(&"x".repeat(1000))?;
        assert!(doc.pending().len() <= BLOCK_LOOKAHEAD + 3);
        assert_eq!(doc.work().inline_bytes, 0);
    }
    doc.append("\r\n")?;
    assert!(doc.tables().is_empty());
    let update = doc.append(">   |:--|--:|\n>   one|two\n")?;
    assert_eq!(update.new_tables, 0..1);
    doc.finish();
    let mut preview = TablePreview::new(
        &doc,
        TableId(0),
        TablePreviewLimits {
            rows: 1,
            source_bytes: 64,
            display_bytes: 64,
        },
    )?;
    let mut frames = 0;
    let mut charged = 0;
    let mut text = String::new();
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
            TablePreviewStep::Run { run, .. } => text.push_str(run.text),
            TablePreviewStep::BudgetExhausted => {}
            TablePreviewStep::Complete(end) => {
                assert_eq!(end, TablePreviewEnd::SourceLimit);
                assert!(charged < 128);
                break;
            }
        }
    }
    println!(
        "header summary bytes={}, preview frames={frames}, charged bytes={charged}, text={text:?}",
        doc.work().table_header_bytes
    );
    Ok(())
}
