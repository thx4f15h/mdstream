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
fn partial_item_tab_preserves_code_scope_and_maps_remaining_columns() {
    let source = "- ```\n\t中🙂\n\t```\n";
    let doc = parse(source);
    assert_eq!(doc.blocks().len(), 1);
    assert_eq!(doc.blocks()[0].source, Span::new(0, source.len()));
    assert_eq!(visible(&doc), "  中🙂\n");
    let tab = source.find('\t').unwrap();
    let run = doc
        .display_runs_in(Span::new(tab, tab + 1))
        .unwrap()
        .next()
        .unwrap();
    assert_eq!(run.source, Span::new(tab, tab + 1));
    assert_eq!(run.replacement, Some(run.source));
    assert_eq!(run.text, "  ");
    assert!(!run.style.contains(Style::HIDDEN));
    assert!(run.style.contains(Style::CODE));
    assert_eq!(doc.text(run.source), Some("\t"));
}

#[test]
fn virtual_columns_participate_in_closing_fence_classification() {
    let doc = parse("- ```\n\t  ```\n\t ```\nafter\n");
    assert_eq!(visible(&doc), "    ```\nafter\n");
    assert_eq!(doc.blocks()[0].source.end, "- ```\n\t  ```\n\t ```\n".len());
}

#[test]
fn nested_frames_share_tabs_and_quote_separators_retain_their_remainder() {
    for (source, expected) in [
        ("- - ```\n\tpayload\n\t```\n", "payload\n"),
        ("- - - ```\n\t\tpayload\n\t\t```\n", "  payload\n"),
        ("> ```\n>\tpayload\n>\t```\n", "  payload\n"),
        ("> > ```\n>\t>payload\n>\t>```\n", "payload\n"),
        ("- > ```\n\t>\tpayload\n\t>\t```\n", "  payload\n"),
        ("- > ```\n \t> payload\n \t> ```\n", "payload\n"),
        (
            "- ~~~~\r\n\t&copy; $x$ [a](u)\r\n\t~~~~~\r\n",
            "  &copy; $x$ [a](u)\n",
        ),
    ] {
        let doc = parse(source);
        assert_eq!(doc.blocks().len(), 1, "{source:?}");
        assert_eq!(doc.blocks()[0].source.end, source.len());
        assert_eq!(visible(&doc), expected, "{source:?}");
        assert!(doc.links().is_empty());
        assert!(doc.maths().is_empty());
    }
}

#[test]
fn failed_complete_path_is_atomic_and_other_leaf_dialects_are_unchanged() {
    for (source, expected) in [
        ("- > ```\n\tpayload\n", "\tpayload\n"),
        ("- > ```\n \tpayload\n", " \tpayload\n"),
        ("- > ```\n  outside\n", "outside\n"),
        ("- ```\n- sibling\n", "sibling\n"),
        ("- > $$\n\tpayload\n", "\tpayload\n"),
        ("- first\n\t# literal\n", "first\n\t# literal\n"),
        (">\t# literal\n", "\t# literal\n"),
    ] {
        let doc = parse(source);
        assert_eq!(visible(&doc), expected, "{source:?}");
        assert!(
            doc.display_runs_in(Span::new(0, source.len()))
                .unwrap()
                .all(|r| r.replacement.is_none())
        );
    }
}

#[test]
fn unfinished_blank_and_overindented_fences_keep_literal_mapping() {
    for (source, expected) in [
        ("- ```\n\t", "  "),
        ("- ```\n\t\r\n", "  \n"),
        ("- ```\n\t\n\t```", "  \n"),
        ("- ```\n\t``", "  ``"),
        ("- ```\n\t``` suffix\n", "  ``` suffix\n"),
        ("- ```\n\t\t```\n", "  \t```\n"),
        ("- ```\n\t  ```\n", "    ```\n"),
        ("- ~~~\n\t~~~\t\r\n", ""),
    ] {
        let doc = parse(source);
        assert_eq!(visible(&doc), expected, "{source:?}");
        assert_eq!(doc.blocks().len(), 1);
    }
}

