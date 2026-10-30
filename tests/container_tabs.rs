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
fn tabbed_list_markers_and_continuations_use_physical_four_column_stops() {
    let source = "-\t```\n\t中🙂\n\t```\n-\t# sibling\n";
    let doc = parse(source);
    assert_eq!(doc.container_count(), 3);
    assert_eq!(doc.blocks().len(), 2);
    assert!(matches!(doc.blocks()[0].kind, BlockKind::Code { .. }));
    assert!(matches!(doc.blocks()[1].kind, BlockKind::Heading { .. }));
    assert_eq!(doc.blocks()[0].source.end, source.find("-\t#").unwrap());
    assert_eq!(visible(&doc), "中🙂\nsibling\n");
}

#[test]
fn nested_tabs_use_absolute_columns_instead_of_parent_relative_stops() {
    let doc = parse("> \t-\tfirst\n> \t\t# next\n> \t-\tlast\n");
    assert_eq!(doc.container_count(), 4);
    assert_eq!(doc.blocks()[0].container, doc.blocks()[1].container);
    assert_ne!(doc.blocks()[1].container, doc.blocks()[2].container);
    assert_eq!(doc.headings().entries().len(), 1);
    assert_eq!(visible(&doc), "first\nnext\nlast\n");
}

#[test]
fn a_quote_consumes_a_tab_separator_only_when_it_is_one_column() {
    let doc = parse("  >\t# title\n  >\ttext\n");
    assert_eq!(doc.container_count(), 1);
    assert_eq!(doc.headings().entries().len(), 1);
    assert_eq!(visible(&doc), "title\ntext\n");
    let doc = parse(">\t# literal\n");
    assert!(doc.headings().entries().is_empty());
    assert_eq!(visible(&doc), "\t# literal\n");
}

#[test]
fn partial_tabs_keep_lazy_rules_and_extend_existing_code_fences() {
    for whitespace in ["\t", " \t"] {
        let doc = parse(&format!("- first\n{whitespace}# literal\n"));
        assert_eq!(doc.blocks()[0].container, doc.blocks()[1].container);
        assert!(doc.headings().entries().is_empty());
        assert_eq!(visible(&doc), format!("first\n{whitespace}# literal\n"));
    }
    let doc = parse("- ```\n\tpayload\n");
    assert_eq!(doc.blocks().len(), 1);
    assert!(doc.blocks()[0].container.is_some());
    assert_eq!(visible(&doc), "  payload\n");
}

#[test]
fn ordered_siblings_tasks_and_empty_items_keep_their_own_column_widths() {
    let source = "1.\t[x]\tfirst\n\t# one\n123.\t[ ]\tsecond\n\t\t# two\n9.\n  # empty\n";
    let doc = parse(source);
    assert_eq!(doc.container_count(), 4);
    assert_eq!(doc.headings().entries().len(), 3);
    for (block, number, checked) in [(0, 1, Some(true)), (2, 123, Some(false)), (4, 9, None)] {
        let item = doc.blocks()[block].container.unwrap();
        assert_eq!(doc.blocks()[block + 1].container, Some(item));
        let ContainerKind::Item { number: n, task } = doc.container(item).unwrap().kind else {
            panic!("expected item")
        };
        assert_eq!(n, Some(number));
        assert_eq!(task.map(|t| t.checked), checked);
        if let Some(task) = task {
            assert_eq!(
                doc.text(task.source),
                Some(if task.checked { "[x]" } else { "[ ]" })
            );
        }
    }
}

#[test]
fn aligned_container_prefixes_leave_leaf_block_dialects_unchanged() {
    let source = "-\t# heading\n\tfirst\n\tsecond\n\t===\n\t    code\n\t[r]: /url\n\t[^n]: first\n\t    second\n\t$$\n\tmath\n\t$$\n\th|h\n\t-|-\n\tx|y\n";
    let doc = parse(source);
    let item = doc.blocks()[0].container;
    assert!(item.is_some());
    assert!(doc.blocks().iter().all(|b| b.container == item));
    assert_eq!(doc.headings().entries().len(), 2);
    assert_eq!(doc.tables().len(), 1);
    assert_eq!(doc.references().len(), 1);
    assert!(
        doc.blocks()
            .iter()
            .any(|b| matches!(b.kind, BlockKind::Code { .. }))
    );
    let footnote = doc.footnote(FootnoteId(0)).unwrap().definition.unwrap();
    assert_eq!(doc.text(footnote.content), Some("first\n\t    second"));
    let math = doc
        .blocks()
        .iter()
        .find(|b| matches!(b.kind, BlockKind::Math { .. }))
        .unwrap();
    assert!(matches!(math.kind, BlockKind::Math { closed: true, .. }));
    // Tabs left in the payload are not a new block-level indentation feature.
    let doc = parse("-\tfirst\n\t\t# literal\n\t\t[^n]: literal\n\t\t```\n");
    assert!(doc.headings().entries().is_empty());
    assert!(doc.footnotes().iter().all(|f| f.definition.is_none()));
    assert!(
        !doc.blocks()
            .iter()
            .any(|b| matches!(b.kind, BlockKind::Code { .. }))
    );
    assert_eq!(
        visible(&doc),
        "first\n\t# literal\n\t[^n]: literal\n\t```\n"
    );
    let doc = parse("\t- literal\n1234567890.\tliteral\n-\tx\n\t\n-\ty\n");
    assert_eq!(doc.container_count(), 3);
    assert_eq!(
        doc.container(ContainerId(0)).unwrap().list_tightness,
        Some(ListTightness::Loose)
    );
}

