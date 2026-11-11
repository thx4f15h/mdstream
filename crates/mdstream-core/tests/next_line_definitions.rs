use mdstream::*;

#[path = "support/snapshot.rs"]
mod support;

#[path = "support/link_metadata.rs"]
mod link_metadata;

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

fn visible(doc: &Document) -> String {
    doc.display_runs_in(Span::new(0, doc.source().len()))
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect()
}

fn partial_tab_table_target(first: &str, next: &str) {
    let source = format!("{first}[r|s]:\n{next}  -|-\n");
    let model = parse(&format!("{first}seed|:\n{next}  -|-\n"));
    assert!(model.tables().is_empty());
    let mut doc = Document::new();
    doc.append(&source).unwrap();
    assert_eq!(definitions(&doc), 1, "{source:?}");
    assert!(doc.tables().is_empty());
    assert!(doc.links().is_empty());
    assert_eq!(
        doc.text(doc.references()[0].destination.unwrap()),
        Some("-|-")
    );
    doc.append("\n[r|s]\n").unwrap();
    doc.finish();
    assert_eq!(doc.link_destination(LinkId(0)), Some("-|-"));
}

#[test]
fn next_line_partial_tab_quote_target_obeys_real_table_eligibility() {
    partial_tab_table_target(">\t", ">\t");
}

#[test]
fn next_line_partial_tab_list_target_obeys_real_table_eligibility() {
    partial_tab_table_target("- ", "\t");
}

#[test]
fn next_line_table_eligibility_matches_equal_length_paragraph_controls() {
    let mut cases = 0;
    for (first, next) in [
        ("", ""),
        ("> ", "> "),
        ("- ", "  "),
        ("> - ", ">   "),
        ("- > ", "  > "),
        ("- - ", "    "),
        (">\t", ">\t"),
        ("> ", ">\t"),
        ("   > ", "   >\t"),
        ("- ", "\t"),
        ("1. ", "\t"),
        ("- - ", "\t"),
        ("- - - ", "\t\t"),
        ("- > ", "\t>\t"),
        ("> > ", ">\t>\t"),
    ] {
        for indent in [
            "", " ", "  ", "   ", "    ", "     ", "\t", " \t", "\t ", "  \t", "\t  ", "   \t",
            "\t\t",
        ] {
            for label in ["r|s", "中|🙂", "r\\|s", "r"] {
                for (target, destination, title) in [
                    ("-|-", Some("-|-"), None),
                    ("-:|:-", Some("-:|:-"), None),
                    ("-|-|-", Some("-|-|-"), None),
                    ("-\\|-", Some("-\\|-"), None),
                    ("-|x", Some("-|x"), None),
                    ("-|- '中🙂'", Some("-|-"), Some("中🙂")),
                    ("-|- extra", None, None),
                ] {
                    for ending in ["\n", "\r\n", ""] {
                        let tail = format!("{next}{indent}{target}{ending}");
                        // Same source length, pipe count and escaping as the
                        // definition label, without definition/link syntax.
                        let model = parse(&format!("{first}s{label}s:\n{tail}"));
                        let source = format!("{first}[{label}]:\n{tail}");
                        let doc = parse(&source);
                        let confirmed = destination.is_some() && model.tables().is_empty();
                        assert_eq!(definitions(&doc), usize::from(confirmed), "{source:?}");
                        assert_eq!(doc.tables().len(), model.tables().len(), "{source:?}");
                        if confirmed {
                            assert_eq!(doc.references().len(), 1);
                            let reference = &doc.references()[0];
                            assert_eq!(
                                reference.destination.and_then(|s| doc.text(s)),
                                destination
                            );
                            assert_eq!(reference.title.and_then(|s| doc.text(s)), title);
                            assert!(doc.links().is_empty());
                            assert!(visible(&doc).is_empty());
                            assert_eq!(doc.blocks()[0].source, Span::new(0, source.len()));
                        } else {
                            assert_eq!(doc.blocks(), model.blocks(), "{source:?}");
                            assert!(doc.references().iter().all(|r| r.destination.is_none()));
                        }
                        assert_eq!(doc.container_count(), model.container_count());
                        cases += 1;
                    }
                }
            }
        }
    }
    assert_eq!(cases, 16380);
}

#[test]
fn next_line_joint_table_probes_charge_bounded_work_without_growth_rescans() {
    for (first, next) in [("", ""), ("> ", "> "), (">\t", ">\t"), ("- ", "\t")] {
        for padding in 0..6 {
            for target in ["-|-", "-|- '中🙂'", "-|- extra", "-"] {
                let source = format!("{first}[r|s]:\n{next}{}{target}\n", " ".repeat(padding));
                let mut doc = Document::new();
                for c in source.chars() {
                    let before = doc.work();
                    doc.append(c.encode_utf8(&mut [0; 4])).unwrap();
                    if c != '\n' {
                        assert_eq!(doc.work().container_bytes, before.container_bytes);
                        assert_eq!(doc.work().container_steps, before.container_steps);
                        assert_eq!(doc.work().classified_bytes, before.classified_bytes);
                    }
                }
                doc.finish();
                assert_eq!(doc.work(), parse(&source).work());
                let n = source.len() as u64;
                assert!(doc.work().container_bytes < 24 * n);
                assert!(doc.work().container_steps < 12 * n);
                assert!(doc.work().classified_bytes < 16 * n);
                assert!(doc.work().inline_bytes < 2 * n);
            }
        }
    }
}

fn partial_tab_leaf_fallback(tail: &str, kind: BlockKind) {
    let source = format!(">\t[r]:\n>\t {tail}\n");
    let model = parse(&format!(">\tseed\n>\t {tail}\n"));
    assert_eq!(
        std::mem::discriminant(&model.blocks().last().unwrap().kind),
        std::mem::discriminant(&kind)
    );
    let doc = parse(&source);
    assert_eq!(definitions(&doc), 0, "{source:?}");
    assert_eq!(doc.blocks(), model.blocks());
    assert_eq!(doc.container_count(), model.container_count());
    assert!(doc.references().iter().all(|r| r.destination.is_none()));
}

