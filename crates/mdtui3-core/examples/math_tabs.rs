//! Read inline/block math through one frame budget, retaining tab source maps.
use mdtui3::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new();
    doc.append("- $中\\alpha\n\t🙂$\n  $$\n\t中 &amp;\n")?;
    let inline = *doc.math(MathId(0)).unwrap();
    let block = BlockId(2);
    let fold = FoldId::Math(block);
    let mut view = FoldView::new(&doc, 1)?;
    view.set_collapsed(&doc, fold, true)?;
    let mut update = doc.append("\t🙂\r\n\t $$\n")?;
    update.merge(doc.finish())?;
    view.apply(&doc, &update)?;
    assert_eq!(*doc.math(MathId(0)).unwrap(), inline);
    assert!(view.is_collapsed(fold));
    assert!(matches!(
        doc.block(block).unwrap().kind,
        BlockKind::Math { closed: true, .. }
    ));

    let mut readers = [
        doc.math_payload(MathId(0)).unwrap(),
        doc.math_block_payload(block).unwrap(),
    ];
    let mut outputs = [String::new(), String::new()];
    let mut index = 0;
    let mut frames = 0;
    let mut bytes = 0;
    while index < readers.len() {
        frames += 1;
        let mut budget = Budget::new(WorkLimit {
            bytes: 8,
            queries: 2,
            objects: 2,
            ..Default::default()
        });
        while index < readers.len() {
            match readers[index].advance(&mut budget) {
                Ok(Some(chunk)) => {
                    println!(
                        "{:?}: {:?} -> {:?}",
                        chunk.source,
                        doc.text(chunk.source),
                        chunk.text
                    );
                    outputs[index].push_str(chunk.text);
                }
                Ok(None) => index += 1,
                Err(Completion::BudgetExhausted) => break,
                Err(other) => panic!("unexpected {other:?}"),
            }
        }
        bytes += budget.used().bytes;
    }
    assert_eq!(outputs, ["中\\alpha\n  🙂", "  中 &amp;\n  🙂\r\n"]);
    let BlockKind::Math { content, .. } = doc.block(block).unwrap().kind else {
        unreachable!()
    };
    assert_eq!(bytes, inline.content.len() + content.len());
    println!("{frames} frames / {bytes} source bytes; tab mappings and fold choice preserved");
    Ok(())
}
