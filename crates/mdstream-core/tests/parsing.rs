use mdstream::{BlockKind, Document, LinkTarget, Span, Style, TILE_BYTES, TileId};

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn visible(doc: &Document) -> String {
    doc.runs_in(Span::new(0, doc.source().len()))
        .unwrap()
        .filter(|run| !run.style.contains(Style::HIDDEN))
        .map(|run| run.text)
        .collect()
}

#[test]
fn inline_styles_links_and_escapes() {
    let doc = parse(
        "plain *em* **strong** _one_ __two__ ~~gone~~ `*code*` \\*lit\\* [web](https://x/a(b))\n",
    );
    assert_eq!(
        visible(&doc),
        "plain em strong one two gone *code* *lit* web\n"
    );
    let runs: Vec<_> = doc
        .runs_in(Span::new(0, doc.source().len()))
        .unwrap()
        .collect();
    for (text, style) in [
        ("em", Style::EMPHASIS),
        ("strong", Style::STRONG),
        ("one", Style::EMPHASIS),
        ("two", Style::STRONG),
        ("gone", Style::STRIKE),
        ("*code*", Style::CODE),
    ] {
        assert!(
            runs.iter()
                .any(|r| r.text == text && r.style.contains(style)),
            "{text}: {runs:?}"
        );
    }
    let link = runs.iter().find(|r| r.text == "web").unwrap().link.unwrap();
    assert_eq!(doc.link_destination(link), Some("https://x/a(b)"));
}

#[test]
fn block_classification_and_setext() {
    let source = "# Title #\r\n\npara one\npara two\n---\n\n> > quote\n- item\n12. numbered\n\n***\n\n```rs\nx * y\n```\n    code\n";
    let doc = parse(source);
    let kinds: Vec<_> = doc.blocks().iter().map(|b| &b.kind).collect();
    assert!(matches!(kinds[0], BlockKind::Heading { level: 1 }));
    assert!(matches!(kinds[2], BlockKind::Heading { level: 2 }));
    assert!(
        kinds
            .iter()
            .any(|k| matches!(k, BlockKind::Quote { depth: 2 }))
    );
    assert!(kinds.iter().any(|k| matches!(
        k,
        BlockKind::ListItem {
            number: Some(12),
            ..
        }
    )));
    assert!(kinds.iter().any(|k| matches!(k, BlockKind::ThematicBreak)));
    assert_eq!(
        kinds
            .iter()
            .filter(|k| matches!(k, BlockKind::Code { .. }))
            .count(),
        2
    );
    assert!(!visible(&doc).contains('\r'));
    assert!(!visible(&doc).contains("```"));
    assert!(visible(&doc).contains("x * y\ncode\n"));
    for block in doc.blocks().windows(2) {
        assert_eq!(block[0].source.end, block[1].source.start);
    }
    assert_eq!(doc.blocks().last().unwrap().source.end, source.len());
}

#[test]
fn reference_definitions_are_late_and_first_wins() {
    let mut doc = Document::new();
    doc.append("[Guide] and [name][ GUIDE ] and [Guide][]\n\n")
        .unwrap();
    let key = doc.tile_key(TileId(0)).unwrap();
    assert!(
        doc.links()
            .iter()
            .all(|target| matches!(target, LinkTarget::Reference(_)))
    );
    for id in 0..doc.links().len() {
        assert_eq!(doc.link_destination(mdstream::LinkId(id)), None);
    }
    let update = doc.append("[guide]: <https://first>\n").unwrap();
    assert_eq!(update.references_changed.len(), 1);
    assert_ne!(key, doc.tile_key(TileId(0)).unwrap());
    for id in 0..doc.links().len() {
        assert_eq!(
            doc.link_destination(mdstream::LinkId(id)),
            Some("https://first")
        );
    }
    assert!(
        doc.append("[GUIDE]: https://second\n")
            .unwrap()
            .references_changed
            .is_empty()
    );
    assert_eq!(
        doc.link_destination(mdstream::LinkId(0)),
        Some("https://first")
    );
    assert_eq!(visible(&doc), "Guide and name and Guide\n\n");
}

#[test]
fn unfinished_and_malformed_constructs_do_not_reparse() {
    let doc =
        parse("***literal*** a_b_c ``unclosed `code * x\n\n[x](bad target) [outer [inner]]\n");
    assert!(visible(&doc).starts_with("***literal*** a_b_c ``unclosed `code * x"));
    assert!(visible(&doc).contains("x(bad target) [outer inner]"));
}

#[test]
fn code_runs_must_match_exactly_and_cross_lines() {
    let doc = parse("a ``b ` c\nd`` e\n");
    assert_eq!(visible(&doc), "a b ` c\nd e\n");
    assert!(
        doc.runs_in(Span::new(0, doc.source().len()))
            .unwrap()
            .any(|r| r.text == "b ` c\nd" && r.style.contains(Style::CODE))
    );
}

#[test]
fn utf8_source_tiles_and_provisional_tail_are_immediately_readable() {
    let source = "中文🙂e\u{301}".repeat(1000);
    let mut doc = Document::new();
    for c in source.chars() {
        doc.append(c.encode_utf8(&mut [0; 4])).unwrap();
    }
    let mut recovered = String::new();
    for i in 0..doc.tile_count() {
        let span = doc.tile(TileId(i)).unwrap();
        assert!(span.len() <= TILE_BYTES);
        recovered.push_str(doc.text(span).unwrap());
    }
    assert_eq!(recovered, source);
    assert_eq!(visible(&doc), source);
    assert!(doc.pending().is_empty());
    let mut short = Document::new();
    short.append("**尚未闭合").unwrap();
    assert_eq!(visible(&short), "**尚未闭合");
    assert_eq!(short.block_at(0), None);
    short.finish();
    assert_eq!(short.block_at(0), Some(mdstream::BlockId(0)));
}

#[test]
fn finish_is_idempotent_and_append_after_finish_fails() {
    let mut doc = Document::new();
    assert!(doc.append("").unwrap().dirty.is_empty());
    doc.append("**ok**").unwrap();
    doc.finish();
    assert_eq!(visible(&doc), "ok");
    let work = doc.work();
    assert!(doc.finish().dirty.is_empty());
    assert_eq!(doc.work(), work);
    assert!(doc.append("no").is_err());
    let mut empty = Document::new();
    empty.finish();
    assert!(empty.blocks().is_empty());
    assert_eq!(empty.tile_count(), 0);
}

#[test]
fn boundaries_and_invalid_ranges_are_checked() {
    let doc = parse("é🙂");
    assert!(doc.runs_in(Span::new(1, 2)).is_none());
    assert!(doc.runs_in(Span::new(5, 2)).is_none());
    assert!(doc.runs(TileId(999)).is_none());
    assert!(doc.tile_at(doc.source().len()).is_none());
    assert_eq!(doc.tiles_overlapping(Span::new(0, 0)), 0..0);
    assert_eq!(doc.tiles_overlapping(Span::new(0, 1)), 0..1);
}
