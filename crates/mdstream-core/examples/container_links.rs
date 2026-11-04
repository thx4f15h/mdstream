//! Read source-mapped multiline image alt text using the frame's shared budget.
use mdstream::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new();
    doc.append("> - ![中 &amp;\r\n")?;
    let first = doc.blocks()[0].clone();
    let mut view = FoldView::new(&doc, 1)?;
    let fold = FoldId::Container(first.container.unwrap());
    view.set_collapsed(&doc, fold, true)?;
    let update = doc.append(">   🙂][]\n")?;
    view.apply(&doc, &update)?;
    let id = LinkId(0);
    let info = *doc.link_info(id).unwrap();
    assert_eq!(doc.link_destination(id), None);
    let mut update = doc.append(">\n>   [中 &amp; 🙂]: /image '说明'\n")?;
    update.merge(doc.finish())?;
    view.apply(&doc, &update)?;
    assert_eq!(doc.blocks()[0], first);
    assert_eq!(*doc.link_info(id).unwrap(), info);
    assert_eq!(doc.link_destination(id), Some("/image"));
    assert!(view.is_collapsed(fold));
    let mut cursor = doc.display_cursor(info.label).unwrap();
    let mut text = String::new();
    let mut frames = 0;
    let mut charged = 0;
    'frames: loop {
        frames += 1;
        let mut budget = Budget::new(WorkLimit {
            queries: 2,
            objects: 2,
            bytes: 16,
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
    assert_eq!(text, "中 &\n🙂");
    println!("{text}\n{frames} frames / {charged} charged bytes; stable image ID and item fold");
    Ok(())
}
