use mdtui3::*;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn styles(doc: &Document, span: Span) -> Vec<(Style, Option<LinkId>)> {
    doc.runs_in(span)
        .unwrap()
        .flat_map(|r| std::iter::repeat_n((r.style, r.link), r.source.len()))
        .collect()
}

fn visible(doc: &Document, span: Span) -> String {
    doc.display_runs_in(span)
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect()
}

fn compare_header(prior: &str, header: &str) {
    let tail = ":--|--:\r\nv|z\n";
    let late = parse(&format!("{prior}{header}{tail}"));
    let early = parse(&format!("{prior}|{header}{tail}"));
    assert_eq!(late.tables().len(), 1, "{prior:?} {header:?}");
    let old = Span::new(0, prior.len());
    assert_eq!(styles(&late, old), styles(&early, old));
    assert_eq!(visible(&late, old), visible(&early, old));
    let span = Span::new(prior.len(), prior.len() + header.len());
    let shifted = Span::new(span.start + 1, span.end + 1);
    assert_eq!(styles(&late, span), styles(&early, shifted));
    assert_eq!(visible(&late, span), visible(&early, shifted));
    assert_eq!(late.links().len(), early.links().len());
    for id in 0..late.links().len() {
        assert_eq!(
            late.link_destination(LinkId(id)),
            early.link_destination(LinkId(id))
        );
    }
    assert_eq!(late.maths().len(), early.maths().len());
    for (left, right) in late.maths().iter().zip(early.maths()) {
        assert_eq!(left.kind, right.kind);
        assert_eq!(late.text(left.content), early.text(right.content));
        let shift = usize::from(left.source.start >= prior.len());
        assert_eq!(
            right.source,
            Span::new(left.source.start + shift, left.source.end + shift)
        );
    }
    assert_eq!(
        late.footnote_references().len(),
        early.footnote_references().len()
    );
    assert!(late.work().inline_bytes <= late.source().len() as u64 + late.work().table_bytes);
}

fn compare_plain(source: &str) {
    let doc = parse(source);
    let ordinary = parse(&source.replace('|', "/"));
    let span = Span::new(0, source.len());
    assert!(doc.tables().is_empty());
    assert_eq!(styles(&doc, span), styles(&ordinary, span));
    assert_eq!(
        visible(&doc, span).replace('|', "/"),
        visible(&ordinary, span)
    );
    assert_eq!(doc.blocks(), ordinary.blocks());
    assert_eq!(doc.maths(), ordinary.maths());
    assert_eq!(doc.links(), ordinary.links());
    assert!(doc.work().inline_bytes <= source.len() as u64);
}

#[test]
fn opaque_math_confirmation_matches_independent_cell_semantics() {
    for marker in ["$", "$$"] {
        let local = if marker == "$" { "$$" } else { "$" };
        for ending in ["", "  ", "\\"] {
            let prior = format!("[p](u) $p$ [^p] &amp; *old {marker} prior{ending}\r\n");
            let header = format!(
                "**local** ` a\\|b ` [h](v) {local}中🙂{local} [^n] &amp; {}|$tail$\r\n",
                "x".repeat(1030)
            );
            compare_header(&prior, &header);
        }
    }
}

#[test]
fn pending_dollar_runs_at_pipe_are_discarded_only_on_confirmation() {
    for marker in ["$", "$$"] {
        for run in ["$", "$$", "$$$", &"$".repeat(2048)] {
            let prior = format!("old {marker} 中\r\n");
            let header = format!("{} {run}|b\r\n", "x".repeat(1030));
            compare_header(&prior, &header);
            compare_plain(&format!("{prior}{header}wrong\n"));
        }
    }
}

#[test]
fn fallback_eof_setext_long_delimiter_and_blocks_preserve_raw_math() {
    for marker in ["$", "$$"] {
        for end in [
            "",
            "\r\n",
            "\r\nwrong\n",
            "\r\n---\n",
            "\r\n# out\n",
            "\r\n> out\n",
        ] {
            for close in ["", marker] {
                compare_plain(&format!(
                    "a {marker} prior\\\r\n{} &amp; [raw](u)|end{close}{end}",
                    "中🙂".repeat(150)
                ));
            }
        }
        compare_plain(&format!(
            "a {marker} prior\n{}|end\n{}x|close{marker}\n",
            "x".repeat(1030),
            "-".repeat(4096)
        ));
        compare_plain(&format!(
            "a {marker} prior\n{}\nend{marker}\n",
            "x".repeat(1030)
        ));
    }
}

