use mdstream::*;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn unlimited() -> Budget {
    Budget::new(WorkLimit::unlimited())
}

fn tiny() -> Budget {
    Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 4,
        ..Default::default()
    })
}

fn definition(doc: &Document, id: usize) -> FootnoteDefinition {
    doc.footnote(FootnoteId(id)).unwrap().definition.unwrap()
}

fn read(doc: &Document, id: usize, mapped: bool) -> String {
    let mut cursor = if mapped {
        doc.footnote_payload(FootnoteId(id))
    } else {
        doc.footnote_content(FootnoteId(id))
    }
    .unwrap();
    let range = definition(doc, id).content;
    let mut position = range.start;
    let mut charged = 0;
    let mut text = String::new();
    loop {
        let mut budget = tiny();
        let Some(chunk) = cursor.advance(&mut budget).unwrap() else {
            break;
        };
        assert_eq!(chunk.source.start, position);
        assert!(!chunk.source.is_empty() && chunk.source.len() <= 4);
        assert_eq!(budget.used().bytes, chunk.source.len());
        assert_eq!(budget.used().queries, 1);
        assert_eq!(budget.used().objects, 1);
        if !chunk.text.is_empty() {
            assert_eq!(doc.text(chunk.source), Some(chunk.text));
        }
        text.push_str(chunk.text);
        position = cursor.source_position();
        assert_eq!(position, chunk.source.end);
        charged += budget.used().bytes;
    }
    assert_eq!(position, range.end);
    assert_eq!(charged, range.len());
    assert_eq!(
        cursor.advance(&mut Budget::new(WorkLimit::default())),
        Ok(None)
    );
    text
}

fn visible(doc: &Document) -> String {
    doc.display_runs_in(Span::new(0, doc.source().len()))
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect()
}

#[test]
fn multiline_definition_keeps_one_block_and_raw_envelope() {
    let source = "[^n]\n\n[^n]: first\r\n    second\r\n\r\n    third\r\nafter\n";
    let doc = parse(source);
    let definition = doc.footnote(FootnoteId(0)).unwrap().definition.unwrap();
    assert_eq!(
        doc.text(definition.content),
        Some("first\r\n    second\r\n\r\n    third")
    );
    assert_eq!(definition.state, SyntaxState::Sealed);
    let block = doc.block(definition.block).unwrap();
    assert_eq!(block.source.end, source.find("after").unwrap());
    assert_eq!(
        doc.block_at(source.find("third").unwrap()),
        Some(definition.block)
    );
    assert_eq!(doc.footnote_references().len(), 1);
    assert_eq!(read(&doc, 0, true), "first\r\nsecond\r\n\r\nthird");
    assert_eq!(visible(&doc), "[^n]\n\nafter\n");
}

#[test]
fn indentation_blank_paragraphs_and_raw_spelling_have_explicit_semantics() {
    for leading in ["", " ", "   "] {
        for end in ["\n", "\r\n"] {
            let source = format!(
                "{leading}[^n]:\tfirst{end}    **中🙂** &amp; $x$ [^raw]{end}{end}      extra  {end}    \tlast\\\rtail{end}after\n"
            );
            let doc = parse(&source);
            assert_eq!(
                read(&doc, 0, true),
                format!(
                    "first{end}**中🙂** &amp; $x$ [^raw]{end}{end}  extra  {end}\tlast\\\rtail"
                )
            );
            assert!(doc.maths().is_empty());
            assert!(doc.links().is_empty());
            assert!(doc.footnote_references().is_empty());
            assert_eq!(doc.footnotes().len(), 1);
            assert_eq!(visible(&doc), "after\n");
        }
    }
    for tail in [
        "no indent\n",
        "   three\n",
        "# heading\n",
        "---\n",
        "> child\n",
        "- sibling\n",
        "$$\nraw\n$$\n",
    ] {
        let doc = parse(&format!("[^n]: first\n{tail}"));
        assert_eq!(read(&doc, 0, true), "first", "{tail:?}");
        assert_eq!(
            doc.block(definition(&doc, 0).block).unwrap().source.end,
            "[^n]: first\n".len()
        );
    }
    for (tail, expected) in [("\ttab\n", "first\ntab"), (" \t mixed\n", "first\n mixed")] {
        assert_eq!(
            read(&parse(&format!("[^n]: first\n{tail}")), 0, true),
            expected
        );
    }
    let doc = parse("[^n]:\n\n    second\n\t\n    third\n\n\n");
    assert_eq!(read(&doc, 0, true), "\n\nsecond\n\nthird\n\n");
    assert_eq!(visible(&doc), "");
}

