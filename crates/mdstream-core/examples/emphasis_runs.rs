use mdstream::{Budget, DisplayStep, Document, Span, Style, WorkLimit};

fn main() {
    let mut doc = Document::new();
    for chunk in ["***中", "** 文* and __foo_", " bar_ and **tail", "***\n"] {
        doc.append(chunk).unwrap();
    }
    doc.finish();
    let mut cursor = doc
        .display_cursor(Span::new(0, doc.source().len()))
        .unwrap();
    let mut shown = String::new();
    while cursor.source_position() < doc.source().len() {
        let mut budget = Budget::new(WorkLimit {
            bytes: 6,
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        let DisplayStep::Run(run) = cursor.next(&doc, &mut budget).unwrap() else {
            panic!("six-byte frame must advance")
        };
        if !run.style.contains(Style::HIDDEN) {
            println!(
                "{:?}: {:?}, emphasis={}, strong={}",
                run.source, run.text, run.emphasis_depth, run.strong_depth
            );
            shown.push_str(run.text);
        }
    }
    assert_eq!(shown, "中 文 and foo bar and tail*\n");
    let work = doc.work();
    assert!(work.delimiter_search_steps <= 6 * work.delimiter_steps);
}
