use mdstream::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 1)?;
    view.set_defaults(FoldDefaults {
        table: true,
        ..Default::default()
    });
    for chunk in [
        "# Guide\n> ## Data\n> - Name | Count\n",
        ">   :-- | --:\n",
        ">   **中🙂** | 12\n",
        "> - sibling\n",
    ] {
        let update = doc.append(chunk)?;
        view.apply(&doc, &update)?;
        println!(
            "new tables={:?}, changed rows={:?}",
            update.new_tables, update.rows_changed
        );
    }
    let update = doc.finish();
    view.apply(&doc, &update)?;
    let id = TableId(0);
    let table = doc.table(id).unwrap();
    let scope = doc.block(table.block).unwrap().container.unwrap();
    assert_eq!(doc.container(scope).unwrap().depth, 3);
    assert_eq!(table.source.end, doc.source().find("> - sibling").unwrap());
    // The table is innermost even at the shared table/item opener.
    let candidate = doc.fold_candidate_at(table.source.start).unwrap();
    assert_eq!(candidate.id, FoldId::Table(id));
    // "Data" belongs to the quote scope; this item has no local heading.
    assert_eq!(candidate.parent, Some(FoldId::Container(scope)));
    let mut preview = TablePreview::new(
        &doc,
        id,
        TablePreviewLimits {
            rows: 2,
            source_bytes: 128,
            display_bytes: 64,
        },
    )?;
    let mut text = String::new();
    let mut frames = 0;
    let mut bytes = 0;
    'frames: loop {
        frames += 1;
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 6,
            ..Default::default()
        });
        loop {
            match preview.next(&doc, &mut budget)? {
                TablePreviewStep::Run { run, .. } => text.push_str(run.text),
                TablePreviewStep::BudgetExhausted => break,
                TablePreviewStep::Complete(end) => {
                    assert_eq!(end, TablePreviewEnd::RowsEnd);
                    bytes += budget.used().bytes;
                    break 'frames;
                }
            }
        }
        bytes += budget.used().bytes;
    }
    assert_eq!(text, "NameCount\n中🙂12\n");
    assert!(view.is_collapsed(FoldId::Table(id)));
    println!(
        "scope={scope:?}, source={:?}, rows={}",
        table.source,
        table.rows().len()
    );
    println!("{frames} frames, {bytes} charged bytes, folded preview: {text:?}");
    Ok(())
}
