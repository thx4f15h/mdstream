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

fn resolve(doc: &Document, id: usize) -> FootnoteResolution {
    doc.resolve_footnote(FootnoteReferenceId(id), &mut unlimited())
        .unwrap()
        .unwrap()
}

fn visible(doc: &Document) -> String {
    doc.display_runs_in(Span::new(0, doc.source().len()))
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect()
}

#[test]
fn first_reference_numbering_is_stable_and_independent_of_definition_order() {
    let mut doc = Document::new();
    doc.append("[^b]: earlier definition\n[^unused]: unused\n")
        .unwrap();
    assert!(doc.footnotes().iter().all(|n| n.number.is_none()));
    doc.append("[^a] [^b] [^a] [^A] [^中_1-字]\n").unwrap();
    assert_eq!(
        (0..5).map(|i| resolve(&doc, i).number).collect::<Vec<_>>(),
        [1, 2, 1, 3, 4]
    );
    let old = doc.footnote_references().to_vec();
    let a = old[0].symbol;
    assert_ne!(a, FootnoteId(0)); // Symbol order is not number order.
    assert_eq!(resolve(&doc, 0).target, FootnoteTarget::Unresolved);
    let update = doc
        .append("[^a]: later body\n[^a]: ignored replacement\n")
        .unwrap();
    assert!(update.new_footnote_references.is_empty());
    assert_eq!(doc.footnote_references(), old);
    assert_eq!(
        doc.text(doc.footnote(a).unwrap().definition.unwrap().content),
        Some("later body")
    );
    doc.finish();
    assert_eq!(resolve(&doc, 3).target, FootnoteTarget::Missing);
    assert_eq!(resolve(&doc, 4).number, 4);
    assert_eq!(doc.footnote(FootnoteId(1)).unwrap().number, None);
}

#[test]
fn labels_namespaces_raw_bodies_and_literal_display_are_explicit() {
    let doc = parse(
        "[^x] [x] [^empty] [^no-space]\n\n[x]: /url\n[^x]: **raw** &amp; $x$ [^nested]  \r\n[^empty]:\n[^no-space]:body\n",
    );
    assert_eq!(doc.footnote_references().len(), 3);
    assert_eq!(doc.references().len(), 1);
    assert_eq!(doc.link_destination(LinkId(0)), Some("/url"));
    let body = doc.footnote(FootnoteId(0)).unwrap().definition.unwrap();
    assert_eq!(
        doc.text(body.content),
        Some("**raw** &amp; $x$ [^nested]  ")
    );
    assert_eq!(body.state, SyntaxState::Sealed);
    assert!(doc.maths().is_empty());
    assert!(
        doc.footnote(FootnoteId(1))
            .unwrap()
            .definition
            .unwrap()
            .content
            .is_empty()
    );
    assert_eq!(
        doc.text(
            doc.footnote(FootnoteId(2))
                .unwrap()
                .definition
                .unwrap()
                .content
        ),
        Some("body")
    );
    assert_eq!(visible(&doc), "[^x] x [^empty] [^no-space]\n\n");
}

#[test]
fn invalid_and_unfinished_labels_do_not_become_reference_links_or_replay() {
    for source in [
        "[^]", "[^a b]", "[^a\nb]", "[^a\\b]", "[^a.b]", "[^🙂]", "[^&amp;]", "[^open", "[^x]:",
    ] {
        let doc = parse(source);
        assert!(doc.footnote_references().is_empty(), "{source:?}");
        assert!(doc.links().is_empty(), "{source:?}");
    }
    for label in ["a".repeat(MAX_FOOTNOTE_LABEL_BYTES + 1), "中".repeat(334)] {
        let doc = parse(&format!("[^{label}]\n[^{label}]: body\n"));
        assert!(doc.footnotes().is_empty());
    }
    for label in ["a".repeat(MAX_FOOTNOTE_LABEL_BYTES), "中".repeat(333)] {
        let doc = parse(&format!("[^{label}]\n[^{label}]: body\n"));
        assert_eq!(doc.footnotes().len(), 1);
        assert!(matches!(
            resolve(&doc, 0).target,
            FootnoteTarget::Defined(_)
        ));
    }
    let mut doc = Document::new();
    doc.append(&"x".repeat(BLOCK_LOOKAHEAD)).unwrap();
    doc.append("[^late").unwrap();
    assert!(doc.footnotes().is_empty());
    let update = doc.append("]").unwrap();
    assert_eq!(update.new_footnotes, 0..1);
    assert_eq!(update.new_footnote_references, 0..1);
    assert_eq!(resolve(&doc, 0).number, 1);
    doc.append("(url)").unwrap();
    assert!(doc.links().is_empty()); // A confirmed footnote never converts into a link.
}

