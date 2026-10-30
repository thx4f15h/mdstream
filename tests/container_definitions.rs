use mdtui3::*;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn unlimited() -> Budget {
    Budget::new(WorkLimit::unlimited())
}

fn visible(doc: &Document, span: Span) -> String {
    doc.display_runs_in(span)
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect()
}

fn note<'a>(doc: &'a Document, label: &str) -> (FootnoteId, &'a Footnote) {
    doc.footnotes()
        .iter()
        .enumerate()
        .find(|(_, n)| n.label == label)
        .map(|(i, n)| (FootnoteId(i), n))
        .unwrap()
}

#[test]
fn definitions_share_global_namespaces_with_first_definition_and_reference_order() {
    let source = "[before][中 KEY] [^n]\n> - [x] [中   Key]: <a b> 'title'\n>   [^n]: **raw** &amp; $x$ [^opaque]\r\n>   > [nested][中 key] [^n]\n> - [中 KEY]: /ignored\n>   [^n]: duplicate\n> [^unused]: no number\n[after][中 key] [^n] [^N]\n[中 key]: /root-ignored\n[^n]: root duplicate\n[^N]: root case\n";
    let doc = parse(source);
    assert_eq!(
        doc.references()
            .iter()
            .filter(|r| r.destination.is_some())
            .count(),
        1
    );
    assert_eq!(doc.links().len(), 3);
    for i in 0..3 {
        assert_eq!(doc.link_destination(LinkId(i)), Some("a b"));
        assert_eq!(doc.link_title(LinkId(i)), Some("title"));
    }
    let (n, footnote) = note(&doc, "n");
    let definition = footnote.definition.unwrap();
    assert_eq!(footnote.number, Some(1));
    assert_eq!(
        doc.text(definition.content),
        Some("**raw** &amp; $x$ [^opaque]")
    );
    assert_eq!(definition.state, SyntaxState::Sealed);
    assert_eq!(note(&doc, "unused").1.number, None);
    assert_eq!(note(&doc, "N").1.number, Some(2));
    assert_eq!(doc.footnotes().len(), 3);
    assert!(doc.maths().is_empty());
    assert_eq!(doc.footnote_references().len(), 4);
    assert!(doc.footnote_references()[..3].iter().all(|r| r.symbol == n));
    let block = doc.block(definition.block).unwrap();
    assert_eq!(block.container, doc.blocks()[1].container);
    assert_eq!(block.container, Some(ContainerId(2)));
    assert!(matches!(
        doc.container(block.container.unwrap()).unwrap().kind,
        ContainerKind::Item {
            task: Some(TaskMarker { checked: true, .. }),
            ..
        }
    ));
    for b in doc.blocks() {
        if matches!(
            b.kind,
            BlockKind::Definition { .. } | BlockKind::FootnoteDefinition { .. }
        ) {
            assert_eq!(b.state, SyntaxState::Sealed);
            assert!(visible(&doc, b.source).is_empty());
            for at in b.source.range() {
                assert_eq!(doc.block_at(at).map(|id| doc.block(id).unwrap()), Some(b));
            }
        }
    }
    // Definitions in a closed container also serve later root references.
    let doc = parse("> [r]: /first\n> [^f]: first\n\n[r] [^f]\n");
    assert_eq!(doc.link_destination(LinkId(0)), Some("/first"));
    assert_eq!(note(&doc, "f").1.number, Some(1));
    let doc = parse("![image][r]\n> [r]: /image 'caption'\n");
    assert_eq!(doc.link_info(LinkId(0)).unwrap().kind, LinkKind::Image);
    assert_eq!(doc.link_destination(LinkId(0)), Some("/image"));
    assert_eq!(doc.link_title(LinkId(0)), Some("caption"));
}

