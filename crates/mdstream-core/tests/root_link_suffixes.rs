use mdstream::*;

#[path = "support/snapshot.rs"]
mod support;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

const NON_TARGETS: &[&str] = &[
    "> [a](u\n> 't')\n",
    "> ![a](u\r\n> )\n",
    "- [a](\n  u)\n",
    "> [a](u\nlazy)\n",
    ">\t![a](u\n>\t't')\n",
    "# [a](u\n)\n",
    "[a](u|b\n-|-\n 't')|c\n",
    "`[a](u\n)`\n",
    "$[a](u\n)$\n",
    "[^f]: [a](u\n 't')\n",
    "[a](<u\nrest>)\n",
    "[a](u(\nrest))\n",
    "[a](u 't\nrest')\n",
    "[a](u\nbar)\n",
    "![a][r\nrest]\n",
    "[a]\n(u)\n",
    "[a](u\n\n 't')\n",
    "[a](u\n# h\n)\n",
    "[a](u\n    code\n)\n",
    "[a](u\n~~~\nraw\n~~~\n)\n",
    "[a](u\n---\n)\n",
    "[a](u\nh|h\n-|-\n)\n",
    "[a](u\n> 't')\n",
    "[a](u\r 't')\n",
];

#[test]
fn non_target_snapshots_match_real_prechange_baseline() {
    let mut actual = String::new();
    for source in NON_TARGETS {
        actual.push_str(&format!("{:?}\n", support::snapshot(&parse(source))));
    }
    assert_eq!(
        actual,
        include_str!("fixtures/root_link_suffix_baseline.txt")
    );
}

#[test]
fn root_component_separators_have_independent_line_ending_allowances() {
    for image in ["", "!"] {
        for ending in ["\n", "\r\n"] {
            for (suffix, destination, title) in [
                (format!("({ending}u)"), "u", None),
                (format!("(u{ending})"), "u", None),
                (format!("(u{ending} 't')"), "u", Some("t")),
                (format!("(u 't'{ending})"), "u", Some("t")),
                (
                    format!("( {ending}u {ending} 't' {ending})"),
                    "u",
                    Some("t"),
                ),
                (format!("({ending})"), "", None),
                (format!("(<>{ending} 't'{ending})"), "", Some("t")),
            ] {
                let source = format!("{image}[中🙂]{suffix}\n");
                let doc = parse(&source);
                assert_eq!(doc.links().len(), 1, "{source:?}");
                assert_eq!(
                    doc.link_destination(LinkId(0)),
                    Some(destination),
                    "{source:?}"
                );
                assert_eq!(doc.link_title(LinkId(0)), title, "{source:?}");
                let info = doc.link_info(LinkId(0)).unwrap();
                assert_eq!(info.source, Span::new(0, source.len() - 1));
                assert_eq!(
                    info.kind,
                    if image.is_empty() {
                        LinkKind::Link
                    } else {
                        LinkKind::Image
                    }
                );
            }
        }
    }
}

#[test]
fn generated_components_preserve_original_spans_and_multiline_labels() {
    let gaps = ["", " \t ", "\n", "\r\n", " \r\n  "];
    let mut cases = 0;
    for image in ["", "!"] {
        for (target, raw) in [
            ("u", "u"),
            ("a(b)c", "a(b)c"),
            ("a\\)b", "a\\)b"),
            ("<a b>", "a b"),
            ("<中&amp;>", "中&amp;"),
            ("<>", ""),
        ] {
            for (title, raw_title) in [
                ("'t'", "t"),
                ("\"中&amp;\\\"\"", "中&amp;\\\""),
                ("(a\\)b)", "a\\)b"),
            ] {
                for before in gaps {
                    for between in &gaps[1..] {
                        for after in gaps {
                            let prefix = format!("{image}[中&amp;\n🙂](");
                            let source =
                                format!("{prefix}{before}{target}{between}{title}{after})\n");
                            let doc = parse(&source);
                            let id = LinkId(0);
                            assert_eq!(doc.links().len(), 1, "{source:?}");
                            let start =
                                prefix.len() + before.len() + usize::from(target.starts_with('<'));
                            assert_eq!(
                                doc.links()[0],
                                LinkTarget::Direct(Span::new(start, start + raw.len())),
                                "{source:?}"
                            );
                            assert_eq!(doc.link_destination(id), Some(raw));
                            assert_eq!(doc.link_url(id).as_deref(), Some(raw));
                            assert_eq!(doc.link_title(id), Some(raw_title));
                            let info = doc.link_info(id).unwrap();
                            let title_start =
                                prefix.len() + before.len() + target.len() + between.len() + 1;
                            assert_eq!(
                                info.title,
                                Some(Span::new(title_start, title_start + raw_title.len()))
                            );
                            assert_eq!(doc.text(info.label), Some("中&amp;\n🙂"));
                            assert_eq!(info.state, SyntaxState::Sealed);
                            assert_eq!(info.source, Span::new(0, source.len() - 1));
                            let displayed: String = doc
                                .display_runs_in(info.label)
                                .unwrap()
                                .filter(|run| !run.style.contains(Style::HIDDEN))
                                .map(|run| run.text)
                                .collect();
                            assert_eq!(displayed, "中&\n🙂");
                            cases += 1;
                        }
                    }
                }
            }
        }
    }
    assert_eq!(cases, 3600);
}

