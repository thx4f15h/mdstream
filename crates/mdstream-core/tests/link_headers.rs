use mdstream::*;

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
    for i in 0..late.links().len() {
        let id = LinkId(i);
        let left = late.link_info(id).unwrap();
        let mut right = *early.link_info(id).unwrap();
        let shift = usize::from(left.source.start >= prior.len());
        for span in [&mut right.source, &mut right.label]
            .into_iter()
            .chain(right.title.as_mut())
        {
            span.start -= shift;
            span.end -= shift;
        }
        assert_eq!(*left, right);
        assert_eq!(late.link_destination(id), early.link_destination(id));
        assert_eq!(late.link_title(id), early.link_title(id));
    }
    assert_eq!(late.maths().len(), early.maths().len());
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
    for i in 0..doc.links().len() {
        assert_eq!(doc.link_info(LinkId(i)), ordinary.link_info(LinkId(i)));
    }
    assert!(doc.work().inline_bytes <= source.len() as u64);
}

#[test]
fn unclosed_labels_confirm_with_independent_first_cell_semantics() {
    for marker in ["[", "!["] {
        for ending in ["", "  ", "\\"] {
            let prior = format!("[p](u) $p$ [^p] *old {marker} prior{ending}\r\n");
            let header = format!(
                "**local** ` a\\|b ` $中🙂$ &amp; {}|](v) [tail](w)\r\n",
                "x".repeat(1030)
            );
            compare_header(&prior, &header);
        }
    }
}

#[test]
fn replaced_labels_and_opaque_combinations_match_independent_cells() {
    for marker in ["[", "!["] {
        for opaque in ["", "`", "$$"] {
            let prior = format!("*old {marker} prior {opaque}\n");
            for local in [
                "[local](v)",
                "![alt](img)",
                "<https://a>",
                "[open",
                "![open",
                "\\]",
            ] {
                let header = format!("{} {local}|](w) [tail](z)\n", "x".repeat(1030));
                compare_header(&prior, &header);
            }
        }
    }
}

#[test]
fn label_close_cancels_probe_when_replaced_and_isolates_inert_direct_tails() {
    for marker in ["[", "!["] {
        for padding in [0, 1010, 2048] {
            for suffix in ["", "(u)", "[ref]", "(bad target)"] {
                let source = format!(
                    "old {marker} prior\r\n{}]{suffix} [new {}|b\n-|-\n",
                    "x".repeat(padding),
                    "y".repeat(1100)
                );
                compare_plain(&source);
            }
            let mut doc = Document::new();
            doc.append(&format!(
                "old {marker} prior\n{}](u) {}",
                "x".repeat(padding),
                "y".repeat(1100)
            ))
            .unwrap();
            assert!(doc.links().is_empty());
            doc.append("|b\n-|-\n").unwrap();
            assert_eq!(doc.tables().len(), 1);
            assert!(doc.links().is_empty());
            assert!(doc.references().is_empty());
        }
    }
}

#[test]
fn failed_candidates_eof_setext_and_blocks_preserve_labels_and_breaks() {
    for marker in ["[", "!["] {
        for ending in ["", "  ", "\\"] {
            for end in [
                "",
                "\r\n",
                "\r\nwrong\n",
                "\r\n---\n",
                "\r\n# out\n",
                "\r\n> out\n",
            ] {
                compare_plain(&format!(
                    "a {marker} prior{ending}\r\n**中🙂** &amp; {}|end](v){end}",
                    "x".repeat(1030)
                ));
            }
        }
        compare_plain(&format!(
            "a {marker} prior\n{}|b\n{}x|end](v)\n",
            "x".repeat(1030),
            "-".repeat(4096)
        ));
        compare_plain(&format!(
            "a {marker} prior\n{}\nend](v)\n",
            "x".repeat(1030)
        ));
    }
}

#[test]
fn live_objects_and_source_ranges_survive_confirmation_and_failure() {
    for success in [false, true] {
        let mut doc = Document::new();
        doc.append("[p](u) $p$ [^p] ![old  \r\n").unwrap();
        let prior = *doc.link_info(LinkId(0)).unwrap();
        let paragraph = doc.blocks()[0].source;
        let update = doc
            .append(&format!(
                "[local](v 'title') $中🙂$ [^n] &amp; {}",
                "x".repeat(2048)
            ))
            .unwrap();
        assert_eq!(update.new_links, 1..2);
        assert_eq!(update.new_maths, 1..2);
        assert_eq!(update.new_footnote_references, 1..2);
        let local = *doc.link_info(LinkId(1)).unwrap();
        let math = *doc.math(MathId(1)).unwrap();
        doc.append("|![tail](w)\r\n").unwrap();
        let update = doc
            .append(if success { "-|-\n" } else { "wrong\n" })
            .unwrap();
        assert_eq!(update.new_links, 2..3);
        assert!(update.new_maths.is_empty());
        assert!(update.new_footnote_references.is_empty());
        assert_eq!(*doc.link_info(LinkId(0)).unwrap(), prior);
        assert_eq!(*doc.link_info(LinkId(1)).unwrap(), local);
        assert_eq!(*doc.math(MathId(1)).unwrap(), math);
        assert_eq!(doc.link_info(LinkId(2)).unwrap().kind, LinkKind::Image);
        if success {
            assert_eq!(doc.blocks()[0].source, paragraph);
            assert_eq!(doc.tables()[0].block, BlockId(1));
        }
        assert!(doc.finish().new_links.is_empty());
    }
}

