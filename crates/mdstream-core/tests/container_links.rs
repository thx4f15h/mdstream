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

fn label(doc: &Document, id: usize) -> String {
    visible(doc, doc.link_info(LinkId(id)).unwrap().label)
}

#[test]
fn direct_labels_preserve_source_envelopes_and_physical_leaf_identity() {
    for (first, next) in [
        ("> ", "> "),
        ("- [x] ", "  "),
        ("> - ", ">   "),
        ("12) ", "    "),
    ] {
        for marker in ["[", "!["] {
            let mut doc = Document::new();
            doc.append(&format!("{first}{marker}**中\r\n")).unwrap();
            let block = doc.blocks()[0].clone();
            doc.append(&format!("{next}🙂** &amp; `a\r\n{next}b`](<a b> '说明')\n"))
                .unwrap();
            doc.finish();
            assert_eq!(doc.links().len(), 1);
            assert_eq!(doc.blocks()[0], block);
            assert_eq!(doc.blocks().len(), 3);
            assert_eq!(label(&doc, 0), "中\n🙂 & a b");
            assert_eq!(doc.link_destination(LinkId(0)), Some("a b"));
            assert_eq!(doc.link_title(LinkId(0)), Some("说明"));
            let info = doc.link_info(LinkId(0)).unwrap();
            assert_eq!(
                info.kind,
                if marker == "[" {
                    LinkKind::Link
                } else {
                    LinkKind::Image
                }
            );
            assert_eq!(info.state, SyntaxState::Sealed);
            assert_eq!(info.source.start, first.len());
            assert_eq!(info.source.end, doc.source().len() - 1);
            assert!(doc.text(info.label).unwrap().contains(next));
            assert!(
                doc.display_runs_in(info.label)
                    .unwrap()
                    .all(|r| r.link == Some(LinkId(0)))
            );
            assert_eq!(doc.links_in(doc.blocks()[1].source).count(), 1);
        }
    }
}

#[test]
fn references_normalize_payload_only_and_reuse_collapsed_symbols() {
    for suffix in ["", "[]", "[key]"] {
        let mut doc = Document::new();
        doc.append(&format!("> ![Ä  中\r\n> 🙂]{suffix}\n"))
            .unwrap();
        assert_eq!(doc.links().len(), 1);
        let id = LinkId(0);
        let info = *doc.link_info(id).unwrap();
        assert_eq!(label(&doc, 0), "Ä  中\n🙂");
        let name = if suffix == "[key]" {
            "key"
        } else {
            "ä 中 🙂"
        };
        let LinkTarget::Reference(symbol) = doc.links()[0] else {
            panic!()
        };
        assert_eq!(doc.references()[symbol.0].label, name);
        assert_eq!(doc.link_destination(id), None);
        let update = doc.append(&format!("> [{name}]: /late 'title'\n")).unwrap();
        assert!(update.new_links.is_empty());
        assert_eq!(*doc.link_info(id).unwrap(), info);
        assert_eq!(doc.link_destination(id), Some("/late"));
        assert_eq!(doc.link_title(id), Some("title"));
        doc.finish();
    }
    let doc = parse("> [a&amp;\n> b][]\n> [a&amp; b]: /raw\n");
    assert_eq!(label(&doc, 0), "a&\nb");
    assert_eq!(doc.link_destination(LinkId(0)), Some("/raw"));
}

#[test]
fn reference_limit_counts_payload_not_prefixes_and_direct_alt_is_unbounded() {
    let prefix = ">".repeat(1000);
    for bytes in [998, 999, 1000, 1001, 4096] {
        let payload = format!("a\n{}", "x".repeat(bytes - 2));
        for suffix in ["", "[]", "(url)", "[r]"] {
            let doc = parse(&format!(
                "{prefix}![a\n{prefix}{}]{suffix}\n",
                "x".repeat(bytes - 2)
            ));
            let published = bytes <= 999 || matches!(suffix, "(url)" | "[r]");
            assert_eq!(
                doc.links().len(),
                usize::from(published),
                "{bytes} {suffix}"
            );
            if published {
                assert_eq!(label(&doc, 0), payload);
            }
            assert!(doc.work().label_bytes <= 1000);
        }
    }
}

