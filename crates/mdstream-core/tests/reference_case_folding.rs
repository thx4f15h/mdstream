use mdstream::*;

#[path = "support/link_metadata.rs"]
mod link_metadata;
#[path = "support/case_folding.rs"]
mod model;
#[path = "support/snapshot.rs"]
mod support;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

#[test]
fn independent_data_model_bounds_and_default_semantics() {
    let table = model::mappings();
    let mut max_scalars = 0;
    let mut max_bytes = 0;
    let mut max_ratio = 0;
    for (&c, value) in &table {
        max_scalars = max_scalars.max(value.chars().count());
        max_bytes = max_bytes.max(value.len());
        max_ratio = max_ratio.max(value.len().div_ceil(c.len_utf8()));
        assert!(value.len() <= 3 * c.len_utf8());
        assert_eq!(model::normalize(value, &table).as_ref(), Some(value));
    }
    assert_eq!((max_scalars, max_bytes, max_ratio), (3, 6, 3));
    for (raw, expected) in [
        (" ẞ\u{a0}\tﬃ  ", "ss ffi"),
        ("Iİıi", "ii\u{307}ıi"),
        ("É E\u{301} Ａ A", "é e\u{301} ａ a"),
        ("Ꭰꭰ", "ᎠᎠ"),
        ("r\\! r! &szlig;", "r\\! r! &szlig;"),
    ] {
        assert_eq!(model::normalize(raw, &table).as_deref(), Some(expected));
    }
    let raw = format!("{}a", "ΐ".repeat(499));
    assert_eq!(raw.len(), 999);
    assert_eq!(model::normalize(&raw, &table).unwrap().len(), 2995);
    assert_eq!(model::normalize(&(raw + "a"), &table), None);
}

#[test]
fn expanding_and_non_lowercase_folds_share_one_first_definition() {
    for (label, folded) in [
        ("ẞ", "ss"),
        ("ß", "ss"),
        ("ﬃ", "ffi"),
        ("ΐ", "ι\u{308}\u{301}"),
        ("ᾲ", "ὰι"),
        ("և", "եւ"),
        ("Σςσ", "σσσ"),
        ("µ", "μ"),
        ("Ꭰꭰ", "ᎠᎠ"),
    ] {
        let source = format!(
            "[{label}] ![{folded}][] [text][{label}]\n\n[{folded}]: /first\n[{label}]: /ignored\n"
        );
        let doc = parse(&source);
        assert_eq!(doc.links(), &[LinkTarget::Reference(SymbolId(0)); 3]);
        assert_eq!(doc.references().len(), 2, "shortcut 'text' also interns");
        assert_eq!(doc.references()[0].label, folded);
        for i in 0..3 {
            assert_eq!(
                doc.link_destination(LinkId(i)),
                Some("/first"),
                "{source:?}"
            );
        }
    }
}

#[test]
fn default_non_turkic_and_raw_keys_keep_normalization_forms_distinct() {
    for (query, definition, matches) in [
        ("I", "i", true),
        ("I", "ı", false),
        ("İ", "i\u{307}", true),
        ("İ", "i", false),
        ("ı", "i", false),
        ("É", "é", true),
        ("É", "e\u{301}", false),
        ("Å", "å", true),
        ("Å", "a\u{30a}", false),
        ("Ａ", "ａ", true),
        ("Ａ", "a", false),
        ("ⓐ", "a", false),
        ("ﬃ", "ffi", true),
        ("r\\!ẞ", "r\\!ss", true),
        ("r\\!ẞ", "r!ss", false),
        ("&szlig;", "ss", false),
        ("&#223;", "ß", false),
        (" \tẞ\u{a0}ﬃ\u{2003} ", "SS FFI", true),
    ] {
        for image in ["", "!"] {
            let source = format!("{image}[text][{query}]\n\n[{definition}]: /u\n");
            let doc = parse(&source);
            assert_eq!(
                doc.link_destination(LinkId(0)),
                matches.then_some("/u"),
                "{source:?}"
            );
            assert_eq!(
                doc.text(doc.link_info(LinkId(0)).unwrap().label),
                Some("text")
            );
        }
    }
    let doc = parse("[^ẞ] [^SS] [^ss] [^ß] [ẞ]\n\n[ss]: /u\n[^SS]: body\n");
    assert_eq!(doc.link_destination(LinkId(0)), Some("/u"));
    assert_eq!(doc.references()[0].label, "ss");
    assert_eq!(
        doc.footnotes()
            .iter()
            .map(|n| n.label.as_str())
            .collect::<Vec<_>>(),
        ["ẞ", "SS", "ss", "ß"]
    );
    for (i, note) in doc.footnotes().iter().enumerate() {
        assert_eq!(note.definition.is_some(), i == 1);
    }
}