#[test]
fn definitions_interrupt_tables_but_code_and_other_inline_contexts_remain_opaque() {
    let doc = parse(
        "> h|h\n> -|-\n> a|b\n> [r]: /a|b\n> [^f]: a|b\n> ---\n> ```\n> [code]: /no\n> [^code]: no\n> ```\n>     [indent]: /no\n>     [^indent]: no\n> # [^heading]: inline\n> h|h\n> -|-\n> [cell]: /no|[^cell]: inline\n>\n> `[^span]: raw` $[^math]: raw$\n",
    );
    assert_eq!(doc.tables().len(), 2);
    assert_eq!(doc.tables()[0].rows().len(), 2);
    assert_eq!(doc.tables()[1].rows().len(), 2);
    assert_eq!(
        doc.references()
            .iter()
            .filter(|r| r.destination.is_some())
            .count(),
        1
    );
    assert_eq!(
        doc.footnotes()
            .iter()
            .filter(|r| r.definition.is_some())
            .count(),
        1
    );
    assert_eq!(
        doc.text(note(&doc, "f").1.definition.unwrap().content),
        Some("a|b")
    );
    assert_eq!(doc.headings().entries().len(), 1); // A definition is not a Setext title.
    assert_eq!(
        doc.footnote_references()
            .iter()
            .map(|r| doc.footnote(r.symbol).unwrap().label.as_str())
            .collect::<Vec<_>>(),
        ["heading", "cell"]
    );
    assert!(
        doc.blocks()
            .iter()
            .any(|b| b.kind == BlockKind::Code { info: None })
    );
}

#[test]
fn invalid_labels_and_destinations_keep_container_prefixes_and_leaf_boundaries() {
    for prefix in ["> ", "- ", "> - [x] ", "1. "] {
        for body in [
            "[ ]: /bad",
            "[\t]: /bad",
            "[r]: <unclosed",
            "[r]: /a 'unclosed",
            "[^bad label]: raw",
            "[^]: raw",
            "[open",
            "[^open",
        ] {
            let source = format!("root\n{prefix}{body}\n");
            let doc = parse(&source);
            assert_eq!(doc.blocks()[0].source, Span::new(0, 5));
            assert_eq!(doc.blocks().len(), 2, "{source:?}");
            let b = &doc.blocks()[1];
            assert!(b.container.is_some());
            assert!(!matches!(
                b.kind,
                BlockKind::Definition { .. } | BlockKind::FootnoteDefinition { .. }
            ));
            assert!(doc.references().iter().all(|r| r.destination.is_none()));
            assert!(doc.footnotes().iter().all(|r| r.definition.is_none()));
            let text = visible(&doc, b.source);
            assert!(!text.starts_with(prefix), "{source:?}: {text:?}");
            assert_eq!(doc.text(b.source), Some(&source[5..]));
        }
    }
    let doc = parse("root\n> [ ]: /bad\n> ---\n");
    assert_eq!(doc.headings().entries().len(), 1);
    assert_eq!(
        doc.text(doc.headings().entries()[0].content),
        Some("[ ]: /bad")
    );
    assert_eq!(visible(&doc, doc.blocks()[1].source), "[ ]: /bad\n");
    let doc = parse("> *[ ]: /bad\n> [ ]: /bad*\n");
    assert!(
        doc.runs_in(Span::new(0, doc.source().len()))
            .unwrap()
            .any(|r| r.style.contains(Style::EMPHASIS))
    );
}

#[test]
fn short_pending_definitions_publish_at_newline_or_eof_and_never_take_continuations() {
    let mut doc = Document::new();
    doc.append("[r] [^f]\n> [r]: <url> 'title'").unwrap();
    assert_eq!(doc.link_destination(LinkId(0)), None);
    assert_eq!(doc.container_count(), 0);
    let update = doc.append("\r").unwrap();
    assert!(update.references_changed.is_empty());
    let update = doc.append("\n> [^f]:").unwrap();
    assert_eq!(update.references_changed, [SymbolId(0)]);
    assert_eq!(doc.link_title(LinkId(0)), Some("title"));
    assert!(note(&doc, "f").1.definition.is_none());
    let update = doc.finish();
    assert!(update.footnotes_changed.contains(&note(&doc, "f").0));
    let definition = note(&doc, "f").1.definition.unwrap();
    assert!(definition.content.is_empty());
    assert_eq!(definition.state, SyntaxState::Sealed);
    assert_eq!(
        doc.block(definition.block).unwrap().container,
        Some(ContainerId(0))
    );
    let doc =
        parse("> [^f]: first\n>   more\n- [^f]: duplicate\n  continuation\n[^outside]: root\n");
    assert_eq!(
        doc.text(note(&doc, "f").1.definition.unwrap().content),
        Some("first")
    );
    assert!(visible(&doc, Span::new(0, doc.source().len())).contains("more"));
    assert!(visible(&doc, Span::new(0, doc.source().len())).contains("continuation"));
    assert_eq!(doc.blocks().last().unwrap().container, None);
}