#[test]
fn mapped_tabs_are_owned_by_one_source_byte_and_budget_steps_are_atomic() {
    let doc = parse(&format!("- ```\n  {}\n\t中🙂\n\t```\n", "x".repeat(247)));
    let whole = Span::new(0, doc.source().len());
    let tab = doc.source().find('\t').unwrap();
    assert_eq!(tab, 256); // replacement starts exactly at a tile boundary
    let expected = visible(&doc);
    let tiles: String = (0..doc.tile_count())
        .flat_map(|i| doc.display_runs(TileId(i)).unwrap())
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    assert_eq!(tiles, expected);
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
    assert_eq!(scalars, expected);
    let mut reader = doc.display_cursor(whole).unwrap();
    let mut output = String::new();
    let mut charged = 0;
    loop {
        let position = reader.source_position();
        let mut empty = Budget::new(WorkLimit::default());
        if position < whole.end {
            assert_eq!(
                reader.next(&doc, &mut empty).unwrap(),
                DisplayStep::BudgetExhausted
            );
            assert_eq!(reader.source_position(), position);
            assert_eq!(empty.used(), WorkLimit::default());
        }
        let mut budget = Budget::new(WorkLimit {
            bytes: 6,
            objects: 1,
            queries: 1,
            ..Default::default()
        });
        match reader.next(&doc, &mut budget).unwrap() {
            DisplayStep::Run(run) => {
                assert_eq!(run.source.start, position);
                assert!(run.source.end > position);
                if !run.style.contains(Style::HIDDEN) {
                    output.push_str(run.text);
                }
            }
            DisplayStep::Complete => break,
            _ => panic!("no progress"),
        }
        charged += budget.used().bytes;
    }
    assert_eq!(output, expected);
    assert!(charged >= whole.len());
    assert_eq!(reader.source_position(), whole.end);
}

#[test]
fn display_search_maps_each_virtual_space_back_to_the_same_tab() {
    let doc = parse("- ```\n\t中\n\t```\n");
    let tab = doc.source().find('\t').unwrap();
    for (query, expected) in [
        (" ", vec![Span::new(tab, tab + 1); 2]),
        ("  中", vec![Span::new(tab, tab + 4)]),
    ] {
        let mut search = DisplaySearch::new(&doc, query).unwrap();
        let mut hits = Vec::new();
        let mut complete = false;
        for _ in 0..200 {
            let mut budget = Budget::new(WorkLimit {
                bytes: 6,
                objects: 1,
                queries: 1,
                ..Default::default()
            });
            if search.advance(&doc, &mut budget, &mut hits).unwrap() == Completion::EndOfDocument {
                complete = true;
                break;
            }
        }
        assert!(complete);
        assert_eq!(hits.iter().map(|h| h.source).collect::<Vec<_>>(), expected);
    }
}

#[test]
fn long_growth_never_revisits_prefixes_or_allocates_per_virtual_column() {
    for open in [
        format!("{}- ```\n{}\t", ">".repeat(1000), ">".repeat(1000)),
        format!("{}```\n{}", "- ".repeat(250), "\t".repeat(125)),
    ] {
        let mut doc = Document::new();
        doc.append(&format!("{open}{}", "x".repeat(1_000_000)))
            .unwrap();
        assert_eq!(doc.blocks().len(), 1);
        let before = doc.work();
        let arena = doc.container_arena_stats();
        let annotations = doc.annotation_arena_stats();
        for _ in 0..1000 {
            let update = doc.append("x").unwrap();
            assert!(update.new_containers.is_empty());
            assert!(update.closed_containers.is_empty());
        }
        assert_eq!(doc.work().container_steps, before.container_steps);
        assert_eq!(doc.work().container_bytes, before.container_bytes);
        assert_eq!(doc.work().classified_bytes, before.classified_bytes);
        assert_eq!(doc.work().heading_steps, before.heading_steps);
        assert_eq!(doc.work().inline_bytes, before.inline_bytes);
        assert_eq!(doc.container_arena_stats(), arena);
        assert_eq!(doc.annotation_arena_stats().nodes, annotations.nodes);
    }
}