#[test]
fn lazy_and_equivalent_prefixes_continue_but_scope_and_blocks_end_labels() {
    for source in [
        "> > [a\n>> b\nc](u)\n",
        "> - ![a\n> b\nc](u)\n",
        "- [a\n  b\nc](u)\n",
    ] {
        let doc = parse(source);
        assert_eq!(doc.links().len(), 1, "{source:?}");
        assert_eq!(label(&doc, 0), "a\nb\nc");
        assert!(
            doc.blocks()
                .iter()
                .all(|b| b.container == doc.blocks()[0].container)
        );
    }
    for middle in [
        ">\n",
        "\n",
        "> > nested\n",
        "> # heading\n",
        "> ```\n> raw\n> ```\n",
        ">     code\n",
        "> [r]: /url\n",
        "> [^n]: body\n",
        "> h|h\n> -|-\n>\n",
        "> ---\n",
    ] {
        let doc = parse(&format!("> [open\n{middle}> end](u)\n"));
        assert!(doc.links().is_empty(), "{middle:?}");
    }
    for source in ["- [open\n- end](u)\n", "> > [open\n> # h\n> end](u)\n"] {
        assert!(parse(source).links().is_empty());
    }
}

#[test]
fn failed_candidates_never_replay_and_suffixes_keep_single_line_rules() {
    let doc = parse("> [*open\n> end*\n");
    assert!(doc.links().is_empty());
    assert_eq!(
        visible(&doc, Span::new(0, doc.source().len())),
        "[open\nend\n"
    );
    for suffix in [
        "(unclosed\n> target)",
        "[ref\n> label]",
        "(url 'title\n> rest')",
    ] {
        let doc = parse(&format!("> [a\n> b]{suffix}\n"));
        assert_eq!(doc.links().len(), 1);
        assert_eq!(doc.link_destination(LinkId(0)), None);
        assert_eq!(label(&doc, 0), "a\nb");
        assert_eq!(doc.link_info(LinkId(0)).unwrap().state, SyntaxState::Sealed);
        assert!(doc.work().inline_bytes <= doc.source().len() as u64);
    }
    for source in [
        "> !\n> [alt](u)\n",
        "> [old\n> [new](u)\n",
        "> [old\n> <https://a>\n",
    ] {
        let doc = parse(source);
        assert_eq!(doc.links().len(), 1);
        assert_eq!(doc.link_info(LinkId(0)).unwrap().kind, LinkKind::Link);
    }
    let doc = parse("> [a\\]\n> `]` b](u)\n");
    assert_eq!(label(&doc, 0), "a]\n] b");
}

#[test]
fn table_confirmation_discards_old_labels_and_failures_resume_them_once() {
    for padding in [0, 1100] {
        let body = "x".repeat(padding);
        let prior = "> ![old\n";
        let header = format!("> {body}a|b\n");
        let doc = parse(&format!("{prior}{header}> -|-\n"));
        assert_eq!(doc.tables().len(), 1);
        assert!(doc.links().is_empty());
        assert_eq!(visible(&doc, doc.blocks()[0].source), "![old\n");
        for tail in ["> end](u)\n", "> --|--x\n> end](u)\n", "> ---\n"] {
            let doc = parse(&format!("{prior}{header}{tail}"));
            assert!(doc.tables().is_empty());
            if tail.ends_with("](u)\n") {
                assert_eq!(doc.links().len(), 1);
                assert!(label(&doc, 0).starts_with(&format!("old\n{body}a|b\n")));
            } else {
                assert!(doc.links().is_empty());
            }
            assert!(doc.work().inline_bytes <= doc.source().len() as u64);
        }
        let doc = parse(&format!("{prior}> {body}end](u)|b\n> -|-\n"));
        assert_eq!(doc.tables().len(), 1);
        assert!(doc.links().is_empty());
    }
}

