use mdtui3::*;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn visible(doc: &Document) -> String {
    doc.display_runs_in(Span::new(0, doc.source().len()))
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect()
}

#[test]
fn prose_retains_scope_and_maps_partial_columns_without_changing_source() {
    for (source, expected) in [
        ("- first\n\t中🙂\n", "first\n  中🙂\n"),
        ("> first\n>\t中\n", "first\n  中\n"),
        ("- - first\n\t中\n", "first\n中\n"),
        ("- - - first\n\t\t中\n", "first\n  中\n"),
        ("- > first\n\t>\t中\n", "first\n  中\n"),
        ("> > first\n>\t>中\n", "first\n中\n"),
        ("- first\r\n \t中\r\n", "first\n  中\n"),
        ("- first\n\t中", "first\n  中"),
        ("1. first\n\t中\n", "first\n 中\n"),
        ("   > first\n   >\t中\n", "first\n   中\n"),
    ] {
        let doc = parse(source);
        assert_eq!(doc.source(), source);
        assert_eq!(visible(&doc), expected, "{source:?}");
        assert_eq!(doc.blocks().len(), 2);
        assert_eq!(doc.blocks()[0].container, doc.blocks()[1].container);
        assert!(doc.tables().is_empty());
    }
}

#[test]
fn emphasis_breaks_entities_and_cross_line_labels_consume_virtual_whitespace() {
    let doc = parse("- *a  \n\tb* &amp; [x\n\ty](u) ![i\n\tj](v)\n");
    assert_eq!(visible(&doc), "a\n  b & x\n  y i\n  j\n");
    assert_eq!(doc.links().len(), 2);
    let tab = doc.source().find('\t').unwrap();
    let run = doc
        .display_runs_in(Span::new(tab, tab + 1))
        .unwrap()
        .next()
        .unwrap();
    assert_eq!(run.text, "  ");
    assert_eq!(run.replacement, Some(run.source));
    assert!(run.style.contains(Style::EMPHASIS));
    let newline = doc.source().find('\n').unwrap();
    assert!(
        doc.display_runs_in(Span::new(newline, newline + 1))
            .unwrap()
            .next()
            .unwrap()
            .style
            .contains(Style::HARD_BREAK)
    );
    let doc = parse("- [a\n\tb][]\n\n[a b]: /ok\n");
    assert_eq!(visible(&doc), "a\n  b\n\n");
    assert!(matches!(doc.links()[0], LinkTarget::Reference(_)));
    assert_eq!(doc.references().len(), 1);
}

#[test]
fn late_code_closure_overlays_spaces_and_trims_one_virtual_column() {
    for (source, expected) in [
        ("- `a\n\tb`\n", "a   b\n"),
        ("- ` a\n\t`\n", "a  \n"),
        ("- ` \n\t`\n", "    \n"),
        ("- ` a\n\t中 `\n", "a   中\n"),
        ("- `a\n\tstill open\n", "`a\n  still open\n"),
        ("- first\n\t` x ` &amp;\n", "first\n  x &\n"),
        ("- `a\r\n\tb`\r\n", "a   b\n"),
        ("1. ` a\n\t`\n", "a \n"),
        ("   > ` a\n   >\t`\n", "a   \n"),
    ] {
        let doc = parse(source);
        assert_eq!(visible(&doc), expected, "{source:?}");
    }
}

#[test]
fn failed_paths_blank_lines_and_new_leaf_candidates_keep_original_dialect() {
    for (source, expected) in [
        ("- > first\n\ttail\n", "first\n\ttail\n"),
        ("- first\n\t# title\n", "first\n\t# title\n"),
        ("- first\n\t- nested\n", "first\n\t- nested\n"),
        ("- first\n\t> nested\n", "first\n\t> nested\n"),
        ("- first\n\t  tail\n", "first\n\t  tail\n"),
        ("- first\n\t\n", "first\n\t\n"),
        ("- first\n\t", "first\n\t"),
        ("- $a\n\tb$\n", "a\n\tb\n"),
        ("- first\n- sibling\n", "first\nsibling\n"),
    ] {
        let doc = parse(source);
        assert_eq!(visible(&doc), expected, "{source:?}");
        assert!(
            doc.display_runs_in(Span::new(0, source.len()))
                .unwrap()
                .all(|r| r.replacement.is_none())
        );
    }
    // Partial text is not a new table candidate. Fully matched Setext may
    // subsequently promote the complete existing paragraph without replay.
    let doc = parse("- first\n\ta | b\n  - | -\n");
    assert!(doc.tables().is_empty());
    let doc = parse("- *first\n\tsecond*\n  ===\n");
    assert_eq!(doc.headings().entries().len(), 1);
    assert_eq!(visible(&doc), "first\n  second\n");
}

