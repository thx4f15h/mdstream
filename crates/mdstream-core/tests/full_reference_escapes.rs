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

fn visible(doc: &Document, span: Span) -> String {
    doc.display_runs_in(span)
        .unwrap()
        .filter(|run| !run.style.contains(Style::HIDDEN))
        .map(|run| run.text)
        .collect()
}

#[test]
fn full_labels_follow_backslash_parity_without_decoding_symbol_keys() {
    let mut cases = 0;
    for prefix in ["", "> ", "- ", ">\t "] {
        for image in ["", "!"] {
            for base in ["ref", "中🙂", "a\\*b"] {
                for slashes in 0..8 {
                    for bracket in [']', '['] {
                        let raw = format!("{base}{}", "\\".repeat(slashes));
                        let (tail, key, accepted) = if bracket == ']' {
                            if slashes % 2 == 1 {
                                (format!("{raw}]]"), format!("{raw}]"), true)
                            } else {
                                (format!("{raw}]"), raw.clone(), true)
                            }
                        } else {
                            (format!("{raw}[x]"), format!("{raw}[x"), slashes % 2 == 1)
                        };
                        for ending in ["\n", "\r\n"] {
                            let source = format!(
                                "{prefix}{image}[text][{tail}{ending}\n[{key}]: /yes 'title'\n"
                            );
                            let doc = parse(&source);
                            assert_eq!(
                                doc.link_destination(LinkId(0)),
                                accepted.then_some("/yes"),
                                "{source:?}"
                            );
                            let info = doc.link_info(LinkId(0)).unwrap();
                            assert_eq!(doc.text(info.label), Some("text"));
                            assert_eq!(
                                info.kind,
                                if image.is_empty() {
                                    LinkKind::Link
                                } else {
                                    LinkKind::Image
                                }
                            );
                            if accepted {
                                assert_eq!(
                                    doc.text(info.source),
                                    Some(format!("{image}[text][{tail}").as_str())
                                );
                                assert_eq!(info.state, SyntaxState::Sealed);
                                let LinkTarget::Reference(symbol) = doc.links()[0] else {
                                    panic!()
                                };
                                assert_eq!(doc.references()[symbol.0].label, key.to_lowercase());
                            }
                            cases += 1;
                        }
                    }
                }
            }
        }
    }
    assert_eq!(cases, 768);
    let doc = parse("[a][r\\*] [b][r*]\n\n[r\\*]: /raw\n[r*]: /plain\n");
    assert_eq!(doc.link_destination(LinkId(0)), Some("/raw"));
    assert_eq!(doc.link_destination(LinkId(1)), Some("/plain"));
}

#[test]
fn pending_failed_and_length_boundaries_keep_the_existing_dialect() {
    for tail in ["r\\]", "r\\", "r\\]\n]", "r\\\r\n]", "r\\\\[bad]", " \t ]"] {
        let source = format!("[text][{tail}\n\n[text]: /shortcut\n[r\\]]: /full\n");
        let doc = parse(&source);
        assert_eq!(
            doc.link_destination(LinkId(0)),
            Some("/shortcut"),
            "{source:?}"
        );
        assert_eq!(
            doc.text(doc.link_info(LinkId(0)).unwrap().source),
            Some("[text]")
        );
    }
    for len in [MAX_LABEL_BYTES - 1, MAX_LABEL_BYTES, MAX_LABEL_BYTES + 1] {
        for unicode in [false, true] {
            let base = if unicode { "中" } else { "a" };
            let key = format!("{base}{}\\]", "a".repeat(len - base.len() - 2));
            let doc = parse(&format!("[text][{key}]\n\n[{key}]: /yes\n"));
            assert_eq!(
                doc.link_destination(LinkId(0)),
                (len <= MAX_LABEL_BYTES).then_some("/yes")
            );
        }
    }
    let mut doc = Document::new();
    doc.append("[text][r\\]\n").unwrap();
    assert_eq!(
        doc.text(doc.link_info(LinkId(0)).unwrap().source),
        Some("[text]")
    );
    doc.finish();
    assert_eq!(doc.link_info(LinkId(0)).unwrap().state, SyntaxState::Sealed);
    let doc = parse("[r\\]][] [r\\]] ![r\\]][]\n\n[r\\]]: /yes\n");
    assert_eq!(doc.links().len(), 3);
    for id in 0..3 {
        assert_eq!(doc.link_destination(LinkId(id)), Some("/yes"));
    }
}

