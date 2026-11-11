use mdstream::*;

#[path = "support/snapshot.rs"]
mod support;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

#[test]
fn root_title_body_all_definition_entries_keep_raw_source() {
    for label in ["[r]:", "[r\n]:"] {
        for next_target in [false, true] {
            for independent in [false, true] {
                for (open, close) in [('\'', '\''), ('"', '"'), ('(', ')')] {
                    for newline in ["\n", "\r\n"] {
                        let gap = if next_target { newline } else { " " };
                        let title_gap = if independent { newline } else { " " };
                        let raw = format!("中🙂 &amp; \\*{newline}  tail");
                        let source = format!(
                            "{label}{gap}/url{title_gap}{open}{raw}{close}{newline}\n[r]\n"
                        );
                        let doc = parse(&source);
                        assert_eq!(doc.link_destination(LinkId(0)), Some("/url"), "{source:?}");
                        assert_eq!(doc.link_title(LinkId(0)), Some(raw.as_str()), "{source:?}");
                        let start = source.find(&raw).unwrap();
                        assert_eq!(
                            doc.references()[0].title,
                            Some(Span::new(start, start + raw.len()))
                        );
                        assert_eq!(doc.links().len(), 1);
                        assert_eq!(doc.blocks()[0].container, None);
                    }
                }
            }
        }
    }
}

#[test]
fn same_line_open_title_does_not_publish_retractable_definition_or_links() {
    let mut doc = Document::new();
    doc.append("[r]: /url 'first\n").unwrap();
    assert!(doc.links().is_empty());
    assert!(doc.references().iter().all(|r| r.destination.is_none()));
    assert_eq!(doc.source_fold_limit(), 0);
    assert_eq!(doc.blocks()[0].state, SyntaxState::Growing);
    doc.append("last'\n").unwrap();
    assert!(matches!(doc.blocks()[0].kind, BlockKind::Definition { .. }));
    assert_eq!(
        doc.references()[0].title.and_then(|s| doc.text(s)),
        Some("first\nlast")
    );
    assert_eq!(doc.blocks()[0].state, SyntaxState::Sealed);
}

#[test]
fn root_title_decoding_normalizes_crlf_with_original_byte_mapping() {
    let source = "[r]: /url '中🙂&amp;\\*\r\n  &NotEqualTilde;\nend'\n\n[r]\n";
    let doc = parse(source);
    let mut cursor = doc.decoded_link_title(LinkId(0)).expect("multiline title");
    let mut chunks = Vec::new();
    loop {
        match cursor.advance(&mut Budget::new(WorkLimit::unlimited())) {
            Ok(Some(c)) => chunks.push((c.source, c.text.to_owned())),
            Ok(None) => break,
            Err(e) => panic!("{e:?}"),
        }
    }
    let raw = doc.references()[0].title.unwrap();
    let mut at = raw.start;
    let mut expected = Vec::new();
    for (spelling, output) in [
        ("中", "中"),
        ("🙂", "🙂"),
        ("&amp;", "&"),
        ("\\*", "*"),
        ("\r\n", "\n"),
        (" ", " "),
        (" ", " "),
        ("&NotEqualTilde;", "≂̸"),
        ("\n", "\n"),
        ("e", "e"),
        ("n", "n"),
        ("d", "d"),
    ] {
        let span = Span::new(at, at + spelling.len());
        assert_eq!(doc.text(span), Some(spelling));
        expected.push((span, output.to_owned()));
        at = span.end;
    }
    assert_eq!(at, raw.end);
    assert_eq!(chunks, expected);
}

fn equivalence(source: &str) {
    let whole = parse(source);
    for split in 0..=source.len() {
        let mut doc = Utf8Document::new();
        doc.append(&source.as_bytes()[..split]).unwrap();
        doc.append(&source.as_bytes()[split..]).unwrap();
        doc.finish();
        assert_eq!(
            support::snapshot(doc.document()),
            support::snapshot(&whole),
            "split {split}: {source:?}"
        );
    }
    let mut doc = Document::new();
    for (at, ch) in source.char_indices() {
        doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
        let prefix = &source[..at + ch.len_utf8()];
        let mut bulk = Document::new();
        bulk.append(prefix).unwrap();
        assert_eq!(
            support::snapshot(&doc),
            support::snapshot(&bulk),
            "prefix {prefix:?}"
        );
        assert_eq!(doc.pending(), bulk.pending());
        let mut finished = Document::new();
        for ch in prefix.chars() {
            finished.append(ch.encode_utf8(&mut [0; 4])).unwrap();
        }
        finished.finish();
        bulk.finish();
        assert_eq!(support::snapshot(&finished), support::snapshot(&bulk));
    }
    doc.finish();
    assert_eq!(support::snapshot(&doc), support::snapshot(&whole));
}

