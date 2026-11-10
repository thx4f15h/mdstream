use mdstream::*;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

#[test]
fn confirmed_same_style_nesting_exposes_repeat_count() {
    for (source, word, strong) in [
        ("_foo _bar_ baz_\n", "bar", false),
        ("**foo **bar** baz**\n", "bar", true),
    ] {
        let doc = parse(source);
        let at = source.find(word).unwrap();
        let run = doc
            .runs_in(Span::new(at, at + word.len()))
            .unwrap()
            .next()
            .unwrap();
        let depth = if strong {
            run.strong_depth
        } else {
            run.emphasis_depth
        };
        assert_eq!(depth, 2, "same-style inner depth lost for {source:?}");
    }
}

#[path = "support/emphasis_model.rs"]
mod model;
#[path = "support/snapshot.rs"]
mod support;

type Depths = Vec<(bool, usize, usize)>;

fn depths(doc: &Document) -> Depths {
    doc.runs_in(Span::new(0, doc.source().len()))
        .unwrap()
        .flat_map(|r| {
            std::iter::repeat_n(
                (
                    r.style.contains(Style::HIDDEN),
                    r.emphasis_depth,
                    r.strong_depth,
                ),
                r.source.len(),
            )
        })
        .collect()
}

fn depth_at(doc: &Document, word: &str) -> (usize, usize) {
    let at = doc.source().find(word).unwrap();
    let r = doc
        .runs_in(Span::new(at, at + word.len()))
        .unwrap()
        .next()
        .unwrap();
    (r.emphasis_depth, r.strong_depth)
}

#[test]
fn independent_batch_range_painting_counts_nested_mixed_and_failed_pairs() {
    let mut cases = 0;
    for outer in ["*", "**", "_", "__"] {
        for inner in ["*", "**", "_", "__", "***", "___"] {
            for left in [" ", "。", "😀", "\u{a0}", "a"] {
                for right in [" ", "。", "😀", "\u{a0}", "b"] {
                    for closing in [inner, outer, "", "***"] {
                        let source =
                            format!("p {outer}中 {left}{inner}x{right}{closing} 尾{outer} z");
                        let doc = parse(&source);
                        assert_eq!(depths(&doc), model::depths(&source), "{source:?}");
                        let flags: Vec<_> = depths(&doc)
                            .into_iter()
                            .map(|(h, e, s)| (h, e > 0, s > 0))
                            .collect();
                        assert_eq!(flags, model::marks(&source));
                        cases += 1;
                    }
                }
            }
        }
    }
    for source in [
        "*(*foo*)*",
        "_(_foo_)_",
        "**foo **bar** baz**",
        "_foo _bar_ baz_",
        "*中 **文 _字_ 尾** 末*",
        "*foo\\**bar*",
        "***literal***",
        "*a _b* c_",
        "**a __b** c__",
    ] {
        assert_eq!(depths(&parse(source)), model::depths(source), "{source:?}");
    }
    assert_eq!(cases, 2400);
}

#[test]
fn every_byte_split_prefix_and_eof_preserves_confirmed_depths() {
    let mut splits = 0;
    let mut prefixes = 0;
    for seed in [String::new(), format!("{}\n", "x".repeat(BLOCK_LOOKAHEAD))] {
        for tail in [
            "_foo _中😀_ baz_\r\n",
            "**foo **中** baz**",
            "*中 _文_ 尾",
            "***中***",
            "> - *中 **文** 尾*\n",
            "> *中\n_文_\n尾*\n",
            "*中\n\n*文*",
            "| *中 *文* 尾* | **字** |\n| - | - |\n",
            "*中 `*文*` $*式*$ [签](u) 尾*",
            "*中 &amp; *文* 尾*",
            "> - *中\n\t*文* 尾*",
        ] {
            let expected = support::snapshot(&parse(&format!("{seed}{tail}")));
            for cut in 0..=tail.len() {
                let mut doc = Utf8Document::new();
                doc.append(seed.as_bytes()).unwrap();
                doc.append(&tail.as_bytes()[..cut]).unwrap();
                doc.append(&tail.as_bytes()[cut..]).unwrap();
                doc.finish();
                assert_eq!(
                    support::snapshot(doc.document()),
                    expected,
                    "{tail:?} split {cut}"
                );
                splits += 1;
            }
            let mut doc = Utf8Document::new();
            doc.append(seed.as_bytes()).unwrap();
            for end in 1..=tail.len() {
                doc.append(&tail.as_bytes()[end - 1..end]).unwrap();
                let mut whole = Utf8Document::new();
                whole.append(seed.as_bytes()).unwrap();
                whole.append(&tail.as_bytes()[..end]).unwrap();
                assert_eq!(
                    support::snapshot(doc.document()),
                    support::snapshot(whole.document())
                );
                let mut eof = Utf8Document::new();
                eof.append(seed.as_bytes()).unwrap();
                for byte in &tail.as_bytes()[..end] {
                    eof.append(&[*byte]).unwrap();
                }
                eof.finish();
                whole.finish();
                assert_eq!(
                    support::snapshot(eof.document()),
                    support::snapshot(whole.document()),
                    "{tail:?} EOF {end}"
                );
                prefixes += 1;
            }
        }
    }
    eprintln!("depths: {splits} byte splits, {prefixes} byte prefixes/EOF");
}

