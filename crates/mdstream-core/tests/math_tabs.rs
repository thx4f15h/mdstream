use mdstream::*;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn read(mut cursor: MathContent<'_>) -> String {
    let mut text = String::new();
    let mut budget = Budget::new(WorkLimit::unlimited());
    while let Some(chunk) = cursor.advance(&mut budget).unwrap() {
        text.push_str(chunk.text);
    }
    text
}

fn visible(doc: &Document) -> String {
    doc.display_runs_in(Span::new(0, doc.source().len()))
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect()
}

#[test]
fn inline_math_preserves_raw_content_and_maps_partial_tab_columns() {
    for (opener, prefix, spaces) in [
        ("- ", "\t", "  "),
        ("> ", ">\t", "  "),
        ("1. ", "\t", " "),
        ("   > ", "   >\t", "   "),
        ("- - ", "\t", ""),
        ("- - - ", "\t\t", "  "),
        ("- > ", "\t>\t", "  "),
    ] {
        for delimiter in ["$", "$$"] {
            let source = format!("{opener}{delimiter}中\\alpha\r\n{prefix}🙂 &amp;{delimiter}\n");
            let doc = parse(&source);
            assert_eq!(doc.maths().len(), 1, "{source:?}");
            let raw = format!("中\\alpha\r\n{prefix}🙂 &amp;");
            let payload = format!("中\\alpha\r\n{spaces}🙂 &amp;");
            assert_eq!(read(doc.math_content(MathId(0)).unwrap()), raw);
            assert_eq!(read(doc.math_payload(MathId(0)).unwrap()), payload);
            assert_eq!(
                visible(&doc),
                format!("{}\n", payload.replace("\r\n", "\n"))
            );
            assert_eq!(doc.blocks()[0].container, doc.blocks()[1].container);
            assert_eq!(doc.source(), source);
        }
    }
}

#[test]
fn math_block_tabs_affect_closing_indent_and_keep_literal_content() {
    let source = "- $$\n\t中 &amp;\\alpha\r\n\t  $$\n\t\n\t $$\n- sibling\n";
    let doc = parse(source);
    let id = BlockId(0);
    assert!(matches!(
        doc.blocks()[0].kind,
        BlockKind::Math { closed: true, .. }
    ));
    assert_eq!(
        read(doc.math_block_content(id).unwrap()),
        "\t中 &amp;\\alpha\r\n\t  $$\n\t\n"
    );
    assert_eq!(
        read(doc.math_block_payload(id).unwrap()),
        "  中 &amp;\\alpha\r\n    $$\n  \n"
    );
    assert!(doc.maths().is_empty());
    assert_eq!(doc.blocks().len(), 2);
}

#[test]
fn delimiters_escapes_and_failed_candidates_keep_existing_inline_rules() {
    for (source, payloads) in [
        ("- $a\\\n\tb$\n", vec!["a\\\n  b"]),
        ("- $$a\n\tb$ c$$\n", vec!["a\n  b$ c"]),
        ("- $a\n\t\\$b$\n", vec!["a\n  \\$b"]),
        ("- $a\n\t$$ b$\n", vec!["a\n  $$ b"]),
        ("- $a\n\tb$ $c\n\td$\n", vec!["a\n  b", "c\n  d"]),
        ("- first\n\t$a\n\tb$\n", vec!["a\n  b"]),
        ("- $a\n\tstill open\n", vec![]),
        ("- `a\n\t$b$`\n", vec![]),
        ("- $a\n\t# new leaf\n", vec![]),
    ] {
        let doc = parse(source);
        assert_eq!(doc.maths().len(), payloads.len(), "{source:?}");
        for (i, expected) in payloads.into_iter().enumerate() {
            assert_eq!(read(doc.math_payload(MathId(i)).unwrap()), expected);
        }
        if source.contains("still open") {
            assert_eq!(visible(&doc), "$a\n  still open\n");
        }
    }
}

