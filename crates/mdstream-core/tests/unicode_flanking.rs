use mdstream::*;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn visible(doc: &Document) -> String {
    doc.display_runs_in(Span::new(0, doc.source().len()))
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect()
}

#[test]
fn unicode_whitespace_punctuation_and_symbols_are_scalar_boundaries() {
    for (source, expected) in [
        ("*\u{a0}a\u{a0}*", "*\u{a0}a\u{a0}*"),
        ("a*£*b", "a*£*b"),
        ("a**😀**b", "a**😀**b"),
        ("a*「x」*b", "a*「x」*b"),
        ("「_中_」", "「中」"),
        ("😀__中__😀", "😀中😀"),
        ("_中_文", "_中_文"),
        ("*\u{2028}x\u{2028}*", "\u{2028}x\u{2028}"),
    ] {
        assert_eq!(visible(&parse(source)), expected, "{source:?}");
    }
}

#[path = "support/emphasis_model.rs"]
mod model;
#[path = "support/snapshot.rs"]
mod support;

fn flags(doc: &Document) -> Vec<(bool, bool, bool)> {
    doc.runs_in(Span::new(0, doc.source().len()))
        .unwrap()
        .flat_map(|r| {
            std::iter::repeat_n(
                (
                    r.style.contains(Style::HIDDEN),
                    r.style.contains(Style::EMPHASIS),
                    r.style.contains(Style::STRONG),
                ),
                r.source.len(),
            )
        })
        .collect()
}

#[test]
fn independent_batch_model_covers_unicode_contexts_and_mixed_runs() {
    let contexts = [
        "", "a", "中", "\u{a0}", "\u{2007}", " ", "。", "「", "😀", "𝄞", "\u{301}", "\u{2028}",
        "\u{85}",
    ];
    for marker in ["*", "**", "_", "__"] {
        for before in contexts {
            for first in contexts {
                for last in contexts {
                    for after in contexts {
                        let source = format!("p {before}{marker}{first}x{last}{marker}{after} z");
                        let doc = parse(&source);
                        assert_eq!(flags(&doc), model::marks(&source), "{source:?}");
                        assert!(doc.work().flanking_steps <= 48);
                    }
                }
            }
        }
    }
    for a in [
        "*", "**", "_", "__", "***", "___", "😀", "。", "\u{a0}", "\u{301}",
    ] {
        for b in ["*", "**", "_", "__", "中", "😀", "\u{a0}"] {
            let source = format!("p {a}中{b}文{a}。{b} z");
            assert_eq!(flags(&parse(&source)), model::marks(&source), "{source:?}");
        }
    }
}