#[test]
fn late_closure_keeps_ids_mapping_replacements_and_shared_budget() {
    for marker in ["*", "**", "_", "__", "***", "****", "___", "____"] {
        let mut doc = Document::new();
        doc.append(&format!(
            "> - {marker}中 {marker}文 &amp; `code` $式$ [签](/目标) {}",
            "x".repeat(2048)
        ))
        .unwrap();
        let block = doc.block_at(0).unwrap();
        let container = doc.blocks()[block.0].container;
        let link = *doc.link_info(LinkId(0)).unwrap();
        let math = doc.maths()[0];
        let key = doc.tile_key(TileId(0)).unwrap();
        assert_eq!(depth_at(&doc, "文"), (0, 0));
        let update = doc.append(&format!("尾{marker} 末{marker}。\n")).unwrap();
        assert!(
            update
                .changes
                .iter()
                .any(|c| c.kind == ChangeKind::Style && c.source.start < link.source.start)
        );
        doc.finish();
        assert_eq!(doc.block_at(0), Some(block));
        assert_eq!(doc.blocks()[block.0].container, container);
        assert_eq!(*doc.link_info(LinkId(0)).unwrap(), link);
        assert_eq!(doc.maths()[0], math);
        assert_ne!(doc.tile_key(TileId(0)).unwrap(), key);
        let expected = (2 * (marker.len() % 2), 2 * (marker.len() / 2));
        for word in ["文", "&amp;", "code", "式", "签"] {
            assert_eq!(depth_at(&doc, word), expected);
        }
        let all = Span::new(0, doc.source().len());
        let full = depths(&doc);
        let mut scratch = QueryScratch::default();
        for (at, ch) in doc.source().char_indices() {
            let span = Span::new(at, at + ch.len_utf8());
            let r = doc
                .display_runs_in_with_scratch(span, &mut scratch)
                .unwrap()
                .next()
                .unwrap();
            assert_eq!(
                (
                    r.style.contains(Style::HIDDEN),
                    r.emphasis_depth,
                    r.strong_depth
                ),
                full[at]
            );
            assert_eq!(r.source, span);
        }
        let mut tiled = Vec::new();
        for id in 0..doc.tile_count() {
            tiled.extend(doc.runs(TileId(id)).unwrap().flat_map(|r| {
                std::iter::repeat_n(
                    (
                        r.style.contains(Style::HIDDEN),
                        r.emphasis_depth,
                        r.strong_depth,
                    ),
                    r.source.len(),
                )
            }));
        }
        assert_eq!(tiled, full);
        let mut cursor = doc.display_cursor(all).unwrap();
        let mut peer = doc.display_cursor(all).unwrap();
        assert_eq!(
            cursor
                .next(&doc, &mut Budget::new(WorkLimit::default()))
                .unwrap(),
            DisplayStep::BudgetExhausted
        );
        let mut shown = String::new();
        while cursor.source_position() < all.end {
            let mut budget = Budget::new(WorkLimit {
                bytes: 6,
                queries: 1,
                objects: 1,
                ..Default::default()
            });
            let DisplayStep::Run(r) = cursor.next(&doc, &mut budget).unwrap() else {
                panic!("no progress")
            };
            assert_eq!(
                (r.emphasis_depth, r.strong_depth),
                (full[r.source.start].1, full[r.source.start].2)
            );
            if !r.style.contains(Style::HIDDEN) {
                shown.push_str(r.text);
            }
            let used = budget.used();
            assert_eq!(
                peer.next(&doc, &mut budget).unwrap(),
                DisplayStep::BudgetExhausted
            );
            let mut target = doc.decoded_link_destination(LinkId(0)).unwrap();
            assert_eq!(
                target.advance(&mut budget),
                Err(Completion::BudgetExhausted)
            );
            assert_eq!(budget.used(), used);
            assert_eq!(peer.source_position(), 0);
        }
        let expected_text: String = doc
            .display_runs_in(all)
            .unwrap()
            .filter(|r| !r.style.contains(Style::HIDDEN))
            .map(|r| r.text)
            .collect();
        assert_eq!(shown, expected_text);
        assert!(shown.contains("文 & code 式 签"));
    }
}

