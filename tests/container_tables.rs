use mdtui3::*;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn visible(doc: &Document, span: Span) -> String {
    doc.display_runs_in(span)
        .unwrap()
        .filter(|run| !run.style.contains(Style::HIDDEN))
        .map(|run| run.text)
        .collect()
}

fn tiny() -> Budget {
    Budget::new(WorkLimit {
        bytes: 6,
        queries: 1,
        objects: 1,
        ..Default::default()
    })
}

#[test]
fn nested_tables_keep_physical_rows_exact_scope_and_cell_semantics() {
    let source = "# root\n> - [x] | Name | Count | Note |\r\n>   :-- | --: | :-:\r\n>   **中🙂** | `x\\|y` | &amp;\r\n>   sparse\n>   a|b|c|ignored\n>   > | inner |\n>   > | - |\n>   > value\n>   after\n- sibling\n";
    let doc = parse(source);
    assert_eq!(doc.tables().len(), 2);
    let outer = &doc.tables()[0];
    let inner = &doc.tables()[1];
    assert_eq!(
        doc.block(outer.block).unwrap().container,
        Some(ContainerId(2))
    );
    assert_eq!(
        doc.block(inner.block).unwrap().container,
        Some(ContainerId(3))
    );
    assert_eq!(
        outer
            .columns
            .iter()
            .map(|c| c.alignment)
            .collect::<Vec<_>>(),
        [Alignment::Left, Alignment::Right, Alignment::Center]
    );
    assert_eq!(outer.rows().len(), 4);
    assert_eq!(
        outer.source,
        Span::new(source.find("> -").unwrap(), inner.source.start)
    );
    assert_eq!(inner.source.end, source.find(">   after").unwrap());
    assert_eq!(visible(&doc, outer.row(1).unwrap().source), "中🙂x|y&\n");
    assert_eq!(doc.text(outer.cell(1, 0).unwrap().source), Some("**中🙂**"));
    assert_eq!(doc.text(outer.cell(1, 1).unwrap().source), Some("`x\\|y`"));
    assert_eq!(outer.row(2).unwrap().cells.len(), 1);
    assert_eq!(outer.row(3).unwrap().cells.len(), 3);
    assert!(matches!(
        doc.container(ContainerId(2)).unwrap().kind,
        ContainerKind::Item {
            task: Some(TaskMarker { checked: true, .. }),
            ..
        }
    ));
    for (i, table) in doc.tables().iter().enumerate() {
        assert_eq!(doc.block(table.block).unwrap().source, table.source);
        assert_eq!(doc.block(table.block).unwrap().state, SyntaxState::Sealed);
        for at in table.source.range() {
            assert_eq!(doc.table_at(at), Some(TableId(i)));
            assert_eq!(
                table.row_at(at),
                table
                    .rows()
                    .iter()
                    .position(|row| row.source.range().contains(&at))
            );
        }
        for row in table.rows() {
            assert!(row.complete);
            assert_eq!(&source[row.source.start..row.source.start + 1], ">");
            assert!(row.cells.iter().all(|c| c.source.start > row.source.start));
        }
    }
}

#[test]
fn delimiter_precedence_and_container_boundaries_are_explicit() {
    for source in [
        "> a|b\n> - | -\n",
        "- a|b\n  - | -\n",
        "> - a|b\n>   - | -\n",
        "1. a|b\n   - | -\n",
        "> |a|\n> |-|",
    ] {
        assert_eq!(parse(source).tables().len(), 1, "{source:?}");
    }
    for source in [
        "> a|b\n-|-\n",       // left quote
        "a|b\n> -|-\n",       // entered quote
        "> a|b\n> > -|-\n",   // entered nested quote
        "- a|b\n- -|-\n",     // sibling item
        "- a|b\n  - -|-\n",   // nested item
        "> a|b\n>\n> -|-\n",  // blank
        "> a|b\n> -|-|-\n",   // wrong column count
        "> a|b\n> ::-|--\n",  // invalid alignment
        "> a\\|b\n> -|-\n",   // no unescaped pipe
        "> # a|b\n> -|-\n",   // heading cannot become a header
        ">     a|b\n> -|-\n", // code-like literal cannot become a header
        "```\n> a|b\n> -|-\n```\n",
    ] {
        assert!(parse(source).tables().is_empty(), "{source:?}");
    }
    for boundary in [
        "outside\n",
        "> > nested\n",
        "> - nested\n",
        ">\n",
        "> # heading\n",
        "> ---\n",
        "> ```\n",
        ">     code\n",
        "> [ref]: /url\n",
    ] {
        let head = "> a|b\n> -|-\n> x|y\n";
        let doc = parse(&format!("{head}{boundary}> tail\n"));
        assert_eq!(doc.tables()[0].source.end, head.len(), "{boundary:?}");
        assert_eq!(doc.tables()[0].rows().len(), 2);
    }
    let doc = parse("- a|b\n  -|-\n  x|y\n- sibling\n  tail\n");
    assert_eq!(
        doc.tables()[0].source.end,
        doc.source().find("- sibling").unwrap()
    );
}