#[test]
fn every_split_scalar_prefix_and_eof_preserves_semantics() {
    for source in [
        "[text][r\\]]\n\n[r\\]]: /raw&amp; 't\\*'\n",
        "![中🙂][r\\[x]\r\n\r\n[r\\[x]: /yes\r\n",
        "> [old\n> text][r\\]]\n\n[r\\]]: /yes\n",
        "- [text][r\\\\] extra\n\n[r\\\\]: /yes\n",
        "[text][r\\]",
        "[text][r\\\n] next\n",
        "[text][r\\\\[new] /tail\n",
        "[text][r\\\\\\]] [next](/url)\n",
    ] {
        let expected = support::snapshot(&parse(source));
        for at in (0..=source.len()).filter(|&at| source.is_char_boundary(at)) {
            let mut doc = Document::new();
            doc.append(&source[..at]).unwrap();
            doc.append(&source[at..]).unwrap();
            doc.finish();
            assert_eq!(support::snapshot(&doc), expected, "{source:?} split {at}");
            link_metadata::snapshot(&doc);
        }
        let mut doc = Document::new();
        for (at, ch) in source.char_indices() {
            let end = at + ch.len_utf8();
            doc.append(&source[at..end]).unwrap();
            let mut prefix = Document::new();
            prefix.append(&source[..end]).unwrap();
            assert_eq!(support::snapshot(&doc), support::snapshot(&prefix));
            assert_eq!(doc.pending(), prefix.pending());
            let mut eof = Document::new();
            eof.append(&source[..at]).unwrap();
            eof.append(&source[at..end]).unwrap();
            eof.finish();
            assert_eq!(
                support::snapshot(&eof),
                support::snapshot(&parse(&source[..end]))
            );
        }
    }
}

#[test]
fn live_suffix_state_crosses_appends_after_the_classification_window() {
    let prefix = format!("{} ", "x".repeat(1100));
    for tail in [
        "r\\]]",
        "r\\[x]",
        "r\\\\]",
        "r\\\\\\]]",
        "r\\]",
        "r\\\r\n]",
        "r\\\\[new]",
    ] {
        let source = format!("{prefix}[text][{tail}");
        let mut doc = Document::new();
        doc.append(&prefix).unwrap();
        for (at, ch) in source[prefix.len()..].char_indices() {
            let at = at + prefix.len();
            let end = at + ch.len_utf8();
            let before = doc.work();
            doc.append(&source[at..end]).unwrap();
            if ch != '\n'
                && ch != '\r'
                && !source[..at].ends_with('\n')
                && !source[..at].ends_with('\r')
            {
                assert!(doc.work().inline_bytes - before.inline_bytes <= 1);
            }
            let mut whole = Document::new();
            whole.append(&source[..end]).unwrap();
            assert_eq!(support::snapshot(&doc), support::snapshot(&whole));
            assert_eq!(doc.pending(), whole.pending());
        }
        doc.finish();
        assert_eq!(support::snapshot(&doc), support::snapshot(&parse(&source)));
    }
    let mut doc = Document::new();
    doc.append(&format!("{prefix}[text][r\\")).unwrap();
    let info = *doc.link_info(LinkId(0)).unwrap();
    assert_eq!(info.state, SyntaxState::Provisional);
    doc.append("]").unwrap();
    assert_eq!(doc.link_info(LinkId(0)), Some(&info));
    doc.append("]").unwrap();
    let sealed = doc.link_info(LinkId(0)).unwrap();
    assert_eq!(sealed.state, SyntaxState::Sealed);
    assert_eq!(sealed.label, info.label);
    assert_eq!(doc.text(sealed.source), Some("[text][r\\]]"));
    assert_eq!(doc.links().len(), 1);
}

#[test]
fn late_definitions_keep_identity_notifications_and_first_winner() {
    let mut doc = Document::new();
    doc.append("[text][R\\]] ![alt][R\\[]\n\n").unwrap();
    let links = doc.links().to_vec();
    let infos = [
        *doc.link_info(LinkId(0)).unwrap(),
        *doc.link_info(LinkId(1)).unwrap(),
    ];
    assert_eq!(doc.text(infos[0].source), Some("[text][R\\]]"));
    assert_eq!(doc.text(infos[1].source), Some("![alt][R\\[]"));
    let update = doc.append("[r\\]]: /one 't'\n[r\\[]: /two\n").unwrap();
    assert_eq!(update.references_changed.len(), 2);
    assert!(update.new_links.is_empty());
    assert!(
        update
            .changes
            .iter()
            .any(|c| c.kind == ChangeKind::Reference)
    );
    assert_eq!(doc.link_destination(LinkId(0)), Some("/one"));
    assert_eq!(doc.link_destination(LinkId(1)), Some("/two"));
    assert_eq!(doc.link_title(LinkId(0)), Some("t"));
    let update = doc.append("[R\\]]: /ignored\n").unwrap();
    assert!(update.references_changed.is_empty());
    doc.finish();
    assert_eq!(doc.links(), links);
    for (i, info) in infos.iter().enumerate() {
        assert_eq!(doc.link_info(LinkId(i)), Some(info));
    }
}

