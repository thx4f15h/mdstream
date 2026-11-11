use mdstream::*;

// Independent typed model for the exact historical inputs newly completed by B.
// The allowlist lives in the caller; this helper cannot exempt any input.
pub fn assert_direct(d: &Document, label: &str, index: usize) {
    let source = d.source();
    let open = source.find(&format!("[{label}]")).unwrap();
    let label_start = open + 1;
    let suffix_start = label_start + label.len() + 2;
    let title_open = source.rfind('\'').unwrap();
    let title_start = source[..title_open].rfind('\'').unwrap() + 1;
    let target_start = suffix_start
        + source.as_bytes()[suffix_start..]
            .iter()
            .take_while(|&&b| b == b' ')
            .count();
    let target_end = source[target_start..].find(['\n', '\r']).unwrap() + target_start;
    let id = LinkId(index);
    assert_eq!(
        d.links()[index],
        LinkTarget::Direct(Span::new(target_start, target_end))
    );
    assert_eq!(
        d.link_destination(id),
        Some(&source[target_start..target_end])
    );
    assert_eq!(d.link_title(id), Some(&source[title_start..title_open]));
    let info = d.link_info(id).unwrap();
    assert_eq!(
        *info,
        LinkInfo {
            kind: LinkKind::Link,
            label: Span::new(label_start, label_start + label.len()),
            source: Span::new(
                open,
                source[title_open..].find(')').unwrap() + title_open + 1
            ),
            title: Some(Span::new(title_start, title_open)),
            state: SyntaxState::Sealed,
        }
    );
    let visible: String = d
        .display_runs_in(info.source)
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    assert_eq!(visible, label);
    let reference = &d.references()[index];
    assert_eq!(reference.label, label);
    assert_eq!(reference.first_use, Some(open));
    assert_eq!(reference.revision, 0);
}