#[test]
fn container_open_title_lazy_text_keeps_its_scope_and_link() {
    for prefix in ["> ", "- "] {
        let source = format!("{prefix}x\n[r]: /u 'a\nb'\n");
        let doc = parse(&source);
        let scope = doc.blocks()[0].container.unwrap();
        assert_eq!(doc.blocks().len(), 3, "{source:?}");
        assert!(doc.blocks().iter().all(|b| b.container == Some(scope)));
        assert_eq!(doc.block_at(4), Some(BlockId(1)));
        assert_eq!(doc.link_info(LinkId(0)).unwrap().source.start, 4);
        assert_eq!(doc.links(), [LinkTarget::Reference(SymbolId(0))]);
        assert_eq!(doc.references()[0].destination, None);
        assert_eq!(doc.references()[0].title, None);
        equivalence(&source);
    }
}

#[test]
fn container_open_title_partial_tab_keeps_replacement_mapping() {
    let source = ">\t[r]: /u '\n";
    let doc = parse(source);
    let tab = doc
        .display_runs_in(Span::new(1, 2))
        .unwrap()
        .next()
        .unwrap();
    assert_eq!(tab.source, Span::new(1, 2));
    assert_eq!(tab.replacement, Some(tab.source));
    assert_eq!(tab.text, "  ");
    assert!(!tab.style.contains(Style::HIDDEN));
    assert_eq!(doc.text(tab.source), Some("\t"));
    assert_eq!(doc.blocks()[0].container, Some(ContainerId(0)));
    assert_eq!(doc.links(), [LinkTarget::Reference(SymbolId(0))]);
    assert_eq!(doc.references()[0].destination, None);
    equivalence(source);
}

#[test]
fn container_open_title_nested_lazy_utf8_eof_and_shared_display_budget() {
    for (first, next) in [
        ("> ", ""),
        ("- ", ""),
        ("> - > ", "> "),
        ("- - ", "  "),
        ("> > ", ">\t"),
    ] {
        for (open, close) in [('x', 'x'), ('\'', '\''), ('"', '"'), ('(', ')')] {
            for newline in ["\n", "\r\n"] {
                // A non-delimiter is an ordinary-text control through the same path.
                let source =
                    format!("{first}x{newline}{next}[中]: /u {open}a{newline}🙂{close}{newline}");
                let doc = parse(&source);
                let scope = doc.blocks()[0].container;
                assert!(scope.is_some());
                assert!(
                    doc.blocks().iter().all(|b| b.container == scope),
                    "{source:?}"
                );
                assert!(doc.references().iter().all(|r| r.destination.is_none()));
                assert_eq!(doc.links(), [LinkTarget::Reference(SymbolId(0))]);
                equivalence(&source);
                let span = Span::new(0, source.len());
                let expected: String = doc
                    .display_runs_in(span)
                    .unwrap()
                    .filter(|r| !r.style.contains(Style::HIDDEN))
                    .map(|r| r.text)
                    .collect();
                let mut cursor = doc.display_cursor(span).unwrap();
                let mut text = String::new();
                let mut complete = false;
                for _ in 0..source.len() * 4 {
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
                    let mut budget = Budget::new(WorkLimit {
                        bytes: 6,
                        queries: 1,
                        objects: 1,
                        ..Default::default()
                    });
                    loop {
                        match cursor.next(&doc, &mut budget).unwrap() {
                            DisplayStep::Run(run) => {
                                if !run.style.contains(Style::HIDDEN) {
                                    text.push_str(run.text);
                                }
                            }
                            DisplayStep::BudgetExhausted => break,
                            DisplayStep::Complete => {
                                complete = true;
                                break;
                            }
                        }
                    }
                    assert!(budget.used().bytes <= 6);
                    assert!(budget.used().queries <= 1);
                    assert!(budget.used().objects <= 1);
                    if complete {
                        break;
                    }
                }
                assert!(complete);
                assert_eq!(text, expected);
            }
        }
    }
}