#[test]
fn inline_contexts_containers_tables_and_block_precedence_compose() {
    let doc = parse(
        "`[^code]` $[^math]$ [link](/[^target] '[^title]') <https://x/[^auto]> \\[^escaped] ![^image] **[^yes]**\n\n```\n[^fence]: body\n```\n\n    [^indent]: body\n> [^quote]: container definition\n- [^list]\n\n| [^head] | h |\n| -- | -- |\n| [^cell] | [^cut |\n| off] | end |\n\n[^yes]: raw\n",
    );
    let labels: Vec<_> = doc
        .footnote_references()
        .iter()
        .map(|r| doc.footnote(r.symbol).unwrap().label.as_str())
        .collect();
    assert_eq!(labels, ["yes", "list", "head", "cell"]);
    assert_eq!(
        doc.footnotes()
            .iter()
            .filter(|n| n.definition.is_some())
            .count(),
        2
    );
    let first = doc.footnote_references()[0];
    assert!(
        doc.runs_in(first.source)
            .unwrap()
            .all(|r| r.style.contains(Style::STRONG))
    );
    assert_eq!(doc.tables().len(), 1);
    // Definitions end a table rather than becoming another data row.
    let doc = parse("h | h\n-- | --\n[^stop]: a|b\n");
    assert_eq!(doc.tables()[0].rows().len(), 1);
    assert_eq!(
        doc.text(
            doc.footnote(FootnoteId(0))
                .unwrap()
                .definition
                .unwrap()
                .content
        ),
        Some("a|b")
    );
}

#[test]
fn growing_definition_and_split_crlf_match_every_observed_prefix() {
    let source = format!("[^x]\n\n[^x]: {}\r\n[^x]: duplicate\n", "中".repeat(500));
    let mut streamed = Document::new();
    for (offset, c) in source.char_indices() {
        streamed.append(c.encode_utf8(&mut [0; 4])).unwrap();
        let mut whole = Document::new();
        whole.append(&source[..offset + c.len_utf8()]).unwrap();
        assert_eq!(streamed.footnotes(), whole.footnotes(), "prefix {offset}");
        assert_eq!(streamed.footnote_references(), whole.footnote_references());
        assert_eq!(streamed.blocks(), whole.blocks());
    }
    streamed.finish();
    let whole = parse(&source);
    assert_eq!(streamed.footnotes(), whole.footnotes());
    assert_eq!(
        streamed
            .footnote(FootnoteId(0))
            .unwrap()
            .definition
            .unwrap()
            .state,
        SyntaxState::Sealed
    );
}

#[test]
fn updates_can_reconstruct_symbols_and_occurrences_and_merge_without_history_scans() {
    let mut doc = Document::new();
    let mut symbols = Vec::new();
    let mut refs = Vec::new();
    let mut batch = Update::default();
    let mut scratch = Update::default();
    for chunk in [
        "[^a]\n",
        "[^b]: ",
        &"x".repeat(2000),
        "more",
        "\n[^b] [^a]\n",
        "[^a]: done",
    ] {
        doc.append_into(chunk, &mut scratch).unwrap();
        symbols.extend_from_slice(&doc.footnotes()[scratch.new_footnotes.clone()]);
        refs.extend_from_slice(&doc.footnote_references()[scratch.new_footnote_references.clone()]);
        for id in &scratch.footnotes_changed {
            symbols[id.0] = doc.footnote(*id).unwrap().clone();
        }
        assert_eq!(symbols, doc.footnotes());
        assert_eq!(refs, doc.footnote_references());
        batch.merge(scratch.clone()).unwrap();
    }
    doc.finish_into(&mut scratch);
    symbols.extend_from_slice(&doc.footnotes()[scratch.new_footnotes.clone()]);
    for id in &scratch.footnotes_changed {
        symbols[id.0] = doc.footnote(*id).unwrap().clone();
    }
    assert_eq!(symbols, doc.footnotes());
    batch.merge(scratch).unwrap();
    assert_eq!(batch.new_footnotes, 0..2);
    assert_eq!(batch.new_footnote_references, 0..3);
    let empty = doc.finish();
    assert!(empty.new_footnotes.is_empty());
    assert!(empty.new_footnote_references.is_empty());
    assert!(empty.footnotes_changed.is_empty());
    let mut doc = Document::new();
    let empty = doc.append("").unwrap();
    assert!(empty.footnotes_changed.is_empty());
}