#[test]
fn generated_prefixes_and_inline_content_match_explicit_space_model() {
    let mut seed = 0xab12_9183_u64;
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
        let (first, last) = [
            ("*first", "中* &amp; [x](u)"),
            ("` first", "中 ` &amp;"),
            ("[first", "中](u)"),
            ("![first", "中][]"),
            ("` first", "`"),
        ][case % 5];
        let opener = format!("{parent}{}{first}\n", "- ".repeat(depth));
        let source = format!("{opener}{prefix}{last}\r\n");
        let model = format!(
            "{opener}{parent}{}{last}\r\n",
            " ".repeat(column - parent.len())
        );
        let doc = parse(&source);
        let model = parse(&model);
        assert_eq!(visible(&doc), visible(&model), "case {case}: {source:?}");
        assert_eq!(doc.container_count(), model.container_count());
        assert_eq!(doc.blocks()[0].container, doc.blocks()[1].container);
        assert_eq!(doc.links().len(), model.links().len());
        assert_eq!(
            doc.references()
                .iter()
                .map(|r| &r.label)
                .collect::<Vec<_>>(),
            model
                .references()
                .iter()
                .map(|r| &r.label)
                .collect::<Vec<_>>()
        );
        assert!(doc.work().container_steps < 4 * source.len() as u64);
        assert!(doc.work().container_bytes < 12 * source.len() as u64);
    }
}