#[test]
fn closure_before_pipe_cancels_probe_even_when_new_math_opens() {
    for marker in ["$", "$$"] {
        for padding in [0, 1010, 2048] {
            let mut doc = Document::new();
            doc.append(&format!("$prior$ a {marker} 中\r\n")).unwrap();
            let first = *doc.math(MathId(0)).unwrap();
            let update = doc
                .append(&format!(
                    "{}end{marker} [live](u) {marker}new {}",
                    "x".repeat(padding),
                    "y".repeat(1100)
                ))
                .unwrap();
            assert_eq!(update.new_maths, 1..2);
            let closed = *doc.math(MathId(1)).unwrap();
            assert_eq!(doc.links().len(), 1);
            let before = doc.work();
            let update = doc.append(&format!("|b{marker}\r\n")).unwrap();
            assert_eq!(update.new_maths, 2..3);
            assert_eq!(doc.work().table_header_bytes, before.table_header_bytes);
            doc.append("-|-\n").unwrap();
            doc.finish();
            assert_eq!(*doc.math(MathId(0)).unwrap(), first);
            assert_eq!(*doc.math(MathId(1)).unwrap(), closed);
            compare_plain(doc.source());
        }
    }
}

#[test]
fn deferred_objects_publish_once_and_existing_ids_and_ranges_stay_stable() {
    for success in [false, true] {
        let mut doc = Document::new();
        doc.append("[p](u) $p$ [^p] *old $$ prior\n").unwrap();
        let link = *doc.link_info(LinkId(0)).unwrap();
        let math = *doc.math(MathId(0)).unwrap();
        let footnote = doc.footnote_references()[0];
        let paragraph = doc.blocks()[0].source;
        doc.append(&format!("[h](v) $中🙂$ [^n] &amp; {}", "x".repeat(2048)))
            .unwrap();
        assert_eq!(
            (
                doc.links().len(),
                doc.maths().len(),
                doc.footnote_references().len()
            ),
            (1, 1, 1)
        );
        doc.append("|[t](w) $tail$ [^t]\n").unwrap();
        let update = doc
            .append(if success { "-|-\n" } else { "wrong\n" })
            .unwrap();
        assert_eq!(update.new_links, 1..if success { 3 } else { 1 });
        assert_eq!(update.new_maths, 1..if success { 3 } else { 1 });
        assert_eq!(
            update.new_footnote_references,
            1..if success { 3 } else { 1 }
        );
        assert_eq!(*doc.link_info(LinkId(0)).unwrap(), link);
        assert_eq!(*doc.math(MathId(0)).unwrap(), math);
        assert_eq!(doc.footnote_references()[0], footnote);
        if success {
            assert_eq!(doc.block(BlockId(0)).unwrap().source, paragraph);
            assert_eq!(doc.tables()[0].block, BlockId(1));
            assert_eq!(doc.text(doc.math(MathId(1)).unwrap().content), Some("中🙂"));
            assert_eq!(doc.footnotes()[2].number, Some(3));
        } else {
            let update = doc.append("end$$ [after](z)\n").unwrap();
            assert_eq!(update.new_maths, 1..2);
            assert_eq!(
                doc.math(MathId(1)).unwrap().source.start,
                doc.source().find("$$").unwrap()
            );
            assert_eq!(doc.links().len(), 2);
        }
        let update = doc.finish();
        assert!(update.new_maths.is_empty());
        assert!(
            doc.maths()
                .windows(2)
                .all(|m| m[0].source.end <= m[1].source.start)
        );
    }
}

