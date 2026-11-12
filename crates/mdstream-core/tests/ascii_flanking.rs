#[path = "support/header_confirmation.rs"]
mod header_oracle;
use mdstream::*;

#[path = "support/emphasis_model.rs"]
mod model;
#[path = "support/snapshot.rs"]
mod support;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

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

fn visible(doc: &Document) -> String {
    doc.display_runs_in(Span::new(0, doc.source().len()))
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect()
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
fn punctuation_and_word_counterexamples_keep_literal_or_close_the_right_pair() {
    for (source, expected) in [
        ("a*\"foo\"*", "a*\"foo\"*"),
        ("a**\"foo\"**", "a**\"foo\"**"),
        ("aa_\"bb\"_cc", "aa_\"bb\"_cc"),
        ("aa__\"bb\"__cc", "aa__\"bb\"__cc"),
        ("*(*foo)", "*(*foo)"),
        ("**(**foo)", "**(**foo)"),
        ("foo-_(bar)_", "foo-(bar)"),
        ("foo*bar*baz", "foobarbaz"),
        ("foo_bar_baz", "foo_bar_baz"),
        ("中_文_尾", "中_文_尾"),
        ("_中_文", "_中_文"),
        ("中__文__尾", "中__文__尾"),
        ("*\u{a0}x\u{a0}*", "*\u{a0}x\u{a0}*"),
        ("a*£*b", "a*£*b"),
        ("a~\"x\"~b", "a\"x\"b"),
    ] {
        assert_eq!(visible(&parse(source)), expected, "{source:?}");
    }
}

#[test]
fn independent_batch_model_covers_categories_mixed_runs_and_crossing() {
    let contexts = [
        "", " ", "\t", "!", "\"", "(", ")", ",", ".", ":", "a", "9", "中", "\u{a0}",
    ];
    for marker in ["*", "**", "_", "__"] {
        for before in contexts {
            for first in contexts {
                for last in contexts {
                    for after in contexts {
                        let source = format!("p {before}{marker}{first}x{last}{marker}{after} z");
                        assert_eq!(flags(&parse(&source)), model::marks(&source), "{source:?}");
                    }
                }
            }
        }
    }
    for a in [
        "*", "**", "_", "__", "***", "___", "*a ", "_a ", "x*", "x_", " ", "!", "中", "\\*",
    ] {
        for b in ["*", "**", "_", "__", " ", "!", "中"] {
            for c in ["*", "**", "_", "__", " ", "!", "中"] {
                let source = format!("p {a}{b}{c} z");
                assert_eq!(flags(&parse(&source)), model::marks(&source), "{source:?}");
            }
        }
    }
}

#[test]
fn every_utf8_split_prefix_and_eof_preserves_pending_run_semantics() {
    for seed in [String::new(), format!("{}\n", "x".repeat(BLOCK_LOOKAHEAD))] {
        for tail in [
            "a*\"中😀\"*",
            "aa__\"bb\"__cc",
            "_中_文",
            "*(*foo)",
            "foo-_(bar)_!",
            "*a **b* c**",
            "*a\r\nb*!",
            "a*",
            "a**",
            "a_",
            "a___",
            "*a\\**!",
            "*a &amp;*?",
            "_中\u{a0}_",
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
                    "{tail:?} cut {cut}"
                );
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
                whole.finish();
                eof.finish();
                assert_eq!(
                    support::snapshot(eof.document()),
                    support::snapshot(whole.document()),
                    "{tail:?} EOF {end}"
                );
            }
        }
    }
}

#[test]
fn late_punctuation_closers_match_independent_cells_or_commit_on_failure() {
    for marker in ["*", "**", "_", "__"] {
        for punctuation in ["!", ",", ")", ":"] {
            for prefix in ["", "> "] {
                let prior = format!("{prefix}[p](u) {marker}prior\n");
                let body = format!("end{marker}{punctuation} [h](v) {}|b\n", "x".repeat(1100));
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
                        assert!(style_at(&late, "prior").contains(if marker.len() == 1 {
                            Style::EMPHASIS
                        } else {
                            Style::STRONG
                        }));
                    }
                    assert_eq!(late.links().len(), 2);
                    assert_eq!(late.link_destination(LinkId(0)), Some("u"));
                    assert_eq!(late.link_destination(LinkId(1)), Some("v"));
                    assert!(
                        late.work().inline_bytes
                            <= source.len() as u64 + late.work().header_replay_bytes
                    );
                }
            }
        }
    }
    // Tilde retains its existing whitespace-only close/open classification.
    for marker in ["~", "~~"] {
        let doc = parse(&format!(
            "{marker}prior\nend{marker}! {}|b\n-|-\n",
            "x".repeat(1100)
        ));
        header_oracle::assert_scope_composition(&doc);
    }
}