#[test]
fn million_byte_label_growth_has_no_rescan_or_extra_arena_growth() {
    for success in [false, true] {
        let mut doc = Document::new();
        doc.append(&format!(
            "![{}\n{}",
            "o".repeat(1_000_000),
            "x".repeat(1040)
        ))
        .unwrap();
        let before = doc.work();
        let annotations = doc.annotation_arena_stats();
        let containers = doc.container_arena_stats();
        let chunk = "x".repeat(1000);
        for _ in 0..1000 {
            doc.append(&chunk).unwrap();
        }
        let after = doc.work();
        assert_eq!(after.inline_bytes - before.inline_bytes, 1_000_000);
        assert_eq!(
            after.table_header_bytes - before.table_header_bytes,
            1_000_000
        );
        assert_eq!(after.classified_bytes, before.classified_bytes);
        assert_eq!(after.label_bytes, before.label_bytes);
        assert_eq!(after.delimiter_steps, before.delimiter_steps);
        assert_eq!(after.container_steps, before.container_steps);
        assert_eq!(after.heading_steps, before.heading_steps);
        assert_eq!(doc.annotation_arena_stats(), annotations);
        assert_eq!(doc.container_arena_stats(), containers);
        doc.append("|b](v)\n").unwrap();
        let update = doc
            .append(if success { "-|-\n" } else { "wrong\n" })
            .unwrap();
        assert_eq!(update.new_tables.len(), usize::from(success));
        assert_eq!(update.new_links.len(), usize::from(!success));
        // Reuse the complete first-cell prefix; neither branch replays it.
        assert!(doc.work().inline_bytes - after.inline_bytes <= 13);
        assert_eq!(doc.work().label_bytes, before.label_bytes);
        if !success {
            assert_eq!(doc.link_info(LinkId(0)).unwrap().source.start, 0);
            assert_eq!(doc.link_info(LinkId(0)).unwrap().kind, LinkKind::Image);
        }
        doc.finish();
        assert!(doc.work().inline_bytes <= doc.source().len() as u64);
    }
}

#[test]
fn repeated_failed_probes_keep_old_stack_and_label_without_traversal() {
    let mut doc = Document::new();
    doc.append(&format!("{} ![prior\n", "*old ".repeat(4096)))
        .unwrap();
    let before = doc.work();
    for i in 0..200 {
        doc.append(&format!(
            "{}{}\r\nwrong\n",
            "x".repeat(1030),
            if i % 2 == 0 { "|b" } else { "" }
        ))
        .unwrap();
    }
    assert_eq!(doc.work().delimiter_steps, before.delimiter_steps);
    assert_eq!(doc.work().label_bytes, before.label_bytes);
    assert!(doc.links().is_empty());
    doc.append("end](u)\n").unwrap();
    assert_eq!(doc.links().len(), 1);
    assert_eq!(doc.link_info(LinkId(0)).unwrap().kind, LinkKind::Image);
    doc.finish();
    assert_eq!(doc.work().delimiter_steps - before.delimiter_steps, 4096);
    compare_plain(doc.source());
}

// The existing label dialect keeps one opener: a new '[' replaces it, an
// unescaped ']' closes it. This model deliberately has no parser state or IDs.
fn closes_old_label(prefix: &str) -> Option<usize> {
    let mut old = true;
    let mut escaped = false;
    for (at, b) in prefix.bytes().enumerate() {
        if escaped {
            escaped = false;
        } else {
            match b {
                b'\\' => escaped = true,
                b'[' => old = false,
                b']' if old => return Some(at + 1),
                _ => {}
            }
        }
    }
    None
}

#[test]
fn generated_replacement_and_escape_sequences_match_label_model() {
    let tokens = [
        "a", "[", "]", "![", "\\]", "\\\\]", "[x](u)", "![x](v)", "](v)", "[]",
    ];
    for marker in ["[", "!["] {
        for a in tokens {
            for b in tokens {
                let prior = format!("a {marker} prior\n");
                let prefix = format!("{}{a}{b}", "x".repeat(1030));
                let header = format!("{prefix}|b\n");
                // This finite token model includes one inert direct or reference
                // tail. Later syntax still cancels; pending plain labels can wait.
                let rejected = closes_old_label(&prefix).is_some_and(|end| {
                    let suffix = &prefix[end..];
                    let syntax = |text: &str| {
                        text.contains(['&', '<', '\\', '*', '_', '~', '`', '$', '!', '[', ']'])
                    };
                    if let Some(label) = suffix.strip_prefix('[') {
                        match label.split_once(']') {
                            Some((body, rest)) => {
                                syntax(body) || syntax(rest) || rest.contains('(')
                            }
                            None => syntax(label),
                        }
                    } else {
                        syntax(suffix.strip_prefix("(v)").unwrap_or(suffix))
                    }
                });
                if rejected {
                    compare_plain(&format!("{prior}{header}-|-\n"));
                } else {
                    compare_header(&prior, &header);
                }
            }
        }
    }
}

