use mdstream::*;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn codes(doc: &Document) -> Vec<BlockId> {
    doc.blocks()
        .iter()
        .enumerate()
        .filter_map(|(i, b)| matches!(b.kind, BlockKind::Code { .. }).then_some(BlockId(i)))
        .collect()
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
        queries: 1,
        objects: 1,
        bytes: 6,
        ..Default::default()
    })
}

#[test]
fn nested_fences_keep_physical_ranges_and_opaque_payloads() {
    let source = "# root\n> - [x] ```rust &amp;\r\n>   # *中🙂* &amp; $x$ [^f]\r\n>   > - [ ] literal\n>   a|b\n>   -|-\n>   [ref]: /url\n>   [^f]: body\n>   \\\n>   ````\r\n>   after\n";
    let doc = parse(source);
    let ids = codes(&doc);
    assert_eq!(ids, [BlockId(1)]);
    let block = doc.block(ids[0]).unwrap();
    assert_eq!(block.container, Some(ContainerId(2)));
    assert_eq!(doc.container_count(), 3);
    assert_eq!(block.state, SyntaxState::Sealed);
    assert_eq!(
        block.source,
        Span::new(
            source.find("> -").unwrap(),
            source.find(">   after").unwrap()
        )
    );
    let BlockKind::Code { info: Some(info) } = block.kind else {
        panic!("missing fence info")
    };
    assert_eq!(doc.text(info), Some("rust &amp;"));
    assert_eq!(
        visible(&doc, block.source),
        "# *中🙂* &amp; $x$ [^f]\n> - [ ] literal\na|b\n-|-\n[ref]: /url\n[^f]: body\n\\\n"
    );
    assert!(doc.tables().is_empty());
    assert!(doc.references().is_empty());
    assert!(doc.footnotes().is_empty());
    assert!(doc.maths().is_empty());
    assert_eq!(doc.headings().entries().len(), 1);
    for at in block.source.range() {
        assert_eq!(doc.block_at(at), Some(ids[0]));
        assert_eq!(doc.fold_candidate_at(at).unwrap().id, FoldId::Code(ids[0]));
    }
    assert!(
        doc.runs_in(block.source)
            .unwrap()
            .all(|r| r.style.contains(Style::CODE))
    );
}

#[test]
fn fence_closers_and_scope_exits_have_explicit_precedence() {
    for (head, body, boundary) in [
        ("> ```\n", "> raw\n", "outside\n"),
        ("> > ~~~\n", "> > raw\n", "> # outer\n"),
        ("- ```\n", "  raw\n", "- # sibling\n"),
        ("> - ```\n", ">   raw\n", "> - # sibling\n"),
        ("1. ~~~\n", "   raw\n", "2. # sibling\n"),
        ("> ```\n", "> raw\n", "\n"),
    ] {
        let doc = parse(&format!("{head}{body}{boundary}"));
        let block = doc.block(codes(&doc)[0]).unwrap();
        assert_eq!(block.source, Span::new(0, head.len() + body.len()));
        assert_eq!(block.state, SyntaxState::Sealed);
        assert_eq!(visible(&doc, block.source), "raw\n");
        assert!(!doc.fold_candidate(FoldId::Code(BlockId(0))).unwrap().open);
    }
    // Missing inner prefix cannot close the inner fence with an outer closer:
    // it seals the old block first, then opens a separate outer block.
    let doc = parse("> > ```\n> > raw\n> ```\n> outer\n> ```\n");
    assert_eq!(codes(&doc), [BlockId(0), BlockId(1)]);
    assert_eq!(doc.blocks()[0].container, Some(ContainerId(1)));
    assert_eq!(doc.blocks()[1].container, Some(ContainerId(0)));
    for marker in ["`", "~"] {
        let open = marker.repeat(4);
        let short = marker.repeat(3);
        let other = if marker == "`" { "~~~~" } else { "````" };
        let raw = format!("{short}\n{other}\n{open} trailing\n    {open}\n");
        let body = raw.lines().map(|l| format!("> {l}\n")).collect::<String>();
        let source = format!("> {open}\n{body}>   {open}{marker}\t\r\n> after\n");
        let doc = parse(&source);
        assert_eq!(codes(&doc).len(), 1);
        assert_eq!(visible(&doc, doc.blocks()[0].source), raw);
        assert_eq!(doc.blocks()[0].source.end, source.find("> after").unwrap());
    }
}

