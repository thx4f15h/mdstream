use mdstream::{Budget, DisplayStep, LinkId, Span, Style, Utf8Document, WorkLimit};

fn main() {
    let source = format!(
        "p [keep](/u) `q{} &amp; *em* [new](/v)",
        "`` 中😀 ``x".repeat(40)
    );
    let mut streamed = Utf8Document::new();
    for chunk in source.as_bytes().chunks(7) {
        streamed.append(chunk).unwrap();
    }
    streamed.finish();
    let doc = streamed.document();
    let mut cursor = doc.display_cursor(Span::new(0, source.len())).unwrap();
    let mut shown = String::new();
    loop {
        let mut budget = Budget::new(WorkLimit {
            bytes: 6,
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        match cursor.next(doc, &mut budget).unwrap() {
            DisplayStep::Run(run) => {
                if !run.style.contains(Style::HIDDEN) {
                    shown.push_str(run.text);
                }
            }
            DisplayStep::Complete => break,
            DisplayStep::BudgetExhausted => panic!("six bytes must advance"),
        }
    }
    assert_eq!(shown, format!("p keep `q{} & em new", "中😀x".repeat(40)));
    assert_eq!(doc.link_destination(LinkId(0)), Some("/u"));
    assert_eq!(
        doc.link_activity(LinkId(1))
            .unwrap()
            .advance(doc, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap(),
        Some(mdstream::LinkDisposition::Active)
    );
    println!("80 secondary runs recovered into 40 code spans with six-byte frames");
    println!("work: {:?}", doc.work());
}
