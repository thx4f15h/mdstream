use mdstream::*;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn visible(doc: &Document, span: Span) -> String {
    doc.display_runs_in(span)
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect()
}

#[test]
fn atx_tabs_keep_scope_and_source_offsets_without_feeding_hidden_columns() {
    let source = "- *old\n\t## 中🙂 &amp; [x](u) ##\r\n\t### next\n- sibling\n";
    let doc = parse(source);
    assert_eq!(doc.headings().entries().len(), 2);
    let heading = doc.headings().entries()[0];
    assert_eq!(heading.container, doc.blocks()[0].container);
    assert_eq!(heading.block, BlockId(1));
    assert_eq!(heading.source.start, source.find('\t').unwrap());
    assert_eq!(heading.content.start, source.find('中').unwrap());
    assert_eq!(visible(&doc, heading.content), "中🙂 & x");
    assert_eq!(doc.links().len(), 1);
    assert!(
        doc.runs_in(heading.content)
            .unwrap()
            .all(|r| !r.style.contains(Style::EMPHASIS))
    );
    assert_eq!(doc.headings().parent(HeadingId(1)), Some(HeadingId(0)));
    assert_eq!(
        doc.headings()
            .section(HeadingId(0), source.len())
            .unwrap()
            .end,
        source.find("- sibling").unwrap()
    );
    assert_eq!(doc.source(), source);
}

#[test]
fn partial_fence_openers_interrupt_paragraphs_and_preserve_literal_payloads() {
    for marker in ["```rs", "~~~rs", "$$"] {
        let close = if marker == "$$" { "$$" } else { &marker[..3] };
        let source = format!("- old\n\t{marker}\n\t*中* &amp; | $x$\r\n\t{close}\n- sibling\n");
        let doc = parse(&source);
        let block = &doc.blocks()[1];
        assert_eq!(block.container, doc.blocks()[0].container);
        assert!(matches!(
            block.kind,
            BlockKind::Code { .. } | BlockKind::Math { closed: true, .. }
        ));
        assert_eq!(
            visible(&doc, block.source),
            if marker == "$$" {
                "  *中* &amp; | $x$\n"
            } else {
                "*中* &amp; | $x$\n"
            }
        );
        assert!(doc.maths().is_empty());
        assert!(doc.tables().is_empty());
        assert_eq!(block.source.end, source.find("- sibling").unwrap());
    }
}

#[test]
fn virtual_plus_real_spaces_choose_indented_code_before_heading_fence_or_table() {
    for (prefix, spaces) in [("\t", 2), (" \t", 2)] {
        for text in [
            "# title",
            "```rs",
            "$$",
            "a | b",
            "-|-",
            "[^n]: body",
            "> quote",
        ] {
            let source = format!("- old\n{prefix}{}{text}\n", " ".repeat(4 - spaces));
            let doc = parse(&source);
            let block = &doc.blocks()[1];
            assert_eq!(block.kind, BlockKind::Code { info: None }, "{source:?}");
            assert_eq!(block.container, doc.blocks()[0].container);
            assert_eq!(visible(&doc, block.source), format!("{text}\n"));
            assert!(doc.headings().entries().is_empty());
            assert!(doc.tables().is_empty());
            assert!(doc.footnotes().is_empty());
        }
    }
}

#[test]
fn column_thresholds_match_expanded_space_model_for_all_selected_leaves() {
    let mut cases = 0;
    for (open, prefix, expanded, leftover) in [
        ("- ", "\t", "    ", 2),
        ("1. ", "\t", "    ", 1),
        ("> ", ">\t", ">   ", 2),
        ("   > ", "   >\t", "   >    ", 3),
        ("- - ", "\t", "    ", 0),
        ("- - - ", "\t\t", "        ", 2),
        ("- > ", "\t>\t", "    >   ", 2),
        ("> > ", ">\t>\t", ">   >   ", 2),
    ] {
        for padding in 0..6 {
            for leaf in ["# *中* &amp;", "```rs", "~~~rs", "$$"] {
                for ending in ["\n", "\r\n", ""] {
                    let tail = format!("{}{leaf}{ending}", " ".repeat(padding));
                    let source = format!("{open}old\n{prefix}{tail}");
                    let doc = parse(&source);
                    let model = parse(&format!("{open}old\n{expanded}{tail}"));
                    let block = &doc.blocks()[1];
                    assert_eq!(block.container, doc.blocks()[0].container, "{source:?}");
                    assert_eq!(
                        std::mem::discriminant(&block.kind),
                        std::mem::discriminant(&model.blocks()[1].kind),
                        "{source:?}"
                    );
                    assert_eq!(
                        visible(&doc, block.source),
                        visible(&model, model.blocks()[1].source),
                        "{source:?}"
                    );
                    if leftover + padding >= 4 {
                        assert_eq!(block.kind, BlockKind::Code { info: None });
                        assert_eq!(
                            visible(&doc, block.source),
                            format!(
                                "{}{leaf}{}",
                                " ".repeat(leftover + padding - 4),
                                if ending.is_empty() { "" } else { "\n" }
                            )
                        );
                    }
                    if let BlockKind::Code { info: Some(info) } = block.kind {
                        assert_eq!(doc.text(info), Some("rs"));
                    }
                    cases += 1;
                }
            }
        }
    }
    assert_eq!(cases, 576);
}