#[test]
fn indented_code_merges_only_within_exact_scope_and_blank_seals() {
    let source = "> text\n>     *raw* &amp;\n>       extra\n>\n>     next\n> >     nested\n>     outer\n    root\n- item\n      item code\n      more\n- sibling\n      sibling code\n";
    let doc = parse(source);
    let expected = [
        "*raw* &amp;\n  extra\n",
        "next\n",
        "nested\n",
        "outer\n",
        "root\n",
        "item code\nmore\n",
        "sibling code\n",
    ];
    let ids = codes(&doc);
    assert_eq!(ids.len(), expected.len());
    for (&id, expected) in ids.iter().zip(expected) {
        let block = doc.block(id).unwrap();
        assert_eq!(visible(&doc, block.source), expected);
        assert_eq!(block.kind, BlockKind::Code { info: None });
        assert_eq!(block.state, SyntaxState::Sealed);
    }
    assert_ne!(
        doc.block(ids[1]).unwrap().container,
        doc.block(ids[2]).unwrap().container
    );
    assert_eq!(
        doc.block(ids[1]).unwrap().container,
        doc.block(ids[3]).unwrap().container
    );
    assert_eq!(doc.block(ids[4]).unwrap().container, None);
    assert_ne!(
        doc.block(ids[5]).unwrap().container,
        doc.block(ids[6]).unwrap().container
    );
    // Five marker spaces start code; only one belongs to the item prefix,
    // so the continuation preserves its four extra spaces in the same block.
    let doc = parse("-     text\n          code\n> \ttext\n");
    assert_eq!(codes(&doc).len(), 1);
    assert_eq!(
        visible(&doc, doc.block(codes(&doc)[0]).unwrap().source),
        "text\n    code\n"
    );
}

#[test]
fn streaming_identity_pending_exit_eof_and_updates_are_stable() {
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 2).unwrap();
    view.set_defaults(FoldDefaults {
        code: true,
        ..Default::default()
    });
    let update = doc.append("> ```rs\n").unwrap();
    view.apply(&doc, &update).unwrap();
    assert_eq!(update.new_blocks, 0..1);
    let id = FoldId::Code(BlockId(0));
    assert!(view.is_collapsed(id));
    let update = doc.append("> 中🙂\n").unwrap();
    view.apply(&doc, &update).unwrap();
    assert!(update.new_blocks.is_empty());
    assert!(update.changed_blocks.contains(&0));
    let boundary = doc.source().len();
    let update = doc.append("out").unwrap();
    view.apply(&doc, &update).unwrap();
    assert_eq!(doc.blocks()[0].source.end, boundary);
    assert_eq!(
        doc.fold_candidate(id).unwrap().source.end,
        doc.source().len()
    );
    assert!(matches!(
        view.hidden_at(&doc, boundary, &mut tiny()).unwrap(),
        FoldLookup::Hidden(_)
    ));
    let ticket = view.heights().ticket(TileId(0)).unwrap();
    let mut merged = doc.append("side\n> ~~~\n> tail").unwrap();
    merged.merge(doc.finish()).unwrap();
    view.apply(&doc, &merged).unwrap();
    assert!(view.measure(ticket, 100).is_err());
    assert_eq!(doc.blocks()[0].source.end, boundary);
    assert!(!doc.fold_candidate(id).unwrap().open);
    assert_eq!(
        view.hidden_at(&doc, boundary, &mut tiny()).unwrap(),
        FoldLookup::Visible
    );
    assert_eq!(codes(&doc), [BlockId(0), BlockId(2)]);
    assert!(view.is_collapsed(FoldId::Code(BlockId(2))));
    assert_eq!(visible(&doc, doc.blocks()[2].source), "tail");
    let version = view.version();
    view.apply(&doc, &merged).unwrap();
    assert_eq!(view.version(), version);
    let update = doc.finish();
    assert!(update.closed_containers.is_empty());
    assert!(update.changed_blocks.is_empty());
    // A closer without LF is committed by finish, with no new identity.
    let mut doc = Document::new();
    doc.append("> ```\n> x\n> ```").unwrap();
    assert_eq!(doc.blocks()[0].state, SyntaxState::Growing);
    doc.finish();
    assert_eq!(codes(&doc), [BlockId(0)]);
    assert_eq!(visible(&doc, doc.blocks()[0].source), "x\n");
}

