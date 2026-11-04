use mdstream::*;

#[path = "support/snapshot.rs"]
mod support;

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

fn title(doc: &Document, id: usize) -> String {
    doc.display_runs_in(doc.headings().entries()[id].content)
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect()
}

#[test]
fn atx_indentation_uses_physical_columns_and_keeps_scope() {
    let mut cases = 0;
    for (before, prefix, residual) in [
        ("", ">", 0),
        ("", "> ", 0),
        ("", ">> ", 0),
        ("", "> > ", 0),
        ("", ">>> ", 0),
        ("- seed\n", "  ", 0),
        ("1. seed\n", "   ", 0),
        ("> - seed\n", ">   ", 0),
        ("- seed\n", "\t", 2),
        ("> seed\n", ">\t", 2),
        ("- - seed\n", "\t", 0),
    ] {
        for whitespace in ["", " ", "   ", "\t", " \t", "  \t", "\t ", "\t\t"] {
            let mut column = 0;
            for byte in prefix.bytes() {
                column += if byte == b'\t' { 4 - column % 4 } else { 1 };
            }
            let start = column;
            for byte in whitespace.bytes() {
                column += if byte == b'\t' { 4 - column % 4 } else { 1 };
            }
            // A new quote consumes one column of a directly adjacent tab.
            let separator = usize::from(
                before.is_empty() && prefix.ends_with('>') && whitespace.starts_with('\t'),
            );
            let indent = residual + column - start - separator;
            for level in 1..=7 {
                for ending in ["\n", "\r\n", ""] {
                    let source = format!(
                        "{before}{prefix}{whitespace}{} 中🙂 *title* ### {ending}",
                        "#".repeat(level)
                    );
                    let doc = parse(&source);
                    let headings = doc.headings().entries();
                    assert_eq!(
                        headings.len(),
                        usize::from(indent <= 3 && level <= 6),
                        "{source:?}"
                    );
                    if let Some(heading) = headings.first() {
                        assert_eq!(heading.level, level as u8);
                        assert_eq!(title(&doc, 0), "中🙂 title");
                        assert!(heading.container.is_some());
                        if !before.is_empty() {
                            assert_eq!(heading.container, doc.blocks()[0].container);
                        }
                        assert_eq!(
                            doc.block(heading.block).unwrap().container,
                            heading.container
                        );
                        assert_eq!(&source[heading.source.range()], &source[before.len()..]);
                    }
                    assert_eq!(doc.source(), source);
                    cases += 1;
                }
            }
        }
    }
    assert_eq!(cases, 1848);
}

#[test]
fn failures_empty_titles_fences_and_window_boundaries() {
    for (source, expected) in [
        ("> \t# title\n", Some("title")),
        ("> \t###\n", Some("")),
        ("> \t###", Some("")),
        ("> \t###\t中 ###\r\n", Some("中")),
        ("> \t#no-space\n", None),
        ("> \t####### title\n", None),
        ("> \t\\# title\n", None),
        ("> \t\t# title\n", None),
        ("\t# title\n", None),
        ("> seed\n>\t # title\n", Some("title")),
        ("> seed\n>\t  # title\n", None),
        ("- > seed\n\t# title\n", None),
        ("> ```\n> \t# title\n> ```\n", None),
        ("> $$\n> \t# title\n> $$\n", None),
    ] {
        let doc = parse(source);
        assert_eq!(
            doc.headings().entries().len(),
            usize::from(expected.is_some()),
            "{source:?}"
        );
        if let Some(expected) = expected {
            assert_eq!(title(&doc, 0), expected, "{source:?}");
        }
    }
    // The shared window includes containers and indentation. A cut marker
    // remains ordinary text, even when later input could complete a heading.
    for depth in [1016, 1017, 1018, 1019, 1020] {
        let prefix = format!("{} \t", ">".repeat(depth));
        let source = format!("{prefix}### title\n");
        let doc = parse(&source);
        assert_eq!(
            doc.headings().entries().len(),
            usize::from(prefix.len() + 3 < BLOCK_LOOKAHEAD),
            "depth {depth}"
        );
    }
}

