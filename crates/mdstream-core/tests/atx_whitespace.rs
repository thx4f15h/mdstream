use mdstream::*;

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

#[test]
fn raw_spaces_tabs_and_closing_hashes_are_removed_before_inline_parsing() {
    for (raw, content, text) in [
        ("foo                     ", "foo", "foo"),
        ("foo    ###   \t", "foo", "foo"),
        ("\t  *中* &amp; ` a ` \t#\t", "*中* &amp; ` a `", "中 & a"),
        ("###\t", "", ""),
        (" \t", "", ""),
        ("foo ### b  ", "foo ### b", "foo ### b"),
        ("foo#  ", "foo#", "foo#"),
        ("foo \\###  ", "foo \\###", "foo ###"),
        ("foo #\\##  ", "foo #\\##", "foo ###"),
        ("foo ### ###\t", "foo ###", "foo ###"),
        ("foo\u{a0}\t", "foo\u{a0}", "foo\u{a0}"),
        ("foo\u{c} \t", "foo\u{c}", "foo\u{c}"),
        ("foo&#32; \t", "foo&#32;", "foo "),
        ("foo\\  ", "foo\\", "foo\\"),
    ] {
        for prefix in ["# ", "> - ##\t", "- item\n\t# "] {
            for ending in ["", "\n", "\r\n"] {
                let source = format!("{prefix}{raw}{ending}");
                let doc = parse(&source);
                let heading = doc.headings().entries().last().unwrap();
                assert_eq!(doc.text(heading.content), Some(content), "{source:?}");
                assert_eq!(visible(&doc, heading.content), text, "{source:?}");
                assert_eq!(
                    visible(&doc, heading.source),
                    format!("{text}{}", if ending.is_empty() { "" } else { "\n" }),
                    "{source:?}"
                );
            }
        }
    }
}

#[test]
fn long_atx_closure_trims_tail_and_preserves_failed_candidates() {
    for ending in ["", "\n", "\r\n"] {
        let body = "中🙂".repeat(1000);
        for (tail, kept) in [(" \t### \t", ""), ("   ", ""), (" ## x \t", " ## x")] {
            let source = format!("> # {body}{tail}{ending}");
            let doc = parse(&source);
            let heading = doc.headings().entries()[0];
            assert_eq!(doc.text(heading.content).unwrap(), format!("{body}{kept}"));
        }
    }
}

#[test]
fn raw_content_matches_independent_trim_model() {
    // Independent whole-string model of the raw ATX rule; no library helpers.
    let mut seed = 0x005e_eda7_u64;
    for case in 0..1600 {
        let mut raw = if case % 4 == 0 {
            "x".repeat(1050)
        } else {
            String::new()
        };
        for _ in 0..40 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            raw.push_str(["a", "#", " ", "\t", "\\", "中", "\u{a0}", "\u{c}"][(seed % 8) as usize]);
        }
        raw.push_str(["   ", " ###\t", "#", " ## x\t"][case % 4]);
        let trimmed = raw.trim_matches([' ', '\t']);
        let without_hashes = trimmed.trim_end_matches('#');
        let expected = if without_hashes.len() < trimmed.len()
            && (without_hashes.is_empty() || without_hashes.ends_with([' ', '\t']))
        {
            without_hashes.trim_end_matches([' ', '\t'])
        } else {
            trimmed
        };
        let source = format!("> ## {raw}\r\n");
        let doc = parse(&source);
        let heading = doc.headings().entries()[0];
        assert_eq!(doc.text(heading.content), Some(expected), "case {case}");
        assert_eq!(heading.source, Span::new(0, source.len()));
        assert_eq!(heading.block, BlockId(0));
        assert_eq!(heading.container, Some(ContainerId(0)));
    }
}

