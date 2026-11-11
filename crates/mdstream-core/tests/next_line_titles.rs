use mdstream::*;

#[path = "support/link_metadata.rs"]
mod link_metadata;
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

#[test]
fn next_line_title_matches_scope_indent_delimiter_and_raw_span_model() {
    let mut cases = 0;
    for (first, next) in [
        ("", ""),
        ("> ", "> "),
        ("- ", "  "),
        ("> - ", ">   "),
        ("- > ", "  > "),
        ("- - ", "\t"),
        (">\t", ">\t"),
        ("-  - ", "\t "),
        ("1. ", "\t"),
        ("> > ", ">\t>\t"),
    ] {
        for target_next in [false, true] {
            for indent in ["", "   ", "\t", "        ", " \t"] {
                for (title, raw) in [
                    ("''", ""),
                    ("\"\"", ""),
                    ("()", ""),
                    ("'中🙂 &amp; \\'t\\''", "中🙂 &amp; \\'t\\'"),
                    ("\"a\\\" &quot; 中🙂\"", "a\\\" &quot; 中🙂"),
                    ("(a\\(b\\) &amp; 中🙂)", "a\\(b\\) &amp; 中🙂"),
                    (
                        "'[*x*](url) $m$ [^n] | `code`'",
                        "[*x*](url) $m$ [^n] | `code`",
                    ),
                ] {
                    for ending in ["\n", "\r\n", ""] {
                        let head = if target_next {
                            format!("{first}[r\\]]: \r\n{next}<my url>\r\n")
                        } else {
                            format!("{first}[r\\]]: <my url>\r\n")
                        };
                        let start = head.len() + next.len() + indent.len() + 1;
                        let source = format!("{head}{next}{indent}{title} \t{ending}");
                        let doc = parse(&source);
                        assert_eq!(doc.blocks().len(), 1, "{source:?}");
                        let block = &doc.blocks()[0];
                        assert!(matches!(block.kind, BlockKind::Definition { .. }));
                        assert_eq!(block.source, Span::new(0, source.len()));
                        assert_eq!(block.state, SyntaxState::Sealed);
                        assert_eq!(block.container.is_some(), !first.is_empty());
                        assert_eq!(doc.references().len(), 1);
                        let reference = &doc.references()[0];
                        assert_eq!(
                            reference.destination.and_then(|s| doc.text(s)),
                            Some("my url")
                        );
                        assert_eq!(reference.title, Some(Span::new(start, start + raw.len())));
                        assert_eq!(reference.title.and_then(|s| doc.text(s)), Some(raw));
                        assert_eq!(reference.revision, 2);
                        assert!(
                            doc.links().is_empty(),
                            "title metadata leaked inline objects"
                        );
                        assert!(doc.maths().is_empty());
                        assert!(doc.footnotes().is_empty());
                        assert!(doc.tables().is_empty());
                        assert!(visible(&doc).is_empty());
                        for at in 0..source.len() {
                            assert_eq!(doc.block_at(at), Some(BlockId(0)));
                        }
                        cases += 1;
                    }
                }
            }
        }
    }
    assert_eq!(cases, 2100);
}