#[test]
fn next_line_partial_tab_atx_keeps_leaf_priority() {
    partial_tab_leaf_fallback("# 'title'", BlockKind::Heading { level: 1 });
}

#[test]
fn next_line_partial_tab_setext_keeps_leaf_priority() {
    partial_tab_leaf_fallback("===", BlockKind::Heading { level: 1 });
}

#[test]
fn next_line_partial_tab_rule_keeps_leaf_priority() {
    partial_tab_leaf_fallback("***", BlockKind::ThematicBreak);
}

#[test]
fn next_line_partial_tab_fence_keeps_leaf_priority() {
    partial_tab_leaf_fallback("~~~ 'title'", BlockKind::Code { info: None });
}

#[test]
fn next_line_leaf_column_thresholds_match_existing_paths() {
    let mut cases = 0;
    for (first, next, residue) in [
        ("", "", 0),
        ("> ", "> ", 0),
        ("- ", "  ", 0),
        ("> - ", ">   ", 0),
        ("- > ", "  > ", 0),
        ("- - ", "    ", 0),
        (">\t", ">\t", 2),
        ("> ", ">\t", 2),
        ("   > ", "   >\t", 3),
        ("- ", "\t", 2),
        ("1. ", "\t", 1),
        ("- - ", "\t", 0),
        ("- - - ", "\t\t", 2),
        ("- > ", "\t>\t", 2),
        ("> > ", ">\t>\t", 2),
    ] {
        for padding in 0..6 {
            for (tail, leaf) in [
                ("# 'title'", true),
                ("###### 'title'", true),
                ("#url", false),
                ("#######", false),
                ("===", true),
                ("---", true),
                ("***", true),
                ("___", true),
                ("~~~ 'title'", true),
                ("```rs", true),
                ("$$", true),
                ("$$ 'title'", false),
                ("[^n]: 'title'", true),
                ("/中🙂 'title'", false),
            ] {
                for ending in ["\n", "\r\n", ""] {
                    let tail = format!("{next}{}{tail}{ending}", " ".repeat(padding));
                    let source = format!("{first}[r]:\n{tail}");
                    let model = parse(&format!("{first}seed\n{tail}"));
                    let fallback = leaf && residue + padding <= 3;
                    let doc = parse(&source);
                    assert_eq!(definitions(&doc), usize::from(!fallback), "{source:?}");
                    if fallback {
                        assert_eq!(doc.blocks(), model.blocks(), "{source:?}");
                        assert_eq!(doc.container_count(), model.container_count());
                        for i in 0..model.container_count() {
                            assert_eq!(
                                doc.container(ContainerId(i)),
                                model.container(ContainerId(i))
                            );
                        }
                        assert!(doc.references().iter().all(|r| r.destination.is_none()));
                    } else {
                        assert_eq!(
                            doc.container_count(),
                            parse(&format!("{first}seed\n")).container_count()
                        );
                        assert!(doc.links().is_empty());
                        assert!(visible(&doc).is_empty());
                    }
                    cases += 1;
                }
            }
        }
    }
    assert_eq!(cases, 3780);
}

#[test]
fn next_line_leaf_confirmation_and_fallback_keep_identity_and_source_spans() {
    for (first, next, residue) in [(">\t", ">\t", 2), ("- > ", "\t>\t", 2), ("- - ", "\t", 0)] {
        for padding in 0..5 {
            for (target, title) in [
                ("#", Some("中🙂")),
                ("===", None),
                ("***", None),
                ("~~~", Some("中🙂")),
                ("$$", None),
            ] {
                for ending in ["\n", "\r\n", ""] {
                    let mut doc = Document::new();
                    doc.append("[old]\n\n").unwrap();
                    let old = *doc.link_info(LinkId(0)).unwrap();
                    let start = doc.source().len();
                    let update = doc.append(&format!("{first}[r]:\n")).unwrap();
                    let id = BlockId(update.new_blocks.start);
                    let candidate = doc.block(id).unwrap().clone();
                    let containers: Vec<_> = (0..doc.container_count())
                        .map(|i| doc.container(ContainerId(i)).unwrap())
                        .collect();
                    let title_source = title.map_or(String::new(), |s| format!(" '{s}'"));
                    let target_start = doc.source().len() + next.len() + padding;
                    doc.append(&format!(
                        "{next}{}{target}{title_source}",
                        " ".repeat(padding)
                    ))
                    .unwrap();
                    assert_eq!(doc.block(id), Some(&candidate));
                    assert_eq!(doc.source_fold_limit(), start);
                    assert_eq!(doc.links().len(), 1);
                    let update = if ending.is_empty() {
                        doc.finish()
                    } else {
                        doc.append(ending).unwrap()
                    };
                    let confirmed = residue + padding >= 4;
                    assert_eq!(definitions(&doc), usize::from(confirmed));
                    assert_eq!(update.references_changed.len(), usize::from(confirmed));
                    assert!(
                        update
                            .dirty
                            .iter()
                            .any(|s| s.start <= start && s.end > start)
                    );
                    assert_eq!(doc.block_at(start), Some(id));
                    assert_eq!(doc.block(id).unwrap().container, candidate.container);
                    assert_eq!(doc.link_info(LinkId(0)), Some(&old));
                    assert_eq!(doc.container_count(), containers.len());
                    for (i, old) in containers.iter().enumerate() {
                        let current = doc.container(ContainerId(i)).unwrap();
                        assert_eq!(
                            (current.parent, current.kind, current.source.start),
                            (old.parent, old.kind, old.source.start)
                        );
                    }
                    if confirmed {
                        assert_eq!(doc.links().len(), 1);
                        let BlockKind::Definition { symbol } = doc.block(id).unwrap().kind else {
                            panic!("lost original definition block");
                        };
                        let reference = &doc.references()[symbol.0];
                        assert_eq!(
                            reference.destination,
                            Some(Span::new(target_start, target_start + target.len()))
                        );
                        assert_eq!(reference.title.and_then(|s| doc.text(s)), title);
                    } else {
                        assert_eq!(doc.links().len(), 2);
                        assert_eq!(doc.text(doc.link_info(LinkId(1)).unwrap().label), Some("r"));
                        assert!(doc.references().iter().all(|r| r.destination.is_none()));
                    }
                    doc.finish();
                    assert_eq!(
                        support::snapshot(&doc),
                        support::snapshot(&parse(doc.source()))
                    );
                }
            }
        }
    }
}

