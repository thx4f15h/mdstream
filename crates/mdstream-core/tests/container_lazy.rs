use mdstream::*;

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

fn style(doc: &Document, offset: usize) -> Style {
    let width = doc.source()[offset..].chars().next().unwrap().len_utf8();
    doc.runs_in(Span::new(offset, offset + width))
        .unwrap()
        .next()
        .unwrap()
        .style
}

#[test]
fn omitted_and_partial_prefixes_keep_the_innermost_identity_and_inline_state() {
    for (prefix, middle, end) in [
        ("> ", "", "> "),
        (">>> ", "> ", ""),
        ("- ", " ", "  "),
        ("> - > ", "> ", ""),
        ("- - ", "  ", ""),
        ("> 12. [x] ", "", ">     "),
    ] {
        let source = format!("{prefix}**中\n{middle}two\n{end}🙂**\n");
        let doc = parse(&source);
        let scope = doc.blocks()[0].container.unwrap();
        assert_eq!(doc.blocks().len(), 3, "{source:?}");
        assert!(doc.blocks().iter().all(|b| b.container == Some(scope)));
        for word in ["中", "two", "🙂"] {
            assert!(style(&doc, source.find(word).unwrap()).contains(Style::STRONG));
        }
        assert_eq!(doc.container(scope).unwrap().source.end, source.len());
        assert!(visible(&doc).contains("two\n🙂\n"));
        let mut offset = 0;
        for (i, line) in source.split_inclusive('\n').enumerate() {
            assert_eq!(
                doc.blocks()[i].source,
                Span::new(offset, offset + line.len())
            );
            assert_eq!(doc.block_at(offset), Some(BlockId(i)));
            offset += line.len();
        }
    }
}

#[test]
fn block_interruptions_close_only_the_unmatched_path_before_parsing() {
    for payload in [
        "\n",
        "# h\n",
        "```rs\n",
        "~~~\n",
        "---\n",
        "* * *\n",
        "[^n]: raw\n",
        "- next\n",
        "1. next\n",
        "+ next\n",
    ] {
        for retained in ["", "> "] {
            let first = "> > *open\n";
            let doc = parse(&format!("{first}{retained}{payload}"));
            assert_eq!(
                doc.container(ContainerId(1)).unwrap().source.end,
                first.len(),
                "{retained:?}{payload:?}"
            );
            assert!(!style(&doc, first.find("open").unwrap()).contains(Style::EMPHASIS));
            if retained.is_empty() && !payload.starts_with('>') {
                assert_eq!(
                    doc.container(ContainerId(0)).unwrap().source.end,
                    first.len()
                );
            }
        }
    }
    let doc = parse("> - *open\n> > nested\n");
    assert_eq!(doc.container(ContainerId(2)).unwrap().source.end, 10);
    assert_ne!(doc.blocks()[1].container, Some(ContainerId(2)));
}

#[test]
fn only_nonempty_paragraphs_allow_lazy_continuation() {
    for first in [
        ">\n",
        "-\n",
        "- [x]\n",
        "> # h\n",
        "> title\n> ===\n",
        "> ```\n",
        "> ```\n> raw\n> ```\n",
        ">     code\n",
        "> [r]: /url\n",
        "> [^n]: raw\n",
        "> a|b\n> -|-\n",
        "> a\n> \n",
        "- a\n  \n",
    ] {
        let source = format!("{first}outside\n");
        let doc = parse(&source);
        assert_eq!(doc.blocks().last().unwrap().container, None, "{source:?}");
        assert_eq!(
            doc.container(ContainerId(0)).unwrap().source.end,
            first.len()
        );
    }
}