#[test]
fn every_utf8_byte_split_scalar_prefix_and_eof_preserve_pending_and_failed_states() {
    for source in [
        "[中\n🙂](\n<a b&amp;>\r\n 't&amp;\\\'' \n)\n",
        "![中&amp;](u\n\"title\"\r\n)\n",
        "[a](u\r\n 't'x)\n",
        "![中](<u\r\nrest>)\n",
        "[a](u\n 't\nrest')\n",
        "[a](u\r\n\r\n)\n",
        "[a](u\n# h\n)\n",
        "[a](u\n---\n)\n",
        "[a](u\nh|h\n-|-\n)\n",
        "[a](u\n> )\n",
        "[a](u \"t\"\r",
        "[a](\r",
    ] {
        let whole = support::snapshot(&parse(source));
        for split in 0..=source.len() {
            let mut stream = Utf8Document::new();
            stream.append(&source.as_bytes()[..split]).unwrap();
            stream.append(&source.as_bytes()[split..]).unwrap();
            stream.finish();
            assert_eq!(
                support::snapshot(stream.document()),
                whole,
                "{source:?} split {split}"
            );
            // EOF in the middle of a scalar follows Utf8Document's existing
            // replacement contract, independently of the suffix state.
            let mut prefix = Utf8Document::new();
            for byte in &source.as_bytes()[..split] {
                prefix.append(&[*byte]).unwrap();
            }
            let delivered = prefix.document().source();
            let mut expected = Document::new();
            expected.append(delivered).unwrap();
            assert_eq!(
                support::snapshot(prefix.document()),
                support::snapshot(&expected)
            );
            prefix.finish();
            assert_eq!(
                support::snapshot(prefix.document()),
                support::snapshot(&parse(&String::from_utf8_lossy(
                    &source.as_bytes()[..split]
                )))
            );
        }
        let mut doc = Document::new();
        for (at, ch) in source.char_indices() {
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            let mut expected = Document::new();
            expected.append(&source[..at + ch.len_utf8()]).unwrap();
            assert_eq!(support::snapshot(&doc), support::snapshot(&expected));
        }
        doc.finish();
        assert_eq!(support::snapshot(&doc), whole);
    }
}

#[test]
fn published_shortcut_upgrades_atomically_or_keeps_its_identity_on_failure_and_eof() {
    for image in ["", "!"] {
        for suffix in ["(\nu\n 't'\n)", "(u\n 't'x)", "(u\n 't'", "(u\r", "(u\n\n)"] {
            let mut doc = Document::new();
            let mut batch = doc
                .append(&format!("p {} {image}[old]", "x".repeat(1100)))
                .unwrap();
            let id = LinkId(0);
            let original = *doc.link_info(id).unwrap();
            let LinkTarget::Reference(symbol) = doc.links()[0] else {
                panic!("expected shortcut")
            };
            let reference = doc.references()[symbol.0].clone();
            let block = BlockId(0);
            assert_eq!(original.state, SyntaxState::Provisional);
            let complete = suffix == "(\nu\n 't'\n)";
            for ch in suffix.chars() {
                let update = doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
                assert!(update.new_links.is_empty());
                batch.merge(update).unwrap();
                if doc.link_info(id).unwrap().source == original.source {
                    assert_eq!(doc.links()[0], LinkTarget::Reference(symbol));
                    assert_eq!(doc.link_title(id), None);
                }
            }
            batch.merge(doc.finish()).unwrap();
            let info = doc.link_info(id).unwrap();
            assert_eq!(info.label, original.label);
            assert_eq!(info.kind, original.kind);
            assert_eq!(info.state, SyntaxState::Sealed);
            assert_eq!(doc.block(block).unwrap().source.start, 0);
            assert_eq!(doc.block(block).unwrap().container, None);
            assert_eq!(doc.references()[symbol.0], reference);
            assert_eq!(doc.links().len(), 1);
            assert_eq!(batch.new_links, 0..1);
            assert_eq!(doc.link_destination(id), complete.then_some("u"));
            assert_eq!(doc.link_title(id), complete.then_some("t"));
            if !complete {
                assert_eq!(info.source, original.source);
            }
        }
    }
}

#[test]
fn raw_metadata_and_source_display_slices_share_zero_one_and_six_byte_budgets() {
    for image in ["", "!"] {
        let source =
            format!("{image}[中&amp;\r\n🙂](\r\n<a b%20中\\\\&amp;>\n 't&#10;&amp;\\\''\r\n)\n");
        let doc = parse(&source);
        let id = LinkId(0);
        assert_eq!(doc.link_destination(id), Some("a b%20中\\\\&amp;"));
        assert_eq!(doc.link_title(id), Some("t&#10;&amp;\\'"));
        assert_eq!(doc.link_url(id).as_deref(), Some("a b%20中\\\\&amp;"));
        let mut destination = doc.decoded_link_destination(id).unwrap();
        let mut title = doc.decoded_link_title(id).unwrap();
        let span = doc.link_info(id).unwrap().label;
        let mut display = doc.display_cursor(span).unwrap();
        let mut zero = Budget::new(WorkLimit::default());
        assert_eq!(
            destination.advance(&mut zero),
            Err(Completion::BudgetExhausted)
        );
        assert_eq!(title.advance(&mut zero), Err(Completion::BudgetExhausted));
        assert_eq!(
            display.next(&doc, &mut zero).unwrap(),
            DisplayStep::BudgetExhausted
        );
        assert_eq!(zero.used(), WorkLimit::default());
        let mut strings = [String::new(), String::new(), String::new()];
        let mut done = [false; 3];
        for frame in 0..1000 {
            let mut budget = Budget::new(WorkLimit {
                bytes: if frame % 2 == 0 { 1 } else { 6 },
                queries: 1,
                objects: 1,
                ..WorkLimit::default()
            });
            // Rotate readers so all share and eventually spend the same budget.
            for index in (0..3).map(|i| (i + frame) % 3) {
                if done[index] {
                    continue;
                }
                match index {
                    0 => match destination.advance(&mut budget) {
                        Ok(Some(chunk)) => {
                            assert!(doc.text(chunk.source).is_some());
                            strings[0].push_str(chunk.text);
                        }
                        Ok(None) => done[0] = true,
                        Err(Completion::BudgetExhausted) => {}
                        other => panic!("{other:?}"),
                    },
                    1 => match title.advance(&mut budget) {
                        Ok(Some(chunk)) => {
                            assert!(doc.text(chunk.source).is_some());
                            strings[1].push_str(chunk.text);
                        }
                        Ok(None) => done[1] = true,
                        Err(Completion::BudgetExhausted) => {}
                        other => panic!("{other:?}"),
                    },
                    _ => match display.next(&doc, &mut budget).unwrap() {
                        DisplayStep::Run(run) => {
                            assert!(doc.text(run.source).is_some());
                            if !run.style.contains(Style::HIDDEN) {
                                strings[2].push_str(run.text);
                            }
                        }
                        DisplayStep::Complete => done[2] = true,
                        DisplayStep::BudgetExhausted => {}
                    },
                }
            }
            assert!(budget.used().bytes <= if frame % 2 == 0 { 1 } else { 6 });
            assert!(budget.used().queries <= 1 && budget.used().objects <= 1);
            if done == [true; 3] {
                break;
            }
        }
        assert_eq!(done, [true; 3]);
        assert_eq!(
            strings,
            [
                "a b%20中\\&".to_owned(),
                "t\n&'".to_owned(),
                "中&\n🙂".to_owned()
            ]
        );
        for (at, ch) in source.char_indices() {
            let raw: String = doc
                .runs_in(Span::new(at, at + ch.len_utf8()))
                .unwrap()
                .map(|run| run.text)
                .collect();
            assert_eq!(raw, ch.to_string());
            for run in doc
                .display_runs_in(Span::new(at, at + ch.len_utf8()))
                .unwrap()
            {
                assert!(doc.text(run.source).is_some());
            }
        }
        // A decoded newline in metadata is payload, never a physical separator.
        let doc = parse(&format!("{image}[a](u&#10;v 't&#10;x')\n"));
        assert_eq!(doc.link_destination(id), Some("u&#10;v"));
        assert_eq!(doc.link_title(id), Some("t&#10;x"));
    }
}