#[test]
fn new_leaves_interrupt_pending_and_confirmed_tables_without_consuming_cells() {
    for lead in ["- a | b\n", "- a | b\n  -|-\n  x|y\n"] {
        for leaf in ["# h | h", "```rs", "$$", "  -|-", "  a|b"] {
            let source = format!("{lead}\t{leaf}\n");
            let doc = parse(&source);
            let block = doc.blocks().last().unwrap();
            assert_eq!(block.container, doc.blocks()[0].container);
            assert_eq!(block.source.start, lead.len());
            assert!(
                matches!(
                    block.kind,
                    BlockKind::Heading { .. } | BlockKind::Code { .. } | BlockKind::Math { .. }
                ),
                "{source:?}"
            );
            assert_eq!(doc.tables().len(), usize::from(lead.contains("-|-")));
            if let Some(table) = doc.tables().first() {
                assert_eq!(table.source.end, lead.len());
                assert_eq!(table.rows().len(), 2);
            }
        }
    }
    // An arbitrarily long separator-looking code line cannot activate the
    // long-separator probe, or replay its buffered candidate as table cells.
    let source = format!("- a|b\n\t  {}|-\n", "-".repeat(1_000_000));
    let doc = parse(&source);
    assert!(doc.tables().is_empty());
    assert_eq!(doc.blocks()[1].kind, BlockKind::Code { info: None });
    assert_eq!(doc.work().table_candidate_bytes, 0);
    assert!(doc.work().inline_bytes < 20);
    let title = parse("- first\n\t===\n");
    assert_eq!(title.headings().entries()[0].block, BlockId(0));
    assert_eq!(
        title.headings().entries()[0].container,
        title.blocks()[0].container
    );
    let definition = parse("- first\n\t[r]: /u\n");
    assert_eq!(
        definition.blocks()[1].container,
        definition.blocks()[0].container
    );
    assert_eq!(definition.blocks()[1].kind, BlockKind::Paragraph);
    assert!(
        definition
            .references()
            .iter()
            .all(|r| r.destination.is_none())
    );
    let footnote = parse("- first\n\t[^n]: body\n");
    let definition = footnote
        .footnote(FootnoteId(0))
        .unwrap()
        .definition
        .unwrap();
    assert_eq!(footnote.text(definition.content), Some("body"));
    assert_eq!(
        footnote.block(definition.block).unwrap().container,
        footnote.blocks()[0].container
    );
    // Fully matched partial-tab headers now use the existing table path.
    {
        let source = "- first\n\ta|b\n  -|-\n";
        let doc = parse(source);
        assert_eq!(doc.tables().len(), 1);
        assert!(doc.headings().entries().is_empty());
        assert!(doc.references().iter().all(|r| r.destination.is_none()));
        assert!(doc.footnotes().iter().all(|f| f.definition.is_none()));
    }
}

