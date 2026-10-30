use mdtui3::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new();
    doc.append(&format!("{} $$中🙂 + \\alpha", "prefix ".repeat(160)))?;
    assert!(doc.maths().is_empty());
    let update = doc.append("$$ ")?;
    assert_eq!(update.new_maths, 0..1);
    let math = *doc.math(MathId(0)).unwrap();
    println!("confirmed {:?}: {:?}", math.kind, math.content);

    let mut objects = doc.maths_in(math.source).unwrap();
    let mut budget = Budget::new(WorkLimit {
        queries: 2,
        objects: 2,
        bytes: 4,
        ..Default::default()
    });
    let (id, _) = objects.advance(&mut budget).unwrap().unwrap();
    let mut content = doc.math_content(id).unwrap();
    let mut output = String::new();
    let mut frames = 1;
    let mut bytes = 0;
    loop {
        match content.advance(&mut budget) {
            Ok(Some(chunk)) => output.push_str(chunk.text),
            Ok(None) => {
                bytes += budget.used().bytes;
                break;
            }
            Err(Completion::BudgetExhausted) => {
                bytes += budget.used().bytes;
                frames += 1;
                budget = Budget::new(WorkLimit {
                    queries: 1,
                    objects: 1,
                    bytes: 4,
                    ..Default::default()
                });
            }
            Err(other) => panic!("unexpected completion: {other:?}"),
        }
    }
    assert_eq!(output, "中🙂 + \\alpha");
    assert_eq!(bytes, math.content.len());
    println!("{frames} frames / {bytes} bytes: {output}");

    // Confirmed IDs/ranges survive append. Unclosed candidates do not publish
    // objects, and EOF does not replay their Markdown-looking content.
    doc.append("$unclosed **literal**")?;
    doc.finish();
    assert_eq!(doc.maths(), &[math]);
    Ok(())
}