#[test]
fn explicit_container_paths_are_matched_without_parsing_nested_body_markers() {
    let source = "> - [x] [^n]: first\r\n>       > # literal\r\n>   \r\n>       - [^raw]: body\r\n>       ``` $x$ a|b\r\n> - sibling\n";
    let doc = parse(source);
    assert_eq!(
        read(&doc, 0, true),
        "first\r\n> # literal\r\n\r\n- [^raw]: body\r\n``` $x$ a|b"
    );
    assert_eq!(doc.container_count(), 4); // Quote, List, two Items only.
    assert_eq!(doc.footnotes().len(), 1);
    assert!(doc.headings().entries().is_empty());
    assert!(doc.maths().is_empty());
    assert!(doc.tables().is_empty());
    assert_eq!(doc.blocks()[0].container, Some(ContainerId(2)));
    assert_eq!(
        doc.blocks()[0].source.end,
        source.find("> - sibling").unwrap()
    );
    for (source, want) in [
        (
            "> > [^n]: first\n> >     same\n>     parent\n",
            "first\nsame",
        ),
        ("> [^n]: first\n    missing\n", "first"),
        ("- [^n]: first\n      same\n- [^m]: next\n", "first\nsame"),
        ("- [^n]: first\n\n      outside\n", "first"),
        ("> [^n]: first\n> >     child\n", "first"),
        ("> [^n]: first\n> lazy\n", "first"),
    ] {
        assert_eq!(read(&parse(source), 0, true), want, "{source:?}");
    }
}

#[test]
fn internal_blanks_and_trailing_list_separators_are_distinct() {
    for (source, want) in [
        (
            "- [^n]: first\n  \n      second\n- next\n",
            ListTightness::Tight,
        ),
        ("- [^n]: first\n  \n  tail\n", ListTightness::Loose),
        ("- [^n]: first\n  \n- next\n", ListTightness::Loose),
        ("- [^n]: first\n  \n", ListTightness::Tight),
        ("- > [^n]: first\n  >\n- next\n", ListTightness::Tight),
    ] {
        let doc = parse(source);
        assert_eq!(
            doc.container(ContainerId(0)).unwrap().list_tightness,
            Some(want),
            "{source:?}"
        );
    }
}

#[test]
fn first_definition_wins_including_empty_and_multiline_duplicates() {
    for first in ["[^n]:\n", "> [^n]: first\n>     second\n"] {
        let mut doc = Document::new();
        doc.append(first).unwrap();
        doc.append("[^n] [^n]\n").unwrap();
        let old = doc.footnotes().to_vec();
        let references = doc.footnote_references().to_vec();
        let update = doc
            .append(&format!(
                "[^n]: duplicate\n    {}\n\n    ignored\nend\n",
                "x".repeat(10_000)
            ))
            .unwrap();
        assert!(update.footnotes_changed.is_empty());
        assert!(update.new_footnotes.is_empty());
        assert!(update.new_footnote_references.is_empty());
        assert_eq!(doc.footnotes(), old);
        assert_eq!(doc.footnote_references(), references);
        assert_eq!(
            read(&doc, 0, true),
            if first.starts_with('>') {
                "first\nsecond"
            } else {
                ""
            }
        );
        assert_eq!(doc.footnotes()[0].number, Some(1));
        assert_eq!(visible(&doc), "[^n] [^n]\nend\n");
    }
    // Indented definition-like text is part of the current body, not a duplicate.
    let doc = parse("[^n]: first\n    [^m]: opaque\n[^m]: real\n");
    assert_eq!(doc.footnotes().len(), 2);
    assert_eq!(read(&doc, 0, true), "first\n[^m]: opaque");
    assert_eq!(read(&doc, 1, true), "real");
}

