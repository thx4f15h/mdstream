use mdstream::{Alignment, BlockKind, Document, Span, Style, TableId};

fn parsed(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}
fn cell(doc: &Document, row: usize, col: usize) -> &str {
    doc.text(
        doc.table(TableId(0))
            .unwrap()
            .cell(row, col)
            .unwrap()
            .source,
    )
    .unwrap()
}

#[test]
fn tables_preserve_alignment_rows_and_ragged_cells() {
    let doc = parsed(
        "| Name | Count | Note |\n| :--- | ---: | :---: |\n| **one** | 12 | middle |\nshort\n| a | b | c | ignored |\n\nend\n",
    );
    assert_eq!(doc.tables().len(), 1);
    let table = &doc.tables()[0];
    assert!(matches!(
        doc.blocks()[0].kind,
        BlockKind::Table { id: TableId(0) }
    ));
    assert_eq!(
        table
            .columns
            .iter()
            .map(|c| c.alignment)
            .collect::<Vec<_>>(),
        [Alignment::Left, Alignment::Right, Alignment::Center]
    );
    assert_eq!(table.rows().len(), 4);
    assert_eq!(cell(&doc, 0, 0), "Name");
    assert_eq!(cell(&doc, 1, 0), "**one**");
    assert_eq!(cell(&doc, 2, 0), "short");
    assert!(table.cell(2, 1).is_none());
    assert!(table.cell(3, 3).is_none());
    let content = table.cell(1, 0).unwrap().source;
    assert!(
        doc.runs_in(content)
            .unwrap()
            .any(|run| run.text == "one" && run.style.contains(Style::STRONG))
    );
    assert_eq!(table.row_at(content.start), Some(1));
    assert_eq!(table.row_at(table.delimiter.start), None);
    assert!(table.rows().iter().all(|row| row.complete));
    assert_eq!(doc.table_at(content.start), Some(TableId(0)));
}

#[test]
fn escaped_pipes_work_in_text_and_code_and_cells_isolate_inline_state() {
    let doc = parsed("a | b\n- | -\nleft\\|right | `x\\|y`\n*open | close*\n");
    let table = &doc.tables()[0];
    assert_eq!(cell(&doc, 1, 0), "left\\|right");
    let displayed: String = doc
        .runs_in(table.cell(1, 1).unwrap().source)
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    assert_eq!(displayed, "x|y");
    for col in 0..2 {
        assert!(
            doc.runs_in(table.cell(2, col).unwrap().source)
                .unwrap()
                .all(|r| !r.style.contains(Style::EMPHASIS))
        );
    }
}

#[test]
fn optional_outer_pipes_empty_cells_and_eof() {
    for source in ["| a | b |\n|---|---|\n|| b |", "a | b\n--- | ---\n| | b\n"] {
        let doc = parsed(source);
        assert_eq!(cell(&doc, 1, 0), "");
        assert_eq!(cell(&doc, 1, 1), "b");
        assert!(doc.tables()[0].row(1).unwrap().complete);
    }
    let doc = parsed("| solo |\n| :--: |\n| value |\n");
    assert_eq!(doc.tables()[0].columns[0].alignment, Alignment::Center);
    assert_eq!(cell(&doc, 1, 0), "value");
}

#[test]
fn empty_table_headers_still_produce_a_table_creation_event() {
    let mut doc = Document::new();
    doc.append("| | |\n").unwrap();
    let update = doc.append("|-|-|\n").unwrap();
    assert_eq!(update.new_tables, 0..1);
    assert_eq!(doc.tables()[0].columns.len(), 2);
    assert!(
        doc.tables()[0]
            .columns
            .iter()
            .all(|column| column.source_width_hint == 0)
    );
}

#[test]
fn false_candidates_recover_without_losing_inline_semantics() {
    for source in [
        "**a** | b\n---\n",
        "**a** | b\n---|---|---\n",
        "**a** | b",
        "**a** | b\nordinary\n",
    ] {
        let doc = parsed(source);
        assert!(doc.tables().is_empty());
        assert!(
            doc.runs_in(Span::new(0, source.len()))
                .unwrap()
                .any(|r| r.text == "a" && r.style.contains(Style::STRONG))
        );
    }
    let doc = parsed("```\na | b\n---|---\n```\n");
    assert!(doc.tables().is_empty());
}