#[test]
fn next_line_title_failures_keep_published_target_and_ordinary_block_semantics() {
    for (first, next) in [("", ""), ("> ", "> "), ("- ", "\t"), ("- > ", "\t>\t")] {
        for target_next in [false, true] {
            for tail in [
                "",
                "\n",
                " \t\r\n",
                "'unclosed\n'\n",
                "\"unclosed",
                "(a(b))\n",
                "'closed' bad\n",
                "(closed))\n",
                "ordinary\n===\n",
                "[s]: /s\n",
                "# heading\n",
                "~~~\n",
                "$$\n",
                "[^n]: note\n",
                "a|b\n-|-\n",
                "    'unclosed\n",
                "'first\nsecond'\n",
                "\"\"x\n",
            ] {
                if first.is_empty() && matches!(tail, "'unclosed\n'\n" | "'first\nsecond'\n") {
                    // Root title bodies now continue across physical lines.
                    // Container scope still exercises the original failure path.
                    continue;
                }
                let head = if target_next {
                    format!("{first}[r]:\n{next}/url    \n")
                } else {
                    format!("{first}[r]: /url    \n")
                };
                // An equal-length, already titled definition is an independent
                // control for the successor's unchanged ordinary block path.
                let model_head = head.replace("/url    ", "/url 'x'");
                let source = format!("{head}{next}{tail}");
                let doc = parse(&source);
                let model = parse(&format!("{model_head}{next}{tail}"));
                assert_eq!(doc.blocks(), model.blocks(), "{source:?}");
                assert_eq!(doc.container_count(), model.container_count());
                for i in 0..doc.container_count() {
                    assert_eq!(
                        doc.container(ContainerId(i)),
                        model.container(ContainerId(i))
                    );
                }
                assert_eq!(doc.links(), model.links());
                assert_eq!(doc.headings().entries(), model.headings().entries());
                assert_eq!(doc.tables(), model.tables());
                assert_eq!(visible(&doc), visible(&model));
                assert_eq!(
                    doc.references()[0].destination.and_then(|s| doc.text(s)),
                    Some("/url")
                );
                assert_eq!(doc.references()[0].title, None);
                assert_eq!(doc.references()[0].revision, 1);
            }
        }
    }
    for source in [
        "> [r]: /url\n'outside'\n",
        "> - [r]: /url\n> 'missing item'\n",
        "> - [r]: /url\n> - 'sibling'\n",
        "- > [r]: /url\n  'missing quote'\n",
        "> [r]: /url\n> > 'new quote'\n",
        "> [r]: /url\n> - 'new list'\n",
        "- [r]: /url\n- 'sibling'\n",
        "> 2. [r]: /url\n>\t2. 'sibling'\n",
    ] {
        let doc = parse(source);
        let model = parse(&source.replacen("/url", "/url ''", 1));
        assert_eq!(doc.references()[0].title, None, "{source:?}");
        assert_eq!(
            doc.references()[0].destination.and_then(|s| doc.text(s)),
            Some("/url")
        );
        assert_eq!(visible(&doc), visible(&model));
        assert_eq!(doc.container_count(), model.container_count());
    }
}

#[test]
fn next_line_title_only_winning_definition_owns_symbol_and_existing_titles_stop() {
    // A winner without a title must not receive a duplicate's continuation.
    // This distinguishes ownership from merely checking entry.title == None.
    for (target, destination) in [("/first", "/first"), ("<>", "")] {
        let mut doc = Document::new();
        doc.append(&format!("[r]\n\n[r]: {target}\n")).unwrap();
        let revision = doc.tile_key(TileId(0)).unwrap().reference_revision;
        let update = doc.append("[r]: /duplicate\n'ignored'\n").unwrap();
        assert!(update.references_changed.is_empty());
        assert!(update.new_links.is_empty());
        assert_eq!(doc.link_destination(LinkId(0)), Some(destination));
        assert_eq!(doc.link_title(LinkId(0)), None);
        assert_eq!(doc.references()[0].revision, 1);
        assert_eq!(
            doc.tile_key(TileId(0)).unwrap().reference_revision,
            revision
        );
        doc.finish();
        assert_eq!(visible(&doc), "r\n\n");
    }
    for title in ["''", "\"\"", "()", "'first'"] {
        let doc = parse(&format!(
            "[r]\n\n[r]: /first {title}\n'second'\n\n[r]: /other\n'ignored'\n"
        ));
        assert_eq!(
            doc.link_title(LinkId(0)),
            Some(if title == "'first'" { "first" } else { "" })
        );
        assert_eq!(doc.link_destination(LinkId(0)), Some("/first"));
        assert_eq!(doc.references()[0].revision, 1);
        assert_eq!(visible(&doc), "r\n\n'second'\n\n");
        assert_eq!(doc.links().len(), 1);
    }
    let mut doc = Document::new();
    doc.append("[r]\n\n[r]: /first\n").unwrap();
    let info = *doc.link_info(LinkId(0)).unwrap();
    let update = doc.append("'winner'\n").unwrap();
    assert_eq!(update.references_changed, [SymbolId(0)]);
    assert!(update.new_links.is_empty());
    assert!(
        update
            .changes
            .iter()
            .any(|c| c.kind == ChangeKind::Reference && c.source.start == 0)
    );
    assert_eq!(doc.references()[0].revision, 2);
    let revision = doc.tile_key(TileId(0)).unwrap().reference_revision;
    let update = doc.append("[r]:\n/duplicate\n'ignored'\n").unwrap();
    assert!(update.references_changed.is_empty());
    assert_eq!(
        doc.tile_key(TileId(0)).unwrap().reference_revision,
        revision
    );
    assert_eq!(doc.references()[0].revision, 2);
    assert_eq!(doc.link_title(LinkId(0)), Some("winner"));
    assert_eq!(doc.link_info(LinkId(0)), Some(&info));
    assert_eq!(doc.links().len(), 1);
    doc.finish();
    assert_eq!(visible(&doc), "r\n\n");
    // A definition-shaped line inside an existing paragraph remains prose.
    for source in ["text\n[r]: /url\n'no'\n", "> text\n> [r]: /url\n> 'no'\n"] {
        let doc = parse(source);
        assert!(doc.references().iter().all(|r| r.destination.is_none()));
        assert!(
            !doc.blocks()
                .iter()
                .any(|b| matches!(b.kind, BlockKind::Definition { .. }))
        );
    }
}