#[test]
fn atomic_scope_failure_and_unexpanded_whitespace_do_not_leak_mappings() {
    // Completed short blanks, including EOF, now consume the Item indent.
    for source in ["- $a\n\t\n", "- $a\n\t"] {
        let doc = parse(source);
        assert_eq!(doc.blocks()[1].kind, BlockKind::Blank);
        let tab = source.find('\t').unwrap();
        let runs: Vec<_> = doc
            .display_runs_in(Span::new(tab, tab + 1))
            .unwrap()
            .collect();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "  ");
        assert_eq!(runs[0].replacement, Some(Span::new(tab, tab + 1)));
    }

    // Lazy text after a partially consumed Item tab can now omit the Quote.
    let source = "- > $a\n\tb$\n";
    let doc = parse(source);
    assert_eq!(read(doc.math_content(MathId(0)).unwrap()), "a\n\tb");
    assert_eq!(read(doc.math_payload(MathId(0)).unwrap()), "a\n  b");
    let tab = source.find('\t').unwrap();
    let runs: Vec<_> = doc
        .display_runs_in(Span::new(tab, tab + 1))
        .unwrap()
        .collect();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].text, "  ");
    assert_eq!(runs[0].replacement, Some(Span::new(tab, tab + 1)));

    for source in ["- $a\n\t  b$\n", "- $a\n\t- nested\n"] {
        let doc = parse(source);
        assert!(
            doc.display_runs_in(Span::new(0, source.len()))
                .unwrap()
                .all(|r| r.replacement.is_none()),
            "{source:?}"
        );
    }
    let doc = parse("- > $$\n\toutside\n");
    assert!(matches!(
        doc.blocks()[0].kind,
        BlockKind::Math { closed: false, .. }
    ));
    assert_eq!(read(doc.math_block_payload(BlockId(0)).unwrap()), "");
    assert_eq!(doc.blocks()[0].source.end, "- > $$\n".len());
    assert_eq!(doc.blocks()[1].kind, BlockKind::Paragraph);
    assert_eq!(doc.blocks()[1].container, Some(ContainerId(1)));
    assert_eq!(visible(&doc), "  outside\n");
    // Existing block contents stay literal, even blank or over-indented;
    // only the path's final partially consumed tab is expanded.
    let doc = parse("- $$\n\t\t x\n\t");
    assert_eq!(
        read(doc.math_block_payload(BlockId(0)).unwrap()),
        "  \t x\n  "
    );
    assert!(matches!(
        doc.blocks()[0].kind,
        BlockKind::Math { closed: false, .. }
    ));
}

#[test]
fn closing_math_blocks_counts_virtual_columns_and_keeps_bounded_failures() {
    for (opener, prefix, columns) in [("- ", "\t", 2), ("1. ", "\t", 1), ("   > ", "   >\t", 3)] {
        for spaces in 0..5 {
            for end in ["\n", "\r\n", ""] {
                let line = format!("{prefix}{}$${end}", " ".repeat(spaces));
                let doc = parse(&format!("{opener}$$\n{line}"));
                let closed = spaces + columns <= 3;
                assert!(
                    matches!(doc.blocks()[0].kind, BlockKind::Math { closed: c, .. } if c == closed)
                );
                let expected = if closed {
                    String::new()
                } else {
                    format!("{}$${end}", " ".repeat(spaces + columns))
                };
                assert_eq!(read(doc.math_block_payload(BlockId(0)).unwrap()), expected);
                assert_eq!(visible(&doc), expected.replace("\r\n", "\n"));
            }
        }
    }
    for candidate in [
        format!("$${}", " ".repeat(BLOCK_LOOKAHEAD)),
        "$".repeat(BLOCK_LOOKAHEAD + 10),
    ] {
        let source = format!("- $$\n\t{candidate}\n\t$$\n");
        let doc = parse(&source);
        assert_eq!(
            read(doc.math_block_payload(BlockId(0)).unwrap()),
            format!("  {candidate}\n")
        );
        assert!(matches!(
            doc.blocks()[0].kind,
            BlockKind::Math { closed: true, .. }
        ));
    }
}