#[test]
fn million_byte_growth_is_single_pass_and_confirmation_only_consumes_current_row() {
    for success in [false, true] {
        let mut doc = Document::new();
        doc.append(&format!(
            "a ${}\n{}",
            "o".repeat(1_000_000),
            "x".repeat(1040)
        ))
        .unwrap();
        let before = doc.work();
        let annotations = doc.annotation_arena_stats();
        let containers = doc.container_arena_stats();
        for _ in 0..1000 {
            doc.append(&"x".repeat(1000)).unwrap();
        }
        let after = doc.work();
        assert_eq!(after.inline_bytes - before.inline_bytes, 1_000_000);
        assert_eq!(
            after.table_header_bytes - before.table_header_bytes,
            1_000_000
        );
        assert_eq!(after.classified_bytes, before.classified_bytes);
        assert_eq!(after.delimiter_steps, before.delimiter_steps);
        assert_eq!(after.math_steps, before.math_steps);
        assert_eq!(after.container_steps, before.container_steps);
        assert_eq!(after.heading_steps, before.heading_steps);
        assert_eq!(doc.annotation_arena_stats(), annotations);
        assert_eq!(doc.container_arena_stats(), containers);
        doc.append("|b$\n").unwrap();
        let update = doc
            .append(if success { "-|-\n" } else { "wrong\n" })
            .unwrap();
        assert_eq!(update.new_tables.len(), usize::from(success));
        assert_eq!(update.new_maths.len(), usize::from(!success));
        assert_eq!(
            doc.work().inline_bytes - after.inline_bytes,
            if success { 1_001_042 } else { 10 }
        );
        assert_eq!(
            doc.work().table_bytes - after.table_bytes,
            if success { 1_001_044 } else { 0 }
        );
        doc.finish();
        assert!(doc.work().inline_bytes <= doc.source().len() as u64 + doc.work().table_bytes);
    }
}

#[test]
fn repeated_failed_probes_never_walk_old_openers_or_revisit_math_body() {
    let mut doc = Document::new();
    doc.append(&format!("{} $$ prior\\\r\n", "*open ".repeat(4096)))
        .unwrap();
    let before = doc.work();
    for i in 0..200 {
        doc.append(&format!(
            "{}{}\r\nwrong\r\n",
            "x".repeat(1030),
            if i % 2 == 0 { "|b" } else { "" }
        ))
        .unwrap();
    }
    assert_eq!(doc.work().delimiter_steps, before.delimiter_steps);
    assert_eq!(doc.work().math_steps, before.math_steps);
    assert_eq!(doc.maths().len(), 0);
    doc.append("end$$ [live](u)\n").unwrap();
    assert_eq!(doc.maths().len(), 1);
    assert_eq!(doc.links().len(), 1);
    assert!(doc.tables().is_empty());
    doc.finish();
    compare_plain(doc.source());
    assert!(doc.work().delimiter_steps - before.delimiter_steps <= 4098);
}

// Independent model: only a matching run followed by new syntax cancels the
// probe. An inert tail can now isolate the closure; pipe does not flush a run.
fn cancels_before_pipe(prefix: &str, width: usize) -> bool {
    let bytes = prefix.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'\\' {
            at += 2;
        } else if bytes[at] == b'$' {
            let start = at;
            while at < bytes.len() && bytes[at] == b'$' {
                at += 1;
            }
            if at < bytes.len() && at - start == width {
                return bytes[at..].iter().any(|b| b"&<\\*_~`$![]".contains(b));
            }
        } else {
            at += 1;
        }
    }
    false
}

#[test]
fn generated_dollar_runs_and_escapes_match_independent_closure_model() {
    let tokens = [
        "a ", "$", "$$", "$$$", "$ ", "$$ ", "$$$ ", "\\$", "\\\\$", "&amp;", "[x](u)", "`x`",
        "*x*",
    ];
    let mut confirmed = 0;
    let mut rejected = 0;
    for width in [1, 2] {
        for a in tokens {
            for b in tokens {
                let prior = format!("old {} prior\r\n", "$".repeat(width));
                let prefix = format!("{}{a}{b}", "x".repeat(1030));
                let header = format!("{prefix}|end\n");
                let source = format!("{prior}{header}-|-\n");
                let doc = parse(&source);
                let cancelled = cancels_before_pipe(&prefix, width);
                assert_eq!(
                    doc.tables().len(),
                    usize::from(!cancelled),
                    "{width} {a:?} {b:?}"
                );
                if cancelled {
                    rejected += 1;
                    compare_plain(&source);
                } else {
                    confirmed += 1;
                    compare_header(&prior, &header);
                }
            }
        }
    }
    assert!(confirmed > 80 && rejected > 20);
}