fn check_work(work: Work) {
    assert!(work.label_scalars <= work.label_bytes);
    assert!(work.label_fold_steps <= 12 * work.label_scalars);
    assert!(work.label_key_bytes <= 3 * work.label_bytes);
}

#[test]
fn raw_999_byte_limit_precedes_expansion_and_contraction() {
    for len in [998, 999, 1000] {
        for raw in [
            format!("{}{}", "ΐ".repeat(len / 2), "a".repeat(len % 2)),
            format!("{}{}", "K".repeat(len / 3), "A".repeat(len % 3)),
        ] {
            assert_eq!(raw.len(), len);
            for label in [
                format!("[{raw}]"),
                format!("[{raw}][]"),
                format!("[text][{raw}]"),
            ] {
                for image in ["", "!"] {
                    let source = format!("{image}{label}\n\n[{raw}]: /u\n");
                    let doc = parse(&source);
                    assert_eq!(
                        doc.link_destination(LinkId(0)),
                        (len <= 999).then_some("/u")
                    );
                    check_work(doc.work());
                    if len <= 999 {
                        let LinkTarget::Reference(symbol) = doc.links()[0] else {
                            panic!()
                        };
                        let expected = model::normalize(&raw, &model::mappings()).unwrap();
                        assert_eq!(doc.references()[symbol.0].label, expected);
                        assert_eq!(
                            doc.text(doc.link_info(LinkId(0)).unwrap().source),
                            Some(format!("{image}{label}").as_str())
                        );
                    }
                }
            }
        }
    }
    // A rejected 1000-byte spelling must not match its much shorter folded key.
    let long = format!("{}A", "K".repeat(333));
    let short = format!("{}a", "k".repeat(333));
    assert_eq!(long.len(), 1000);
    let doc = parse(&format!("[text][{long}]\n\n[{short}]: /u\n"));
    assert_eq!(doc.link_destination(LinkId(0)), None);

    // The raw count includes CRLF and whitespace, and excludes Quote prefixes.
    for len in [998, 999, 1000] {
        for image in ["", "!"] {
            for tail in ["", "[]"] {
                let spaces = " ".repeat(len - 8);
                let raw = format!("ẞ{spaces}\r\nﬃ");
                assert_eq!(raw.len(), len);
                let doc = parse(&format!(
                    "> {image}[ẞ{spaces}\r\n> ﬃ]{tail}\n\n[SS FFI]: /u\n"
                ));
                assert_eq!(
                    doc.link_destination(LinkId(0)),
                    (len <= 999).then_some("/u")
                );
                check_work(doc.work());
            }
        }
    }
}

