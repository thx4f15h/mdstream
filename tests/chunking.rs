use mdtui3::{BLOCK_LOOKAHEAD, Block, Document, LinkTarget, Reference, Span, Style, TileId};

#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    source: String,
    blocks: Vec<Block>,
    tiles: Vec<Span>,
    references: Vec<Reference>,
    links: Vec<LinkTarget>,
    link_info: Vec<mdtui3::LinkInfo>,
    headings: Vec<mdtui3::Heading>,
    tables: Vec<mdtui3::Table>,
    runs: Vec<(Span, Option<mdtui3::BlockId>, Style, Option<mdtui3::LinkId>)>,
}

fn snapshot(doc: &Document) -> Snapshot {
    Snapshot {
        source: doc.source().to_owned(),
        blocks: doc.blocks().to_vec(),
        tiles: (0..doc.tile_count())
            .map(|i| doc.tile(TileId(i)).unwrap())
            .collect(),
        references: doc.references().to_vec(),
        links: doc.links().to_vec(),
        link_info: (0..doc.links().len())
            .map(|i| *doc.link_info(mdtui3::LinkId(i)).unwrap())
            .collect(),
        headings: doc.headings().entries().to_vec(),
        tables: doc.tables().to_vec(),
        runs: doc
            .runs_in(Span::new(0, doc.source().len()))
            .unwrap()
            .map(|r| (r.source, r.block, r.style, r.link))
            .collect(),
    }
}

fn check_chunks(source: &str, boundaries: &[usize]) {
    let mut whole = Document::new();
    whole.append(source).unwrap();
    whole.finish();
    let mut split = Document::new();
    let mut start = 0;
    for &end in boundaries {
        split.append(&source[start..end]).unwrap();
        assert!(split.pending().len() <= BLOCK_LOOKAHEAD + 3);
        start = end;
    }
    split.append(&source[start..]).unwrap();
    split.finish();
    assert_eq!(snapshot(&whole), snapshot(&split), "source {source:?}");
}

#[test]
fn every_single_split_for_small_constructs() {
    for source in [
        "**a _b_ c**",
        "[x](a(b)c) [y][z]\n\n[z]: url\n",
        "```rs\r\nx\r\n```\r\n",
        "# title #\n\ntext\n===\n",
        "\\*a* `b` 中文🙂\n",
        "a\r\nb\n",
        "[a](x\\)y)\n",
        "| a | b |\r\n|:---|---:|\r\n| **x** | y\\|z |\r\n",
        "intro\na | b\n-- | --\nx | y\n",
        "- [ ] todo\n- [X] done\n![alt](img) ![x][ref]\n",
    ] {
        for (i, _) in source.char_indices() {
            check_chunks(source, &[i]);
        }
    }
}

#[test]
fn character_chunks_through_lookahead_and_fence_boundaries() {
    let source = format!(
        "**{}**\n\n```rust {}\n{}\n```\n{}\r\n[x]: url\n",
        "中🙂".repeat(400),
        "x".repeat(1500),
        "x".repeat(5000),
        "[x] ".repeat(1000)
    );
    check_chunks(
        &source,
        &source.char_indices().map(|(i, _)| i).collect::<Vec<_>>(),
    );
}

#[test]
fn streaming_unicode_table_rows_match_single_chunk_input() {
    let source = format!(
        "intro\n| h1 | h2 |\n| :-- | --: |\n| {} | ![图](img) |\r\n| `x\\|y` | [link][key] |\n\n[key]: /target\n",
        "中文🙂 ".repeat(500)
    );
    check_chunks(
        &source,
        &source.char_indices().map(|(i, _)| i).collect::<Vec<_>>(),
    );
}

#[test]
fn deterministic_fuzz_chunk_invariance_and_source_coverage() {
    let alphabet = [
        "a", " ", "\n", "\r", "*", "_", "`", "~", "[", "]", "(", ")", "\\", ":", "#", ">", "-",
        "é", "🙂", "|", "!",
    ];
    let mut state = 0x1234_5678_u64;
    let mut random = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state as usize
    };
    for _ in 0..200 {
        let mut source = String::new();
        for _ in 0..300 {
            source.push_str(alphabet[random() % alphabet.len()]);
        }
        let boundaries: Vec<_> = source
            .char_indices()
            .filter_map(|(i, _)| (random() % 4 == 0).then_some(i))
            .collect();
        check_chunks(&source, &boundaries);
        let mut doc = Document::new();
        doc.append(&source).unwrap();
        doc.finish();
        let mut cursor = 0;
        for block in doc.blocks() {
            assert_eq!(cursor, block.source.start);
            assert!(doc.text(block.source).is_some());
            cursor = block.source.end;
        }
        assert_eq!(cursor, source.len());
    }
}