#[test]
fn containers_cancel_table_probe_when_inherited_math_closes_before_pipe() {
    for (open, prefix) in [
        ("> ".to_owned(), "> ".to_owned()),
        ("- ".repeat(250), " ".repeat(500)),
        (">".repeat(1000), ">".repeat(1000)),
    ] {
        let mut doc = Document::new();
        doc.append(&format!(
            "{open}$old\n{prefix}$local$ [h](u) {}",
            "x".repeat(2048)
        ))
        .unwrap();
        // The second dollar after the old closure cancels its isolated probe.
        assert_eq!(doc.maths().len(), 1);
        let math = *doc.math(MathId(0)).unwrap();
        doc.append(&format!("|$tail$\n{prefix}-|-\n{prefix}$body$|z\n"))
            .unwrap();
        doc.finish();
        assert!(doc.tables().is_empty());
        assert_eq!(*doc.math(MathId(0)).unwrap(), math);
        assert_eq!(doc.maths().len(), 3);
        assert_eq!(
            doc.text(math.content),
            Some(format!("old\n{prefix}").as_str())
        );
    }
    let source = format!("old $$\n{}|b\n> -|-\n", "x".repeat(1030));
    assert!(parse(&source).tables().is_empty());
    compare_header("[old $$ prior\n", &format!("{}|b\n", "x".repeat(1030)));
    let doc = parse("h|h\n-|-\n$open|$next$\n");
    assert_eq!(doc.maths().len(), 1);
    assert_eq!(doc.text(doc.maths()[0].content), Some("next"));
}

#[test]
fn dense_objects_and_columns_publish_once_with_budgeted_math_and_preview_reads() {
    let mut doc = Document::new();
    doc.append(&format!(
        "{} $$ prior\n{}",
        "*old ".repeat(4096),
        "[a](u) $中🙂$ [^n] ".repeat(4096)
    ))
    .unwrap();
    assert!(doc.maths().is_empty());
    assert!(doc.links().is_empty());
    doc.append(&format!("|{}\n", "h|".repeat(4095))).unwrap();
    let before = doc.work();
    let update = doc.append(&format!("|{}\n", "-:|".repeat(4096))).unwrap();
    assert_eq!(update.new_links, 0..4096);
    assert_eq!(update.new_maths, 0..4096);
    assert_eq!(update.new_footnote_references, 0..4096);
    assert_eq!(doc.work().math_steps - before.math_steps, 8192);
    assert_eq!(
        doc.work().delimiter_steps - before.delimiter_steps,
        4096 + 8192
    );
    assert_eq!(doc.tables()[0].columns.len(), 4096);
    for i in [0, 1, 2048, 4095] {
        let math = doc.math(MathId(i)).unwrap();
        let mut objects = doc.maths_in(math.source).unwrap();
        let mut zero = Budget::new(WorkLimit::default());
        assert_eq!(objects.advance(&mut zero), Err(Completion::BudgetExhausted));
        let mut budget = Budget::new(WorkLimit {
            queries: 3,
            objects: 3,
            bytes: 7,
            ..Default::default()
        });
        assert_eq!(
            objects.advance(&mut budget).unwrap(),
            Some((MathId(i), *math))
        );
        let mut content = doc.math_content(MathId(i)).unwrap();
        assert_eq!(content.advance(&mut zero), Err(Completion::BudgetExhausted));
        assert_eq!(content.source_position(), math.content.start);
        assert_eq!(content.advance(&mut budget).unwrap().unwrap().text, "中🙂");
        assert_eq!(content.advance(&mut zero).unwrap(), None);
        assert_eq!(objects.advance(&mut budget).unwrap(), None);
        assert_eq!(budget.used().bytes, 7);
    }
    let mut preview = TablePreview::new(
        &doc,
        TableId(0),
        TablePreviewLimits {
            rows: 1,
            source_bytes: 64,
            display_bytes: 32,
        },
    )
    .unwrap();
    let mut zero = Budget::new(WorkLimit::default());
    assert_eq!(
        preview.next(&doc, &mut zero).unwrap(),
        TablePreviewStep::BudgetExhausted
    );
    assert_eq!(zero.used(), WorkLimit::default());
    for frame in 0..200 {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 8,
            ..Default::default()
        });
        let step = preview.next(&doc, &mut budget).unwrap();
        assert!(budget.used().bytes <= 8);
        if matches!(step, TablePreviewStep::Complete(_)) {
            return;
        }
        assert!(frame < 199);
    }
}