#[test]
fn opaque_failure_and_eof_keep_newly_consumed_entities_and_punctuation_raw() {
    for image in ["", "!"] {
        for ending in ["\n", "\r\n"] {
            for payload in ["a&amp;", "中🙂&#10;\\*`x`"] {
                for tail in [format!("'x){ending}"), String::new()] {
                    let source = format!("{image}[a](u{ending} '{payload}{tail}");
                    let doc = parse(&source);
                    let shown: String = doc
                        .display_runs_in(Span::new(0, source.len()))
                        .unwrap()
                        .filter(|run| !run.style.contains(Style::HIDDEN))
                        .map(|run| run.text)
                        .collect();
                    let visible_tail = if tail.is_empty() { "" } else { "'x)\n" };
                    assert_eq!(
                        shown,
                        format!("a(u\n'{payload}{visible_tail}"),
                        "{source:?}"
                    );
                    // Before root multiline admission these entities were ordinary
                    // prose after the rejected LF and decoded. This is new opaque
                    // scope, not a claim of baseline-equivalent literal fallback.
                    let start = source.find(payload).unwrap();
                    let raw = Span::new(start, start + payload.len());
                    assert_eq!(doc.text(raw), Some(payload));
                    let runs: Vec<_> = doc.display_runs_in(raw).unwrap().collect();
                    assert!(runs.iter().all(|run| {
                        run.replacement.is_none()
                            && run.text == doc.text(run.source).unwrap()
                            && !run.style.contains(Style::HIDDEN)
                            && !run.style.contains(Style::CODE)
                    }));
                    assert_eq!(runs.iter().map(|run| run.text).collect::<String>(), payload);
                    assert_eq!(doc.links(), &[LinkTarget::Reference(SymbolId(0))]);
                    assert_eq!(
                        *doc.link_info(LinkId(0)).unwrap(),
                        LinkInfo {
                            kind: if image.is_empty() {
                                LinkKind::Link
                            } else {
                                LinkKind::Image
                            },
                            label: Span::new(image.len() + 1, image.len() + 2),
                            source: Span::new(0, image.len() + 3),
                            title: None,
                            state: SyntaxState::Sealed,
                        }
                    );
                    assert_eq!(
                        doc.references(),
                        &[Reference {
                            label: "a".to_owned(),
                            destination: None,
                            title: None,
                            revision: 0,
                            first_use: Some(0),
                        }]
                    );
                    assert_eq!(doc.link_destination(LinkId(0)), None);
                    assert_eq!(doc.link_url(LinkId(0)), None);
                    assert_eq!(doc.link_title(LinkId(0)), None);
                    assert!(doc.decoded_link_destination(LinkId(0)).is_none());
                    assert!(doc.decoded_link_title(LinkId(0)).is_none());
                    for split in 0..=source.len() {
                        let mut stream = Utf8Document::new();
                        stream.append(&source.as_bytes()[..split]).unwrap();
                        stream.append(&source.as_bytes()[split..]).unwrap();
                        stream.finish();
                        assert_eq!(
                            support::snapshot(stream.document()),
                            support::snapshot(&doc)
                        );
                    }
                    let mut stream = Utf8Document::new();
                    for byte in source.bytes() {
                        stream.append(&[byte]).unwrap();
                    }
                    stream.finish();
                    assert_eq!(
                        support::snapshot(stream.document()),
                        support::snapshot(&doc)
                    );
                }
            }
        }
    }
}

#[test]
fn opaque_failure_utf8_splits_preserve_published_shortcut_and_revision_chain() {
    for image in ["", "!"] {
        for ending in ["\n", "\r\n"] {
            for tail in [format!("'&amp;){ending}"), String::new()] {
                let label = format!("p {} {image}[中🙂]", "x".repeat(1100));
                let suffix = format!("(u{ending} '中🙂&amp;{tail}");
                let source = format!("{label}{suffix}");
                let expected = parse(&source);
                for split in 0..=suffix.len() {
                    let mut stream = Utf8Document::new();
                    stream.append(label.as_bytes()).unwrap();
                    stream.append(&suffix.as_bytes()[..split]).unwrap();
                    stream.append(&suffix.as_bytes()[split..]).unwrap();
                    stream.finish();
                    assert_eq!(
                        support::snapshot(stream.document()),
                        support::snapshot(&expected)
                    );
                    let mut prefix = Utf8Document::new();
                    prefix.append(label.as_bytes()).unwrap();
                    prefix.append(&suffix.as_bytes()[..split]).unwrap();
                    prefix.finish();
                    let delivered = format!(
                        "{label}{}",
                        String::from_utf8_lossy(&suffix.as_bytes()[..split])
                    );
                    assert_eq!(
                        support::snapshot(prefix.document()),
                        support::snapshot(&parse(&delivered))
                    );
                }
                let mut doc = Document::new();
                let mut batch = doc.append(&label).unwrap();
                let original = *doc.link_info(LinkId(0)).unwrap();
                let reference = doc.references()[0].clone();
                assert_eq!(original.state, SyntaxState::Provisional);
                for ch in suffix.chars() {
                    let revision = doc.revision();
                    let update = doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
                    assert_eq!(
                        (update.base_revision, update.revision),
                        (revision, revision + 1)
                    );
                    assert!(update.new_links.is_empty());
                    batch.merge(update).unwrap();
                    let mut info = *doc.link_info(LinkId(0)).unwrap();
                    info.state = original.state;
                    assert_eq!(info, original);
                    assert_eq!(doc.links(), &[LinkTarget::Reference(SymbolId(0))]);
                    assert_eq!(doc.references(), std::slice::from_ref(&reference));
                }
                let revision = doc.revision();
                let update = doc.finish();
                assert_eq!(
                    (update.base_revision, update.revision),
                    (revision, revision + 1)
                );
                batch.merge(update).unwrap();
                assert_eq!(batch.new_links, 0..1);
                assert_eq!(batch.revision, doc.revision());
                assert_eq!(doc.link_info(LinkId(0)).unwrap().state, SyntaxState::Sealed);
                assert_eq!(support::snapshot(&doc), support::snapshot(&expected));
                let shown: String = doc
                    .display_runs_in(Span::new(label.len(), source.len()))
                    .unwrap()
                    .filter(|run| !run.style.contains(Style::HIDDEN))
                    .map(|run| run.text)
                    .collect();
                // The rejecting '&' starts an ordinary entity exactly once.
                assert_eq!(
                    shown,
                    format!(
                        "(u\n'中🙂&amp;{}",
                        if tail.is_empty() { "" } else { "'&)\n" }
                    )
                );
                let revision = doc.revision();
                doc.finish();
                assert_eq!(doc.revision(), revision);
            }
        }
    }
}

