use mdstream::*;

fn caps(rows: usize, source_bytes: usize, display_bytes: usize) -> TablePreviewLimits {
    TablePreviewLimits {
        rows,
        source_bytes,
        display_bytes,
    }
}

fn frame(bytes: usize) -> Budget {
    Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        bytes,
        ..Default::default()
    })
}

fn collect(
    doc: &Document,
    limits: TablePreviewLimits,
    bytes: usize,
) -> (Vec<String>, TablePreviewEnd, usize) {
    let mut task = TablePreview::new(doc, TableId(0), limits).unwrap();
    let mut rows = Vec::<String>::new();
    let mut reads = 0;
    for _ in 0..10000 {
        let mut budget = frame(bytes);
        loop {
            match task.next(doc, &mut budget).unwrap() {
                TablePreviewStep::Run { row, run } => {
                    assert!(row < limits.rows);
                    assert!(!run.style.contains(Style::HIDDEN));
                    assert!(
                        doc.table(TableId(0))
                            .unwrap()
                            .row(row)
                            .unwrap()
                            .source
                            .range()
                            .contains(&run.source.start)
                    );
                    if run.replacement.is_none() {
                        assert_eq!(doc.text(run.source), Some(run.text));
                    }
                    rows.resize_with(row + 1, String::new);
                    rows[row].push_str(run.text);
                }
                TablePreviewStep::BudgetExhausted => break,
                TablePreviewStep::Complete(end) => {
                    let mut zero = frame(0);
                    assert_eq!(
                        task.next(doc, &mut zero).unwrap(),
                        TablePreviewStep::Complete(end)
                    );
                    assert_eq!(zero.used(), WorkLimit::default());
                    assert!(rows.iter().map(String::len).sum::<usize>() <= limits.display_bytes);
                    return (rows, end, reads + budget.used().bytes);
                }
            }
        }
        reads += budget.used().bytes;
    }
    panic!("no progress");
}

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

#[test]
fn indexed_rows_match_display_model_at_every_utf8_split() {
    let source = "| 名 | value |\n| - | - |\n| **中🙂** | &amp; `x` |\n| sparse |\n| a | b | ignored |\n\nafter\n";
    let reference = parse(source);
    let table = reference.table(TableId(0)).unwrap();
    let model: Vec<String> = table
        .rows()
        .iter()
        .map(|row| {
            reference
                .display_runs_in(row.source)
                .unwrap()
                .filter(|run| !run.style.contains(Style::HIDDEN))
                .map(|run| run.text.to_string())
                .collect()
        })
        .collect();
    for split in (0..=source.len()).filter(|&i| source.is_char_boundary(i)) {
        let mut doc = Document::new();
        doc.append(&source[..split]).unwrap();
        doc.append(&source[split..]).unwrap();
        doc.finish();
        for count in 0..=5 {
            for bytes in [6, 256] {
                let (rows, end, _) = collect(&doc, caps(count, usize::MAX, usize::MAX), bytes);
                assert_eq!(rows, model[..count.min(model.len())]);
                assert_eq!(
                    end,
                    if count < model.len() {
                        TablePreviewEnd::RowLimit
                    } else {
                        TablePreviewEnd::RowsEnd
                    }
                );
            }
        }
    }
}

#[test]
fn aggregate_caps_utf8_and_delimiter_exclusion() {
    let doc = parse("| h |\n| - |\n| 中🙂 &amp; |\n| last |\n");
    let header = doc.table(TableId(0)).unwrap().row(0).unwrap().source.len();
    let (rows, end, _) = collect(&doc, caps(3, header, 100), 6);
    assert_eq!(rows.len(), 1);
    assert_eq!(end, TablePreviewEnd::SourceLimit);
    for source_cap in 0..60 {
        for display_cap in 0..20 {
            let small = collect(&doc, caps(3, source_cap, display_cap), 6);
            let large = collect(&doc, caps(3, source_cap, display_cap), 256);
            assert_eq!(small.0, large.0);
            if small.1 != large.1 {
                assert!(matches!(
                    small.1,
                    TablePreviewEnd::SourceLimit | TablePreviewEnd::DisplayLimit
                ));
                assert!(matches!(
                    large.1,
                    TablePreviewEnd::SourceLimit | TablePreviewEnd::DisplayLimit
                ));
            }
        }
    }
    let header_only = parse("| h |\n| - |\n");
    assert_eq!(
        collect(&header_only, caps(1, header, 100), 6).1,
        TablePreviewEnd::RowsEnd
    );
}