#[test]
fn failed_headers_keep_inline_setext_and_following_candidate_semantics() {
    for tail in ["> -|-\n", "> ordinary\n"] {
        let doc = parse(&format!("intro *open\n> a|b\n{tail}"));
        assert_eq!(doc.text(doc.blocks()[0].source), Some("intro *open\n"));
        assert_eq!(doc.blocks()[0].container, None);
        assert_eq!(doc.blocks()[1].container, Some(ContainerId(0)));
    }
    for tail in ["> ordinary\n", "> -|-|-\n", "outside\n", ""] {
        let doc = parse(&format!("> **a**|b\r\n{tail}"));
        assert!(doc.tables().is_empty());
        let a = doc.source().find('a').unwrap();
        assert!(
            doc.runs_in(Span::new(a, a + 1))
                .unwrap()
                .next()
                .unwrap()
                .style
                .contains(Style::STRONG)
        );
        assert!(!visible(&doc, Span::new(0, doc.source().len())).contains('>'));
    }
    let doc = parse("> **title**|b\n> ---\n> a|b\n> -|-\n");
    assert_eq!(doc.headings().entries().len(), 1);
    assert_eq!(
        visible(&doc, doc.headings().entries()[0].content),
        "title|b"
    );
    assert_eq!(doc.tables().len(), 1);
    let doc = parse("> first|candidate\n> second|header\n> -|-\n");
    assert_eq!(
        doc.tables()[0].source.start,
        doc.source().find("> second").unwrap()
    );
    assert_eq!(visible(&doc, doc.blocks()[0].source), "first|candidate\n");
    // A failed header joins the same-container inline sequence exactly once.
    let doc = parse("> *open|b\n> close*\n");
    assert!(
        doc.runs_in(Span::new(0, doc.source().len()))
            .unwrap()
            .any(|r| r.style.contains(Style::EMPHASIS))
    );
}

#[test]
fn streaming_creation_rows_width_and_eof_report_stable_identities() {
    let mut doc = Document::new();
    doc.append("> a|b\n").unwrap();
    let block = BlockId(0);
    assert!(doc.tables().is_empty());
    assert_eq!(doc.block(block).unwrap().state, SyntaxState::Growing);
    doc.append("> -|").unwrap();
    assert!(doc.tables().is_empty());
    let update = doc.append("-\n").unwrap();
    assert_eq!(update.new_tables, 0..1);
    assert!(update.new_blocks.is_empty());
    assert!(update.changed_blocks.contains(&block.0));
    assert_eq!(doc.tables()[0].block, block);
    assert_eq!(doc.tables()[0].rows().len(), 1);
    let start = doc.source().len();
    doc.append("> ").unwrap();
    assert_eq!(doc.tables()[0].rows().len(), 1);
    let text = "x".repeat(BLOCK_LOOKAHEAD + 3);
    let update = doc.append(&text).unwrap();
    assert!(!update.rows_changed.is_empty());
    let row = doc.tables()[0].row(1).unwrap();
    assert_eq!(row.source, Span::new(start, doc.source().len()));
    assert!(!row.complete);
    assert_eq!(doc.text(row.cells[0].source), Some(text.as_str()));
    let update = doc.append("|y\r\n").unwrap();
    assert_eq!(update.tables_changed, [TableId(0)]);
    assert!(doc.tables()[0].row(1).unwrap().complete);
    assert_eq!(doc.tables()[0].columns[0].source_width_hint, text.len());
    doc.append("> final|cell").unwrap();
    let update = doc.finish();
    assert!(!update.rows_changed.is_empty());
    assert_eq!(doc.tables()[0].rows().len(), 3);
    assert_eq!(
        doc.text(doc.tables()[0].cell(2, 0).unwrap().source),
        Some("final")
    );
    assert_eq!(doc.block(block).unwrap().state, SyntaxState::Sealed);
    let before = doc.work();
    doc.finish();
    assert_eq!(before, doc.work());
    let eof = parse("> a|b\n> -|-");
    assert_eq!(eof.tables().len(), 1);
}