#[test]
fn million_byte_tails_are_summarized_once_and_confirm_without_body_reads() {
    for prefix in ["# ".to_owned(), format!("{} # ", ">".repeat(1000))] {
        for tail in [
            " ".repeat(1_000_000),
            format!(" {}\t", "#".repeat(1_000_000)),
        ] {
            let mut doc = Document::new();
            let start = format!("{prefix}{}", "x".repeat(1_000_000));
            doc.append(&start).unwrap();
            let before = doc.work();
            doc.append(&tail).unwrap();
            assert_eq!(doc.work().atx_bytes - before.atx_bytes, tail.len() as u64);
            assert_eq!(doc.work().inline_bytes, before.inline_bytes);
            let nodes = doc.annotation_node_count();
            let containers = doc.container_arena_stats();
            for _ in 0..1000 {
                doc.append("\t").unwrap();
            }
            assert_eq!(
                doc.work().atx_bytes - before.atx_bytes,
                tail.len() as u64 + 1000
            );
            assert_eq!(doc.work().classified_bytes, before.classified_bytes);
            assert_eq!(doc.work().container_steps, before.container_steps);
            assert_eq!(doc.work().container_bytes, before.container_bytes);
            assert_eq!(doc.work().heading_steps, before.heading_steps);
            assert_eq!(doc.work().inline_bytes, before.inline_bytes);
            assert_eq!(doc.annotation_node_count(), nodes);
            assert_eq!(doc.container_arena_stats(), containers);
            let before = doc.work();
            let update = doc.finish();
            assert_eq!(doc.work().atx_bytes, before.atx_bytes);
            assert_eq!(doc.work().inline_bytes, before.inline_bytes);
            assert!(doc.work().tree_visits - before.tree_visits < 200);
            assert!(update.changed_headings.contains(&0));
            assert_eq!(doc.headings().entries()[0].content.len(), 1_000_000);
            assert_eq!(doc.headings().entries()[0].block, BlockId(0));
        }
    }
}

#[test]
fn leading_whitespace_and_rejected_suffixes_have_linear_work() {
    let mut doc = Document::new();
    doc.append(&format!("# {}", " \t".repeat(500_000))).unwrap();
    assert_eq!(doc.work().inline_bytes, 0);
    let before = doc.work();
    doc.append("中").unwrap();
    assert_eq!(doc.work().inline_bytes - before.inline_bytes, 3);
    assert_eq!(doc.headings().entries()[0].content.start, 1_000_002);
    doc.append(&format!(" {}\t", "#".repeat(1_000_000)))
        .unwrap();
    let before = doc.work();
    doc.append("x").unwrap();
    // Failure consumes the deferred raw interval once; it never replays it.
    assert_eq!(doc.work().atx_bytes - before.atx_bytes, 1);
    assert_eq!(doc.work().inline_bytes - before.inline_bytes, 1_000_003);
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("x").unwrap();
    }
    assert_eq!(doc.work().atx_bytes - before.atx_bytes, 1000);
    assert_eq!(doc.work().inline_bytes - before.inline_bytes, 1000);
    doc.append(" \t###\n").unwrap();
    doc.finish();
    let heading = doc.headings().entries()[0];
    assert!(doc.text(heading.content).unwrap().starts_with("中 #"));
    assert!(
        doc.text(heading.content)
            .unwrap()
            .ends_with(&"x".repeat(1001))
    );
    assert!(doc.work().atx_bytes <= doc.source().len() as u64);
    assert!(doc.work().inline_bytes <= doc.source().len() as u64);
}