#[test]
fn splits_prefixes_eof_and_late_heading_keep_identity() {
    for source in [
        "> \t## 中🙂 *title* ###\r\n> tail\n",
        "1. seed\n   \t### title\n   tail\n",
        "- seed\n\t # title\n- next\n",
        "> seed\n>\t # title\n> ## child\n",
        "> \t###",
        "> \t#no-space\n> after\n",
        "> \t####### title\n",
        "- > seed\n\t# title\n",
    ] {
        let whole = support::snapshot(&parse(source));
        for split in (0..=source.len()).filter(|&at| source.is_char_boundary(at)) {
            let mut doc = Document::new();
            doc.append(&source[..split]).unwrap();
            doc.append(&source[split..]).unwrap();
            doc.finish();
            assert_eq!(support::snapshot(&doc), whole, "split {split}: {source:?}");
        }
        let mut doc = Document::new();
        for (at, ch) in source.char_indices() {
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            let text = &source[..at + ch.len_utf8()];
            let mut prefix = Document::new();
            prefix.append(text).unwrap();
            assert_eq!(support::snapshot(&doc), support::snapshot(&prefix));
            assert_eq!(doc.pending(), prefix.pending());
            let mut eof = Document::new();
            for ch in text.chars() {
                eof.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            }
            eof.finish();
            assert_eq!(support::snapshot(&eof), support::snapshot(&parse(text)));
        }
    }
    let mut doc = Document::new();
    doc.append("> seed\n> \t#").unwrap();
    let container = doc.blocks()[0].container;
    let key = doc.tile_key(TileId(0)).unwrap();
    let mut cursor = doc
        .display_cursor(Span::new(0, doc.source().len()))
        .unwrap();
    doc.append(" title\n> \t## child\n# outside\n").unwrap();
    doc.finish();
    assert_eq!(visible(&doc), "seed\ntitle\nchild\noutside\n");
    let headings = doc.headings();
    assert_eq!(headings.entries()[0].container, container);
    assert_eq!(headings.entries()[1].container, container);
    assert_eq!(headings.parent(HeadingId(1)), Some(HeadingId(0)));
    assert_eq!(headings.entries()[2].container, None);
    assert_ne!(doc.tile_key(TileId(0)).unwrap(), key);
    assert_eq!(
        cursor.next(&doc, &mut Budget::new(WorkLimit::unlimited())),
        Err(LayoutError::StaleContinuation)
    );
}

