use mdtui3::*;

fn parsed(source: &str, split: usize) -> Document {
    let mut doc = Document::new();
    doc.append(&source[..split]).unwrap();
    doc.append(&source[split..]).unwrap();
    doc.finish();
    doc
}

#[test]
fn angle_uri_and_email_are_chunk_invariant_and_source_ordered() {
    let source = format!(
        "{}<https://例子.test/a?b> <foo+tag@example.com> [x](url) <ab:>\n",
        "x".repeat(BLOCK_LOOKAHEAD)
    );
    let whole = parsed(&source, 0);
    assert_eq!(whole.links().len(), 4);
    assert_eq!(
        whole.link_url(LinkId(1)).unwrap(),
        "mailto:foo+tag@example.com"
    );
    for (split, _) in source.char_indices().skip(BLOCK_LOOKAHEAD) {
        let split = parsed(&source, split);
        assert_eq!(whole.links(), split.links());
        for i in 0..whole.links().len() {
            assert_eq!(whole.link_info(LinkId(i)), split.link_info(LinkId(i)));
        }
    }
    let objects: Vec<_> = whole.links_in(Span::new(0, source.len())).collect();
    assert!(
        objects
            .windows(2)
            .all(|w| w[0].1.source.end <= w[1].1.source.start)
    );
}

#[test]
fn failed_unfinished_escaped_code_and_oversized_candidates_stay_literal() {
    for text in [
        "<m:a>",
        "<https://a b>",
        "<a@-host>",
        "<a@host->",
        "<@host>",
        "<a@>",
        "<https://unfinished",
        "`<https://code>`",
        "\\<https://escaped>",
    ] {
        let doc = parsed(text, 0);
        assert!(doc.links().is_empty(), "{text}");
    }
    let source = format!("<https://{}>", "x".repeat(MAX_AUTOLINK_BYTES));
    let doc = parsed(&source, BLOCK_LOOKAHEAD);
    assert!(doc.links().is_empty());
    assert!(doc.work().inline_bytes <= source.len() as u64);
}

#[test]
fn nested_brackets_cannot_create_overlapping_autolinks() {
    let doc = parsed("[outer <https://example.test>](other) [next](x)\n", 0);
    assert_eq!(doc.links().len(), 2);
    let a = doc.link_info(LinkId(0)).unwrap();
    let b = doc.link_info(LinkId(1)).unwrap();
    assert!(a.source.end <= b.source.start);
}
