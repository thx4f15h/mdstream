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
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
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
fn unlimited() -> Budget {
    Budget::new(WorkLimit::unlimited())
}

#[test]
fn only_outer_raw_spaces_and_tabs_are_trimmed() {
    for (raw, text) in [
        ("  Foo *bar\nbaz*\t", "Foo bar\nbaz"),
        ("   中🙂 \t", "中🙂"),
        (" \t&#32; &#9; \t", "  \t"),
        ("  ` a ` \t", "a"),
        ("  `unterminated \t", "`unterminated"),
        ("  $unterminated \t", "$unterminated"),
        ("  [unterminated \t", "[unterminated"),
        ("  *unterminated \t", "*unterminated"),
        ("  x\\  \t", "x\\"),
        ("  \u{a0}x\u{a0} \t", "\u{a0}x\u{a0}"),
        ("  \u{b}x\u{c} \t", "\u{b}x\u{c}"),
        ("  x\r \t", "x\r"),
        ("  x\r\r \t", "x\r\r"),
        ("  a  \nb \t", "a\nb"),
        ("  a\n  b \t", "a\n  b"),
        ("  a\t\nb \t", "a\t\nb"),
    ] {
        for prefix in ["", "> "] {
            let continuation = prefix;
            for newline in ["\n", "\r\n"] {
                for ending in ["", "\n", "\r\n"] {
                    let body = raw.replace('\n', &format!("{newline}{continuation}"));
                    let source = format!("{prefix}{body}{newline}{continuation}==={ending}");
                    let doc = parse(&source);
                    let title = *doc
                        .headings()
                        .entries()
                        .first()
                        .unwrap_or_else(|| panic!("no heading: {source:?}"));
                    let expected_raw = body.trim_matches([' ', '\t']);
                    assert_eq!(doc.text(title.content), Some(expected_raw), "{source:?}");
                    assert_eq!(visible(&doc, title.content), text, "{source:?}");
                    assert_eq!(
                        visible(&doc, title.source),
                        format!("{text}\n"),
                        "{source:?}"
                    );
                    assert_eq!(title.block, BlockId(0));
                    assert_eq!(title.source, Span::new(0, source.len()));
                }
            }
        }
    }
}

#[test]
fn raw_envelopes_match_independent_whole_string_model() {
    let mut seed = 0x005e_7e87_u64;
    for case in 0..1200 {
        let prefix = ">".repeat(case % 7);
        let mut raw = format!("{}a", " ".repeat(case % 4));
        for _ in 0..24 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            raw.push_str(["a", "*", " ", "\t", "`", "$", "中", "\u{a0}"][(seed % 8) as usize]);
        }
        if case % 3 == 0 {
            raw.push_str(&"x".repeat(1100));
        }
        raw.push_str(&format!("\r\n{prefix} b "));
        raw.push_str([" ", "\t", "\r \t", "&#32; \t"][case % 4]);
        let source = format!("{prefix}{raw}\r\n{prefix}===\n");
        let doc = parse(&source);
        let title = doc.headings().entries()[0];
        assert_eq!(
            doc.text(title.content),
            Some(raw.trim_matches([' ', '\t'])),
            "case {case}"
        );
        assert!(doc.work().inline_bytes <= source.len() as u64);
    }
}

#[test]
fn rejected_underlines_and_scope_boundaries_do_not_trim_prose() {
    for suffix in ["==x\n", "\n", "# stop\n", "    code\n", "```\nx\n```\n"] {
        let source = format!("  x \t\n{suffix}");
        let doc = parse(&source);
        assert_eq!(
            visible(&doc, Span::new(0, "  x \t\n".len())),
            "  x \t\n",
            "{suffix:?}"
        );
    }
    let source = format!("  old \t\n{}\n===\n", "=".repeat(BLOCK_LOOKAHEAD + 1));
    let doc = parse(&source);
    let title = doc.headings().entries()[0];
    // The overlong underline is ordinary interior text; only the outer edges trim.
    assert_eq!(
        visible(&doc, title.content),
        format!("old \t\n{}", "=".repeat(BLOCK_LOOKAHEAD + 1))
    );
    let doc = parse(">   old \t\n>\n>   new \t\n> ===\n");
    assert_eq!(visible(&doc, doc.headings().entries()[0].content), "new");
    let doc = parse("- a\n  - child\n    \n  b \t\n  ===\n");
    assert_eq!(visible(&doc, doc.headings().entries()[0].content), "b");
}