#[test]
fn math_payload_budget_charges_source_and_returns_atomic_tab_replacements() {
    let doc = parse("   > $中\r\n   >\t🙂\\alpha &amp;$\n   > $$\n   >\t🙂\r\n   >\t$$\n");
    for (mapped, block) in [(true, false), (false, false), (true, true), (false, true)] {
        let mut cursor = match (mapped, block) {
            (true, false) => doc.math_payload(MathId(0)),
            (false, false) => doc.math_content(MathId(0)),
            (true, true) => doc.math_block_payload(BlockId(2)),
            (false, true) => doc.math_block_content(BlockId(2)),
        }
        .unwrap();
        let start = cursor.source_position();
        let mut consumed = 0;
        let mut replacements = 0;
        let mut text = String::new();
        loop {
            let position = cursor.source_position();
            let mut zero = Budget::new(WorkLimit::default());
            if cursor.advance(&mut zero) == Ok(None) {
                break;
            }
            assert_eq!(zero.used(), WorkLimit::default());
            assert_eq!(cursor.source_position(), position);
            // Insufficient object/query/UTF-8 budgets all pause atomically.
            for limits in [
                WorkLimit {
                    bytes: 4,
                    queries: 1,
                    ..Default::default()
                },
                WorkLimit {
                    bytes: 4,
                    objects: 1,
                    ..Default::default()
                },
            ] {
                let mut budget = Budget::new(limits);
                assert_eq!(
                    cursor.advance(&mut budget),
                    Err(Completion::BudgetExhausted)
                );
                assert_eq!(cursor.source_position(), position);
                assert_eq!(budget.used(), WorkLimit::default());
            }
            let scalar = doc.source()[position..].chars().next().unwrap().len_utf8();
            let mut short = Budget::new(WorkLimit {
                bytes: scalar - 1,
                queries: 1,
                objects: 1,
                ..Default::default()
            });
            assert_eq!(cursor.advance(&mut short), Err(Completion::BudgetExhausted));
            assert_eq!(short.used(), WorkLimit::default());
            let mut shared = Budget::new(WorkLimit {
                bytes: scalar,
                queries: 1,
                objects: 1,
                ..Default::default()
            });
            let chunk = cursor.advance(&mut shared).unwrap().unwrap();
            assert_eq!(chunk.source, Span::new(position, position + scalar));
            assert_eq!(shared.used().bytes, scalar);
            assert_eq!(shared.used().queries, 1);
            assert_eq!(shared.used().objects, 1);
            if !chunk.text.is_empty() && doc.text(chunk.source) != Some(chunk.text) {
                assert!(mapped);
                assert_eq!(doc.text(chunk.source), Some("\t"));
                assert_eq!(chunk.text, "   ");
                replacements += 1;
            }
            text.push_str(chunk.text);
            consumed += scalar;
        }
        assert_eq!(consumed, cursor.source_position() - start);
        assert_eq!(replacements, usize::from(mapped));
        assert_eq!(text.contains('\t'), !mapped);
        assert!(text.contains('\r'));
    }
}