#[test]
fn classification_window_includes_container_prefix_and_never_retries_failed_candidates() {
    for prefix in ["> ".to_owned(), ">".repeat(990)] {
        let mut doc = Document::new();
        let line = format!(
            "{prefix}[r]: /{}\n",
            "a".repeat(BLOCK_LOOKAHEAD - prefix.len() - 7)
        );
        assert_eq!(line.len(), BLOCK_LOOKAHEAD);
        doc.append(&line).unwrap();
        assert!(doc.references()[0].destination.is_some());
        let over = line.replacen("\n", "a\n", 1);
        let doc = parse(&over);
        assert!(doc.references().iter().all(|r| r.destination.is_none()));
    }
    let prefix = ">".repeat(BLOCK_LOOKAHEAD - 4);
    let doc = parse(&format!("{prefix}[^f]: {}\n", "x".repeat(4096)));
    assert!(doc.footnotes().iter().all(|n| n.definition.is_none()));
    for label in ["a".repeat(MAX_LABEL_BYTES + 1), "中".repeat(334)] {
        let doc = parse(&format!("> [{label}]: /url\n> [^{label}]: body\n"));
        assert!(doc.references().iter().all(|r| r.destination.is_none()));
        assert!(doc.footnotes().is_empty());
    }
    let doc = parse(&format!("> [^{}]: body\n", "中".repeat(333)));
    assert!(doc.footnotes()[0].definition.is_some());
    let mut doc = Document::new();
    doc.append(&format!("> [r]: /{}", "x".repeat(1_000_000)))
        .unwrap();
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("x").unwrap();
    }
    assert_eq!(doc.work().inline_bytes - before.inline_bytes, 1000);
    assert_eq!(doc.work().classified_bytes, before.classified_bytes);
    assert_eq!(doc.work().container_steps, before.container_steps);
    assert_eq!(doc.work().label_bytes, before.label_bytes);
    doc.finish();
    assert!(doc.references().iter().all(|r| r.destination.is_none()));
}

#[test]
fn late_container_definitions_do_not_visit_old_link_or_footnote_occurrences() {
    let mut doc = Document::new();
    doc.append(&format!("{}\n\n", "[r] [^f] ".repeat(20_000)))
        .unwrap();
    let links = doc.links().to_vec();
    let notes = doc.footnote_references().to_vec();
    let before = doc.work();
    let update = doc
        .append("> - [r]: /late 'title'\n>   [^f]: late body\n")
        .unwrap();
    assert_eq!(doc.links(), links);
    assert_eq!(doc.footnote_references(), notes);
    assert!(update.new_links.is_empty());
    assert!(update.new_footnote_references.is_empty());
    assert_eq!(update.references_changed, [SymbolId(0)]);
    assert_eq!(doc.work().label_bytes - before.label_bytes, 1);
    assert_eq!(doc.work().footnote_steps - before.footnote_steps, 3);
    assert_eq!(
        doc.work().footnote_label_bytes - before.footnote_label_bytes,
        1
    );
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert!(doc.work().tree_visits - before.tree_visits < 2000);
    assert!(update.changes.len() < 12);
    assert!(update.footnotes_changed.len() <= 2);
    assert_eq!(doc.link_destination(LinkId(19_999)), Some("/late"));
    assert!(matches!(
        doc.resolve_footnote(FootnoteReferenceId(19_999), &mut unlimited())
            .unwrap()
            .unwrap()
            .target,
        FootnoteTarget::Defined(_)
    ));
}