fn compare(a: &Document, b: &Document) {
    assert_eq!(a.blocks(), b.blocks());
    assert_eq!(a.tables(), b.tables());
    assert_eq!(a.headings().entries(), b.headings().entries());
    assert_eq!(a.container_count(), b.container_count());
    for i in 0..a.container_count() {
        assert_eq!(a.container(ContainerId(i)), b.container(ContainerId(i)));
    }
    assert_eq!(
        visible(a, Span::new(0, a.source().len())),
        visible(b, Span::new(0, b.source().len()))
    );
    for (at, ch) in a.source().char_indices() {
        let span = Span::new(at, at + ch.len_utf8());
        assert_eq!(
            a.runs_in(span).unwrap().next().unwrap().style,
            b.runs_in(span).unwrap().next().unwrap().style,
            "offset {at}"
        );
    }
}

#[test]
fn every_utf8_cut_and_live_prefix_preserves_tables_and_display() {
    let source = format!(
        "# root\n> - | 中🙂 | b |\r\n>   - | -\r\n>   **text** | &amp;\n>   {}|`x\\|y`\r\n>   > a|b\n>   > -|-\n>   > row|end\n> title|bad\n> ===\n- peer|head\n  -|-\n  eof|cell",
        "x".repeat(BLOCK_LOOKAHEAD + 9)
    );
    let whole = parse(&source);
    for (cut, _) in source.char_indices().chain([(source.len(), '\0')]) {
        let mut split = Document::new();
        split.append(&source[..cut]).unwrap();
        split.append(&source[cut..]).unwrap();
        split.finish();
        compare(&whole, &split);
    }
    let mut live = Document::new();
    for ch in source.chars() {
        live.append(ch.encode_utf8(&mut [0; 4])).unwrap();
        let mut once = Document::new();
        once.append(live.source()).unwrap();
        compare(&live, &once);
    }
    live.finish();
    compare(&whole, &live);
}

#[test]
fn header_pipe_can_follow_the_window_with_an_explicit_fresh_container() {
    let prefix = ">".repeat(BLOCK_LOOKAHEAD - 4);
    let accepted = parse(&format!("{prefix}a|b\n{prefix}-|-\n"));
    assert_eq!(accepted.tables().len(), 1);
    let prefix = ">".repeat(BLOCK_LOOKAHEAD - 3);
    let accepted = parse(&format!("{prefix}a|b\n{prefix}-|-\n"));
    assert_eq!(accepted.tables().len(), 1);
    let prefix = ">".repeat(BLOCK_LOOKAHEAD - 1);
    let accepted = parse(&format!("{prefix}a|b\n{prefix}-|-\n"));
    assert_eq!(accepted.tables().len(), 1);
    for (source, tables) in [
        (format!("> {}|b\n> -|-\n", "x".repeat(BLOCK_LOOKAHEAD)), 1),
        (format!("> a|b\n> {}|-\n", "-".repeat(BLOCK_LOOKAHEAD)), 1),
    ] {
        let doc = parse(&source);
        assert_eq!(doc.tables().len(), tables);
        assert!(doc.work().classified_bytes <= 2 * source.len() as u64);
        assert!(doc.work().inline_bytes <= source.len() as u64);
    }
}

