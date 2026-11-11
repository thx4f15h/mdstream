use mdstream::*;
#[allow(dead_code)]
#[path = "support/shortcut_hashes.rs"]
mod shortcut_hashes;
#[allow(dead_code)]
#[path = "support/shortcut_text_hashes.rs"]
mod shortcut_text;

#[path = "support/component_gap_model.rs"]
mod component_model;
#[path = "support/snapshot.rs"]
mod support;
include!("fixtures/container_gap_scope_changes.rs");

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn assert_new_container(prior: &str, marker: &str, count: usize) {
    let source = format!("{prior}{marker}{}", "x".repeat(1100));
    let doc = parse(&source);
    assert_eq!(doc.source(), source);
    assert!(doc.tables().is_empty());
    assert_eq!(doc.container_count(), count);
}

macro_rules! scope_regression {
    ($name:ident, $prior:expr, $marker:expr, $count:expr) => {
        #[test]
        fn $name() {
            assert_new_container($prior, $marker, $count);
        }
    };
}

scope_regression!(pending_root_to_quote, "[a](u\n", "> ", 1);
scope_regression!(completed_root_to_quote, "[a](u\n 't')\n", "> ", 1);
scope_regression!(failed_root_to_quote, "[a](u\nbar)\n", "> ", 1);
scope_regression!(pending_root_to_list, "[a](u\n", "- ", 2);
scope_regression!(completed_root_to_list, "[a](u\n 't')\n", "- ", 2);
scope_regression!(failed_root_to_list, "[a](u\nbar)\n", "- ", 2);

struct ScopeCase {
    source: String,
    boundary: usize,
    completed: bool,
    table: bool,
}

fn scope_cases() -> Vec<ScopeCase> {
    let mut cases = Vec::new();
    for ending in ["\n", "\r\n"] {
        for image in ["", "!"] {
            for suffix in ["", " 't')", "bar)"] {
                let prior = if suffix.is_empty() {
                    format!("{image}[a](u{ending}")
                } else {
                    format!("{image}[a](u{ending}{suffix}{ending}")
                };
                for (marker, continuation) in [
                    ("> ", "> "),
                    ("- ", "  "),
                    ("1. ", "   "),
                    ("> - ", ">   "),
                    (">\t", ">\t"),
                    ("- >\t", "  >\t"),
                ] {
                    for width in [
                        BLOCK_LOOKAHEAD - 1,
                        BLOCK_LOOKAHEAD,
                        BLOCK_LOOKAHEAD + 1,
                        1102,
                    ] {
                        let body = "**中🙂** [z](v) ![i](q) &amp; \\* `c` ";
                        let row = format!(
                            "{marker}{body}{}",
                            "x".repeat(width - marker.len() - body.len())
                        );
                        for decision in 0..4 {
                            let tail = match decision {
                                0 => String::new(),
                                1 => ending.to_owned(),
                                2 => format!("|[tail](v){ending}{continuation}-|-{ending}"),
                                3 => format!("|[tail](v){ending}{continuation}wrong{ending}"),
                                _ => unreachable!(),
                            };
                            cases.push(ScopeCase {
                                source: format!("{prior}{row}{tail}"),
                                boundary: prior.len(),
                                completed: suffix == " 't')",
                                table: decision == 2,
                            });
                        }
                    }
                }
            }
        }
    }
    cases
}

fn role_cases() -> Vec<ScopeCase> {
    let mut cases = Vec::new();
    for ending in ["\n", "\r\n"] {
        for image in ["", "!"] {
            for suffix in ["", " 't')", "bar)"] {
                let prior = if suffix.is_empty() {
                    format!("{image}[a](u{ending}")
                } else {
                    format!("{image}[a](u{ending}{suffix}{ending}")
                };
                let long = "x".repeat(1100);
                for tail in [
                    format!("> [b](v{ending}> 't'){ending}lazy {long}{ending}"),
                    format!("- [b](v{ending}  't'){ending}  {long}{ending}"),
                    format!("> - [b](v{ending}>   't'){ending}>   {long}{ending}"),
                    format!(">\t[b](v{ending}>\t't'){ending}>\t{long}{ending}"),
                    format!("# head [b](v 't'){ending}{long}{ending}"),
                    format!("~~~{ending}[b](v{ending} 't'){ending}~~~{ending}{long}{ending}"),
                    format!("    {long}{ending}"),
                    format!("{ending}{long} [b](v 't'){ending}"),
                    format!(
                        "> first{ending}> - [b](v{ending}>   't'){ending}> - [c](q){ending}{ending}{long}"
                    ),
                ] {
                    cases.push(ScopeCase {
                        source: format!("{prior}{tail}"),
                        boundary: prior.len(),
                        completed: suffix == " 't')",
                        table: false,
                    });
                }
            }
        }
    }
    cases
}

