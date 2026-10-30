use mdtui3::{BLOCK_LOOKAHEAD, Block, Document, LinkTarget, Reference, Span, Style, TileId};

#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    source: String,
    blocks: Vec<Block>,
    containers: Vec<mdtui3::Container>,
    tiles: Vec<Span>,
    references: Vec<Reference>,
    links: Vec<LinkTarget>,
    link_info: Vec<mdtui3::LinkInfo>,
    headings: Vec<mdtui3::Heading>,
    tables: Vec<mdtui3::Table>,
    maths: Vec<mdtui3::Math>,
    footnotes: Vec<mdtui3::Footnote>,
    footnote_references: Vec<mdtui3::FootnoteReference>,
    runs: Vec<(Span, Option<mdtui3::BlockId>, Style, Option<mdtui3::LinkId>)>,
    display: Vec<(Span, Option<Span>, String)>,
}

fn snapshot(doc: &Document) -> Snapshot {
    Snapshot {
        source: doc.source().to_owned(),
        blocks: doc.blocks().to_vec(),
        containers: (0..doc.container_count())
            .map(|i| doc.container(mdtui3::ContainerId(i)).unwrap())
            .collect(),
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
        maths: doc.maths().to_vec(),
        footnotes: doc.footnotes().to_vec(),
        footnote_references: doc.footnote_references().to_vec(),
        runs: doc
            .runs_in(Span::new(0, doc.source().len()))
            .unwrap()
            .map(|r| (r.source, r.block, r.style, r.link))
            .collect(),
        display: doc
            .display_runs_in(Span::new(0, doc.source().len()))
            .unwrap()
            .map(|r| (r.source, r.replacement, r.text.to_owned()))
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
        "[^中] [^a] [^中]\r\n\n[^a]: first\n[^中]: **raw**\n[^a]: duplicate\n",
        "> [^x]\n| [^x] | b |\n| -- | -- |\n| [^y] | [^x] |\n\n[^x]: body\n",
        "[^a b] [^] [^a\\b] [^open\n\n[^last]",
        "[x](a(b)c) [y][z]\n\n[z]: url\n",
        "```rs\r\nx\r\n```\r\n",
        "# title #\n\ntext\n===\n",
        "\\*a* `b` 中文🙂\n",
        "a\r\nb\n",
        "[a](x\\)y)\n",
        "| a | b |\r\n|:---|---:|\r\n| **x** | y\\|z |\r\n",
        "intro\na | b\n-- | --\nx | y\n",
        "- [ ] todo\n- [X] done\n![alt](img) ![x][ref]\n",
        "> - [x] task\r\n>   > nested\n>   continuation\n> - sibling\n\nplain\n",
        "$x$ $$中🙂 \\alpha$$ $$$literal$$$ $a\\$b$ $open\n\n$next$\n",
        "| $h$ | h2 |\n| -- | -- |\n| $a | b$ |\n",
    ] {
        for (i, _) in source.char_indices() {
            check_chunks(source, &[i]);
        }
    }
}

#[test]
fn every_inline_split_after_lookahead_exercises_live_parser_states() {
    for suffix in [
        "[^中] [^a] [^unclosed",
        "**中🙂** [a](<a b> \"t\\\"t\")\r\nnext\n",
        "![alt](a(b) 'title') [b][key]\n\n[key]: /u (title)\n",
        "a  \r\nb\\\r\nc\\\\\nd\n",
        "[a](bad target) `a  \nb`\n",
        "$x$ $$中🙂 \\alpha$$ $a$$b$ $$$literal$$$ $unclosed",
    ] {
        let prefix = "x".repeat(BLOCK_LOOKAHEAD);
        let source = format!("{prefix}{suffix}");
        for (offset, _) in suffix.char_indices() {
            check_chunks(&source, &[prefix.len() + offset]);
        }
        check_chunks(
            &source,
            &source.char_indices().map(|(i, _)| i).collect::<Vec<_>>(),
        );
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
        "é", "🙂", "|", "!", "<", ">", "\"", "'", "\t", "&", "$", "^",
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

#[test]
fn provisional_semantics_match_at_every_observed_prefix() {
    let parts = [
        "**a _b_ c** ",
        "[^中] [^missing]\n\n[^中]: raw **body**\r\n",
        "[x](<a b> 'title') ",
        "![图][key] ",
        "  \r\n",
        "continued\\\n",
        "[bad](<bad<target>)\n",
        "\n",
        "| a | b |\n|:--|--:|\n| `x\\|y` | **z** |\n",
        "# heading\n",
        "title\n---\n",
        "[key]: <a b> \"说明\"\n",
        "`unfinished ",
        "&amp; &#x1f642; &NotEqualTilde; &bad*ok*; ",
        "` a\r\nb ` ",
        "$x$ $$中🙂 &amp; \\alpha$$ $a$$b$ $unclosed\n\n",
        "> - item\n>   > quote\n> - sibling\n  - nested list\n    continuation\n",
    ];
    let mut state = 981_u64;
    for _ in 0..24 {
        let mut source = "x".repeat(BLOCK_LOOKAHEAD);
        for _ in 0..16 {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            source.push_str(parts[(state >> 32) as usize % parts.len()]);
        }
        let mut incremental = Document::new();
        incremental.append(&source[..BLOCK_LOOKAHEAD]).unwrap();
        let mut start = BLOCK_LOOKAHEAD;
        for (offset, c) in source[BLOCK_LOOKAHEAD..].char_indices() {
            let end = BLOCK_LOOKAHEAD + offset + c.len_utf8();
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            if !state.is_multiple_of(7) && end < source.len() {
                continue;
            }
            incremental.append(&source[start..end]).unwrap();
            let mut whole = Document::new();
            whole.append(&source[..end]).unwrap();
            assert_eq!(
                snapshot(&incremental),
                snapshot(&whole),
                "prefix {:?}",
                &source[..end]
            );
            start = end;
        }
    }
}