#[test]
fn million_byte_row_never_revisits_deep_ancestors_or_old_rows() {
    let prefix = ">".repeat(1000);
    let mut doc = Document::new();
    doc.append(&format!(
        "{prefix}a|b\n{prefix}-|-\n{prefix}{}",
        "x".repeat(50)
    ))
    .unwrap();
    let before = doc.work();
    for _ in 0..1000 {
        doc.append(&"x".repeat(1000)).unwrap();
    }
    let after = doc.work();
    assert_eq!(after.classified_bytes, before.classified_bytes);
    assert_eq!(after.container_bytes, before.container_bytes);
    assert_eq!(after.container_steps, before.container_steps);
    assert_eq!(after.heading_steps, before.heading_steps);
    assert_eq!(after.table_bytes - before.table_bytes, 1_000_000);
    assert_eq!(after.inline_bytes - before.inline_bytes, 1_000_000);
    let update = doc.append("|z\n").unwrap();
    assert_eq!(update.tables_changed, [TableId(0)]);
    assert!(doc.work().tree_visits - after.tree_visits < 2000);
    assert_eq!(doc.tables()[0].columns[0].source_width_hint, 1_000_050);
    assert_eq!(doc.container_count(), 1000);
    assert_eq!(doc.tables()[0].rows().len(), 2);
    let end = doc.source().len();
    doc.append("outside\n").unwrap();
    assert_eq!(doc.tables()[0].source.end, end);
    assert!(doc.work().container_steps < 10 * 1000);
    assert!(doc.work().table_bytes <= doc.source().len() as u64);
    assert!(doc.work().inline_bytes <= doc.source().len() as u64);
    doc.finish();
}

#[test]
fn wide_sparse_container_tables_keep_storage_and_append_work_linear() {
    let mut doc = Document::new();
    doc.append(&format!(
        "> - |{}\n>   |{}\n{}",
        "h|".repeat(400),
        "-|".repeat(400),
        ">   x\n".repeat(20_000)
    ))
    .unwrap();
    assert_eq!(doc.tables()[0].columns.len(), 400);
    assert_eq!(
        doc.tables()[0]
            .rows()
            .iter()
            .map(|r| r.cells.len())
            .sum::<usize>(),
        20_400
    );
    let before = doc.work();
    let old_key = doc.tile_key(TileId(0)).unwrap();
    let update = doc.append(">   wider\n").unwrap();
    assert_eq!(update.tables_changed, [TableId(0)]);
    assert_ne!(old_key, doc.tile_key(TileId(0)).unwrap());
    assert_eq!(doc.work().table_bytes - before.table_bytes, 6);
    assert!(doc.work().tree_visits - before.tree_visits < 1000);
    assert!(doc.work().container_steps - before.container_steps <= 3);
    assert_eq!(doc.container_count(), 3);
    assert_eq!(doc.blocks().len(), 1);
    assert!(doc.work().container_bytes < 4 * doc.source().len() as u64);
    doc.finish();
}

#[test]
fn seeded_table_ranges_rows_and_scope_depth_match_independent_model() {
    let mut source = String::new();
    let mut expected = Vec::new();
    let mut seed = 0x847c_3fa2_u64;
    for i in 0..400 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let depth = (seed >> 32) as usize % 20 + 1;
        let prefix = ">".repeat(depth);
        source.push_str("intro\n");
        let start = source.len();
        source.push_str(&format!("{prefix} - h{i}|b\n"));
        let header = Span::new(start, source.len());
        let delimiter_start = source.len();
        source.push_str(&format!("{prefix}   - | -\n"));
        let delimiter = Span::new(delimiter_start, source.len());
        let mut rows = vec![header];
        for j in 0..(seed >> 56) as usize % 7 {
            let at = source.len();
            source.push_str(&format!("{prefix}   v{i}_{j}|z\n"));
            rows.push(Span::new(at, source.len()));
        }
        expected.push((Span::new(start, source.len()), delimiter, rows, depth + 2));
        source.push_str(&format!("{prefix} - sibling\n\n"));
    }
    let doc = parse(&source);
    assert_eq!(doc.tables().len(), expected.len());
    for (i, (span, delimiter, rows, depth)) in expected.iter().enumerate() {
        let table = &doc.tables()[i];
        assert_eq!(table.source, *span);
        assert_eq!(table.delimiter, *delimiter);
        assert_eq!(
            table.rows().iter().map(|r| r.source).collect::<Vec<_>>(),
            *rows
        );
        let scope = doc.block(table.block).unwrap().container.unwrap();
        assert_eq!(doc.container(scope).unwrap().depth, *depth);
        assert!(doc.container(scope).unwrap().source.end >= span.end);
        for (j, row) in table.rows().iter().enumerate() {
            let expected_cell = if j == 0 {
                format!("h{i}")
            } else {
                format!("v{i}_{}", j - 1)
            };
            assert_eq!(doc.text(row.cells[0].source), Some(expected_cell.as_str()));
        }
    }
    for at in 0..source.len() {
        let expected_table = expected
            .iter()
            .position(|(span, ..)| span.range().contains(&at))
            .map(TableId);
        assert_eq!(doc.table_at(at), expected_table, "at {at}");
        if let Some(id) = expected_table {
            assert_eq!(
                doc.tables()[id.0].row_at(at),
                expected[id.0]
                    .2
                    .iter()
                    .position(|row| row.range().contains(&at))
            );
        }
    }
    assert!(doc.work().table_bytes <= source.len() as u64);
    assert!(doc.work().container_bytes < 4 * source.len() as u64);
}