#[test]
fn failed_and_overlong_candidates_never_reclassify_old_input() {
    for source in [
        "> ``short\n",
        "> ```bad`info\n",
        &format!("> {}bad`\n", "`".repeat(4096)),
    ] {
        assert!(codes(&parse(source)).is_empty(), "{source:?}");
    }
    // Two columns of raw tab indentation now open a scoped code fence.
    assert_eq!(codes(&parse("> \t```\n")), [BlockId(0)]);
    let prefix = ">".repeat(BLOCK_LOOKAHEAD - 4);
    let doc = parse(&format!("{prefix}```\n{prefix}```\n"));
    assert_eq!(codes(&doc).len(), 1);
    assert_eq!(visible(&doc, doc.blocks()[0].source), "");
    // A closer may confirm beyond the classification window.
    let source = format!("> ```\n> ```{}\n> ```\n", " ".repeat(BLOCK_LOOKAHEAD));
    let doc = parse(&source);
    assert_eq!(codes(&doc), [BlockId(0), BlockId(1)]);
    assert_eq!(visible(&doc, doc.blocks()[0].source), "");
    let prefix = ">".repeat(BLOCK_LOOKAHEAD);
    assert!(codes(&parse(&format!("{prefix}```\n"))).is_empty());
    // A late backtick rejects the raw info candidate and consumes it once.
    let mut doc = Document::new();
    doc.append(&format!("> ```{}", "x".repeat(BLOCK_LOOKAHEAD)))
        .unwrap();
    let before = doc.work();
    doc.append("`late\n> literal\n").unwrap();
    doc.finish();
    assert!(codes(&doc).is_empty());
    assert_eq!(doc.work().inline_bytes, (doc.source().len() - 4) as u64);
    assert!(doc.work().classified_bytes - before.classified_bytes < 100);
    assert!(visible(&doc, Span::new(0, doc.source().len())).ends_with("literal\n"));
}

#[test]
fn deep_million_byte_code_growth_and_close_never_rescan_body_or_ancestors() {
    for opener in ["```rs\n", "     "] {
        let prefix = ">".repeat(1000);
        let mut doc = Document::new();
        let head = if opener.ends_with('\n') {
            format!("{prefix}{opener}{prefix}{}", "x".repeat(50))
        } else {
            format!("{prefix}{opener}{}", "x".repeat(50))
        };
        doc.append(&head).unwrap();
        let mut view = FoldView::new(&doc, 1).unwrap();
        view.set_collapsed(&doc, FoldId::Container(ContainerId(0)), true)
            .unwrap();
        view.set_collapsed(&doc, FoldId::Code(BlockId(0)), true)
            .unwrap();
        let before = doc.work();
        let visits = view.container_index_visits();
        let sections = view.section_index_visits();
        let ranges = view.range_visits();
        for _ in 0..1000 {
            let update = doc.append(&"x".repeat(1000)).unwrap();
            view.apply(&doc, &update).unwrap();
            assert!(update.new_blocks.is_empty());
            assert!(update.closed_containers.is_empty());
        }
        let after = doc.work();
        assert_eq!(after.input_bytes - before.input_bytes, 1_000_000);
        assert_eq!(after.classified_bytes, before.classified_bytes);
        assert_eq!(after.container_bytes, before.container_bytes);
        assert_eq!(after.container_steps, before.container_steps);
        assert_eq!(after.heading_steps, before.heading_steps);
        assert_eq!(after.inline_bytes, before.inline_bytes);
        assert_eq!(view.container_index_visits(), visits);
        assert_eq!(view.section_index_visits(), sections);
        assert!(view.range_visits() - ranges <= 3000);
        assert_eq!(doc.blocks().len(), 1);
        let update = doc.append("\noutside\n").unwrap();
        view.apply(&doc, &update).unwrap();
        assert_eq!(update.closed_containers.len(), 1000);
        assert_eq!(
            doc.blocks()[0].source.end,
            doc.source().find("outside").unwrap()
        );
        assert!(doc.work().container_steps - after.container_steps <= 1002);
        assert!(doc.work().classified_bytes - after.classified_bytes < 100);
        assert!(doc.work().tree_visits - after.tree_visits < 2000);
    }
}