#[test]
fn next_line_title_pending_fold_boundary_identity_and_real_view_updates() {
    for (first, next) in [
        ("", ""),
        ("> ", "> "),
        ("- - ", "\t"),
        (">\t", ">\t"),
        ("- > ", "\t>\t"),
    ] {
        for target_next in [false, true] {
            for success in [false, true] {
                let mut doc = Document::new();
                doc.append("[r]\n\n").unwrap();
                let info = *doc.link_info(LinkId(0)).unwrap();
                let head = if target_next {
                    format!("{first}[r]:\n{next}/url\n")
                } else {
                    format!("{first}[r]: /url\n")
                };
                let update = doc.append(&head).unwrap();
                let id = BlockId(update.new_blocks.start);
                let before = doc.block(id).unwrap().clone();
                assert_eq!(before.state, SyntaxState::Growing);
                assert_eq!(doc.link_destination(LinkId(0)), Some("/url"));
                assert_eq!(doc.link_title(LinkId(0)), None);
                assert_eq!(doc.source_fold_limit(), before.source.start);
                let mut view: FoldView = FoldView::new(&doc, 1).unwrap();
                assert!(SourceFoldRegistration::new(&doc, &view, before.source).is_err());
                assert!(SourceFoldRegistration::paragraph(&doc, &view, id, 0).is_err());
                let ticket = view.heights().ticket(TileId(0)).unwrap();
                view.measure(ticket, 7).unwrap();
                let start = doc.source().len() + next.len() + 5;
                let pending = doc.append(&format!("{next}    '中🙂&amp;'")).unwrap();
                assert!(pending.references_changed.is_empty());
                view.apply(&doc, &pending).unwrap();
                assert_eq!(doc.block(id), Some(&before));
                assert_eq!(doc.source_fold_limit(), before.source.start);
                let update = doc
                    .append(if success { "\r\n" } else { " bad\r\n" })
                    .unwrap();
                view.apply(&doc, &update).unwrap();
                assert!(view.measure(ticket, 7).is_err());
                assert_eq!(doc.block_at(before.source.start), Some(id));
                assert_eq!(doc.block(id).unwrap().container, before.container);
                assert_eq!(doc.block(id).unwrap().state, SyntaxState::Sealed);
                assert_eq!(doc.links().len(), 1);
                assert!(update.new_links.is_empty());
                assert_eq!(doc.link_info(LinkId(0)), Some(&info));
                assert_eq!(doc.link_destination(LinkId(0)), Some("/url"));
                if success {
                    assert_eq!(doc.block(id).unwrap().source.end, doc.source().len());
                    assert_eq!(
                        doc.references()[0].title,
                        Some(Span::new(start, start + "中🙂&amp;".len()))
                    );
                    assert_eq!(doc.link_title(LinkId(0)), Some("中🙂&amp;"));
                    assert_eq!(update.references_changed, [SymbolId(0)]);
                    assert_eq!(
                        doc.tile_key(TileId(0)).unwrap().reference_revision,
                        update.revision
                    );
                    assert_eq!(doc.source_fold_limit(), doc.source().len());
                    assert!(
                        update
                            .changes
                            .iter()
                            .any(|c| c.kind == ChangeKind::Structure
                                && c.source == doc.block(id).unwrap().source)
                    );
                    assert!(
                        update
                            .changes
                            .iter()
                            .any(|c| c.kind == ChangeKind::Reference
                                && c.source == Span::new(0, doc.source().len()))
                    );
                } else {
                    assert_eq!(doc.block(id).unwrap().source, before.source);
                    assert_eq!(doc.references()[0].title, None);
                    assert!(update.references_changed.is_empty());
                    assert_eq!(doc.source_fold_limit(), before.source.end);
                    assert!(visible(&doc).contains("'中🙂&amp;' bad"));
                }
                let update = doc.finish();
                view.apply(&doc, &update).unwrap();
                assert_eq!(doc.source_fold_limit(), doc.source().len());
                let mut fixed =
                    SourceFoldRegistration::new(&doc, &view, doc.block(id).unwrap().source)
                        .unwrap();
                assert!(
                    fixed
                        .advance(&doc, &mut view, &mut Budget::new(WorkLimit::unlimited()))
                        .unwrap()
                        .id
                        .is_some()
                );
            }
            let mut eof = Document::new();
            eof.append(&format!("{first}[r]: /url\n")).unwrap();
            assert_eq!(eof.source_fold_limit(), 0);
            let update = eof.finish();
            assert_eq!(eof.blocks()[0].state, SyntaxState::Sealed);
            assert!(update.changed_blocks.contains(&0));
            assert_eq!(eof.references()[0].revision, 1);
            assert_eq!(eof.source_fold_limit(), eof.source().len());
        }
    }
}

