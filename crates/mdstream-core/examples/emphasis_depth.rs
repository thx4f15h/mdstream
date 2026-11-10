use mdstream::{Budget, DisplayStep, Document, Span, Style, WorkLimit};

fn main() {
    let mut document = Document::new();
    document
        .append("_foo _bar_ baz_ and **one **two** three**")
        .unwrap();
    document.finish();
    let mut cursor = document
        .display_cursor(Span::new(0, document.source().len()))
        .unwrap();
    let mut inner = 0;
    while cursor.source_position() < document.source().len() {
        let mut budget = Budget::new(WorkLimit {
            bytes: 6,
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        let DisplayStep::Run(run) = cursor.next(&document, &mut budget).unwrap() else {
            panic!("six byte budget must make progress")
        };
        if !run.style.contains(Style::HIDDEN) {
            println!(
                "{:?}: {:?}, emphasis={}, strong={}",
                run.source, run.text, run.emphasis_depth, run.strong_depth
            );
            inner += usize::from(run.emphasis_depth == 2 || run.strong_depth == 2);
        }
    }
    assert!(inner >= 2);
}