#[test]
fn growing_ranges_exclude_pending_lines_and_seal_only_at_boundary_or_eof() {
    let mut doc = Document::new();
    doc.append("[^n]: first\n").unwrap();
    let initial = definition(&doc, 0);
    assert_eq!(initial.state, SyntaxState::Growing);
    doc.append("    中🙂").unwrap();
    assert_eq!(definition(&doc, 0), initial);
    assert_eq!(read(&doc, 0, true), "first");
    doc.append("\r").unwrap();
    assert_eq!(definition(&doc, 0), initial);
    let update = doc.append("\n").unwrap();
    assert_eq!(update.footnotes_changed, [FootnoteId(0)]);
    assert_eq!(definition(&doc, 0).block, initial.block);
    assert_eq!(read(&doc, 0, true), "first\n中🙂");
    assert_eq!(definition(&doc, 0).state, SyntaxState::Growing);
    let end = doc.source().len();
    doc.append("not a continuation").unwrap();
    assert_eq!(definition(&doc, 0).state, SyntaxState::Growing);
    let update = doc.finish();
    assert_eq!(update.footnotes_changed, [FootnoteId(0)]);
    assert_eq!(definition(&doc, 0).state, SyntaxState::Sealed);
    assert_eq!(doc.block(initial.block).unwrap().source.end, end);
    assert_eq!(read(&doc, 0, true), "first\n中🙂");
    assert!(doc.finish().footnotes_changed.is_empty());
    assert_eq!(read(&parse("[^n]:\n    last\r"), 0, true), "\nlast\r");
}

#[test]
fn bounded_failed_candidates_and_existing_block_precedence_remain_explicit() {
    let source = "```\n[^code]: no\n    raw\n```\n\n    [^indent]: no\n$$\n[^math]: no\n$$\nh|h\n-|-\n[^n]: first\n    a|b\n    -|-\n    ```\n    # raw\n    [r]: /raw\nafter|table\n-|-\n";
    let doc = parse(source);
    assert_eq!(doc.footnotes().len(), 1);
    assert_eq!(
        read(&doc, 0, true),
        "first\na|b\n-|-\n```\n# raw\n[r]: /raw"
    );
    assert_eq!(doc.tables().len(), 2);
    assert!(doc.references().is_empty());
    assert!(doc.headings().entries().is_empty());
    for source in [
        format!("[^{}]: invalid\n    not a definition\n", "a".repeat(1000)),
        format!(
            "{}[^n]: beyond prefix\n{}    text\n",
            ">".repeat(BLOCK_LOOKAHEAD),
            ">".repeat(BLOCK_LOOKAHEAD)
        ),
        "[^bad label]: invalid\n    not a definition\n".to_owned(),
    ] {
        assert_eq!(
            parse(&source)
                .footnotes()
                .iter()
                .any(|n| n.definition.is_some()),
            source.starts_with('>')
        );
    }
    // A tab supplies four columns; the rest of an overlong whitespace line
    // streams as literal body without waiting for its end or rescanning it.
    let doc = parse(&format!(
        "[^n]: first\n{}\n    after\n",
        "\t".repeat(BLOCK_LOOKAHEAD + 5)
    ));
    assert_eq!(
        read(&doc, 0, true),
        format!("first\n{}\nafter", "\t".repeat(BLOCK_LOOKAHEAD + 4))
    );
}

