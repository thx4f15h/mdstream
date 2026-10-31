//! Whole-tab container prefixes retain byte mappings and shared read budgets.
use mdstream::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new();
    doc.append("> \t-\t$中\r\n")?;
    let item = doc.blocks()[0].container.unwrap();
    let fold = FoldId::Container(item);
    let mut view = FoldView::new(&doc, 1)?;
    view.set_collapsed(&doc, fold, true)?;
    let mut update = doc.append("> \t\t🙂$\n")?;
    update.merge(doc.finish())?;
    view.apply(&doc, &update)?;
    assert_eq!(doc.blocks()[1].container, Some(item));
    assert!(view.is_collapsed(fold));
    assert_eq!(doc.container_count(), 3);

    let math = doc.math(MathId(0)).unwrap();
    let mut cursor = doc.math_payload(MathId(0)).unwrap();
    let mut text = String::new();
    let mut frames = 0;
    let mut charged = 0;
    loop {
        let mut budget = Budget::new(WorkLimit {
            bytes: 4,
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        let Some(chunk) = cursor
            .advance(&mut budget)
            .expect("one UTF-8 step fits the fresh budget")
        else {
            break;
        };
        frames += 1;
        charged += budget.used().bytes;
        text.push_str(chunk.text);
        println!("{:?} -> {:?}", chunk.source, chunk.text);
    }
    assert_eq!(text, "中\r\n🙂");
    assert_eq!(charged, math.content.len());
    println!("{frames} frames / {charged} source bytes; {item:?} stays folded");
    Ok(())
}