#[test]
fn next_line_leaf_probes_charge_work_once_per_bounded_line() {
    for depth in [0, 32, 128] {
        let prefix = format!(">{}\t", ">".repeat(depth));
        for padding in [0, 1, 2, 64, 512] {
            for tail in ["# '中🙂'", "===", "***", "~~~ '中🙂'", "$$", "/中🙂 't'"] {
                let first = format!("{prefix}[r]:\n");
                let next = format!("{prefix}{}{tail}", " ".repeat(padding));
                let source = format!("{first}{next}\r\n");
                let bulk = parse(&source);
                let mut doc = Document::new();
                doc.append(&first).unwrap();
                let before = doc.work();
                for ch in next.chars() {
                    doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
                    assert_eq!(doc.work().container_bytes, before.container_bytes);
                    assert_eq!(doc.work().container_steps, before.container_steps);
                    assert_eq!(doc.work().classified_bytes, before.classified_bytes);
                }
                doc.append("\r\n").unwrap();
                // Leaf priority must charge its own classification even when
                // the candidate is confirmed under the >=4-column exception.
                assert!(
                    doc.work().classified_bytes - before.classified_bytes
                        >= (next.len() + padding + tail.len()) as u64
                );
                doc.finish();
                assert_eq!(support::snapshot(&doc), support::snapshot(&bulk));
                assert_eq!(doc.work(), bulk.work());
                let n = source.len() as u64;
                assert!(doc.work().container_bytes < 24 * n);
                assert!(doc.work().container_steps < 12 * n);
                assert!(doc.work().classified_bytes < 16 * n);
                assert!(doc.work().inline_bytes < 2 * n);
            }
        }
    }
}

#[test]
fn next_line_new_containers_match_the_existing_paragraph_structure_model() {
    assert_eq!(
        [
            "[r]:\n>/url\n",
            "[r]:\n* 'title'\n",
            "> [r]:\n> >/url\n",
            "- [r]:\n  * 'title'\n"
        ]
        .map(|source| definitions(&parse(source))),
        [0; 4],
        "all four independent-review counterexamples must fall back"
    );
    let mut cases = 0;
    for (first, next) in [
        ("", ""),
        ("> ", "> "),
        ("- ", "  "),
        ("> - ", ">   "),
        ("- > ", "  > "),
        ("- - ", "\t"),
        ("-  - ", "\t "),
        (">\t", ">\t"),
        (">  - ", ">\t"),
        (">  + ", ">\t"),
        ("> 2. ", ">\t"),
    ] {
        let base = parse(&format!("{first}seed\n")).container_count();
        for indent in ["", " ", "   ", "    ", "\t"] {
            for target in [
                ">/url",
                ">\t'title'",
                "- 'title'",
                "+ 'title'",
                "* 'title'",
                "-\t'title'",
                "1. 'title'",
                "1) 'title'",
                "000000001. 'title'",
                "0. 'title'",
                "2. 'title'",
                "999999999. 'title'",
                "1000000000. 'title'",
                "+",
                "*",
                "1.",
                "2.",
                "-/url",
                "+/url",
                "*/url",
                "1./url",
            ] {
                for ending in ["\n", "\r\n", ""] {
                    let source = format!("{first}[r]:\n{next}{indent}{target}{ending}");
                    // Equal first-line lengths make every structural source
                    // range comparable, independently of definition parsing.
                    let model = parse(&format!("{first}seed\n{next}{indent}{target}{ending}"));
                    let interrupts = model.container_count() > base;
                    // Outside a list, the unmatched ')' is a bad destination.
                    let fallback = interrupts || target == "1) 'title'";
                    let doc = parse(&source);
                    assert_eq!(definitions(&doc), usize::from(!fallback), "{source:?}");
                    if fallback {
                        assert_eq!(doc.blocks(), model.blocks(), "{source:?}");
                        assert_eq!(doc.container_count(), model.container_count());
                        for i in 0..model.container_count() {
                            assert_eq!(
                                doc.container(ContainerId(i)),
                                model.container(ContainerId(i))
                            );
                        }
                        assert!(doc.references().iter().all(|r| r.destination.is_none()));
                        assert!(visible(&doc).contains("r:"));
                    } else {
                        assert_eq!(doc.container_count(), base, "{source:?}");
                        assert!(doc.links().is_empty());
                        assert!(visible(&doc).is_empty());
                    }
                    cases += 1;
                }
            }
        }
    }
    assert_eq!(cases, 3465);
    // The model must preserve the existing root ordered-interruption dialect.
    for (target, count) in [("1. 'title'", 2), ("2. 'title'", 0), ("*", 0)] {
        assert_eq!(parse(&format!("seed\n{target}\n")).container_count(), count);
    }
}

#[test]
fn next_line_new_container_fallback_keeps_objects_and_pending_updates() {
    for (first, next) in [
        ("", ""),
        ("> ", "> "),
        ("- ", "  "),
        ("- - ", "\t"),
        (">  - ", ">\t"),
    ] {
        for target in [
            ">/中🙂",
            "- 'title'",
            "+ 'title'",
            "* 'title'",
            "1) 'title'",
        ] {
            let mut doc = Document::new();
            doc.append("[old]\n\n").unwrap();
            let link = *doc.link_info(LinkId(0)).unwrap();
            let start = doc.source().len();
            let update = doc.append(&format!("{first}[r]:\n")).unwrap();
            let id = BlockId(update.new_blocks.start);
            let block = doc.block(id).unwrap().clone();
            let containers: Vec<_> = (0..doc.container_count())
                .map(|i| doc.container(ContainerId(i)).unwrap())
                .collect();
            assert!(update.new_links.is_empty());
            assert_eq!(doc.source_fold_limit(), start);
            doc.append(&format!("{next}{target}")).unwrap();
            assert_eq!(doc.block(id), Some(&block));
            assert_eq!(doc.links().len(), 1);
            assert_eq!(doc.container_count(), containers.len());
            let update = doc.append("\r\n").unwrap();
            assert!(update.references_changed.is_empty());
            assert!(
                update
                    .dirty
                    .iter()
                    .any(|span| span.start <= start && span.end > start)
            );
            assert_eq!(doc.block_at(start), Some(id));
            assert_eq!(doc.block(id).unwrap().source, block.source);
            assert_eq!(doc.block(id).unwrap().container, block.container);
            assert_eq!(doc.link_info(LinkId(0)), Some(&link));
            assert_eq!(doc.links().len(), 2);
            assert_eq!(doc.text(doc.link_info(LinkId(1)).unwrap().label), Some("r"));
            assert!(doc.container_count() > containers.len());
            for (i, old) in containers.iter().enumerate() {
                let current = doc.container(ContainerId(i)).unwrap();
                assert_eq!(
                    (current.parent, current.kind, current.source.start),
                    (old.parent, old.kind, old.source.start)
                );
            }
            doc.finish();
            assert_eq!(definitions(&doc), 0);
            assert!(visible(&doc).contains("r:"));
        }
    }
}