#[test]
fn table_headers_interrupt_paragraphs_without_rewriting_them() {
    let doc = parsed("intro *unfinished\na | b\n-- | --\nx | y\n");
    assert_eq!(doc.tables().len(), 1);
    assert_eq!(doc.blocks()[0].source.end, "intro *unfinished\n".len());
    assert!(matches!(doc.blocks()[0].kind, BlockKind::Paragraph));
    assert_eq!(cell(&doc, 0, 0), "a");
    let doc = parsed("intro\n**a** | b\nordinary\n");
    assert!(doc.tables().is_empty());
    assert_eq!(doc.blocks().len(), 1);
    assert!(
        doc.runs_in(Span::new(0, doc.source().len()))
            .unwrap()
            .any(|r| r.text == "a" && r.style.contains(Style::STRONG))
    );
}

#[test]
fn long_cells_stream_and_only_new_content_is_scanned() {
    let mut doc = Document::new();
    doc.append("a | b\n--- | ---\n").unwrap();
    let key = doc.tile_key(mdstream::TileId(0)).unwrap();
    doc.append("| ").unwrap();
    for _ in 0..100_000 {
        doc.append("x").unwrap();
    }
    let table = doc.table(TableId(0)).unwrap();
    assert!(!table.row(1).unwrap().complete);
    assert_eq!(cell(&doc, 1, 0).len(), 100_000);
    let update = doc.append(" | done |\n").unwrap();
    assert_eq!(update.tables_changed, [TableId(0)]);
    assert_ne!(key, doc.tile_key(mdstream::TileId(0)).unwrap());
    assert_eq!(doc.tables()[0].columns[0].source_width_hint, 100_000);
    assert_eq!(cell(&doc, 1, 1), "done");
    assert!(doc.work().table_bytes <= doc.source().len() as u64);
    assert!(doc.work().inline_bytes <= doc.source().len() as u64);
}

#[test]
fn wide_sparse_tables_do_not_allocate_a_rectangle() {
    let source = format!(
        "|{}\n|{}\n{}",
        "h|".repeat(400),
        "-|".repeat(400),
        "x\n".repeat(20_000)
    );
    let doc = parsed(&source);
    let table = &doc.tables()[0];
    assert_eq!(table.columns.len(), 400);
    assert_eq!(
        table.rows().iter().map(|r| r.cells.len()).sum::<usize>(),
        400 + 20_000
    );
    assert!(doc.work().table_bytes <= source.len() as u64);
    assert!(doc.annotation_node_count() <= 4 * source.len());
}

#[test]
fn column_growth_invalidates_old_layout_without_scanning_old_rows() {
    use mdstream::{CheckpointIndex, HeightIndex, LayoutError, TileId};
    let mut doc = Document::new();
    let mut heights = HeightIndex::default();
    let mut checkpoints = CheckpointIndex::new();
    let update = doc
        .append(&format!("a|b\n-|-\n{}", "x|y\n".repeat(20_000)))
        .unwrap();
    heights.apply(&doc, &update).unwrap();
    checkpoints.apply(&doc, &update).unwrap();
    let key = doc.tile_key(TileId(0)).unwrap();
    let update = doc.append("z|w\n").unwrap();
    assert!(update.tables_changed.is_empty());
    heights.apply(&doc, &update).unwrap();
    checkpoints.apply(&doc, &update).unwrap();
    assert_eq!(key, doc.tile_key(TileId(0)).unwrap());
    heights
        .measure(heights.ticket(TileId(0)).unwrap(), 20)
        .unwrap();
    checkpoints.insert(TileId(0), "layout").unwrap();
    let ticket = heights.ticket(TileId(0)).unwrap();
    let before = doc.work();
    let update = doc.append("wider|row\n").unwrap();
    assert_eq!(update.tables_changed, [TableId(0)]);
    assert!(doc.work().tree_visits - before.tree_visits < 500);
    assert_eq!(doc.work().table_bytes - before.table_bytes, 10);
    heights.apply(&doc, &update).unwrap();
    checkpoints.apply(&doc, &update).unwrap();
    assert_eq!(
        heights.measure(ticket, 10),
        Err(LayoutError::StaleMeasurement)
    );
    assert!(checkpoints.is_empty());
    assert_ne!(key, doc.tile_key(TileId(0)).unwrap());
}
