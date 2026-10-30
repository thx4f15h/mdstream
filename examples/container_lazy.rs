//! Omitted container prefixes retain paragraph identity and bounded consumers.
use mdtui3::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new();
    doc.append("> - > **first\r\n")?;
    let first = doc.blocks()[0].clone();
    let scope = first.container.unwrap();
    let fold = FoldId::Container(scope);
    let mut view = FoldView::new(&doc, 1)?;
    view.set_collapsed(&doc, fold, true)?;
    let mut update = doc.append("> second**\\\r\nthird\r\n# Outside\n")?;
    update.merge(doc.finish())?;
    view.apply(&doc, &update)?;
    assert_eq!(doc.blocks()[0], first);
    assert_eq!(doc.blocks()[1].container, Some(scope));
    assert_eq!(doc.blocks()[2].container, Some(scope));
    assert_eq!(doc.blocks()[3].container, None);
    assert_eq!(doc.container_count(), 4);
    assert!(view.is_collapsed(fold));
    let mut cursor = doc
        .display_cursor(Span::new(0, doc.source().len()))
        .unwrap();
    let mut text = String::new();
    let mut frames = 0;
    let mut charged = 0;
    'frames: loop {
        frames += 1;
        let mut budget = Budget::new(WorkLimit {
            queries: 2,
            objects: 2,
            bytes: 24,
            ..Default::default()
        });
        loop {
            match cursor.next(&doc, &mut budget)? {
                DisplayStep::Run(run) => {
                    if !run.style.contains(Style::HIDDEN) {
                        text.push_str(run.text);
                    }
                }
                DisplayStep::BudgetExhausted => break,
                DisplayStep::Complete => {
                    charged += budget.used().bytes;
                    break 'frames;
                }
            }
        }
        charged += budget.used().bytes;
    }
    assert_eq!(text, "first\nsecond\nthird\nOutside\n");
    println!(
        "{text}{frames} frames / {charged} charged bytes; 4 containers, lazy scope and fold retained"
    );
    Ok(())
}