#[test]
fn next_line_container_probes_charge_bounded_work_without_growth_rescans() {
    for depth in [0, 32, 128] {
        let prefix = ">".repeat(depth);
        for spaces in [1, 64, 512] {
            for marker in [">", "*", "+", "-", "1.", "1)", "2.", "1000000000."] {
                let source = format!("{prefix}[r]:\n{prefix}{marker}{}'t'\n", " ".repeat(spaces));
                let bulk = parse(&source);
                let mut doc = Document::new();
                doc.append(&format!("{prefix}[r]:\n")).unwrap();
                let before = doc.work();
                for ch in source[doc.source().len()..source.len() - 1].chars() {
                    doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
                    assert_eq!(doc.work().container_bytes, before.container_bytes);
                    assert_eq!(doc.work().container_steps, before.container_steps);
                    assert_eq!(doc.work().classified_bytes, before.classified_bytes);
                }
                doc.append("\n").unwrap();
                // Even an accepted target pays for the marker/whitespace
                // scan; counters cannot hide the speculative structural work.
                let scanned = doc.work().container_bytes - before.container_bytes;
                if matches!(marker, "*" | "+" | "-" | "1." | "1)" | "2.") {
                    assert!(
                        scanned >= spaces as u64,
                        "{depth} {spaces} {marker}: {scanned}"
                    );
                } else {
                    // Quotes and invalid ten-digit markers stop before the
                    // following whitespace; they do not claim to scan it.
                    assert!(scanned > 0);
                }
                doc.finish();
                assert_eq!(support::snapshot(&doc), support::snapshot(&bulk));
                assert_eq!(doc.work().container_bytes, bulk.work().container_bytes);
                let n = source.len() as u64;
                assert!(
                    doc.work().container_bytes < 24 * n,
                    "{depth} {spaces} {marker}"
                );
                assert!(doc.work().container_steps < 12 * n);
                assert!(doc.work().classified_bytes < 16 * n);
                assert!(doc.work().inline_bytes < 2 * n);
            }
        }
    }
    for first in [">  - ", ">  + ", "> 2. "] {
        for target in ["0. 'title'", "+", "/url"] {
            let first = format!("{first}[r]:\n");
            let mut doc = Document::new();
            doc.append(&first).unwrap();
            let before = doc.work();
            for ch in format!(">\t{target}").chars() {
                doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
                assert_eq!(doc.work().container_bytes, before.container_bytes);
                assert_eq!(doc.work().container_steps, before.container_steps);
            }
            doc.append("\n").unwrap();
            doc.finish();
            assert_eq!(
                support::snapshot(&doc),
                support::snapshot(&parse(doc.source()))
            );
            assert!(doc.work().container_bytes < 24 * doc.source().len() as u64);
            assert!(doc.work().container_steps < 12 * doc.source().len() as u64);
        }
    }
}

#[test]
fn next_line_target_matches_scope_indent_ending_and_first_definition_model() {
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
    ] {
        for label in ["r", "Straße 中🙂", "r\\]", "r\\[", "r|s"] {
            for indent in [0, 3, 6] {
                for (target, destination, title) in [
                    ("/url", "/url", None),
                    ("<my url> 't&amp;'", "my url", Some("t&amp;")),
                    ("<> \"\"", "", Some("")),
                    ("/a(b) 'title'", "/a(b)", Some("title")),
                ] {
                    for ending in ["\n", "\r\n", ""] {
                        let source = format!(
                            "{first}[{label}]: \t\r\n{next}{}{target}{ending}",
                            " ".repeat(indent)
                        );
                        let doc = parse(&source);
                        assert_eq!(definitions(&doc), 1, "{source:?}");
                        assert!(doc.links().is_empty(), "candidate leaked a shortcut");
                        assert_eq!(doc.references().len(), 1);
                        let reference = &doc.references()[0];
                        assert_eq!(doc.text(reference.destination.unwrap()), Some(destination));
                        assert_eq!(reference.title.and_then(|s| doc.text(s)), title);
                        assert_eq!(doc.blocks()[0].source, Span::new(0, source.len()));
                        assert_eq!(doc.blocks().len(), 1);
                        assert_eq!(doc.blocks()[0].container.is_some(), !first.is_empty());
                        assert!(visible(&doc).is_empty());
                        cases += 1;
                    }
                }
            }
        }
    }
    assert_eq!(cases, 1440);
    let doc = parse("[STRASSE 中🙂]\n\n[Straße 中🙂]:\n/first 't'\n[strasse 中🙂]:\n/second\n");
    assert_eq!(doc.link_destination(LinkId(0)), Some("/first"));
    assert_eq!(doc.link_title(LinkId(0)), Some("t"));
    assert_eq!(doc.references()[0].revision, 1);
    assert_eq!(doc.links().len(), 1);
}