#[test]
fn complete_document_matches_independent_unicode_data_model() {
    use std::collections::BTreeMap;

    let table = model::mappings();
    let mut source = String::new();
    let mut expected = Vec::new();
    let mut definitions = String::new();
    let mut winners = BTreeMap::new();
    // All 1585 full mappings plus unlisted, non-BMP and normalization witnesses.
    let scalars: Vec<_> = table
        .keys()
        .copied()
        .chain(['🙂', '中', '\u{10ffff}', 'é', '\u{301}'])
        .collect();
    for (i, c) in scalars.iter().enumerate() {
        let raw = format!(" K{c}\u{2003}Z ");
        let key = model::normalize(&raw, &table).unwrap();
        source.push_str(&format!(
            "[{raw}] ![{raw}][] [visible][{raw}] ![visible][{raw}]\n"
        ));
        expected.extend([key.clone(), key.clone(), key.clone(), key.clone()]);
        let folded = table.get(c).cloned().unwrap_or_else(|| c.to_string());
        let definition = format!("\t k{folded}\u{a0}z\t");
        assert_eq!(model::normalize(&definition, &table).as_ref(), Some(&key));
        winners.entry(key).or_insert(i);
        definitions.push_str(&format!("[{definition}]: /v{i} 't\\*'\n"));
    }
    source.push('\n');
    source.push_str(&definitions);
    let doc = parse(&source);
    assert_eq!(doc.source(), source);
    assert_eq!(doc.links().len(), 6360);
    for (i, key) in expected.iter().enumerate() {
        let LinkTarget::Reference(symbol) = doc.links()[i] else {
            panic!()
        };
        let reference = &doc.references()[symbol.0];
        assert_eq!(&reference.label, key);
        assert_eq!(reference.revision, 1);
        assert_eq!(
            doc.link_destination(LinkId(i)),
            Some(format!("/v{}", winners[key]).as_str())
        );
        assert_eq!(doc.link_title(LinkId(i)), Some("t\\*"));
        let info = doc.link_info(LinkId(i)).unwrap();
        assert_eq!(
            info.kind,
            if i % 2 == 0 {
                LinkKind::Link
            } else {
                LinkKind::Image
            }
        );
        let label = if i % 4 < 2 {
            format!(" K{}\u{2003}Z ", scalars[i / 4])
        } else {
            "visible".to_owned()
        };
        assert_eq!(doc.text(info.label), Some(label.as_str()));
    }
    assert!(doc.work().label_bytes <= 2 * source.len() as u64);
    check_work(doc.work());
}

#[test]
fn utf8_every_split_scalar_and_byte_prefix_including_eof() {
    for source in [
        "[ẞ]\n\n[SS]: /url\n",
        "![ΐ][] [text][ﬃ]\r\n\r\n[ι\u{308}\u{301}]: /a&amp; 't\\*'\r\n[FFI]: /b\r\n",
        "> [ ẞ\r\n> \u{2003}ﬃ ]\n\n[SS FFI]: /u\n",
        "[Ꭰꭰ] [𐐀]\n\n[ᎠᎠ]: /c\n[𐐨]: /d\n",
        "[x][İ] [y][I] [z][e\u{301}]\n\n[i]: /i\n[é]: /e\n",
        "[x][ﬃ\\!]\n\n[ffi!]: /no\n",
        "[x][ΐ",
        "![ẞ] [text][ﬃ\n",
    ] {
        let whole = support::snapshot(&parse(source));
        let bytes = source.as_bytes();
        for split in 0..=bytes.len() {
            let mut doc = Utf8Document::new();
            doc.append(&bytes[..split]).unwrap();
            doc.append(&bytes[split..]).unwrap();
            doc.finish();
            assert_eq!(
                support::snapshot(doc.document()),
                whole,
                "split {split}, {source:?}"
            );
        }
        let mut streamed = Utf8Document::new();
        for end in 1..=bytes.len() {
            streamed.append(&bytes[end - 1..end]).unwrap();
            let mut prefix = Utf8Document::new();
            prefix.append(&bytes[..end]).unwrap();
            assert_eq!(
                support::snapshot(streamed.document()),
                support::snapshot(prefix.document())
            );
            assert_eq!(streamed.pending_bytes(), prefix.pending_bytes());
            assert_eq!(streamed.document().pending(), prefix.document().pending());
            prefix.finish();
            let eof = parse(&String::from_utf8_lossy(&bytes[..end]));
            assert_eq!(
                support::snapshot(prefix.document()),
                support::snapshot(&eof)
            );
            if source.is_char_boundary(end) {
                let mut scalar = Document::new();
                for c in source[..end].chars() {
                    scalar.append(c.encode_utf8(&mut [0; 4])).unwrap();
                }
                assert_eq!(
                    support::snapshot(&scalar),
                    support::snapshot(streamed.document())
                );
                assert_eq!(
                    link_metadata::snapshot(&scalar),
                    link_metadata::snapshot(streamed.document())
                );
                scalar.finish();
                assert_eq!(support::snapshot(&scalar), support::snapshot(&eof));
            }
        }
        streamed.finish();
        assert_eq!(support::snapshot(streamed.document()), whole);
        for width in 1..8 {
            let mut doc = Utf8Document::new();
            for chunk in bytes.chunks(width) {
                doc.append(chunk).unwrap();
            }
            doc.finish();
            assert_eq!(support::snapshot(doc.document()), whole);
            check_work(doc.document().work());
        }
    }
}