#[test]
fn long_table_delimiters_share_the_column_matcher_and_failed_probes_keep_source() {
    for valid in [true, false] {
        let delimiter = format!("\t{}{}|-\n", "-".repeat(4096), if valid { "" } else { "x" });
        let doc = parse(&format!("-\th|h\n{delimiter}\tx|y\n"));
        assert_eq!(doc.container_count(), 2);
        assert!(
            doc.blocks()
                .iter()
                .all(|b| b.container == Some(ContainerId(1)))
        );
        if valid {
            assert_eq!(doc.tables().len(), 1);
            let table = &doc.tables()[0];
            assert_eq!(doc.text(table.delimiter), Some(delimiter.as_str()));
            assert_eq!(doc.text(table.cell(1, 0).unwrap().source), Some("x"));
        } else {
            assert!(doc.tables().is_empty());
            assert!(visible(&doc).contains(&format!("{}x|-", "-".repeat(4096))));
        }
    }
    let doc = parse(&format!("- h|h\n\t{}|-\n", "-".repeat(4096)));
    assert!(doc.tables().is_empty()); // Partial tab cannot match this exact scope.
}

// An independent oracle expands whitespace prefixes, then finds the byte that
// reaches/passes the required column. Fences disable lazy
// continuation so accepted/rejected indentation is directly observable.
fn column(text: &str, mut at: usize) -> usize {
    for c in text.chars() {
        at = if c == '\t' { (at / 4 + 1) * 4 } else { at + 1 };
    }
    at
}

#[test]
fn whitespace_combinations_match_independent_column_and_range_model() {
    let mut cases = 0;
    for parent in ["", "> "] {
        for marker in ["-", "1.", "123.", "123456789."] {
            for gap in [" ", "\t", " \t", "\t ", "  \t", " \t  "] {
                let open = format!("{parent}{marker}{gap}```\n");
                let target = column(&format!("{parent}{marker}{gap}"), 0);
                for bits in 0..64 {
                    let space: String = (0..6)
                        .map(|n| if bits & (1 << n) == 0 { ' ' } else { '\t' })
                        .collect();
                    let consumed = (0..=space.len())
                        .find(|&end| column(&space[..end], parent.len()) >= target);
                    let text = format!("{open}{parent}{space}payload\n");
                    let doc = parse(&text);
                    let code = &doc.blocks()[0];
                    assert!(matches!(code.kind, BlockKind::Code { .. }));
                    assert_eq!(
                        code.source.end,
                        if consumed.is_some() {
                            text.len()
                        } else {
                            open.len()
                        },
                        "{text:?}"
                    );
                    if let Some(bytes) = consumed {
                        assert_eq!(
                            visible(&doc),
                            format!(
                                "{}{}payload\n",
                                " ".repeat(column(&space[..bytes], parent.len()) - target),
                                &space[bytes..]
                            ),
                            "{text:?}"
                        );
                        assert_eq!(doc.blocks().len(), 1);
                    }
                    cases += 1;
                }
            }
        }
    }
    assert_eq!(cases, 3072);
}

#[test]
fn nested_tab_runs_are_linear_and_lazy_mismatch_never_walks_the_remaining_path() {
    for depth in [64, 128, 250] {
        let mut doc = Document::new();
        doc.append(&format!("{}first\n", "-\t".repeat(depth)))
            .unwrap();
        let item = doc.blocks()[0].container;
        let prefix = "\t".repeat(depth);
        let before = doc.work();
        for _ in 0..100 {
            doc.append(&format!("{prefix}next\n")).unwrap();
        }
        assert_eq!(
            doc.work().container_steps - before.container_steps,
            200 * depth as u64 // partial-tab probe, then original whole-byte path
        );
        assert!(
            doc.work().container_bytes - before.container_bytes < 100 * (6 * depth as u64 + 30)
        );
        assert_eq!(doc.container_count(), 2 * depth);
        assert!(doc.blocks().iter().all(|b| b.container == item));
        let before = doc.work();
        for _ in 0..100 {
            doc.append("\tlazy\n").unwrap();
        }
        assert_eq!(doc.work().container_steps - before.container_steps, 400);
        assert_eq!(doc.work().heading_steps - before.heading_steps, 100);
        assert_eq!(doc.container_count(), 2 * depth);
        // An ordinary continuation can now share a tab between item frames.
        let doc = parse("- - first\n\tstill lazy\n");
        assert_eq!(doc.container_count(), 4);
        assert_eq!(visible(&doc), "first\nstill lazy\n");
    }
}