#[test]
fn next_line_candidate_never_publishes_a_revocable_link_and_keeps_identity() {
    for (first, next, label, target) in [
        ("", "", "r", "/中🙂&amp;"),
        ("> ", "> ", "r", "/中🙂&amp;"),
        ("- - ", "\t", "r", "/中🙂&amp;"),
        (">\t", ">\t  ", "r|s", "-|-"),
        ("- ", "\t  ", "r|s", "-|-"),
    ] {
        for success in [true, false] {
            let mut doc = Document::new();
            doc.append(&format!("[{label}]\n\n")).unwrap();
            let info = *doc.link_info(LinkId(0)).unwrap();
            let update = doc.append(&format!("{first}[{label}]:\n")).unwrap();
            let id = BlockId(update.new_blocks.start);
            let block = doc.block(id).unwrap().clone();
            assert_eq!(doc.links().len(), 1);
            assert!(update.new_links.is_empty());
            assert_eq!(doc.source_fold_limit(), block.source.start);
            let target_start = doc.source().len() + next.len();
            doc.append(&format!("{next}{target} 't'")).unwrap();
            assert_eq!(doc.links().len(), 1);
            assert_eq!(doc.block(id), Some(&block));
            let update = doc.append(if success { "\r\n" } else { " bad\n" }).unwrap();
            assert_eq!(doc.block_at(block.source.start), Some(id));
            assert_eq!(doc.block(id).unwrap().container, block.container);
            assert_eq!(doc.link_info(LinkId(0)), Some(&info));
            assert!(
                update
                    .changes
                    .iter()
                    .any(|c| c.source.start <= block.source.start)
            );
            if success {
                assert_eq!(doc.links().len(), 1);
                assert_eq!(doc.link_destination(LinkId(0)), Some(target));
                assert_eq!(
                    doc.references()[0].destination,
                    Some(Span::new(target_start, target_start + target.len()))
                );
                assert_eq!(doc.link_title(LinkId(0)), Some("t"));
                assert_eq!(update.references_changed.len(), 1);
                assert!(matches!(
                    doc.block(id).unwrap().kind,
                    BlockKind::Definition { .. }
                ));
            } else {
                assert_eq!(doc.links().len(), 2);
                assert!(update.references_changed.is_empty());
                assert_eq!(doc.link_destination(LinkId(0)), None);
                assert!(visible(&doc).contains(&format!("{label}:")));
            }
            doc.finish();
        }
    }
}

#[test]
fn next_line_failures_preserve_lazy_paths_and_existing_block_priorities() {
    for source in [
        "[r]:\n\n/url\n",
        "[r]:\n<unclosed\n",
        "[r]:\n/url 'unclosed\n",
        "[r]:\n/url 't' junk\n",
        "[r]:\n/url)\n",
        "seed\n[r]:\n/url\n",
        "> [r]:\n/url\n",
        "> - [r]:\n> /url\n",
        "- [r]:\n- /url\n",
        "-  - [r]:\n\t/url\n",
        "[ ]:\n/url\n",
        "[r]:\n",
        "[r]:",
        "[r]:\n# heading\n",
        "[r]:\n~~~\n/url\n~~~\n",
        "    [r]:\n/url\n",
        "[r|x]:\n-|-\n",
        "> [r|x]:\n> -|-\n",
        "[r]:\n===\n",
        "> [r]:\n> ---\n",
    ] {
        let doc = parse(source);
        assert_eq!(definitions(&doc), 0, "{source:?}");
        assert!(doc.references().iter().all(|r| r.destination.is_none()));
    }
    assert_eq!(parse("[r|x]:\n-|-\n").tables().len(), 1);
    assert_eq!(parse("> [r|x]:\n> -|-\n").tables().len(), 1);
    assert_eq!(parse("[r]:\n===\n").headings().entries().len(), 1);
    assert_eq!(parse("> [r]:\n> ---\n").headings().entries().len(), 1);
    for source in ["> [r]:\n/url\n", "> - [r]:\n> /url\n"] {
        let doc = parse(source);
        assert_eq!(doc.blocks()[0].container, doc.blocks()[1].container);
        assert!(visible(&doc).contains("r:\n/url"));
    }
    // A complete independent title now extends this already valid target.
    let doc = parse("[r]:\n/url\n'outside'\n\n[r]\n");
    assert_eq!(definitions(&doc), 1);
    assert_eq!(doc.link_title(LinkId(0)), Some("outside"));
    assert!(!visible(&doc).contains("'outside'"));
}

#[test]
fn decorated_failed_candidate_consumes_inline_once_with_original_styles() {
    for prefix in ["", "> "] {
        for pipe in ["", "|s"] {
            let first = format!("{prefix}[*a* &amp; `x` r\\]{pipe}]:\n");
            let tail = format!("{prefix}<bad\n");
            let mut doc = Document::new();
            doc.append(&first).unwrap();
            assert!(doc.links().is_empty());
            assert_eq!(doc.work().inline_bytes, 0);
            let block = doc.blocks()[0].clone();
            doc.append(&tail).unwrap();
            doc.finish();
            assert_eq!(definitions(&doc), 0);
            assert_eq!(doc.links().len(), 1);
            assert_eq!(doc.blocks()[0].source.start, block.source.start);
            assert_eq!(doc.blocks()[0].container, block.container);
            assert_eq!(visible(&doc), format!("a & x r]{pipe}:\n<bad\n"));
            for (needle, style) in [("a*", Style::EMPHASIS), ("x`", Style::CODE)] {
                let start = first.find(needle).unwrap();
                let run = doc
                    .display_runs_in(Span::new(start, start + 1))
                    .unwrap()
                    .next()
                    .unwrap();
                assert!(run.style.contains(style));
            }
            assert!(doc.work().inline_bytes <= doc.source().len() as u64);
        }
    }
}