#[test]
fn generated_nested_math_matches_independent_column_and_source_models() {
    let mut seed = 0x323c_0a10_u64;
    for case in 0..400 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let depth = (seed >> 32) as usize % 25 + 1;
        let parent = "> ".repeat((seed >> 48) as usize % 8);
        let target = parent.len() + depth * 2;
        let mut column = parent.len();
        let mut prefix = parent.clone();
        while column < target {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            if seed & 8 == 0 {
                prefix.push(' ');
                column += 1;
            } else {
                prefix.push('\t');
                column = (column / 4 + 1) * 4;
            }
        }
        let opening = format!("{parent}{}", "- ".repeat(depth));
        let columns = " ".repeat(column - target);
        for block in [false, true] {
            let (source, expected, raw) = if block {
                (
                    format!("{opening}$$\n{prefix}中\\alpha &amp;\r\n{prefix}$$\n"),
                    format!("{columns}中\\alpha &amp;\r\n"),
                    format!("{prefix}中\\alpha &amp;\r\n"),
                )
            } else {
                (
                    format!("{opening}$first\n{prefix}中\\alpha &amp;$\r\n"),
                    format!("first\n{columns}中\\alpha &amp;"),
                    format!("first\n{prefix}中\\alpha &amp;"),
                )
            };
            let doc = parse(&source);
            let (mapped, original) = if block {
                (
                    doc.math_block_payload(BlockId(0)),
                    doc.math_block_content(BlockId(0)),
                )
            } else {
                (doc.math_payload(MathId(0)), doc.math_content(MathId(0)))
            };
            assert_eq!(read(mapped.unwrap()), expected, "case {case}: {source:?}");
            assert_eq!(read(original.unwrap()), raw);
            assert_eq!(doc.container_count(), parent.len() / 2 + 2 * depth);
            assert!(doc.work().container_bytes < 12 * source.len() as u64);
            assert!(doc.work().container_steps < 4 * source.len() as u64);
        }
    }
}

#[test]
fn math_tabs_compose_with_setext_and_partial_table_headers() {
    for padding in [0, BLOCK_LOOKAHEAD + 10] {
        let source = format!("- $a\n\t{}b$ | h\n  - | -\n", "x".repeat(padding));
        let doc = parse(&source);
        assert_eq!(doc.tables().len(), 1);
        assert!(doc.maths().is_empty());
        assert_eq!(
            doc.text(doc.tables()[0].cell(0, 0).unwrap().source),
            Some(format!("{}b$", "x".repeat(padding)).as_str())
        );
    }
    let doc = parse("- $a\n\tb$\n  ===\n");
    assert_eq!(doc.headings().entries().len(), 1);
    assert_eq!(read(doc.math_payload(MathId(0)).unwrap()), "a\n  b");
    // Unclosed math followed by a confirmed ordinary table is discarded;
    // preceding tab mappings must not be reused by the table's new object.
    let doc = parse("- $abandoned\n\told\n  $cell$ | h\n  - | -\n");
    assert_eq!(doc.tables().len(), 1);
    assert_eq!(doc.maths().len(), 1);
    assert_eq!(read(doc.math_payload(MathId(0)).unwrap()), "cell");
}

#[test]
fn mapped_math_spaces_keep_display_search_and_excerpt_source_identity() {
    let doc = parse("- $a\n\tb$\n");
    let tab = doc.source().find('\t').unwrap();
    let run = doc
        .display_runs_in(Span::new(tab, tab + 1))
        .unwrap()
        .next()
        .unwrap();
    assert_eq!(run.text, "  ");
    assert_eq!(run.replacement, Some(run.source));
    assert!(run.style.contains(Style::MATH));
    let mut search = DisplaySearch::new(&doc, " ").unwrap();
    let mut hits = Vec::new();
    let mut budget = Budget::new(WorkLimit::unlimited());
    assert_eq!(
        search.advance(&doc, &mut budget, &mut hits).unwrap(),
        Completion::EndOfDocument
    );
    assert_eq!(hits.len(), 2);
    assert!(hits.iter().all(|hit| hit.source == Span::new(tab, tab + 1)));
    let mut excerpt = DisplayExcerpt::new(
        &doc,
        Span::new(tab, tab + 1),
        ExcerptLimits {
            source_bytes: 1,
            display_bytes: 1,
            lines: 1,
        },
    )
    .unwrap();
    let ExcerptStep::Run(run) = excerpt.next(&doc, &mut budget).unwrap() else {
        panic!()
    };
    assert_eq!(run.text, " ");
    assert_eq!(run.replacement, Some(Span::new(tab, tab + 1)));
    assert_eq!(
        excerpt.next(&doc, &mut budget).unwrap(),
        ExcerptStep::Complete(ExcerptEnd::DisplayLimit)
    );
    let tiled: String = (0..doc.tile_count())
        .flat_map(|i| doc.display_runs(TileId(i)).unwrap())
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    assert_eq!(tiled, visible(&doc));
}