#[test]
fn scalar_and_tile_display_mapping_matches_budgeted_label_reading() {
    let doc = parse(&format!(
        "> ![{}&amp;\r\n> `中\r\n> 🙂` b](u)\n",
        "a".repeat(510)
    ));
    let info = doc.link_info(LinkId(0)).unwrap();
    let expected = format!("{}&\n中 🙂 b", "a".repeat(510));
    let mut scalars = String::new();
    for (at, c) in doc.text(info.label).unwrap().char_indices() {
        scalars.push_str(&visible(
            &doc,
            Span::new(info.label.start + at, info.label.start + at + c.len_utf8()),
        ));
    }
    assert_eq!(scalars, expected);
    let tiles: String = doc
        .tiles_overlapping(info.label)
        .map(|i| {
            let tile = doc.tile(TileId(i)).unwrap();
            visible(
                &doc,
                Span::new(
                    tile.start.max(info.label.start),
                    tile.end.min(info.label.end),
                ),
            )
        })
        .collect();
    assert_eq!(tiles, expected);
    let mut cursor = doc.display_cursor(info.label).unwrap();
    let mut text = String::new();
    for _ in 0..5000 {
        let position = cursor.source_position();
        let mut zero = Budget::new(WorkLimit::default());
        assert_eq!(
            cursor.next(&doc, &mut zero).unwrap(),
            DisplayStep::BudgetExhausted
        );
        assert_eq!(cursor.source_position(), position);
        assert_eq!(zero.used(), WorkLimit::default());
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 6,
            ..Default::default()
        });
        let DisplayStep::Run(run) = cursor.next(&doc, &mut budget).unwrap() else {
            panic!()
        };
        assert!(run.source.start >= info.label.start && run.source.end <= info.label.end);
        if !run.style.contains(Style::HIDDEN) {
            text.push_str(run.text);
        }
        assert!(cursor.source_position() > position);
        assert!(budget.used().bytes <= 6);
        if cursor.source_position() == info.label.end {
            break;
        }
    }
    assert_eq!(text, expected);
    assert_eq!(
        cursor
            .next(&doc, &mut Budget::new(WorkLimit::default()))
            .unwrap(),
        DisplayStep::Complete
    );
}

#[test]
fn display_search_skips_prefixes_while_source_search_keeps_exact_bytes() {
    let doc = parse("> - ![中&amp;\r\n>   🙂](hidden)\n");
    let mut task = DisplaySearch::new(&doc, "中&\n🙂").unwrap();
    let mut hits = Vec::new();
    for _ in 0..1000 {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 6,
            ..Default::default()
        });
        if task.advance(&doc, &mut budget, &mut hits).unwrap() == Completion::EndOfDocument {
            break;
        }
    }
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].source, doc.link_info(LinkId(0)).unwrap().label);
    let mut source = SourceSearch::new(&doc, "\r\n>   ").unwrap();
    let mut raw = Vec::new();
    source
        .advance(&doc, &mut Budget::new(WorkLimit::unlimited()), &mut raw)
        .unwrap();
    assert_eq!(raw.len(), 1);
    assert_eq!(doc.text(raw[0]), Some("\r\n>   "));
    for hidden in [">", "hidden", "amp;"] {
        let mut task = DisplaySearch::new(&doc, hidden).unwrap();
        let mut hits = Vec::new();
        task.advance(&doc, &mut Budget::new(WorkLimit::unlimited()), &mut hits)
            .unwrap();
        assert!(hits.is_empty());
    }
}