#[test]
fn deep_million_byte_footnote_growth_only_updates_current_symbol_and_shared_endpoint() {
    let prefix = ">".repeat(1000);
    let mut doc = Document::new();
    doc.append(&format!("[^f]\n{prefix}[^f]: {}", "x".repeat(1_000_000)))
        .unwrap();
    let (id, f) = note(&doc, "f");
    let definition = f.definition.unwrap();
    assert_eq!(definition.state, SyntaxState::Growing);
    assert_eq!(doc.container_count(), 1000);
    let mut view = FoldView::new(&doc, 1).unwrap();
    view.set_collapsed(&doc, FoldId::Container(ContainerId(0)), true)
        .unwrap();
    let before = doc.work();
    let visits = view.container_index_visits();
    let sections = view.section_index_visits();
    for _ in 0..1000 {
        let update = doc.append("x").unwrap();
        assert!(update.new_blocks.is_empty());
        assert!(update.new_containers.is_empty());
        assert!(update.closed_containers.is_empty());
        assert_eq!(update.footnotes_changed, [id]);
        view.apply(&doc, &update).unwrap();
    }
    let after = doc.work();
    assert_eq!(after.input_bytes - before.input_bytes, 1000);
    assert_eq!(after.footnote_steps - before.footnote_steps, 1000);
    assert_eq!(after.inline_bytes, before.inline_bytes);
    assert_eq!(after.classified_bytes, before.classified_bytes);
    assert_eq!(after.container_bytes, before.container_bytes);
    assert_eq!(after.container_steps, before.container_steps);
    assert_eq!(after.heading_steps, before.heading_steps);
    assert_eq!(after.footnote_label_bytes, before.footnote_label_bytes);
    assert_eq!(view.container_index_visits(), visits);
    assert_eq!(view.section_index_visits(), sections);
    doc.append("\r").unwrap();
    let update = doc.append("\noutside\n").unwrap();
    assert_eq!(update.closed_containers.len(), 1000);
    assert!(doc.work().container_steps - after.container_steps <= 1002);
    let f = doc.footnote(id).unwrap().definition.unwrap();
    assert_eq!(f.content.len(), 1_001_000);
    assert_eq!(f.block, definition.block);
    assert_eq!(f.state, SyntaxState::Sealed);
    let before = doc.work();
    doc.append(&format!("> [^f]: {}", "y".repeat(1_000_000)))
        .unwrap();
    let revision = doc.footnote(id).unwrap().revision;
    let update = doc.append("z\n").unwrap();
    assert!(update.footnotes_changed.is_empty());
    assert_eq!(doc.footnote(id).unwrap().revision, revision);
    assert_eq!(doc.footnote(id).unwrap().definition, Some(f));
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
}

#[test]
fn nested_list_definition_prefixes_are_linear_and_symbol_history_adds_no_refresh() {
    let depth = 250;
    let mut doc = Document::new();
    doc.append(&format!("{}[r]: /url\n", "- ".repeat(depth)))
        .unwrap();
    let before = doc.work();
    doc.append(&format!("{}[^f]: body\n", " ".repeat(2 * depth)))
        .unwrap();
    assert_eq!(doc.container_count(), 2 * depth);
    assert_eq!(doc.blocks()[0].container, doc.blocks()[1].container);
    assert_eq!(
        doc.work().container_steps - before.container_steps,
        depth as u64
    );
    assert!(doc.work().container_bytes - before.container_bytes < 4 * depth as u64);
    doc.append("outside\n").unwrap();
    doc.append(
        &(0..4096)
            .map(|i| format!("> [r{i}]: /url\n> [^f{i}]: body\n"))
            .collect::<String>(),
    )
    .unwrap();
    let before = doc.work();
    let update = doc
        .append("> [r]: /duplicate\n> [^f]: duplicate\n")
        .unwrap();
    assert!(update.references_changed.is_empty());
    assert!(update.footnotes_changed.is_empty());
    assert_eq!(doc.work().label_bytes - before.label_bytes, 1);
    assert_eq!(doc.work().footnote_steps - before.footnote_steps, 3);
    assert_eq!(doc.work().container_steps - before.container_steps, 2);
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert!(doc.work().classified_bytes - before.classified_bytes < 100);
}

#[test]
fn container_body_reader_and_resolution_share_atomic_utf8_budget() {
    let doc = parse("[^f]\n> - [^f]: 中🙂raw\n");
    let mut budget = Budget::new(WorkLimit {
        queries: 2,
        objects: 2,
        dependencies: 1,
        bytes: 3,
        ..Default::default()
    });
    let mut objects = doc
        .footnote_references_in(Span::new(0, doc.source().len()))
        .unwrap();
    let (reference, _) = objects.advance(&mut budget).unwrap().unwrap();
    let resolution = doc
        .resolve_footnote(reference, &mut budget)
        .unwrap()
        .unwrap();
    let mut reader = doc.footnote_content(resolution.symbol).unwrap();
    let start = reader.source_position();
    assert_eq!(
        reader.advance(&mut budget),
        Err(Completion::BudgetExhausted)
    );
    assert_eq!(reader.source_position(), start);
    let mut text = String::new();
    for bytes in [3, 3, 4, 4] {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes,
            ..Default::default()
        });
        match reader.advance(&mut budget) {
            Ok(Some(chunk)) => {
                assert_eq!(doc.text(chunk.source), Some(chunk.text));
                text.push_str(chunk.text);
            }
            Err(Completion::BudgetExhausted) => assert_eq!(budget.used(), WorkLimit::default()),
            result => panic!("unexpected {result:?}"),
        }
    }
    assert_eq!(text, "中🙂raw");
    assert_eq!(
        reader.advance(&mut Budget::new(WorkLimit::default())),
        Ok(None)
    );
    assert_eq!(
        doc.block(note(&doc, "f").1.definition.unwrap().block)
            .unwrap()
            .container,
        Some(ContainerId(2))
    );
}