#[test]
fn next_line_title_utf8_splits_prefixes_and_eof_match_bulk() {
    for source in [
        "[r]\n\n[r]: /中🙂\n'中🙂 &amp; \\'t\\''\n",
        "[Straße 中🙂]:\r\n<my url>\r\n\t\"中🙂 &quot;\"\r\n\r\n[STRASSE 中🙂]\n",
        ">\t[r|s]:\n>\t  -|-\n>\t \"中🙂\"\n\n[r|s]\n",
        "- [r]:\r\n\t/url\r\n\t()",
        "- > [r]: /url\n\t>\t (a\\(b\\) &amp;)",
        "[r]: /url\n''",
        "[r]: /url\n\"\"\r\n",
        "[r]: /url\n'first\nsecond'\n",
        "[r]: /url\n\"中🙂\" bad\n",
        "[r]: /url\n(a(b))\n",
        "[r]: /url\n\n'no'\n",
        "[r]: /url ''\n'no'\n",
        "[r]: /first\n'yes'\n[r]: /other\n'no'\n[r]\n",
        "[r]: /url\nordinary\n---\n",
        "[r]: /url\n[s]: /s\n's title'\n\n[s]\n",
        "text\n[r]: /url\n'no'\n",
        "> [r]: /url\n'outside'\n",
        "> - [r]: /url\n> - 'sibling'\n",
        "- > [r]: /url\n  'missing quote'\n",
        "> > [r]: /url\n>\t>\t \"中🙂\"\r\n",
        "[r]: /url\n'[*x*](u) $m$ [^n] | `c`'\n",
    ] {
        let expected = support::snapshot(&parse(source));
        for at in 0..=source.len() {
            let mut doc = Utf8Document::new();
            doc.append(&source.as_bytes()[..at]).unwrap();
            doc.append(&source.as_bytes()[at..]).unwrap();
            doc.finish();
            assert_eq!(
                support::snapshot(doc.document()),
                expected,
                "{source:?}, {at}"
            );
        }
        let mut bytes = Utf8Document::new();
        for byte in source.as_bytes() {
            bytes.append(std::slice::from_ref(byte)).unwrap();
        }
        bytes.finish();
        assert_eq!(support::snapshot(bytes.document()), expected);
        let mut doc = Document::new();
        for (at, ch) in source.char_indices() {
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            let prefix = &source[..at + ch.len_utf8()];
            let mut bulk = Document::new();
            bulk.append(prefix).unwrap();
            assert_eq!(
                support::snapshot(&doc),
                support::snapshot(&bulk),
                "{prefix:?}"
            );
            assert_eq!(doc.pending(), bulk.pending());
            assert_eq!(doc.source_fold_limit(), bulk.source_fold_limit());
            assert_eq!(doc.work(), bulk.work());
            let mut eof = Document::new();
            for c in prefix.chars() {
                eof.append(c.encode_utf8(&mut [0; 4])).unwrap();
            }
            eof.finish();
            assert_eq!(
                support::snapshot(&eof),
                support::snapshot(&parse(prefix)),
                "{prefix:?}"
            );
        }
    }
}