#[test]
fn million_byte_growth_has_no_prefix_ancestor_or_history_work() {
    for open in [format!("{}-\t", ">".repeat(1000)), "-\t".repeat(250)] {
        let mut doc = Document::new();
        doc.append(&format!("{open}{}", "x".repeat(1_000_000)))
            .unwrap();
        let before = doc.work();
        let arena = doc.container_arena_stats();
        for _ in 0..1000 {
            let update = doc.append("x").unwrap();
            assert!(update.new_containers.is_empty());
            assert!(update.closed_containers.is_empty());
        }
        assert_eq!(doc.work().container_steps, before.container_steps);
        assert_eq!(doc.work().container_bytes, before.container_bytes);
        assert_eq!(doc.work().classified_bytes, before.classified_bytes);
        assert_eq!(doc.work().heading_steps, before.heading_steps);
        assert_eq!(doc.work().inline_bytes - before.inline_bytes, 1000);
        assert_eq!(doc.container_arena_stats(), arena);
    }
}

#[test]
fn tab_prefix_mapping_and_display_reads_charge_source_bytes_and_pause_atomically() {
    let doc = parse("-\t$中\r\n\t🙂$ [label\n\tend](u)\n");
    assert_eq!(doc.link_destination(LinkId(0)), Some("u"));
    assert_eq!(
        doc.text(doc.link_info(LinkId(0)).unwrap().label),
        Some("label\n\tend")
    );
    let math = doc.math(MathId(0)).unwrap();
    assert_eq!(doc.text(math.content), Some("中\r\n\t🙂"));
    let mut cursor = doc.math_payload(MathId(0)).unwrap();
    let mut empty = Budget::new(WorkLimit::default());
    assert_eq!(cursor.advance(&mut empty), Err(Completion::BudgetExhausted));
    assert_eq!(cursor.source_position(), math.content.start);
    assert_eq!(empty.used(), WorkLimit::default());
    let mut output = String::new();
    let mut charged = 0;
    let mut position = math.content.start;
    loop {
        let mut budget = Budget::new(WorkLimit {
            bytes: 4,
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        let Some(chunk) = cursor.advance(&mut budget).unwrap() else {
            break;
        };
        assert_eq!(chunk.source.start, position);
        assert_eq!(budget.used().bytes, chunk.source.len());
        if !chunk.text.is_empty() {
            assert_eq!(doc.text(chunk.source), Some(chunk.text));
        }
        output.push_str(chunk.text);
        charged += budget.used().bytes;
        position = chunk.source.end;
    }
    assert_eq!(output, "中\r\n🙂");
    assert_eq!(position, math.content.end);
    assert_eq!(charged, math.content.len());

    let span = Span::new(0, doc.source().len());
    let mut reader = doc.display_cursor(span).unwrap();
    assert_eq!(
        reader.next(&doc, &mut empty).unwrap(),
        DisplayStep::BudgetExhausted
    );
    assert_eq!(reader.source_position(), 0);
    let mut output = String::new();
    let mut charged = 0;
    loop {
        let mut budget = Budget::new(WorkLimit {
            bytes: 8,
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        match reader.next(&doc, &mut budget).unwrap() {
            DisplayStep::Run(run) => {
                if !run.style.contains(Style::HIDDEN) {
                    output.push_str(run.text);
                }
            }
            DisplayStep::Complete => break,
            step => panic!("unexpected {step:?}"),
        }
        charged += budget.used().bytes;
    }
    assert_eq!(output, visible(&doc));
    assert!(charged >= span.len());
    assert!(!output.contains('\t'));
}

#[test]
fn streamed_scope_changes_preserve_ids_fold_intentions_and_merged_updates() {
    let mut doc = Document::new();
    let mut batch = doc.append("-\t```\n\tfirst\n").unwrap();
    let item = doc.blocks()[0].container.unwrap();
    let fold = FoldId::Container(item);
    let mut view = FoldView::new(&doc, 1).unwrap();
    view.set_collapsed(&doc, fold, true).unwrap();
    let ticket = view.heights().ticket(TileId(0)).unwrap();
    let mut reader = doc
        .display_cursor(Span::new(0, doc.source().len()))
        .unwrap();
    let mut heights = HeightIndex::default();
    heights.apply(&doc, &batch).unwrap();
    for chunk in ["\t", "second", "\n\t```\n", "-\t# sibling\n"] {
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
    assert_eq!(
        reader
            .next(&doc, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap_err(),
        LayoutError::StaleContinuation
    );
    assert_eq!(doc.container_count(), 3);
    assert_eq!(batch.new_blocks, 0..2);
    assert_eq!(batch.new_containers, 0..3);
    let mut rebuilt = HeightIndex::default();
    rebuilt.apply(&doc, &batch).unwrap();
    assert_eq!(heights.total_rows(), rebuilt.total_rows());
    assert_eq!(
        doc.container(item).unwrap().source.end,
        doc.source().find("-\t#").unwrap()
    );
}