#[test]
fn noninterrupting_candidates_and_missing_quote_indentation_remain_text() {
    for payload in [
        "#no-space",
        "####### h",
        "``bad",
        "```bad`info",
        "[r]: /url",
        "[r]: <bad<target>",
        "[ ]: /url",
        "[\u{2003}]: /url",
        "[^bad label]: raw",
        "2.no-space",
        "1234567890. too-many-digits",
        "===",
        "    # indented",
        "\t# tab",
    ] {
        let source = format!("> *open\n{payload}\nend*\n");
        let doc = parse(&source);
        assert!(
            doc.blocks()
                .iter()
                .all(|b| b.container == Some(ContainerId(0))),
            "{source:?}"
        );
        assert!(doc.headings().entries().is_empty());
        assert!(doc.references().iter().all(|r| r.destination.is_none()));
        assert!(doc.footnotes().iter().all(|f| f.definition.is_none()));
        if payload == "===" || payload.starts_with("    ") || payload.starts_with('\t') {
            assert!(visible(&doc).contains(payload), "{source:?}");
        } else {
            let explicit = parse(&format!("> *open\n> {payload}\n> end*\n"));
            assert_eq!(visible(&doc), visible(&explicit), "{source:?}");
        }
    }
}

#[test]
fn pending_tail_crlf_breaks_and_eof_preserve_physical_leaves() {
    for ending in ["\n", "\r\n"] {
        let mut doc = Document::new();
        let first = format!("> **one\\{ending}");
        doc.append(&first).unwrap();
        let leaf = doc.blocks()[0].clone();
        let key = doc.tile_key(TileId(0)).unwrap();
        let pending = doc.append("two**").unwrap();
        assert_eq!(pending.container_end, first.len());
        assert!(pending.closed_containers.is_empty());
        assert!(!style(&doc, first.len() - 1).contains(Style::HARD_BREAK));
        let done = doc.finish();
        assert_eq!(doc.blocks()[0], leaf);
        assert_eq!(doc.blocks()[1].container, leaf.container);
        assert_eq!(done.closed_containers, [ContainerId(0)]);
        assert!(style(&doc, first.len() - 1).contains(Style::HARD_BREAK));
        assert_ne!(doc.tile_key(TileId(0)).unwrap(), key);
        assert_eq!(visible(&doc), "one\ntwo");
        let before = doc.work();
        assert!(doc.finish().closed_containers.is_empty());
        assert_eq!(doc.work(), before);
    }
}

#[test]
fn lazy_table_looking_lines_do_not_confirm_headers_or_extend_tables() {
    let doc = parse("> *a|b\n|--|--|\nend*\n");
    assert!(doc.tables().is_empty());
    assert_eq!(visible(&doc), "a|b\n|--|--|\nend\n");
    assert!(style(&doc, 3).contains(Style::EMPHASIS));
    let doc = parse("> start\nlazy|header\n> -|-\n");
    assert!(doc.tables().is_empty());
    let first = "> a|b\n> -|-\n> x|y\n";
    let doc = parse(&format!("{first}outside|table\n"));
    assert_eq!(doc.tables().len(), 1);
    assert_eq!(doc.tables()[0].source.end, first.len());
    assert_eq!(doc.blocks().last().unwrap().container, None);
    // An explicit header may still begin immediately after lazy prose.
    let doc = parse("> first\nlazy\n> a|b\n> -|-\n");
    assert_eq!(doc.tables().len(), 1);
    assert_eq!(doc.blocks()[2].container, Some(ContainerId(0)));
}

#[test]
fn lazy_equals_is_text_but_later_explicit_setext_promotes_the_sequence() {
    let doc = parse("> *first\nlast*\n> ===\n");
    assert_eq!(doc.headings().entries().len(), 1);
    assert_eq!(doc.headings().entries()[0].block, BlockId(0));
    assert_eq!(doc.headings().entries()[0].container, Some(ContainerId(0)));
    assert!(style(&doc, 3).contains(Style::EMPHASIS));
    let doc = parse("> title\n===\n> next\n");
    assert!(doc.headings().entries().is_empty());
    assert_eq!(visible(&doc), "title\n===\nnext\n");
    let doc = parse("> title\n---\nout\n");
    assert_eq!(doc.blocks()[1].kind, BlockKind::ThematicBreak);
    assert_eq!(doc.blocks()[2].container, None);
}

