use mdstream::*;

#[path = "support/snapshot.rs"]
mod support;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn decoded(doc: &Document, id: LinkId) -> String {
    let mut cursor = doc.decoded_link_title(id).expect("resolved title");
    let mut text = String::new();
    while let Some(chunk) = cursor
        .advance(&mut Budget::new(WorkLimit::unlimited()))
        .unwrap()
    {
        text.push_str(chunk.text);
    }
    text
}

#[test]
fn container_title_entries_keep_raw_envelope_and_root_equivalent_semantics() {
    let mut cases = 0;
    for (first, next) in [
        ("> ", "> "),
        ("- ", "  "),
        ("> - ", ">   "),
        ("- > ", "  > "),
        ("- - ", "\t"), // fully consumed tab, no virtual remainder
        ("> > ", ">\t> "),
        (">\t", "> "), // opening line keeps its existing partial tab ability
    ] {
        for multiline_label in [false, true] {
            for next_target in [false, true] {
                for independent in [false, true] {
                    for (open, close) in [('\'', '\''), ('"', '"'), ('(', ')')] {
                        for newline in ["\n", "\r\n"] {
                            for close_only in [false, true] {
                                let label = if multiline_label {
                                    format!("[Straße{newline}{next}中🙂]:")
                                } else {
                                    "[r]:".to_owned()
                                };
                                let target_gap = if next_target {
                                    format!("{newline}{next}")
                                } else {
                                    " ".into()
                                };
                                let title_gap = if independent {
                                    format!("{newline}{next}\t")
                                } else {
                                    " ".into()
                                };
                                let last = if close_only { "" } else { "  tail" };
                                let raw = format!("中🙂 &amp; \\*{newline}{next}{last}");
                                let usage = if multiline_label {
                                    "STRASSE 中🙂"
                                } else {
                                    "r"
                                };
                                let source = format!(
                                    "{first}{label}{target_gap}<my url>{title_gap}{open}{raw}{close}{newline}\n[{usage}]\n"
                                );
                                let doc = parse(&source);
                                assert_eq!(
                                    doc.link_destination(LinkId(0)),
                                    Some("my url"),
                                    "{source:?}"
                                );
                                assert_eq!(
                                    doc.link_title(LinkId(0)),
                                    Some(raw.as_str()),
                                    "{source:?}"
                                );
                                let start = source.find(&raw).unwrap();
                                assert_eq!(
                                    doc.references()[0].title,
                                    Some(Span::new(start, start + raw.len()))
                                );
                                let root = parse(&format!(
                                    "[r]: <my url> {open}中🙂 &amp; \\*{newline}{last}{close}{newline}\n[r]\n"
                                ));
                                assert_eq!(
                                    decoded(&doc, LinkId(0)),
                                    decoded(&root, LinkId(0)),
                                    "{source:?}"
                                );
                                assert_eq!(doc.links().len(), 1);
                                assert!(doc.blocks()[0].container.is_some());
                                assert_eq!(
                                    doc.references()[0].revision,
                                    if independent { 2 } else { 1 }
                                );
                                assert!(
                                    doc.tables().is_empty()
                                        && doc.maths().is_empty()
                                        && doc.footnotes().is_empty()
                                );
                                cases += 1;
                            }
                        }
                    }
                }
            }
        }
    }
    assert_eq!(cases, 672);
}