#[test]
fn thousands_of_closed_code_blocks_do_not_add_append_refreshes() {
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 1).unwrap();
    view.set_defaults(FoldDefaults {
        code: true,
        ..Default::default()
    });
    let update = doc.append(&"> ```\n> x\n> ```\n".repeat(4096)).unwrap();
    view.apply(&doc, &update).unwrap();
    assert_eq!(codes(&doc).len(), 4096);
    let defaults = view.default_visits();
    let ranges = view.range_visits();
    let before = doc.work();
    let update = doc.append("> ordinary\n").unwrap();
    view.apply(&doc, &update).unwrap();
    assert!(view.default_visits() - defaults <= 1);
    assert!(view.range_visits() - ranges <= 2);
    assert!(doc.work().container_steps - before.container_steps <= 1);
    assert!(doc.work().classified_bytes - before.classified_bytes < 30);
    assert!(view.is_collapsed(FoldId::Code(BlockId(4095))));
}

#[test]
fn deep_list_fence_matches_indent_once_without_opening_payload_markers() {
    let depth = 250;
    let prefix = " ".repeat(depth * 2);
    let mut doc = Document::new();
    doc.append(&format!("{}```\n", "- ".repeat(depth))).unwrap();
    let before = doc.work();
    doc.append(&format!("{prefix}- [x] > # raw\n{prefix}```\n"))
        .unwrap();
    assert_eq!(doc.container_count(), depth * 2);
    assert_eq!(codes(&doc), [BlockId(0)]);
    assert_eq!(visible(&doc, doc.blocks()[0].source), "- [x] > # raw\n");
    assert!(doc.work().container_bytes - before.container_bytes < 6 * depth as u64);
    assert_eq!(
        doc.work().container_steps - before.container_steps,
        2 * depth as u64
    );
    assert_eq!(doc.blocks()[0].state, SyntaxState::Sealed);
    let update = doc.append("outside\n").unwrap();
    assert_eq!(update.closed_containers.len(), 2 * depth);
}

#[test]
fn seeded_code_ranges_and_display_match_generated_physical_line_model() {
    let mut source = String::new();
    let mut expected = Vec::new();
    let mut seed = 0xb172_a82d_u64;
    for i in 0..400 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let depth = (seed >> 32) as usize % 20 + 1;
        let prefix = ">".repeat(depth);
        let start = source.len();
        let fenced = seed & 1 == 0;
        let mut display = String::new();
        if fenced {
            source.push_str(&format!("{prefix} - ~~~lang\n"));
        } else {
            source.push_str(&format!("{prefix} - item\n"));
        }
        let code_start = if fenced { start } else { source.len() };
        for j in 0..(seed >> 56) as usize % 5 + 1 {
            let text = format!("*v{i}_{j}* $x$ &amp; > - literal\n");
            source.push_str(&format!(
                "{prefix}   {}{text}",
                if fenced { "" } else { "    " }
            ));
            display.push_str(&text);
        }
        if fenced && seed & 2 == 0 {
            source.push_str(&format!("{prefix}   ~~~~\n"));
        }
        expected.push((Span::new(code_start, source.len()), display, depth + 2));
        source.push_str("outside\n");
    }
    let doc = parse(&source);
    let ids = codes(&doc);
    assert_eq!(ids.len(), expected.len());
    for (id, (span, display, depth)) in ids.into_iter().zip(expected) {
        let block = doc.block(id).unwrap();
        assert_eq!(block.source, span);
        assert_eq!(visible(&doc, span), display);
        assert_eq!(
            doc.container(block.container.unwrap()).unwrap().depth,
            depth
        );
        assert_eq!(doc.fold_candidate(FoldId::Code(id)).unwrap().source, span);
    }
    assert!(doc.work().classified_bytes <= 2 * source.len() as u64);
    assert!(doc.work().container_bytes <= 4 * source.len() as u64);
    assert!(doc.work().container_steps <= 2 * source.len() as u64);
}