#[test]
fn source_mapping_and_six_byte_shared_budgets_agree() {
    let source = "[中🙂 &amp;][r\\]]\r\n\r\n[r\\]]: /raw&amp; 't\\*'\r\n";
    let doc = parse(source);
    let span = Span::new(0, source.len());
    assert_eq!(
        doc.runs_in(span)
            .unwrap()
            .map(|r| r.text)
            .collect::<String>(),
        source
    );
    let expected = visible(&doc, span);
    assert_eq!(expected, "中🙂 &\n\n");
    assert_eq!(
        source
            .char_indices()
            .map(|(at, ch)| visible(&doc, Span::new(at, at + ch.len_utf8())))
            .collect::<String>(),
        expected
    );
    let mut cursor = doc.display_cursor(span).unwrap();
    assert_eq!(
        cursor
            .next(&doc, &mut Budget::new(WorkLimit::default()))
            .unwrap(),
        DisplayStep::BudgetExhausted
    );
    let mut output = String::new();
    while cursor.source_position() < source.len() {
        let at = cursor.source_position();
        let mut budget = Budget::new(WorkLimit {
            bytes: 6,
            queries: 1,
            objects: 1,
            ..WorkLimit::default()
        });
        let DisplayStep::Run(run) = cursor.next(&doc, &mut budget).unwrap() else {
            panic!("one scalar fits")
        };
        assert_eq!(run.source.start, at);
        assert_eq!(cursor.source_position(), run.source.end);
        assert!(run.source.end > at);
        if let Some(replacement) = run.replacement {
            assert_eq!(doc.text(replacement), Some("&amp;"));
            assert_eq!(run.text, if at == replacement.start { "&" } else { "" });
        } else {
            assert_eq!(run.text, &source[run.source.range()]);
        }
        assert!(
            budget.used().bytes <= 6 && budget.used().queries <= 1 && budget.used().objects <= 1
        );
        if !run.style.contains(Style::HIDDEN) {
            output.push_str(run.text);
        }
    }
    assert_eq!(output, expected);
    assert_eq!(
        link_metadata::snapshot(&doc),
        [Some("<a href=\"/raw&amp;\" title=\"t*\">".to_owned())]
    );
}

#[test]
fn escaped_suffixes_keep_late_header_cancellation_and_regular_tables() {
    for suffix in ["[r\\]]", "[r\\[]", "[r\\\\]", "[r\\]"] {
        for image in ["", "!"] {
            for ending in ["\n", "\n> ---|---\n", "\n> not a delimiter\n", ""] {
                let source = format!(
                    "> {image}[old\n> text]{suffix} {}| cell{ending}",
                    "x".repeat(1100)
                );
                let whole = parse(&source);
                assert_eq!(
                    whole.tables().len(),
                    usize::from(ending == "\n> ---|---\n"),
                    "{source:?}"
                );
                let expected = support::snapshot(&whole);
                for at in 0..=source.len() {
                    let mut doc = Document::new();
                    doc.append(&source[..at]).unwrap();
                    doc.append(&source[at..]).unwrap();
                    doc.finish();
                    assert_eq!(support::snapshot(&doc), expected, "split {at}");
                }
            }
        }
    }
    let doc = parse(&format!(
        "> [old\n> text][key] {}| cell\n> ---|---\n",
        "x".repeat(1100)
    ));
    assert_eq!(doc.tables().len(), 1);
    let doc = parse("[text][r\\]] | cell\n---|---\n\n[r\\]]: /yes\n");
    assert_eq!(doc.tables().len(), 1);
    assert_eq!(doc.link_destination(LinkId(0)), Some("/yes"));
}

#[test]
fn long_candidates_and_historical_references_do_not_rescan() {
    let mut doc = Document::new();
    doc.append(&format!(
        ">{} [text][r\\]{}",
        ">".repeat(979),
        "x".repeat(1_000_000)
    ))
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

    let mut doc = Document::new();
    doc.append(&format!("{}\n\n", "[text][r\\]] ".repeat(20_000)))
        .unwrap();
    assert_eq!(doc.links().len(), 20_000);
    let before = doc.work();
    let update = doc.append("[r\\]]: /late\n").unwrap();
    assert_eq!(update.references_changed.len(), 1);
    assert!(update.new_links.is_empty());
    assert!(update.changes.len() < 12);
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert_eq!(doc.work().label_bytes - before.label_bytes, 3);
    assert!(doc.work().tree_visits - before.tree_visits < 2000);
    assert_eq!(doc.link_destination(LinkId(19_999)), Some("/late"));
}