#[test]
fn container_open_title_work_keeps_old_ids_without_rescanning_history() {
    for first in ["> ", "- ", "> - > "] {
        let mut doc = Document::new();
        doc.append(&format!("{first}[old](/old)\n{}\n", "a".repeat(1_000_000)))
            .unwrap();
        let old = *doc.link_info(LinkId(0)).unwrap();
        let scope = doc.blocks()[0].container;
        let before = doc.work();
        let start = doc.source().len();
        for _ in 0..1000 {
            doc.append("[r]: /u 'a\n").unwrap();
        }
        doc.append("tail'\n").unwrap();
        doc.finish();
        let bytes = (doc.source().len() - start) as u64;
        let after = doc.work();
        assert!(after.classified_bytes - before.classified_bytes < 48 * bytes);
        assert!(after.container_bytes - before.container_bytes < 24 * bytes);
        assert!(after.container_steps - before.container_steps < 8 * 1001);
        assert_eq!(doc.link_info(LinkId(0)), Some(&old));
        assert_eq!(doc.block_at(0), Some(BlockId(0)));
        assert!(doc.blocks().iter().all(|b| b.container == scope));
        assert!(doc.references().iter().all(|r| r.destination.is_none()));
    }
}

#[test]
fn root_title_utf8_every_split_prefix_eof_and_delimiter_syntax() {
    for source in [
        "[Straße\n中🙂]:\r\n<my url> 'a\\'\r\n&ouml;\\*'\r\n\r\n[STRASSE 中🙂]\n",
        "[r]: /url\n(a\\(\nb\\))\n\n[r]\n",
        "[r]: /url \"\n中🙂\n\"\n\n[r]\n",
        "[r]: /url 'a\\\nb'\n",
        "[r]: /url 'first\nsecond' bad\n",
        "[r]: /url\n'first\nsecond' bad\n",
        "[r]: /url 'first\n\nlast'\n",
        "[r]: /url\n'first\n# h\nlast'\n",
        "[r]: /url 'x|y\n-|-\n",
        "[r]: /url\n'x|y\n-|-\n",
        "[r]: /url (a\n(b))\n",
        "> [r]: /url 'a\n> b'\n",
        "text\n[r]: /url 'a\nb'\n",
        "[r](/url 'a\nb')\n",
    ] {
        equivalence(source);
    }
    let unicode = parse("[Straße\n中🙂]:\r\n<my url> 'first\r\nlast'\r\n\r\n[STRASSE 中🙂]\n");
    assert_eq!(unicode.link_destination(LinkId(0)), Some("my url"));
    assert_eq!(unicode.link_title(LinkId(0)), Some("first\r\nlast"));
    for independent in [false, true] {
        let head = if independent {
            "[r]: /url\n'first\n"
        } else {
            "[r]: /url 'first\n"
        };
        let mut doc = Document::new();
        doc.append(head).unwrap();
        assert!(doc.links().is_empty());
        assert_eq!(
            doc.references().iter().any(|r| r.destination.is_some()),
            independent
        );
        assert_eq!(doc.source_fold_limit(), 0);
        doc.finish();
        assert_eq!(
            doc.references().iter().any(|r| r.destination.is_some()),
            independent
        );
        assert!(doc.references().iter().all(|r| r.title.is_none()));
        assert_eq!(doc.blocks()[0].state, SyntaxState::Sealed);
        assert_eq!(doc.source_fold_limit(), doc.source().len());
    }
    for (title, expected) in [
        ("'\n'", Some("\n")),
        ("' \n  x'", Some(" \n  x")),
        ("(a\\(\nb\\))", Some("a\\(\nb\\)")),
        ("(a\n(b))", None),
        ("'a\n' bad", None),
        ("'a\n  \nb'", None),
    ] {
        let source = format!("[r]: /url {title}\n\n[r]\n");
        let doc = parse(&source);
        assert_eq!(
            doc.link_title(LinkId(doc.links().len() - 1)),
            expected,
            "{source:?}"
        );
    }
}

