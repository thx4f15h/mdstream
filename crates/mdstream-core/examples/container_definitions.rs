use mdstream::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new();
    doc.append("[guide][ref] and [^note]\n")?;
    let mut budget = Budget::new(WorkLimit::unlimited());
    let before = doc
        .resolve_footnote(FootnoteReferenceId(0), &mut budget)
        .unwrap()
        .unwrap();
    assert_eq!(doc.link_destination(LinkId(0)), None);
    assert_eq!(before.target, FootnoteTarget::Unresolved);
    doc.append(
        "> - [x] [ref]: <guide.md#details> 'Guide'\n>   [^note]: 容器中的 **原始正文**。\n",
    )?;
    let resolved = doc
        .resolve_footnote(FootnoteReferenceId(0), &mut budget)
        .unwrap()
        .unwrap();
    assert_eq!(
        (before.symbol, before.number),
        (resolved.symbol, resolved.number)
    );
    let FootnoteTarget::Defined(definition) = resolved.target else {
        panic!("missing definition");
    };
    let container = doc.block(definition.block).unwrap().container.unwrap();
    assert_eq!(doc.container(container).unwrap().depth, 3);
    let mut view = FoldView::new(&doc, 1)?;
    view.set_collapsed(&doc, FoldId::Container(container), true)?;
    let mut update = doc.append("[again][ref] [^note]\n> [ref]: /ignored\n> [^note]: ignored\n")?;
    update.merge(doc.finish())?;
    view.apply(&doc, &update)?;
    assert!(view.is_collapsed(FoldId::Container(container)));
    assert_eq!(doc.link_destination(LinkId(1)), Some("guide.md#details"));
    assert_eq!(doc.link_title(LinkId(1)), Some("Guide"));
    let mut body = doc.footnote_content(resolved.symbol).unwrap();
    let mut text = String::new();
    let mut frames = 0;
    let mut bytes = 0;
    loop {
        frames += 1;
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 8,
            ..Default::default()
        });
        match body.advance(&mut budget) {
            Ok(Some(chunk)) => text.push_str(chunk.text),
            Ok(None) => break,
            Err(status) => panic!("unexpected {status:?}"),
        }
        bytes += budget.used().bytes;
    }
    assert_eq!(text, "容器中的 **原始正文**。");
    println!(
        "global link: {:?}; footnote #{} in {container:?}: {text}; {frames} frames / {bytes} bytes",
        doc.link_destination(LinkId(0)),
        resolved.number
    );
    Ok(())
}