#[test]
fn container_title_source_chunks_exclude_only_known_prefixes() {
    let source = "[r]\n\n> - [r]: /u '中&#13;&#10;\r\n>   \\*&amp;\n>   '\n";
    let doc = parse(source);
    let mut cursor = doc.decoded_link_title(LinkId(0)).unwrap();
    let mut chunks = Vec::new();
    while let Some(chunk) = cursor
        .advance(&mut Budget::new(WorkLimit::unlimited()))
        .unwrap()
    {
        chunks.push((
            doc.text(chunk.source).unwrap().to_owned(),
            chunk.text.to_owned(),
        ));
    }
    assert_eq!(
        chunks,
        [
            ("中", "中"),
            ("&#13;", "\r"),
            ("&#10;", "\n"),
            ("\r\n", "\n"),
            ("\\*", "*"),
            ("&amp;", "&"),
            ("\n", "\n")
        ]
        .map(|(a, b)| (a.to_owned(), b.to_owned()))
    );
    assert_eq!(
        cursor.source_position(),
        doc.references()[0].title.unwrap().end
    );
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
        assert_eq!(doc.document().work(), whole.work());
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
            "{prefix:?}"
        );
        assert_eq!(doc.pending(), bulk.pending());
        assert_eq!(doc.work(), bulk.work());
        assert_eq!(doc.source_fold_limit(), bulk.source_fold_limit());
        let mut eof = Document::new();
        for c in prefix.chars() {
            eof.append(c.encode_utf8(&mut [0; 4])).unwrap();
        }
        eof.finish();
        bulk.finish();
        assert_eq!(
            support::snapshot(&eof),
            support::snapshot(&bulk),
            "EOF {prefix:?}"
        );
    }
    doc.finish();
    assert_eq!(support::snapshot(&doc), support::snapshot(&whole));
}

#[test]
fn container_title_utf8_splits_prefix_eof_and_failure_boundaries() {
    for source in [
        "> [r]: /中🙂 'a\\'\r\n> 🙂&amp;\r\n> '\r\n\n[r]\n",
        "- [r]: /u\n  (a\\(\n  🙂\\))\n\n[r]\n",
        "> - [Straße\r\n>   中🙂]:\r\n>   <my url>\r\n>   \t\"中🙂&quot;\r\n>   tail\"\r\n\n[STRASSE 中🙂]\n",
        "- - [r]:\n\t/u '中\n\t🙂'",
        "> > [r]: /u 'a\n>\t> b'\n",
        ">\t[r]: /u 'a\n> b'\n",
        "> [r]: /u\n>\t'a\n> b'\n",
        "> [r]:\n>\t/u 'a\n> b'\n",
        "> [r\n>\t中]: /u 'a\n> b'\n",
        "> [r]: /u 'a\n>\tb'\n", // partial remainder is structural
        "> [r]: /u\n> 'a\n>\tb'\n",
        "> [r]: /u \"a\\\"\r\n>\t中&quot;\\\"\"\r\n",
        "1. [r]: /u\n\t(a\\(\n\t🙂\\))\n",
        "-    [r]: /u (a\n\t\tbad(\n",
        "> x\n[r]: /u 'a\nb'\n", // task18 lazy qualification
        ">\t[r]: /u '\n",        // task18 partial-tab ordinary mapping
        "> - [r]: /u 'a\n> - sibling'\n",
        "> [r]: /u\n> 'a\n> b' bad\n",
        "> [r]: /u 'a|b\n> -|-\n",
        "> [r]: /u 'a\n> b|c\n> -|-\n",
        "> [r]: /u 'a\n> \t\n> b'\n",
        "> [r]: /u 'a\n> # h\n",
        "> [r]: /u 'a\n> ===\n",
        "> [r]: /u 'a\n> ~~~\n",
        "- > [r]: /u 'a\n  missing quote'\n",
    ] {
        equivalence(source);
    }
    for &(first, zero, partial, _) in PARTIAL_PATHS {
        for independent in [false, true] {
            for ending in ["🙂&amp;\\*'\r\n", "' bad\n", "", "\n", "# h\n", "a|b\n"] {
                let gap = if independent {
                    format!("\n{zero}")
                } else {
                    " ".into()
                };
                let source =
                    format!("[old](/old) [r]\n\n{first}[r]: /中🙂{gap}'a\r\n{partial}{ending}");
                equivalence(&source);
            }
        }
    }
    for source in [
        "> [r]: /u\n>\t'a\n> b'\n\n[r]\n",
        "> [r]:\n>\t/u 'a\n> b'\n\n[r]\n",
        "> [r\n>\t中]: /u 'a\n> b'\n\n[r 中]\n",
    ] {
        let doc = parse(source);
        assert_eq!(doc.link_destination(LinkId(0)), Some("/u"), "{source:?}");
        assert_eq!(decoded(&doc, LinkId(0)), "a\nb", "{source:?}");
    }
}