#[test]
fn virtual_spaces_keep_the_original_tab_mapping_at_tiles_scalars_and_tiny_budgets() {
    let doc = parse(&format!("- `{}\n\t中🙂`\n", "x".repeat(252)));
    let source = Span::new(0, doc.source().len());
    let tab = doc.source().find('\t').unwrap();
    assert_eq!(tab, 256);
    let expected = visible(&doc);
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
    let mut output = String::new();
    loop {
        let position = reader.source_position();
        let mut empty = Budget::new(WorkLimit::default());
        if position < source.end {
            assert_eq!(
                reader.next(&doc, &mut empty).unwrap(),
                DisplayStep::BudgetExhausted
            );
            assert_eq!(reader.source_position(), position);
            assert_eq!(empty.used(), WorkLimit::default());
        }
        let mut budget = Budget::new(WorkLimit {
            bytes: 6,
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        match reader.next(&doc, &mut budget).unwrap() {
            DisplayStep::Run(run) => {
                if run.source.start == tab {
                    assert_eq!(run.source, Span::new(tab, tab + 1));
                    assert_eq!(run.replacement, Some(run.source));
                    assert_eq!(run.text, "  ");
                    assert!(run.style.contains(Style::CODE));
                }
                if !run.style.contains(Style::HIDDEN) {
                    output.push_str(run.text);
                }
            }
            DisplayStep::Complete => break,
            _ => panic!("one scalar must fit"),
        }
    }
    assert_eq!(output, expected);
}

#[test]
fn search_and_excerpt_preserve_mapping_after_partial_code_trim() {
    let doc = parse("- ` a\n\t`\n");
    let tab = doc.source().find('\t').unwrap();
    let mut search = DisplaySearch::new(&doc, " ").unwrap();
    let mut hits = Vec::new();
    let mut completed = false;
    for _ in 0..100 {
        let mut budget = Budget::new(WorkLimit {
            bytes: 6,
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        if search.advance(&doc, &mut budget, &mut hits).unwrap() == Completion::EndOfDocument {
            completed = true;
            break;
        }
    }
    assert!(completed);
    assert_eq!(
        hits.iter().map(|h| h.source).collect::<Vec<_>>(),
        vec![Span::new(tab - 1, tab), Span::new(tab, tab + 1)]
    );
    let mut excerpt = DisplayExcerpt::new(
        &doc,
        Span::new(tab, doc.source().len()),
        ExcerptLimits {
            source_bytes: 20,
            display_bytes: 1,
            lines: 1,
        },
    )
    .unwrap();
    let mut budget = Budget::new(WorkLimit::unlimited());
    let ExcerptStep::Run(run) = excerpt.next(&doc, &mut budget).unwrap() else {
        panic!("mapped space")
    };
    assert_eq!(run.text, " ");
    assert_eq!(run.source, Span::new(tab, tab + 1));
    assert_eq!(run.replacement, Some(run.source));
    assert_eq!(
        excerpt.next(&doc, &mut budget).unwrap(),
        ExcerptStep::Complete(ExcerptEnd::DisplayLimit)
    );
}

#[test]
fn streaming_preserves_objects_fold_intentions_and_merged_update_invalidation() {
    let mut doc = Document::new();
    let mut batch = doc.append("- [old](u) `a\n\tb\n").unwrap();
    let old = *doc.link_info(LinkId(0)).unwrap();
    let item = doc.blocks()[0].container.unwrap();
    let fold = FoldId::Container(item);
    let mut view = FoldView::new(&doc, 1).unwrap();
    view.set_collapsed(&doc, fold, true).unwrap();
    let ticket = view.heights().ticket(TileId(0)).unwrap();
    let range = Span::new(0, doc.source().len());
    let mut reader = doc.display_cursor(range).unwrap();
    let mut search = DisplaySearch::new(&doc, "b").unwrap();
    let mut heights = HeightIndex::default();
    heights.apply(&doc, &batch).unwrap();
    for chunk in ["\t", "中", "🙂` [new](v)\r", "\n", "- sibling\n"] {
        let update = doc.append(chunk).unwrap();
        view.apply(&doc, &update).unwrap();
        heights.apply(&doc, &update).unwrap();
        batch.merge(update).unwrap();
        assert!(view.is_collapsed(fold));
        assert_eq!(*doc.link_info(LinkId(0)).unwrap(), old);
    }
    let end = doc.finish();
    view.apply(&doc, &end).unwrap();
    heights.apply(&doc, &end).unwrap();
    batch.merge(end).unwrap();
    let mut rebuilt = HeightIndex::default();
    rebuilt.apply(&doc, &batch).unwrap();
    assert_eq!(rebuilt.total_rows(), heights.total_rows());
    assert!(view.measure(ticket, 9).is_err());
    let mut budget = Budget::new(WorkLimit::unlimited());
    assert_eq!(
        reader.next(&doc, &mut budget).unwrap_err(),
        LayoutError::StaleContinuation
    );
    assert_eq!(
        search
            .advance(&doc, &mut budget, &mut Vec::new())
            .unwrap_err(),
        LayoutError::StaleContinuation
    );
    assert_eq!(budget.used(), WorkLimit::default());
    assert_eq!(doc.links().len(), 2);
    assert_eq!(
        doc.container(item).unwrap().source.end,
        doc.source().find("- sibling").unwrap()
    );
}

#[test]
fn million_byte_growth_never_rescans_prefixes_ancestors_or_open_inline_state() {
    for (prefix, continuation, first, close) in [
        (
            format!("{}- ", ">".repeat(1000)),
            format!("{}\t", ">".repeat(1000)),
            "*a",
            "b*",
        ),
        ("- ".repeat(250), "\t".repeat(125), "[a", "b](u)"),
        ("- ".to_owned(), "\t".to_owned(), "`a", "b`"),
    ] {
        let mut doc = Document::new();
        doc.append(&format!(
            "{prefix}{first}\n{continuation}{}",
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
        assert_eq!(doc.work().inline_bytes - before.inline_bytes, 1000);
        assert_eq!(doc.container_arena_stats(), arena);
        assert!(doc.annotation_arena_stats().nodes < annotations.nodes + 100);
        let before = doc.work();
        doc.append(&format!("{close}\n")).unwrap();
        doc.finish();
        assert!(doc.work().inline_bytes - before.inline_bytes < 10);
        assert!(doc.work().tree_visits - before.tree_visits < 2000);
    }
}

#[test]
fn deep_partial_paths_only_visit_the_supplied_frames_and_history_is_not_revisited() {
    let depth = 250;
    let mut doc = Document::new();
    doc.append(&format!("{}*a\n", "- ".repeat(depth))).unwrap();
    let before = doc.work();
    for _ in 0..200 {
        doc.append(&format!("{}b\n", "\t".repeat(depth / 2)))
            .unwrap();
    }
    assert_eq!(
        doc.work().container_steps - before.container_steps,
        200 * depth as u64
    );
    assert!(doc.work().container_bytes - before.container_bytes < 200 * (depth + 30) as u64);
    let before = doc.work();
    for _ in 0..200 {
        doc.append("\tlazy\n").unwrap();
    }
    assert_eq!(doc.work().container_steps - before.container_steps, 800);
    assert_eq!(doc.container_count(), 2 * depth);
    let mut doc = Document::new();
    doc.append(&"- `a\n\tb`\n\n".repeat(4096)).unwrap();
    let start = doc.source().len();
    let before = doc.work();
    doc.append("- `a\n\tnew`\n").unwrap();
    assert!(doc.work().container_steps - before.container_steps < 20);
    let mut runs = doc
        .display_runs_in(Span::new(start, doc.source().len()))
        .unwrap();
    let text: String = runs
        .by_ref()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    assert_eq!(text, "a   new\n");
    assert!(runs.annotation_visits() < 200);
}

#[test]
fn folded_million_byte_prose_reads_no_body_and_keeps_preview_intention() {
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
    let doc = parse(&format!("- first\n\t{}\n", "x".repeat(1_000_000)));
    let id = FoldId::Container(doc.blocks()[0].container.unwrap());
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
    assert!(view.is_collapsed(id));
}