#[test]
fn failed_table_candidates_feed_edges_once_and_success_resets_them() {
    for prefix in ["", "> "] {
        for row in [
            "a|b \t".to_owned(),
            format!("{}*|b \t", "x".repeat(1100)),
            format!("{}`|b \t", "x".repeat(1100)),
            format!("{}$|b \t", "x".repeat(1100)),
            format!("{}](u)|b \t", "x".repeat(1100)),
        ] {
            let source = format!("{prefix}  *`$[first\n{prefix}{row}\r\n{prefix}===\n");
            let doc = parse(&source);
            assert!(doc.tables().is_empty());
            let title = doc.headings().entries()[0];
            assert_eq!(title.content.start, prefix.len() + 2);
            assert_eq!(doc.text(title.content).unwrap().chars().last(), Some('b'));
            assert!(doc.work().inline_bytes <= source.len() as u64);
        }
    }
    for barrier in ["> a|b\n> -|-\n>\n", "> # atx \t\n", "> [r]: /u\n"] {
        let source = format!(">  before\n{barrier}>   fresh \t\n> ===\n");
        let doc = parse(&source);
        assert_eq!(
            visible(&doc, doc.headings().entries().last().unwrap().content),
            "fresh"
        );
    }
}

#[test]
fn million_byte_tails_confirm_without_old_reads_or_ancestor_updates() {
    for prefix in [String::new(), ">".repeat(1000)] {
        let mut doc = Document::new();
        doc.append(&format!("{prefix}  {}", "x".repeat(1_000_000)))
            .unwrap();
        let before = doc.work();
        for _ in 0..1000 {
            doc.append(" \t").unwrap();
        }
        assert_eq!(doc.work().inline_bytes - before.inline_bytes, 2000);
        assert_eq!(doc.work().classified_bytes, before.classified_bytes);
        assert_eq!(doc.work().container_steps, before.container_steps);
        assert_eq!(doc.work().heading_steps, before.heading_steps);
        doc.append(&" \t".repeat(500_000)).unwrap();
        doc.append("\r\n").unwrap();
        let before = doc.work();
        let update = doc.append(&format!("{prefix}=\n")).unwrap();
        assert_eq!(doc.work().inline_bytes, before.inline_bytes);
        assert_eq!(doc.work().label_bytes, before.label_bytes);
        assert!(doc.work().tree_visits - before.tree_visits < 1000);
        assert_eq!(update.new_headings, 0..1);
        assert_eq!(doc.headings().entries()[0].content.len(), 1_000_000);
        assert!(doc.work().inline_bytes <= doc.source().len() as u64);
    }
}

#[test]
fn thousands_of_leaves_and_historical_objects_are_not_revisited() {
    let mut doc = Document::new();
    doc.append(&">   [x](u) $m$ \t\n> ===\n".repeat(4096))
        .unwrap();
    let old = doc.headings().entries()[2048];
    let link = *doc.link_info(LinkId(2048)).unwrap();
    let math = doc.maths()[2048];
    doc.append(">   start\n").unwrap();
    doc.append(&"> middle\n".repeat(4096)).unwrap();
    doc.append("> last \t\n").unwrap();
    let blocks = doc.blocks().to_vec();
    let before = doc.work();
    let update = doc.append("> ===\n").unwrap();
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert_eq!(doc.work().label_bytes, before.label_bytes);
    assert_eq!(doc.work().heading_steps - before.heading_steps, 2);
    assert!(doc.work().tree_visits - before.tree_visits < 1000);
    assert_eq!(update.new_headings, 4096..4097);
    assert_eq!(&doc.blocks()[4097..8193], &blocks[4097..8193]);
    assert_eq!(doc.headings().entries()[2048], old);
    assert_eq!(*doc.link_info(LinkId(2048)).unwrap(), link);
    assert_eq!(doc.maths()[2048], math);
    let title = doc.headings().entries()[4096];
    assert_eq!(
        visible(&doc, title.content),
        format!("start\n{}last", "middle\n".repeat(4096))
    );
}