#[test]
fn failed_paths_new_containers_blank_and_unhandled_leaf_tabs_are_atomic() {
    // Completed short blanks, including EOF, now consume the Item indent.
    for source in ["- first\n\t\n", "- first\n\t"] {
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

    let nested = parse("- first\n\t- nested\n");
    assert_eq!(nested.container_count(), 4);
    assert_eq!(
        visible(&nested, Span::new(0, nested.source().len())),
        "first\nnested\n"
    );
    let quote = parse("- first\n\t> nested\n");
    assert_eq!(quote.container_count(), 3);
    assert_eq!(
        visible(&quote, Span::new(0, quote.source().len())),
        "first\nnested\n"
    );
    assert_eq!(
        quote.container(ContainerId(2)).unwrap().parent,
        Some(ContainerId(1))
    );
    let heading = parse(">\t # three columns after the quote separator\n");
    assert_eq!(heading.headings().entries().len(), 1);
    assert_eq!(
        visible(&heading, Span::new(0, heading.source().len())),
        "three columns after the quote separator\n"
    );
    for (source, expected, heading) in [
        (
            "- > first\n\t# missing quote\n",
            "first\nmissing quote\n",
            true,
        ),
        ("- > first\n\t```\n", "first\n", false),
        ("- first\n\t\t# literal\n", "first\n  # literal\n", false),
    ] {
        let doc = parse(source);
        assert_eq!(
            doc.headings().entries().len(),
            usize::from(heading),
            "{source:?}"
        );
        assert_eq!(visible(&doc, Span::new(0, source.len())), expected);
        assert_eq!(doc.blocks()[1].container, Some(ContainerId(1)));
    }
    let doc = parse("- first\n\t  raw\n  \n\t  next\n- sibling\n\t# sibling title\n");
    assert_eq!(doc.blocks()[1].kind, BlockKind::Code { info: None });
    assert_eq!(doc.blocks()[3].kind, BlockKind::Code { info: None });
    assert_ne!(
        doc.blocks()[1].container,
        doc.headings().entries()[0].container
    );
    assert_eq!(
        doc.blocks()[4].container,
        doc.headings().entries()[0].container
    );
    assert_eq!(
        doc.container(ContainerId(0)).unwrap().list_tightness,
        Some(ListTightness::Loose)
    );
}

#[test]
fn indentation_code_merges_and_new_leaves_after_nontext_blocks_keep_scope() {
    let source = "- [r]: /u\n\t# [r]\n\t  *raw*\n\t   &amp;\n\t```rs\n\tcode\n\t```\n\t# after\n\t$$\n\tx\n\t$$\n\t## last\n";
    let doc = parse(source);
    assert!(
        doc.blocks()
            .iter()
            .all(|b| b.container == doc.blocks()[0].container)
    );
    assert_eq!(doc.blocks().len(), 7);
    assert_eq!(visible(&doc, doc.blocks()[2].source), "*raw*\n &amp;\n");
    assert_eq!(doc.headings().entries().len(), 3);
    assert_eq!(doc.links().len(), 1);
    assert_eq!(doc.references().len(), 1);
    assert_eq!(doc.headings().parent(HeadingId(2)), Some(HeadingId(1)));
}

#[test]
fn classification_limits_and_failed_markers_keep_one_pass_semantics() {
    for candidate in [
        "####### invalid".to_owned(),
        "#nospace".to_owned(),
        "```bad`".to_owned(),
        format!("{}bad`", "`".repeat(4096)),
        format!("$${}", " ".repeat(BLOCK_LOOKAHEAD)),
        format!("{}# late", "x".repeat(BLOCK_LOOKAHEAD)),
    ] {
        let source = format!("- old\n\t{candidate}\n");
        let doc = parse(&source);
        assert!(doc.headings().entries().is_empty(), "{source:?}");
        assert_eq!(
            doc.blocks()
                .iter()
                .any(|b| matches!(b.kind, BlockKind::Code { .. } | BlockKind::Math { .. })),
            candidate.starts_with("$$ ")
        );
        assert!(doc.work().classified_bytes < 6 * BLOCK_LOOKAHEAD as u64);
    }
    for leaf in ["# ", "```rs", "  "] {
        let source = format!("- old\n\t{leaf}{}\n", "x".repeat(10_000));
        let doc = parse(&source);
        assert_eq!(doc.blocks()[1].container, doc.blocks()[0].container);
        assert!(matches!(
            doc.blocks()[1].kind,
            BlockKind::Heading { .. } | BlockKind::Code { .. }
        ));
        assert!(doc.work().classified_bytes < 6 * BLOCK_LOOKAHEAD as u64);
    }
}

#[test]
fn generated_nested_prefixes_match_space_expansion_without_source_rewriting() {
    let mut seed = 0x1eaf_ab12_u64;
    for case in 0..400 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let depth = (seed >> 32) as usize % 20 + 1;
        let parent = "> ".repeat((seed >> 48) as usize % 10);
        let target = parent.len() + depth * 2;
        let mut column = parent.len();
        let mut prefix = parent.clone();
        while column < target {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            if seed & 16 == 0 {
                prefix.push(' ');
                column += 1;
            } else {
                prefix.push('\t');
                column = (column / 4 + 1) * 4;
            }
        }
        let expanded = format!("{parent}{}", " ".repeat(column - parent.len()));
        let open = format!("{parent}{}old\n", "- ".repeat(depth));
        let (leaf, closer) = [
            ("# *中* &amp; [x](u)", ""),
            ("```rs", "```"),
            ("~~~rs", "~~~"),
            ("$$", "$$"),
            ("    # raw &amp;", ""),
        ][case % 5];
        let tail = |p: &str| {
            if closer.is_empty() {
                format!("{p}{leaf}\r\n")
            } else {
                format!("{p}{leaf}\r\n{p}中 &amp; $x$\r\n{p}{closer}\n")
            }
        };
        let source = format!("{open}{}", tail(&prefix));
        let doc = parse(&source);
        let model = parse(&format!("{open}{}", tail(&expanded)));
        assert_eq!(
            visible(&doc, Span::new(0, source.len())),
            visible(&model, Span::new(0, model.source().len())),
            "case {case}: {source:?}"
        );
        assert_eq!(doc.container_count(), model.container_count());
        assert_eq!(doc.blocks().len(), model.blocks().len());
        for (block, expected) in doc.blocks().iter().zip(model.blocks()) {
            assert_eq!(block.container, expected.container);
            assert_eq!(
                std::mem::discriminant(&block.kind),
                std::mem::discriminant(&expected.kind)
            );
        }
        assert_eq!(doc.links().len(), model.links().len());
        assert_eq!(
            doc.headings().entries().len(),
            model.headings().entries().len()
        );
        assert!(doc.work().container_steps < 5 * source.len() as u64);
        assert!(doc.work().container_bytes < 12 * source.len() as u64);
        assert_eq!(doc.source(), source);
    }
}

