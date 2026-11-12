// Only three exact historical inputs (and their EOF prefixes) acquire a new
// global shortcut symbol from the newly parsed footnote. Keep the old golden
// arrays unchanged. Assert the new symbol/hosted object independently, then
// compare every remaining root field with the original snapshot verbatim.
use mdstream::*;
pub fn snapshot(
    doc: &Document,
    mut snapshot: super::support::Snapshot,
) -> super::support::Snapshot {
    const SOURCES: &[&str] = &[
        "[^f]: [a]\n",
        "[^f]: [a](u\n 't')\n",
        "> [^f]: [a](u 'a\n> b')\n",
    ];
    if !SOURCES.iter().any(|s| s.starts_with(doc.source())) {
        return snapshot;
    }
    let Some(start) = doc.source().find("[a]") else {
        return snapshot;
    };
    assert_eq!(
        doc.references(),
        [Reference {
            label: "a".into(),
            destination: None,
            title: None,
            revision: 0,
            first_use: Some(start)
        }]
    );
    assert!(doc.links().is_empty());
    let body = doc.footnote_body(FootnoteId(0)).unwrap();
    let id = body.qualify(LinkId(0));
    let info = body.link_info(id).unwrap().unwrap();
    assert_eq!(info.body, body.key());
    assert_eq!(info.value.label, Span::new(1, 2));
    assert_eq!(info.value.source, Span::new(0, 3));
    assert_eq!(info.value.kind, LinkKind::Link);
    assert_eq!(
        body.canonical_symbol(body.qualify(SymbolId(0))).unwrap(),
        Some(SymbolId(0))
    );
    assert_eq!(body.link_destination(id).unwrap(), None);
    let mut resolver = body.link_resolver();
    let mut activity = body.link_activity(id).unwrap().unwrap();
    assert_eq!(
        activity
            .advance(
                body,
                &mut resolver,
                &mut Budget::new(WorkLimit::unlimited())
            )
            .unwrap(),
        Some(LinkDisposition::Unresolved)
    );
    snapshot.references.clear();
    snapshot
}