#[test]
fn streaming_math_keeps_objects_folds_and_merged_invalidations() {
    for block in [false, true] {
        let mut doc = Document::new();
        let mut update = doc
            .append(if block {
                "- $old$\n  $$\n\tx\n"
            } else {
                "- $old$ $open\n\tx\n"
            })
            .unwrap();
        let old = doc.maths()[0];
        let fold = if block {
            FoldId::Math(BlockId(1))
        } else {
            FoldId::Container(doc.blocks()[0].container.unwrap())
        };
        let mut view = FoldView::new(&doc, 1).unwrap();
        view.set_collapsed(&doc, fold, true).unwrap();
        let ticket = view.heights().ticket(TileId(0)).unwrap();
        let mut reader = doc
            .display_cursor(Span::new(0, doc.source().len()))
            .unwrap();
        let mut search = DisplaySearch::new(&doc, "x").unwrap();
        let mut heights = HeightIndex::default();
        heights.apply(&doc, &update).unwrap();
        for chunk in [
            "\t",
            "中",
            "🙂",
            if block { "\n\t$$\r" } else { "$\r" },
            "\n",
            "- sibling\n",
        ] {
            let next = doc.append(chunk).unwrap();
            view.apply(&doc, &next).unwrap();
            heights.apply(&doc, &next).unwrap();
            update.merge(next).unwrap();
            assert_eq!(doc.maths()[0], old);
            assert!(view.is_collapsed(fold));
        }
        let end = doc.finish();
        view.apply(&doc, &end).unwrap();
        heights.apply(&doc, &end).unwrap();
        update.merge(end).unwrap();
        let mut rebuilt = HeightIndex::default();
        rebuilt.apply(&doc, &update).unwrap();
        assert_eq!(rebuilt.total_rows(), heights.total_rows());
        assert!(view.measure(ticket, 8).is_err());
        let mut budget = Budget::new(WorkLimit::unlimited());
        assert_eq!(
            reader.next(&doc, &mut budget),
            Err(LayoutError::StaleContinuation)
        );
        assert_eq!(
            search.advance(&doc, &mut budget, &mut Vec::new()),
            Err(LayoutError::StaleContinuation)
        );
        assert_eq!(budget.used(), WorkLimit::default());
        let payload = if block {
            read(doc.math_block_payload(BlockId(1)).unwrap())
        } else {
            read(doc.math_payload(MathId(1)).unwrap())
        };
        assert_eq!(
            payload,
            if block {
                "  x\n  中🙂\n"
            } else {
                "open\n  x\n  中🙂"
            }
        );
        assert_eq!(doc.maths().len(), if block { 1 } else { 2 });
    }
}