#[test]
fn root_title_owners_duplicates_existing_titles_and_scope_boundaries() {
    for (head, expected_target, expected_title) in [
        ("[r]: /winner\n", "/winner", None),
        ("[r]: /winner ''\n", "/winner", Some("")),
        ("[r]: /winner 'yes'\n", "/winner", Some("yes")),
    ] {
        let doc = parse(&format!(
            "[r]\n\n{head}\n[r]: /duplicate 'a\nb'\n[r]: /another\n'c\nd'\n"
        ));
        assert_eq!(doc.link_destination(LinkId(0)), Some(expected_target));
        assert_eq!(doc.link_title(LinkId(0)), expected_title);
        assert_eq!(doc.links().len(), 1);
        assert_eq!(doc.references()[0].revision, 1);
    }
    for title in ["''", "\"\"", "()", "'first'"] {
        let doc = parse(&format!("[r]: /url {title}\n'a\nb'\n"));
        assert_eq!(
            doc.blocks()[0].source.end,
            format!("[r]: /url {title}\n").len()
        );
        assert_eq!(doc.references()[0].revision, 1);
    }
    for (first, next) in [
        ("> ", "> "),
        ("- ", "  "),
        ("> - ", ">   "),
        ("- > ", "  > "),
        (">\t", ">\t"),
    ] {
        for independent in [false, true] {
            let source = if independent {
                format!("{first}[r]: /url\n{next}'a\n{next}b'\n")
            } else {
                format!("{first}[r]: /url 'a\n{next}b'\n")
            };
            let doc = parse(&source);
            assert_eq!(
                doc.references()
                    .iter()
                    .filter(|r| r.destination.is_some())
                    .count(),
                usize::from(independent || next != ">\t"),
                "{source:?}"
            );
            let expected = (next != ">\t").then(|| format!("a\n{next}b"));
            assert_eq!(
                doc.references()[0].title.and_then(|s| doc.text(s)),
                expected.as_deref()
            );
            assert_eq!(
                doc.container_count(),
                parse(&source.replace("'a", "xa")).container_count()
            );
        }
    }
    for source in [
        "text\n[r]: /url 'a\nb'\n",
        "[r](/url 'a\nb')\n",
        "![r](/url 'a\nb')\n",
    ] {
        let doc = parse(source);
        assert!(
            !doc.blocks()
                .iter()
                .any(|b| matches!(b.kind, BlockKind::Definition { .. }))
        );
        assert!(
            doc.links()
                .iter()
                .enumerate()
                .all(|(i, _)| doc.link_title(LinkId(i)).is_none())
        );
    }
}

#[test]
fn root_title_limits_count_title_bytes_separately_from_physical_windows_and_label_fragments() {
    for independent in [false, true] {
        for length in [4095, 4096, 4097] {
            // More than 1002 nonblank physical fragments, still a legal title.
            let raw = format!(
                "a\n{}{}",
                "x\n".repeat((length - 2) / 2),
                if length % 2 == 1 { "x" } else { "" }
            );
            assert_eq!(raw.len(), length);
            let gap = if independent { "\n" } else { " " };
            let source = format!("[r]: /url{gap}'{raw}'\n\n[r]\n");
            let doc = parse(&source);
            let id = LinkId(doc.links().len() - 1);
            assert_eq!(
                doc.link_title(id),
                (length <= MAX_LINK_TITLE_BYTES).then_some(raw.as_str()),
                "independent={independent} length={length}"
            );
            assert_eq!(
                doc.link_destination(id).is_some(),
                independent || length <= MAX_LINK_TITLE_BYTES
            );
            assert!(doc.work().classified_bytes < 18 * source.len() as u64);
            eprintln!(
                "title fragments independent={independent} len={length}: {:?}",
                doc.work()
            );
        }
        for width in [BLOCK_LOOKAHEAD - 1, BLOCK_LOOKAHEAD, BLOCK_LOOKAHEAD + 1] {
            let gap = if independent { "\n" } else { " " };
            let source = format!("[r]: /url{gap}'a\n{}\nb'\n\n[r]\n", "x".repeat(width - 1));
            let doc = parse(&source);
            let id = LinkId(doc.links().len() - 1);
            assert_eq!(
                doc.link_title(id).is_some(),
                width <= BLOCK_LOOKAHEAD,
                "width={width}"
            );
        }
        let label = format!("a\n{}z", "x\n".repeat(498));
        assert_eq!(label.len(), 999);
        let gap = if independent { "\n" } else { " " };
        let raw = format!("a\n{}", "x\n".repeat(2047));
        let source = format!("[{label}]:\n/url{gap}'{raw}'\n");
        let doc = parse(&source);
        assert_eq!(doc.references().len(), 1);
        assert_eq!(
            doc.references()[0].title.and_then(|s| doc.text(s)),
            Some(raw.as_str())
        );
    }
}

