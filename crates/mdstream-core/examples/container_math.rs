//! Read a multiline formula with source mapping and the shared frame budget.
use mdstream::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new();
    doc.append("> - $$中 + \\alpha\r\n")?;
    let first = doc.blocks()[0].clone();
    let fold = FoldId::Container(first.container.unwrap());
    let mut view = FoldView::new(&doc, 1)?;
    view.set_collapsed(&doc, fold, true)?;
    let mut update = doc.append(">   🙂 + &amp;$$\n")?;
    update.merge(doc.finish())?;
    view.apply(&doc, &update)?;
    assert_eq!(doc.blocks()[0], first);
    assert!(view.is_collapsed(fold));
    assert_eq!(update.new_maths, 0..1);

    let id = MathId(0);
    let math = *doc.math(id).unwrap();
    assert_eq!(
        doc.text(math.content),
        Some("中 + \\alpha\r\n>   🙂 + &amp;")
    );
    let mut cursor = doc.math_payload(id).unwrap();
    let mut text = String::new();
    let mut frames = 0;
    let mut bytes = 0;
    'frames: loop {
        frames += 1;
        let mut budget = Budget::new(WorkLimit {
            queries: 2,
            objects: 2,
            bytes: 8,
            ..Default::default()
        });
        loop {
            match cursor.advance(&mut budget) {
                Ok(Some(chunk)) => {
                    println!("{:?} -> {:?}", chunk.source, chunk.text);
                    text.push_str(chunk.text);
                }
                Ok(None) => {
                    bytes += budget.used().bytes;
                    break 'frames;
                }
                Err(Completion::BudgetExhausted) => break,
                Err(other) => panic!("unexpected completion: {other:?}"),
            }
        }
        bytes += budget.used().bytes;
    }
    assert_eq!(text, "中 + \\alpha\r\n🙂 + &amp;");
    assert_eq!(bytes, math.content.len());
    println!(
        "{frames} frames / {bytes} source bytes; prefixes omitted, raw CRLF and TeX preserved"
    );
    Ok(())
}