#[test]
fn mapped_reads_share_budget_and_omitted_prefixes_never_advance_for_free() {
    let doc = parse("[^n]\n> [^n]: 中\n>     🙂&amp;\r\n>\r\n>     last\n");
    let mut frame = Budget::new(WorkLimit {
        queries: 3,
        objects: 3,
        dependencies: 1,
        bytes: 3,
        ..Default::default()
    });
    let mut refs = doc
        .footnote_references_in(Span::new(0, doc.source().len()))
        .unwrap();
    let (reference, _) = refs.advance(&mut frame).unwrap().unwrap();
    let resolved = doc
        .resolve_footnote(reference, &mut frame)
        .unwrap()
        .unwrap();
    let mut cursor = doc.footnote_payload(resolved.symbol).unwrap();
    assert_eq!(cursor.advance(&mut frame).unwrap().unwrap().text, "中");
    assert_eq!(cursor.advance(&mut frame), Err(Completion::BudgetExhausted));
    assert_eq!(cursor.advance(&mut tiny()).unwrap().unwrap().text, "\n");
    let start = cursor.source_position();
    for limit in [
        WorkLimit {
            queries: 0,
            objects: 1,
            bytes: 4,
            ..Default::default()
        },
        WorkLimit {
            queries: 1,
            objects: 0,
            bytes: 4,
            ..Default::default()
        },
        WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 0,
            ..Default::default()
        },
    ] {
        let mut budget = Budget::new(limit);
        assert_eq!(
            cursor.advance(&mut budget),
            Err(Completion::BudgetExhausted)
        );
        assert_eq!(cursor.source_position(), start);
        assert_eq!(budget.used(), WorkLimit::default());
    }
    let mut budget = tiny();
    let gap = cursor.advance(&mut budget).unwrap().unwrap();
    assert_eq!(gap.text, "");
    assert_eq!(gap.source.len(), 4);
    assert_eq!(budget.used().bytes, 4);
    assert_eq!(cursor.advance(&mut tiny()).unwrap().unwrap().text, "");
    let start = cursor.source_position();
    let mut too_small = Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 3,
        ..Default::default()
    });
    assert_eq!(
        cursor.advance(&mut too_small),
        Err(Completion::BudgetExhausted)
    );
    assert_eq!(cursor.source_position(), start);
    assert_eq!(too_small.used(), WorkLimit::default());
    assert_eq!(cursor.advance(&mut tiny()).unwrap().unwrap().text, "🙂");
    assert_eq!(read(&doc, 0, true), "中\n🙂&amp;\r\n\r\nlast");
    assert_eq!(
        read(&doc, 0, false),
        doc.text(definition(&doc, 0).content).unwrap()
    );
    assert!(doc.footnote_payload(FootnoteId(99)).is_none());
    assert!(
        parse("[^missing]\n")
            .footnote_payload(FootnoteId(0))
            .is_none()
    );
}

