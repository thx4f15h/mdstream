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
        "> > **中\r\n> _🙂_**\\\r\nlazy\n> # end\n",
        "> - [x] ` 中\n> 🙂\nend`\n- next\n",
        "> *a|b\n|--|--|\nend*\n> title\n===\n> ---\n",
        "> *a\n[r]: <bad<target>\nend*\n[^n]: raw\n",
        "- a\n  - b\n  lazy\n  # parent\n2. sibling\n\nroot\n",
        "**a _b_ c**",
        "> - [x] **中\r\n>   _🙂_**\\\r\n>   next\n> - sibling\n",
        "> ` 中\r\n> 🙂\r\n> ` &amp;\n> $open\n> end$ [open\n> label](u)\n",
        "> *one\n> two*|three\n> ---\n> *a|b\n> c*|d\n> -|-\n",
        "> *a\n> > b*\n> c*\n>\n> **d\n> e**\n",
        "[r] [^中]\n> - [x] [r]: <a b> '说明'\r\n>   [^中]: **raw**\r\n> - [r]: /duplicate\n[^中]: duplicate\n",
        "> [ ]: /bad\n> ---\n> [bad]: <bad<target>\n> [^bad label]: raw\n> [^e]:\n> [^open",
        "> h|h\n> -|-\n> [r]: /a|b\n> [^f]: a|b\n> [r] [^f]\n",
        "- [^a]: a\n  [r]: /r\n  > [^b]: b\n- [^a]: duplicate\n\n[r] [^a] [^b]\n",
        "[^中] [^a] [^中]\r\n\n[^a]: first\n[^中]: **raw**\n[^a]: duplicate\n",
        "> [^x]\n| [^x] | b |\n| -- | -- |\n| [^y] | [^x] |\n\n[^x]: body\n",
        "[^a b] [^] [^a\\b] [^open\n\n[^last]",
        "[x](a(b)c) [y][z]\n\n[z]: url\n",
        "```rs\r\nx\r\n```\r\n",
        "> - [x] ```rs\r\n>   > # *中🙂*\r\n>   ```\n>       $raw$\n> - ~~~\n>   [^x]: raw\nout\n",
        "> ```\n> - literal\n> > literal\n>\n> ````\n>     code\n>\n>     next\n",
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
fn container_code_character_chunks_cross_classification_and_crlf_boundaries() {
    let source = format!(
        "> - ```rs {}\r\n>   > {}\r\n>   ```{}\n>   ```\n>       {}\r\n> - next\n",
        "中🙂".repeat(300),
        "*中🙂* &amp; ".repeat(400),
        " ".repeat(BLOCK_LOOKAHEAD),
        "`raw` ".repeat(300),
    );
    check_chunks(
        &source,
        &source.char_indices().map(|(i, _)| i).collect::<Vec<_>>(),
    );
}

#[test]
fn container_inline_long_lines_match_every_live_prefix_and_bytewise_completion() {
    for delimiter in ["**", "`", "$$"] {
        let prefix = format!("> - {delimiter}{}", "中🙂".repeat(150));
        let tail = format!("\r\n>   next{delimiter}\\\r\n>   end\n> - *a\n>   b*|c\n>   ---\n");
        let source = format!("{prefix}{tail}");
        let mut streamed = Document::new();
        streamed.append(&prefix).unwrap();
        let mut start = prefix.len();
        for (offset, c) in tail.char_indices() {
            let end = prefix.len() + offset + c.len_utf8();
            streamed.append(&source[start..end]).unwrap();
            let mut whole = Document::new();
            whole.append(&source[..end]).unwrap();
            assert_eq!(snapshot(&streamed), snapshot(&whole), "prefix {end}");
            start = end;
        }
        check_chunks(
            &source,
            &source.char_indices().map(|(i, _)| i).collect::<Vec<_>>(),
        );
    }
}

#[test]
fn lazy_continuations_match_every_live_prefix_across_lookahead_and_eof() {
    let source = format!(
        "> - > **中\r\n{}\r\n> end**\n> [r]: /{}\nlast\n# outside\n> > `a\n> b\n c`",
        "🙂".repeat(270),
        "x".repeat(1100),
    );
    let mut streamed = Document::new();
    for (offset, c) in source.char_indices() {
        let end = offset + c.len_utf8();
        streamed.append(&source[offset..end]).unwrap();
        let mut whole = Document::new();
        whole.append(&source[..end]).unwrap();
        assert_eq!(snapshot(&streamed), snapshot(&whole), "prefix {end}");
    }
    streamed.finish();
    let mut whole = Document::new();
    whole.append(&source).unwrap();
    whole.finish();
    assert_eq!(snapshot(&streamed), snapshot(&whole));
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
        "> > **中\r\n> 🙂**\nmissing\n> # end\n",
        "> - *a|b\n|--|--|\nend*\n# root\n",
        "**a _b_ c** ",
        "> **a\n> b**\r\n> ` 中\r\n> 🙂\r\n> `\n",
        "> *a\n> b*|c\n> ---\n> [open\n> label](u)\n",
        "> a\\\r\n> b\n> $open\n> end$\n>\n",
        "> - [x] [key]: /late 'title'\n>   [^中]: raw body\r\n> - [^中]: duplicate\n",
        "> [ ]: /bad\n> ---\n> [^open\n",
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
        "> - ```rs\n>   # raw\n>   > raw\n>   ```\n>       code\n> - next\n",
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

#[test]
fn container_definition_streaming_snapshots_match_every_prefix_and_final_state() {
    let source = format!(
        "[r] [^中]\n> - [r]: <a b> '说明'\r\n>   [^中]: {}\r\n> - [^中]: duplicate {}\r\n> [r]: /ignored\n> [^last]: tail",
        "中🙂".repeat(150),
        "x".repeat(1024),
    );
    let mut streamed = Document::new();
    for (offset, c) in source.char_indices() {
        let end = offset + c.len_utf8();
        streamed.append(&source[offset..end]).unwrap();
        let mut whole = Document::new();
        whole.append(&source[..end]).unwrap();
        assert_eq!(snapshot(&streamed), snapshot(&whole), "prefix {end}");
    }
    streamed.finish();
    let mut whole = Document::new();
    whole.append(&source).unwrap();
    whole.finish();
    assert_eq!(snapshot(&streamed), snapshot(&whole));
}
