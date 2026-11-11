use mdstream::*;

// Callers pass the expected raw title and structural gaps, independently of
// Document metadata. Used only for the exact new successes in old corpora.
pub fn assert_plain(
    d: &Document,
    id: LinkId,
    label: Span,
    source: Span,
    target: Span,
    title: Span,
    gaps: &[Span],
) {
    assert_eq!(d.links()[id.0], LinkTarget::Direct(target));
    assert_eq!(
        d.link_info(id),
        Some(&LinkInfo {
            kind: LinkKind::Link,
            label,
            source,
            title: Some(title),
            state: SyntaxState::Sealed,
        })
    );
    let raw = &d.source()[title.range()];
    assert_eq!(d.link_title(id), Some(raw));
    let mut expected = Vec::new();
    let mut at = title.start;
    while at < title.end {
        if let Some(gap) = gaps.iter().find(|gap| gap.start == at) {
            at = gap.end;
        } else if d.source()[at..title.end].starts_with("\r\n") {
            expected.push((Span::new(at, at + 2), "\n".to_owned()));
            at += 2;
        } else {
            let c = d.source()[at..title.end].chars().next().unwrap();
            expected.push((Span::new(at, at + c.len_utf8()), c.to_string()));
            at += c.len_utf8();
        }
    }
    let mut c = d.decoded_link_title(id).unwrap();
    let mut actual = Vec::new();
    while let Some(chunk) = c.advance(&mut Budget::new(WorkLimit::unlimited())).unwrap() {
        actual.push((chunk.source, chunk.text.to_owned()));
    }
    assert_eq!(actual, expected);
}