#[test]
fn stale_wrong_document_and_budget_atomicity() {
    let mut doc = parse("| h |\n| - |\n| row |\n");
    assert!(matches!(
        TablePreview::new(&doc, TableId(99), caps(2, 100, 100)),
        Err(LayoutError::InvalidRange)
    ));
    let mut task = TablePreview::new(&doc, TableId(0), caps(2, 100, 100)).unwrap();
    let other = parse(doc.source());
    let mut budget = frame(6);
    assert_eq!(
        task.next(&other, &mut budget),
        Err(LayoutError::WrongDocument)
    );
    assert_eq!(budget.used(), WorkLimit::default());
    for limit in [
        WorkLimit::default(),
        WorkLimit {
            queries: 1,
            ..Default::default()
        },
        WorkLimit {
            objects: 1,
            ..Default::default()
        },
    ] {
        let mut budget = Budget::new(limit);
        assert_eq!(
            task.next(&doc, &mut budget).unwrap(),
            TablePreviewStep::BudgetExhausted
        );
        assert_eq!(budget.used(), WorkLimit::default());
    }
    doc.finish();
    assert_eq!(task.revision(), doc.revision());
    assert!(task.next(&doc, &mut budget).is_ok());
    let mut live = Document::new();
    live.append("| h |\n| - |\n").unwrap();
    let mut task = TablePreview::new(&live, TableId(0), caps(0, 0, 0)).unwrap();
    assert_eq!(
        task.next(&live, &mut frame(0)).unwrap(),
        TablePreviewStep::Complete(TablePreviewEnd::RowLimit)
    );
    live.append("").unwrap();
    assert_eq!(task.revision(), live.revision());
    live.append("| growing").unwrap();
    assert_eq!(
        task.next(&live, &mut frame(0)),
        Err(LayoutError::StaleContinuation)
    );
    let mut task = TablePreview::new(&live, TableId(0), caps(2, 100, 100)).unwrap();
    live.finish();
    assert_eq!(
        task.next(&live, &mut frame(0)),
        Err(LayoutError::StaleContinuation)
    );
}

#[test]
fn streaming_snapshots_exclude_unindexed_tail() {
    let mut doc = Document::new();
    for chunk in [
        "| h |\n", "| - |\n", "| ", "growing", " |\n", "# next", "\n",
    ] {
        doc.append(chunk).unwrap();
        if let Some(table) = doc.table(TableId(0)) {
            let expected: Vec<String> = table
                .rows()
                .iter()
                .map(|row| {
                    doc.display_runs_in(row.source)
                        .unwrap()
                        .filter(|run| !run.style.contains(Style::HIDDEN))
                        .map(|run| run.text.to_string())
                        .collect()
                })
                .collect();
            let (mut rows, end, _) = collect(&doc, caps(100, 1000, 1000), 6);
            rows.resize(expected.len(), String::new());
            assert_eq!(rows, expected);
            assert_eq!(end, TablePreviewEnd::RowsEnd);
        }
    }
}

#[test]
fn giant_cells_and_omitted_rows_stay_bounded_without_view_mutation() {
    for size in [1000, 1_000_000] {
        let doc = parse(&format!(
            "# parent\n| h |\n| - |\n| {} |\n{}",
            "x".repeat(size),
            "| omitted |\n".repeat(10000)
        ));
        let id = FoldId::Table(TableId(0));
        let mut view = FoldView::new(&doc, 1).unwrap();
        view.set_collapsed(&doc, id, true).unwrap();
        view.set_collapsed(&doc, FoldId::Heading(HeadingId(0)), true)
            .unwrap();
        let before = (
            view.version(),
            view.heights().total_rows(),
            view.heights().mutation_visits(),
        );
        let (_, end, reads) = collect(&doc, caps(2, 64, 64), 6);
        assert_eq!(end, TablePreviewEnd::SourceLimit);
        assert!(reads < 1500, "{reads}");
        assert!(view.is_collapsed(id));
        assert_eq!(
            before,
            (
                view.version(),
                view.heights().total_rows(),
                view.heights().mutation_visits()
            )
        );
        assert_eq!(
            collect(&doc, caps(1, 64, 64), 6).1,
            TablePreviewEnd::RowLimit
        );
    }
}
