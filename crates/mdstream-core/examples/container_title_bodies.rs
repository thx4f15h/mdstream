//! Stable image identity and source-mapped, budgeted container title decoding.
use mdstream::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new();
    doc.append("> ![图](u 'a\r\n")?;
    let original = *doc.link_info(LinkId(0)).unwrap();
    doc.append(">\t 中&amp;\r\n>   ')\n")?;
    doc.finish();
    assert_eq!(doc.links().len(), 1);
    assert_eq!(doc.link_info(LinkId(0)).unwrap().label, original.label);
    assert_eq!(doc.link_title(LinkId(0)), Some("a\r\n>\t 中&amp;\r\n>   "));
    let mut title = doc.decoded_link_title(LinkId(0)).unwrap();
    let mut text = String::new();
    loop {
        let mut budget = Budget::new(WorkLimit {
            bytes: 6,
            queries: 1,
            objects: 1,
            ..WorkLimit::default()
        });
        match title.advance(&mut budget) {
            Ok(Some(chunk)) => {
                println!("{:?}: {:?}", chunk.source, chunk.text);
                text.push_str(chunk.text);
            }
            Ok(None) => break,
            Err(Completion::BudgetExhausted) => (),
            Err(error) => panic!("{error:?}"),
        }
    }
    assert_eq!(text, "a\n 中&\n  ");
    Ok(())
}
