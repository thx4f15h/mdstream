//! Read a streamed, multi-paragraph footnote with source mapping and frame budgets.
use mdtui3::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new();
    doc.append("See [^note] twice: [^note].\n> - [^note]: 第一段\r\n")?;
    let old = doc.footnote(FootnoteId(0)).unwrap().definition.unwrap();
    assert_eq!(old.state, SyntaxState::Growing);
    let fold = FoldId::Container(doc.block(old.block).unwrap().container.unwrap());
    let mut view = FoldView::<()>::new(&doc, 1)?;
    view.set_collapsed(&doc, fold, true)?;
    let mut update = doc.append(">       **raw** &amp;\r\n>   \r\n>       第二段🙂\r\n")?;
    update.merge(doc.finish())?;
    view.apply(&doc, &update)?;
    assert!(view.is_collapsed(fold));
    let mut budget = Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        dependencies: 1,
        ..Default::default()
    });
    let note = doc
        .resolve_footnote(FootnoteReferenceId(1), &mut budget)
        .unwrap()
        .unwrap();
    let FootnoteTarget::Defined(definition) = note.target else {
        unreachable!()
    };
    assert_eq!(note.number, 1);
    assert_eq!(definition.block, old.block);
    assert_eq!(definition.state, SyntaxState::Sealed);

    let mut cursor = doc.footnote_payload(note.symbol).unwrap();
    let mut text = String::new();
    let mut frames = 0;
    let mut bytes = 0;
    'frames: loop {
        frames += 1;
        let mut budget = Budget::new(WorkLimit {
            queries: 2,
            objects: 2,
            bytes: 12,
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
    assert_eq!(text, "第一段\r\n**raw** &amp;\r\n\r\n第二段🙂");
    assert_eq!(bytes, definition.content.len());
    println!(
        "footnote #{}: {frames} frames / {bytes} source bytes; fold preserved",
        note.number
    );
    Ok(())
}