#[test]
fn classification_window_commits_once_including_failed_and_incomplete_candidates() {
    for start in ["[r]: /", "#", "~"] {
        let mut doc = Document::new();
        doc.append("> first\n").unwrap();
        doc.append(&format!("{start}{}", "x".repeat(BLOCK_LOOKAHEAD)))
            .unwrap();
        assert_eq!(doc.blocks()[1].container, Some(ContainerId(0)));
        let before = doc.work();
        for _ in 0..1000 {
            doc.append("🙂").unwrap();
        }
        let after = doc.work();
        assert_eq!(after.inline_bytes - before.inline_bytes, 4000);
        assert_eq!(after.classified_bytes, before.classified_bytes);
        assert_eq!(after.container_bytes, before.container_bytes);
        assert_eq!(after.container_steps, before.container_steps);
        doc.append("\r\n").unwrap();
        doc.finish();
        assert!(doc.references().iter().all(|r| r.destination.is_none()));
    }
    // Whitespace-only bounded prefixes cannot retain a paragraph, even if
    // later bytes would have supplied nonblank content.
    let doc = parse(&format!("> first\n{}tail\n", " ".repeat(BLOCK_LOOKAHEAD)));
    assert_eq!(doc.blocks()[1].container, None);
    assert!(matches!(doc.blocks()[1].kind, BlockKind::Code { .. }));
}

#[test]
fn thousand_missing_prefixes_cost_one_step_per_line_and_no_ancestor_updates() {
    let mut doc = Document::new();
    doc.append(&format!("{}*first\n", ">".repeat(1000)))
        .unwrap();
    let before = doc.work();
    for _ in 0..4096 {
        let update = doc.append("x\n").unwrap();
        assert!(update.new_containers.is_empty());
        assert!(update.closed_containers.is_empty());
    }
    assert_eq!(doc.work().container_steps - before.container_steps, 4096);
    assert!(doc.work().container_bytes - before.container_bytes < 4096 * 16);
    doc.append(&"x".repeat(1_000_000)).unwrap();
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("x").unwrap();
    }
    assert_eq!(doc.work().inline_bytes - before.inline_bytes, 1000);
    assert_eq!(doc.work().container_steps, before.container_steps);
    assert_eq!(doc.work().classified_bytes, before.classified_bytes);
    assert_eq!(doc.work().container_bytes, before.container_bytes);
    doc.append("*\n").unwrap();
    assert!(style(&doc, 1001).contains(Style::EMPHASIS));
    let before = doc.work();
    let closed = doc.append("\n").unwrap();
    assert_eq!(closed.closed_containers.len(), 1000);
    assert_eq!(doc.work().container_steps - before.container_steps, 1001);
    assert_eq!(doc.container_count(), 1000);
}

#[test]
fn partial_deep_list_prefixes_scan_only_the_supplied_indentation() {
    let mut doc = Document::new();
    doc.append(&format!("{}first\n", "- ".repeat(250))).unwrap();
    for matched in [0, 1, 100, 249, 250] {
        let before = doc.work();
        let line = format!("{}tail\n", "  ".repeat(matched));
        for _ in 0..50 {
            doc.append(&line).unwrap();
        }
        assert_eq!(
            doc.work().container_steps - before.container_steps,
            (matched + usize::from(matched < 250)) as u64 * 50
        );
        assert!(doc.work().container_bytes - before.container_bytes < line.len() as u64 * 500);
        assert_eq!(
            doc.blocks().last().unwrap().container,
            Some(ContainerId(499))
        );
    }
    assert_eq!(doc.container_count(), 500);
}