#[test]
fn only_space_and_tab_are_atx_separators_and_trimmable_content() {
    for ch in ['\u{b}', '\u{c}', '\u{a0}'] {
        assert!(parse(&format!("#{ch}x\n")).headings().entries().is_empty());
        let doc = parse(&format!("# {ch}x{ch}## \t\n"));
        assert_eq!(
            doc.text(doc.headings().entries()[0].content).unwrap(),
            format!("{ch}x{ch}##")
        );
    }
    // Keep the existing LF/CRLF line dialect: an isolated CR is body text.
    for raw in ["x\r", "x \r", "x\r \t", "x ##\r", "\r x"] {
        let doc = parse(&format!("# {raw}"));
        assert_eq!(
            doc.text(doc.headings().entries()[0].content).unwrap(),
            raw.trim_end_matches([' ', '\t'])
        );
    }
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
fn source_mapping_budget_search_and_slug_use_trimmed_raw_content() {
    let source = format!("# {}*中🙂* &amp; ` a ` &#32; \t###\t\r\n", " ".repeat(1100));
    let doc = parse(&source);
    let span = Span::new(0, source.len());
    let expected = "中🙂 & a  \n";
    assert_eq!(visible(&doc, span), expected);
    assert_eq!(doc.source(), source);
    assert_eq!(
        doc.runs_in(span)
            .unwrap()
            .map(|r| r.text)
            .collect::<String>(),
        source
    );
    let tiled: String = (0..doc.tile_count())
        .flat_map(|i| doc.display_runs(TileId(i)).unwrap())
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    let scalars: String = source
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
    let mut cursor = doc.display_cursor(span).unwrap();
    let mut output = String::new();
    let mut charged = 0;
    for _ in 0..source.len() * 2 {
        let mut empty = Budget::new(WorkLimit::default());
        let at = cursor.source_position();
        if at < span.end {
            assert_eq!(
                cursor.next(&doc, &mut empty).unwrap(),
                DisplayStep::BudgetExhausted
            );
            assert_eq!(cursor.source_position(), at);
        }
        let mut budget = tiny();
        match cursor.next(&doc, &mut budget).unwrap() {
            DisplayStep::Run(run) => {
                if !run.style.contains(Style::HIDDEN) {
                    output.push_str(run.text);
                }
            }
            DisplayStep::Complete => break,
            DisplayStep::BudgetExhausted => panic!("one scalar fits"),
        }
        charged += budget.used().bytes;
        assert!(budget.used().bytes <= 6);
    }
    assert_eq!(output, expected);
    assert_eq!(cursor.source_position(), span.end);
    assert!((source.len()..=source.len() * 6).contains(&charged));
    let mut search = HeadingSearch::new(&doc, "a  ", 6).unwrap();
    let mut hits = Vec::new();
    for _ in 0..500 {
        if search.advance(&doc, &mut tiny(), &mut hits).unwrap() == Completion::EndOfDocument {
            break;
        }
    }
    assert_eq!(hits.len(), 1);
    assert_eq!(doc.text(hits[0].source), Some("a ` &#32;"));
    let mut anchors = HeadingAnchors::new(&doc);
    for _ in 0..500 {
        if anchors.advance(&doc, &mut tiny()).unwrap() == Completion::EndOfDocument {
            break;
        }
    }
    assert!(anchors.is_complete());
    // Entity-generated spaces survive raw trimming; slug rules are unchanged.
    assert_eq!(
        anchors
            .get(&doc, HeadingId(0))
            .unwrap()
            .unwrap()
            .slug
            .to_string(),
        "中--a--"
    );
}

#[test]
fn late_confirmation_invalidates_old_tiles_tasks_and_measurements_with_stable_ids() {
    for ending in ["", "\r\n"] {
        let mut doc = Document::new();
        let mut batch = doc
            .append(&format!("# [link](u) $x$ *中* {}", "x".repeat(1100)))
            .unwrap();
        batch
            .merge(doc.append(&format!(" {}", "#".repeat(2000))).unwrap())
            .unwrap();
        let old_heading = doc.headings().entries()[0];
        let old_link = *doc.link_info(LinkId(0)).unwrap();
        let old_math = doc.maths()[0];
        let tail = doc.tile_at(old_heading.content.end - 1000).unwrap();
        let old_key = doc.tile_key(tail).unwrap();
        let mut heights = HeightIndex::default();
        heights.apply(&doc, &batch).unwrap();
        let ticket = heights.ticket(tail).unwrap();
        heights.measure(ticket, 7).unwrap();
        let mut checkpoints = CheckpointIndex::new();
        checkpoints.rebuild(&doc);
        checkpoints.insert(tail, 7_u32).unwrap();
        let mut view = FoldView::<()>::new(&doc, 1).unwrap();
        view.set_collapsed(&doc, FoldId::Heading(HeadingId(0)), true)
            .unwrap();
        let mut cursor = doc.display_cursor(old_heading.source).unwrap();
        let mut search = HeadingSearch::new(&doc, "##", 6).unwrap();
        let mut anchors = HeadingAnchors::new(&doc);
        let update = if ending.is_empty() {
            doc.finish()
        } else {
            doc.append(ending).unwrap()
        };
        assert!(update.changed_headings.contains(&0));
        assert!(update.changed_blocks.contains(&0));
        assert!(
            update
                .dirty
                .iter()
                .any(|span| span.start <= doc.tile(tail).unwrap().start)
        );
        apply_layout_update(&doc, &update, &mut heights, &mut checkpoints).unwrap();
        view.apply(&doc, &update).unwrap();
        batch.merge(update).unwrap();
        assert_ne!(doc.tile_key(tail).unwrap(), old_key);
        assert_eq!(
            heights.measure(ticket, 7),
            Err(LayoutError::StaleMeasurement)
        );
        assert_eq!(checkpoints.get(tail), None);
        assert!(view.is_collapsed(FoldId::Heading(HeadingId(0))));
        assert_eq!(doc.headings().entries()[0].block, old_heading.block);
        assert_eq!(*doc.link_info(LinkId(0)).unwrap(), old_link);
        assert_eq!(doc.maths()[0], old_math);
        assert!(doc.headings().entries()[0].content.end < old_heading.content.end);
        let mut budget = tiny();
        assert_eq!(
            cursor.next(&doc, &mut budget),
            Err(LayoutError::StaleContinuation)
        );
        assert_eq!(
            search.advance(&doc, &mut budget, &mut Vec::new()),
            Err(LayoutError::StaleContinuation)
        );
        assert_eq!(
            anchors.advance(&doc, &mut budget),
            Err(LayoutError::StaleContinuation)
        );
        assert_eq!(budget.used(), WorkLimit::default());
        let mut delayed = HeightIndex::default();
        delayed.apply(&doc, &batch).unwrap();
        // Invalidation preserves the old height as an estimate, so compare
        // after the same fresh measurement rather than assuming it resets to 1.
        assert_ne!(heights.height_state(tail), Some(HeightState::Measured));
        for index in [&mut heights, &mut delayed] {
            index.measure(index.ticket(tail).unwrap(), 1).unwrap();
        }
        assert_eq!(
            heights.viewport(0..1000, 0, 1000),
            delayed.viewport(0..1000, 0, 1000)
        );
    }
}

#[test]
fn historical_headings_and_inline_objects_are_not_revisited_on_closure() {
    let mut doc = Document::new();
    doc.append(&"# [x](u) $m$  ###\n".repeat(4096)).unwrap();
    let old = doc.headings().entries()[2048];
    let link = *doc.link_info(LinkId(2048)).unwrap();
    let math = doc.maths()[2048];
    doc.append(&format!("# {} \t###", "x".repeat(4096)))
        .unwrap();
    let before = doc.work();
    doc.finish();
    assert_eq!(doc.work().atx_bytes, before.atx_bytes);
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert_eq!(doc.work().label_bytes, before.label_bytes);
    assert_eq!(doc.work().heading_steps, before.heading_steps + 1);
    assert!(doc.work().tree_visits - before.tree_visits < 500);
    assert_eq!(doc.headings().entries()[2048], old);
    assert_eq!(*doc.link_info(LinkId(2048)).unwrap(), link);
    assert_eq!(doc.maths()[2048], math);
    assert_eq!(doc.headings().entries().last().unwrap().content.len(), 4096);
}
