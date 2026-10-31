//! Read a folded code summary without expanding or laying out its body.
use mdstream::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new();
    doc.append("```rs\nlet name = \"世界\";\nprintln!(\"{name}\");\n")?;
    doc.append(&"unread body\n".repeat(100_000))?;
    doc.append("```\n")?;
    doc.finish();
    let id = FoldId::Code(BlockId(0));
    let mut view = FoldView::new(&doc, 1)?;
    view.set_collapsed(&doc, id, true)?;
    let candidate = doc.fold_candidate(id).unwrap();
    let mut excerpt = DisplayExcerpt::new(
        &doc,
        candidate.hidden,
        ExcerptLimits {
            source_bytes: 512,
            display_bytes: 128,
            lines: 2,
        },
    )?;
    let mut frames = 0;
    let mut read_bytes = 0;
    let end = 'frames: loop {
        frames += 1;
        let mut budget = Budget::new(WorkLimit {
            queries: 2,
            objects: 2,
            bytes: 24,
            ..Default::default()
        });
        loop {
            match excerpt.next(&doc, &mut budget)? {
                ExcerptStep::Run(run) => print!("{}", run.text),
                ExcerptStep::BudgetExhausted => break,
                ExcerptStep::Complete(end) => {
                    read_bytes += budget.used().bytes;
                    break 'frames end;
                }
            }
        }
        read_bytes += budget.used().bytes;
    };
    assert!(view.is_collapsed(id));
    println!("end={end:?}, frames={frames}, charged bytes={read_bytes}, fold unchanged");
    Ok(())
}