#[test]
fn deep_million_byte_continuation_never_revisits_body_or_ancestors() {
    let prefix = ">".repeat(1000);
    let mut doc = Document::new();
    doc.append(&format!(
        "{prefix} [^n]: first\n{prefix}     {}",
        "x".repeat(1_000_000)
    ))
    .unwrap();
    let id = definition(&doc, 0).block;
    let mut view = FoldView::new(&doc, 1).unwrap();
    view.set_collapsed(&doc, FoldId::Container(ContainerId(0)), true)
        .unwrap();
    let containers = doc.container_arena_stats();
    let before = doc.work();
    let visits = view.container_index_visits();
    let sections = view.section_index_visits();
    for _ in 0..1000 {
        let update = doc.append("x").unwrap();
        view.apply(&doc, &update).unwrap();
        assert_eq!(update.footnotes_changed, [FootnoteId(0)]);
        assert!(update.new_blocks.is_empty());
        assert!(update.new_containers.is_empty());
    }
    let after = doc.work();
    assert_eq!(after.input_bytes - before.input_bytes, 1000);
    assert_eq!(after.footnote_steps - before.footnote_steps, 1000);
    assert_eq!(after.inline_bytes, before.inline_bytes);
    assert_eq!(after.classified_bytes, before.classified_bytes);
    assert_eq!(after.footnote_label_bytes, before.footnote_label_bytes);
    assert_eq!(after.container_steps, before.container_steps);
    assert_eq!(after.container_bytes, before.container_bytes);
    assert_eq!(after.heading_steps, before.heading_steps);
    assert_eq!(doc.container_arena_stats(), containers);
    assert_eq!(view.container_index_visits(), visits);
    assert_eq!(view.section_index_visits(), sections);
    let before = doc.work();
    doc.append("\r\n").unwrap();
    assert_eq!(doc.work().footnote_steps - before.footnote_steps, 1);
    assert_eq!(definition(&doc, 0).state, SyntaxState::Growing);
    let before = doc.work();
    let update = doc.append("outside\n").unwrap();
    assert_eq!(doc.work().footnote_steps - before.footnote_steps, 1);
    assert_eq!(doc.work().container_steps - before.container_steps, 1002);
    assert!(doc.work().tree_visits - before.tree_visits < 1500);
    assert_eq!(update.closed_containers.len(), 1000);
    assert_eq!(definition(&doc, 0).block, id);
    assert_eq!(definition(&doc, 0).state, SyntaxState::Sealed);
    assert_eq!(doc.blocks().len(), 2);
}

#[test]
fn many_old_references_and_short_continuations_only_touch_current_symbol() {
    let mut doc = Document::new();
    doc.append(&format!("{}\n\n", "[^n] ".repeat(20_000)))
        .unwrap();
    let old = doc.footnote_references().to_vec();
    let before = doc.work();
    let update = doc
        .append("> - [^n]: first\n>       second\n>   \n>       third\n")
        .unwrap();
    assert_eq!(doc.work().footnote_steps - before.footnote_steps, 5);
    assert_eq!(
        doc.work().footnote_label_bytes - before.footnote_label_bytes,
        1
    );
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert_eq!(doc.footnote_references(), old);
    assert!(update.new_footnote_references.is_empty());
    assert_eq!(update.footnotes_changed.len(), 4);
    assert!(update.changes.len() < 16);
    assert!(doc.work().tree_visits - before.tree_visits < 2500);
    let first = doc
        .resolve_footnote(FootnoteReferenceId(0), &mut unlimited())
        .unwrap()
        .unwrap();
    let last = doc
        .resolve_footnote(FootnoteReferenceId(19_999), &mut unlimited())
        .unwrap()
        .unwrap();
    assert_eq!(first, last);
    assert_eq!(first.number, 1);
    assert_eq!(read(&doc, 0, true), "first\nsecond\n\nthird");
    let before = doc.work();
    for _ in 0..4096 {
        doc.append(">       x\n").unwrap();
    }
    assert_eq!(doc.work().footnote_steps - before.footnote_steps, 4096);
    assert_eq!(
        doc.work().container_steps - before.container_steps,
        2 * 4096
    );
    assert_eq!(doc.work().heading_steps, before.heading_steps);
    assert_eq!(doc.work().footnote_label_bytes, before.footnote_label_bytes);
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert_eq!(doc.footnote_references(), old);
    assert_eq!(doc.blocks().len(), 3);
}