#[test]
fn late_definitions_keep_ids_first_winners_notifications_and_original_spans() {
    let mut doc = Document::new();
    doc.append("[ẞ] ![ss][] [text][ſS] [ ẞ\u{a0}ﬃ ]\n\n")
        .unwrap();
    let targets = doc.links().to_vec();
    let infos: Vec<_> = (0..4).map(|i| *doc.link_info(LinkId(i)).unwrap()).collect();
    assert_eq!(&targets[..3], &[LinkTarget::Reference(SymbolId(0)); 3]);
    let last = match targets[3] {
        LinkTarget::Reference(id) => id,
        _ => panic!(),
    };
    assert_eq!(doc.references()[last.0].label, "ss ffi");
    for (definition, symbol) in [
        ("[SS]: /raw&amp; 't\\*'\n", SymbolId(0)),
        ("> [ſs\u{2003}FFI]: /other '🙂'\n", last),
    ] {
        let before = doc.work();
        let start = doc.source().len();
        let mut notices = 0;
        for (at, c) in definition.char_indices() {
            let update = doc.append(c.encode_utf8(&mut [0; 4])).unwrap();
            assert!(update.new_links.is_empty());
            if at + c.len_utf8() < definition.len() {
                assert!(update.references_changed.is_empty());
            } else {
                assert_eq!(update.references_changed, [symbol]);
                assert!(
                    update
                        .changes
                        .iter()
                        .any(|change| change.kind == ChangeKind::Reference
                            && change.source.start
                                == doc.references()[symbol.0].first_use.unwrap())
                );
                notices += 1;
            }
            assert_eq!(doc.links(), targets);
            for (i, info) in infos.iter().enumerate() {
                assert_eq!(doc.link_info(LinkId(i)), Some(info));
            }
        }
        assert_eq!(notices, 1);
        assert_eq!(doc.work().inline_bytes, before.inline_bytes);
        let reference = &doc.references()[symbol.0];
        assert!(reference.destination.unwrap().start >= start);
        assert_eq!(
            doc.text(reference.destination.unwrap()),
            doc.link_destination(if symbol == SymbolId(0) {
                LinkId(0)
            } else {
                LinkId(3)
            })
        );
    }
    let winners = doc.references().to_vec();
    let update = doc.append("\n[ß]: /ignored\n[ss ffi]: /ignored\n").unwrap();
    assert!(update.references_changed.is_empty());
    doc.finish();
    assert_eq!(doc.references(), winners);
    for (i, info) in infos.iter().enumerate() {
        assert_eq!(doc.link_info(LinkId(i)), Some(info));
        assert_eq!(
            doc.text(info.label),
            Some(["ẞ", "ss", "text", " ẞ\u{a0}ﬃ "][i])
        );
    }
    assert_eq!(
        link_metadata::snapshot(&doc),
        [
            Some("<a href=\"/raw&amp;\" title=\"t*\">".to_owned()),
            Some("<a href=\"/raw&amp;\" title=\"t*\">".to_owned()),
            Some("<a href=\"/raw&amp;\" title=\"t*\">".to_owned()),
            Some("<a href=\"/other\" title=\"🙂\">".to_owned()),
        ]
    );
    let span = Span::new(0, doc.source().len());
    assert_eq!(
        doc.runs_in(span)
            .unwrap()
            .map(|r| r.text)
            .collect::<String>(),
        doc.source()
    );
    for (at, c) in doc.source().char_indices() {
        for run in doc
            .display_runs_in(Span::new(at, at + c.len_utf8()))
            .unwrap()
        {
            assert_eq!(run.replacement, None);
            assert_eq!(doc.text(run.source), Some(run.text));
        }
    }
    check_work(doc.work());
}

