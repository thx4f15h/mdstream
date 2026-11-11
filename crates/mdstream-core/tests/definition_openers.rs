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

fn definitions(doc: &Document) -> usize {
    doc.blocks()
        .iter()
        .filter(|b| matches!(b.kind, BlockKind::Definition { .. }))
        .count()
}

#[test]
fn opening_brackets_require_odd_backslash_parity_in_each_scope() {
    let mut cases = 0;
    for prefix in ["", "   ", "> ", "- ", "1. ", ">\t ", "- > ", "> - "] {
        for slashes in 0..8 {
            for base in ["r", "中🙂", "r\\]", "r\\[x"] {
                for ending in ["\n", "\r\n", ""] {
                    let label = format!("{base}{}[x", "\\".repeat(slashes));
                    let source = format!("{prefix}[{label}]: /raw&amp; 't\\*'{ending}");
                    let doc = parse(&source);
                    let accepted = slashes % 2 == 1;
                    assert_eq!(definitions(&doc), usize::from(accepted), "{source:?}");
                    let resolved: Vec<_> = doc
                        .references()
                        .iter()
                        .filter(|r| r.destination.is_some())
                        .collect();
                    assert_eq!(resolved.len(), usize::from(accepted), "{source:?}");
                    if accepted {
                        assert_eq!(resolved[0].label, label.to_lowercase());
                        assert_eq!(
                            doc.text(resolved[0].destination.unwrap()),
                            Some("/raw&amp;")
                        );
                        assert_eq!(doc.text(resolved[0].title.unwrap()), Some("t\\*"));
                    }
                    cases += 1;
                }
            }
        }
    }
    assert_eq!(cases, 768);
}

#[test]
fn invalid_definitions_remain_content_and_do_not_resolve_symbols() {
    let source = "[foo][ref[]\n\n[ref[]: /uri\n";
    let doc = parse(source);
    assert_eq!(definitions(&doc), 0);
    assert!(doc.references().iter().all(|r| r.destination.is_none()));
    let visible: String = doc
        .display_runs_in(Span::new(0, source.len()))
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    // Existing unresolved-reference fallback still hides matched brackets.
    assert_eq!(visible, "foo[ref[]\n\n[ref[]: /uri\n");
    for source in [
        "[[r]: /bad\n",
        "[r[x]: /bad\n",
        "[r\\[x[y]: /bad\n",
        "[r\\\\[x]: /bad\n",
    ] {
        assert_eq!(definitions(&parse(source)), 0, "{source:?}");
    }
    let mut doc = Document::new();
    doc.append("[r[x]: /bad").unwrap();
    assert!(doc.blocks().is_empty());
    assert!(doc.references().is_empty());
    doc.append("\n").unwrap();
    assert_eq!(definitions(&doc), 0);
    assert!(doc.references().iter().all(|r| r.destination.is_none()));
}

#[test]
fn every_split_prefix_and_eof_keeps_fallback_and_valid_definitions() {
    for source in [
        "[foo][ref[]\n\n[ref[]: /uri\n",
        "[中\\[x] [text][中\\[x]\r\n\r\n>\t [中\\[x]: /a&amp; '🙂'\r\n",
        "- seed\n\t[r[x]: /bad\n- next\n",
        "- - seed\n\t[r\\[x]: /yes\n",
        "[r\\[x]: /a 'open\n> next\n",
        "[r\\\\[x]: /bad",
        "[r\\]\\[x]: /yes\n",
        "[r[x]: /bad | cell\n--- | ---\n",
    ] {
        let expected = support::snapshot(&parse(source));
        for at in (0..=source.len()).filter(|&at| source.is_char_boundary(at)) {
            let mut doc = Document::new();
            doc.append(&source[..at]).unwrap();
            doc.append(&source[at..]).unwrap();
            doc.finish();
            assert_eq!(support::snapshot(&doc), expected, "{source:?}, {at}");
            link_metadata::snapshot(&doc);
        }
        let mut doc = Document::new();
        for (at, ch) in source.char_indices() {
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            let text = &source[..at + ch.len_utf8()];
            let mut prefix = Document::new();
            prefix.append(text).unwrap();
            assert_eq!(support::snapshot(&doc), support::snapshot(&prefix));
            assert_eq!(doc.pending(), prefix.pending());
            let mut eof = Document::new();
            eof.append(&source[..at]).unwrap();
            eof.append(&source[at..at + ch.len_utf8()]).unwrap();
            eof.finish();
            assert_eq!(support::snapshot(&eof), support::snapshot(&parse(text)));
        }
    }
}