#[test]
fn shared_budget_resolution_and_utf8_body_reader_resume_atomically() {
    let doc = parse("[^x]\n\n[^x]: 中🙂raw\n");
    let mut objects = doc
        .footnote_references_in(Span::new(0, doc.source().len()))
        .unwrap();
    let mut zero = Budget::new(WorkLimit::default());
    assert_eq!(objects.advance(&mut zero), Err(Completion::BudgetExhausted));
    assert_eq!(
        doc.resolve_footnote(FootnoteReferenceId(0), &mut zero),
        Err(Completion::BudgetExhausted)
    );
    let mut frame = Budget::new(WorkLimit {
        queries: 3,
        objects: 3,
        dependencies: 1,
        bytes: 3,
        ..WorkLimit::default()
    });
    let (id, reference) = objects.advance(&mut frame).unwrap().unwrap();
    let resolution = doc.resolve_footnote(id, &mut frame).unwrap().unwrap();
    assert_eq!(resolution.number, 1);
    let mut reader = doc.footnote_content(reference.symbol).unwrap();
    assert_eq!(reader.advance(&mut frame).unwrap().unwrap().text, "中");
    assert_eq!(reader.advance(&mut frame), Err(Completion::BudgetExhausted));
    let position = reader.source_position();
    for limit in [
        WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 3,
            ..WorkLimit::default()
        },
        WorkLimit {
            queries: 1,
            objects: 0,
            bytes: 4,
            ..WorkLimit::default()
        },
        WorkLimit {
            queries: 0,
            objects: 1,
            bytes: 4,
            ..WorkLimit::default()
        },
    ] {
        let mut budget = Budget::new(limit);
        assert_eq!(
            reader.advance(&mut budget),
            Err(Completion::BudgetExhausted)
        );
        assert_eq!(reader.source_position(), position);
        assert_eq!(budget.used(), WorkLimit::default());
    }
    let mut text = String::from("中");
    loop {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 4,
            ..WorkLimit::default()
        });
        let Some(chunk) = reader.advance(&mut budget).unwrap() else {
            break;
        };
        assert_eq!(doc.text(chunk.source), Some(chunk.text));
        text.push_str(chunk.text);
    }
    assert_eq!(text, "中🙂raw");
    assert_eq!(reader.advance(&mut zero), Ok(None));
    assert!(doc.footnote_content(FootnoteId(99)).is_none());
    assert_eq!(
        doc.resolve_footnote(FootnoteReferenceId(99), &mut unlimited()),
        Ok(None)
    );
    for span in [Span::new(5, 1), Span::new(0, usize::MAX), Span::new(14, 15)] {
        assert!(doc.footnote_references_in(span).is_none());
    }
    assert_eq!(
        doc.footnote_references_in(Span::new(0, 0))
            .unwrap()
            .advance(&mut zero),
        Ok(None)
    );
}

#[test]
fn indexed_viewport_queries_match_linear_model_and_never_read_bodies() {
    let source: String = (0..4096).map(|i| format!("[^n{i}] ")).collect();
    let doc = parse(&source);
    assert_eq!(doc.footnotes().len(), 4096);
    assert_eq!(doc.footnote_references().len(), 4096);
    for start in (0..source.len()).step_by(137) {
        let range = Span::new(start, (start + 31).min(source.len()));
        let want: Vec<_> = doc
            .footnote_references()
            .iter()
            .enumerate()
            .filter(|(_, r)| r.source.start < range.end && r.source.end > range.start)
            .map(|(i, r)| (FootnoteReferenceId(i), *r))
            .collect();
        let mut cursor = doc.footnote_references_in(range).unwrap();
        let mut got = Vec::new();
        loop {
            let mut budget = Budget::new(WorkLimit {
                queries: 1,
                objects: 1,
                ..WorkLimit::default()
            });
            let Some(next) = cursor.advance(&mut budget).unwrap() else {
                break;
            };
            got.push(next);
            assert_eq!(budget.used().bytes, 0);
            assert_eq!(
                cursor.advance(&mut budget),
                Err(Completion::BudgetExhausted)
            );
        }
        assert_eq!(got, want);
        assert_eq!(
            cursor.advance(&mut Budget::new(WorkLimit::default())),
            Ok(None)
        );
    }
}

#[test]
fn late_definition_touches_one_symbol_without_visiting_twenty_thousand_old_references() {
    let mut doc = Document::new();
    doc.append(&format!("{}\n\n", "[^x] ".repeat(20_000)))
        .unwrap();
    let before = doc.work();
    let old = doc.footnote_references().to_vec();
    let update = doc.append("[^x]: late body\n").unwrap();
    // Intern + define; the definition remains open for indented continuation.
    assert_eq!(doc.work().footnote_steps - before.footnote_steps, 2);
    assert_eq!(
        doc.work().footnote_label_bytes - before.footnote_label_bytes,
        1
    );
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert!(doc.work().tree_visits - before.tree_visits < 1000);
    assert_eq!(doc.footnote_references(), old);
    assert!(update.new_footnote_references.is_empty());
    assert!(update.dirty.len() < 4);
    assert!(update.changes.len() < 8);
    assert!(update.footnotes_changed.len() <= 2);
    assert!(matches!(
        resolve(&doc, 19_999).target,
        FootnoteTarget::Defined(_)
    ));
}

