use mdstream::{BlockKind, Document, HeadingId, LinkId, Span, Style, TileId};

#[derive(Debug, PartialEq, Eq)]
struct SemanticRun {
    source: Span,
    style: Style,
    block: Option<BlockKind>,
    destination: Option<String>,
    title: Option<String>,
}

fn semantics(doc: &Document, tile: TileId) -> Vec<SemanticRun> {
    doc.runs(tile)
        .unwrap()
        .map(|run| SemanticRun {
            source: run.source,
            style: run.style,
            block: run.block.map(|id| doc.block(id).unwrap().kind.clone()),
            destination: run
                .link
                .and_then(|id| doc.link_destination(id))
                .map(str::to_owned),
            title: run
                .link
                .and_then(|id| doc.link_title(id))
                .map(str::to_owned),
        })
        .collect()
}

#[test]
fn unchanged_tile_keys_never_hide_changed_semantics() {
    let source = format!(
        "**{}**\n\n[x][key] ![图](<img path> 'title')\n\n| a | b |\n|:--|--:|\n| {} | y |\n\n{}  \r\ncontinuation\n\n[key]: /target 'late title'\n",
        "a".repeat(1600),
        "b".repeat(1300),
        "z".repeat(1500)
    );
    let mut doc = Document::new();
    let mut cache = Vec::new();
    let mut start = 0;
    for end in source
        .char_indices()
        .enumerate()
        .filter_map(|(i, (offset, _))| (i % 97 == 0 && offset > 0).then_some(offset))
        .chain(std::iter::once(source.len()))
    {
        doc.append(&source[start..end]).unwrap();
        start = end;
        for (i, (key, old)) in cache.iter().enumerate() {
            if doc.tile_key(TileId(i)).as_ref() == Some(key) {
                assert_eq!(&semantics(&doc, TileId(i)), old);
            }
        }
        cache = (0..doc.tile_count())
            .map(|i| (doc.tile_key(TileId(i)).unwrap(), semantics(&doc, TileId(i))))
            .collect();
    }
    doc.finish();
    for (i, (key, old)) in cache.iter().enumerate() {
        if doc.tile_key(TileId(i)).as_ref() == Some(key) {
            assert_eq!(&semantics(&doc, TileId(i)), old);
        }
    }
}

#[test]
fn source_indexes_match_linear_oracles_including_empty_alt_images() {
    let source =
        "# one\n## two\n[a](u 't') ![](img) [b][ref]\n\nthree\n===\n\n[ref]: v\n".repeat(40);
    let mut doc = Document::new();
    doc.append(&source).unwrap();
    doc.finish();
    for start in (0..source.len()).step_by(7) {
        let end = (start + 53).min(source.len());
        let actual: Vec<_> = doc
            .links_in(Span::new(start, end))
            .map(|(id, _)| id)
            .collect();
        let expected: Vec<_> = (0..doc.links().len())
            .map(LinkId)
            .filter(|&id| {
                let info = doc.link_info(id).unwrap();
                info.source.start < end && start < info.source.end
            })
            .collect();
        assert_eq!(actual, expected);
        for level in 0..=7 {
            let eligible = || {
                doc.headings()
                    .entries()
                    .iter()
                    .enumerate()
                    .filter(|(_, h)| h.level <= level)
            };
            let next = eligible()
                .find(|(_, h)| h.source.start > start)
                .map(|(i, _)| HeadingId(i));
            let previous = eligible()
                .rev()
                .find(|(_, h)| h.source.start < start)
                .map(|(i, _)| HeadingId(i));
            assert_eq!(doc.headings().next(start, level), next);
            assert_eq!(doc.headings().previous(start, level), previous);
        }
    }
}