#[test]
fn opaque_failure_source_display_and_decoded_controls_share_micro_budgets() {
    for image in ["", "!"] {
        for ending in ["\n", "\r\n"] {
            for tail in [format!("'&amp;){ending}"), String::new()] {
                let prefix = "[b](<p&amp;中> 't&#10;') ";
                let source = format!("{prefix}{image}[a](u{ending} '中🙂&#10;\\*{tail}");
                let doc = parse(&source);
                let whole = Span::new(0, source.len());
                let raw: String = doc.runs_in(whole).unwrap().map(|run| run.text).collect();
                assert_eq!(raw, source);
                let expected = format!(
                    "b a(u\n'中🙂&#10;\\*{}",
                    if tail.is_empty() { "" } else { "'&)\n" }
                );
                for (at, ch) in source.char_indices() {
                    let span = Span::new(at, at + ch.len_utf8());
                    assert_eq!(
                        doc.runs_in(span)
                            .unwrap()
                            .map(|run| run.text)
                            .collect::<String>(),
                        ch.to_string()
                    );
                    // Joining scalar slices must preserve the same mapping too.
                    assert!(
                        doc.display_runs_in(span)
                            .unwrap()
                            .all(|run| doc.text(run.source).is_some())
                    );
                }
                let sliced: String = source
                    .char_indices()
                    .flat_map(|(at, ch)| {
                        doc.display_runs_in(Span::new(at, at + ch.len_utf8()))
                            .unwrap()
                    })
                    .filter(|run| !run.style.contains(Style::HIDDEN))
                    .map(|run| run.text)
                    .collect();
                assert_eq!(sliced, expected);
                let mut display = doc.display_cursor(whole).unwrap();
                let mut destination = doc.decoded_link_destination(LinkId(0)).unwrap();
                let mut title = doc.decoded_link_title(LinkId(0)).unwrap();
                let positions = (
                    display.source_position(),
                    destination.source_position(),
                    title.source_position(),
                );
                let mut zero = Budget::new(WorkLimit::default());
                assert_eq!(
                    display.next(&doc, &mut zero).unwrap(),
                    DisplayStep::BudgetExhausted
                );
                assert_eq!(
                    destination.advance(&mut zero),
                    Err(Completion::BudgetExhausted)
                );
                assert_eq!(title.advance(&mut zero), Err(Completion::BudgetExhausted));
                assert_eq!(zero.used(), WorkLimit::default());
                assert_eq!(
                    (
                        display.source_position(),
                        destination.source_position(),
                        title.source_position()
                    ),
                    positions
                );
                let mut strings = [String::new(), String::new(), String::new()];
                let mut done = [false; 3];
                let mut charged = [0; 2];
                for frame in 0..1000 {
                    let bytes = if frame % 2 == 0 { 1 } else { 6 };
                    let mut budget = Budget::new(WorkLimit {
                        bytes,
                        queries: 1,
                        objects: 1,
                        ..WorkLimit::default()
                    });
                    for index in (0..3).map(|i| (i + frame) % 3) {
                        if done[index] {
                            continue;
                        }
                        match index {
                            0 => match display.next(&doc, &mut budget).unwrap() {
                                DisplayStep::Run(run) => {
                                    assert!(doc.text(run.source).is_some());
                                    if !run.style.contains(Style::HIDDEN) {
                                        strings[0].push_str(run.text);
                                    }
                                }
                                DisplayStep::Complete => done[0] = true,
                                DisplayStep::BudgetExhausted => {}
                            },
                            1 => match destination.advance(&mut budget) {
                                Ok(Some(chunk)) => {
                                    assert!(doc.text(chunk.source).is_some());
                                    strings[1].push_str(chunk.text);
                                }
                                Ok(None) => done[1] = true,
                                Err(Completion::BudgetExhausted) => {}
                                other => panic!("{other:?}"),
                            },
                            _ => match title.advance(&mut budget) {
                                Ok(Some(chunk)) => {
                                    assert!(doc.text(chunk.source).is_some());
                                    strings[2].push_str(chunk.text);
                                }
                                Ok(None) => done[2] = true,
                                Err(Completion::BudgetExhausted) => {}
                                other => panic!("{other:?}"),
                            },
                        }
                    }
                    assert!(budget.used().bytes <= bytes);
                    assert!(budget.used().queries <= 1 && budget.used().objects <= 1);
                    charged[frame % 2] += budget.used().bytes;
                    if done == [true; 3] {
                        break;
                    }
                }
                assert_eq!(done, [true; 3]);
                assert!(charged.iter().all(|&bytes| bytes > 0));
                assert_eq!(strings, [expected, "p&中".to_owned(), "t\n".to_owned()]);
            }
        }
    }
}

