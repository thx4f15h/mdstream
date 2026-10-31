//! Partially consumed tabs retain source mapping across streamed inline closure.
use mdstream::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new();
    doc.append("- ` 中\n\t🙂\n")?;
    let id = FoldId::Container(doc.blocks()[0].container.unwrap());
    let mut view = FoldView::new(&doc, 1)?;
    view.set_collapsed(&doc, id, true)?;
    let mut update = doc.append("\t` &amp; [a\n\tb](u)\n")?;
    update.merge(doc.finish())?;
    view.apply(&doc, &update)?;
    let mut cursor = doc
        .display_cursor(Span::new(0, doc.source().len()))
        .unwrap();
    let mut output = String::new();
    let mut frames = 0;
    let mut charged = 0;
    loop {
        let mut budget = Budget::new(WorkLimit {
            bytes: 6,
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        match cursor.next(&doc, &mut budget)? {
            DisplayStep::Run(run) => {
                frames += 1;
                charged += budget.used().bytes;
                if !run.style.contains(Style::HIDDEN) {
                    println!(
                        "{:?} -> {:?} (replacement {:?})",
                        run.source, run.text, run.replacement
                    );
                    output.push_str(run.text);
                }
            }
            DisplayStep::Complete => break,
            DisplayStep::BudgetExhausted => panic!("one UTF-8 step fits"),
        }
    }
    assert_eq!(output, "中   🙂   & a\n  b\n");
    assert!(view.is_collapsed(id));
    assert_eq!(doc.links().len(), 1);
    println!("{frames} frames / {charged} charged bytes; item stays folded");
    Ok(())
}