#[test]
fn deep_list_prefixes_and_historical_definitions_have_linear_work() {
    let depth = 250;
    let mut doc = Document::new();
    doc.append(&format!("{}[^n]: first\n", "- ".repeat(depth)))
        .unwrap();
    let before = doc.work();
    doc.append(&format!("{}second\n", " ".repeat(2 * depth + 4)))
        .unwrap();
    assert_eq!(
        doc.work().container_steps - before.container_steps,
        depth as u64
    );
    assert!(doc.work().container_bytes - before.container_bytes < 4 * depth as u64);
    assert_eq!(doc.work().heading_steps, before.heading_steps);
    assert_eq!(doc.container_count(), 2 * depth);
    assert_eq!(doc.blocks().len(), 1);
    assert_eq!(read(&doc, 0, true), "first\nsecond");
    doc.append("outside\n").unwrap();
    doc.append(
        &(0..4096)
            .map(|i| format!("[^h{i}]: first\n    second\n"))
            .collect::<String>(),
    )
    .unwrap();
    let before = doc.work();
    let update = doc.append("[^next]: first\n    last\n").unwrap();
    assert_eq!(doc.work().footnote_steps - before.footnote_steps, 4);
    assert_eq!(
        doc.work().footnote_label_bytes - before.footnote_label_bytes,
        4
    );
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert_eq!(update.footnotes_changed.len(), 3);
    assert_eq!(read(&doc, 4097, true), "first\nlast");
    for i in (1..4097).step_by(137) {
        assert_eq!(read(&doc, i, true), "first\nsecond");
    }
}

#[test]
fn generated_ranges_and_payloads_match_independent_line_model() {
    let mut source = String::new();
    let mut expected = Vec::new();
    for i in 0..400 {
        let (open, prefix, depth) = match i % 4 {
            0 => ("", "", 0),
            1 => ("> ", "> ", 1),
            2 => ("> - ", ">   ", 3),
            _ => ("- > ", "  > ", 3),
        };
        let eol = if i % 3 == 0 { "\r\n" } else { "\n" };
        let start = source.len();
        source.push_str(&format!("{open}[^n{i}]: "));
        let content_start = source.len();
        source.push_str("中🙂");
        let mut payload = "中🙂".to_owned();
        let mut content_end = source.len();
        source.push_str(eol);
        for line in 0..i % 7 + 1 {
            payload.push_str(eol);
            source.push_str(prefix);
            if line % 3 != 1 {
                let text = if line % 2 == 0 {
                    "**raw** &amp;\\"
                } else {
                    "> [^opaque]: text"
                };
                source.push_str("    ");
                source.push_str(text);
                payload.push_str(text);
            }
            content_end = source.len();
            source.push_str(eol);
        }
        expected.push((
            Span::new(start, source.len()),
            Span::new(content_start, content_end),
            payload,
            depth,
        ));
        source.push_str("outside\n");
    }
    let doc = parse(&source);
    assert_eq!(doc.footnotes().len(), expected.len());
    for (i, (span, content, payload, depth)) in expected.into_iter().enumerate() {
        let d = definition(&doc, i);
        assert_eq!(d.content, content);
        assert_eq!(d.state, SyntaxState::Sealed);
        assert_eq!(doc.block(d.block).unwrap().source, span);
        assert_eq!(read(&doc, i, true), payload);
        assert_eq!(read(&doc, i, false), source[content.range()]);
        assert_eq!(
            doc.block(d.block)
                .unwrap()
                .container
                .map_or(0, |id| doc.container(id).unwrap().depth),
            depth
        );
        for at in [span.start, content.start, span.end - 1] {
            assert_eq!(doc.block_at(at), Some(d.block));
        }
    }
    assert_eq!(visible(&doc), "outside\n".repeat(400));
    assert!(doc.work().classified_bytes < 3 * source.len() as u64);
    assert!(doc.work().container_bytes < 4 * source.len() as u64);
}