#[test]
fn summary_discovery_and_code_excerpt_share_budget_and_reject_stale_tasks() {
    let prefix = ">".repeat(900);
    let mut doc = parse(&format!(
        "{prefix}```rust\n{prefix}中🙂 &amp;\n{prefix}```\n"
    ));
    let candidate = doc.fold_candidate(FoldId::Code(BlockId(0))).unwrap();
    let mut zero = Budget::new(WorkLimit::default());
    assert_eq!(candidate.summary_text(&doc, 4, &mut zero), None);
    let mut budget = tiny();
    assert_eq!(candidate.summary_text(&doc, 4, &mut budget), Some("rust"));
    assert_eq!(budget.used().bytes, 4);
    let mut cursor = doc.fold_candidates(candidate.source).unwrap();
    let mut out = Vec::new();
    assert_eq!(
        cursor.advance(&doc, &mut budget, &mut out).unwrap(),
        Completion::BudgetExhausted
    );
    assert!(out.is_empty());
    let mut found = Vec::new();
    loop {
        let mut budget = tiny();
        let result = cursor.advance(&doc, &mut budget, &mut out).unwrap();
        assert!(out.len() <= 1);
        assert_eq!(budget.used().bytes, 0);
        found.extend_from_slice(&out);
        if result == Completion::Covered {
            break;
        }
    }
    assert_eq!(found.len(), 901);
    assert_eq!(found.last().unwrap().id, candidate.id);
    let limits = ExcerptLimits {
        source_bytes: 3000,
        display_bytes: 100,
        lines: 1,
    };
    let mut excerpt = DisplayExcerpt::new(&doc, candidate.source, limits).unwrap();
    let mut text = String::new();
    let mut charged = 0;
    for _ in 0..2000 {
        let mut budget = tiny();
        match excerpt.next(&doc, &mut budget).unwrap() {
            ExcerptStep::Run(run) => text.push_str(run.text),
            ExcerptStep::Complete(end) => {
                assert_eq!(end, ExcerptEnd::LineLimit);
                break;
            }
            ExcerptStep::BudgetExhausted => (),
        }
        charged += budget.used().bytes;
        assert!(budget.used().bytes <= 6);
    }
    assert_eq!(text, "中🙂 &amp;\n");
    assert!(charged >= 1800); // Both hidden physical prefixes are paid.
    let mut short = DisplayExcerpt::new(
        &doc,
        candidate.source,
        ExcerptLimits {
            source_bytes: 900,
            ..limits
        },
    )
    .unwrap();
    for _ in 0..1000 {
        match short.next(&doc, &mut tiny()).unwrap() {
            ExcerptStep::Run(_) => panic!("source cap skipped hidden prefix"),
            ExcerptStep::Complete(end) => {
                assert_eq!(end, ExcerptEnd::SourceLimit);
                break;
            }
            ExcerptStep::BudgetExhausted => (),
        }
    }
    doc.finish(); // Idempotent finish preserves revision.
    assert_eq!(
        excerpt.next(&doc, &mut tiny()).unwrap(),
        ExcerptStep::Complete(ExcerptEnd::LineLimit)
    );
    let mut live = Document::new();
    live.append("> ```\n> x\n").unwrap();
    let mut task = DisplayExcerpt::new(&live, live.blocks()[0].source, limits).unwrap();
    live.append("> ```\n").unwrap();
    assert_eq!(
        task.next(&live, &mut zero),
        Err(LayoutError::StaleContinuation)
    );
}