#[test]
fn root_title_shared_zero_six_byte_budgets_and_raw_display_slices() {
    let source = "[r]\n\n[r]:\r\n<中&amp;\\*>\r\n'🙂&amp;\\*\r\n&NotEqualTilde;'\r\n";
    let doc = parse(source);
    let full = Span::new(0, source.len());
    let mut raw = String::new();
    let mut shown = String::new();
    for (at, ch) in source.char_indices() {
        let span = Span::new(at, at + ch.len_utf8());
        for run in doc.runs_in(span).unwrap() {
            assert_eq!(doc.text(run.source), Some(run.text));
            raw.push_str(run.text);
        }
        for run in doc.display_runs_in(span).unwrap() {
            assert_eq!(doc.text(run.source), Some(run.text));
            if !run.style.contains(Style::HIDDEN) {
                shown.push_str(run.text);
            }
        }
    }
    assert_eq!(raw, source);
    let expected: String = doc
        .display_runs_in(full)
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    assert_eq!(shown, expected);
    let mut display = doc.display_cursor(full).unwrap();
    let mut dest = doc.decoded_link_destination(LinkId(0)).unwrap();
    let mut title = doc.decoded_link_title(LinkId(0)).unwrap();
    let (mut d, mut t, mut s) = (String::new(), String::new(), String::new());
    let (mut display_done, mut dest_done, mut title_done) = (false, false, false);
    let mut steps = 0;
    while !display_done || !dest_done || !title_done {
        steps += 1;
        assert!(steps < source.len() * 15);
        let (dp, tp, sp) = (
            dest.source_position(),
            title.source_position(),
            display.source_position(),
        );
        let mut zero = Budget::new(WorkLimit::default());
        if !dest_done {
            assert_eq!(dest.advance(&mut zero), Err(Completion::BudgetExhausted));
        }
        if !title_done {
            assert_eq!(title.advance(&mut zero), Err(Completion::BudgetExhausted));
        }
        if !display_done {
            assert_eq!(
                display.next(&doc, &mut zero).unwrap(),
                DisplayStep::BudgetExhausted
            );
        }
        assert_eq!(
            (
                dest.source_position(),
                title.source_position(),
                display.source_position()
            ),
            (dp, tp, sp)
        );
        let mut budget = Budget::new(WorkLimit {
            bytes: 6,
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        if !display_done {
            match display.next(&doc, &mut budget).unwrap() {
                DisplayStep::Run(run) => {
                    if !run.style.contains(Style::HIDDEN) {
                        s.push_str(run.text);
                    }
                }
                DisplayStep::Complete => display_done = true,
                DisplayStep::BudgetExhausted => (),
            }
        }
        if !dest_done {
            match dest.advance(&mut budget) {
                Ok(Some(c)) => {
                    assert!(doc.text(c.source).is_some());
                    d.push_str(c.text);
                }
                Ok(None) => dest_done = true,
                Err(Completion::BudgetExhausted) => (),
                Err(e) => panic!("{e:?}"),
            }
        }
        if !title_done {
            match title.advance(&mut budget) {
                Ok(Some(c)) => {
                    assert!(doc.text(c.source).is_some());
                    t.push_str(c.text);
                }
                Ok(None) => title_done = true,
                Err(Completion::BudgetExhausted) => (),
                Err(e) => panic!("{e:?}"),
            }
        }
        let used = budget.used();
        assert!(used.bytes <= 6 && used.queries <= 1 && used.objects <= 1);
        // Completed cursors report completion for free, even in a zero frame.
        dest_done |= dest.source_position() == doc.references()[0].destination.unwrap().end;
        title_done |= title.source_position() == doc.references()[0].title.unwrap().end;
        display_done |= display.source_position() == source.len();
    }
    assert_eq!(s, expected);
    assert_eq!(d, "中&*");
    assert_eq!(t, "🙂&*\n≂̸");
    assert_eq!(
        title.advance(&mut Budget::new(WorkLimit::default())),
        Ok(None)
    );
}

#[test]
fn root_title_real_fold_limits_updates_merge_and_stable_ids() {
    for independent in [false, true] {
        for ending in ["end'\n", "end' bad\n", "\n", ""] {
            let mut doc = Document::new();
            doc.append("[old](/old) [r]\n\n").unwrap();
            let infos: Vec<_> = (0..2).map(|i| *doc.link_info(LinkId(i)).unwrap()).collect();
            let start = doc.source().len();
            let first = if independent {
                "[r]: /url\n'first\r\n"
            } else {
                "[r]: /url 'first\r\n"
            };
            let update = doc.append(first).unwrap();
            let id = BlockId(update.new_blocks.start);
            let mut view = FoldView::new(&doc, 1).unwrap();
            let mut batched = FoldView::new(&doc, 1).unwrap();
            let mut rebuilt = doc.blocks().to_vec();
            assert_eq!(doc.block_at(start), Some(id));
            assert_eq!(doc.block(id).unwrap().state, SyntaxState::Growing);
            assert_eq!(doc.source_fold_limit(), start);
            assert_eq!(doc.container_count(), 0);
            assert!(
                SourceFoldRegistration::new(&doc, &view, doc.block(id).unwrap().source).is_err()
            );
            let open_fold = if independent {
                None
            } else {
                let mut reg = SourceFoldRegistration::open_paragraph(&doc, &view, id, 0).unwrap();
                let fold = reg
                    .advance(&doc, &mut view, &mut Budget::new(WorkLimit::unlimited()))
                    .unwrap()
                    .id
                    .unwrap();
                view.set_collapsed(&doc, fold, true).unwrap();
                let mut reg =
                    SourceFoldRegistration::open_paragraph(&doc, &batched, id, 0).unwrap();
                let other = reg
                    .advance(&doc, &mut batched, &mut Budget::new(WorkLimit::unlimited()))
                    .unwrap()
                    .id
                    .unwrap();
                batched.set_collapsed(&doc, other, true).unwrap();
                Some((fold, other))
            };
            let ticket = view.heights().ticket(TileId(0)).unwrap();
            view.measure(ticket, 3).unwrap();
            let batched_ticket = batched.heights().ticket(TileId(0)).unwrap();
            batched.measure(batched_ticket, 3).unwrap();
            let mut merged = Update::default();
            for chunk in ["middle\n", ending] {
                let update = doc.append(chunk).unwrap();
                rebuilt.extend_from_slice(&doc.blocks()[update.new_blocks.clone()]);
                for b in update.changed_blocks.clone() {
                    rebuilt[b] = doc.blocks()[b].clone();
                }
                assert_eq!(rebuilt, doc.blocks());
                view.apply(&doc, &update).unwrap();
                merged.merge(update).unwrap();
                assert_eq!(doc.block_at(start), Some(id));
                for (i, info) in infos.iter().enumerate() {
                    assert_eq!(doc.link_info(LinkId(i)), Some(info));
                }
            }
            let update = doc.finish();
            rebuilt.extend_from_slice(&doc.blocks()[update.new_blocks.clone()]);
            for b in update.changed_blocks.clone() {
                rebuilt[b] = doc.blocks()[b].clone();
            }
            assert_eq!(rebuilt, doc.blocks());
            view.apply(&doc, &update).unwrap();
            merged.merge(update).unwrap();
            batched.apply(&doc, &merged).unwrap();
            assert_eq!(doc.source_fold_limit(), doc.source().len());
            assert_eq!(doc.block(id).unwrap().state, SyntaxState::Sealed);
            assert!(view.measure(ticket, 3).is_err());
            let success = ending == "end'\n";
            assert_eq!(
                doc.link_title(LinkId(1)),
                success.then_some("first\r\nmiddle\nend")
            );
            assert_eq!(
                doc.link_destination(LinkId(1)).is_some(),
                independent || success
            );
            if let Some((fold, other)) = open_fold {
                if success {
                    assert!(view.fold_candidate(&doc, fold).unwrap().is_none());
                    assert!(batched.fold_candidate(&doc, other).unwrap().is_none());
                } else {
                    if let FoldId::Source(source) = fold {
                        view.remove_source_fold(&doc, source).unwrap();
                    }
                    if let FoldId::Source(source) = other {
                        batched.remove_source_fold(&doc, source).unwrap();
                    }
                }
            }
            assert_eq!(view.heights().total_rows(), batched.heights().total_rows());
            for at in (0..doc.source().len()).filter(|&at| doc.source().is_char_boundary(at)) {
                assert_eq!(
                    view.hidden_at(&doc, at, &mut Budget::new(WorkLimit::unlimited()))
                        .unwrap(),
                    batched
                        .hidden_at(&doc, at, &mut Budget::new(WorkLimit::unlimited()))
                        .unwrap()
                );
            }
            let mut fixed =
                SourceFoldRegistration::new(&doc, &view, doc.block(id).unwrap().source).unwrap();
            assert!(
                fixed
                    .advance(&doc, &mut view, &mut Budget::new(WorkLimit::unlimited()))
                    .unwrap()
                    .id
                    .is_some()
            );
            assert!(doc.finish().changes.is_empty());
        }
    }
}

#[test]
fn root_title_work_is_linear_on_long_tails_and_independent_of_old_uses() {
    for independent in [false, true] {
        let mut doc = Document::new();
        let opener = if independent {
            "[r]: /url\n'first\n"
        } else {
            "[r]: /url 'first\n"
        };
        doc.append(opener).unwrap();
        for _ in 0..100 {
            doc.append("x\n").unwrap();
        }
        assert_eq!(doc.work().inline_bytes, 0);
        doc.append(&"x".repeat(1_000_000)).unwrap();
        let before = doc.work();
        for _ in 0..1000 {
            doc.append("x").unwrap();
        }
        let after = doc.work();
        assert_eq!(after.inline_bytes - before.inline_bytes, 1000);
        assert_eq!(after.classified_bytes, before.classified_bytes);
        assert_eq!(after.container_bytes, before.container_bytes);
        assert_eq!(after.container_steps, before.container_steps);
        assert!(after.inline_bytes < 2 * doc.source().len() as u64);
        assert!(after.classified_bytes < 18 * doc.source().len() as u64);
        eprintln!(
            "root title million tail independent={independent}: before={before:?}, after={after:?}"
        );
        doc.finish();
    }
    for independent in [false, true] {
        let mut doc = Document::new();
        doc.append(&format!("{}\n\n", "[x][r] ".repeat(20_000)))
            .unwrap();
        let infos: Vec<_> = (0..20_000)
            .map(|i| *doc.link_info(LinkId(i)).unwrap())
            .collect();
        let before = doc.work();
        let gap = if independent { "\n" } else { " " };
        let update = doc
            .append(&format!("[R]: /late{gap}'first\nlast'\n"))
            .unwrap();
        assert_eq!(doc.work().inline_bytes, before.inline_bytes);
        assert!(doc.work().tree_visits - before.tree_visits < 200);
        assert_eq!(
            update.references_changed.len(),
            if independent { 2 } else { 1 }
        );
        assert!(update.new_links.is_empty());
        assert!(update.changes.len() < 16);
        for (i, info) in infos.iter().enumerate() {
            assert_eq!(doc.link_info(LinkId(i)), Some(info));
        }
        assert_eq!(doc.link_title(LinkId(19_999)), Some("first\nlast"));
        for i in 0..4096 {
            doc.append(&format!("[n{i}]: /url 'a\nb'\n")).unwrap();
        }
        let before = doc.work();
        let update = doc.append("[r]: /ignored\n'c\nd'\n").unwrap();
        assert!(update.references_changed.is_empty());
        assert_eq!(doc.link_destination(LinkId(0)), Some("/late"));
        assert_eq!(doc.link_title(LinkId(0)), Some("first\nlast"));
        assert_eq!(doc.work().inline_bytes, before.inline_bytes);
        assert!(doc.work().tree_visits - before.tree_visits < 200);
        eprintln!(
            "root title 20k uses and 4096 definitions independent={independent}: before={before:?}, after={:?}",
            doc.work()
        );
    }
}
