//! Stream explicit continuations and read their display text with one frame budget.
use mdstream::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new();
    doc.append("> - **first\n")?;
    let first = doc.blocks()[0].clone();
    let key = doc.tile_key(TileId(0)).unwrap();
    let item = FoldId::Container(first.container.unwrap());
    let mut view = FoldView::new(&doc, 1)?;
    view.set_collapsed(&doc, item, true)?;
    let mut update = doc.append(">   second**\\\r\n>   ` 中\r\n>   🙂\r\n>   `\n> - sibling\n")?;
    update.merge(doc.finish())?;
    view.apply(&doc, &update)?;
    assert_eq!(doc.blocks()[0], first);
    assert_ne!(doc.tile_key(TileId(0)).unwrap(), key);
    assert!(view.is_collapsed(item));
    let span = Span::new(0, doc.source().len());
    let mut reader = doc.display_cursor(span).unwrap();
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
            match reader.next(&doc, &mut budget)? {
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
    assert_eq!(text, "first\nsecond\n中 🙂\nsibling\n");
    assert_eq!(doc.blocks().len(), 6);
    println!(
        "{text}{frames} frames / {charged} charged bytes; 6 physical leaves, item fold retained"
    );
    Ok(())
}