#[test]
fn late_definitions_invalidate_dependencies_and_layout_without_changing_fold_state() {
    let mut doc = Document::new();
    doc.append(&format!(
        "# root\n[r] [^f] {}\n\n[y] [^g] {}\n\n",
        "x".repeat(500),
        "y".repeat(500)
    ))
    .unwrap();
    let key = DependencyKey::capture(&doc, TileId(0), &mut unlimited())
        .unwrap()
        .unwrap();
    let other_tile = doc
        .tile_at(doc.footnote_references()[1].source.start)
        .unwrap();
    let other = DependencyKey::capture(&doc, other_tile, &mut unlimited())
        .unwrap()
        .unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    view.set_collapsed(&doc, FoldId::Heading(HeadingId(0)), true)
        .unwrap();
    let ticket = view.heights().ticket(TileId(0)).unwrap();
    let update = doc.append("> [r]: /url\n> [^f]: body\n").unwrap();
    view.apply(&doc, &update).unwrap();
    assert_eq!(
        key.validate(&doc, &mut unlimited()).unwrap(),
        CacheValidity::Changed
    );
    assert_eq!(
        other.validate(&doc, &mut unlimited()).unwrap(),
        CacheValidity::Valid
    );
    assert!(view.measure(ticket, 10).is_err());
    assert!(view.is_collapsed(FoldId::Heading(HeadingId(0))));
    view.set_collapsed(&doc, FoldId::Container(ContainerId(0)), true)
        .unwrap();
    view.set_collapsed(&doc, FoldId::Heading(HeadingId(0)), false)
        .unwrap();
    assert!(view.is_collapsed(FoldId::Container(ContainerId(0))));
    assert!(matches!(
        view.hidden_at(
            &doc,
            note(&doc, "f").1.definition.unwrap().content.start,
            &mut unlimited()
        )
        .unwrap(),
        FoldLookup::Hidden(_)
    ));
    assert_eq!(doc.link_destination(LinkId(0)), Some("/url"));
}

#[test]
fn updates_reconstruct_streamed_symbols_and_blocks_and_merge_preserves_fold_results() {
    let mut doc = Document::new();
    let mut immediate = FoldView::new(&doc, 1).unwrap();
    let mut batched = FoldView::new(&doc, 1).unwrap();
    let defaults = FoldDefaults {
        container_depth: Some(1),
        ..Default::default()
    };
    immediate.set_defaults(defaults);
    batched.set_defaults(defaults);
    let mut symbols = Vec::new();
    let mut occurrences = Vec::new();
    let mut blocks = Vec::new();
    let mut merged = Update::default();
    let mut scratch = Update::default();
    for chunk in [
        "[r] [^a] [^b]\n",
        "> - [r]: /url\n>   [^a]: ",
        &"x".repeat(2000),
        "中\r",
        "\n> - [^a]: duplicate\n>   [^b]: body",
        "",
    ] {
        if chunk.is_empty() {
            doc.finish_into(&mut scratch);
        } else {
            doc.append_into(chunk, &mut scratch).unwrap();
        }
        symbols.extend_from_slice(&doc.footnotes()[scratch.new_footnotes.clone()]);
        occurrences
            .extend_from_slice(&doc.footnote_references()[scratch.new_footnote_references.clone()]);
        blocks.extend_from_slice(&doc.blocks()[scratch.new_blocks.clone()]);
        for &id in &scratch.footnotes_changed {
            symbols[id.0] = doc.footnote(id).unwrap().clone();
        }
        for i in scratch.changed_blocks.clone() {
            blocks[i] = doc.blocks()[i].clone();
        }
        assert_eq!(symbols, doc.footnotes());
        assert_eq!(occurrences, doc.footnote_references());
        assert_eq!(blocks, doc.blocks());
        immediate.apply(&doc, &scratch).unwrap();
        merged.merge(scratch.clone()).unwrap();
    }
    batched.apply(&doc, &merged).unwrap();
    assert_eq!(merged.references_changed, [SymbolId(0)]);
    assert_eq!(merged.new_footnotes, 0..2);
    assert_eq!(merged.new_footnote_references, 0..2);
    assert_eq!(
        batched.heights().total_rows(),
        immediate.heights().total_rows()
    );
    for at in (0..doc.source().len()).filter(|&at| doc.source().is_char_boundary(at)) {
        assert_eq!(
            batched.hidden_at(&doc, at, &mut unlimited()).unwrap(),
            immediate.hidden_at(&doc, at, &mut unlimited()).unwrap()
        );
    }
    assert!(doc.finish().footnotes_changed.is_empty());
}