#[test]
fn closed_container_labels_cancel_late_header_probes_in_deep_scopes() {
    for (open, prefix) in [
        ("> ".to_owned(), "> ".to_owned()),
        ("- ".repeat(250), " ".repeat(500)),
        (">".repeat(1000), ">".repeat(1000)),
    ] {
        let mut doc = Document::new();
        doc.append(&format!(
            "{open}![old\n{prefix}end](u) ![local](v) {}",
            "x".repeat(2048)
        ))
        .unwrap();
        assert_eq!(doc.links().len(), 2);
        let local = *doc.link_info(LinkId(1)).unwrap();
        doc.append(&format!("|![tail](w)\n{prefix}-|-\n{prefix}v|z\n"))
            .unwrap();
        doc.finish();
        assert_eq!(*doc.link_info(LinkId(1)).unwrap(), local);
        assert_eq!(doc.links().len(), 3);
        assert!(doc.tables().is_empty());
        assert_eq!(doc.link_destination(LinkId(0)), Some("u"));
        assert!(
            doc.blocks()
                .iter()
                .all(|b| b.container == doc.blocks()[0].container)
        );
    }
    compare_plain(&format!("![old\n{}|b\n> -|-\n", "x".repeat(1030)));
}

#[test]
fn dense_objects_and_columns_are_reused_and_preview_obeys_shared_budget() {
    let mut doc = Document::new();
    doc.append(&format!(
        "{} ![old\n{}",
        "*old ".repeat(4096),
        "![中🙂](u) $m$ [^n] ".repeat(4096)
    ))
    .unwrap();
    assert_eq!(doc.links().len(), 4096);
    assert_eq!(doc.maths().len(), 4096);
    let first = *doc.link_info(LinkId(0)).unwrap();
    doc.append(&format!("|{}\n", "h|".repeat(4095))).unwrap();
    let before = doc.work();
    let update = doc.append(&format!("|{}\n", "-:|".repeat(4096))).unwrap();
    assert!(update.new_links.is_empty());
    assert!(update.new_maths.is_empty());
    assert!(update.new_footnote_references.is_empty());
    assert_eq!(*doc.link_info(LinkId(0)).unwrap(), first);
    assert_eq!(doc.work().label_bytes, before.label_bytes);
    assert_eq!(doc.work().delimiter_steps - before.delimiter_steps, 4096);
    assert_eq!(doc.tables()[0].columns.len(), 4096);
    assert!(doc.work().inline_bytes <= doc.source().len() as u64);
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
fn generated_tables_and_rejections_match_source_range_model() {
    let mut source = String::new();
    let mut expected = Vec::new();
    for i in 0..400 {
        source.push_str(if i % 2 == 0 {
            "a [prior\n"
        } else {
            "a ![prior\n"
        });
        let start = source.len();
        source.push_str(&format!(
            "{}{}|b\r\n",
            "x".repeat(1030 + i),
            if i % 3 == 0 { "](u) [again" } else { "" }
        ));
        let delimiter_start = source.len();
        source.push_str(&format!(
            "{}|-{}\n",
            "-".repeat(1030),
            if i % 5 == 0 { "x" } else { "" }
        ));
        let delimiter_end = source.len();
        source.push_str("v|z\n");
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
        assert_eq!(doc.block(table.block).unwrap().source, span);
    }
    assert!(doc.work().inline_bytes <= source.len() as u64);
    assert!(doc.work().table_header_bytes <= source.len() as u64);
}

#[test]
fn confirmation_and_fallback_invalidate_old_tickets_and_merge_updates() {
    for success in [false, true] {
        let mut doc = Document::new();
        let mut view = FoldView::new(&doc, 1).unwrap();
        let mut merged = FoldView::new(&doc, 1).unwrap();
        let mut batch = Update::default();
        for chunk in ["a ![prior  \r\n", &"x".repeat(2048), "|end](v)\r\n"] {
            let update = doc.append(chunk).unwrap();
            view.apply(&doc, &update).unwrap();
            batch.merge(update).unwrap();
        }
        let mut heights = HeightIndex::default();
        heights.rebuild(&doc).unwrap();
        let ticket = heights.ticket(TileId(1)).unwrap();
        let key = doc.tile_key(TileId(1)).unwrap();
        let update = doc
            .append(if success { "-|-\n" } else { "wrong\n" })
            .unwrap();
        heights.apply(&doc, &update).unwrap();
        assert_ne!(doc.tile_key(TileId(1)).unwrap(), key);
        assert_eq!(
            heights.measure(ticket, 2),
            Err(LayoutError::StaleMeasurement)
        );
        view.apply(&doc, &update).unwrap();
        batch.merge(update).unwrap();
        assert_eq!(batch.new_links, 0..usize::from(!success));
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