#[test]
fn deep_million_byte_leaf_growth_does_not_revisit_prefixes_or_ancestors() {
    for (opening, prefix) in [
        (
            format!("{}- ", ">".repeat(1000)),
            format!("{}\t", ">".repeat(1000)),
        ),
        ("- ".repeat(251), "\t".repeat(126)),
    ] {
        for leaf in ["# ", "   ", "```rs\n", "$$\n"] {
            let mut doc = Document::new();
            let line = format!("{prefix}{leaf}");
            doc.append(&format!(
                "{opening}old\n{line}{}{}",
                if leaf.ends_with('\n') { &prefix } else { "" },
                "x".repeat(1_000_000)
            ))
            .unwrap();
            assert_eq!(doc.blocks()[1].container, doc.blocks()[0].container);
            assert!(matches!(
                doc.blocks()[1].kind,
                BlockKind::Heading { .. } | BlockKind::Code { .. } | BlockKind::Math { .. }
            ));
            let before = doc.work();
            let containers = doc.container_arena_stats();
            for _ in 0..1000 {
                doc.append("x").unwrap();
            }
            assert_eq!(doc.work().container_steps, before.container_steps);
            assert_eq!(doc.work().container_bytes, before.container_bytes);
            assert_eq!(doc.work().classified_bytes, before.classified_bytes);
            assert_eq!(doc.work().heading_steps, before.heading_steps);
            assert_eq!(
                doc.work().inline_bytes - before.inline_bytes,
                if leaf == "# " { 1000 } else { 0 }
            );
            assert_eq!(doc.container_arena_stats(), containers);
            assert_eq!(doc.blocks().len(), 2);
            doc.finish();
        }
    }
}