#[test]
fn deep_million_byte_and_historical_reference_work_stays_bounded() {
    let prefix = ">".repeat(980);
    let mut doc = Document::new();
    doc.append(&format!("{prefix} {}", "x".repeat(1_000_000)))
        .unwrap();
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("ΐ").unwrap();
    }
    assert_eq!(doc.work().inline_bytes - before.inline_bytes, 2000);
    assert_eq!(doc.work().classified_bytes, before.classified_bytes);
    assert_eq!(doc.work().label_scalars, before.label_scalars);
    assert_eq!(doc.work().container_steps, before.container_steps);
    doc.append(&format!("\n{prefix}\n")).unwrap();
    let before = doc.work();
    doc.append(&format!("{prefix} [ΐ]: /u\n")).unwrap();
    assert_eq!(doc.references()[0].label, "ι\u{308}\u{301}");
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert_eq!(doc.work().label_bytes - before.label_bytes, 2);
    assert_eq!(doc.work().label_scalars - before.label_scalars, 1);
    assert!(doc.work().label_fold_steps - before.label_fold_steps <= 12);
    assert_eq!(doc.work().label_key_bytes - before.label_key_bytes, 6);
    assert!(doc.work().classified_bytes - before.classified_bytes < 6 * BLOCK_LOOKAHEAD as u64);
    assert!(doc.work().container_steps - before.container_steps < 6000);
    doc.finish();
    check_work(doc.work());

    let mut doc = Document::new();
    doc.append(&format!("![ΐ{}", "x".repeat(1_000_000)))
        .unwrap();
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("x").unwrap();
    }
    assert_eq!(doc.work().inline_bytes - before.inline_bytes, 1000);
    assert_eq!(doc.work().classified_bytes, before.classified_bytes);
    assert_eq!(doc.work().label_bytes, before.label_bytes);
    doc.append("] [v][ﬃ]\n\n[ffi]: /u\n").unwrap();
    doc.finish();
    assert_eq!(doc.links().len(), 1);
    assert_eq!(doc.link_destination(LinkId(0)), Some("/u"));
    check_work(doc.work());

    for repetitions in [1, 10_000] {
        let mut doc = Document::new();
        doc.append(&format!("{}\n\n", "[ẞ] ![ß][] ".repeat(repetitions)))
            .unwrap();
        let count = 2 * repetitions;
        assert_eq!(doc.links().len(), count);
        let first = *doc.link_info(LinkId(0)).unwrap();
        let last = *doc.link_info(LinkId(count - 1)).unwrap();
        let before = doc.work();
        let update = doc.append("[ss]: /late\n").unwrap();
        assert_eq!(update.references_changed, [SymbolId(0)]);
        assert!(update.new_links.is_empty());
        assert!(update.changes.len() < 12);
        assert_eq!(doc.work().inline_bytes, before.inline_bytes);
        assert_eq!(doc.work().label_bytes - before.label_bytes, 2);
        assert_eq!(doc.work().label_scalars - before.label_scalars, 2);
        assert!(doc.work().label_fold_steps - before.label_fold_steps <= 24);
        assert_eq!(doc.work().label_key_bytes - before.label_key_bytes, 2);
        assert!(doc.work().tree_visits - before.tree_visits < 2000);
        assert_eq!(doc.link_info(LinkId(0)), Some(&first));
        assert_eq!(doc.link_info(LinkId(count - 1)), Some(&last));
        assert_eq!(doc.link_destination(LinkId(count - 1)), Some("/late"));
        doc.finish();
        check_work(doc.work());
    }
}