#[test]
fn next_line_confirmation_and_fallback_invalidate_layout_and_exclude_fixed_folds() {
    for (prefix, next) in [
        ("", ""),
        ("> ", "> "),
        ("- - ", "\t"),
        (">\t", ">\t"),
        ("- ", "\t"),
        ("- > ", "\t>\t"),
    ] {
        for label in ["r", "r|s"] {
            for tail in [
                "/url\n",
                "<bad\n",
                " # 'title'\n",
                " ===\n",
                " ***\n",
                " ~~~ 'title'\n",
                " $$\n",
                "===\n",
                ">/url\n",
                "* 'title'\n",
                "1. 'title'\n",
                "2. 'title'\n",
                "  -|-\n",
                " -|-\n",
                "  -|- extra\n",
            ] {
                let mut doc = Document::new();
                doc.append(&format!("{prefix}[{label}]:\n")).unwrap();
                let block = doc.blocks()[0].clone();
                assert_eq!(doc.source_fold_limit(), 0);
                let mut view: FoldView = FoldView::new(&doc, 1).unwrap();
                assert!(SourceFoldRegistration::new(&doc, &view, block.source).is_err());
                assert!(SourceFoldRegistration::paragraph(&doc, &view, BlockId(0), 0).is_err());
                let mut registration = if prefix.is_empty() {
                    SourceFoldRegistration::open_paragraph(&doc, &view, BlockId(0), 0)
                } else {
                    SourceFoldRegistration::container_paragraph(&doc, &view, BlockId(0), 0)
                }
                .unwrap();
                let fold = registration
                    .advance(&doc, &mut view, &mut Budget::new(WorkLimit::unlimited()))
                    .unwrap()
                    .id
                    .unwrap();
                view.set_collapsed(&doc, fold, true).unwrap();
                let ticket = view.heights().ticket(TileId(0)).unwrap();
                view.measure(ticket, 4).unwrap();
                let update = doc.append(&format!("{next}{tail}")).unwrap();
                assert!(update.dirty.iter().any(|s| s.start == 0));
                view.apply(&doc, &update).unwrap();
                assert!(view.measure(ticket, 4).is_err());
                assert_eq!(doc.block_at(0), Some(BlockId(0)));
                assert_eq!(doc.blocks()[0].container, block.container);
                let confirmed = matches!(tail, "/url\n" | "2. 'title'\n")
                    || (tail.trim() == "-|-"
                        && (label == "r"
                            || parse(&format!("{prefix}seed|:\n{next}{tail}"))
                                .tables()
                                .is_empty()));
                if confirmed {
                    assert_eq!(definitions(&doc), 1);
                    assert!(doc.links().is_empty());
                    assert_eq!(
                        doc.source_fold_limit(),
                        if tail == "2. 'title'\n" {
                            doc.source().len()
                        } else {
                            block.source.start
                        }
                    );
                    assert!(view.fold_candidate(&doc, fold).unwrap().is_none());
                } else if !doc.tables().is_empty() {
                    assert_eq!(definitions(&doc), 0);
                    assert!(view.fold_candidate(&doc, fold).unwrap().is_none());
                } else if tail.trim() != "===" {
                    assert!(view.fold_candidate(&doc, fold).unwrap().is_some());
                    assert!(view.is_collapsed(fold));
                }
            }
        }
    }
}

#[test]
fn next_line_utf8_all_splits_prefixes_and_eof_match_bulk() {
    for source in [
        ">\t[r|s]:\n>\t  -|-\n\n[r|s]\n",
        "- [r|s]:\r\n\t  -|-\r\n\r\n[r|s]\n",
        ">\t[中|🙂]:\n>\t  -|-",
        "- [中|🙂]:\r\n\t  -|-",
        "> [r|s]:\n>     -|-\n",
        ">\t[r|s]:\n>\t -|-\n",
        "- [r|s]:\n\t -|-\n",
        "- > [中|🙂]:\n\t>\t  -|- '中🙂'\n",
        "> [r|s]:\n>   \t-|-\n",
        ">\t[r\\|s]:\n>\t -|-\n",
        ">\t[r|s]:\n>\t  -|- extra\n",
        ">\t[r|s]:\n>\t  -|x\n",
        ">\t[r|s]:\n> -|-\n",
        "> - [r|s]:\n> -|-\n",
        ">\t[r|s]:\n-|-\n",
        ">\t[r|s]:\n>\t> -|-\n",
        ">\t[r|s]:\n>\t  -\\|-\r\n",
        "[Straße 中🙂]:\r\n <my url> 't中🙂'\r\n\r\n[STRASSE 中🙂]\n",
        ">\t[r\\]]:\n>\t/url 't'\n\n[r\\]]\n",
        "- - [r]:\n\t/url\n",
        "> - [r]:\n> /url\n",
        "[r]:\n/url 'bad' junk\n",
        "[r]:\n===\n",
        "[r|x]:\n-|-\n",
        "[r]:\n\n/url\n",
        "[r]:\n/中🙂",
        "[a|b]:\n/url\n\n[a|b]\n",
        "> [a|b]:\n> /url\n\n[a|b]\n",
        "> [a|b]:\n> <bad\n",
        "[*a* &amp; `x` r\\]]:\n<bad\n",
        "> [*a* &amp; `x` r\\]]:\n> <bad\n",
        "[r]:\n[s]:\n/url\n\n[s]\n",
        "[r]:\n[s]: /url\n\n[s]\n",
        "[r]:\n>/url\n",
        "[r]:\r\n* 'title'\r\n",
        "> [r]:\r\n> >/中🙂\r\n",
        "- [r]:\n  * 'title'\n",
        "- - [r]:\n\t+ '中🙂'\n",
        ">\t[r]:\n>\t >中🙂\n",
        "> - [r]:\n>   000000001) 't'\n",
        "[r]:\n1. 'title'\n",
        "[r]:\n2. 'title'\n",
        "[r]:\n+\n",
        "[r]:\n1./url\n",
        "[r]:\n    >/url\n",
        ">\t[r]:\n>\t  +/url\n",
        "[r|s]:\n>/url\n",
        "> [r|s]:\n> * 'title'\n",
        ">  - [r]:\r\n>\t0. 'title'\r\n",
        ">  - [r]:\n>\t+\n",
        ">  + [r]:\n>\t+\n",
        "> 2. [r]:\n>\t2.\n",
        ">  - [r]:\n>\t/url\n",
        ">\t[r]:\n>\t # '中🙂'\n",
        ">\t[r]:\r\n>\t ===\r\n",
        ">\t[r]:\n>\t ***\n",
        ">\t[r]:\n>\t ~~~ '中🙂'",
        ">\t[r]:\n>\t $$\r\n",
        ">\t[r]:\n>\t  # '中🙂'\n",
        ">\t[r]:\n>\t  ===\n",
        "- > [r]:\n\t>\t ~~~ '中🙂'\r\n",
        "- - [r]:\n\t $$",
    ] {
        let expected = support::snapshot(&parse(source));
        for at in 0..=source.len() {
            let mut stream = Utf8Document::new();
            stream.append(&source.as_bytes()[..at]).unwrap();
            stream.append(&source.as_bytes()[at..]).unwrap();
            stream.finish();
            assert_eq!(
                support::snapshot(stream.document()),
                expected,
                "{source:?}, {at}"
            );
        }
        let mut stream = Utf8Document::new();
        for byte in source.as_bytes() {
            stream.append(std::slice::from_ref(byte)).unwrap();
        }
        stream.finish();
        assert_eq!(support::snapshot(stream.document()), expected);
        let mut doc = Document::new();
        for (at, ch) in source.char_indices() {
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            let prefix = &source[..at + ch.len_utf8()];
            let mut bulk = Document::new();
            bulk.append(prefix).unwrap();
            assert_eq!(support::snapshot(&doc), support::snapshot(&bulk));
            assert_eq!(doc.pending(), bulk.pending());
            assert_eq!(doc.source_fold_limit(), bulk.source_fold_limit());
            let mut eof = Document::new();
            for c in prefix.chars() {
                eof.append(c.encode_utf8(&mut [0; 4])).unwrap();
            }
            eof.finish();
            assert_eq!(support::snapshot(&eof), support::snapshot(&parse(prefix)));
        }
    }
}