#[test]
fn late_closure_keeps_object_ids_mapping_and_shared_query_budget() {
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
        doc.append(&format!("尾{marker}!\n")).unwrap();
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
fn objects_failed_candidates_and_scope_boundaries_keep_existing_priority() {
    for (source, text, links, maths) in [
        ("a*\"[x](/u)\"*", "a*\"x\"*", 1, 0),
        ("*a `*raw*` b*!", "a *raw* b!", 0, 0),
        ("`*raw* *unfinished", "`raw *unfinished", 0, 0),
        ("*a $*raw*$ b*!", "a *raw* b!", 0, 1),
        ("$*raw* *unfinished", "$*raw* *unfinished", 0, 0),
        ("[*label*!](/u)", "label!", 1, 0),
        ("a*_label_*b", "a*label*b", 0, 0),
        ("x***literal***!", "xliteral!", 0, 0),
        ("x___literal___!", "x___literal___!", 0, 0),
        ("*open\n\nclose*!", "*open\n\nclose*!", 0, 0),
        ("> *open\nplain*!", "open\nplain!", 0, 0),
        ("> *open\n\nplain*!", "*open\n\nplain*!", 0, 0),
    ] {
        let doc = parse(source);
        assert_eq!(visible(&doc), text, "{source:?}");
        assert_eq!(doc.links().len(), links);
        assert_eq!(doc.maths().len(), maths);
    }
}

#[test]
fn deep_million_byte_stream_and_historical_openers_have_linear_counts() {
    let prefix = ">".repeat(970);
    for marker in ["*", "**", "_", "__"] {
        for success in [false, true] {
            let mut doc = Document::new();
            doc.append(&format!("{}\n\n", "[p](u) ".repeat(4096)))
                .unwrap();
            let link = *doc.link_info(LinkId(4095)).unwrap();
            doc.append(&format!(
                "{prefix}{marker}old\n{prefix}end{marker}! {}",
                "x".repeat(2048)
            ))
            .unwrap();
            let before = doc.work();
            for _ in 0..1000 {
                doc.append(&"x".repeat(1000)).unwrap();
            }
            assert_eq!(doc.work().inline_bytes - before.inline_bytes, 1_000_000);
            assert_eq!(doc.work().delimiter_steps, before.delimiter_steps);
            assert_eq!(doc.work().container_steps, before.container_steps);
            assert_eq!(doc.work().label_bytes, before.label_bytes);
            doc.append(&format!(
                "|b\n{prefix}{}\n",
                if success { "-|-" } else { "wrong" }
            ))
            .unwrap();
            doc.finish();
            assert_eq!(doc.tables().len(), usize::from(success));
            assert_eq!(*doc.link_info(LinkId(4095)).unwrap(), link);
            assert!(doc.work().inline_bytes <= doc.source().len() as u64 + doc.work().table_bytes);
            assert!(doc.work().delimiter_steps - before.delimiter_steps < 12);
            assert!(doc.work().container_steps - before.container_steps < 10_000);
        }
    }
    let mut doc = Document::new();
    doc.append(&format!("p {}", "*a **b _c __d ".repeat(4096)))
        .unwrap();
    let before = doc.work();
    assert!(before.delimiter_steps > 16_000);
    for _ in 0..1000 {
        doc.append("中😀").unwrap();
    }
    assert_eq!(doc.work().delimiter_steps, before.delimiter_steps);
    assert_eq!(doc.work().inline_bytes - before.inline_bytes, 7000);
    doc.append("end__! end_! end**! end*!\n\n").unwrap();
    doc.finish();
    // Each opener is pushed/popped at most once; finish visits only the stack.
    assert!(doc.work().delimiter_steps <= doc.source().len() as u64);
    assert!(doc.work().inline_bytes <= doc.source().len() as u64 + doc.work().header_replay_bytes);
}
