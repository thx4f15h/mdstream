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
fn escaped_closers_follow_backslash_parity_in_root_and_containers() {
    let mut cases = 0;
    for prefix in ["", "   ", "> ", "- ", "1. ", ">\t ", "- > ", "> - "] {
        for slashes in 0..8 {
            for base in ["Foo", "中🙂", "Foo\\*bar", "A\\[b"] {
                let raw = format!("{base}{}", "\\".repeat(slashes));
                // Even runs leave the first ] as the closer; odd runs escape
                // it, requiring one more ]. The raw spelling is the symbol key.
                let label = format!("{raw}{}", if slashes % 2 == 1 { "]" } else { "" });
                for ending in ["\n", "\r\n", ""] {
                    let source = format!("{prefix}[{label}]: /raw&amp; 't\\*'{ending}");
                    let doc = parse(&source);
                    assert_eq!(definitions(&doc), 1, "{source:?}");
                    let reference = doc
                        .references()
                        .iter()
                        .find(|r| r.destination.is_some())
                        .unwrap();
                    assert_eq!(reference.label, label.to_lowercase());
                    assert_eq!(doc.text(reference.destination.unwrap()), Some("/raw&amp;"));
                    assert_eq!(doc.text(reference.title.unwrap()), Some("t\\*"));
                    let block = doc
                        .blocks()
                        .iter()
                        .find(|b| matches!(b.kind, BlockKind::Definition { .. }))
                        .unwrap();
                    assert_eq!(block.state, SyntaxState::Sealed);
                    assert!(
                        doc.display_runs_in(block.source)
                            .unwrap()
                            .all(|r| r.style.contains(Style::HIDDEN))
                    );
                    let wrong_label = format!("{raw}{}", if slashes % 2 == 0 { "]" } else { "" });
                    let invalid = format!("{prefix}[{wrong_label}]: /bad{ending}");
                    assert_eq!(definitions(&parse(&invalid)), 0, "{invalid:?}");
                    cases += 2;
                }
            }
        }
    }
    assert_eq!(cases, 1536);
}

#[test]
fn failed_pending_and_window_candidates_keep_existing_boundaries() {
    for body in [
        "[r\\]: /no",
        "[r\\]] /no",
        "[r\\]]:",
        "[r\\]]: <open",
        "[r\\]]: /a 'open",
        "[r\\]]: /a 'title' extra",
        "[r\\]]:\n\n/url",
        "[r\\\n]]: /no",
        "[^r\\]]: /no",
    ] {
        assert_eq!(definitions(&parse(body)), 0, "{body:?}");
    }
    let mut doc = Document::new();
    doc.append("[r\\]]: /a").unwrap();
    assert!(doc.blocks().is_empty());
    assert!(doc.references().is_empty());
    doc.append(" 'open\n").unwrap();
    assert_eq!(definitions(&doc), 0);
    for prefix in ["", ">\t "] {
        for size in [
            BLOCK_LOOKAHEAD - 1,
            BLOCK_LOOKAHEAD,
            BLOCK_LOOKAHEAD + 1,
            2048,
        ] {
            for ending in ["\n", "\r\n", ""] {
                let base = format!("{prefix}[r\\]]: /a");
                let source = format!(
                    "{base}{}{ending}",
                    " ".repeat(size - base.len() - ending.len())
                );
                let accepted =
                    size < BLOCK_LOOKAHEAD || (size == BLOCK_LOOKAHEAD && !ending.is_empty());
                assert_eq!(
                    definitions(&parse(&source)),
                    usize::from(accepted),
                    "size {size}, {prefix:?}, {ending:?}"
                );
            }
        }
    }
    for len in [MAX_LABEL_BYTES, MAX_LABEL_BYTES + 1] {
        let label = format!("{}\\]", "a".repeat(len - 2));
        assert_eq!(
            definitions(&parse(&format!("[{label}]: /a\n"))),
            usize::from(len == MAX_LABEL_BYTES)
        );
    }
    for source in [
        "    [r\\]]: /no\n",
        "```\n[r\\]]: /no\n```\n",
        "$$\n[r\\]]: /no\n$$\n",
    ] {
        assert_eq!(definitions(&parse(source)), 0);
    }
}