#[test]
fn valid_escaped_label_keeps_identity_first_winner_and_raw_mapping() {
    let mut doc = Document::new();
    doc.append("[R\\[x] [R\\[x][] ![R\\[x] [text][R\\[x]\n\n")
        .unwrap();
    let targets = doc.links().to_vec();
    let infos: Vec<_> = (0..targets.len())
        .map(|i| *doc.link_info(LinkId(i)).unwrap())
        .collect();
    assert_eq!(targets.len(), 4);
    let update = doc.append(">\t [r\\[x]: /raw&amp; 't\\*'\n").unwrap();
    assert_eq!(update.references_changed, [SymbolId(0)]);
    assert!(update.new_links.is_empty());
    let winner = doc.references()[0].clone();
    let update = doc.append("[r[x]: /invalid\n[R\\[x]: /ignored\n").unwrap();
    assert!(update.references_changed.is_empty());
    doc.finish();
    assert_eq!(&doc.links()[..4], targets);
    assert_eq!(doc.references()[0], winner);
    for (i, info) in infos.iter().enumerate() {
        assert_eq!(doc.link_info(LinkId(i)), Some(info));
        assert_eq!(doc.link_destination(LinkId(i)), Some("/raw&amp;"));
        assert_eq!(doc.link_title(LinkId(i)), Some("t\\*"));
    }
    link_metadata::snapshot(&doc);
}

#[test]
fn bounded_window_length_and_literal_context_rules_stay_intact() {
    for prefix in ["", ">\t "] {
        for size in [
            BLOCK_LOOKAHEAD - 1,
            BLOCK_LOOKAHEAD,
            BLOCK_LOOKAHEAD + 1,
            2048,
        ] {
            for ending in ["\n", "\r\n", ""] {
                for label in ["r\\[", "r["] {
                    let base = format!("{prefix}[{label}]: /a");
                    let source = format!(
                        "{base}{}{ending}",
                        " ".repeat(size - base.len() - ending.len())
                    );
                    let accepted = label == "r\\["
                        && (size < BLOCK_LOOKAHEAD
                            || (size == BLOCK_LOOKAHEAD && !ending.is_empty()));
                    assert_eq!(
                        definitions(&parse(&source)),
                        usize::from(accepted),
                        "{size}, {ending:?}, {label:?}"
                    );
                }
            }
        }
    }
    for len in [MAX_LABEL_BYTES, MAX_LABEL_BYTES + 1] {
        let label = format!("{}\\[", "a".repeat(len - 2));
        assert_eq!(
            definitions(&parse(&format!("[{label}]: /a\n"))),
            usize::from(len == MAX_LABEL_BYTES)
        );
    }
    for source in [
        "    [r\\[]: /no\n",
        "```\n[r\\[]: /no\n```\n",
        "$$\n[r\\[]: /no\n$$\n",
        "[^r[x]: note\n",
        "[r[\nx]: /no\n",
    ] {
        assert_eq!(definitions(&parse(source)), 0);
    }
    // An escaped opener remains raw matching data across physical lines.
    let multiline = parse("[r\\[\nx]: /no\n");
    assert_eq!(definitions(&multiline), 1);
    assert_eq!(multiline.references()[0].label, "r\\[ x");
    // An opener in the destination or title is outside the label scanner.
    assert_eq!(definitions(&parse("[r]: /[x 't['\n")), 1);
}

