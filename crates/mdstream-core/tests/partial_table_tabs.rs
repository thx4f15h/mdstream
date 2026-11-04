use mdstream::*;

#[path = "support/snapshot.rs"]
mod support;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn visible(doc: &Document) -> String {
    doc.display_runs_in(Span::new(0, doc.source().len()))
        .unwrap()
        .filter(|run| !run.style.contains(Style::HIDDEN))
        .map(|run| run.text)
        .collect()
}

#[test]
fn partial_headers_and_rows_keep_the_existing_container() {
    for source in [
        "- seed\n\n\ta | b\n  - | -\n\tc | d\n",
        "1. seed\n\n\ta | b\n   - | -\n\tc | d\n",
        "> seed\n>\n>\ta | b\n> - | -\n>\tc | d\n",
        "- seed\n\ta | b\n  - | -\n\tc | d\n",
    ] {
        let doc = parse(source);
        assert_eq!(doc.tables().len(), 1, "{source:?}");
        assert_eq!(doc.tables()[0].rows().len(), 2, "{source:?}");
        assert_eq!(
            doc.blocks()[0].container,
            doc.blocks().last().unwrap().container
        );
        assert_eq!(doc.source(), source);
    }
}

#[test]
fn headers_rows_and_fallback_match_physical_column_model() {
    let mut cases = 0;
    for outer in ["", "> ", ">> ", "   > "] {
        for marker in ["- ", "1. "] {
            for depth in 1..=6 {
                let opening = format!("{outer}{}seed\n", marker.repeat(depth));
                let target = outer.len() + marker.len() * depth;
                let full = format!("{outer}{}", " ".repeat(target - outer.len()));
                for leading in 0..4 {
                    let mut col = outer.len() + leading;
                    let mut prefix = format!("{outer}{}", " ".repeat(leading));
                    while col < target {
                        prefix.push('\t');
                        col += 4 - col % 4;
                    }
                    let expanded = format!("{outer}{}", " ".repeat(col - outer.len()));
                    for blank in [false, true] {
                        let gap = if blank {
                            format!("{full}\n")
                        } else {
                            String::new()
                        };
                        for delimiter in ["- | -", ":- | -:", "nope", "| - | - | - |"] {
                            let valid = matches!(delimiter, "- | -" | ":- | -:");
                            let row = if valid {
                                format!("{prefix}&amp; | `x`\n")
                            } else {
                                String::new()
                            };
                            let model_row = if valid {
                                format!("{expanded}&amp; | `x`\n")
                            } else {
                                String::new()
                            };
                            let source = format!(
                                "{opening}{gap}{prefix}*中🙂* | [a](u)\n{full}{delimiter}\n{row}"
                            );
                            let model = format!(
                                "{opening}{gap}{expanded}*中🙂* | [a](u)\n{full}{delimiter}\n{model_row}"
                            );
                            let actual = parse(&source);
                            let model = parse(&model);
                            assert_eq!(visible(&actual), visible(&model), "{source:?}");
                            assert_eq!(actual.tables().len(), model.tables().len(), "{source:?}");
                            assert_eq!(
                                actual.tables().len(),
                                usize::from(delimiter != "nope" && delimiter != "| - | - | - |")
                            );
                            assert_eq!(actual.container_count(), model.container_count());
                            for i in 0..actual.container_count() {
                                let a = actual.container(ContainerId(i)).unwrap();
                                let b = model.container(ContainerId(i)).unwrap();
                                assert_eq!(
                                    (a.kind, a.parent, a.depth, a.list_tightness),
                                    (b.kind, b.parent, b.depth, b.list_tightness)
                                );
                            }
                            for (a, b) in actual.tables().iter().zip(model.tables()) {
                                assert_eq!(a.rows().len(), b.rows().len());
                                assert_eq!(a.columns, b.columns);
                                for (a, b) in a.rows().iter().zip(b.rows()) {
                                    let a: Vec<_> =
                                        a.cells.iter().map(|c| actual.text(c.source)).collect();
                                    let b: Vec<_> =
                                        b.cells.iter().map(|c| model.text(c.source)).collect();
                                    assert_eq!(a, b);
                                }
                            }
                            assert!(actual.work().container_bytes < 16 * source.len() as u64);
                            assert!(actual.work().classified_bytes < 12 * source.len() as u64);
                            cases += 1;
                        }
                    }
                }
            }
        }
    }
    assert_eq!(cases, 1536);
}