#[test]
fn streaming_ranges_and_closures_match_independent_lazy_quote_model() {
    let mut doc = Document::new();
    let mut expected: Vec<Container> = Vec::new();
    let mut reconstructed: Vec<Container> = Vec::new();
    let mut path = Vec::<usize>::new();
    let mut eligible = false;
    let mut seed = 741_u64;
    let mut merged = Update::default();
    for _ in 0..1200 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let depth = (seed >> 32) as usize % 17;
        let interrupt = seed & 7 == 0;
        let line = format!(
            "{}{}\n",
            ">".repeat(depth),
            if interrupt { "# h" } else { "text" }
        );
        let old_end = doc.source().len();
        let desired = if eligible && !interrupt {
            depth.max(path.len())
        } else {
            depth
        };
        while path.len() > desired {
            let old = path.pop().unwrap();
            expected[old].state = SyntaxState::Sealed;
            expected[old].source.end = old_end;
        }
        while path.len() < desired {
            let parent = path.last().copied().map(ContainerId);
            path.push(expected.len());
            expected.push(Container {
                parent,
                depth: path.len(),
                kind: ContainerKind::Quote,
                source: Span::new(old_end, old_end),
                state: SyntaxState::Growing,
                list_tightness: None,
            });
        }
        let update = doc.append(&line).unwrap();
        for &id in &path {
            expected[id].source.end = doc.source().len();
        }
        for i in update.new_containers.clone() {
            reconstructed.push(doc.container(ContainerId(i)).unwrap());
        }
        for &id in &update.closed_containers {
            reconstructed[id.0] = doc.container(id).unwrap();
        }
        for node in &mut reconstructed {
            if node.state == SyntaxState::Growing {
                node.source.end = update.container_end;
            }
        }
        assert_eq!(reconstructed, expected);
        assert_eq!(
            doc.blocks().last().unwrap().container,
            path.last().copied().map(ContainerId)
        );
        eligible = !interrupt && !path.is_empty();
        merged.merge(update).unwrap();
    }
    merged.merge(doc.finish()).unwrap();
    assert_eq!(merged.closed_containers.len(), expected.len());
    for id in path {
        expected[id].state = SyntaxState::Sealed;
    }
    assert_eq!(
        (0..doc.container_count())
            .map(|i| doc.container(ContainerId(i)).unwrap())
            .collect::<Vec<_>>(),
        expected
    );
    assert!(doc.work().container_steps < 3 * doc.source().len() as u64);
}

#[test]
fn lazy_closure_invalidates_cached_layout_and_preserves_fold_intentions() {
    let mut doc = Document::new();
    let first = doc.append(&format!("> **{}\n", "x".repeat(2000))).unwrap();
    let mut heights = HeightIndex::default();
    let mut checkpoints = CheckpointIndex::<usize>::new();
    apply_layout_update(&doc, &first, &mut heights, &mut checkpoints).unwrap();
    let ticket = heights.ticket(TileId(0)).unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let fold = FoldId::Container(ContainerId(0));
    view.set_collapsed(&doc, fold, true).unwrap();
    let leaf = doc.blocks()[0].clone();
    let update = doc.append("tail**\n").unwrap();
    apply_layout_update(&doc, &update, &mut heights, &mut checkpoints).unwrap();
    view.apply(&doc, &update).unwrap();
    assert!(heights.measure(ticket, 99).is_err());
    assert_eq!(doc.blocks()[0], leaf);
    assert!(view.is_collapsed(fold));
    let mut budget = Budget::new(WorkLimit::unlimited());
    assert!(matches!(
        view.hidden_at(&doc, doc.source().len() - 2, &mut budget)
            .unwrap(),
        FoldLookup::Hidden(_)
    ));
    let update = doc.append("# outside\n").unwrap();
    view.apply(&doc, &update).unwrap();
    assert_eq!(
        view.hidden_at(&doc, doc.source().len() - 2, &mut budget)
            .unwrap(),
        FoldLookup::Visible
    );
    view.set_collapsed(&doc, fold, false).unwrap();
    assert!(visible(&doc).ends_with("tail\noutside\n"));
}

