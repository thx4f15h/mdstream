use mdstream::{Budget, DisplayStep, Document, LinkId, Span, Style, WorkLimit};

fn main() {
    let mut doc = Document::new();
    for chunk in ["**前 [签 *中", "😀][r] 后** 尾*\n\n", "[r]: /目标\n"] {
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
            assert_eq!(run.emphasis_depth, 0);
            println!(
                "{:?}: {:?}, strong={}",
                run.source, run.text, run.strong_depth
            );
            shown.push_str(run.text);
        }
    }
    assert_eq!(shown, "前 签 *中😀 后 尾*\n\n");
    assert_eq!(doc.link_destination(LinkId(0)), Some("/目标"));
    assert!(doc.work().delimiter_search_steps <= 6 * doc.work().delimiter_steps);
}