#[test]
fn existing_raw_label_target_title_and_suffix_limits_are_independent() {
    for image in ["", "!"] {
        for label_len in [999, 1000] {
            for target_len in [MAX_DESTINATION_BYTES, MAX_DESTINATION_BYTES + 1] {
                let source = format!(
                    "{image}[{}](\n{}\n)\n",
                    "a".repeat(label_len),
                    "u".repeat(target_len)
                );
                let doc = parse(&source);
                if target_len == MAX_DESTINATION_BYTES {
                    assert_eq!(doc.links().len(), 1);
                    assert_eq!(doc.link_destination(LinkId(0)).unwrap().len(), target_len);
                    assert_eq!(doc.link_info(LinkId(0)).unwrap().label.len(), label_len);
                } else {
                    assert_eq!(doc.links().len(), usize::from(label_len == 999));
                    assert_eq!(doc.link_destination(LinkId(0)), None);
                }
            }
        }
        for title_len in [MAX_LINK_TITLE_BYTES, MAX_LINK_TITLE_BYTES + 1] {
            let doc = parse(&format!("{image}[a](u\n '{}')\n", "t".repeat(title_len)));
            assert_eq!(doc.links().len(), 1);
            assert_eq!(
                doc.link_title(LinkId(0)).map(str::len),
                (title_len == MAX_LINK_TITLE_BYTES).then_some(title_len)
            );
        }
        for ending in ["\n", "\r\n"] {
            for extra in [0, 1] {
                // Keep the long whitespace on the first line: a new line
                // beginning with four spaces remains a code interruption.
                let spaces = MAX_LINK_SUFFIX_BYTES - 6 - ending.len() + extra;
                let doc = parse(&format!(
                    "{image}[a](u{}{ending} 't')\n",
                    " ".repeat(spaces)
                ));
                assert_eq!(doc.link_destination(LinkId(0)), (extra == 0).then_some("u"));
            }
        }
    }
    for ending in ["\n", "\r\n"] {
        for suffix in [
            format!("(u{ending} 't'{ending}"),
            format!("(u{ending} 't'x)"),
        ] {
            let mut doc = Document::new();
            doc.append(&format!("[a]{suffix}")).unwrap();
            assert_eq!(doc.link_destination(LinkId(0)), None);
            doc.finish();
            assert_eq!(doc.link_destination(LinkId(0)), None);
            assert_eq!(doc.link_title(LinkId(0)), None);
        }
    }
}

#[test]
fn suffix_work_is_new_bytes_only_with_twenty_thousand_old_objects_and_late_definition() {
    let mut doc = Document::new();
    doc.append(&format!("{}\n\n", "[r] ".repeat(20_000)))
        .unwrap();
    let old_links = doc.links().to_vec();
    let old_infos: Vec<_> = (0..20_000)
        .map(|i| *doc.link_info(LinkId(i)).unwrap())
        .collect();
    let old_blocks = doc.blocks().to_vec();
    doc.append(&format!(
        "p {} ![new](\nu {}'",
        "x".repeat(1100),
        " ".repeat(1100)
    ))
    .unwrap();
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("t").unwrap();
    }
    let after = doc.work();
    assert_eq!(after.input_bytes - before.input_bytes, 1000);
    // The current physical line can still belong to a table. Only the old
    // grammar observer consumes it until the no-pipe line ending releases it.
    assert_eq!(after.inline_bytes, before.inline_bytes);
    assert_eq!(
        after.root_eligibility_bytes - before.root_eligibility_bytes,
        1000
    );
    assert_eq!(after.table_header_bytes - before.table_header_bytes, 1000);
    assert_eq!(after.label_bytes, before.label_bytes);
    assert_eq!(after.tree_visits, before.tree_visits);
    let update = doc.append("')\n\n").unwrap();
    assert!(update.new_links.is_empty());
    assert_eq!(doc.link_destination(LinkId(20_000)), Some("u"));
    let before = doc.work();
    let update = doc.append("[r]: /late 'title'\n").unwrap();
    assert_eq!(update.references_changed.len(), 1);
    assert!(doc.work().inline_bytes - before.inline_bytes < 64);
    assert!(doc.work().tree_visits - before.tree_visits < 1000);
    assert_eq!(&doc.links()[..20_000], old_links);
    assert_eq!(&doc.blocks()[..old_blocks.len()], old_blocks);
    for (i, info) in old_infos.iter().enumerate() {
        assert_eq!(doc.link_info(LinkId(i)), Some(info));
        assert_eq!(doc.link_destination(LinkId(i)), Some("/late"));
        assert_eq!(doc.link_title(LinkId(i)), Some("title"));
    }
    eprintln!("old-object work before={before:?} after={:?}", doc.work());
}

#[test]
fn long_unfinished_and_failed_suffixes_do_not_replay_into_a_million_byte_tail() {
    for (body, closed) in [(" ", false), ("t", false), ("t", true)] {
        let mut doc = Document::new();
        let source = if body == " " { "[a](\nu" } else { "[a](u\n '" };
        doc.append(source).unwrap();
        // Exceed both title and suffix limits. Once rejected, no old prefix
        // enters the scanner again, regardless of later plain appends or EOF.
        for _ in 0..10 {
            doc.append(&body.repeat(1000)).unwrap();
        }
        if closed {
            doc.append("')\n").unwrap();
        }
        let before_tail = doc.work();
        for _ in 0..1000 {
            doc.append(&"x".repeat(1000)).unwrap();
        }
        let after = doc.work();
        assert_eq!(after.input_bytes - before_tail.input_bytes, 1_000_000);
        assert!(after.inline_bytes - before_tail.inline_bytes <= 1_002_048);
        assert_eq!(after.label_bytes, before_tail.label_bytes);
        assert_eq!(after.entity_bytes, before_tail.entity_bytes);
        assert!(after.tree_visits - before_tail.tree_visits < 2000);
        doc.finish();
        let final_work = doc.work();
        assert!(final_work.inline_bytes <= final_work.input_bytes);
        assert!(final_work.classified_bytes <= 3 * final_work.input_bytes);
        assert_eq!(doc.links().len(), 1);
        assert_eq!(doc.link_destination(LinkId(0)), None);
        eprintln!("long suffix {body:?}/{closed}: {final_work:?}");
    }
}

