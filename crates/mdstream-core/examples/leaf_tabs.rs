//! Start scoped headings/code/math with a split tab; consume original maps.
use mdstream::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new();
    doc.append("- intro\n\t# 中🙂\n\t```rs\n\tlet x = 1;\n")?;
    let heading = doc.headings().entries()[0];
    assert_eq!(heading.container, doc.blocks()[0].container);
    assert_eq!(doc.text(heading.content), Some("中🙂"));
    let code = FoldId::Code(BlockId(2));
    let mut view = FoldView::new(&doc, 1)?;
    view.set_collapsed(&doc, code, true)?;
    let mut update = doc.append("\t```\n\t$$\n\tx+y\n\t$$\n\t  literal\n")?;
    update.merge(doc.finish())?;
    view.apply(&doc, &update)?;
    assert!(view.is_collapsed(code));
    assert_eq!(doc.headings().entries()[0], heading);
    assert!(matches!(
        doc.blocks()[3].kind,
        BlockKind::Math { closed: true, .. }
    ));
    assert_eq!(doc.blocks()[4].kind, BlockKind::Code { info: None });

    let mut cursor = doc
        .display_cursor(Span::new(0, doc.source().len()))
        .unwrap();
    let mut text = String::new();
    let mut frames = 0;
    let mut bytes = 0;
    loop {
        frames += 1;
        let mut budget = Budget::new(WorkLimit {
            bytes: 16,
            queries: 3,
            objects: 3,
            ..Default::default()
        });
        let mut complete = false;
        loop {
            match cursor.next(&doc, &mut budget)? {
                DisplayStep::Run(run) => {
                    if !run.style.contains(Style::HIDDEN) {
                        if run.replacement.is_some() {
                            println!(
                                "{:?}: {:?} -> {:?}",
                                run.source,
                                doc.text(run.source),
                                run.text
                            );
                        }
                        text.push_str(run.text);
                    }
                }
                DisplayStep::BudgetExhausted => break,
                DisplayStep::Complete => {
                    complete = true;
                    break;
                }
            }
        }
        bytes += budget.used().bytes;
        if complete {
            break;
        }
    }
    assert_eq!(text, "intro\n中🙂\n  let x = 1;\n  x+y\nliteral\n");
    println!("{frames} frames / {bytes} charged bytes; heading, code and math retain their scope");
    Ok(())
}