#[test]
fn million_byte_definition_growth_and_unclosed_candidates_have_linear_work() {
    let mut doc = Document::new();
    doc.append("[^x]\n\n[^x]: ").unwrap();
    doc.append(&"x".repeat(1_000_000)).unwrap();
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("x").unwrap();
    }
    assert_eq!(doc.work().footnote_steps - before.footnote_steps, 1000);
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert_eq!(doc.work().classified_bytes, before.classified_bytes);
    assert_eq!(doc.work().footnote_label_bytes, before.footnote_label_bytes);
    let before = doc.work();
    doc.append("\r").unwrap();
    doc.append("\n").unwrap();
    assert_eq!(doc.work().footnote_steps - before.footnote_steps, 2);
    let body = doc.footnote(FootnoteId(0)).unwrap().definition.unwrap();
    assert_eq!(body.content.len(), 1_001_000);
    assert_eq!(body.state, SyntaxState::Growing);
    assert_eq!(doc.next_visible(body.content.start), None);
    let mut reader = doc.footnote_content(FootnoteId(0)).unwrap();
    let mut budget = Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 16,
        ..WorkLimit::default()
    });
    assert_eq!(reader.advance(&mut budget).unwrap().unwrap().text.len(), 16);
    for n in [1024, 4096, 16_384] {
        for source in [format!("[^{}", "a".repeat(n)), "[^".repeat(n)] {
            let mut doc = Document::new();
            for c in source.chars() {
                doc.append(c.encode_utf8(&mut [0; 4])).unwrap();
            }
            doc.finish();
            assert!(doc.footnotes().is_empty());
            assert_eq!(doc.work().inline_bytes, source.len() as u64);
            assert_eq!(doc.work().footnote_steps, 0);
            assert!(doc.annotation_node_count() < 32);
        }
    }
}

#[test]
fn symbol_dependencies_and_layout_tickets_invalidate_on_late_definition_and_eof() {
    let mut doc = Document::new();
    let update = doc
        .append(&format!(
            "[^x] {}\n\n[^y] {}\n\n",
            "a".repeat(500),
            "b".repeat(500)
        ))
        .unwrap();
    let key = doc.tile_key(TileId(0)).unwrap();
    let x = DependencyKey::capture(&doc, TileId(0), &mut unlimited())
        .unwrap()
        .unwrap();
    let y_tile = doc
        .tile_at(doc.footnote_references()[1].source.start)
        .unwrap();
    let y = DependencyKey::capture(&doc, y_tile, &mut unlimited())
        .unwrap()
        .unwrap();
    let mut heights = HeightIndex::default();
    heights.apply(&doc, &update).unwrap();
    let ticket = heights.ticket(TileId(0)).unwrap();
    let next = doc.append("[^x]: new\n").unwrap();
    assert_ne!(doc.tile_key(TileId(0)).unwrap(), key);
    assert_eq!(
        x.validate(&doc, &mut unlimited()).unwrap(),
        CacheValidity::Changed
    );
    assert_eq!(
        y.validate(&doc, &mut unlimited()).unwrap(),
        CacheValidity::Valid
    );
    heights
        .apply_with_policy(
            &doc,
            &next,
            LayoutPolicy {
                style_affects_size: false,
                references_affect_size: false,
                table_width_affects_size: false,
            },
        )
        .unwrap();
    assert!(heights.measure(ticket, 3).is_err());
    let ticket = heights.ticket(y_tile).unwrap();
    let eof = doc.finish();
    assert_eq!(resolve(&doc, 1).target, FootnoteTarget::Missing);
    assert_eq!(
        y.validate(&doc, &mut unlimited()).unwrap(),
        CacheValidity::Changed
    );
    heights.apply(&doc, &eof).unwrap();
    assert!(heights.measure(ticket, 1).is_err());
    let mut too_small = Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        ..WorkLimit::default()
    });
    assert!(
        DependencyKey::capture(&doc, TileId(0), &mut too_small)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        x.validate(
            &doc,
            &mut Budget::new(WorkLimit {
                queries: 1,
                ..WorkLimit::default()
            })
        )
        .unwrap(),
        CacheValidity::BudgetExhausted
    );
}