#[test]
fn container_title_ownership_and_published_identity_versions() {
    for (first, next) in [("> ", "> "), ("- ", "  "), ("> - > ", ">   > ")]
        .into_iter()
        .chain(
            PARTIAL_PATHS
                .iter()
                .map(|&(first, _, next, _)| (first, next)),
        )
    {
        for independent in [false, true] {
            for ending in ["end'\n", "end' bad\n", "\n", ""] {
                let mut doc = Document::new();
                doc.append("[old](/old) [r]\n\n").unwrap();
                let infos: Vec<_> = (0..2).map(|i| *doc.link_info(LinkId(i)).unwrap()).collect();
                let gap = if independent {
                    format!("\n{next}")
                } else {
                    " ".into()
                };
                let update = doc
                    .append(&format!("{first}[r]: /u{gap}'first\r\n"))
                    .unwrap();
                let id = BlockId(update.new_blocks.start);
                let block = doc.block(id).unwrap().clone();
                let containers: Vec<_> = (0..doc.container_count())
                    .map(|i| doc.container(ContainerId(i)).unwrap())
                    .collect();
                let LinkTarget::Reference(symbol) = doc.links()[1] else {
                    panic!("reference use")
                };
                let reference = doc.references()[symbol.0].clone();
                let revision = doc.tile_key(TileId(0)).unwrap().reference_revision;
                assert_eq!(doc.source_fold_limit(), block.source.start);
                assert_eq!(block.state, SyntaxState::Growing);
                assert_eq!(doc.link_destination(LinkId(1)).is_some(), independent);
                assert_eq!(doc.link_title(LinkId(1)), None);
                let mut changed = Update::default();
                changed
                    .merge(doc.append(&format!("{next}middle\n")).unwrap())
                    .unwrap();
                assert_eq!(doc.references()[symbol.0], reference);
                assert_eq!(
                    doc.tile_key(TileId(0)).unwrap().reference_revision,
                    revision
                );
                if !ending.is_empty() {
                    changed
                        .merge(doc.append(&format!("{next}{ending}")).unwrap())
                        .unwrap();
                }
                changed.merge(doc.finish()).unwrap();
                let success = ending == "end'\n";
                assert_eq!(changed.references_changed.len(), usize::from(success));
                assert_eq!(
                    doc.references()[symbol.0].revision,
                    u64::from(independent) + u64::from(success)
                );
                assert_eq!(
                    doc.link_destination(LinkId(1)).is_some(),
                    independent || success
                );
                assert_eq!(
                    doc.link_title(LinkId(1)),
                    success
                        .then(|| format!("first\r\n{next}middle\n{next}end"))
                        .as_deref()
                );
                if success {
                    assert_eq!(decoded(&doc, LinkId(1)), "first\nmiddle\nend");
                }
                for (i, info) in infos.iter().enumerate() {
                    assert_eq!(doc.link_info(LinkId(i)), Some(info));
                }
                assert_eq!(doc.block(id).unwrap().container, block.container);
                for (i, old) in containers.iter().enumerate() {
                    let now = doc.container(ContainerId(i)).unwrap();
                    assert_eq!(
                        (now.parent, now.depth, &now.kind, now.source.start),
                        (old.parent, old.depth, &old.kind, old.source.start)
                    );
                }
            }
        }
        for winner in ["/winner", "/winner ''", "/winner 'yes'"] {
            let source = format!(
                "[r]\n\n{first}[r]: {winner}\n\n{first}[r]: /other 'a\n{next}b'\n\n{first}[r]: /other\n{next}'c\n{next}d'\n"
            );
            let doc = parse(&source);
            assert_eq!(doc.link_destination(LinkId(0)), Some("/winner"));
            assert_eq!(
                doc.link_title(LinkId(0)),
                if winner.ends_with("''") {
                    Some("")
                } else if winner.ends_with("'yes'") {
                    Some("yes")
                } else {
                    None
                }
            );
            assert_eq!(doc.references()[0].revision, 1);
        }
    }
}