#[test]
fn next_line_title_mapping_and_shared_display_destination_title_microbudget() {
    for (title, raw, decoded) in [
        ("''", "", ""),
        ("\"a\\\" &quot; 中🙂\"", "a\\\" &quot; 中🙂", "a\" \" 中🙂"),
        (
            "(a\\(b\\) &amp; &#10;)",
            "a\\(b\\) &amp; &#10;",
            "a(b) & \n",
        ),
        (
            "'&NotEqualTilde; \\&amp; &#x1F642;'",
            "&NotEqualTilde; \\&amp; &#x1F642;",
            "≂̸ &amp; 🙂",
        ),
    ] {
        let source = format!("[r]\n\n- > [r]:\r\n\t>\t<my url&amp;>\r\n\t>\t  {title}\r\n");
        let doc = parse(&source);
        let full = Span::new(0, source.len());
        let spans = [
            doc.references()[0].destination.unwrap(),
            doc.references()[0].title.unwrap(),
        ];
        assert_eq!(doc.text(spans[1]), Some(raw));
        assert_eq!(doc.link_title(LinkId(0)), Some(raw));
        assert_eq!(
            doc.runs_in(full)
                .unwrap()
                .map(|r| r.text)
                .collect::<String>(),
            source
        );
        let sliced: String = source
            .char_indices()
            .flat_map(|(at, ch)| {
                doc.display_runs_in(Span::new(at, at + ch.len_utf8()))
                    .unwrap()
            })
            .filter(|r| !r.style.contains(Style::HIDDEN))
            .map(|r| r.text)
            .collect();
        assert_eq!(sliced, visible(&doc));
        assert!(link_metadata::snapshot(&doc)[0].is_some());
        let mut display = doc.display_cursor(full).unwrap();
        let mut destination = doc.decoded_link_destination(LinkId(0)).unwrap();
        let mut title = doc.decoded_link_title(LinkId(0)).unwrap();
        let mut budget = Budget::new(WorkLimit::default());
        assert_eq!(
            display.next(&doc, &mut budget).unwrap(),
            DisplayStep::BudgetExhausted
        );
        assert_eq!(
            destination.advance(&mut budget),
            Err(Completion::BudgetExhausted)
        );
        if !raw.is_empty() {
            assert_eq!(title.advance(&mut budget), Err(Completion::BudgetExhausted));
        }
        assert_eq!(destination.source_position(), spans[0].start);
        assert_eq!(title.source_position(), spans[1].start);
        let mut budget = Budget::new(link_metadata::SIX_BYTES);
        let mut output = [String::new(), String::new(), String::new()];
        // One budget survives phase changes; only exhaustion renews it.
        for (phase, span) in [full, spans[0], spans[1]].into_iter().enumerate() {
            let mut end = span.start;
            for steps in 0..4 * source.len() {
                assert!(steps + 1 < 4 * source.len(), "cursor did not progress");
                let result = match phase {
                    0 if end == span.end => Ok(None),
                    0 => match display.next(&doc, &mut budget).unwrap() {
                        DisplayStep::Run(run) => Ok(Some((
                            run.source,
                            if run.style.contains(Style::HIDDEN) {
                                ""
                            } else {
                                run.text
                            },
                        ))),
                        DisplayStep::BudgetExhausted => Err(Completion::BudgetExhausted),
                        other => panic!("unexpected {other:?}"),
                    },
                    1 => destination
                        .advance(&mut budget)
                        .map(|chunk| chunk.map(|c| (c.source, c.text))),
                    _ => title
                        .advance(&mut budget)
                        .map(|chunk| chunk.map(|c| (c.source, c.text))),
                };
                match result {
                    Ok(Some((source, text))) => {
                        assert_eq!(source.start, end);
                        assert!(doc.text(source).is_some());
                        end = source.end;
                        output[phase].push_str(text);
                    }
                    Ok(None) => break,
                    Err(Completion::BudgetExhausted) => {
                        assert_ne!(budget.used(), WorkLimit::default());
                        budget = Budget::new(link_metadata::SIX_BYTES);
                    }
                    Err(other) => panic!("unexpected {other:?}"),
                }
                assert!(budget.used().bytes <= 6);
                assert!(budget.used().queries <= 1);
                assert!(budget.used().objects <= 1);
                assert_eq!(
                    match phase {
                        0 => display.source_position(),
                        1 => destination.source_position(),
                        _ => title.source_position(),
                    },
                    end
                );
            }
            assert_eq!(end, span.end);
        }
        assert_eq!(
            output,
            [visible(&doc), "my url&".to_owned(), decoded.to_owned()]
        );
    }
}