#[test]
fn header_confirmation_and_failure_discard_or_commit_depth_once() {
    for marker in ["*", "**", "_", "__", "***", "****", "___", "____"] {
        for prefix in ["", "> ", "- "] {
            let continuation = if prefix == "- " { "  " } else { prefix };
            let prior = format!("{prefix}{marker}旧 {marker}内\n");
            let body = format!("尾{marker} 末{marker}。 {}|b\n", "中".repeat(400));
            for ending in ["-|-\n", "wrong\n", "", "\n", "---\n"] {
                let source = format!("{prior}{continuation}{body}{continuation}{ending}");
                let doc = parse(&source);
                let success = ending == "-|-\n";
                assert_eq!(doc.tables().len(), usize::from(success));
                if success {
                    let early = parse(&format!(
                        "{prior}{continuation}|{body}{continuation}{ending}"
                    ));
                    let a = depths(&doc);
                    let b = depths(&early);
                    let at = prior.len() + continuation.len();
                    assert_eq!(a[..at], b[..at]);
                    assert_eq!(a[at..at + body.len()], b[at + 1..at + 1 + body.len()]);
                } else {
                    assert_eq!(
                        depth_at(&doc, "内"),
                        (2 * (marker.len() % 2), 2 * (marker.len() / 2)),
                        "{source:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn deep_counts_use_usize_and_logarithmic_updates_without_history_replay() {
    for marker in ["*", "**"] {
        let depth = 65_536;
        let mut doc = Document::new();
        doc.append(&format!("p {}center", format!("{marker}a ").repeat(depth)))
            .unwrap();
        let before = doc.work();
        for _ in 0..1000 {
            doc.append("中😀").unwrap();
        }
        assert_eq!(doc.work().tree_visits, before.tree_visits);
        assert_eq!(doc.work().delimiter_steps, before.delimiter_steps);
        doc.append(&format!("{}!", format!(" b{marker}").repeat(depth)))
            .unwrap();
        doc.finish();
        assert_eq!(
            depth_at(&doc, "center"),
            if marker.len() == 1 {
                (depth, 0)
            } else {
                (0, depth)
            }
        );
        let n = doc.source().len();
        assert!(doc.work().delimiter_steps <= n as u64);
        assert!(doc.work().inline_bytes <= n as u64);
        assert!(doc.work().tree_visits <= 24 * n as u64 * usize::BITS as u64);
        assert!(doc.annotation_arena_stats().nodes <= 4 * n);
        let at = doc.source().find("center").unwrap();
        let mut query = doc.runs_in(Span::new(at, at + 6)).unwrap();
        assert_eq!(query.next().unwrap().text, "center");
        assert!(query.next().is_none());
        assert!(query.annotation_visits() <= 4 * usize::BITS as u64);
    }
}

#[test]
fn deep_containers_million_bytes_and_late_definitions_preserve_counts() {
    let prefix = ">".repeat(970);
    let mut doc = Document::new();
    doc.append(&format!(
        "{}\n\n{prefix}*中 *文 [签][r] {}",
        "[h](u) ".repeat(4096),
        "x".repeat(2048)
    ))
    .unwrap();
    let link = *doc.link_info(LinkId(4096)).unwrap();
    let before = doc.work();
    let chunk = "中😀".repeat(143);
    for _ in 0..1000 {
        doc.append(&chunk).unwrap();
    }
    assert_eq!(doc.work().delimiter_steps, before.delimiter_steps);
    assert_eq!(doc.work().tree_visits, before.tree_visits);
    assert_eq!(
        doc.work().inline_bytes - before.inline_bytes,
        chunk.len() as u64 * 1000
    );
    assert!(doc.work().container_steps - before.container_steps < 2000);
    doc.append("尾* 末*。\n\n").unwrap();
    let count = depth_at(&doc, "签");
    assert_eq!(count, (2, 0));
    let steps = doc.work().delimiter_steps;
    doc.append("[r]: /目标\n").unwrap();
    doc.finish();
    assert_eq!(doc.work().delimiter_steps, steps);
    assert_eq!(depth_at(&doc, "签"), count);
    assert_eq!(*doc.link_info(LinkId(4096)).unwrap(), link);
    assert_eq!(doc.link_destination(LinkId(4096)), Some("/目标"));
    assert!(doc.work().tree_visits <= 24 * doc.source().len() as u64 * usize::BITS as u64);
}

// A depth-only consumer for unambiguous, single-kind paragraphs. It reads no
// source syntax and never reconstructs a delimiter tree or missing marker.
fn project_one_style(source: &str, strong: bool) -> String {
    let doc = parse(source);
    let tag = if strong { "strong" } else { "em" };
    let mut html = String::from("<p>");
    let mut active = 0;
    for r in doc.display_runs_in(Span::new(0, source.len())).unwrap() {
        if r.style.contains(Style::HIDDEN) {
            continue;
        }
        let (count, other) = if strong {
            (r.strong_depth, r.emphasis_depth)
        } else {
            (r.emphasis_depth, r.strong_depth)
        };
        assert_eq!(other, 0);
        for _ in count..active {
            html.push_str(&format!("</{tag}>"));
        }
        for _ in active..count {
            html.push_str(&format!("<{tag}>"));
        }
        active = count;
        let text = if r.source.end == source.len() && r.replacement.is_none() {
            r.text.strip_suffix('\n').unwrap_or(r.text)
        } else {
            r.text
        };
        html.push_str(text);
    }
    assert_eq!(active, 0);
    html.push_str("</p>\n");
    html
}

mod official_single {
    include!("fixtures/commonmark_emphasis_single.rs");
    #[test]
    fn existing_official_369_and_373_expose_two_levels() {
        for number in [369, 373] {
            let &(_, source, html) = EMPHASIS_CASES.iter().find(|c| c.0 == number).unwrap();
            assert_eq!(
                super::project_one_style(source, false),
                html,
                "example {number}"
            );
        }
    }
}

mod official_strong {
    include!("fixtures/commonmark_emphasis_strong.rs");
    #[test]
    fn existing_official_389_exposes_repeated_strong() {
        let &(_, source, html) = EMPHASIS_CASES.iter().find(|c| c.0 == 389).unwrap();
        assert_eq!(super::project_one_style(source, true), html);
    }
}

mod official_nesting {
    include!("fixtures/commonmark_emphasis_nesting.rs");
    #[test]
    fn existing_official_407_exposes_repeated_emphasis() {
        for number in [407, 408, 409] {
            let &(_, source, html) = EMPHASIS_CASES.iter().find(|c| c.0 == number).unwrap();
            assert_eq!(
                super::project_one_style(source, false),
                html,
                "example {number}"
            );
        }
    }
}

mod official_runs {
    include!("fixtures/commonmark_emphasis_runs.rs");
    #[test]
    fn partial_runs_publish_original_official_repeated_strong_depths() {
        for number in [417, 425, 426, 427] {
            let &(_, source, html) = EMPHASIS_CASES.iter().find(|c| c.0 == number).unwrap();
            assert_eq!(
                super::project_one_style(source, true),
                html,
                "example {number}"
            );
        }
    }
}

mod official_final {
    include!("fixtures/commonmark_emphasis_final.rs");
    #[test]
    fn long_runs_and_repeated_single_styles_match_official_html_using_only_depths() {
        for number in [461, 463, 464, 465, 466] {
            let &(_, source, html) = EMPHASIS_CASES.iter().find(|c| c.0 == number).unwrap();
            assert_eq!(
                super::project_one_style(source, number >= 464),
                html,
                "example {number}"
            );
        }
    }
}
