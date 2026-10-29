use mdtui3::{BlockKind, Document, LinkId, LinkKind, Span, Style};

#[test]
fn tasks_are_structured_and_never_reference_links() {
    let mut doc = Document::new();
    doc.append("- [ ] todo\n- [x] done\n1.   [X] **yes**\n- [x]text\n- normal\n")
        .unwrap();
    doc.finish();
    let tasks: Vec<_> = doc
        .blocks()
        .iter()
        .filter_map(|b| match b.kind {
            BlockKind::ListItem { task, .. } => task,
            _ => None,
        })
        .collect();
    assert_eq!(
        tasks.iter().map(|t| t.checked).collect::<Vec<_>>(),
        [false, true, true]
    );
    assert_eq!(doc.text(tasks[1].source), Some("[x]"));
    assert_eq!(doc.links().len(), 1); // only the ordinary [x]text in the fourth line
    assert!(
        doc.runs_in(Span::new(0, doc.source().len()))
            .unwrap()
            .any(|r| r.text == "yes" && r.style.contains(Style::STRONG))
    );
}

#[test]
fn images_keep_kind_alt_and_late_destinations() {
    let mut doc = Document::new();
    doc.append(
        "![alt](img.png) [link](url) ![photo][pic] ![](empty.png) \\![ordinary](target)\n\n",
    )
    .unwrap();
    let infos: Vec<_> = (0..doc.links().len())
        .map(|i| *doc.link_info(LinkId(i)).unwrap())
        .collect();
    assert_eq!(
        infos.iter().map(|i| i.kind).collect::<Vec<_>>(),
        [
            LinkKind::Image,
            LinkKind::Link,
            LinkKind::Image,
            LinkKind::Image,
            LinkKind::Link
        ]
    );
    assert_eq!(doc.text(infos[0].label), Some("alt"));
    assert_eq!(doc.text(infos[0].source), Some("![alt](img.png)"));
    assert_eq!(doc.link_destination(LinkId(2)), None);
    doc.append("[pic]: /photo.jpg\n").unwrap();
    assert_eq!(doc.link_destination(LinkId(2)), Some("/photo.jpg"));
    let empty = infos[3].source;
    assert_eq!(
        doc.links_in(empty).map(|(id, _)| id).collect::<Vec<_>>(),
        [LinkId(3)]
    );
    assert_eq!(doc.links_in(Span::new(empty.start, empty.start)).count(), 0);
    for pair in infos.windows(2) {
        assert!(pair[0].source.end <= pair[1].source.start);
    }
}