#[test]
fn splits_prefixes_and_eof_preserve_success_and_failure() {
    for source in [
        "- seed\n\n\t*中🙂* | [a](u)\r\n  :- | -:\r\n\t&amp; | `x`\r\n",
        "1. seed\n\ta|b\n   -|-\n\tc|d",
        "- seed\n\n\ta|b\n  nope\n",
        "- seed\n\n\ta|b",
        "- seed\n\t`a\n\tb`|c\n  -|-\n",
        "- seed\n\t[a\n\tb](u)|c\n  -|-\n",
        "- seed\n\t*a\n\tb*|c\n  -|-\n",
        "- a|b\n\t-|-\n",
        "- > seed\n\ta|b\n  > -|-\n",
        "- seed\n\t  a|b\n  -|-\n",
        "- seed\n\ta|b\n  -|-\n\t# title\n",
    ] {
        let expected = support::snapshot(&parse(source));
        for split in (0..=source.len()).filter(|&at| source.is_char_boundary(at)) {
            let mut doc = Document::new();
            doc.append(&source[..split]).unwrap();
            doc.append(&source[split..]).unwrap();
            doc.finish();
            assert_eq!(
                support::snapshot(&doc),
                expected,
                "split {split}: {source:?}"
            );
        }
        let mut doc = Document::new();
        for (at, ch) in source.char_indices() {
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            let prefix = &source[..at + ch.len_utf8()];
            let mut bulk = Document::new();
            bulk.append(prefix).unwrap();
            assert_eq!(
                support::snapshot(&doc),
                support::snapshot(&bulk),
                "prefix {prefix:?}"
            );
            assert_eq!(doc.pending(), bulk.pending());
            let mut eof = Document::new();
            for ch in prefix.chars() {
                eof.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            }
            eof.finish();
            assert_eq!(support::snapshot(&eof), support::snapshot(&parse(prefix)));
        }
    }
}

#[test]
fn mapping_and_shared_budget_cover_confirmed_and_rejected_headers() {
    for ending in ["  -|-\n\t&amp;|`x`\n", "  nope\n", ""] {
        let source = format!("- seed\n\n\t*中🙂*|[a](u)\n{ending}");
        let doc = parse(&source);
        let full = Span::new(0, source.len());
        assert_eq!(
            doc.runs_in(full)
                .unwrap()
                .map(|r| r.text)
                .collect::<String>(),
            source
        );
        let scalars: String = source
            .char_indices()
            .flat_map(|(at, ch)| {
                doc.display_runs_in(Span::new(at, at + ch.len_utf8()))
                    .unwrap()
            })
            .filter(|r| !r.style.contains(Style::HIDDEN))
            .map(|r| r.text)
            .collect();
        let tiled: String = (0..doc.tile_count())
            .flat_map(|i| doc.display_runs(TileId(i)).unwrap())
            .filter(|r| !r.style.contains(Style::HIDDEN))
            .map(|r| r.text)
            .collect();
        assert_eq!(scalars, visible(&doc));
        assert_eq!(tiled, scalars);
        let tab = source.find('\t').unwrap();
        let shown: String = doc
            .display_runs_in(Span::new(tab, tab + 1))
            .unwrap()
            .filter(|r| !r.style.contains(Style::HIDDEN))
            .map(|r| r.text)
            .collect();
        assert_eq!(shown, if ending.starts_with("  -") { "" } else { "  " });
        let mut cursor = doc.display_cursor(full).unwrap();
        let mut second = doc.display_cursor(full).unwrap();
        let mut output = String::new();
        loop {
            let position = cursor.source_position();
            let mut empty = Budget::new(WorkLimit::default());
            if position < source.len() {
                assert!(matches!(
                    cursor.next(&doc, &mut empty).unwrap(),
                    DisplayStep::BudgetExhausted
                ));
                assert_eq!(cursor.source_position(), position);
            }
            let mut budget = Budget::new(WorkLimit {
                queries: 1,
                objects: 1,
                bytes: 6,
                ..Default::default()
            });
            match cursor.next(&doc, &mut budget).unwrap() {
                DisplayStep::Run(run) => {
                    assert_eq!(run.source.start, position);
                    if !run.style.contains(Style::HIDDEN) {
                        output.push_str(run.text);
                    }
                    assert!(matches!(
                        second.next(&doc, &mut budget).unwrap(),
                        DisplayStep::BudgetExhausted
                    ));
                    assert_eq!(second.source_position(), 0);
                }
                DisplayStep::Complete => break,
                DisplayStep::BudgetExhausted => panic!("one scalar fits"),
            }
            assert!(budget.used().bytes <= 6);
        }
        assert_eq!(output, visible(&doc));
    }
}

#[test]
fn delimiter_code_and_missing_path_boundaries_remain_explicit() {
    // A rejected delimiter containing pipes can itself become a pending
    // header. Its next partial-tab text line retains the virtual-space map.
    let pending = parse("- seed\n\ta|b\n  | - | - | - |\n\tc|d\n");
    assert!(pending.tables().is_empty());
    assert_eq!(visible(&pending), "seed\n  a|b\n| - | - | - |\n  c|d\n");
    for source in [
        "- seed\n\n\t  a|b\n  -|-\n",
        "- > seed\n\ta|b\n  > -|-\n",
        "- seed\n\ta|b\n  -|-|-\n",
    ] {
        assert!(parse(source).tables().is_empty(), "{source:?}");
    }
    for body in ["# title", "```", "$$", "- nested", "> quote", "[r]: /u"] {
        let source = format!("- seed\n\ta|b\n  -|-\n\t{body}\n");
        let doc = parse(&source);
        assert_eq!(doc.tables().len(), 1);
        assert_eq!(doc.tables()[0].rows().len(), 1, "{body}");
        assert_eq!(doc.tables()[0].source.end, source.rfind('\t').unwrap());
    }
}

