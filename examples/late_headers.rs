//! Keep streaming first-cell semantics, then confirm a late-pipe header.
use mdtui3::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new();
    doc.append("> - **name** [link](url) ")?;
    for _ in 0..1000 {
        doc.append(&"x".repeat(1000))?;
    }
    assert_eq!(doc.links().len(), 1);
    assert_eq!(doc.work().inline_bytes, 1_000_021);
    let original = *doc.link_info(LinkId(0)).unwrap();
    doc.append("|**second** [next](target)\r\n")?;
    assert_eq!(doc.links().len(), 1);
    let update = doc.append(">   :--|--:\n>   one|two\n")?;
    assert_eq!(update.new_tables, 0..1);
    assert_eq!(update.new_links, 1..2);
    assert_eq!(*doc.link_info(LinkId(0)).unwrap(), original);
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
        "stable first link, summary bytes={}, preview frames={frames}, charged bytes={charged}, text={text:?}",
        doc.work().table_header_bytes
    );
    Ok(())
}
