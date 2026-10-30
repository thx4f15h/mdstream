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
    math_payloads: Vec<Vec<(Span, String)>>,
    math_block_payloads: Vec<Vec<(Span, String)>>,
    footnotes: Vec<mdtui3::Footnote>,
    footnote_payloads: Vec<Vec<(Span, String)>>,
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
        math_block_payloads: doc
            .blocks()
            .iter()
            .enumerate()
            .filter_map(|(i, _)| {
                let mut cursor = doc.math_block_payload(mdtui3::BlockId(i))?;
                let mut budget = mdtui3::Budget::new(mdtui3::WorkLimit::unlimited());
                let mut chunks = Vec::new();
                while let Some(chunk) = cursor.advance(&mut budget).unwrap() {
                    chunks.push((chunk.source, chunk.text.to_owned()));
                }
                Some(chunks)
            })
            .collect(),
        math_payloads: (0..doc.maths().len())
            .map(|i| {
                let mut cursor = doc.math_payload(mdtui3::MathId(i)).unwrap();
                let mut budget = mdtui3::Budget::new(mdtui3::WorkLimit::unlimited());
                let mut chunks = Vec::new();
                while let Some(chunk) = cursor.advance(&mut budget).unwrap() {
                    chunks.push((chunk.source, chunk.text.to_owned()));
                }
                chunks
            })
            .collect(),
        footnotes: doc.footnotes().to_vec(),
        footnote_payloads: (0..doc.footnotes().len())
            .filter_map(|i| {
                let mut cursor = doc.footnote_payload(mdtui3::FootnoteId(i))?;
                let mut budget = mdtui3::Budget::new(mdtui3::WorkLimit::unlimited());
                let mut chunks = Vec::new();
                while let Some(chunk) = cursor.advance(&mut budget).unwrap() {
                    chunks.push((chunk.source, chunk.text.to_owned()));
                }
                Some(chunks)
            })
            .collect(),
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
        "- ```\r\n\t中🙂\r\n\t ```\r\n- sibling\n",
        "> ```\n>\tpayload\n>\t  ```\n>\t```\n",
        "- - - ~~~\n\t\tpayload\n\t\t~~~\n",
        "- > ```\n \t>\tpayload\n\twrong scope\n",
        "-\t```\r\n\t中🙂\r\n\t```\n-\t[x]\t# task\n",
        "> \t-\tfirst\n> \t\t# title\n> \t-\tlast\n",
        "- first\n\t# lazy\n\t- literal\n- ```\n\toutside\n",
        "  >\t# title\n  >\t*中\n  >\t🙂*\n  >\t===\n",
        "-\t[^n]: a\n\t    b\n\t\n\t$$\n\tx\n\t$$\n\th|h\n\t-|-\n",
        "[^n]\n[^n]: 中\r\n    🙂&amp;\r\n\r\n    last\r\nafter\n",
        "> - [^n]: first\n>       > # raw\n>   \n>       last\n> - [^n]: duplicate\n>       ignored\n",
        "- [^n]:\n  \n      second\n  \n  tail\n- next\n",
        "> > [^n]: first\n> >     second\n>     parent\n> > [^m]: child\n    outside\n",
        "[^n]: first\n   short\n[^n]: duplicate\n    [^m]: raw\n[^m]:real",
        "[^n]:\n    last\r",
        "$$\r\n中🙂 &amp;\\alpha\r\n\n$$\r\nafter\n",
        "> - $$\n>   中\r\n>   > - literal\n>   \n>   $$\n> out\n",
        "> > $$\n> > inner\n> $$\n> outer\n> $$",
        "h|h\n$$\nbody\n$$\n\n> lazy\n$$\nroot\n$$ suffix\n",
        "> - $$中 &amp;\\\r\n>   🙂$$\n>\n> $a\n> > b$\n",
        "> $a\n> b$\n> ---\n> $next\nlazy$\n",
        "> $$a\n> b|$c$\n> -|-\n> $a\\|b$|$ c $\n",
        "> - ![中&amp;\r\n> 🙂 `a\n b`][r]\n>   [r]: /u 't'\n",
        "> [a\n> b][] ![x\n> y](u)\n> [a b]: /late\n",
        "> [open\n> > nested](u)\n> [a\n> b|c\n> -|-\n",
        "> [a\\]\n> `]` b](bad target)\n> [^a\n> b]\n",
        "- **中**\r\n  \r\n- 🙂\r\n",
        "> - a\n>   \n>   - b\n>     \n>     c\n> - d\n",
        "- ```\n  \n  ```\n  \n  tail\n",
        "- [x] *中\r\n\t🙂* &amp; [a\n\tb][]\n\n[a b]: /u\n",
        "- ` a\n\t`\n\n1. ` a\n\t`\n\n   > ` a\n   >\t`\n",
        "- - *a\n\tb*\n\t# heading\n\n- > a\n\tmissing\n",
        "- a\n\tb|c\n  -|-\n- $a\n\tb$\n- a\n\t\n\tlast",
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
fn long_table_delimiters_match_all_splits_live_prefixes_and_eof() {
    for prefix in ["", "> - ", "> "] {
        let continuation = if prefix == "> - " { ">   " } else { prefix };
        for tail in ["|-:\r\n", "|-🙂\r\n", "|", "|-\n> out\n"] {
            let source = format!(
                "{prefix}**中**|🙂\n{continuation}{}{}",
                "-".repeat(1030),
                tail
            );
            for (at, _) in source.char_indices() {
                check_chunks(&source, &[at]);
            }
            let mut streamed = Document::new();
            for (at, c) in source.char_indices() {
                let end = at + c.len_utf8();
                streamed.append(&source[at..end]).unwrap();
                let mut whole = Document::new();
                whole.append(&source[..end]).unwrap();
                assert_eq!(snapshot(&streamed), snapshot(&whole), "prefix {end}");
                assert!(streamed.pending().len() <= BLOCK_LOOKAHEAD + 3);
            }
            streamed.finish();
            let mut whole = Document::new();
            whole.append(&source).unwrap();
            whole.finish();
            assert_eq!(snapshot(&streamed), snapshot(&whole));
        }
    }
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
fn long_headers_match_utf8_splits_live_prefixes_confirmation_and_fallback() {
    for source in [
        format!(
            "> - *prior\n>   end*|{} `x\\|y` [x](u) $m$ [^n]\r\n>   |-|-\r\n>   v|z\n",
            "中🙂".repeat(160)
        ),
        format!(
            "*prior\nend*|{}\n|{}|-\n",
            "x".repeat(1030),
            "-".repeat(1030)
        ),
        format!(
            "> *prior\n> end*|{}\r\n> wrong\n> ---\n",
            "中🙂".repeat(160)
        ),
        format!("> |{}\r\n", " ".repeat(1030)),
        format!("|{}\\|tail|last\r\n|-|-", "x".repeat(1022)),
        format!("intro\n|**{}**|b", "中🙂".repeat(160)),
    ] {
        for (at, _) in source.char_indices() {
            check_chunks(&source, &[at]);
        }
        let mut live = Document::new();
        for (at, c) in source.char_indices() {
            let end = at + c.len_utf8();
            live.append(&source[at..end]).unwrap();
            let mut whole = Document::new();
            whole.append(&source[..end]).unwrap();
            assert_eq!(snapshot(&live), snapshot(&whole), "prefix {end}");
            assert!(live.pending().len() <= BLOCK_LOOKAHEAD + 3);
        }
        live.finish();
        let mut whole = Document::new();
        whole.append(&source).unwrap();
        whole.finish();
        assert_eq!(snapshot(&live), snapshot(&whole));
    }
}

#[test]
fn inherited_headers_match_all_utf8_splits_and_live_prefix_snapshots() {
    for source in [
        format!(
            "prior  \r\n**中** [h](u) {}|end\r\n-|-\n",
            "中🙂".repeat(150)
        ),
        format!(
            "> - [x] prior\\\r\n>   `中` {}|b\r\n>   -|-\r\n",
            "x".repeat(1030)
        ),
        format!("> prior\n> *{}|end*\r\n> wrong\n> ---\n", "x".repeat(1030)),
        format!("prior\n{}\n===\n", "x".repeat(1030)),
        format!("> prior\n> {}\n> next\n", "x".repeat(1030)),
        format!("prior\n{}|b", "x".repeat(1030)),
        format!("prior\n{}", "x".repeat(1030)),
        format!(
            "> prior  \n> {}|b\n> {}x|tail\n",
            "x".repeat(1030),
            "-".repeat(1030)
        ),
        format!("[open\nend](u) {}|b\n-|-\n", "x".repeat(1030)),
        format!("> prior\n> {}\\|escaped|b\n> -|-\n", "x".repeat(1021)),
    ] {
        for (at, _) in source.char_indices() {
            check_chunks(&source, &[at]);
        }
        let mut live = Document::new();
        for (at, c) in source.char_indices() {
            let end = at + c.len_utf8();
            live.append(&source[at..end]).unwrap();
            let mut whole = Document::new();
            whole.append(&source[..end]).unwrap();
            assert_eq!(snapshot(&live), snapshot(&whole), "prefix {end}");
            assert!(live.pending().len() <= BLOCK_LOOKAHEAD + 3);
        }
        live.finish();
        let mut whole = Document::new();
        whole.append(&source).unwrap();
        whole.finish();
        assert_eq!(snapshot(&live), snapshot(&whole));
    }
}

#[test]
fn container_labels_match_all_utf8_splits_and_live_prefixes() {
    for source in [
        format!("> - ![中&amp;\r\n>   {}🙂](u 'title')\n", "x".repeat(1030)),
        format!("> [a\n> {}][]\n", "中".repeat(333)),
        format!("> ![old\n> {}|end](u)\n> -|-\n", "x".repeat(1030)),
        format!(
            "> [old `\n> {}|end](u)\n> wrong\n> end`](u)\n",
            "x".repeat(1030)
        ),
        format!(
            "> ![old\n> {}|b\n> {}x|end](u)\n",
            "x".repeat(1030),
            "-".repeat(1030)
        ),
        "> - [a\r\n> b\nc][]\n>   [a b c]: /late\n".to_owned(),
    ] {
        for (at, _) in source.char_indices() {
            check_chunks(&source, &[at]);
        }
        let mut live = Document::new();
        for (at, c) in source.char_indices() {
            let end = at + c.len_utf8();
            live.append(&source[at..end]).unwrap();
            let mut whole = Document::new();
            whole.append(&source[..end]).unwrap();
            assert_eq!(snapshot(&live), snapshot(&whole), "prefix {end}");
        }
        live.finish();
        let mut whole = Document::new();
        whole.append(&source).unwrap();
        whole.finish();
        assert_eq!(snapshot(&live), snapshot(&whole));
    }
}

#[test]
fn link_headers_match_all_utf8_splits_and_live_prefix_snapshots() {
    for source in [
        format!(
            "[p](u) *old ![中  \r\n**🙂** &amp; {}|end](v)\r\n-|-\n",
            "x".repeat(1030)
        ),
        format!("[old\n{}|end](v)\r\nwrong\n", "x".repeat(1030)),
        format!(
            "![old\n{}](v) [new {}|b\n-|-\n",
            "x".repeat(1030),
            "y".repeat(20)
        ),
        format!("[old\n{}[local](v 'title')|b\n-|-\n", "x".repeat(1030)),
        format!("![old\n{}![local|](v)\r\n-|-\n", "x".repeat(1030)),
        format!("[old\n{}\\]|b\n-|-\n", "x".repeat(1030)),
        format!("[old\n{}\\\\]|b\n-|-\n", "x".repeat(1030)),
        format!("[old `\n{}|b](v)\n-|-\n", "x".repeat(1030)),
        format!("![old $$\n{}|b](v)\nwrong\nend$$\n", "x".repeat(1030)),
        format!(
            "![old\n{}|b\n{}x|end](v)\n",
            "x".repeat(1030),
            "-".repeat(1030)
        ),
        format!("![old\n{}\n===\n", "x".repeat(1030)),
        format!("![old\n{}|b", "x".repeat(1030)),
        format!("![old\n{}", "x".repeat(1030)),
        format!(
            "> ![old\n> end](u) [local](v) {}|b\n> -|-\n",
            "x".repeat(1030)
        ),
    ] {
        for (at, _) in source.char_indices() {
            check_chunks(&source, &[at]);
        }
        let mut live = Document::new();
        for (at, c) in source.char_indices() {
            let end = at + c.len_utf8();
            live.append(&source[at..end]).unwrap();
            let mut whole = Document::new();
            whole.append(&source[..end]).unwrap();
            assert_eq!(snapshot(&live), snapshot(&whole), "prefix {end}");
            assert!(live.pending().len() <= BLOCK_LOOKAHEAD + 3);
        }
        live.finish();
        let mut whole = Document::new();
        whole.append(&source).unwrap();
        whole.finish();
        assert_eq!(snapshot(&live), snapshot(&whole));
    }
}

#[test]
fn math_headers_match_all_utf8_splits_and_live_prefix_snapshots() {
    for source in [
        format!(
            "$p$ *old $$ 中\\\r\n$中🙂$ &amp; [h](u) {}|$tail$\r\n-|-\r\n",
            "中🙂".repeat(150)
        ),
        format!("a $ prior\n{} $|b\r\n-|-\n", "x".repeat(1030)),
        format!("a $$ prior\n{} $$|b\r\nwrong\n---\n", "x".repeat(1030)),
        format!("a $ prior\n{} $ *new $again |b\n-|-\n", "x".repeat(1030)),
        format!("a $ prior\n{}\n===\n", "x".repeat(1030)),
        format!("a $$ prior\n{}|b", "x".repeat(1030)),
        format!("a $ prior\n{}", "x".repeat(1030)),
        format!(
            "a $$ prior\n{}|b\n{}x|end$$\n",
            "x".repeat(1030),
            "-".repeat(1030)
        ),
        format!("a $$ prior\n{}\\$\\\\|$tail$\r\n-|-\n", "x".repeat(1030)),
        format!("> a $$ prior\n> $local$ {}|b\n> -|-\n", "x".repeat(1030)),
        format!("a $$ prior\n{}|b\n> -|-\n", "x".repeat(1030)),
    ] {
        for (at, _) in source.char_indices() {
            check_chunks(&source, &[at]);
        }
        let mut live = Document::new();
        for (at, c) in source.char_indices() {
            let end = at + c.len_utf8();
            live.append(&source[at..end]).unwrap();
            let mut whole = Document::new();
            whole.append(&source[..end]).unwrap();
            assert_eq!(snapshot(&live), snapshot(&whole), "prefix {end}");
            assert!(live.pending().len() <= BLOCK_LOOKAHEAD + 3);
        }
        live.finish();
        let mut whole = Document::new();
        whole.append(&source).unwrap();
        whole.finish();
        assert_eq!(snapshot(&live), snapshot(&whole));
    }
}

#[test]
fn code_headers_match_all_utf8_splits_and_live_prefix_snapshots() {
    for source in [
        format!(
            "> - a `` 中\r\n>   ` a\\|b ` &amp; [h](u) {}|b\r\n>   -|-\r\n",
            "中🙂".repeat(150)
        ),
        format!("a ` prior\n{} `|b\r\n-|-\n", "x".repeat(1030)),
        format!(
            "> a ` prior\n> {} `|b\r\n> wrong\n> ---\n",
            "x".repeat(1030)
        ),
        format!("a ` prior\n{} ` *new `again |b\n-|-\n", "x".repeat(1030)),
        format!("a ` prior\n{}\n===\n", "x".repeat(1030)),
        format!("> a ` prior\n> {}\n> next`\n", "x".repeat(1030)),
        format!("a ` prior\n{}|b", "x".repeat(1030)),
        format!("a ` prior\n{}", "x".repeat(1030)),
        format!(
            "> a ` prior\n> {}|b\n> {}x|end`\n",
            "x".repeat(1030),
            "-".repeat(1030)
        ),
        format!("> a ` prior\n> {}|b\n-| -\n", "x".repeat(1030)),
    ] {
        for (at, _) in source.char_indices() {
            check_chunks(&source, &[at]);
        }
        let mut live = Document::new();
        for (at, c) in source.char_indices() {
            let end = at + c.len_utf8();
            live.append(&source[at..end]).unwrap();
            let mut whole = Document::new();
            whole.append(&source[..end]).unwrap();
            assert_eq!(snapshot(&live), snapshot(&whole), "prefix {end}");
            assert!(live.pending().len() <= BLOCK_LOOKAHEAD + 3);
        }
        live.finish();
        let mut whole = Document::new();
        whole.append(&source).unwrap();
        whole.finish();
        assert_eq!(snapshot(&live), snapshot(&whole));
    }
}

#[test]
fn open_headers_match_all_utf8_splits_and_live_prefix_snapshots() {
    for source in [
        format!(
            "*prior  \r\n**中** [h](u) {}|end*\r\n-|-\n",
            "中🙂".repeat(150)
        ),
        format!(
            "> - [x] ~~prior\\\r\n>   `中` {}~~|b\r\n>   -|-\r\n",
            "x".repeat(1030)
        ),
        format!(
            "> *prior\n> **local {}|end*\r\n> wrong\n> ---\n",
            "x".repeat(1030)
        ),
        format!("**prior\n{}\n===\n", "x".repeat(1030)),
        format!("> *prior\n> {}\n> next*\n", "x".repeat(1030)),
        format!("*prior\n{}|b", "x".repeat(1030)),
        format!("*prior\n{}", "x".repeat(1030)),
        format!(
            "> *prior  \n> {}|b\n> {}x|end*\n",
            "x".repeat(1030),
            "-".repeat(1030)
        ),
        format!("> *prior\n> end* *new {}|b\n> -|-\n", "x".repeat(1030)),
        format!("> *prior\n> {}end* *new|b\r\n> -|-\n", "x".repeat(1030)),
        format!("> *prior\n> {}\\|escaped|b\n> -|-\n", "x".repeat(1021)),
        format!("> *prior\n> {}|end*\n-|-\n", "x".repeat(1030)),
    ] {
        for (at, _) in source.char_indices() {
            check_chunks(&source, &[at]);
        }
        let mut live = Document::new();
        for (at, c) in source.char_indices() {
            let end = at + c.len_utf8();
            live.append(&source[at..end]).unwrap();
            let mut whole = Document::new();
            whole.append(&source[..end]).unwrap();
            assert_eq!(snapshot(&live), snapshot(&whole), "prefix {end}");
            assert!(live.pending().len() <= BLOCK_LOOKAHEAD + 3);
        }
        live.finish();
        let mut whole = Document::new();
        whole.append(&source).unwrap();
        whole.finish();
        assert_eq!(snapshot(&live), snapshot(&whole));
    }
}

#[test]
fn late_headers_match_all_utf8_splits_and_live_prefix_snapshots() {
    for source in [
        format!(
            "> - [x] **中** [a](u) $m$ [^n] {}|`x\\|y`\r\n>   -|-\r\n>   v|z\n",
            "中🙂".repeat(150)
        ),
        format!("`{}|close`\n-|-", "x".repeat(1030)),
        format!("> *{}|end* &amp;\r\n> wrong\n> ---\n", "x".repeat(1030)),
        format!("{}\\|escaped|last\r\n-|-\n", "x".repeat(1023)),
        format!("> {}\\\\|last\n> -|-\n", "x".repeat(1021)),
        format!("> *{}\r\n> end*\n", "中🙂".repeat(150)),
        format!("> {}|b\n> {}x|tail\n", "x".repeat(1030), "-".repeat(1030)),
        format!("> *prior\n> end*{}|b\n> -|-\n", "x".repeat(1030)),
    ] {
        for (at, _) in source.char_indices() {
            check_chunks(&source, &[at]);
        }
        let mut live = Document::new();
        for (at, c) in source.char_indices() {
            let end = at + c.len_utf8();
            live.append(&source[at..end]).unwrap();
            let mut whole = Document::new();
            whole.append(&source[..end]).unwrap();
            assert_eq!(snapshot(&live), snapshot(&whole), "prefix {end}");
            assert!(live.pending().len() <= BLOCK_LOOKAHEAD + 3);
        }
        live.finish();
        let mut whole = Document::new();
        whole.append(&source).unwrap();
        whole.finish();
        assert_eq!(snapshot(&live), snapshot(&whole));
    }
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
        "\n$$\r\n中🙂 &amp;\r\n\n$$\r\n",
        "> - $$\n>   x\\alpha\n>   $$\n> outside\n",
        "> - $$中 &amp;\\\r\n>   🙂$$\n>\n",
        "> $a\n> b$\n> ---\n> $next\nlazy$\n",
        "> ![中&amp;\r\n> 🙂][ref]\n> [ref]: /late\n",
        "> [a\n> b][]\n> [a b]: /u\n",
        "- **中**\r\n  \r\n- 🙂\r\n",
        "> - a\n>   \n>   - b\n>     \n>     c\n> - d\n",
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

#[test]
fn container_math_matches_all_utf8_splits_and_live_prefixes() {
    for source in [
        format!("> - $$中 &amp;\r\n>   {}🙂$$\n", "x".repeat(1030)),
        format!("> $$old\n> {}|$local$\n> -|-\n", "x".repeat(1030)),
        format!("> $$old\n> {}|tail$$\n> wrong\n", "x".repeat(1030)),
        format!("> $old\n> {}$ closed|b\n> -|-\n", "x".repeat(1030)),
        format!(
            "> $old\n> {}|b\n> {}x|tail$\n",
            "x".repeat(1030),
            "-".repeat(1030)
        ),
        "> - $中\\\r\n> 🙂\nend$\n>   ---\n".to_owned(),
    ] {
        for (at, _) in source.char_indices() {
            check_chunks(&source, &[at]);
        }
        let mut live = Document::new();
        for (at, c) in source.char_indices() {
            let end = at + c.len_utf8();
            live.append(&source[at..end]).unwrap();
            let mut whole = Document::new();
            whole.append(&source[..end]).unwrap();
            assert_eq!(snapshot(&live), snapshot(&whole), "prefix {end}");
        }
        live.finish();
        let mut whole = Document::new();
        whole.append(&source).unwrap();
        whole.finish();
        assert_eq!(snapshot(&live), snapshot(&whole));
    }
}

#[test]
fn container_setext_matches_all_utf8_splits_and_live_prefixes() {
    for source in [
        "> - [x] **中\r\n>   🙂**\\\r\nlast [x](u)\n>   ===\r\n>   body\n>   # peer\n".to_owned(),
        "> first\n> [a\n> b](u) $c\n> d$ `e\n> f`\n> ===".to_owned(),
        "> first\n> two\n> > # nested\n> new\n> tail\n> ---\n".to_owned(),
        "> first\n> second\n> ==x\n> ===\n> # stop\n".to_owned(),
        format!("> first\n> {}|b\n> ===\n", "x".repeat(1030)),
        format!("> first\n> a|b\n> -|{}x\n> ===\n", "-".repeat(1030)),
        format!("> *first\n> {}*|b\n> ===\n", "x".repeat(1030)),
        format!("> first\n> {}\n> ===\n", "=".repeat(1030)),
    ] {
        for (at, _) in source.char_indices() {
            check_chunks(&source, &[at]);
        }
        let mut live = Document::new();
        for (at, c) in source.char_indices() {
            let end = at + c.len_utf8();
            live.append(&source[at..end]).unwrap();
            let mut whole = Document::new();
            whole.append(&source[..end]).unwrap();
            assert_eq!(snapshot(&live), snapshot(&whole), "prefix {end}");
        }
        live.finish();
        let mut whole = Document::new();
        whole.append(&source).unwrap();
        whole.finish();
        assert_eq!(snapshot(&live), snapshot(&whole));
    }
}

#[test]
fn math_blocks_match_all_utf8_splits_and_live_prefixes_at_classification_limit() {
    for ending in ["$$\n> tail\n", "$$ suffix\n> $$", "$$$\r\noutside\n", ""] {
        let source = format!(
            "> - $$\r\n>   {}\r\n>   $${}\n>   {ending}",
            "中🙂".repeat(150),
            " ".repeat(BLOCK_LOOKAHEAD)
        );
        for (at, _) in source.char_indices() {
            check_chunks(&source, &[at]);
        }
        let mut live = Document::new();
        for (at, c) in source.char_indices() {
            let end = at + c.len_utf8();
            live.append(&source[at..end]).unwrap();
            let mut whole = Document::new();
            whole.append(&source[..end]).unwrap();
            assert_eq!(snapshot(&live), snapshot(&whole), "prefix {end}");
        }
        live.finish();
        let mut whole = Document::new();
        whole.append(&source).unwrap();
        whole.finish();
        assert_eq!(snapshot(&live), snapshot(&whole));
    }
}

#[test]
fn footnote_blocks_match_all_utf8_splits_and_live_prefixes_at_classification_limit() {
    for ending in [">   \r\n>       last\r\n> - sibling\n", ">   tail\n", ""] {
        let source = format!(
            "[^n]\n> - [^n]: {}\r\n>       {}\r\n{ending}",
            "中🙂".repeat(150),
            "🙂中".repeat(150)
        );
        for (at, _) in source.char_indices() {
            check_chunks(&source, &[at]);
        }
        let mut live = Document::new();
        for (at, c) in source.char_indices() {
            let end = at + c.len_utf8();
            live.append(&source[at..end]).unwrap();
            let mut whole = Document::new();
            whole.append(&source[..end]).unwrap();
            assert_eq!(snapshot(&live), snapshot(&whole), "prefix {end}");
        }
        live.finish();
        let mut whole = Document::new();
        whole.append(&source).unwrap();
        whole.finish();
        assert_eq!(snapshot(&live), snapshot(&whole));
    }
}

#[test]
fn container_tabs_match_all_utf8_splits_and_live_prefixes_at_classification_limit() {
    let sources = [
        format!(
            "-\t[中{}\r\n\t🙂](u)\n\t===\n-\t[x]\tlast",
            "x".repeat(1020)
        ),
        format!("> \t-\th|h\r\n> \t\t{}|-\r\n> \t\tx|y\n", "-".repeat(1030)),
        format!("{}-\t中\n{}\t# next\n", ">".repeat(1022), ">".repeat(1022)),
        format!("-{}\t# literal\n\tcontinued\n", " ".repeat(1023)),
    ];
    for source in sources {
        for (at, _) in source.char_indices() {
            check_chunks(&source, &[at]);
        }
        let mut live = Document::new();
        for (at, c) in source.char_indices() {
            let end = at + c.len_utf8();
            live.append(&source[at..end]).unwrap();
            let mut whole = Document::new();
            whole.append(&source[..end]).unwrap();
            assert_eq!(snapshot(&live), snapshot(&whole), "prefix {end}");
        }
        live.finish();
        let mut whole = Document::new();
        whole.append(&source).unwrap();
        whole.finish();
        assert_eq!(snapshot(&live), snapshot(&whole));
    }
}

#[test]
fn partial_tabs_match_all_utf8_splits_and_live_prefixes_at_classification_limit() {
    let sources = [
        format!("- ```\n\t中{}🙂\r\n\t```\n- sibling\n", "x".repeat(1030)),
        format!("> - ```\n>\t{}~~~\n>\t```", " ".repeat(1030)),
        format!("{}- ```\n{}\t中\n", ">".repeat(1018), ">".repeat(1018)),
        format!("- ```\n\t{}\n\t```", "`".repeat(1030)),
    ];
    for source in sources {
        for (at, _) in source.char_indices() {
            check_chunks(&source, &[at]);
        }
        let mut live = Document::new();
        for (at, c) in source.char_indices() {
            let end = at + c.len_utf8();
            live.append(&source[at..end]).unwrap();
            let mut whole = Document::new();
            whole.append(&source[..end]).unwrap();
            assert_eq!(snapshot(&live), snapshot(&whole), "prefix {end}");
        }
        live.finish();
        let mut whole = Document::new();
        whole.append(&source).unwrap();
        whole.finish();
        assert_eq!(snapshot(&live), snapshot(&whole));
    }
}

#[test]
fn prose_partial_tabs_match_utf8_splits_live_prefixes_and_bounded_fallback() {
    let sources = [
        format!("- [x] *中\n\t{}🙂* &amp; [a\n\tb](u)\r\n", "x".repeat(1030)),
        format!("- ` a\n\t{}\n\t`\n", "x".repeat(1030)),
        format!("{}- [a\n{}\t中](u)\n", ">".repeat(1018), ">".repeat(1018)),
        format!("- first\n\t{}late\n", " ".repeat(1030)),
        format!("- first\n\t{}|h\n  -|-\n", "x".repeat(1030)),
    ];
    for source in sources {
        for (at, _) in source.char_indices() {
            check_chunks(&source, &[at]);
        }
        let mut live = Document::new();
        for (at, c) in source.char_indices() {
            let end = at + c.len_utf8();
            live.append(&source[at..end]).unwrap();
            let mut whole = Document::new();
            whole.append(&source[..end]).unwrap();
            assert_eq!(snapshot(&live), snapshot(&whole), "prefix {end}");
        }
        live.finish();
        let mut whole = Document::new();
        whole.append(&source).unwrap();
        whole.finish();
        assert_eq!(snapshot(&live), snapshot(&whole));
    }
}

#[test]
fn math_partial_tabs_match_all_utf8_splits_live_prefixes_and_classification_bounds() {
    let sources = [
        "- $a\r\n\t🙂 &amp;\\$b$\r\n- sibling\n".to_owned(),
        "- > $$a\n\t>\t中$$\n  > ===\n".to_owned(),
        "- $$\n\t\n\t  $$\n\t中\r\n\t $$".to_owned(),
        "- > $$\n\tmissing quote\n- sibling\n".to_owned(),
        format!("- $a\n\t{}🙂$ | b\n  -|-\n", "x".repeat(1030)),
        format!("- $$\n\t{}\r\n\t$$\n", "$".repeat(1030)),
        format!("{}- $a\n{}\t中$\n", ">".repeat(1018), ">".repeat(1018)),
        format!("{}- $$\n{}\t中\n", ">".repeat(1018), ">".repeat(1018)),
        format!("- $a\n\t{}late$\n", " ".repeat(1030)),
    ];
    for source in sources {
        for (at, _) in source.char_indices() {
            check_chunks(&source, &[at]);
        }
        let mut live = Document::new();
        for (at, c) in source.char_indices() {
            let end = at + c.len_utf8();
            live.append(&source[at..end]).unwrap();
            let mut whole = Document::new();
            whole.append(&source[..end]).unwrap();
            assert_eq!(snapshot(&live), snapshot(&whole), "prefix {end}");
        }
        live.finish();
        let mut whole = Document::new();
        whole.append(&source).unwrap();
        whole.finish();
        assert_eq!(snapshot(&live), snapshot(&whole));
    }
}

#[test]
fn new_leaf_partial_tabs_match_all_utf8_splits_live_prefixes_and_bounds() {
    for source in [
        "- [old](u)\n\t## 中🙂 &amp; [new](v) ##\r\n\t### child\n- sibling\n".to_owned(),
        "- first\n\t```rs\n\t中🙂\r\n\t  ```\n\t```\n\t$$\n\tx\n\t$$".to_owned(),
        "- - - old\n\t\t# h\n\t\t  # literal\n\t\t   next\n".to_owned(),
        "- > old\n\t>\t# h\n\t>\t~~~\n\t>\t中🙂\n\tmissing\n".to_owned(),
        "- h|h\n\t  -|-\n  h|h\n  -|-\n\t# next\n".to_owned(),
        "- old\n\ta|b\n  -|-\n\t===\n\t\t# literal\n".to_owned(),
        format!("- old\n\t# {}🙂\n", "x".repeat(1030)),
        format!("- old\n\t  {}|h\n  -|-\n", "-".repeat(1030)),
        format!("- old\n\t$${}\n", " ".repeat(1030)),
        format!("{}- old\n{}\t# 中🙂\n", ">".repeat(1018), ">".repeat(1018)),
        format!("{}- old\n{}\t# 中🙂\n", ">".repeat(1022), ">".repeat(1022)),
    ] {
        for (at, _) in source.char_indices() {
            check_chunks(&source, &[at]);
        }
        let mut live = Document::new();
        for (at, c) in source.char_indices() {
            let end = at + c.len_utf8();
            live.append(&source[at..end]).unwrap();
            let mut whole = Document::new();
            whole.append(&source[..end]).unwrap();
            assert_eq!(snapshot(&live), snapshot(&whole), "prefix {end}");
        }
        live.finish();
        let mut whole = Document::new();
        whole.append(&source).unwrap();
        whole.finish();
        assert_eq!(snapshot(&live), snapshot(&whole));
    }
}

#[test]
fn list_blank_retention_matches_all_utf8_splits_live_prefixes_and_eof() {
    for source in [
        "- # 中\r\n\r\n- [x] 🙂\r\n\n- [ ]".to_owned(),
        "- *open\n  - [label](u)\n\n    **中**\n\n  # parent\n\n- sibling\n".to_owned(),
        "- - child\n  \n  # parent\n- > - deep\n\n  > new\n\n- sibling\n".to_owned(),
        "> - a\n>\n> - b\n\n> - c\n>\n>   tail\n> \n".to_owned(),
        "- ```\n  raw\n\n  next\n\n  $$\n  raw\n\n  [^n]: body\n\n- tail".to_owned(),
        "- h|h\n\n  -|-\n\n  head\n\n  ===\n\n+ other\n".to_owned(),
        "- a\n\n\t# title\n\n\ttext\n- b\n\t\r\n- c".to_owned(),
        "-\n\n text\n\n- [x]\n\n- [ ]\n\n".to_owned(),
        format!("- a\n{}\n- 中🙂\n", " ".repeat(BLOCK_LOOKAHEAD - 1)),
        format!("- a\n{}\n- 中🙂\n", " ".repeat(BLOCK_LOOKAHEAD)),
        format!(
            "- a\n\n  {}|h\n  -|-\n\n- end\n",
            "x".repeat(BLOCK_LOOKAHEAD + 20)
        ),
    ] {
        for (at, _) in source.char_indices() {
            check_chunks(&source, &[at]);
        }
        let mut live = Document::new();
        for (at, c) in source.char_indices() {
            let end = at + c.len_utf8();
            live.append(&source[at..end]).unwrap();
            let mut whole = Document::new();
            whole.append(&source[..end]).unwrap();
            assert_eq!(
                snapshot(&live),
                snapshot(&whole),
                "prefix {end}: {source:?}"
            );
        }
        live.finish();
        let mut whole = Document::new();
        whole.append(&source).unwrap();
        whole.finish();
        assert_eq!(snapshot(&live), snapshot(&whole));
    }
}

#[test]
fn nested_list_tails_match_all_utf8_splits_live_prefixes_and_eof() {
    for source in [
        "- # 中\r\n  - [x] **🙂** [l](u)\r\n    \r\n- [ ] sibling".to_owned(),
        "- - > quote\n    >\n  # parent\n- sibling\n".to_owned(),
        "- > - 中\n  >   \n  > - 🙂\n- sibling\n".to_owned(),
        "- - first\n    \n    more\n- sibling\n  - child\n\t\n  + new child\n".to_owned(),
        "- - [^n]: body\n    \n- next\n  - $$\n    \n- last\n".to_owned(),
        "- - ```\n    \n    ```\n    \n- next\n  - h|h\n    -|-\n    \n  head\n  ===".to_owned(),
        "> - - first\n>     \n> - next\n>   - last\n>     \n# exit\n".to_owned(),
        "- - child\n    \n".to_owned(),
        format!("- - child\n{}\n- 中🙂\n", " ".repeat(BLOCK_LOOKAHEAD - 1)),
        format!("- - child\n{}\n- 中🙂\n", " ".repeat(BLOCK_LOOKAHEAD)),
        format!(
            "- - child\n    \n  {}|h\n  -|-\n- next\n",
            "x".repeat(BLOCK_LOOKAHEAD + 20)
        ),
    ] {
        for (at, _) in source.char_indices() {
            check_chunks(&source, &[at]);
        }
        let mut live = Document::new();
        for (at, c) in source.char_indices() {
            let end = at + c.len_utf8();
            live.append(&source[at..end]).unwrap();
            let mut whole = Document::new();
            whole.append(&source[..end]).unwrap();
            assert_eq!(
                snapshot(&live),
                snapshot(&whole),
                "prefix {end}: {source:?}"
            );
        }
        live.finish();
        let mut whole = Document::new();
        whole.append(&source).unwrap();
        whole.finish();
        assert_eq!(snapshot(&live), snapshot(&whole));
    }
}

#[test]
fn isolated_closed_headers_match_utf8_splits_and_every_live_prefix() {
    for source in [
        format!(
            "> - [x] *old  \r\n>   end* **中** [h](u) {}|$m$\r\n>   -|-\r\n",
            "中🙂".repeat(150)
        ),
        format!("**old *old\nend* end** {}|b\n-|-", "x".repeat(1030)),
        format!(
            "> *old\n> end* {}|b\n> {}x|b\n> ===\n",
            "x".repeat(1030),
            "-".repeat(1030)
        ),
        format!("_old\nend_ {}\r\n", "x".repeat(1030)),
        format!("~~old\nend~~ {}", "x".repeat(1030)),
        format!("*old\n{}end* \t\\|escaped|b\n-|-\n", "x".repeat(1030)),
        format!("**old *old\nend* end**new {}|b\n-|-\n", "x".repeat(1030)),
        format!("> *old\n> **local end* {}|b\n> -|-\n", "x".repeat(1030)),
        format!("> *old\n> end* {}|b\n-|-\n", "x".repeat(1030)),
    ] {
        for (at, _) in source.char_indices() {
            check_chunks(&source, &[at]);
        }
        let mut live = Document::new();
        for (at, c) in source.char_indices() {
            let end = at + c.len_utf8();
            live.append(&source[at..end]).unwrap();
            let mut whole = Document::new();
            whole.append(&source[..end]).unwrap();
            assert_eq!(snapshot(&live), snapshot(&whole), "prefix {end}");
        }
        live.finish();
        let mut whole = Document::new();
        whole.append(&source).unwrap();
        whole.finish();
        assert_eq!(snapshot(&live), snapshot(&whole));
    }
}