#[test]
fn deep_nested_short_lines_charge_only_provided_prefix_and_stop_on_mismatch() {
    let depth = 250;
    let mut doc = Document::new();
    doc.append(&format!("{}```\n", "- ".repeat(depth))).unwrap();
    let before = doc.work();
    for _ in 0..200 {
        doc.append(&format!("{}x\n", "\t".repeat(depth / 2)))
            .unwrap();
    }
    assert_eq!(doc.blocks().len(), 1);
    assert_eq!(
        doc.work().container_steps - before.container_steps,
        200 * depth as u64
    );
    assert!(doc.work().container_bytes - before.container_bytes < 200 * (depth + 10) as u64);
    let before = doc.work();
    doc.append("\toutside\n").unwrap();
    assert_eq!(doc.blocks().len(), 2);
    assert!(doc.work().container_steps - before.container_steps < (2 * depth + 10) as u64);
}

#[test]
fn streaming_keeps_ids_fold_intentions_and_invalidates_old_readers() {
    let mut doc = Document::new();
    let mut batch = doc.append("- ```\n\tfirst\n").unwrap();
    let item = doc.blocks()[0].container.unwrap();
    let fold = FoldId::Code(BlockId(0));
    let mut view = FoldView::new(&doc, 1).unwrap();
    view.set_collapsed(&doc, fold, true).unwrap();
    let ticket = view.heights().ticket(TileId(0)).unwrap();
    let mut reader = doc
        .display_cursor(Span::new(0, doc.source().len()))
        .unwrap();
    let mut heights = HeightIndex::default();
    heights.apply(&doc, &batch).unwrap();
    for chunk in ["\t", "中", "🙂\n", "\t```\n", "- sibling\n"] {
        let update = doc.append(chunk).unwrap();
        view.apply(&doc, &update).unwrap();
        heights.apply(&doc, &update).unwrap();
        batch.merge(update).unwrap();
        assert!(view.is_collapsed(fold));
        assert_eq!(doc.blocks()[0].container, Some(item));
    }
    let end = doc.finish();
    view.apply(&doc, &end).unwrap();
    heights.apply(&doc, &end).unwrap();
    batch.merge(end).unwrap();
    assert!(view.measure(ticket, 9).is_err());
    let mut budget = Budget::new(WorkLimit::unlimited());
    assert_eq!(
        reader.next(&doc, &mut budget).unwrap_err(),
        LayoutError::StaleContinuation
    );
    assert_eq!(budget.used(), WorkLimit::default());
    let mut rebuilt = HeightIndex::default();
    rebuilt.apply(&doc, &batch).unwrap();
    assert_eq!(rebuilt.total_rows(), heights.total_rows());
    assert_eq!(doc.blocks().len(), 2);
    assert_eq!(
        doc.blocks()[0].source.end,
        doc.source().find("- sibling").unwrap()
    );
    assert_eq!(visible(&doc), "  first\n  中🙂\nsibling\n");
}

