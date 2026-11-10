use mdstream::*;

#[path = "support/snapshot.rs"]
mod support;

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
fn partial_runs_consume_the_inner_edges_and_keep_unmatched_bytes() {
    for (source, shown, depth) in [
        ("__foo_ bar_", "foo bar", (2, 0)),
        ("*foo *bar**", "foo bar", (1, 0)),
        ("***foo** bar*", "foo bar", (1, 1)),
        ("foo***bar***baz", "foobarbaz", (0, 0)),
        ("foo******bar*********baz", "foobar***baz", (0, 0)),
        ("****foo*", "***foo", (1, 0)),
        ("**foo***", "foo*", (0, 1)),
        ("*foo**bar*", "foo**bar", (1, 0)),
    ] {
        let doc = parse(source);
        assert_eq!(visible(&doc), shown, "{source:?}");
        assert_eq!(depth_at(&doc, "foo"), depth, "{source:?}");
    }
}

#[path = "support/emphasis_model.rs"]
mod model;

fn depths(doc: &Document) -> Vec<(bool, usize, usize)> {
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

#[test]
fn independent_batch_model_covers_partial_runs_dual_boundaries_and_crossing() {
    let mut cases = 0;
    for marker in ['*', '_'] {
        for a in 1..=9 {
            for b in 1..=9 {
                for c in 1..=9 {
                    for boundary in [" ", "", "。", "a", "😀"] {
                        let m = |n| marker.to_string().repeat(n);
                        let source = format!("p {}中{boundary}{}文 {}尾 z", m(a), m(b), m(c));
                        assert_eq!(
                            depths(&parse(&source)),
                            model::depths(&source),
                            "{source:?}"
                        );
                        assert_eq!(
                            model::marks(&source),
                            depths(&parse(&source))
                                .into_iter()
                                .map(|(h, e, s)| (h, e > 0, s > 0))
                                .collect::<Vec<_>>()
                        );
                        cases += 1;
                    }
                }
            }
        }
    }
    for source in [
        "*a **b* c**",
        "***a _b*** c_",
        "***a\\**",
        "x**a*b***y",
        "**a*b**c*d",
        "*a**b**c***d*",
    ] {
        assert_eq!(depths(&parse(source)), model::depths(source), "{source:?}");
    }
    eprintln!("{cases} independent batch partial-run cases");
}

#[test]
fn every_byte_split_prefix_and_eof_preserves_partial_remainders() {
    for seed in [String::new(), format!("{}\n", "x".repeat(BLOCK_LOOKAHEAD))] {
        for tail in [
            "***中** 文*\r\n",
            "foo***中😀***baz",
            "__中_ 文_",
            "*中 *文**",
            "**中***",
            "*中**文*",
            "> - ***中\n>   文***\n",
            "***中 `文` $式$ [签](u)***\n",
            "***中 &amp; 文***",
            "***中\n\n文***",
            "| ***中*** | x |\n| - | - |\n",
            "***中\n**文** | x\n--- | ---\n",
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
                    "EOF {end} {tail:?}"
                );
            }
        }
    }
}

#[test]
fn partial_inherited_pairs_discard_or_commit_without_replaying_remainders() {
    for prefix in ["", "> ", "- "] {
        let continuation = if prefix == "- " { "  " } else { prefix };
        for (open, local) in [
            ("****", "**tail* end* "),
            ("***", "*tail** "),
            ("*", "**local end* "),
            ("____", "_tail__ end_ "),
        ] {
            let prior = format!("{prefix}{open}old\n");
            let body = format!("{local}{}|b\n", "中".repeat(400));
            for end in ["-|-\n", "wrong\n", "", "\n"] {
                let source = format!("{prior}{continuation}{body}{continuation}{end}");
                let doc = parse(&source);
                let success = end == "-|-\n";
                assert_eq!(
                    doc.tables().len(),
                    usize::from(success),
                    "{open:?} {local:?} {end:?}"
                );
                if success {
                    let early = parse(&format!("{prior}{continuation}|{body}{continuation}{end}"));
                    let at = prior.len() + continuation.len();
                    let a = depths(&doc);
                    let b = depths(&early);
                    assert_eq!(a[..at], b[..at]);
                    assert_eq!(a[at..at + body.len()], b[at + 1..at + 1 + body.len()]);
                } else if prefix.is_empty() {
                    assert_eq!(depths(&doc), model::depths(&source));
                }
                assert!(doc.work().delimiter_search_steps <= 6 * doc.work().delimiter_steps);
            }
        }
    }
}