#[test]
fn repeated_deep_leaves_and_historical_headings_have_bounded_prefix_work() {
    let depth = 250;
    let mut doc = Document::new();
    doc.append(&format!("{}old\n", "- ".repeat(depth))).unwrap();
    let prefix = "\t".repeat(depth / 2);
    let line = format!("{prefix}# h\n");
    for _ in 0..200 {
        let before = doc.work();
        let update = doc.append(&line).unwrap();
        assert!(update.new_containers.is_empty());
        assert!(update.closed_containers.is_empty());
        assert_eq!(
            doc.work().container_steps - before.container_steps,
            depth as u64
        );
        assert!(doc.work().container_bytes - before.container_bytes < 4 * line.len() as u64);
    }
    // Missing the first necessary prefix inspects only that mismatch, then
    // uses the existing lazy fallback; it cannot mutate a partial leaf path.
    doc.append(&format!("{}text\n", " ".repeat(depth * 2)))
        .unwrap();
    let before = doc.work();
    doc.append(" \ttext\n").unwrap();
    assert!(doc.work().container_steps - before.container_steps < 6);

    let mut doc = Document::new();
    doc.append("- old\n").unwrap();
    for _ in 0..4096 {
        doc.append("\t# h &amp;\n").unwrap();
    }
    let before = doc.work();
    doc.append("\t## last\n").unwrap();
    assert_eq!(doc.work().container_steps - before.container_steps, 1);
    assert!(doc.work().heading_steps - before.heading_steps < 4);
    let heading = doc.headings().entries().last().unwrap();
    let mut runs = doc.display_runs_in(heading.content).unwrap();
    assert_eq!(runs.by_ref().map(|r| r.text).collect::<String>(), "last");
    assert!(runs.annotation_visits() < 200);
}