#[test]
fn pending_suffix_stays_open_for_folds_and_upgrade_invalidates_tickets_and_readers() {
    let mut doc = Document::new();
    doc.append(&format!("p {} [a](u\n", "x".repeat(1100)))
        .unwrap();
    assert_eq!(doc.source_fold_limit(), 0);
    let mut view = FoldView::new(&doc, 1).unwrap();
    let mut merged = FoldView::new(&doc, 1).unwrap();
    assert!(SourceFoldRegistration::paragraph(&doc, &view, BlockId(0), 2).is_err());
    assert!(SourceFoldRegistration::new(&doc, &view, Span::new(0, 2)).is_err());
    let mut task = SourceFoldRegistration::open_paragraph(&doc, &view, BlockId(0), 2).unwrap();
    let mut id = None;
    for _ in 0..30 {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            ..WorkLimit::default()
        });
        let progress = task.advance(&doc, &mut view, &mut budget).unwrap();
        assert_eq!(budget.used().bytes, 0);
        if progress.id.is_some() {
            id = progress.id;
            break;
        }
    }
    let id = id.unwrap();
    view.set_collapsed(&doc, id, true).unwrap();
    let label_tile = doc
        .tile_at(doc.link_info(LinkId(0)).unwrap().label.start)
        .unwrap();
    let ticket = view.heights().ticket(label_tile).unwrap();
    let old_key = doc.tile_key(label_tile).unwrap();
    let mut reader = doc
        .display_cursor(Span::new(0, doc.source().len()))
        .unwrap();
    let mut batch = doc.append(" 't'\n").unwrap();
    view.apply(&doc, &batch).unwrap();
    assert!(view.fold_candidate(&doc, id).unwrap().unwrap().open);
    assert_eq!(doc.source_fold_limit(), 0);
    assert_eq!(doc.link_destination(LinkId(0)), None);
    let update = doc.append(")\n\n").unwrap();
    assert!(update.new_links.is_empty());
    view.apply(&doc, &update).unwrap();
    batch.merge(update).unwrap();
    merged.apply(&doc, &batch).unwrap();
    let rebuilt = FoldView::new(&doc, 1).unwrap();
    assert_eq!(
        merged.heights().total_rows(),
        rebuilt.heights().total_rows()
    );
    let candidate = view.fold_candidate(&doc, id).unwrap().unwrap();
    assert!(!candidate.open);
    assert!(view.is_collapsed(id));
    assert_eq!(candidate.source, doc.blocks()[0].source);
    assert!(doc.source_fold_limit() >= candidate.source.end);
    assert!(SourceFoldRegistration::paragraph(&doc, &merged, BlockId(0), 2).is_ok());
    assert_eq!(doc.link_destination(LinkId(0)), Some("u"));
    assert_eq!(doc.link_title(LinkId(0)), Some("t"));
    assert_ne!(doc.tile_key(label_tile).unwrap(), old_key);
    assert_eq!(view.measure(ticket, 9), Err(LayoutError::StaleMeasurement));
    let mut budget = Budget::new(WorkLimit::unlimited());
    assert_eq!(
        reader.next(&doc, &mut budget),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(budget.used(), WorkLimit::default());
    assert!(matches!(
        view.hidden_at(&doc, 1104, &mut budget).unwrap(),
        FoldLookup::Hidden(_)
    ));
    // Root prose can subsequently be promoted using the original structure
    // pipeline. The direct object keeps its source identity inside Setext.
    let heading = parse("[a](u\n)\n---\n");
    assert!(matches!(
        heading.blocks()[0].kind,
        BlockKind::Heading { .. }
    ));
    assert_eq!(heading.link_destination(LinkId(0)), Some("u"));
    let table = parse("[a](u\n 't')|b\n-|-\n");
    assert_eq!(table.tables().len(), 1);
    assert_eq!(table.link_destination(LinkId(0)), None);
}

#[test]
fn pending_root_suffix_must_preserve_existing_late_table_priority() {
    // Frozen pre-change behavior: the second physical line becomes a table
    // header. The already published shortcut in the prior paragraph survives.
    // Keeping a Destination tail at LF must not bypass that structure decision.
    let source = format!("[a](u\n 't') {}|b\n-|-\n", "x".repeat(1100));
    let doc = parse(&source);
    assert_eq!(
        doc.tables().len(),
        1,
        "pending root suffix disabled the late header probe"
    );
    assert_eq!(doc.link_destination(LinkId(0)), None);
    assert_eq!(doc.link_info(LinkId(0)).unwrap().source, Span::new(0, 3));
}

fn late_table_sources() -> Vec<String> {
    let padding = "x".repeat(1100);
    let mut sources = Vec::new();
    for ending in ["\n", "\r\n"] {
        for image in ["", "!"] {
            // Before target, after bare/angle target, after title; pipe inside
            // target/title, after success, and in pending/rejected suffixes.
            for (prior, header) in [
                ("", format!(" <{padding}|u>)")),
                ("", format!(" {padding}|u)")),
                ("u", format!(" 't') {padding}|b")),
                ("<u>", format!(" '{padding}|title')")),
                ("u 't'", format!(" ) {padding}|b")),
                ("u", format!(" '{padding}|pending")),
                ("u", format!(" '{padding}|t'x)")),
                ("u", format!(" {padding}|failed)")),
                (
                    "u",
                    format!(" '**中🙂** [z](v) ![i](q) &amp; \\* `c` $m$ {padding}|b'"),
                ),
                (
                    "u",
                    format!(" 't') [z](v) ![i](q) &#10; \\| {padding}|&amp; [r]"),
                ),
                ("u", format!(" `opaque {padding}|b`")),
                ("u", format!(" $opaque {padding}|b$")),
            ] {
                sources.push(format!(
                    "{image}[a]({prior}{ending}{header}{ending}-|-{ending}"
                ));
            }
        }
    }
    sources
}

#[test]
fn late_suffix_headers_match_real_baseline_with_arbitrary_cell_syntax() {
    let padding = "x".repeat(1100);
    let mut actual = String::new();
    for source in late_table_sources() {
        let doc = parse(&source);
        assert_eq!(doc.tables().len(), 1, "{source:?}");
        assert_eq!(doc.link_destination(LinkId(0)), None);
        assert_eq!(doc.link_info(LinkId(0)).unwrap().state, SyntaxState::Sealed);
        actual.push_str(&format!("{:?}\n", support::snapshot(&doc)).replace(&padding, "<1100 x>"));
    }
    assert_eq!(
        actual,
        include_str!("fixtures/root_suffix_table_baseline.txt")
    );
}