#[test]
fn generated_tables_and_rejections_match_independent_range_model() {
    let mut source = String::new();
    let mut expected = Vec::new();
    for i in 0..400 {
        source.push_str("a $$ prior\n");
        let start = source.len();
        source.push_str(&format!(
            "{}{}|b\r\n",
            "x".repeat(1030 + i),
            if i % 3 == 0 { "$$ closed &amp; " } else { "" }
        ));
        let delimiter_start = source.len();
        source.push_str(&format!(
            "{}|-{}\n",
            "-".repeat(1030),
            if i % 5 == 0 { "x" } else { "" }
        ));
        let delimiter_end = source.len();
        source.push_str("$v$|z\n");
        if i % 3 != 0 && i % 5 != 0 {
            expected.push((
                Span::new(start, source.len()),
                Span::new(delimiter_start, delimiter_end),
            ));
        }
        source.push('\n');
    }
    let doc = parse(&source);
    assert_eq!(doc.tables().len(), expected.len());
    for (table, (span, delimiter)) in doc.tables().iter().zip(expected) {
        assert_eq!(table.source, span);
        assert_eq!(table.delimiter, delimiter);
        assert_eq!(table.cell(0, 0).unwrap().source.start, span.start);
        assert_eq!(doc.block(table.block).unwrap().source, span);
    }
    assert!(doc.work().inline_bytes <= source.len() as u64 + doc.work().table_bytes);
    assert!(doc.work().table_header_bytes <= source.len() as u64);
}

#[test]
fn math_resolution_invalidates_measurements_and_merged_fold_views_agree() {
    for success in [false, true] {
        let mut doc = Document::new();
        let mut view = FoldView::new(&doc, 1).unwrap();
        let mut merged = FoldView::new(&doc, 1).unwrap();
        let mut batch = Update::default();
        for chunk in ["a $ prior\r\n", &"x".repeat(2048), "|end$\r\n"] {
            let update = doc.append(chunk).unwrap();
            view.apply(&doc, &update).unwrap();
            batch.merge(update).unwrap();
        }
        let mut heights = HeightIndex::default();
        heights.rebuild(&doc).unwrap();
        let tile = TileId(1);
        let ticket = heights.ticket(tile).unwrap();
        let key = doc.tile_key(tile).unwrap();
        let update = doc
            .append(if success { "-|-\n" } else { "wrong\n" })
            .unwrap();
        heights.apply(&doc, &update).unwrap();
        assert_ne!(doc.tile_key(tile).unwrap(), key);
        assert_eq!(
            heights.measure(ticket, 2),
            Err(LayoutError::StaleMeasurement)
        );
        view.apply(&doc, &update).unwrap();
        batch.merge(update).unwrap();
        assert_eq!(batch.new_maths, 0..usize::from(!success));
        merged.apply(&doc, &batch).unwrap();
        assert_eq!(view.heights().total_rows(), merged.heights().total_rows());
        if success {
            for view in [&mut view, &mut merged] {
                view.set_collapsed(&doc, FoldId::Table(TableId(0)), true)
                    .unwrap();
                let mut budget = Budget::new(WorkLimit {
                    queries: 1,
                    ..Default::default()
                });
                assert!(matches!(
                    view.hidden_at(&doc, 1500, &mut budget).unwrap(),
                    FoldLookup::Hidden(_)
                ));
            }
        }
    }
}