#[test]
fn deep_million_byte_math_appends_do_not_revisit_prefixes_or_history() {
    for block in [false, true] {
        for (opening, prefix) in [
            (
                format!("{}- ", ">".repeat(1000)),
                format!("{}\t", ">".repeat(1000)),
            ),
            ("- ".repeat(251), "\t".repeat(126)),
        ] {
            let mut doc = Document::new();
            doc.append(&format!(
                "{opening}{}\n{prefix}{}",
                if block { "$$" } else { "$a" },
                "x".repeat(1_000_000)
            ))
            .unwrap();
            let before = doc.work();
            let arena = doc.container_arena_stats();
            let annotations = doc.annotation_arena_stats();
            for _ in 0..1000 {
                doc.append("x").unwrap();
            }
            assert_eq!(doc.work().container_steps, before.container_steps);
            assert_eq!(doc.work().container_bytes, before.container_bytes);
            assert_eq!(doc.work().classified_bytes, before.classified_bytes);
            assert_eq!(doc.work().heading_steps, before.heading_steps);
            assert_eq!(
                doc.work().inline_bytes - before.inline_bytes,
                if block { 0 } else { 1000 }
            );
            assert_eq!(doc.container_arena_stats(), arena);
            // Blocks invalidate their growing content range on every append:
            // the existing segment index may allocate O(new bytes) endpoints.
            let extra_nodes = if block { 2100 } else { 100 };
            assert!(doc.annotation_arena_stats().nodes < annotations.nodes + extra_nodes);
            assert!(doc.work().tree_visits - before.tree_visits < 100_000);
            let before = doc.work();
            doc.append(if block { "\n" } else { "$\n" }).unwrap();
            if block {
                doc.append(&format!("{prefix}$$\n")).unwrap();
            }
            doc.finish();
            assert!(doc.work().inline_bytes - before.inline_bytes < 10);
            assert!(doc.work().tree_visits - before.tree_visits < 2000);
            let payload = if block {
                doc.math_block_payload(BlockId(0))
            } else {
                doc.math_payload(MathId(0))
            };
            let text = read(payload.unwrap());
            assert!(text.contains(&"x".repeat(1_001_000)));
            assert!(!text.contains('\t'));
        }
    }
}

#[test]
fn folded_math_tabs_skip_million_byte_body_and_navigation_restores_source() {
    struct Sink(usize);
    impl LayoutSink for Sink {
        fn begin_tile(&mut self, _: TileId) {}
        fn text(&mut self, _: RenderRun<'_>) {
            panic!("hidden body read");
        }
        fn object(&mut self, _: LinkId, _: &LinkInfo) {
            panic!("hidden object read");
        }
        fn end_tile(&mut self, _: TileId) -> u64 {
            1
        }
    }
    impl FoldLayoutSink for Sink {
        fn placeholder(&mut self, _: FoldPlaceholder) {
            self.0 += 1;
        }
    }
    for block in [false, true] {
        let doc = parse(&if block {
            format!("- $$\n\t{}\n\t$$\n", "x".repeat(1_000_000))
        } else {
            format!("- $a\n\t{}$\n", "x".repeat(1_000_000))
        });
        let container = FoldId::Container(doc.blocks()[0].container.unwrap());
        let id = if block {
            FoldId::Math(BlockId(0))
        } else {
            container
        };
        let mut view = FoldView::new(&doc, 1).unwrap();
        view.set_collapsed(&doc, id, true).unwrap();
        let mut pass = LayoutPass::folded(&doc, &view, TileId(0), 1).unwrap();
        let mut budget = Budget::new(WorkLimit {
            queries: 10,
            objects: 4,
            tiles: 2,
            ..Default::default()
        });
        let mut sink = Sink(0);
        assert_eq!(
            pass.advance_folded(&doc, &mut view, &mut budget, &mut sink)
                .unwrap(),
            Completion::Covered
        );
        assert_eq!(budget.used().bytes, 0);
        assert_eq!(sink.0, 1);
        let mut budget = Budget::new(WorkLimit::unlimited());
        view.set_preview(&doc, Some(id), &mut budget).unwrap();
        assert!(view.is_collapsed(id));
        view.set_preview(&doc, None, &mut budget).unwrap();
        view.set_collapsed(&doc, container, true).unwrap();
        let tab = doc.source().find('\t').unwrap();
        let mut nav = FoldNavigation::new(
            &doc,
            &view,
            &Navigator::new(),
            NavAction::Source(tab),
            FoldNavigationPolicy::Reveal,
            10,
        )
        .unwrap();
        let progress = nav.advance(&doc, &mut view, &mut budget).unwrap();
        assert_eq!(progress.completion, Completion::Covered);
        assert_eq!(progress.target.unwrap().source, Some(tab));
        assert!(!view.is_collapsed(id));
        assert!(!view.is_collapsed(container));
    }
}