#[test]
fn deep_long_headers_rows_and_history_keep_incremental_work_and_identity() {
    let outer = ">".repeat(980);
    for confirm in [false, true] {
        let mut doc = Document::new();
        doc.append(&format!(
            "{outer}- seed\n{outer}\n{outer}\t[a](u){}",
            "x".repeat(1_000_000)
        ))
        .unwrap();
        let scope = doc.blocks()[0].container;
        let link = *doc.link_info(LinkId(0)).unwrap();
        let before = doc.work();
        for _ in 0..1000 {
            doc.append("x").unwrap();
        }
        assert_eq!(doc.work().container_steps, before.container_steps);
        assert_eq!(doc.work().container_bytes, before.container_bytes);
        assert_eq!(doc.work().classified_bytes, before.classified_bytes);
        assert_eq!(doc.work().inline_bytes - before.inline_bytes, 1000);
        doc.append(&format!(
            "|b\n{outer}   {}\n",
            if confirm { "-|-" } else { "nope" }
        ))
        .unwrap();
        assert_eq!(doc.tables().len(), usize::from(confirm));
        assert_eq!(*doc.link_info(LinkId(0)).unwrap(), link);
        assert_eq!(doc.link_destination(LinkId(0)), Some("u"));
        assert_eq!(doc.blocks().last().unwrap().container, scope);
        if confirm {
            let block = doc.tables()[0].block;
            doc.append(&format!("{outer}\t{}", "x".repeat(100)))
                .unwrap();
            let before = doc.work();
            for _ in 0..1000 {
                doc.append(&"x".repeat(1000)).unwrap();
            }
            assert_eq!(doc.work().container_steps, before.container_steps);
            assert_eq!(doc.work().classified_bytes, before.classified_bytes);
            assert_eq!(doc.work().table_bytes - before.table_bytes, 1_000_000);
            assert_eq!(doc.work().inline_bytes - before.inline_bytes, 1_000_000);
            let update = doc.append("|zzz\n").unwrap();
            assert_eq!(update.tables_changed, [TableId(0)]);
            assert_eq!(doc.tables()[0].block, block);
            assert_eq!(doc.tables()[0].rows().len(), 2);
        }
        doc.finish();
        assert_eq!(doc.container_count(), 982);
        assert!(doc.work().table_header_bytes <= 2 * doc.source().len() as u64);
        assert!(doc.work().inline_bytes <= 2 * doc.source().len() as u64);
    }
    let mut doc = Document::new();
    doc.append("- seed\n\ta|b\n  -|-\n").unwrap();
    for _ in 0..4096 {
        doc.append("\tx|y\n").unwrap();
    }
    let before = doc.work();
    let update = doc.append("\tlonger|z\n").unwrap();
    assert_eq!(update.tables_changed, [TableId(0)]);
    assert_eq!(doc.tables()[0].rows().len(), 4098);
    assert!(doc.work().container_steps - before.container_steps < 10);
    assert!(doc.work().table_bytes - before.table_bytes < 20);
    assert!(doc.work().tree_visits - before.tree_visits < 2000);
    doc.finish();
}

#[test]
fn inherited_long_candidates_match_expanded_paths_and_every_split() {
    for (opener, closer) in [
        ("$a", "b$ "),
        ("[a", "b] "),
        ("[a", "b][key] "),
        ("`a", "b` "),
        ("*a", "b* "),
    ] {
        for ending in ["  -|-\n", "  nope\n", ""] {
            let source = format!("- {opener}\n\t{closer}{}|c\n{ending}", "x".repeat(1030));
            let doc = parse(&source);
            let model = parse(&source.replace('\t', "    "));
            assert_eq!(visible(&doc), visible(&model), "{opener:?}, {ending:?}");
            assert_eq!(doc.tables().len(), usize::from(ending == "  -|-\n"));
            assert_eq!(doc.tables().len(), model.tables().len());
            assert_eq!(doc.links().len(), model.links().len());
            assert_eq!(doc.maths().len(), model.maths().len());
            let expected = support::snapshot(&doc);
            for split in 0..=source.len() {
                let mut chunked = Document::new();
                chunked.append(&source[..split]).unwrap();
                chunked.append(&source[split..]).unwrap();
                chunked.finish();
                assert_eq!(
                    support::snapshot(&chunked),
                    expected,
                    "{opener:?}, {ending:?}, split {split}"
                );
            }
        }
    }
}