#[test]
fn source_mapping_slices_and_shared_six_byte_budgets_agree() {
    let source = "[中\\[] 🙂 &amp;\r\n>\t [中\\[]: /raw&amp; 't\\*'\r\n";
    let doc = parse(source);
    let span = Span::new(0, source.len());
    let expected = "中[ 🙂 &\n";
    assert_eq!(
        doc.runs_in(span)
            .unwrap()
            .map(|r| r.text)
            .collect::<String>(),
        source
    );
    let visible = |range| {
        doc.display_runs_in(range)
            .unwrap()
            .filter(|r| !r.style.contains(Style::HIDDEN))
            .map(|r| r.text)
            .collect::<String>()
    };
    assert_eq!(visible(span), expected);
    assert_eq!(
        source
            .char_indices()
            .map(|(at, ch)| visible(Span::new(at, at + ch.len_utf8())))
            .collect::<String>(),
        expected
    );
    let mut cursor = doc.display_cursor(span).unwrap();
    let mut output = String::new();
    while cursor.source_position() < span.end {
        let at = cursor.source_position();
        assert_eq!(
            cursor
                .next(&doc, &mut Budget::new(WorkLimit::default()))
                .unwrap(),
            DisplayStep::BudgetExhausted
        );
        assert_eq!(cursor.source_position(), at);
        let mut budget = Budget::new(link_metadata::SIX_BYTES);
        let DisplayStep::Run(run) = cursor.next(&doc, &mut budget).unwrap() else {
            panic!("one scalar fits")
        };
        assert_eq!(run.source.start, at);
        if let Some(replacement) = run.replacement {
            assert_eq!(doc.text(replacement), Some("&amp;"));
            assert_eq!(run.text, if at == replacement.start { "&" } else { "" });
            assert!(replacement.start <= at && run.source.end <= replacement.end);
        } else {
            assert_eq!(run.text, &source[run.source.range()]);
        }
        assert_eq!(cursor.source_position(), run.source.end);
        assert!(run.source.end > at);
        assert!(
            budget.used().bytes <= 6 && budget.used().queries <= 1 && budget.used().objects <= 1
        );
        if !run.style.contains(Style::HIDDEN) {
            output.push_str(run.text);
        }
    }
    assert_eq!(output, expected);
    assert_eq!(
        cursor
            .next(&doc, &mut Budget::new(WorkLimit::default()))
            .unwrap(),
        DisplayStep::Complete
    );
    assert_eq!(
        link_metadata::snapshot(&doc),
        [Some("<a href=\"/raw&amp;\" title=\"t*\">".to_owned())]
    );
}

#[test]
fn deep_long_and_historical_inputs_keep_bounded_work() {
    let prefix = ">".repeat(980);
    let mut doc = Document::new();
    doc.append(&format!("{prefix} {}", "x".repeat(1_000_000)))
        .unwrap();
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("x").unwrap();
    }
    assert_eq!(doc.work().inline_bytes - before.inline_bytes, 1000);
    assert_eq!(doc.work().classified_bytes, before.classified_bytes);
    assert_eq!(doc.work().container_steps, before.container_steps);
    doc.append(&format!("\n{prefix}\n")).unwrap();
    let before = doc.work();
    doc.append(&format!("{prefix} [r\\[]: /yes\n")).unwrap();
    assert_eq!(definitions(&doc), 1);
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert_eq!(doc.work().label_bytes - before.label_bytes, 3);
    assert!(doc.work().classified_bytes - before.classified_bytes < 6 * BLOCK_LOOKAHEAD as u64);
    assert!(doc.work().container_steps - before.container_steps < 6000);

    let before = doc.work();
    doc.append(&format!("{prefix} [r[x]: /invalid\n")).unwrap();
    assert_eq!(definitions(&doc), 1);
    assert!(doc.work().classified_bytes - before.classified_bytes < 6 * BLOCK_LOOKAHEAD as u64);
    assert!(doc.work().container_steps - before.container_steps < 6000);
    assert!(doc.work().inline_bytes - before.inline_bytes < 32);

    let mut doc = Document::new();
    doc.append(&format!("[r[x]: /{}", "x".repeat(1_000_000)))
        .unwrap();
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("x").unwrap();
    }
    assert_eq!(doc.work().inline_bytes - before.inline_bytes, 1000);
    assert_eq!(doc.work().classified_bytes, before.classified_bytes);
    assert_eq!(doc.work().label_bytes, before.label_bytes);
    doc.finish();
    assert_eq!(definitions(&doc), 0);

    let mut doc = Document::new();
    doc.append(&format!("{}\n\n", "[r\\[] ".repeat(20_000)))
        .unwrap();
    assert_eq!(doc.links().len(), 20_000);
    let before = doc.work();
    let update = doc.append("[r\\[]: /late\n").unwrap();
    assert_eq!(update.references_changed, [SymbolId(0)]);
    assert!(update.new_links.is_empty());
    assert!(update.changes.len() < 12);
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert_eq!(doc.work().label_bytes - before.label_bytes, 3);
    assert!(doc.work().tree_visits - before.tree_visits < 2000);
    assert_eq!(doc.link_destination(LinkId(19_999)), Some("/late"));
}