#[test]
fn container_title_gap_mapping_and_shared_microbudgets() {
    let quotes = ">".repeat(900);
    let source = format!(
        "[r]\n\n{quotes} [r]: /中&amp;\\*\r\n{quotes} '🙂&#13;&#10;\r\n{quotes}   &NotEqualTilde;\\*\n{quotes} '\n"
    );
    assert_gap_mapping_and_shared_microbudgets(&source, "🙂\r\n\n  ≂̸*\n");
    for (depth, indent) in [(900, ""), (901, " "), (902, "  ")] {
        let quotes = ">".repeat(depth);
        let source = format!(
            "[r]\n\n{quotes} [r]: /中&amp;\\*\r\n{quotes} '🙂&#13;&#10;\r\n{quotes}\t{indent}&NotEqualTilde;\\*\n{quotes}\t'\n"
        );
        assert_gap_mapping_and_shared_microbudgets(&source, &format!("🙂\r\n\n{indent}≂̸*\n"));
    }
}

fn assert_gap_mapping_and_shared_microbudgets(source: &str, expected_title: &str) {
    let doc = parse(source);
    let full = Span::new(0, source.len());
    let raw: String = doc.runs_in(full).unwrap().map(|r| r.text).collect();
    assert_eq!(raw, source);
    let expected: String = doc
        .display_runs_in(full)
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    let sliced: String = source
        .char_indices()
        .flat_map(|(at, c)| {
            doc.display_runs_in(Span::new(at, at + c.len_utf8()))
                .unwrap()
        })
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    assert_eq!(sliced, expected);
    let mut title = doc.decoded_link_title(LinkId(0)).unwrap();
    let mut destination = doc.decoded_link_destination(LinkId(0)).unwrap();
    let mut display = doc.display_cursor(full).unwrap();
    let (mut t, mut d, mut s) = (String::new(), String::new(), String::new());
    let (mut td, mut dd, mut sd) = (false, false, false);
    let mut gap_steps = 0;
    let mut saw_crlf = false;
    for step in 0..source.len() * 10 {
        let at = title.source_position();
        if !td {
            for limit in [
                WorkLimit::default(),
                WorkLimit {
                    bytes: 6,
                    objects: 1,
                    ..Default::default()
                },
                WorkLimit {
                    bytes: 6,
                    queries: 1,
                    ..Default::default()
                },
            ] {
                let mut budget = Budget::new(limit);
                assert_eq!(title.advance(&mut budget), Err(Completion::BudgetExhausted));
                assert_eq!(title.source_position(), at);
                assert_eq!(budget.used(), WorkLimit::default());
            }
        }
        // One-byte frames can skip long prefixes incrementally. Six-byte
        // frames ensure that scalars, entities and physical CRLF can emit.
        let bytes = if step % 2 == 0 { 1 } else { 6 };
        let mut budget = Budget::new(WorkLimit {
            bytes,
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        if !td {
            match title.advance(&mut budget) {
                Ok(Some(c)) => {
                    assert!(c.source.start >= at);
                    assert!(doc.text(c.source).is_some());
                    if doc.text(c.source) == Some("\r\n") {
                        assert_eq!(c.text, "\n");
                        saw_crlf = true;
                    }
                    t.push_str(c.text);
                }
                Ok(None) => td = true,
                Err(Completion::BudgetExhausted) => {
                    if title.source_position() > at {
                        gap_steps += 1;
                        assert!(title.source_position() - at <= bytes);
                    }
                    // A one-byte CRLF frame cannot swallow only its CR.
                    if source[at..].starts_with("\r\n") {
                        assert_eq!(title.source_position(), at);
                    }
                }
                Err(e) => panic!("{e:?}"),
            }
        }
        if !dd {
            match destination.advance(&mut budget) {
                Ok(Some(c)) => d.push_str(c.text),
                Ok(None) => dd = true,
                Err(Completion::BudgetExhausted) => (),
                Err(e) => panic!("{e:?}"),
            }
        }
        if !sd {
            match display.next(&doc, &mut budget).unwrap() {
                DisplayStep::Run(r) => {
                    if !r.style.contains(Style::HIDDEN) {
                        s.push_str(r.text);
                    }
                }
                DisplayStep::Complete => sd = true,
                DisplayStep::BudgetExhausted => (),
            }
        }
        assert!(
            budget.used().bytes <= bytes
                && budget.used().queries <= 1
                && budget.used().objects <= 1
        );
        if td && dd && sd {
            break;
        }
    }
    assert!(td && dd && sd && saw_crlf);
    assert!(gap_steps > 400);
    assert_eq!(t, expected_title);
    assert_eq!(d, "/中&*");
    assert_eq!(s, expected);
    assert_eq!(
        title.source_position(),
        doc.references()[0].title.unwrap().end
    );
    assert_eq!(
        title.advance(&mut Budget::new(WorkLimit::default())),
        Ok(None)
    );
    assert!(std::mem::size_of::<LinkTitle<'_>>() < 160);
}

#[test]
fn container_title_semantic_limits_large_envelopes_and_physical_windows() {
    for separator in [" ", "\t"] {
        assert_title_limits_and_windows(&format!("{}{separator}", ">".repeat(900)));
    }
}

fn assert_title_limits_and_windows(prefix: &str) {
    for independent in [false, true] {
        for length in [4095, 4096, 4097] {
            let gap = if independent {
                format!("\n{prefix}")
            } else {
                " ".into()
            };
            let mut source = format!("[r]\n\n{}[r]: /u{gap}'a\n", prefix.replace('\t', " "));
            for _ in 0..(length - 2) / 2 {
                source.push_str(&format!("{prefix}x\n"));
            }
            if length % 2 == 1 {
                source.push_str(&format!("{prefix}x'\n"));
            } else {
                source.push_str(&format!("{prefix}'\n"));
            }
            let doc = parse(&source);
            let success = length <= MAX_LINK_TITLE_BYTES;
            assert_eq!(
                doc.link_title(LinkId(0)).is_some(),
                success,
                "independent={independent} length={length}"
            );
            assert_eq!(
                doc.link_destination(LinkId(0)).is_some(),
                independent || success
            );
            if success {
                let raw = doc.references()[0].title.unwrap();
                assert!(raw.len() > 1_000_000);
                assert_eq!(decoded(&doc, LinkId(0)).len(), length);
            }
            let n = source.len() as u64;
            assert!(doc.work().classified_bytes < 24 * n && doc.work().container_bytes < 32 * n);
            eprintln!(
                "envelope independent={independent} length={length} physical={} {:?}",
                source.len(),
                doc.work()
            );
        }
        for width in [BLOCK_LOOKAHEAD - 1, BLOCK_LOOKAHEAD, BLOCK_LOOKAHEAD + 1] {
            let next = if prefix.ends_with('\t') { ">\t" } else { "> " };
            for ending in ["\n", "\r\n", ""] {
                let gap = if independent { "\n> " } else { " " };
                let source = format!(
                    "[r]\n\n> [r]: /u{gap}'a\n{next}{}'{ending}",
                    "x".repeat(width - 3 - ending.len())
                );
                let doc = parse(&source);
                let success =
                    width <= BLOCK_LOOKAHEAD && (!ending.is_empty() || width < BLOCK_LOOKAHEAD);
                assert_eq!(
                    doc.link_title(LinkId(0)).is_some(),
                    success,
                    "width={width} ending={ending:?}"
                );
                assert_eq!(
                    doc.link_destination(LinkId(0)).is_some(),
                    independent || success
                );
            }
        }
    }
}

#[test]
fn container_title_fold_updates_merge_rebuild_and_tickets() {
    for (first, next) in [("> ", "> "), ("- > ", "  > ")].into_iter().chain(
        PARTIAL_PATHS
            .iter()
            .map(|&(first, _, next, _)| (first, next)),
    ) {
        for independent in [false, true] {
            for ending in ["end'\n", "end' bad\n", "\n", ""] {
                let mut doc = Document::new();
                doc.append("[r]\n\n").unwrap();
                let gap = if independent {
                    format!("\n{next}")
                } else {
                    " ".into()
                };
                let update = doc.append(&format!("{first}[r]: /u{gap}'first\n")).unwrap();
                let id = BlockId(update.new_blocks.start);
                let mut view = FoldView::new(&doc, 1).unwrap();
                let mut batched = FoldView::new(&doc, 1).unwrap();
                let mut rebuilt = doc.blocks().to_vec();
                assert!(
                    SourceFoldRegistration::new(&doc, &view, doc.block(id).unwrap().source)
                        .is_err()
                );
                let open_fold = if independent {
                    None
                } else {
                    let mut reg =
                        SourceFoldRegistration::container_paragraph(&doc, &view, id, 0).unwrap();
                    let fold = reg
                        .advance(&doc, &mut view, &mut Budget::new(WorkLimit::unlimited()))
                        .unwrap()
                        .id
                        .unwrap();
                    view.set_collapsed(&doc, fold, true).unwrap();
                    let mut reg =
                        SourceFoldRegistration::container_paragraph(&doc, &batched, id, 0).unwrap();
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
                let ticket2 = batched.heights().ticket(TileId(0)).unwrap();
                batched.measure(ticket2, 3).unwrap();
                let mut merged = Update::default();
                // Apply each update at its exact document revision.
                let first_update = doc.append(&format!("{next}middle\n")).unwrap();
                rebuilt.extend_from_slice(&doc.blocks()[first_update.new_blocks.clone()]);
                for b in first_update.changed_blocks.clone() {
                    rebuilt[b] = doc.blocks()[b].clone();
                }
                assert_eq!(rebuilt, doc.blocks());
                view.apply(&doc, &first_update).unwrap();
                merged.merge(first_update).unwrap();
                if !ending.is_empty() {
                    let update = doc.append(&format!("{next}{ending}")).unwrap();
                    rebuilt.extend_from_slice(&doc.blocks()[update.new_blocks.clone()]);
                    for b in update.changed_blocks.clone() {
                        rebuilt[b] = doc.blocks()[b].clone();
                    }
                    assert_eq!(rebuilt, doc.blocks());
                    view.apply(&doc, &update).unwrap();
                    merged.merge(update).unwrap();
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
                assert!(batched.measure(ticket2, 3).is_err());
                if let Some((fold, other)) = open_fold {
                    if ending == "end'\n" {
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
                for at in 0..doc.source().len() {
                    assert_eq!(
                        view.hidden_at(&doc, at, &mut Budget::new(WorkLimit::unlimited()))
                            .unwrap(),
                        batched
                            .hidden_at(&doc, at, &mut Budget::new(WorkLimit::unlimited()))
                            .unwrap()
                    );
                }
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
        }
    }
}

#[test]
fn container_title_work_growing_failure_and_late_definitions_ignore_history() {
    for separator in [" ", "\t"] {
        assert_title_work_ignores_history(&format!("{}{separator}", ">".repeat(970)));
    }
}

fn assert_title_work_ignores_history(prefix: &str) {
    for independent in [false, true] {
        let gap = if independent {
            format!("\n{prefix}")
        } else {
            " ".into()
        };
        let mut doc = Document::new();
        doc.append(&format!("{prefix}[r]: /u{gap}'first\n"))
            .unwrap();
        for _ in 0..100 {
            doc.append(&format!("{prefix}x\n")).unwrap();
        }
        assert_eq!(doc.work().inline_bytes, 0);
        doc.append(&format!("{prefix}{}", "x".repeat(1_000_000)))
            .unwrap();
        let before = doc.work();
        for _ in 0..1000 {
            doc.append("x").unwrap();
        }
        let after = doc.work();
        assert_eq!(after.classified_bytes, before.classified_bytes);
        assert_eq!(after.container_bytes, before.container_bytes);
        assert_eq!(after.container_steps, before.container_steps);
        assert_eq!(after.inline_bytes - before.inline_bytes, 1000);
        doc.finish();
        assert!(doc.references().iter().all(|r| r.title.is_none()));
        assert!(doc.work().classified_bytes < 32 * doc.source().len() as u64);
        assert!(doc.work().container_bytes < 32 * doc.source().len() as u64);
        eprintln!("million failure independent={independent} before={before:?} after={after:?}");

        let mut doc = Document::new();
        doc.append(&format!("{}\n\n", "[x][r] ".repeat(20_000)))
            .unwrap();
        let infos: Vec<_> = (0..doc.links().len())
            .map(|i| *doc.link_info(LinkId(i)).unwrap())
            .collect();
        let before = doc.work();
        let update = doc
            .append(&format!("{prefix}[R]: /late{gap}'first\n{prefix}last'\n"))
            .unwrap();
        assert_eq!(doc.work().inline_bytes, before.inline_bytes);
        assert!(doc.work().tree_visits - before.tree_visits < 256);
        assert_eq!(
            update.references_changed.len(),
            if independent { 2 } else { 1 }
        );
        assert!(update.new_links.is_empty());
        assert!(update.changes.len() < 16);
        for (i, info) in infos.iter().enumerate() {
            assert_eq!(doc.link_info(LinkId(i)), Some(info));
        }
        assert_eq!(decoded(&doc, LinkId(19_999)), "first\nlast");
        for i in 0..4096 {
            doc.append(&format!("> [n{i}]: /u 'a\n> b'\n")).unwrap();
        }
        let before_duplicate = doc.work();
        let update = doc.append("> [r]: /ignored\n> 'c\n> d'\n").unwrap();
        assert!(update.references_changed.is_empty());
        assert_eq!(doc.link_destination(LinkId(0)), Some("/late"));
        assert_eq!(decoded(&doc, LinkId(0)), "first\nlast");
        assert_eq!(doc.work().inline_bytes, before_duplicate.inline_bytes);
        assert!(doc.work().tree_visits - before_duplicate.tree_visits < 256);
        eprintln!(
            "20k uses + 4096 definitions independent={independent} old={before:?} before_duplicate={before_duplicate:?} after={:?}",
            doc.work()
        );
    }
}

// The byte-aligned control has the same post-prefix real source. Tab stops
// depend on the whole path: these are not a uniform three-column allowance.
const PARTIAL_PATHS: &[(&str, &str, &str, u8)] = &[
    ("> ", "> ", ">\t", 2),
    ("- ", "  ", "\t", 2),
    ("1. ", "   ", "\t", 1),
    ("-    ", "     ", "\t\t", 3),
    ("- > ", "  > ", "\t>\t", 2),
    ("> > ", "> > ", ">\t>\t", 2),
    ("- - - ", "      ", "\t\t", 2),
    (" > ", " > ", " >\t", 1),
    ("   > ", "   > ", "   >\t", 3),
    (" > - ", " >   ", " >\t\t", 3),
    ("> - > ", ">   > ", ">\t>\t", 2),
];

#[test]
fn partial_title_columns_are_structure_and_real_source_indentation_is_kept() {
    let mut cases = 0;
    for &(first, zero, partial, remainder) in PARTIAL_PATHS {
        for multiline_label in [false, true] {
            for next_target in [false, true] {
                for independent in [false, true] {
                    for (open, close) in [('\'', '\''), ('"', '"'), ('(', ')')] {
                        for newline in ["\n", "\r\n"] {
                            for close_only in [false, true] {
                                for spaces in 0..4 - remainder {
                                    // Independently build both physical sources and
                                    // compare them to an explicit decoded model.
                                    for next in [zero, partial] {
                                        let label = if multiline_label {
                                            format!("[Straße{newline}{next}中🙂]:")
                                        } else {
                                            "[r]:".into()
                                        };
                                        let target_gap = if next_target {
                                            format!("{newline}{next}")
                                        } else {
                                            " ".into()
                                        };
                                        let title_gap = if independent {
                                            format!("{newline}{next}")
                                        } else {
                                            " ".into()
                                        };
                                        let tail = format!(
                                            "{}{}",
                                            " ".repeat(spaces.into()),
                                            if close_only { "" } else { "中🙂 &amp;\\*" }
                                        );
                                        let raw = format!("a&#13;&#10;{newline}{next}{tail}");
                                        let usage = if multiline_label {
                                            "STRASSE 中🙂"
                                        } else {
                                            "r"
                                        };
                                        let source = format!(
                                            "{first}{label}{target_gap}<my url>{title_gap}{open}{raw}{close}{newline}\n[{usage}]\n"
                                        );
                                        let doc = parse(&source);
                                        assert_eq!(
                                            doc.link_destination(LinkId(0)),
                                            Some("my url"),
                                            "{source:?}"
                                        );
                                        assert_eq!(
                                            doc.link_title(LinkId(0)),
                                            Some(raw.as_str()),
                                            "{source:?}"
                                        );
                                        assert_eq!(
                                            decoded(&doc, LinkId(0)),
                                            format!(
                                                "a\r\n\n{}{}",
                                                " ".repeat(spaces.into()),
                                                if close_only { "" } else { "中🙂 &*" }
                                            ),
                                            "{source:?}"
                                        );
                                        let start = source.find(&raw).unwrap();
                                        assert_eq!(
                                            doc.references()[0].title,
                                            Some(Span::new(start, start + raw.len()))
                                        );
                                        assert_eq!(doc.links().len(), 1);
                                        assert_eq!(
                                            doc.work().inline_bytes,
                                            format!("[{usage}]\n").len() as u64
                                        );
                                        assert!(doc.tables().is_empty());
                                        cases += 1;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    eprintln!("{cases} independently constructed zero/partial source models");
}

#[test]
fn partial_title_label_limit_counts_real_bytes_independently_of_title_gaps() {
    for &(first, _, next, _) in PARTIAL_PATHS {
        for bytes in [999, 1000] {
            for independent in [false, true] {
                let logical = format!("a\n{}{}", "x\n".repeat(498), "z".repeat(bytes - 998));
                assert_eq!(logical.len(), bytes);
                let label = logical.replace('\n', &format!("\n{next}"));
                let gap = if independent {
                    format!("\n{next}")
                } else {
                    " ".into()
                };
                let source = format!("{first}[{label}]: /u{gap}'a\n{next}z'\n");
                let doc = parse(&source);
                let winners: Vec<_> = doc
                    .references()
                    .iter()
                    .filter(|r| r.destination.is_some())
                    .collect();
                assert_eq!(
                    winners.len(),
                    usize::from(bytes == 999),
                    "{first:?} {next:?} {bytes}"
                );
                if bytes == 999 {
                    assert_eq!(
                        winners[0].title.and_then(|s| doc.text(s)),
                        Some(format!("a\n{next}z").as_str())
                    );
                }
                assert!(doc.work().classified_bytes < 24 * source.len() as u64);
            }
        }
    }
}