// The old library cannot upgrade the newly valid root prefix. For that one
// prefix only, compare the entire remaining snapshot, retaining absolute spans,
// implicit container/block ordering and every public LinkId in render runs.
// Pending/failed prefixes compare the unmodified whole-document snapshot.
fn scope_snapshot(doc: &Document, boundary: usize, completed: bool) -> support::Snapshot {
    let mut snapshot = support::snapshot(doc);
    if completed {
        snapshot
            .blocks
            .retain(|block| block.source.start >= boundary);
        snapshot.links.remove(0);
        snapshot.link_info.remove(0);
        snapshot.runs.retain(|run| run.0.start >= boundary);
        snapshot.display.retain(|run| run.0.start >= boundary);
        snapshot.depths.retain(|run| run.0.start >= boundary);
    }
    snapshot
}

fn digest(text: &str) -> u64 {
    text.bytes().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    })
}

#[test]
fn container_scope_structure_maps_and_ids_match_frozen_single_line_grammar() {
    let cases: Vec<_> = scope_cases().into_iter().chain(role_cases()).collect();
    let expected: Vec<_> = include_str!("fixtures/root_suffix_scope_baseline.txt")
        .lines()
        .collect();
    assert_eq!(cases.len(), 1260);
    assert_eq!(expected.len(), cases.len());
    for (index, (case, expected)) in cases.iter().zip(expected).enumerate() {
        let expected = shortcut_text::line(
            "root_suffix_scope_baseline",
            index,
            case.source.as_bytes(),
            expected,
        );
        let doc = parse(&case.source);
        assert_eq!(doc.tables().len(), usize::from(case.table), "case {index}");
        let actual = format!("{:?}", scope_snapshot(&doc, case.boundary, case.completed));
        let expected = if let Some((_, hash)) = COMPONENT_SCOPE_CHANGES
            .iter()
            .find(|entry| entry.0 == case.source)
        {
            component_model::assert_direct(&doc, "b", 1);
            format!(
                "{:016x}",
                shortcut_hashes::source(&case.source, false, *hash)
            )
        } else {
            expected.to_owned()
        };
        assert_eq!(
            format!("{:016x}", digest(&actual)),
            expected,
            "case {index}: {:?}",
            case.source
        );
        assert_eq!(
            doc.link_destination(LinkId(0)),
            case.completed.then_some("u")
        );
        assert_eq!(doc.link_title(LinkId(0)), case.completed.then_some("t"));
        let info = doc.link_info(LinkId(0)).unwrap();
        let image = case.source.starts_with('!');
        assert_eq!(
            info.kind,
            if image {
                LinkKind::Image
            } else {
                LinkKind::Link
            }
        );
        assert_eq!(
            info.label,
            Span::new(1 + usize::from(image), 2 + usize::from(image))
        );
        assert_eq!(info.source.start, 0);
        assert_eq!(info.state, SyntaxState::Sealed);
        assert_eq!(
            info.source.end,
            if case.completed {
                case.source[..case.boundary]
                    .trim_end_matches(['\r', '\n'])
                    .len()
            } else if case.source.starts_with('!') {
                4
            } else {
                3
            }
        );
    }
}