#[test]
fn display_budget_search_and_excerpt_preserve_hidden_and_retained_tab_maps() {
    // One-column opener removes one of the continuation tab's two virtual columns.
    let doc = parse("- old\n\t# 中🙂\n   ```\n\t中🙂\r\n\t```\n\t   中🙂\n");
    let source = Span::new(0, doc.source().len());
    let expected = visible(&doc, source);
    assert_eq!(expected, "old\n中🙂\n 中🙂\n 中🙂\n");
    let tiled: String = (0..doc.tile_count())
        .flat_map(|i| doc.display_runs(TileId(i)).unwrap())
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    let scalars: String = doc
        .source()
        .char_indices()
        .flat_map(|(at, c)| {
            doc.display_runs_in(Span::new(at, at + c.len_utf8()))
                .unwrap()
        })
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    assert_eq!(tiled, expected);
    assert_eq!(scalars, expected);
    let mut reader = doc.display_cursor(source).unwrap();
    let mut result = String::new();
    let mut charged = 0;
    loop {
        let at = reader.source_position();
        if at < source.end {
            let mut empty = Budget::new(WorkLimit::default());
            assert_eq!(
                reader.next(&doc, &mut empty).unwrap(),
                DisplayStep::BudgetExhausted
            );
            assert_eq!(empty.used(), WorkLimit::default());
            assert_eq!(reader.source_position(), at);
        }
        let mut budget = Budget::new(WorkLimit {
            bytes: 6,
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        match reader.next(&doc, &mut budget).unwrap() {
            DisplayStep::Run(run) => {
                if !run.style.contains(Style::HIDDEN) {
                    result.push_str(run.text);
                }
            }
            DisplayStep::Complete => break,
            _ => panic!("one scalar fits"),
        }
        charged += budget.used().bytes;
    }
    // DisplayCursor charges its bounded source window and two lookaround
    // bytes; semantic run boundaries may emit less than that window.
    assert!(charged >= source.len());
    assert!(charged <= 6 * source.len());
    assert_eq!(result, expected);
    let mapped = doc.source().find("\t中").unwrap();
    let mut search = DisplaySearch::new(&doc, " ").unwrap();
    let mut hits = Vec::new();
    let mut budget = Budget::new(WorkLimit::unlimited());
    search.advance(&doc, &mut budget, &mut hits).unwrap();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].source, Span::new(mapped, mapped + 1));
    assert_eq!(doc.text(hits[1].source), Some(" "));
    let mut excerpt = DisplayExcerpt::new(
        &doc,
        Span::new(mapped, mapped + 1),
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
    assert_eq!(run.replacement, Some(Span::new(mapped, mapped + 1)));
    // Heading indentation is hidden, with no synthetic output or replacement.
    let at = doc.headings().entries()[0].source.start;
    let run = doc
        .display_runs_in(Span::new(at, at + 1))
        .unwrap()
        .next()
        .unwrap();
    assert!(run.style.contains(Style::HIDDEN));
    assert_eq!(run.replacement, None);
}

#[test]
fn streaming_leaf_confirmation_keeps_ids_fold_choices_and_merged_updates() {
    let mut doc = Document::new();
    let mut batch = doc.append("- [old](u)\n").unwrap();
    let old = *doc.link_info(LinkId(0)).unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    view.set_defaults(FoldDefaults {
        heading_levels: [true; 6],
        code: true,
        math: true,
        ..Default::default()
    });
    let ticket = view.heights().ticket(TileId(0)).unwrap();
    let mut reader = doc
        .display_cursor(Span::new(0, doc.source().len()))
        .unwrap();
    let mut search = DisplaySearch::new(&doc, "old").unwrap();
    let mut heights = HeightIndex::default();
    heights.apply(&doc, &batch).unwrap();
    for chunk in [
        "\t",
        "# [new](v)",
        "\r",
        "\n",
        "\t```rs\n",
        "\t中",
        "🙂\n",
        "\t```\n",
        "\t$$\n",
        "\tx\n",
        "\t$$\n",
        "- sibling\n",
    ] {
        let update = doc.append(chunk).unwrap();
        view.apply(&doc, &update).unwrap();
        heights.apply(&doc, &update).unwrap();
        batch.merge(update).unwrap();
        assert_eq!(*doc.link_info(LinkId(0)).unwrap(), old);
    }
    let update = doc.finish();
    view.apply(&doc, &update).unwrap();
    heights.apply(&doc, &update).unwrap();
    batch.merge(update).unwrap();
    let mut rebuilt = HeightIndex::default();
    rebuilt.apply(&doc, &batch).unwrap();
    assert_eq!(rebuilt.total_rows(), heights.total_rows());
    assert_eq!(doc.links().len(), 2);
    assert_eq!(doc.headings().entries()[0].block, BlockId(1));
    for id in [
        FoldId::Heading(HeadingId(0)),
        FoldId::Code(BlockId(2)),
        FoldId::Math(BlockId(3)),
    ] {
        assert!(view.is_collapsed(id));
    }
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
}

#[test]
fn folded_new_leaves_skip_large_bodies_and_navigation_retains_source_anchors() {
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
    for (leaf, close, fold) in [
        ("# h\n  ", "", FoldId::Heading(HeadingId(0))),
        ("  ", "", FoldId::Code(BlockId(1))),
        ("```\n\t", "\t```\n", FoldId::Code(BlockId(1))),
        ("$$\n\t", "\t$$\n", FoldId::Math(BlockId(1))),
    ] {
        // Align the hidden body with a tile to check its mask, not its parent.
        let heading = matches!(fold, FoldId::Heading(_));
        let doc = parse(&format!(
            "- {}\n\t{leaf}{}\n{close}",
            "a".repeat(TILE_BYTES - 3 - if heading { 5 } else { 0 }),
            "x".repeat(1_000_000)
        ));
        assert_eq!(doc.fold_candidate(fold).unwrap().hidden.start, TILE_BYTES);
        let mut view = FoldView::new(&doc, 1).unwrap();
        view.set_collapsed(&doc, fold, true).unwrap();
        let mut pass = LayoutPass::folded(&doc, &view, TileId(1), 1).unwrap();
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
            Completion::Covered,
            "{fold:?}: {:?}",
            budget.used()
        );
        assert_eq!(budget.used().bytes, 0);
        assert_eq!(sink.0, 1);
        let mut budget = Budget::new(WorkLimit::unlimited());
        view.set_preview(&doc, Some(fold), &mut budget).unwrap();
        assert!(view.is_collapsed(fold));
        view.set_preview(&doc, None, &mut budget).unwrap();
        let parent = FoldId::Container(doc.blocks()[0].container.unwrap());
        view.set_collapsed(&doc, parent, true).unwrap();
        let target = doc.source().find('x').unwrap();
        let mut navigation = FoldNavigation::new(
            &doc,
            &view,
            &Navigator::new(),
            NavAction::Source(target),
            FoldNavigationPolicy::Reveal,
            10,
        )
        .unwrap();
        let progress = navigation.advance(&doc, &mut view, &mut budget).unwrap();
        assert_eq!(progress.completion, Completion::Covered);
        assert_eq!(progress.target.unwrap().source, Some(target));
        assert!(!view.is_collapsed(parent));
        assert!(!view.is_collapsed(fold));
    }
}