#[test]
fn long_runs_failed_searches_and_partial_openers_have_linear_cumulative_work() {
    for size in [1024, 4096, 16384] {
        for source in [
            format!("p {}中{}。", "*".repeat(size + 1), "*".repeat(size + 2)),
            format!("p {}{}z", "*x ".repeat(size), "a**b ".repeat(size)),
            format!("p {}z{}!", "**x ".repeat(size), " c*".repeat(size * 2)),
        ] {
            let mut doc = Document::new();
            doc.append(&source).unwrap();
            doc.finish();
            let work = doc.work();
            assert!(work.inline_bytes <= source.len() as u64);
            assert!(work.delimiter_steps <= 3 * source.len() as u64);
            assert!(work.delimiter_search_steps <= 6 * work.delimiter_steps);
            assert!(work.tree_visits <= 24 * source.len() as u64 * usize::BITS as u64);
            assert!(doc.annotation_arena_stats().nodes <= 4 * source.len());
        }
    }
}

#[test]
fn deep_container_million_byte_growth_and_history_do_not_revisit_old_runs() {
    let mut doc = Document::new();
    doc.append(&format!(
        "{}\n\n{}***中 ***文 [签][r] {}",
        "[h](u) ".repeat(4096),
        ">".repeat(970),
        "x".repeat(2048)
    ))
    .unwrap();
    let info = *doc.link_info(LinkId(4096)).unwrap();
    let before = doc.work();
    let chunk = "中😀".repeat(143);
    for _ in 0..1000 {
        doc.append(&chunk).unwrap();
    }
    let after = doc.work();
    assert_eq!(after.delimiter_steps, before.delimiter_steps);
    assert_eq!(after.delimiter_search_steps, before.delimiter_search_steps);
    assert_eq!(after.tree_visits, before.tree_visits);
    assert_eq!(
        after.inline_bytes - before.inline_bytes,
        (chunk.len() * 1000) as u64
    );
    assert!(after.container_steps - before.container_steps < 2000);
    doc.append("尾** 末* 字***。\n\n").unwrap();
    assert_eq!(depth_at(&doc, "签"), (2, 2));
    let before_definition = doc.work();
    doc.append("[r]: /目标\n").unwrap();
    doc.finish();
    assert_eq!(*doc.link_info(LinkId(4096)).unwrap(), info);
    assert_eq!(doc.link_destination(LinkId(4096)), Some("/目标"));
    assert_eq!(depth_at(&doc, "签"), (2, 2));
    assert_eq!(
        doc.work().delimiter_search_steps,
        before_definition.delimiter_search_steps
    );
}

#[test]
fn growing_delimiter_runs_wait_for_a_successor_without_scanning_the_old_run() {
    let mut doc = Document::new();
    doc.append(&format!("p {} ", "x".repeat(BLOCK_LOOKAHEAD)))
        .unwrap();
    let before = doc.work();
    for _ in 0..4096 {
        doc.append("***").unwrap();
    }
    assert_eq!(doc.work().delimiter_steps, before.delimiter_steps);
    assert_eq!(
        doc.work().delimiter_search_steps,
        before.delimiter_search_steps
    );
    doc.append("中😀").unwrap();
    let open = doc.work();
    for _ in 0..4096 {
        doc.append("***").unwrap();
    }
    assert_eq!(doc.work().delimiter_steps, open.delimiter_steps);
    assert_eq!(
        doc.work().delimiter_search_steps,
        open.delimiter_search_steps
    );
    doc.finish();
    assert_eq!(depth_at(&doc, "中"), (0, 6144));
    assert!(doc.work().delimiter_search_steps <= 6 * doc.work().delimiter_steps);
    assert!(doc.work().inline_bytes <= doc.source().len() as u64);
    let mut split = Utf8Document::new();
    for chunk in doc.source().as_bytes().chunks(31) {
        split.append(chunk).unwrap();
    }
    split.finish();
    assert_eq!(support::snapshot(&doc), support::snapshot(split.document()));
}