#[test]
fn scope_transitions_preserve_utf8_chunks_scalars_and_every_prefix_eof() {
    let cases = scope_cases();
    for (index, case) in cases
        .iter()
        .enumerate()
        .filter(|(i, _)| matches!(i % 16, 12 | 14 | 15))
    {
        let expected = support::snapshot(&parse(&case.source));
        let mut bytes = Utf8Document::new();
        for byte in case.source.bytes() {
            bytes.append(&[byte]).unwrap();
        }
        bytes.finish();
        assert_eq!(
            support::snapshot(bytes.document()),
            expected,
            "bytes {index}"
        );
        let mut scalars = Document::new();
        for ch in case.source.chars() {
            scalars.append(ch.encode_utf8(&mut [0; 4])).unwrap();
        }
        scalars.finish();
        assert_eq!(support::snapshot(&scalars), expected, "scalars {index}");
    }
    let mut sources: Vec<_> = [3, 15, 94, 190, 384, 575, 771, 1151]
        .map(|index| cases[index].source.clone())
        .into();
    for marker in ["> ", "- >\t"] {
        // The classifier's 1024th byte is inside a UTF-8 scalar, not ASCII.
        sources.push(format!(
            "![a](u\r\n{marker}{}中🙂 [z](v)|b\r\n{}-|-\r\n",
            "x".repeat(BLOCK_LOOKAHEAD - marker.len() - 2),
            if marker == "> " { "> " } else { "  >\t" }
        ));
    }
    for (index, source) in sources.iter().enumerate() {
        let expected = support::snapshot(&parse(source));
        for split in 0..=source.len() {
            let mut stream = Utf8Document::new();
            stream.append(&source.as_bytes()[..split]).unwrap();
            stream.append(&source.as_bytes()[split..]).unwrap();
            stream.finish();
            assert_eq!(
                support::snapshot(stream.document()),
                expected,
                "split {index}/{split}"
            );
            let mut prefix = Utf8Document::new();
            for bytes in source.as_bytes()[..split].chunks(7) {
                prefix.append(bytes).unwrap();
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
    for case in role_cases() {
        let mut stream = Utf8Document::new();
        for byte in case.source.bytes() {
            stream.append(&[byte]).unwrap();
        }
        stream.finish();
        assert_eq!(
            support::snapshot(stream.document()),
            support::snapshot(&parse(&case.source))
        );
    }
}

#[test]
fn new_root_after_container_or_real_block_gets_its_own_suffix_scope() {
    for ending in ["\n", "\r\n"] {
        for image in ["", "!"] {
            for interruption in [
                ending.to_owned(),
                format!("> quote{ending}{ending}"),
                format!("- item{ending}{ending}"),
                format!("# head{ending}"),
                format!("~~~{ending}raw{ending}~~~{ending}"),
                format!("    code{ending}"),
            ] {
                let prefix = format!("{image}[a](u{ending}{interruption}");
                let source = format!("{prefix}{image}[b](v{ending} 't') {}", "x".repeat(1100));
                let doc = parse(&source);
                assert_eq!(doc.links().len(), 2);
                assert_eq!(doc.link_destination(LinkId(0)), None);
                assert_eq!(doc.link_destination(LinkId(1)), Some("v"));
                assert_eq!(doc.link_title(LinkId(1)), Some("t"));
                assert_eq!(doc.link_info(LinkId(1)).unwrap().source.start, prefix.len());
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

#[test]
fn new_container_work_never_observes_or_replays_the_old_root() {
    for prior in ["[a](u\n", "[a](u\n 't')\n", "[a](u\nbar)\n"] {
        for (marker, continuation) in [("> ", "> "), ("- ", "  ")] {
            for old_count in [0, 20_000] {
                let mut doc = Document::new();
                doc.append(&format!("{}\n\n{prior}", "[r] ".repeat(old_count)))
                    .unwrap();
                doc.append(&format!("{marker}{}", "x".repeat(1100)))
                    .unwrap();
                let old: Vec<_> = (0..=old_count)
                    .map(|i| *doc.link_info(LinkId(i)).unwrap())
                    .collect();
                let before = doc.work();
                for _ in 0..1000 {
                    doc.append("x").unwrap();
                }
                let after = doc.work();
                assert_eq!(after.input_bytes - before.input_bytes, 1000);
                assert_eq!(after.inline_bytes - before.inline_bytes, 1000);
                assert_eq!(after.table_header_bytes - before.table_header_bytes, 1000);
                assert_eq!(after.root_eligibility_bytes, before.root_eligibility_bytes);
                assert_eq!(after.root_eligibility_steps, before.root_eligibility_steps);
                assert_eq!(after.label_bytes, before.label_bytes);
                assert_eq!(after.tree_visits, before.tree_visits);
                doc.append("|[z](v)\n").unwrap();
                assert_eq!(doc.work().inline_bytes, after.inline_bytes);
                // Exercise both confirmation and ordinary release with old IDs.
                let table = old_count == 0;
                doc.append(&format!(
                    "{continuation}{}\n",
                    if table { "-|-" } else { "wrong" }
                ))
                .unwrap();
                doc.finish();
                assert_eq!(doc.tables().len(), usize::from(table));
                assert_eq!(doc.link_destination(LinkId(old_count + 1)), Some("v"));
                for (index, info) in old.iter().enumerate() {
                    assert_eq!(doc.link_info(LinkId(index)), Some(info));
                }
                let work = doc.work();
                assert!(work.inline_bytes <= work.input_bytes);
                assert!(work.table_header_bytes <= work.input_bytes);
                assert!(work.root_eligibility_bytes <= work.input_bytes);
                eprintln!(
                    "scope prior={prior:?} marker={marker:?} history={old_count} before={before:?} after={after:?} final={work:?}"
                );
            }
        }
    }
}

#[test]
fn container_source_and_decoded_readers_still_share_zero_one_six_byte_budgets() {
    for marker in ["> ", "- ", "> - ", ">\t"] {
        for image in ["", "!"] {
            let source = format!(
                "[a](u\r\n{marker}{} {image}[中&amp;](v&amp;)\n",
                "x".repeat(1100)
            );
            let doc = parse(&source);
            let id = LinkId(1);
            let info = doc.link_info(id).unwrap();
            let block = doc
                .runs_in(info.label)
                .unwrap()
                .next()
                .unwrap()
                .block
                .unwrap();
            let scope = doc.blocks()[block.0].container.unwrap();
            assert!(matches!(
                doc.container(scope).unwrap().kind,
                ContainerKind::Quote | ContainerKind::Item { .. }
            ));
            let mut destination = doc.decoded_link_destination(id).unwrap();
            let mut display = doc.display_cursor(info.label).unwrap();
            let mut zero = Budget::new(WorkLimit::default());
            assert_eq!(
                destination.advance(&mut zero),
                Err(Completion::BudgetExhausted)
            );
            assert_eq!(
                display.next(&doc, &mut zero).unwrap(),
                DisplayStep::BudgetExhausted
            );
            assert_eq!(zero.used(), WorkLimit::default());
            let mut strings = [String::new(), String::new()];
            let mut done = [false; 2];
            for frame in 0..1000 {
                let limit = if frame % 2 == 0 { 1 } else { 6 };
                let mut budget = Budget::new(WorkLimit {
                    bytes: limit,
                    queries: 1,
                    objects: 1,
                    ..WorkLimit::default()
                });
                for index in (0..2).map(|i| (i + frame) % 2) {
                    if done[index] {
                        continue;
                    }
                    if index == 0 {
                        match destination.advance(&mut budget) {
                            Ok(Some(chunk)) => {
                                assert!(doc.text(chunk.source).is_some());
                                strings[0].push_str(chunk.text);
                            }
                            Ok(None) => done[0] = true,
                            Err(Completion::BudgetExhausted) => {}
                            other => panic!("{other:?}"),
                        }
                    } else {
                        match display.next(&doc, &mut budget).unwrap() {
                            DisplayStep::Run(run) => {
                                assert!(doc.text(run.source).is_some());
                                if !run.style.contains(Style::HIDDEN) {
                                    strings[1].push_str(run.text);
                                }
                            }
                            DisplayStep::Complete => done[1] = true,
                            DisplayStep::BudgetExhausted => {}
                        }
                    }
                }
                assert!(budget.used().bytes <= limit);
                assert!(budget.used().queries <= 1 && budget.used().objects <= 1);
                if done == [true; 2] {
                    break;
                }
            }
            assert_eq!(done, [true; 2]);
            assert_eq!(strings, ["v&", "中&"]);
            for (at, ch) in source.char_indices() {
                assert_eq!(
                    doc.runs_in(Span::new(at, at + ch.len_utf8()))
                        .unwrap()
                        .map(|r| r.text)
                        .collect::<String>(),
                    ch.to_string()
                );
            }
        }
    }
}