#[test]
fn next_line_mapping_and_shared_display_metadata_budgets_remain_exact() {
    for source in [
        "[r|s]\n\n>\t[r|s]:\n>\t  -|- '中🙂&amp;'\n",
        "[r|s]\n\n- [r|s]:\r\n\t  -|-\r\n",
        "- > [中|🙂]:\n\t>\t  -|- '中🙂'\n",
        "> [r|s]:\n>     -|-\n",
        ">\t[r|s]:\n>\t -|-\n",
        ">\t[r|s]:\n>\t  -|- extra\n",
        ">\t[r|s]:\n>\t  -|x\n",
        ">\t[r\\|s]:\n>\t -|-\n",
        "[r]\n\n- - [r]:\r\n\t<中🙂&amp;> 't&amp;'\r\n",
        ">\t[r]:\n>\t<bad\n",
        "[r]:\n/中🙂 't' bad\n",
        "[r]:\n>/中🙂\n",
        "[r]:\r\n* '中🙂'\r\n",
        "> [r]:\n> >/中🙂\n",
        "- [r]:\n  * '中🙂'\n",
        "- - [r]:\n\t+ '中🙂'\n",
        ">\t[r]:\n>\t >中🙂\n",
        ">  - [r]:\r\n>\t0. '中🙂'\r\n",
    ] {
        let doc = parse(source);
        let full = Span::new(0, source.len());
        assert_eq!(
            doc.runs_in(full)
                .unwrap()
                .map(|r| r.text)
                .collect::<String>(),
            source
        );
        let sliced: String = source
            .char_indices()
            .flat_map(|(at, c)| {
                doc.display_runs_in(Span::new(at, at + c.len_utf8()))
                    .unwrap()
            })
            .filter(|r| !r.style.contains(Style::HIDDEN))
            .map(|r| r.text)
            .collect();
        assert_eq!(sliced, visible(&doc));
        link_metadata::snapshot(&doc);
        let mut cursor = doc.display_cursor(full).unwrap();
        let mut output = String::new();
        while cursor.source_position() < full.end {
            let at = cursor.source_position();
            assert_eq!(
                cursor
                    .next(&doc, &mut Budget::new(WorkLimit::default()))
                    .unwrap(),
                DisplayStep::BudgetExhausted
            );
            assert_eq!(cursor.source_position(), at);
            let mut budget = Budget::new(WorkLimit {
                bytes: 6,
                queries: 1,
                objects: 1,
                ..Default::default()
            });
            let DisplayStep::Run(run) = cursor.next(&doc, &mut budget).unwrap() else {
                panic!("scalar fits")
            };
            assert_eq!(run.source.start, at);
            assert!(budget.used().bytes <= 6);
            if !run.style.contains(Style::HIDDEN) {
                output.push_str(run.text);
            }
            if cursor.source_position() < full.end {
                assert_eq!(
                    cursor.next(&doc, &mut budget).unwrap(),
                    DisplayStep::BudgetExhausted
                );
            }
        }
        assert_eq!(output, visible(&doc));
    }
}

#[test]
fn next_line_space_destination_and_title_decode_with_one_shared_tiny_budget() {
    let source = "[r]\n\n[r]:\r\n<my url> 't&amp;中🙂'\r\n";
    let doc = parse(source);
    let id = LinkId(0);
    assert_eq!(doc.link_destination(id), Some("my url"));
    assert_eq!(doc.link_title(id), Some("t&amp;中🙂"));
    let reference = &doc.references()[0];
    let spans = [reference.destination.unwrap(), reference.title.unwrap()];
    assert_eq!(doc.text(spans[0]), Some("my url"));
    assert_eq!(doc.text(spans[1]), Some("t&amp;中🙂"));
    let mut destination = doc.decoded_link_destination(id).unwrap();
    let mut title = doc.decoded_link_title(id).unwrap();
    let mut budget = Budget::new(WorkLimit::default());
    assert_eq!(
        destination.advance(&mut budget),
        Err(Completion::BudgetExhausted)
    );
    assert_eq!(title.advance(&mut budget), Err(Completion::BudgetExhausted));
    assert_eq!(destination.source_position(), spans[0].start);
    assert_eq!(title.source_position(), spans[1].start);
    let limit = WorkLimit {
        bytes: 6,
        queries: 1,
        objects: 1,
        ..Default::default()
    };
    let mut budget = Budget::new(limit);
    let mut output = [String::new(), String::new()];
    let mut steps = 0;
    for (phase, span) in spans.into_iter().enumerate() {
        let mut end = span.start;
        loop {
            steps += 1;
            assert!(steps < 4 * source.len());
            let step = if phase == 0 {
                destination
                    .advance(&mut budget)
                    .map(|chunk| chunk.map(|c| (c.source, c.text)))
            } else {
                title
                    .advance(&mut budget)
                    .map(|chunk| chunk.map(|c| (c.source, c.text)))
            };
            match step {
                Ok(Some((raw, text))) => {
                    assert_eq!(raw.start, end);
                    assert!(doc.text(raw).is_some());
                    end = raw.end;
                    output[phase].push_str(text);
                }
                Ok(None) => break,
                Err(Completion::BudgetExhausted) => {
                    assert_ne!(budget.used(), WorkLimit::default());
                    budget = Budget::new(limit);
                }
                Err(other) => panic!("unexpected {other:?}"),
            }
            assert!(budget.used().bytes <= limit.bytes);
            assert!(budget.used().queries <= limit.queries);
            assert!(budget.used().objects <= limit.objects);
            assert_eq!(
                if phase == 0 {
                    destination.source_position()
                } else {
                    title.source_position()
                },
                end
            );
        }
        assert_eq!(end, span.end);
    }
    assert_eq!(output, ["my url", "t&中🙂"]);
}