#[test]
fn next_line_title_bounded_work_window_and_failed_growth_never_retry() {
    for (first, next) in [("", ""), (">\t", ">\t"), ("- > ", "\t>\t")] {
        for padding in [0, 1, 3, 6] {
            for title in ["''", "\"中🙂\"", "(a\\(b\\))", "'bad' tail", "'unclosed"] {
                let source = format!("{first}[r]: /url\n{next}{}{title}\r\n", " ".repeat(padding));
                let mut doc = Document::new();
                for ch in source.chars() {
                    let before = doc.work();
                    doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
                    if ch != '\n' {
                        assert_eq!(doc.work().container_bytes, before.container_bytes);
                        assert_eq!(doc.work().container_steps, before.container_steps);
                        assert_eq!(doc.work().classified_bytes, before.classified_bytes);
                    }
                }
                doc.finish();
                assert_eq!(doc.work(), parse(&source).work());
                let n = source.len() as u64;
                assert!(doc.work().classified_bytes < 16 * n);
                assert!(doc.work().container_bytes < 24 * n);
                assert!(doc.work().container_steps < 12 * n);
            }
        }
        for length in [BLOCK_LOOKAHEAD - 1, BLOCK_LOOKAHEAD, BLOCK_LOOKAHEAD + 1] {
            for ending in ["\n", "\r\n", ""] {
                let source = format!(
                    "{first}[r]: /url\n{next}'{}'{ending}",
                    "x".repeat(length - next.len() - ending.len() - 2)
                );
                let doc = parse(&source);
                let accepted =
                    length <= BLOCK_LOOKAHEAD && (!ending.is_empty() || length < BLOCK_LOOKAHEAD);
                assert_eq!(
                    doc.references()[0].title.is_some(),
                    accepted,
                    "{first:?} {length} {ending:?}"
                );
                assert_eq!(
                    doc.references()[0].destination.and_then(|s| doc.text(s)),
                    Some("/url")
                );
                let mut bytes = Utf8Document::new();
                for byte in source.as_bytes() {
                    bytes.append(std::slice::from_ref(byte)).unwrap();
                }
                bytes.finish();
                assert_eq!(support::snapshot(bytes.document()), support::snapshot(&doc));
            }
        }
    }
    let quotes = ">".repeat(970);
    let mut doc = Document::new();
    doc.append(&format!(
        "{quotes} [r]: /url\n{quotes} '\u{4e2d}{}",
        "x".repeat(1_000_000)
    ))
    .unwrap();
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("x").unwrap();
    }
    assert_eq!(doc.work().classified_bytes, before.classified_bytes);
    assert_eq!(doc.work().container_bytes, before.container_bytes);
    assert_eq!(doc.work().container_steps, before.container_steps);
    assert_eq!(doc.work().label_bytes, before.label_bytes);
    assert_eq!(doc.work().inline_bytes - before.inline_bytes, 1000);
    doc.append("'\n").unwrap();
    doc.finish();
    assert_eq!(doc.references()[0].title, None);
    assert_eq!(
        doc.references()[0].destination.and_then(|s| doc.text(s)),
        Some("/url")
    );
    let n = doc.source().len() as u64;
    assert!(doc.work().classified_bytes < 16 * n);
    assert!(doc.work().container_bytes < 24 * n);
    assert!(doc.work().inline_bytes < 2 * n);
}