#[test]
fn every_byte_split_prefix_and_eof_keeps_pending_scalars_and_delimiters() {
    let mut splits = 0;
    let mut prefixes = 0;
    for seed in [String::new(), format!("{}\n", "x".repeat(BLOCK_LOOKAHEAD))] {
        for tail in [
            "a*「中😀」*b",
            "😀__中__😀",
            "*\u{a0}x\u{a0}*",
            "。_中_。",
            "*𝄞*x",
            "*\u{2028}x*",
            "\u{301}_x_\u{301}",
            "*中\r\n文*。",
            "a**",
            "😀_",
            "*中\\**。",
            "*中&#160;*。",
            "*中`😀`文*。",
            "> - *中\n>\t文*。",
            "> *中\n文*。",
            "> *中\n\n文*。",
            "| *\u{a0}x* | __中__😀 |\n| - | - |\n",
        ] {
            let source = format!("{seed}{tail}");
            let expected = parse(&source);
            for cut in 0..=tail.len() {
                let mut doc = Utf8Document::new();
                doc.append(seed.as_bytes()).unwrap();
                doc.append(&tail.as_bytes()[..cut]).unwrap();
                doc.append(&tail.as_bytes()[cut..]).unwrap();
                doc.finish();
                assert_eq!(
                    support::snapshot(doc.document()),
                    support::snapshot(&expected),
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
                assert_eq!(doc.pending_bytes(), whole.pending_bytes());
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
    eprintln!("Unicode flanking: {splits} byte splits, {prefixes} byte prefixes/EOF");
}

#[test]
fn late_unicode_closers_isolate_headers_or_commit_once_on_failure() {
    for marker in ["*", "**", "_", "__"] {
        for punctuation in ["。", "😀", "\u{a0}", "\u{2007}"] {
            for prefix in ["", "> "] {
                let prior = format!("{prefix}[p](u) {marker}旧\n");
                let body = format!("尾{marker}{punctuation} [h](v) {}|b\n", "中".repeat(400));
                for ending in ["-|-\n", "wrong\n", "", "\n", "---\n"] {
                    let source = format!("{prior}{prefix}{body}{prefix}{ending}");
                    let late = parse(&source);
                    let success = ending == "-|-\n";
                    assert_eq!(
                        late.tables().len(),
                        usize::from(success),
                        "{marker} {punctuation} {prefix:?} {ending:?}"
                    );
                    if success {
                        let early = parse(&format!("{prior}{prefix}|{body}{prefix}{ending}"));
                        let late_flags = flags(&late);
                        let early_flags = flags(&early);
                        let start = prior.len() + prefix.len();
                        assert_eq!(late_flags[..start], early_flags[..start]);
                        assert_eq!(
                            late_flags[start..start + body.len()],
                            early_flags[start + 1..start + 1 + body.len()]
                        );
                    } else {
                        let at = late.source().find('旧').unwrap();
                        assert!(
                            late.runs_in(Span::new(at, at + 3))
                                .unwrap()
                                .next()
                                .unwrap()
                                .style
                                .contains(if marker.len() == 1 {
                                    Style::EMPHASIS
                                } else {
                                    Style::STRONG
                                })
                        );
                    }
                    assert_eq!(late.links().len(), 2);
                    assert_eq!(late.link_destination(LinkId(0)), Some("u"));
                    assert_eq!(late.link_destination(LinkId(1)), Some("v"));
                    assert!(late.work().inline_bytes <= source.len() as u64);
                    assert!(late.work().flanking_steps <= 24 * late.work().delimiter_steps);
                }
            }
        }
    }
}

fn style_at(doc: &Document, needle: &str) -> Style {
    let at = doc.source().find(needle).unwrap();
    doc.runs_in(Span::new(at, at + needle.len()))
        .unwrap()
        .next()
        .unwrap()
        .style
}

#[test]
fn late_unicode_closure_keeps_object_ids_mapping_and_shared_query_budget() {
    for marker in ["*", "**", "_", "__"] {
        let style = if marker.len() == 1 {
            Style::EMPHASIS
        } else {
            Style::STRONG
        };
        let mut doc = Document::new();
        doc.append(&format!(
            "> - {marker}中\\!😀 [签](/目标 \"标题\") $式$ `code` {}",
            "x".repeat(2048)
        ))
        .unwrap();
        let block = doc.block_at(0).unwrap();
        let container = doc.blocks()[block.0].container;
        let link = *doc.link_info(LinkId(0)).unwrap();
        let math = doc.maths()[0];
        let tile = doc.tile_key(TileId(0)).unwrap();
        assert!(!style_at(&doc, "中").contains(style));
        doc.append(&format!("尾{marker}。\n")).unwrap();
        doc.finish();
        assert_eq!(doc.block_at(0), Some(block));
        assert_eq!(doc.blocks()[block.0].container, container);
        assert_eq!(*doc.link_info(LinkId(0)).unwrap(), link);
        assert_eq!(doc.maths()[0], math);
        assert_ne!(doc.tile_key(TileId(0)).unwrap(), tile);
        assert!(style_at(&doc, "中").contains(style));
        assert!(style_at(&doc, "签").contains(style));
        assert!(style_at(&doc, "code").contains(Style::CODE));
        assert!(style_at(&doc, "code").contains(style));
        let span = Span::new(0, doc.source().len());
        let expected: Vec<_> = doc
            .display_runs_in(span)
            .unwrap()
            .map(|r| (r.source, r.style, r.link))
            .collect();
        let mut end = 0;
        for (source, _, _) in &expected {
            assert_eq!(source.start, end);
            end = source.end;
        }
        assert_eq!(end, span.end);
        for (at, ch) in doc.source().char_indices() {
            let slice = Span::new(at, at + ch.len_utf8());
            let run = doc.display_runs_in(slice).unwrap().next().unwrap();
            let whole = expected
                .iter()
                .find(|r| r.0.start <= at && at < r.0.end)
                .unwrap();
            assert_eq!(run.source, slice);
            assert_eq!((run.style, run.link), (whole.1, whole.2));
            assert_eq!(run.text, doc.text(slice).unwrap());
        }
        let mut cursor = doc.display_cursor(span).unwrap();
        let mut peer = doc.display_cursor(span).unwrap();
        assert_eq!(
            cursor
                .next(&doc, &mut Budget::new(WorkLimit::default()))
                .unwrap(),
            DisplayStep::BudgetExhausted
        );
        assert_eq!(cursor.source_position(), 0);
        let mut text = String::new();
        while cursor.source_position() < span.end {
            let mut budget = Budget::new(WorkLimit {
                bytes: 6,
                queries: 1,
                objects: 1,
                ..Default::default()
            });
            let before = cursor.source_position();
            let DisplayStep::Run(run) = cursor.next(&doc, &mut budget).unwrap() else {
                panic!("no progress")
            };
            assert_eq!(run.source.start, before);
            assert!(cursor.source_position() > before);
            assert_eq!(run.text, doc.text(run.source).unwrap());
            if !run.style.contains(Style::HIDDEN) {
                text.push_str(run.text);
            }
            let used = budget.used();
            let mut target = doc.decoded_link_destination(LinkId(0)).unwrap();
            let target_before = target.source_position();
            assert_eq!(
                target.advance(&mut budget),
                Err(Completion::BudgetExhausted)
            );
            assert_eq!(target.source_position(), target_before);
            assert_eq!(
                peer.next(&doc, &mut budget).unwrap(),
                DisplayStep::BudgetExhausted
            );
            assert_eq!(peer.source_position(), 0);
            assert_eq!(budget.used(), used);
            assert!(used.bytes <= 6 && used.queries <= 1 && used.objects <= 1);
        }
        assert_eq!(text, visible(&doc));
        assert_eq!(
            cursor
                .next(&doc, &mut Budget::new(WorkLimit::default()))
                .unwrap(),
            DisplayStep::Complete
        );
    }
}

#[test]
fn opaque_failed_and_scope_boundaries_retain_existing_dialects() {
    for (source, text, links, maths) in [
        ("a*「[中](/u)」*b", "a*「中」*b", 1, 0),
        ("*中 `*😀*` 文*。", "中 *😀* 文。", 0, 0),
        ("`*😀* *未闭合", "`*😀* *未闭合", 0, 0),
        ("*中 $*😀*$ 文*。", "中 *😀* 文。", 0, 1),
        ("$*😀* *未闭合", "$*😀* *未闭合", 0, 0),
        ("[*中*。](/u)", "中。", 1, 0),
        ("*中\n\n文*。", "*中\n\n文*。", 0, 0),
        ("> *中\n文*。", "中\n文。", 0, 0),
        ("> *中\n\n文*。", "*中\n\n文*。", 0, 0),
        // Entity values do not replace raw-source flanking characters.
        ("*&#160;x*", "\u{a0}x", 0, 0),
        // Tilde intentionally retains its independent ASCII whitespace dialect.
        ("~\u{a0}x\u{a0}~", "\u{a0}x\u{a0}", 0, 0),
        ("a~😀~b", "a😀b", 0, 0),
    ] {
        let doc = parse(source);
        assert_eq!(visible(&doc), text, "{source:?}");
        assert_eq!(doc.links().len(), links);
        assert_eq!(doc.maths().len(), maths);
    }
    for marker in ["*", "_"] {
        let source = format!("p {}中{}。", marker.repeat(65536), marker.repeat(65536));
        let doc = parse(&source);
        assert_eq!(visible(&doc), source);
        assert_eq!(doc.work().flanking_steps, 0);
        assert!(doc.work().inline_bytes <= source.len() as u64);
    }
}

#[test]
fn deep_million_byte_growth_and_historical_openers_never_revisit_boundaries() {
    let prefix = ">".repeat(970);
    for marker in ["*", "**", "_", "__"] {
        for success in [false, true] {
            let mut doc = Document::new();
            doc.append(&format!("{}\n\n", "[p](u) ".repeat(4096)))
                .unwrap();
            let link = *doc.link_info(LinkId(4095)).unwrap();
            doc.append(&format!(
                "{prefix}{marker}旧\n{prefix}尾{marker}。 {}",
                "x".repeat(2048)
            ))
            .unwrap();
            let before = doc.work();
            let chunk = "中😀".repeat(143);
            for _ in 0..1000 {
                doc.append(&chunk).unwrap();
            }
            let after = doc.work();
            assert_eq!(after.inline_bytes - before.inline_bytes, 1_001_000);
            assert_eq!(after.flanking_steps, before.flanking_steps);
            assert_eq!(after.delimiter_steps, before.delimiter_steps);
            assert_eq!(after.container_steps, before.container_steps);
            assert_eq!(after.label_bytes, before.label_bytes);
            doc.append(&format!(
                "|b\n{prefix}{}\n",
                if success { "-|-" } else { "wrong" }
            ))
            .unwrap();
            doc.finish();
            assert_eq!(doc.tables().len(), usize::from(success));
            assert_eq!(*doc.link_info(LinkId(4095)).unwrap(), link);
            assert!(doc.work().inline_bytes <= doc.source().len() as u64 + doc.work().table_bytes);
            assert!(doc.work().flanking_steps - before.flanking_steps < 24);
            assert!(doc.work().container_steps - before.container_steps < 10_000);
        }
    }
    let mut doc = Document::new();
    doc.append(&format!("p {}", "*a **b _c __d ".repeat(4096)))
        .unwrap();
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("中😀").unwrap();
    }
    assert_eq!(doc.work().flanking_steps, before.flanking_steps);
    assert_eq!(doc.work().delimiter_steps, before.delimiter_steps);
    assert_eq!(doc.work().inline_bytes - before.inline_bytes, 7000);
    doc.append("尾__。 尾_。 尾**。 尾*。\n\n").unwrap();
    doc.finish();
    assert!(doc.work().delimiter_steps <= doc.source().len() as u64);
    assert!(doc.work().flanking_steps <= 24 * doc.work().delimiter_steps);
}
