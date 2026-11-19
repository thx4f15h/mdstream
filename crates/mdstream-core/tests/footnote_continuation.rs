use mdstream::*;

fn parse(source: &str) -> Document {
    let mut d = Document::new();
    d.enable_footnote_continuation().unwrap();
    d.append(source).unwrap();
    d.finish();
    d
}
fn body(d: &Document) -> String {
    let v = d.footnote_body(FootnoteId(0)).unwrap();
    let mut cursor = v.source_cursor(v.range()).unwrap();
    let mut text = String::new();
    let mut budget = Budget::new(WorkLimit::unlimited());
    while let Some(chunk) = cursor.advance(&mut budget).unwrap() {
        text.push_str(chunk.text);
    }
    text
}

#[test]
fn paragraph_lazy_and_omitted_items_use_the_original_winning_parser() {
    for (source, expected) in [
        ("[^n]: first\nlazy\n# root\n", "first\nlazy"),
        ("> [^n]: first\n> lazy\n# root\n", "first\nlazy"),
        ("- [^n]: first\nlazy\n# root\n", "first\nlazy"),
        ("> - [^n]: first\n> lazy\n# root\n", "first\nlazy"),
        ("> - [^n]: first\n>   lazy\n# root\n", "first\nlazy"),
        ("> - - > - [^n]: first\n> > lazy\n# root\n", "first\nlazy"),
    ] {
        let d = parse(source);
        assert_eq!(d.source(), source);
        assert_eq!(body(&d), expected, "{source:?}");
        assert_eq!(d.footnote_body_count(), 1);
    }
}

#[test]
fn missing_quotes_new_blocks_blanks_and_siblings_end_lazy_eligibility() {
    for source in [
        "> [^n]: first\nlazy\n",
        "[^n]: first\n# root\n",
        "[^n]: first\n---\n",
        "[^n]: first\n```\n",
        "[^n]: first\n$$\nmath\n$$\n",
        "[^n]: first\n> new quote\n",
        "[^n]: first\n- [ ] new task\n",
        "[^n]: first\n[^m]: second\n",
        "[^n]: first\n[r]: /root\n",
        "- [^n]: first\n- sibling\n",
        "> - - > - [^n]: first\n> missing inner quote\n",
        "- > - > - [^n]: first\nmissing both quotes\n",
    ] {
        assert_eq!(body(&parse(source)), "first", "{source:?}");
    }
    assert_eq!(body(&parse("[^n]: first\n\nroot\n")), "first\n");
}

#[test]
fn duplicate_definitions_have_the_same_lazy_boundary_without_publication() {
    let source = "[^n]: win\n# stop\n[^n]: lost\nlazy [^evil] [r]: /evil\n# end\n";
    let d = parse(source);
    assert_eq!(body(&d), "win");
    assert_eq!(d.footnotes().len(), 1);
    assert_eq!(d.footnote_body_count(), 1);
    let root = d
        .blocks()
        .iter()
        .find(|b| b.source.start == source.find("# end").unwrap())
        .unwrap();
    assert!(matches!(root.kind, BlockKind::Heading { .. }));
    assert_eq!(d.footnote_references().len(), 0);
}

#[test]
fn code_and_table_body_blocks_cannot_grant_paragraph_lazy_eligibility() {
    for (source, expected) in [
        (
            "[^n]: ```\n    literal\n    ```\nroot\n",
            "```\nliteral\n```",
        ),
        ("[^n]: a | b\n    --- | ---\nroot\n", "a | b\n--- | ---"),
        (
            "[^n]: > quote\n    continuation\nroot\n",
            "> quote\ncontinuation",
        ),
    ] {
        assert_eq!(body(&parse(source)), expected, "{source:?}");
    }
}

#[test]
fn delimiter_candidates_do_not_get_committed_as_lazy_note_bytes() {
    assert_eq!(body(&parse("[^n]: a | b\n--- | ---\nroot\n")), "a | b");
}

#[test]
fn every_utf8_cut_prefix_eof_and_bytewise_input_keep_the_same_admitted_mapping() {
    fn snapshot(d: &Document) -> String {
        let mut out = format!("{:?}", d.footnotes());
        for (i, _) in d.footnotes().iter().enumerate() {
            if let Some(v) = d.footnote_body(FootnoteId(i)) {
                let mut c = v.source_cursor(v.range()).unwrap();
                let mut budget = Budget::new(WorkLimit::unlimited());
                let mut projected = String::new();
                while let Some(chunk) = c.advance(&mut budget).unwrap() {
                    projected.push_str(chunk.text)
                }
                out.push_str(&format!(
                    "|{projected}|{}:{}:{}",
                    v.block_count(),
                    v.heading_count(),
                    v.table_count()
                ));
            }
        }
        out
    }
    for source in [
        "> - [^n]: *中🙂* [^b]\r\n> lazy [^c]\r\n>   another\r\n# root\r\n[^b]: B\r\n",
        "[^n]: a | b\n    --- | ---\nroot\n[^n]: lost\nlazy [^evil]\n# stop\n",
        "[^n]: ```\n    opaque\n    ```\n    paragraph\nlazy\n# stop\n",
        "[^n]: first\n[r]: /root \"title\"\n",
        "[^n]: first\n[^m]: second\n",
        "[^n]: first\n```rust\nroot\n```\n",
        "[^n]: first\n1. sibling\n",
        "[^n]: first\n---\nroot\n",
        "> - [^n]: first\n> lazy\nmissing quote\n",
        "> - - > - [^n]: first\r\n> > lazy 中🙂\r\n> missing quote\r\n",
        "[^n]: first\n[ordinary] text\n# root\n",
        "[^n]: first\n*emphasis* continuation\n# root\n",
        "[^n]: first\n1.5 is prose\n# root\n",
        "[^n]: first\n: punctuation\n# root\n",
    ] {
        let expected = snapshot(&parse(source));
        for at in (0..=source.len()).filter(|&i| source.is_char_boundary(i)) {
            let mut d = Document::new();
            d.enable_footnote_continuation().unwrap();
            d.append(&source[..at]).unwrap();
            d.append(&source[at..]).unwrap();
            d.finish();
            assert_eq!(snapshot(&d), expected, "cut {at} of {source:?}");
            let prefix = &source[..at];
            let mut bytes = Utf8Document::new();
            bytes.enable_footnote_continuation().unwrap();
            for byte in prefix.as_bytes() {
                bytes.append(&[*byte]).unwrap();
            }
            bytes.finish();
            assert_eq!(
                snapshot(bytes.document()),
                snapshot(&parse(prefix)),
                "prefix {at} of {source:?}"
            );
            assert_eq!(d.source(), source);
        }
    }
}