#[test]
fn deferred_headers_are_split_independent_including_utf8_and_every_eof() {
    for source in late_table_sources() {
        let expected = support::snapshot(&parse(&source));
        let mut stream = Utf8Document::new();
        for byte in source.bytes() {
            stream.append(&[byte]).unwrap();
        }
        stream.finish();
        assert_eq!(support::snapshot(stream.document()), expected);
    }
    // Include all transitions around BLOCK_LOOKAHEAD, UTF-8, the late pipe,
    // completion/rejection and the delimiter. Every prefix is also sealed EOF.
    for index in [0, 2, 8, 9, 27, 41] {
        let source = &late_table_sources()[index];
        let expected = support::snapshot(&parse(source));
        for split in 0..=source.len() {
            let mut stream = Utf8Document::new();
            stream.append(&source.as_bytes()[..split]).unwrap();
            stream.append(&source.as_bytes()[split..]).unwrap();
            stream.finish();
            assert_eq!(
                support::snapshot(stream.document()),
                expected,
                "{index}/{split}"
            );
            let mut prefix = Utf8Document::new();
            for byte in source.as_bytes()[..split].iter().copied() {
                prefix.append(&[byte]).unwrap();
            }
            prefix.finish();
            assert_eq!(
                support::snapshot(prefix.document()),
                support::snapshot(&parse(&String::from_utf8_lossy(
                    &source.as_bytes()[..split]
                ))),
                "EOF {index}/{split}"
            );
        }
    }
}

#[test]
fn deferred_suffix_decision_preserves_published_ids_updates_and_folds() {
    for ending in ["\n", "\r\n"] {
        for image in ["", "!"] {
            for table in [true, false] {
                let mut doc = Document::new();
                let old = format!("[old](v){ending}{ending}");
                doc.append(&format!("{old}{image}[a](u{ending}")).unwrap();
                let block = BlockId(doc.blocks().len() - 1);
                let previous_block = doc.blocks()[0].clone();
                let previous_link = *doc.link_info(LinkId(0)).unwrap();
                let original = *doc.link_info(LinkId(1)).unwrap();
                let LinkTarget::Reference(symbol) = doc.links()[1] else {
                    panic!("shortcut")
                };
                let reference = doc.references()[symbol.0].clone();
                let header = format!(" 't') {} [z](w)|![i](q){ending}", "x".repeat(1100));
                let mut merged = FoldView::new(&doc, 1).unwrap();
                let mut batch = doc.append(&header).unwrap();
                // A complete ')' must not publish an upgrade before deciding
                // whether its physical line belongs to an independent header.
                assert!(batch.new_links.is_empty());
                assert_eq!(doc.link_info(LinkId(1)), Some(&original));
                assert_eq!(doc.link_destination(LinkId(1)), None);
                assert_eq!(doc.references()[symbol.0], reference);
                assert_eq!(doc.source_fold_limit(), old.len());
                let mut view = FoldView::new(&doc, 1).unwrap();
                assert!(SourceFoldRegistration::paragraph(&doc, &view, block, 2).is_err());
                let mut registration =
                    SourceFoldRegistration::open_paragraph(&doc, &view, block, 2).unwrap();
                let mut budget = Budget::new(WorkLimit::unlimited());
                let id = registration
                    .advance(&doc, &mut view, &mut budget)
                    .unwrap()
                    .id
                    .unwrap();
                view.set_collapsed(&doc, id, true).unwrap();
                let tile = doc.tile_at(original.label.start).unwrap();
                let ticket = view.heights().ticket(tile).unwrap();
                let mut reader = doc.display_cursor(original.label).unwrap();
                let decision = if table {
                    format!("-|-{ending}")
                } else {
                    ending.to_owned()
                };
                let update = doc.append(&decision).unwrap();
                assert_eq!(update.new_links, 2..4);
                view.apply(&doc, &update).unwrap();
                batch.merge(update).unwrap();
                let update = doc.finish();
                view.apply(&doc, &update).unwrap();
                batch.merge(update).unwrap();
                merged.apply(&doc, &batch).unwrap();
                let rebuilt = FoldView::new(&doc, 1).unwrap();
                assert_eq!(
                    merged.heights().total_rows(),
                    rebuilt.heights().total_rows()
                );
                assert_eq!(doc.tables().len(), usize::from(table));
                assert_eq!(doc.link_destination(LinkId(1)), (!table).then_some("u"));
                assert_eq!(doc.link_title(LinkId(1)), (!table).then_some("t"));
                assert_eq!(doc.link_info(LinkId(1)).unwrap().label, original.label);
                if table {
                    assert_eq!(doc.link_info(LinkId(1)).unwrap().source, original.source);
                }
                assert_eq!(doc.link_info(LinkId(1)).unwrap().state, SyntaxState::Sealed);
                assert_eq!(doc.link_destination(LinkId(2)), Some("w"));
                assert_eq!(doc.link_destination(LinkId(3)), Some("q"));
                assert_eq!(doc.references()[symbol.0], reference);
                assert_eq!(doc.blocks()[0], previous_block);
                assert_eq!(doc.link_info(LinkId(0)), Some(&previous_link));
                let fold = view.fold_candidate(&doc, id).unwrap().unwrap();
                assert!(!fold.open);
                assert_eq!(fold.source, doc.block(block).unwrap().source);
                assert!(view.is_collapsed(id));
                assert_eq!(view.measure(ticket, 9), Err(LayoutError::StaleMeasurement));
                let mut zero = Budget::new(WorkLimit::default());
                assert_eq!(
                    reader.next(&doc, &mut zero),
                    Err(LayoutError::StaleContinuation)
                );
                assert_eq!(zero.used(), WorkLimit::default());
                assert!(SourceFoldRegistration::paragraph(&doc, &merged, block, 2).is_ok());
                // Raw source and display remain independently sliceable.
                for (start, ch) in doc.source().char_indices() {
                    let span = Span::new(start, start + ch.len_utf8());
                    assert_eq!(
                        doc.runs_in(span)
                            .unwrap()
                            .map(|run| run.text)
                            .collect::<String>(),
                        &doc.source()[span.range()]
                    );
                    assert!(doc.display_runs_in(span).is_some());
                }
            }
        }
    }
}