#[test]
fn every_split_scalar_prefix_and_eof_preserves_semantics() {
    for source in [
        "[Foo*bar\\]]:my_(url) 'title (with parens)'\n\n[Foo*bar\\]]\n",
        "[中\\]]\r\n>\t [中\\]]: /raw&amp; '🙂'\r\n",
        "- seed\n\t[r\\]]: /url\n- next\n",
        "- - seed\n\t[r\\]]: /url\n",
        "[r\\]][] ![r\\]] [text][r\\]]\n\n[r\\]]: /url\n",
        "[r\\]]: /a 'open\n> next\n",
        "[r\\]: /bad\n",
        "[r\\\\]: /yes",
        "[r\\\\]]: /bad\n",
    ] {
        let whole = support::snapshot(&parse(source));
        for split in (0..=source.len()).filter(|&at| source.is_char_boundary(at)) {
            let mut doc = Document::new();
            doc.append(&source[..split]).unwrap();
            doc.append(&source[split..]).unwrap();
            doc.finish();
            assert_eq!(support::snapshot(&doc), whole, "{source:?}, split {split}");
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
fn late_definition_preserves_identity_first_winner_and_raw_label_matching() {
    let mut doc = Document::new();
    doc.append("[R\\]] [R\\]][] ![R\\]] [text][R\\]]\n\n")
        .unwrap();
    let links = doc.links().to_vec();
    let infos: Vec<_> = (0..links.len())
        .map(|i| *doc.link_info(LinkId(i)).unwrap())
        .collect();
    assert_eq!(links.len(), 4);
    let update = doc.append(">\t [r\\]]: /raw&amp; 't\\*'\n").unwrap();
    assert_eq!(update.references_changed, [SymbolId(0)]);
    assert!(update.new_links.is_empty());
    assert!(
        update
            .changes
            .iter()
            .any(|c| c.kind == ChangeKind::Reference)
    );
    let revision = doc.references()[0].revision;
    let update = doc.append("[R\\]]: /ignored\n").unwrap();
    assert!(update.references_changed.is_empty());
    doc.finish();
    assert_eq!(doc.links(), links);
    assert_eq!(doc.references()[0].revision, revision);
    for (i, info) in infos.iter().enumerate() {
        assert_eq!(doc.link_info(LinkId(i)), Some(info));
        // Full, shortcut/collapsed links and images share the raw label key.
        assert_eq!(doc.link_destination(LinkId(i)), Some("/raw&amp;"));
        assert_eq!(doc.link_title(LinkId(i)), Some("t\\*"));
    }
    link_metadata::snapshot(&doc);
    // Matching retains raw escapes; equal display strings are not equal keys.
    let doc = parse("[a*] [a\\*]\n\n[a\\*]: /escaped\n");
    assert_eq!(doc.link_destination(LinkId(0)), None);
    assert_eq!(doc.link_destination(LinkId(1)), Some("/escaped"));
}

#[test]
fn source_mapping_slices_and_shared_six_byte_budgets_agree() {
    let source = "[中\\]] 🙂 &amp;\r\n>\t [中\\]]: /raw&amp; 't\\*'\r\n";
    let doc = parse(source);
    let span = Span::new(0, source.len());
    let expected = "中] 🙂 &\n";
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
    doc.append(&format!("{prefix} [r\\]]: /yes\n")).unwrap();
    assert_eq!(definitions(&doc), 1);
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert_eq!(doc.work().label_bytes - before.label_bytes, 3);
    assert!(doc.work().classified_bytes - before.classified_bytes < 6 * BLOCK_LOOKAHEAD as u64);
    assert!(doc.work().container_steps - before.container_steps < 6000);

    let mut doc = Document::new();
    doc.append(&format!("[r\\]]: /{}", "x".repeat(1_000_000)))
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
    doc.append(&format!("{}\n\n", "[r\\]] ".repeat(20_000)))
        .unwrap();
    assert_eq!(doc.links().len(), 20_000);
    let before = doc.work();
    let update = doc.append("[r\\]]: /late\n").unwrap();
    assert_eq!(update.references_changed, [SymbolId(0)]);
    assert!(update.new_links.is_empty());
    assert!(update.changes.len() < 12);
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert_eq!(doc.work().label_bytes - before.label_bytes, 3);
    assert!(doc.work().tree_visits - before.tree_visits < 2000);
    assert_eq!(doc.link_destination(LinkId(19_999)), Some("/late"));
}
