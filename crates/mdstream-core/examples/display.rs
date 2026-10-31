use mdstream::{Budget, DisplayStep, Document, Span, Style, WorkLimit};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new();
    doc.append("# Display &amp; source\n\n&#x4e2d;文 ` a\r\nb ` &NotEqualTilde;\n")?;
    doc.finish();
    let mut cursor = doc
        .display_cursor(Span::new(0, doc.source().len()))
        .unwrap();
    let mut frames = 0;
    loop {
        frames += 1;
        let mut budget = Budget::new(WorkLimit {
            bytes: 32,
            queries: 8,
            objects: 8,
            ..Default::default()
        });
        loop {
            match cursor.next(&doc, &mut budget)? {
                DisplayStep::Run(run) => {
                    if !run.style.contains(Style::HIDDEN) {
                        print!("{}", run.text);
                    }
                }
                DisplayStep::BudgetExhausted => break,
                DisplayStep::Complete => {
                    eprintln!(
                        "read {} source bytes in {frames} bounded steps",
                        doc.source().len()
                    );
                    return Ok(());
                }
            }
        }
    }
}