#[test]
fn deferred_suffix_work_consumes_each_new_source_once_on_its_decision() {
    // Deferred ownership charges observer/summary work as input arrives, then
    // charges real inline work once at the structure decision/release event.
    for table in [true, false] {
        let mut doc = Document::new();
        doc.append(&format!("{}\n\np [a](u\n", "[r] ".repeat(20_000)))
            .unwrap();
        let old_infos: Vec<_> = (0..20_000)
            .map(|i| *doc.link_info(LinkId(i)).unwrap())
            .collect();
        doc.append(&format!(" 't') {}", "x".repeat(1100))).unwrap();
        let before = doc.work();
        for _ in 0..1000 {
            doc.append("x").unwrap();
        }
        let after = doc.work();
        assert_eq!(after.input_bytes - before.input_bytes, 1000);
        assert_eq!(after.table_header_bytes - before.table_header_bytes, 1000);
        assert_eq!(after.inline_bytes, before.inline_bytes);
        assert_eq!(
            after.root_eligibility_bytes - before.root_eligibility_bytes,
            1000
        );
        assert_eq!(after.label_bytes, before.label_bytes);
        assert_eq!(after.tree_visits, before.tree_visits);
        assert_eq!(doc.link_destination(LinkId(20_000)), None);
        doc.append(if table { "|b\n-|-\n\n" } else { "\n\n" })
            .unwrap();
        assert_eq!(
            doc.link_destination(LinkId(20_000)),
            (!table).then_some("u")
        );
        let before_definition = doc.work();
        let update = doc.append("[r]: /late\n").unwrap();
        assert_eq!(update.references_changed.len(), 1);
        assert!(doc.work().inline_bytes - before_definition.inline_bytes < 32);
        assert!(doc.work().tree_visits - before_definition.tree_visits < 1000);
        for (index, info) in old_infos.iter().enumerate() {
            assert_eq!(doc.link_info(LinkId(index)), Some(info));
        }
        doc.finish();
        let work = doc.work();
        assert!(work.inline_bytes <= work.input_bytes);
        assert!(work.table_header_bytes <= work.input_bytes);
        assert!(work.root_eligibility_bytes <= work.input_bytes);
        assert!(work.root_eligibility_steps <= 24 * work.input_bytes);
        eprintln!("deferred table={table} before={before:?} after={after:?} final={work:?}");
    }
    // The suffix can fail long before the line ends. A million-byte deferred
    // header/no-header still requires one summary and one semantic consumption.
    for tail in ["", "|b\n-|-\n"] {
        let mut doc = Document::new();
        doc.append("[a](u\n '").unwrap();
        for _ in 0..1000 {
            doc.append(&"x".repeat(1000)).unwrap();
        }
        let before = doc.work();
        assert_eq!(before.inline_bytes, 6);
        assert_eq!(before.table_header_bytes, before.input_bytes);
        doc.append(tail).unwrap();
        doc.finish();
        let after = doc.work();
        assert_eq!(
            after.inline_bytes,
            after.input_bytes - if tail.is_empty() { 0 } else { 6 }
        );
        assert!(after.table_header_bytes <= after.input_bytes);
        assert!(after.tree_visits - before.tree_visits < 1000);
        assert_eq!(doc.tables().len(), usize::from(!tail.is_empty()));
        assert_eq!(doc.link_destination(LinkId(0)), None);
        eprintln!("million deferred {tail:?}: {after:?}");
    }
}

fn inherited_suffix_table_sources() -> Vec<String> {
    let mut sources = Vec::new();
    for ending in ["\n", "\r\n"] {
        for image in ["", "!"] {
            for marker in ["*", "**", "~~"] {
                for after in [" ", "and "] {
                    sources.push(format!(
                        "{marker}prior {image}[a](u{ending}end{marker}{after}{}|b{ending}-|-{ending}",
                        "x".repeat(1100)
                    ));
                }
            }
        }
    }
    sources
}

#[test]
fn pending_suffix_preserves_inherited_emphasis_table_eligibility_baseline() {
    for (index, (source, expected)) in inherited_suffix_table_sources()
        .iter()
        .zip(include_str!("fixtures/root_suffix_inherited_baseline.txt").lines())
        .enumerate()
    {
        assert_eq!(
            format!("{:?}", support::snapshot(&parse(source)))
                .replace(&"x".repeat(1100), "<1100 x>"),
            expected,
            "inherited eligibility case {index}"
        );
    }
    assert_eq!(inherited_suffix_table_sources().len(), 24);
    assert_eq!(
        include_str!("fixtures/root_suffix_inherited_baseline.txt")
            .lines()
            .count(),
        24
    );
}

#[test]
fn old_root_qualification_survives_multiple_gaps_and_valid_opaque_titles() {
    for ending in ["\n", "\r\n"] {
        for image in ["", "!"] {
            for (prior, row, tables, direct) in [
                ("", " 't') ", 1, false),
                ("*prior ", "end*and ", 0, false),
                ("*prior ", "end* ", 1, false),
                ("*prior ", " 'end*and') ", 0, true),
                ("*prior ", " '`end*and`') ", 1, false),
                ("*prior ", " '[end*and]') ", 1, false),
            ] {
                let source = format!(
                    "{prior}{image}[a](u{ending}{row}{}|b{ending}-|-{ending}",
                    "x".repeat(1100)
                );
                let doc = parse(&source);
                assert_eq!(doc.tables().len(), tables, "{source:?}");
                assert_eq!(doc.link_destination(LinkId(0)), direct.then_some("u"));
                let start = prior.len();
                assert_eq!(
                    doc.link_info(LinkId(0)).unwrap().label,
                    Span::new(start + image.len() + 1, start + image.len() + 2)
                );
                let mut stream = Utf8Document::new();
                for byte in source.bytes() {
                    stream.append(&[byte]).unwrap();
                }
                stream.finish();
                assert_eq!(
                    support::snapshot(stream.document()),
                    support::snapshot(&doc)
                );
            }
            for (after, tables) in [("and ", 0), (" ", 1)] {
                let source = format!(
                    "{image}[a]({ending}*u{ending}end*{after}{}|b{ending}-|-{ending}",
                    "x".repeat(1100)
                );
                let doc = parse(&source);
                assert_eq!(doc.tables().len(), tables, "{source:?}");
                assert_eq!(doc.link_destination(LinkId(0)), None);
                let mut stream = Utf8Document::new();
                for byte in source.bytes() {
                    stream.append(&[byte]).unwrap();
                }
                stream.finish();
                assert_eq!(
                    support::snapshot(stream.document()),
                    support::snapshot(&doc)
                );
            }
        }
    }
}