#[test]
fn seeded_nested_columns_match_independent_literal_body_and_fence_range_model() {
    let mut source = String::new();
    let mut expected = Vec::new();
    let mut seed = 0xab12_0319_u64;
    for i in 0..400 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let depth = (seed >> 32) as usize % 20 + 1;
        let quotes = (seed >> 48) as usize % 10;
        let parent = "> ".repeat(quotes);
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
        let residue = column - target;
        let start = source.len();
        source.push_str(&format!("{parent}{}```\n", "- ".repeat(depth)));
        source.push_str(&format!("{prefix}中{i}\r\n{prefix}   ```\n"));
        let display = format!(
            "{}中{i}\n{}",
            " ".repeat(residue),
            if residue == 0 {
                String::new()
            } else {
                format!("{}   ```\n", " ".repeat(residue))
            }
        );
        if residue > 0 {
            source.push_str(&format!("{parent}{}```\n", "  ".repeat(depth)));
        }
        expected.push((Span::new(start, source.len()), display));
        source.push_str("outside\n");
    }
    let doc = parse(&source);
    let codes: Vec<_> = doc
        .blocks()
        .iter()
        .enumerate()
        .filter(|(_, b)| matches!(b.kind, BlockKind::Code { .. }))
        .collect();
    assert_eq!(codes.len(), expected.len());
    for ((id, block), (span, expected)) in codes.into_iter().zip(expected) {
        assert_eq!(block.source, span);
        let display: String = doc
            .display_runs_in(span)
            .unwrap()
            .filter(|r| !r.style.contains(Style::HIDDEN))
            .map(|r| r.text)
            .collect();
        assert_eq!(display, expected);
        assert_eq!(
            doc.fold_candidate(FoldId::Code(BlockId(id)))
                .unwrap()
                .source,
            span
        );
    }
    assert!(doc.work().container_bytes < 8 * source.len() as u64);
    assert!(doc.work().container_steps < 4 * source.len() as u64);
}

#[test]
fn historical_replacements_are_not_revisited_by_new_code_or_local_reads() {
    let mut doc = Document::new();
    doc.append(&"- ```\n\told\n\t```\noutside\n".repeat(4096))
        .unwrap();
    let before = doc.work();
    let start = doc.source().len();
    doc.append("- ```\n\tnew\n\t```\n").unwrap();
    assert!(doc.work().container_steps - before.container_steps < 20);
    assert!(doc.work().container_bytes - before.container_bytes < 60);
    assert!(doc.work().classified_bytes - before.classified_bytes < 60);
    let mut runs = doc
        .display_runs_in(Span::new(start, doc.source().len()))
        .unwrap();
    let mut text = String::new();
    for run in runs.by_ref() {
        if !run.style.contains(Style::HIDDEN) {
            text.push_str(run.text);
        }
    }
    assert_eq!(text, "  new\n");
    assert!(runs.annotation_visits() < 200);
}

#[test]
fn folded_large_code_skips_bytes_and_bounded_excerpt_keeps_partial_mapping() {
    struct Sink(usize);
    impl LayoutSink for Sink {
        fn begin_tile(&mut self, _: TileId) {}
        fn text(&mut self, _: RenderRun<'_>) {
            panic!("fold read body");
        }
        fn object(&mut self, _: LinkId, _: &LinkInfo) {
            panic!("unexpected object");
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
    let doc = parse(&format!("- ```\n\t{}\n\t```\n", "x".repeat(1_000_000)));
    let id = FoldId::Code(BlockId(0));
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
    let tab = doc.source().find('\t').unwrap();
    let mut excerpt = DisplayExcerpt::new(
        &doc,
        doc.blocks()[0].source,
        ExcerptLimits {
            source_bytes: 50,
            display_bytes: 1,
            lines: 1,
        },
    )
    .unwrap();
    let mut output = String::new();
    let mut done = false;
    for _ in 0..100 {
        let mut budget = Budget::new(WorkLimit {
            bytes: 6,
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        match excerpt.next(&doc, &mut budget).unwrap() {
            ExcerptStep::Run(run) => {
                assert_eq!(run.source, Span::new(tab, tab + 1));
                assert_eq!(run.replacement, Some(run.source));
                output.push_str(run.text);
            }
            ExcerptStep::Complete(end) => {
                assert_eq!(end, ExcerptEnd::DisplayLimit);
                done = true;
                break;
            }
            ExcerptStep::BudgetExhausted => (),
        }
    }
    assert!(done);
    assert_eq!(output, " ");
    assert!(view.is_collapsed(id));
}
