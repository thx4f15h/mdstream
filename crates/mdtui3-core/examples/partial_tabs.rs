//! A partial container tab becomes mapped, budgeted spaces in fenced code.
use mdtui3::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new();
    doc.append("- ```rust\n\t中🙂\n")?;
    let id = FoldId::Code(BlockId(0));
    let mut view = FoldView::new(&doc, 1)?;
    view.set_collapsed(&doc, id, true)?;
    let mut update = doc.append("\t```\n")?;
    update.merge(doc.finish())?;
    view.apply(&doc, &update)?;
    assert_eq!(doc.blocks().len(), 1);
    let mut cursor = doc.display_cursor(doc.blocks()[0].source).unwrap();
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
    assert_eq!(output, "  中🙂\n");
    assert!(view.is_collapsed(id));
    println!("{frames} frames / {charged} charged bytes; code stays folded");
    Ok(())
}