#[test]
fn budgeted_mapping_search_and_slug_preserve_generated_whitespace() {
    let source = ">   **中**\r\n> 🙂 &amp; ` a ` &#32; \t\r\n> ===\n";
    let doc = parse(source);
    let span = Span::new(0, source.len());
    let expected = "中\n🙂 & a  \n";
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
        .flat_map(|(at, ch)| {
            doc.display_runs_in(Span::new(at, at + ch.len_utf8()))
                .unwrap()
        })
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    assert_eq!(tiled, expected);
    assert_eq!(scalars, expected);
    let mut cursor = doc.display_cursor(span).unwrap();
    let mut output = String::new();
    for _ in 0..source.len() * 2 {
        let at = cursor.source_position();
        if at < span.end {
            assert_eq!(
                cursor
                    .next(&doc, &mut Budget::new(WorkLimit::default()))
                    .unwrap(),
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
        assert!(budget.used().bytes <= 6);
    }
    assert_eq!(output, expected);
    assert_eq!(cursor.source_position(), span.end);
    let mut search = HeadingSearch::new(&doc, "a  ", 6).unwrap();
    let mut anchors = HeadingAnchors::new(&doc);
    let mut hits = Vec::new();
    for _ in 0..500 {
        let mut budget = tiny();
        let a = anchors.advance(&doc, &mut budget).unwrap();
        let s = search.advance(&doc, &mut budget, &mut hits).unwrap();
        assert!(
            budget.used().bytes <= 6 && budget.used().queries <= 1 && budget.used().objects <= 1
        );
        if a == Completion::EndOfDocument && s == Completion::EndOfDocument {
            break;
        }
    }
    assert!(anchors.is_complete());
    assert_eq!(hits.len(), 1);
    assert_eq!(doc.text(hits[0].source), Some("a ` &#32;"));
    assert_eq!(
        anchors
            .get(&doc, HeadingId(0))
            .unwrap()
            .unwrap()
            .slug
            .to_string(),
        "中---a--"
    );
}

#[test]
fn promotion_invalidates_whitespace_tiles_and_tasks_and_preserves_ids() {
    let mut doc = Document::new();
    let mut batch = doc
        .append(&format!(">   [x](u) $m$\n> {} \t\n", "x".repeat(9000)))
        .unwrap();
    let link = *doc.link_info(LinkId(0)).unwrap();
    let math = doc.maths()[0];
    let mut heights = HeightIndex::default();
    heights.apply(&doc, &batch).unwrap();
    let mut checkpoints = CheckpointIndex::new();
    checkpoints.rebuild(&doc);
    let tail = doc.tile_at(doc.source().len() - 2).unwrap();
    let tickets = [
        heights.ticket(TileId(0)).unwrap(),
        heights.ticket(tail).unwrap(),
    ];
    let keys = [
        doc.tile_key(TileId(0)).unwrap(),
        doc.tile_key(tail).unwrap(),
    ];
    checkpoints.insert(tail, 7_u32).unwrap();
    let mut cursor = doc
        .display_cursor(Span::new(0, doc.source().len()))
        .unwrap();
    let mut search = HeadingSearch::new(&doc, "x", 6).unwrap();
    let mut anchors = HeadingAnchors::new(&doc);
    let mut view = FoldView::<()>::new(&doc, 1).unwrap();
    view.set_defaults(FoldDefaults {
        heading_levels: [true; 6],
        ..Default::default()
    });
    let update = doc.append("> ===\n").unwrap();
    assert_eq!(update.new_blocks, 2..2);
    assert_eq!(update.changed_blocks, 0..2);
    apply_layout_update(&doc, &update, &mut heights, &mut checkpoints).unwrap();
    view.apply(&doc, &update).unwrap();
    batch.merge(update).unwrap();
    for (i, tile) in [TileId(0), tail].into_iter().enumerate() {
        assert_ne!(doc.tile_key(tile).unwrap(), keys[i]);
        assert_eq!(
            heights.measure(tickets[i], 7),
            Err(LayoutError::StaleMeasurement)
        );
    }
    assert_eq!(checkpoints.get(tail), None);
    assert_eq!(*doc.link_info(LinkId(0)).unwrap(), link);
    assert_eq!(doc.maths()[0], math);
    assert_eq!(doc.headings().entries()[0].block, BlockId(0));
    let mut empty = Budget::new(WorkLimit::default());
    assert_eq!(
        cursor.next(&doc, &mut empty),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(
        search.advance(&doc, &mut empty, &mut Vec::new()),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(
        anchors.advance(&doc, &mut empty),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(empty.used(), WorkLimit::default());
    let mut delayed = HeightIndex::default();
    delayed.apply(&doc, &batch).unwrap();
    assert_eq!(heights.total_rows(), delayed.total_rows());
    assert!(view.is_collapsed(FoldId::Heading(HeadingId(0))));
    let body = doc.source().len();
    let update = doc
        .append(&format!("> {}\n", "hidden ".repeat(150_000)))
        .unwrap();
    view.apply(&doc, &update).unwrap();
    let mut budget = Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        ..Default::default()
    });
    assert!(matches!(
        view.hidden_at(&doc, body, &mut budget).unwrap(),
        FoldLookup::Hidden(_)
    ));
    assert_eq!(budget.used().bytes, 0);
    let mut navigator = Navigator::new();
    assert_eq!(
        navigator
            .navigate(&doc, view.heights(), NavAction::Heading(HeadingId(0)), 2)
            .unwrap()
            .source,
        Some(0)
    );
    view.set_preview(&doc, Some(FoldId::Heading(HeadingId(0))), &mut unlimited())
        .unwrap();
    assert_eq!(
        view.hidden_at(&doc, body, &mut unlimited()).unwrap(),
        FoldLookup::Visible
    );
}

#[test]
fn list_padding_and_partial_tab_continuations_keep_source_mapping() {
    for (source, expected) in [
        ("-   first\n    last \t\n    ===\n", "first\nlast"),
        ("- first\n\tsecond \t\n  ===\n", "first\n  second"),
        ("> - [x] first\n>   last \t\n>   ===\n", "first\nlast"),
    ] {
        let doc = parse(source);
        let title = doc.headings().entries()[0];
        assert_eq!(visible(&doc, title.content), expected, "{source:?}");
        assert_eq!(
            doc.text(title.content).unwrap().chars().last(),
            expected.chars().last()
        );
        assert_eq!(title.source, Span::new(0, source.len()));
        assert_eq!(title.block, BlockId(0));
        assert!(title.container.is_some());
    }
}