#[test]
fn mapping_tiles_slices_and_six_byte_budget_hide_indent() {
    let source = format!("> {}\n> \t## 中🙂 ###\r\n", "x".repeat(251));
    let doc = parse(&source);
    let tab = source.find('\t').unwrap();
    assert_eq!(tab, 256);
    let expected = format!("{}\n中🙂\n", "x".repeat(251));
    assert_eq!(visible(&doc), expected);
    let mut covered = 0;
    for run in doc.display_runs_in(Span::new(0, source.len())).unwrap() {
        assert_eq!(run.source.start, covered);
        assert_eq!(run.text, &source[run.source.range()]);
        assert!(run.replacement.is_none());
        if run.source.start <= tab && tab < run.source.end {
            assert!(run.style.contains(Style::HIDDEN));
        }
        covered = run.source.end;
    }
    assert_eq!(covered, source.len());
    let sliced: String = source
        .char_indices()
        .flat_map(|(at, ch)| {
            doc.display_runs_in(Span::new(at, at + ch.len_utf8()))
                .unwrap()
        })
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    assert_eq!(sliced, expected);
    let tiled: String = (0..doc.tile_count())
        .flat_map(|i| doc.display_runs(TileId(i)).unwrap())
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    assert_eq!(tiled, expected);
    let mut cursor = doc.display_cursor(Span::new(0, source.len())).unwrap();
    let mut output = String::new();
    while cursor.source_position() < source.len() {
        let position = cursor.source_position();
        assert_eq!(
            cursor
                .next(&doc, &mut Budget::new(WorkLimit::default()))
                .unwrap(),
            DisplayStep::BudgetExhausted
        );
        assert_eq!(cursor.source_position(), position);
        let mut budget = Budget::new(WorkLimit {
            bytes: 6,
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        let DisplayStep::Run(run) = cursor.next(&doc, &mut budget).unwrap() else {
            panic!("must advance")
        };
        assert!(budget.used().bytes <= 6);
        if !run.style.contains(Style::HIDDEN) {
            output.push_str(run.text);
        }
    }
    assert_eq!(output, expected);
}

#[test]
fn deep_long_headings_and_failed_markers_do_not_rescan_old_text() {
    let prefix = format!("{} ", ">".repeat(1000));
    let mut doc = Document::new();
    doc.append(&format!("{prefix}\t# {}", "x".repeat(1_000_000)))
        .unwrap();
    assert_eq!(doc.headings().entries().len(), 1);
    let block = doc.headings().entries()[0].block;
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("x").unwrap();
    }
    let after = doc.work();
    assert_eq!(after.input_bytes - before.input_bytes, 1000);
    assert_eq!(after.classified_bytes, before.classified_bytes);
    assert_eq!(after.container_bytes, before.container_bytes);
    assert_eq!(after.container_steps, before.container_steps);
    assert_eq!(after.inline_bytes - before.inline_bytes, 1000);
    assert!(after.tree_visits - before.tree_visits < 100_000);
    doc.append(&format!("\n{prefix}\t## child\n")).unwrap();
    doc.finish();
    assert_eq!(title(&doc, 0), "x".repeat(1_001_000));
    assert_eq!(doc.headings().entries()[0].block, block);
    assert_eq!(doc.headings().parent(HeadingId(1)), Some(HeadingId(0)));
    assert_eq!(doc.container_count(), 1000);
    assert!(doc.work().classified_bytes < 30 * BLOCK_LOOKAHEAD as u64);
    let mut failed = Document::new();
    failed
        .append(&format!("{prefix}\t{}", "#".repeat(2048)))
        .unwrap();
    let before = failed.work();
    for _ in 0..1000 {
        failed.append("#").unwrap();
    }
    let after = failed.work();
    assert_eq!(after.classified_bytes, before.classified_bytes);
    assert_eq!(after.container_bytes, before.container_bytes);
    assert_eq!(after.container_steps, before.container_steps);
    failed.append(" title\n").unwrap();
    failed.finish();
    assert!(failed.headings().entries().is_empty());
    let source = "> \t# title\n".repeat(4096);
    let doc = parse(&source);
    assert_eq!(doc.headings().entries().len(), 4096);
    assert_eq!(visible(&doc), "title\n".repeat(4096));
    assert!(doc.work().classified_bytes < 10 * source.len() as u64);
    assert!(doc.work().container_steps < 8 * source.len() as u64);
}

#[test]
fn heading_search_anchors_and_fold_updates_share_budget() {
    let mut doc = Document::new();
    let mut view = FoldView::<()>::new(&doc, 1).unwrap();
    view.set_defaults(FoldDefaults {
        heading_levels: [true; 6],
        ..Default::default()
    });
    for chunk in [
        "> \t# title\n",
        "> body\n> \t## title\n> tail\n",
        "# outside\n",
    ] {
        let update = doc.append(chunk).unwrap();
        view.apply(&doc, &update).unwrap();
    }
    let update = doc.finish();
    view.apply(&doc, &update).unwrap();
    assert_eq!(doc.headings().entries().len(), 3);
    for id in [HeadingId(0), HeadingId(1)] {
        assert!(view.is_collapsed(FoldId::Heading(id)));
        let candidate = doc.fold_candidate(FoldId::Heading(id)).unwrap();
        assert_eq!(candidate.id, FoldId::Heading(id));
    }
    let mut anchors = HeadingAnchors::new(&doc);
    let mut search = HeadingSearch::new(&doc, "title", 6).unwrap();
    let mut hits = Vec::new();
    let mut done = false;
    for _ in 0..1000 {
        let mut budget = Budget::new(WorkLimit {
            bytes: 6,
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        let a = anchors.advance(&doc, &mut budget).unwrap();
        let s = search.advance(&doc, &mut budget, &mut hits).unwrap();
        assert!(
            budget.used().bytes <= 6 && budget.used().queries <= 1 && budget.used().objects <= 1
        );
        if a == Completion::EndOfDocument && s == Completion::EndOfDocument {
            done = true;
            break;
        }
    }
    assert!(done);
    assert_eq!(anchors.indexed_headings(), 3);
    assert_eq!(
        anchors
            .get(&doc, HeadingId(0))
            .unwrap()
            .unwrap()
            .slug
            .to_string(),
        "title"
    );
    assert_eq!(
        anchors
            .get(&doc, HeadingId(1))
            .unwrap()
            .unwrap()
            .slug
            .to_string(),
        "title-1"
    );
    assert_eq!(
        hits.iter().map(|h| h.heading).collect::<Vec<_>>(),
        [HeadingId(0), HeadingId(1)]
    );
    let source = doc.source().find("> tail").unwrap();
    assert!(matches!(
        view.hidden_at(&doc, source, &mut Budget::new(WorkLimit::unlimited())).unwrap(),
        FoldLookup::Hidden(c) if c.id == FoldId::Heading(HeadingId(0))
    ));
}