#[test]
fn generated_definitions_match_independent_first_definition_and_physical_range_model() {
    use std::collections::BTreeMap;
    let mut source = String::new();
    let mut first_links = BTreeMap::new();
    let mut first_notes = BTreeMap::new();
    let mut definitions = Vec::new();
    let mut seed = 0xdec1_a7ed_u64;
    for i in 0..400 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let depth = (seed >> 32) as usize % 20 + 1;
        let label = format!("n{}", (seed >> 48) % 37);
        let prefix = ">".repeat(depth);
        source.push_str(&format!("{prefix} - [{label}] [^{label}]\n"));
        let start = source.len();
        source.push_str(&format!("{prefix}   [{label}]: <url {i}> 'title{i}'\r\n"));
        first_links
            .entry(label.clone())
            .or_insert((format!("url {i}"), format!("title{i}")));
        definitions.push((Span::new(start, source.len()), depth + 2));
        let start = source.len();
        source.push_str(&format!("{prefix}   [^{label}]: "));
        let content_start = source.len();
        let body = format!("raw 中🙂 {i} **a** $x$ [^opaque]");
        source.push_str(&body);
        let content = Span::new(content_start, source.len());
        source.push_str("\r\n");
        first_notes.entry(label).or_insert(content);
        definitions.push((Span::new(start, source.len()), depth + 2));
        source.push_str("outside\n");
    }
    let doc = parse(&source);
    let blocks: Vec<_> = doc
        .blocks()
        .iter()
        .filter(|b| {
            matches!(
                b.kind,
                BlockKind::Definition { .. } | BlockKind::FootnoteDefinition { .. }
            )
        })
        .collect();
    assert_eq!(blocks.len(), definitions.len());
    for (b, (span, depth)) in blocks.into_iter().zip(definitions) {
        assert_eq!(b.source, span);
        assert_eq!(doc.container(b.container.unwrap()).unwrap().depth, depth);
        assert_eq!(b.state, SyntaxState::Sealed);
        assert!(visible(&doc, span).is_empty());
    }
    assert_eq!(doc.links().len(), 400);
    assert_eq!(doc.footnote_references().len(), 400);
    for r in doc.references() {
        let (url, title) = &first_links[&r.label];
        assert_eq!(doc.text(r.destination.unwrap()), Some(url.as_str()));
        assert_eq!(doc.text(r.title.unwrap()), Some(title.as_str()));
    }
    for n in doc.footnotes() {
        assert_eq!(n.definition.unwrap().content, first_notes[&n.label]);
    }
    assert!(doc.work().classified_bytes <= 2 * source.len() as u64);
    assert!(doc.work().container_bytes <= 4 * source.len() as u64);
    assert!(doc.work().container_steps <= 2 * source.len() as u64);
    assert!(doc.maths().is_empty());
}

#[test]
fn hidden_million_byte_definition_layout_skips_body_with_zero_byte_budget() {
    struct Empty;
    impl LayoutSink for Empty {
        fn begin_tile(&mut self, _: TileId) {}
        fn text(&mut self, _: RenderRun<'_>) {
            panic!("hidden definition reached layout");
        }
        fn object(&mut self, _: LinkId, _: &LinkInfo) {
            panic!("raw body created an object");
        }
        fn end_tile(&mut self, _: TileId) -> u64 {
            0
        }
    }
    let doc = parse(&format!("> - [^f]: {}\n", "![raw](url) ".repeat(100_000)));
    assert_eq!(doc.next_drawable(0), None);
    let mut heights = HeightIndex::default();
    heights.rebuild(&doc).unwrap();
    let mut pass = LayoutPass::new(&doc, &heights, TileId(0), 10, false).unwrap();
    let mut budget = Budget::new(WorkLimit {
        queries: 4,
        tiles: 1,
        objects: 1,
        ..Default::default()
    });
    assert_eq!(
        pass.advance(&doc, &mut heights, &mut budget, &mut Empty)
            .unwrap(),
        Completion::EndOfDocument
    );
    assert_eq!(budget.used().bytes, 0);
    assert_eq!(heights.total_rows(), 0);
}