#[test]
fn preview_charges_hidden_prefix_bytes_and_resumes_under_shared_budget() {
    let prefix = ">".repeat(900);
    let mut doc = parse(&format!(
        "{prefix}h|b\n{prefix}-|-\n{prefix}中🙂|&amp;\n{prefix}{}|z\n",
        "x".repeat(1_000_000)
    ));
    let id = FoldId::Table(TableId(0));
    let mut view = FoldView::new(&doc, 1).unwrap();
    view.set_collapsed(&doc, id, true).unwrap();
    let version = view.version();
    let height = view.heights().total_rows();
    let before = doc.work();
    // A source cap spent entirely inside the hidden prefix cannot expose text.
    let mut limited = TablePreview::new(
        &doc,
        TableId(0),
        TablePreviewLimits {
            rows: 2,
            source_bytes: 900,
            display_bytes: 100,
        },
    )
    .unwrap();
    let mut ended = false;
    for _ in 0..2000 {
        match limited.next(&doc, &mut tiny()).unwrap() {
            TablePreviewStep::BudgetExhausted => (),
            TablePreviewStep::Complete(end) => {
                assert_eq!(end, TablePreviewEnd::SourceLimit);
                ended = true;
                break;
            }
            step => panic!("prefix escaped cap: {step:?}"),
        }
    }
    assert!(ended);
    let mut task = TablePreview::new(
        &doc,
        TableId(0),
        TablePreviewLimits {
            rows: 2,
            source_bytes: 3000,
            display_bytes: 100,
        },
    )
    .unwrap();
    let mut text = String::new();
    let mut reads = 0;
    let mut ended = false;
    for _ in 0..4000 {
        let mut budget = Budget::new(WorkLimit {
            bytes: 7,
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        // Simulates another consumer in the same frame.
        assert!(budget.charge(WorkLimit {
            bytes: 1,
            ..Default::default()
        }));
        match task.next(&doc, &mut budget).unwrap() {
            TablePreviewStep::Run { row, run } => {
                assert!(row < 2);
                text.push_str(run.text);
            }
            TablePreviewStep::BudgetExhausted => (),
            TablePreviewStep::Complete(end) => {
                assert_eq!(end, TablePreviewEnd::RowLimit);
                ended = true;
                break;
            }
        }
        assert!(budget.used().bytes <= 7);
        assert!(budget.used().queries <= 1);
        assert!(budget.used().objects <= 1);
        reads += budget.used().bytes;
    }
    assert!(ended);
    assert_eq!(text, "hb\n中🙂&\n");
    assert!(reads < 6000);
    assert_eq!(doc.work(), before);
    assert_eq!(view.version(), version);
    assert_eq!(view.heights().total_rows(), height);
    assert!(view.is_collapsed(id));
    doc.finish(); // Repeated EOF leaves the cursor valid.
    assert_eq!(
        task.next(&doc, &mut tiny()).unwrap(),
        TablePreviewStep::Complete(TablePreviewEnd::RowLimit)
    );
}