#[test]
fn space_url_output_encoding_is_identical_for_single_and_next_line_definitions() {
    for separator in [" ", "\n", "\r\n"] {
        for target in ["my url", "my&#32;url"] {
            let doc = parse(&format!("[r]\n\n[r]:{separator}<{target}> 't&amp;'\n"));
            assert_eq!(doc.link_destination(LinkId(0)), Some(target));
            assert_eq!(
                link_metadata::snapshot(&doc),
                [Some("<a href=\"my%20url\" title=\"t&amp;\">".to_owned())]
            );
        }
    }
}

#[test]
fn next_line_failed_growth_and_late_resolution_have_bounded_cumulative_work() {
    for label in ["r", "r|s"] {
        let quotes = ">".repeat(970);
        let mut doc = Document::new();
        doc.append(&format!(
            "{quotes} [{label}]:\n{quotes} /{}",
            "x".repeat(1_000_000)
        ))
        .unwrap();
        let before = doc.work();
        for _ in 0..1000 {
            doc.append("x").unwrap();
        }
        assert_eq!(doc.work().classified_bytes, before.classified_bytes);
        assert_eq!(doc.work().container_steps, before.container_steps);
        assert_eq!(doc.work().inline_bytes - before.inline_bytes, 1000);
        doc.finish();
        assert_eq!(definitions(&doc), 0);
        let n = doc.source().len() as u64;
        assert!(doc.work().classified_bytes < 16 * n);
        assert!(doc.work().inline_bytes < 2 * n);
        let mut doc = Document::new();
        for _ in 0..20_000 {
            doc.append(&format!("[{label}]\n\n")).unwrap();
        }
        let infos: Vec<_> = (0..doc.links().len())
            .map(|i| *doc.link_info(LinkId(i)).unwrap())
            .collect();
        let before = doc.work();
        let update = doc.append(&format!("[{label}]:\n/url 't'\n")).unwrap();
        assert_eq!(update.references_changed.len(), 1);
        assert_eq!(doc.links().len(), 20_000);
        assert!(doc.work().classified_bytes - before.classified_bytes < 128);
        assert!(doc.work().inline_bytes - before.inline_bytes < 32);
        assert!(doc.work().label_bytes - before.label_bytes < 32);
        assert!(doc.work().table_header_bytes - before.table_header_bytes < 32);
        for (i, info) in infos.iter().enumerate() {
            assert_eq!(doc.link_info(LinkId(i)), Some(info));
            assert_eq!(doc.link_destination(LinkId(i)), Some("/url"));
        }
        for i in 0..4096 {
            doc.append(&format!("[{label}{i}]:\n/url\n")).unwrap();
        }
        assert_eq!(doc.links().len(), 20_000);
        assert_eq!(definitions(&doc), 4097);
        let n = doc.source().len() as u64;
        assert!(doc.work().classified_bytes < 16 * n);
        assert!(doc.work().inline_bytes < 2 * n);
    }
}

#[test]
fn next_line_window_boundaries_and_long_table_probe_keep_single_consumption() {
    for first in [true, false] {
        for length in [BLOCK_LOOKAHEAD - 1, BLOCK_LOOKAHEAD, BLOCK_LOOKAHEAD + 1] {
            for ending in ["\n", "\r\n", ""] {
                if first && ending.is_empty() {
                    continue;
                }
                let source = if first {
                    format!(
                        "[r]:{}{ending}/url\n",
                        " ".repeat(length - 4 - ending.len())
                    )
                } else {
                    format!("[r]:\n/{}{ending}", "x".repeat(length - 1 - ending.len()))
                };
                let doc = parse(&source);
                assert_eq!(
                    definitions(&doc),
                    usize::from(
                        length <= BLOCK_LOOKAHEAD
                            && (!ending.is_empty() || length < BLOCK_LOOKAHEAD)
                    ),
                    "{first} {length} {ending:?}"
                );
                assert!(doc.work().classified_bytes < 16 * source.len() as u64);
            }
        }
    }
    for source in [
        format!("[{}]:\n/url\n", "a".repeat(MAX_LABEL_BYTES + 1)),
        "[r]:\n/ur\nl\n".to_owned(),
    ] {
        let doc = parse(&source);
        if source.starts_with("[r]") {
            assert_eq!(
                doc.text(doc.references()[0].destination.unwrap()),
                Some("/ur")
            );
            assert!(visible(&doc).contains('l'));
        } else {
            assert_eq!(definitions(&doc), 0);
        }
    }
    let mut doc = Document::new();
    doc.append(&format!("[r|s]:\n{}|-", "-".repeat(4096)))
        .unwrap();
    doc.append("\n\n[t]:\n/url\n").unwrap();
    assert_eq!(doc.tables().len(), 1);
    assert_eq!(definitions(&doc), 1);
    assert_eq!(doc.references().len(), 1);
    assert!(doc.links().is_empty());
    let doc = parse("a|b\n-|-\n[r]:\n/url\n");
    assert_eq!(doc.tables().len(), 1);
    assert_eq!(definitions(&doc), 1);
}