#[test]
fn growth_notifications_reconstruct_symbols_invalidate_old_consumers_and_merge() {
    let mut doc = Document::new();
    let mut immediate = FoldView::<()>::new(&doc, 1).unwrap();
    let mut merged = FoldView::<()>::new(&doc, 1).unwrap();
    let first = doc
        .append(&format!(
            "[^n] {}\n\n[^other]\n> [^n]: first\n",
            "x".repeat(500)
        ))
        .unwrap();
    immediate.apply(&doc, &first).unwrap();
    merged.apply(&doc, &first).unwrap();
    let fold = FoldId::Container(ContainerId(0));
    immediate.set_collapsed(&doc, fold, true).unwrap();
    merged.set_collapsed(&doc, fold, true).unwrap();
    let old = definition(&doc, 0);
    let refs = doc.footnote_references().to_vec();
    let ticket = immediate.heights().ticket(TileId(0)).unwrap();
    let key = DependencyKey::capture(&doc, TileId(0), &mut unlimited())
        .unwrap()
        .unwrap();
    let other_tile = doc.tile_at(refs[1].source.start).unwrap();
    let other = DependencyKey::capture(&doc, other_tile, &mut unlimited())
        .unwrap()
        .unwrap();
    let mut search = DisplaySearch::new(&doc, "first").unwrap();
    let mut symbols = doc.footnotes().to_vec();
    let mut blocks = doc.blocks().to_vec();
    let mut batch = Update::default();
    let mut scratch = Update::default();
    for chunk in [">     中🙂\r", "\n", ">\n", ">     last\n", "> tail\n"] {
        doc.append_into(chunk, &mut scratch).unwrap();
        symbols.extend_from_slice(&doc.footnotes()[scratch.new_footnotes.clone()]);
        for id in &scratch.footnotes_changed {
            symbols[id.0] = doc.footnote(*id).unwrap().clone();
        }
        blocks.extend_from_slice(&doc.blocks()[scratch.new_blocks.clone()]);
        for i in scratch.changed_blocks.clone() {
            blocks[i] = doc.blocks()[i].clone();
        }
        assert_eq!(symbols, doc.footnotes());
        assert_eq!(blocks, doc.blocks());
        immediate.apply(&doc, &scratch).unwrap();
        batch.merge(scratch.clone()).unwrap();
    }
    assert_eq!(definition(&doc, 0).block, old.block);
    assert_eq!(doc.footnote_references(), refs);
    assert_eq!(read(&doc, 0, true), "first\n中🙂\r\n\nlast");
    assert!(immediate.measure(ticket, 99).is_err());
    assert_eq!(
        key.validate(&doc, &mut unlimited()).unwrap(),
        CacheValidity::Changed
    );
    assert_eq!(
        other.validate(&doc, &mut unlimited()).unwrap(),
        CacheValidity::Valid
    );
    let mut zero = Budget::new(WorkLimit::default());
    assert_eq!(
        search.advance(&doc, &mut zero, &mut Vec::new()),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(zero.used(), WorkLimit::default());
    doc.finish_into(&mut scratch);
    immediate.apply(&doc, &scratch).unwrap();
    batch.merge(scratch).unwrap();
    merged.apply(&doc, &batch).unwrap();
    assert!(immediate.is_collapsed(fold));
    assert!(merged.is_collapsed(fold));
    assert_eq!(
        immediate.heights().total_rows(),
        merged.heights().total_rows()
    );
    for i in 0..doc.tile_count() {
        assert_eq!(
            immediate.heights().height(TileId(i)),
            merged.heights().height(TileId(i))
        );
    }
    let mut search = DisplaySearch::new(&doc, "last").unwrap();
    let mut hits = Vec::new();
    assert_eq!(
        search.advance(&doc, &mut unlimited(), &mut hits).unwrap(),
        Completion::EndOfDocument
    );
    assert!(hits.is_empty()); // Hidden definition bodies stay outside generic display search.
}

#[test]
fn folded_multiline_million_bytes_have_zero_body_reads_and_keep_scope_boundaries() {
    struct Sink(usize);
    impl LayoutSink for Sink {
        fn begin_tile(&mut self, _: TileId) {}
        fn text(&mut self, _: RenderRun<'_>) {
            panic!("fold read body");
        }
        fn object(&mut self, _: LinkId, _: &LinkInfo) {
            panic!("unexpected object");
        }
        fn end_tile(&mut self, _: TileId) -> u64 {
            1
        }
    }
    impl FoldLayoutSink for Sink {
        fn placeholder(&mut self, _: FoldPlaceholder) {
            self.0 += 1;
        }
    }
    let mut doc = Document::new();
    doc.append(&format!(
        "> [^n]: first\n>     {}\n>\n>     last\n",
        "x".repeat(1_000_000)
    ))
    .unwrap();
    let fold = FoldId::Container(ContainerId(0));
    let mut view = FoldView::new(&doc, 1).unwrap();
    view.set_collapsed(&doc, fold, true).unwrap();
    let mut pass = LayoutPass::folded(&doc, &view, TileId(0), 1).unwrap();
    let mut budget = Budget::new(WorkLimit {
        queries: 10,
        objects: 4,
        tiles: 2,
        ..Default::default()
    });
    let mut sink = Sink(0);
    assert_eq!(
        pass.advance_folded(&doc, &mut view, &mut budget, &mut sink)
            .unwrap(),
        Completion::Covered
    );
    assert_eq!(sink.0, 1);
    assert_eq!(budget.used().bytes, 0);
    let end = doc.source().len();
    let update = doc.append("out").unwrap();
    view.apply(&doc, &update).unwrap();
    assert!(matches!(
        view.hidden_at(&doc, end, &mut tiny()).unwrap(),
        FoldLookup::Hidden(_)
    ));
    let update = doc.append("side\n").unwrap();
    view.apply(&doc, &update).unwrap();
    assert_eq!(doc.blocks()[0].source.end, end);
    assert_eq!(
        view.hidden_at(&doc, end, &mut tiny()).unwrap(),
        FoldLookup::Visible
    );
    assert!(view.is_collapsed(fold));
    assert_eq!(definition(&doc, 0).state, SyntaxState::Sealed);
}

#[test]
fn streaming_fold_visibility_and_heights_match_container_interval_model() {
    let mut doc = Document::new();
    let mut view = FoldView::<()>::new(&doc, 1).unwrap();
    let mut chosen = Vec::new();
    for step in 0..240 {
        let chunk = match step % 6 {
            0 => format!("# section\n> [^n{step}]: first\n"),
            1 => ">     中🙂 &amp;\n".to_owned(),
            2 => ">\n>     last\n".to_owned(),
            3 => "> h|h\n> -|-\n> a|b\n".to_owned(),
            4 => "out".to_owned(),
            _ => "side\n".to_owned(),
        };
        let update = doc.append(&chunk).unwrap();
        view.apply(&doc, &update).unwrap();
        if step % 6 == 0 {
            let id = ContainerId(doc.container_count() - 1);
            view.set_collapsed(&doc, FoldId::Container(id), true)
                .unwrap();
            chosen.push(id);
        }
        let ranges: Vec<_> = chosen
            .iter()
            .map(|id| {
                let node = doc.container(*id).unwrap();
                Span::new(
                    node.source.start,
                    if node.state == SyntaxState::Growing {
                        doc.source().len()
                    } else {
                        node.source.end
                    },
                )
            })
            .collect();
        for (at, _) in doc.source().char_indices() {
            let expected = ranges.iter().any(|r| r.range().contains(&at));
            assert_eq!(
                matches!(
                    view.hidden_at(&doc, at, &mut tiny()).unwrap(),
                    FoldLookup::Hidden(_)
                ),
                expected
            );
        }
        let mut rows = 0;
        for i in 0..doc.tile_count() {
            let tile = doc.tile(TileId(i)).unwrap();
            let hidden = ranges
                .iter()
                .any(|r| doc.tile_at(r.start).unwrap().0 < i && tile.end <= r.end);
            assert_eq!(
                view.heights().height(TileId(i)),
                Some(u64::from(!hidden)),
                "step {step}, tile {i}"
            );
            rows += u128::from(!hidden);
        }
        assert_eq!(view.heights().total_rows(), rows);
    }
}