#[test]
fn late_close_invalidates_readers_measurements_and_caches_but_keeps_folds() {
    let mut doc = Document::new();
    let mut merged = doc.append(&format!("> ![{}\n", "x".repeat(2000))).unwrap();
    let old = doc.blocks()[0].clone();
    let mut heights = HeightIndex::default();
    let mut checkpoints = CheckpointIndex::<usize>::new();
    apply_layout_update(&doc, &merged, &mut heights, &mut checkpoints).unwrap();
    checkpoints.insert(TileId(1), 42).unwrap();
    let ticket = heights.ticket(TileId(0)).unwrap();
    let mut cursor = doc.display_cursor(old.source).unwrap();
    let key = DependencyKey::capture(&doc, TileId(0), &mut Budget::new(WorkLimit::unlimited()))
        .unwrap()
        .unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let fold = FoldId::Container(ContainerId(0));
    view.set_collapsed(&doc, fold, true).unwrap();
    let update = doc.append("> tail](url)\n").unwrap();
    assert_eq!(update.new_links, 0..1);
    assert_eq!(doc.blocks()[0], old);
    apply_layout_update(&doc, &update, &mut heights, &mut checkpoints).unwrap();
    view.apply(&doc, &update).unwrap();
    merged.merge(update).unwrap();
    assert!(heights.measure(ticket, 99).is_err());
    assert!(checkpoints.get(TileId(1)).is_none());
    assert_eq!(
        key.validate(&doc, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap(),
        CacheValidity::Changed
    );
    assert_eq!(
        cursor
            .next(&doc, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap_err(),
        LayoutError::StaleContinuation
    );
    assert!(view.is_collapsed(fold));
    let end = doc.finish();
    heights.apply(&doc, &end).unwrap();
    view.apply(&doc, &end).unwrap();
    merged.merge(end).unwrap();
    let mut rebuilt = HeightIndex::default();
    rebuilt.apply(&doc, &merged).unwrap();
    assert_eq!(rebuilt.total_rows(), heights.total_rows());
    let mut nav = FoldNavigation::new(
        &doc,
        &view,
        &Navigator::new(),
        NavAction::Source(doc.link_info(LinkId(0)).unwrap().label.end - 1),
        FoldNavigationPolicy::Reveal,
        1,
    )
    .unwrap();
    let mut done = false;
    for _ in 0..20 {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        let progress = nav.advance(&doc, &mut view, &mut budget).unwrap();
        assert_eq!(budget.used().bytes, 0);
        if progress.completion == Completion::Covered {
            done = true;
            break;
        }
    }
    assert!(done);
    assert!(!view.is_collapsed(fold));
}

#[test]
fn provisional_reference_becomes_direct_with_the_same_id_and_change_event() {
    let prefix = ">".repeat(1000);
    let mut doc = Document::new();
    let mut batch = doc
        .append(&format!(
            "{prefix}![a\n{prefix}b](url '_{}",
            "t".repeat(1100)
        ))
        .unwrap();
    assert_eq!(doc.links().len(), 1);
    let before = *doc.link_info(LinkId(0)).unwrap();
    assert_eq!(before.state, SyntaxState::Provisional);
    assert_eq!(doc.link_destination(LinkId(0)), None);
    let update = doc.append("')\n").unwrap();
    assert!(update.new_links.is_empty());
    assert_eq!(update.changed_links, 0..1);
    batch.merge(update).unwrap();
    assert_eq!(batch.new_links, 0..1);
    assert_eq!(doc.link_info(LinkId(0)).unwrap().label, before.label);
    assert_eq!(doc.link_info(LinkId(0)).unwrap().state, SyntaxState::Sealed);
    assert_eq!(doc.link_destination(LinkId(0)), Some("url"));
    assert_eq!(doc.link_title(LinkId(0)).unwrap().len(), 1101);
    assert_eq!(label(&doc, 0), "a\nb");
}

#[test]
fn deep_million_byte_labels_only_consume_new_payload_and_close_by_range() {
    for (first, next) in [
        (">".repeat(1000), ">".repeat(1000)),
        ("- ".repeat(250), " ".repeat(500)),
    ] {
        let mut doc = Document::new();
        doc.append(&format!("{first}![{}", "x".repeat(1_000_000)))
            .unwrap();
        let before = doc.work();
        let arena = doc.annotation_arena_stats();
        for _ in 0..1000 {
            doc.append("x").unwrap();
        }
        let after = doc.work();
        assert_eq!(after.inline_bytes - before.inline_bytes, 1000);
        assert_eq!(after.label_bytes, before.label_bytes);
        assert_eq!(after.classified_bytes, before.classified_bytes);
        assert_eq!(after.container_steps, before.container_steps);
        assert_eq!(
            doc.annotation_arena_stats().allocation_calls,
            arena.allocation_calls
        );
        let before = doc.work();
        doc.append(&format!("\n{next}end](u)\n")).unwrap();
        let after = doc.work();
        assert_eq!(after.inline_bytes - before.inline_bytes, 9);
        assert_eq!(after.label_bytes, before.label_bytes);
        assert!(after.tree_visits - before.tree_visits < 2000);
        assert!(after.container_steps - before.container_steps <= 1000);
        assert_eq!(doc.links().len(), 1);
        assert_eq!(doc.link_destination(LinkId(0)), Some("u"));
        assert_eq!(
            doc.link_info(LinkId(0)).unwrap().label.len(),
            1_001_004 + next.len()
        );
        doc.finish();
        assert!(doc.work().inline_bytes <= doc.source().len() as u64);
        struct PlaceholderOnly;
        impl LayoutSink for PlaceholderOnly {
            fn begin_tile(&mut self, _: TileId) {}
            fn text(&mut self, _: RenderRun<'_>) {
                panic!("hidden label read")
            }
            fn object(&mut self, _: LinkId, _: &LinkInfo) {
                panic!("hidden image read")
            }
            fn end_tile(&mut self, _: TileId) -> u64 {
                1
            }
        }
        impl FoldLayoutSink for PlaceholderOnly {
            fn placeholder(&mut self, _: FoldPlaceholder) {}
        }
        let mut view = FoldView::new(&doc, 1).unwrap();
        view.set_collapsed(&doc, FoldId::Container(ContainerId(0)), true)
            .unwrap();
        let mut pass = LayoutPass::folded(&doc, &view, TileId(0), 10).unwrap();
        let mut budget = Budget::new(WorkLimit {
            bytes: 0,
            ..WorkLimit::unlimited()
        });
        assert_eq!(
            pass.advance_folded(&doc, &mut view, &mut budget, &mut PlaceholderOnly)
                .unwrap(),
            Completion::EndOfDocument
        );
        assert_eq!(budget.used().bytes, 0);
    }
}

#[test]
fn many_lazy_lines_and_late_definitions_do_not_scan_ancestors_or_old_references() {
    let mut doc = Document::new();
    doc.append(&format!("{}[a\n", ">".repeat(1000))).unwrap();
    let before = doc.work();
    for _ in 0..4096 {
        doc.append("x\n").unwrap();
    }
    assert_eq!(doc.work().inline_bytes - before.inline_bytes, 8192);
    assert_eq!(doc.work().container_steps - before.container_steps, 4096);
    doc.append("end](u)\n").unwrap();
    assert_eq!(doc.links().len(), 1);
    let mut doc = Document::new();
    for _ in 0..20_000 {
        doc.append("> [a\n> b][]\n").unwrap();
    }
    let before = doc.work();
    let update = doc.append("> [a b]: /late\n").unwrap();
    assert!(update.new_links.is_empty());
    assert!(doc.work().label_bytes - before.label_bytes <= 3);
    assert_eq!(doc.links().len(), 20_000);
    assert_eq!(doc.link_destination(LinkId(0)), Some("/late"));
    assert_eq!(doc.link_destination(LinkId(19_999)), Some("/late"));
    assert!(doc.work().tree_visits - before.tree_visits < 1000);
}

#[test]
fn generated_labels_match_a_prefix_free_parser_and_source_mapping_model() {
    for i in 0..400 {
        let depth = i % 16 + 1;
        let prefix = ">".repeat(depth);
        let marker = if i % 2 == 0 { "![" } else { "[" };
        let suffix = ["(u)", "[]", "[r]", "(bad target)"][i % 4];
        let source = format!("{prefix}{marker}中{i} &amp;\r\n{prefix}*tail*]{suffix}\n");
        let plain = parse(&format!("{marker}中{i} &amp;\r\n*tail*]{suffix}\n"));
        let doc = parse(&source);
        assert_eq!(label(&doc, 0), label(&plain, 0));
        assert_eq!(
            doc.link_destination(LinkId(0)),
            plain.link_destination(LinkId(0))
        );
        assert_eq!(
            doc.references(),
            plain
                .references()
                .iter()
                .map(|r| {
                    let mut r = r.clone();
                    r.first_use = r.first_use.map(|p| p + depth);
                    r
                })
                .collect::<Vec<_>>()
        );
        let info = doc.link_info(LinkId(0)).unwrap();
        let old = plain.link_info(LinkId(0)).unwrap();
        assert_eq!(
            info.label,
            Span::new(old.label.start + depth, old.label.end + 2 * depth)
        );
        assert_eq!(
            info.source,
            Span::new(old.source.start + depth, old.source.end + 2 * depth)
        );
        assert_eq!(info.kind, old.kind);
        assert!(doc.work().inline_bytes <= source.len() as u64);
    }
}