#[test]
fn display_budget_resumes_across_lazy_and_explicit_code_span_prefixes() {
    let source = "> > ` 中\r\n> 🙂\r\nlast\r\n> > `\n";
    let doc = parse(source);
    let expected = "中 🙂 last\n";
    assert_eq!(visible(&doc), expected);
    let mut cursor = doc.display_cursor(Span::new(0, source.len())).unwrap();
    let mut text = String::new();
    let mut complete = false;
    let limit = WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 8,
        ..Default::default()
    };
    for _ in 0..200 {
        let mut budget = Budget::new(limit);
        loop {
            match cursor.next(&doc, &mut budget).unwrap() {
                DisplayStep::Run(run) => {
                    if !run.style.contains(Style::HIDDEN) {
                        text.push_str(run.text);
                    }
                }
                DisplayStep::BudgetExhausted => break,
                DisplayStep::Complete => {
                    complete = true;
                    break;
                }
            }
        }
        assert!(budget.used().bytes <= limit.bytes);
        assert!(budget.used().queries <= limit.queries);
        if complete {
            break;
        }
    }
    assert!(complete);
    assert_eq!(text, expected);
}

#[test]
fn folded_lazy_growth_refreshes_one_tail_and_reads_no_hidden_megabyte() {
    struct Sink;
    impl LayoutSink for Sink {
        fn begin_tile(&mut self, _: TileId) {}
        fn text(&mut self, _: RenderRun<'_>) {
            panic!("hidden body was read");
        }
        fn object(&mut self, _: LinkId, _: &LinkInfo) {
            panic!("hidden object was read");
        }
        fn end_tile(&mut self, _: TileId) -> u64 {
            1
        }
    }
    impl FoldLayoutSink for Sink {
        fn placeholder(&mut self, _: FoldPlaceholder) {}
    }
    let mut doc = Document::new();
    for depth in 1..=500 {
        doc.append(&format!("{}# H\n", ">".repeat(depth))).unwrap();
    }
    doc.append(&format!("{}body\n", ">".repeat(500))).unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    view.set_collapsed(&doc, FoldId::Container(ContainerId(0)), true)
        .unwrap();
    let before = doc.work();
    let containers = view.container_index_visits();
    let sections = view.section_index_visits();
    let ranges = view.range_visits();
    for _ in 0..1000 {
        let update = doc.append("lazy\n").unwrap();
        view.apply(&doc, &update).unwrap();
    }
    assert_eq!(doc.work().container_steps - before.container_steps, 1000);
    assert_eq!(doc.work().heading_steps - before.heading_steps, 1000);
    assert_eq!(view.range_visits() - ranges, 1000);
    assert_eq!(view.container_index_visits(), containers);
    assert_eq!(view.section_index_visits(), sections);
    let update = doc.append(&"x".repeat(1_000_000)).unwrap();
    view.apply(&doc, &update).unwrap();
    let mut pass = LayoutPass::folded(&doc, &view, TileId(0), 10).unwrap();
    let mut budget = Budget::new(WorkLimit {
        queries: 16,
        tiles: 2,
        bytes: 32,
        objects: 4,
        ..Default::default()
    });
    assert_eq!(
        pass.advance_folded(&doc, &mut view, &mut budget, &mut Sink)
            .unwrap(),
        Completion::EndOfDocument
    );
    assert_eq!(budget.used().bytes, 0);
    let end = doc.source().len() + 1;
    let update = doc.append("\n---\n").unwrap();
    view.apply(&doc, &update).unwrap();
    assert_eq!(update.closed_containers.len(), 500);
    assert_eq!(
        doc.headings()
            .section(HeadingId(499), doc.source().len())
            .unwrap()
            .end,
        end
    );
}
