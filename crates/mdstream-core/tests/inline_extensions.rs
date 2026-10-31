use mdstream::{
    Document, LinkId, LinkKind, MAX_DESTINATION_BYTES, MAX_LINK_TITLE_BYTES, Span, Style,
};

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    // Exercise the live inline state machine, beyond bounded line buffering.
    doc.append(&"x".repeat(mdstream::BLOCK_LOOKAHEAD)).unwrap();
    for c in source.chars() {
        doc.append(c.encode_utf8(&mut [0; 4])).unwrap();
    }
    doc.finish();
    doc
}

#[test]
fn destinations_and_titles_preserve_exact_source_spans() {
    for (source, destination, title) in [
        (r#"[a](/uri "title")"#, "/uri", Some("title")),
        ("[a]( /uri\t'标题' )", "/uri", Some("标题")),
        (r"[a](a(b(c)) (ti\)tle))", "a(b(c))", Some(r"ti\)tle")),
        (r#"[a](<a b\>c> "t\"t")"#, r"a b\>c", Some(r#"t\"t"#)),
        ("[a](<> '')", "", Some("")),
        ("[a]()", "", None),
        ("[a](  )", "", None),
        ("[](<a b>)", "a b", None),
        ("![图](<图.png> '说明')", "图.png", Some("说明")),
    ] {
        let doc = parse(source);
        assert_eq!(doc.links().len(), 1, "{source}");
        assert_eq!(
            doc.link_destination(LinkId(0)),
            Some(destination),
            "{source}"
        );
        assert_eq!(doc.link_title(LinkId(0)), title, "{source}");
        let info = doc.link_info(LinkId(0)).unwrap();
        assert_eq!(doc.text(info.source), Some(source));
        assert_eq!(info.kind == LinkKind::Image, source.starts_with('!'));
        assert_eq!(info.title.and_then(|span| doc.text(span)), title);
    }
}

#[test]
fn reference_titles_resolve_late_and_first_definition_wins() {
    let mut doc = Document::new();
    doc.append("[a] ![pic][a]\n\n").unwrap();
    assert_eq!(doc.link_title(LinkId(0)), None);
    let before = doc.work();
    let update = doc.append("[a]: <with space> 'description'\n").unwrap();
    assert_eq!(update.references_changed.len(), 1);
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    for id in [LinkId(0), LinkId(1)] {
        assert_eq!(doc.link_destination(id), Some("with space"));
        assert_eq!(doc.link_title(id), Some("description"));
    }
    doc.append("[a]: /other \"different\"\n").unwrap();
    assert_eq!(doc.link_title(LinkId(0)), Some("description"));
    doc.append("[slash]: path\\\n").unwrap();
    assert!(
        doc.references().iter().any(
            |r| r.label == "slash" && r.destination.and_then(|s| doc.text(s)) == Some("path\\")
        )
    );
}

#[test]
fn malformed_and_oversized_suffixes_do_not_become_direct_links() {
    for source in [
        "[a](<bad<url>)".into(),
        "[a](<url>\"no separator\")".into(),
        "[a](url (nested (title)))".into(),
        "[a](url \"unfinished)".into(),
        "[a](url \"two\nlines\")".into(),
        "[a](bad target)".into(),
        format!("[a]({})", "x".repeat(MAX_DESTINATION_BYTES + 1)),
        format!("[a](url '{}')", "x".repeat(MAX_LINK_TITLE_BYTES + 1)),
    ] {
        let doc = parse(&source);
        assert_eq!(doc.link_destination(LinkId(0)), None, "{source}");
        assert_eq!(doc.link_title(LinkId(0)), None);
        assert_eq!(doc.work().inline_bytes, doc.source().len() as u64);
    }
    let doc = parse(&format!(
        "[a]({} '{}')",
        "x".repeat(MAX_DESTINATION_BYTES),
        "t".repeat(MAX_LINK_TITLE_BYTES)
    ));
    assert_eq!(
        doc.link_destination(LinkId(0)).unwrap().len(),
        MAX_DESTINATION_BYTES
    );
    assert_eq!(
        doc.link_title(LinkId(0)).unwrap().len(),
        MAX_LINK_TITLE_BYTES
    );
    for (spaces, expected) in [
        (mdstream::MAX_LINK_SUFFIX_BYTES - 1, Some("")),
        (mdstream::MAX_LINK_SUFFIX_BYTES, None),
    ] {
        let doc = parse(&format!("[a]({})", " ".repeat(spaces)));
        assert_eq!(doc.link_destination(LinkId(0)), expected);
    }
}

fn breaks(doc: &Document) -> Vec<(usize, bool)> {
    doc.runs_in(Span::new(0, doc.source().len()))
        .unwrap()
        .filter_map(|run| {
            let hard = run.style.contains(Style::HARD_BREAK);
            (hard || run.style.contains(Style::SOFT_BREAK)).then(|| {
                assert_eq!(run.text, "\n");
                (run.source.start, hard)
            })
        })
        .collect()
}

#[test]
fn line_breaks_are_confirmed_only_when_the_inline_block_continues() {
    for (source, hard) in [
        ("a  \nb", true),
        ("a\\\nb", true),
        ("a \nb", false),
        ("a\nb", false),
        ("a\\\r\nb", true),
        ("a  \r\nb", true),
        ("a\\\\\nb", false),
    ] {
        let doc = parse(source);
        let offset = mdstream::BLOCK_LOOKAHEAD + source.find('\n').unwrap();
        assert_eq!(breaks(&doc), vec![(offset, hard)], "{source:?}");
    }
    for source in [
        "a  \n",
        "a\\\n",
        "a  \n\nb",
        "a\\\n# b\n",
        "`a  \nb`",
        "a  \n---\n",
        "a\\\r*literal*\n",
    ] {
        let doc = parse(source);
        assert!(breaks(&doc).is_empty(), "{source:?}");
    }
}

#[test]
fn late_line_break_confirmation_invalidates_earlier_tiles() {
    let mut doc = Document::new();
    doc.append(&format!("{}\\\r\n", "x".repeat(2046))).unwrap();
    let newline = doc.source().len() - 1;
    let tile = doc.tile_at(newline).unwrap();
    let key = doc.tile_key(tile).unwrap();
    assert!(breaks(&doc).is_empty());
    let update = doc.append("b\n").unwrap();
    assert_eq!(breaks(&doc), vec![(newline, true)]);
    assert_ne!(doc.tile_key(tile).unwrap(), key);
    assert!(
        update
            .dirty
            .iter()
            .any(|s| s.start <= newline && newline < s.end)
    );
    let visible: String = doc
        .runs_in(Span::new(2046, doc.source().len()))
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    assert_eq!(visible, "\nb\n");
}