#[test]
fn next_line_title_confirmation_touches_current_symbol_and_incremental_ranges_only() {
    let mut doc = Document::new();
    for _ in 0..20_000 {
        doc.append("[r]\n\n").unwrap();
    }
    let infos: Vec<_> = (0..doc.links().len())
        .map(|i| *doc.link_info(LinkId(i)).unwrap())
        .collect();
    let quotes = ">".repeat(970);
    doc.append(&format!("{quotes} [r]:\n{quotes} /url\n"))
        .unwrap();
    let id = BlockId(doc.blocks().len() - 1);
    let scope = doc.block(id).unwrap().container;
    let containers = doc.container_count();
    let before = doc.work();
    let update = doc.append(&format!("{quotes} '中🙂'\n")).unwrap();
    let after = doc.work();
    assert_eq!(update.references_changed, [SymbolId(0)]);
    assert_eq!(doc.links().len(), 20_000);
    assert!(update.new_links.is_empty());
    assert_eq!(doc.container_count(), containers);
    assert_eq!(doc.block(id).unwrap().container, scope);
    assert_eq!(doc.references()[0].revision, 2);
    assert!(after.classified_bytes - before.classified_bytes < 4096);
    assert!(after.container_bytes - before.container_bytes < 16_000);
    assert!(after.container_steps - before.container_steps < 4096);
    assert_eq!(after.inline_bytes, before.inline_bytes);
    assert_eq!(after.label_bytes, before.label_bytes);
    assert_eq!(after.label_scalars, before.label_scalars);
    assert_eq!(after.table_header_bytes, before.table_header_bytes);
    assert_eq!(after.paragraph_closures, before.paragraph_closures);
    assert!(after.tree_visits - before.tree_visits < 256);
    assert!(update.changes.len() < 8);
    assert!(update.changed_links.len() <= 1);
    for (i, info) in infos.iter().enumerate() {
        assert_eq!(doc.link_info(LinkId(i)), Some(info));
        assert_eq!(doc.link_title(LinkId(i)), Some("中🙂"));
    }
    let before = doc.work();
    let revision = doc.tile_key(TileId(0)).unwrap().reference_revision;
    let update = doc
        .append(&format!("{quotes} [r]: /other\n{quotes} 'ignored'\n"))
        .unwrap();
    assert!(update.references_changed.is_empty());
    assert_eq!(
        doc.tile_key(TileId(0)).unwrap().reference_revision,
        revision
    );
    assert_eq!(doc.references()[0].revision, 2);
    assert_eq!(doc.links().len(), 20_000);
    assert!(doc.work().classified_bytes - before.classified_bytes < 8192);
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    doc.finish();
}
