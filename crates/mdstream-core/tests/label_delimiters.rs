use mdstream::{
    Budget, Completion, DisplayStep, Document, LinkId, Span, Style, Utf8Document, WorkLimit,
};

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

fn depths(doc: &Document, text: &str) -> (usize, usize, bool) {
    let at = doc.source().find(text).unwrap();
    let run = doc
        .runs_in(Span::new(at, at + text.len()))
        .unwrap()
        .next()
        .unwrap();
    (
        run.emphasis_depth,
        run.strong_depth,
        run.style.contains(Style::STRIKE),
    )
}

#[test]
fn published_labels_end_unmatched_openers_but_keep_outer_and_confirmed_styles() {
    let mut cases = 0;
    for (outer, expected) in [
        ("", (0, 0, false)),
        ("*", (1, 0, false)),
        ("**", (0, 1, false)),
    ] {
        for inner in ["*", "**", "_", "__", "~", "~~"] {
            for image in ["", "!"] {
                for suffix in ["(u)", "[r]", "[]", ""] {
                    // The inner run is open-only. An independent scope model
                    // leaves it literal and permits only the outer pair.
                    let source = format!(
                        "p {outer}前 {image}[签 {inner}中]{suffix} 后{outer} 尾{inner} .\n\n[r]: /u\n"
                    );
                    let doc = parse(&source);
                    for text in ["前", "签", "中", "后"] {
                        assert_eq!(depths(&doc, text), expected, "{source:?} {text}");
                    }
                    assert_eq!(depths(&doc, "尾"), (0, 0, false), "{source:?}");
                    assert!(
                        visible(&doc).contains(&format!("签 {inner}中")),
                        "{source:?}"
                    );
                    assert_eq!(doc.links().len(), 1);
                    cases += 1;
                }
            }
        }
    }
    for (source, shown, middle) in [
        ("[签 *中* **文** _末](u)*", "签 中 文 _末*", (1, 0, false)),
        ("*[签 **中** _末](u)*", "签 中 _末", (1, 1, false)),
        ("~~[签 ~中~ *末](u)~~", "签 中 *末", (0, 0, true)),
        ("[签 ***中](u)***", "签 ***中***", (0, 0, false)),
    ] {
        let doc = parse(source);
        assert_eq!(visible(&doc), shown);
        assert_eq!(depths(&doc, "中"), middle);
    }
    eprintln!("{cases} independent label-boundary cases");
}

#[test]
fn unfinished_failed_and_overlong_labels_have_explicit_boundaries() {
    // No closing bracket: the candidate remains ordinary inline text.
    assert_eq!(visible(&parse("[签 *中 后*")), "[签 中 后");
    // Published shortcut references keep the existing hidden-bracket dialect,
    // even when the destination is undefined or an explicit suffix fails.
    for source in ["[签 *中]*", "[签 *中](bad *", "[签 *中][missing]*"] {
        let doc = parse(source);
        assert_eq!(depths(&doc, "中"), (0, 0, false), "{source:?}");
    }
    let label = format!("签 *中{}", "x".repeat(1100));
    for (suffix, linked) in [("(u)", true), ("[r]", true), ("", false), ("(bad z", false)] {
        let source = format!("[{label}]{suffix}*\n\n[r]: /u\n");
        let doc = parse(&source);
        assert_eq!(doc.links().len(), usize::from(linked));
        // Long labels cannot publish shortcuts. Only a successful explicit
        // destination/reference closes their scope; failure keeps old behavior.
        assert_eq!(
            depths(&doc, "中"),
            (usize::from(!linked), 0, false),
            "{suffix}"
        );
    }
}

#[test]
fn source_mapping_ids_late_definitions_and_consumers_share_the_frame_budget() {
    let mut doc = Document::new();
    doc.append(&format!(
        "p {} [签 *中😀][r]* {}",
        "x".repeat(1024),
        "z".repeat(64)
    ))
    .unwrap();
    let id = LinkId(0);
    let info = *doc.link_info(id).unwrap();
    let block = doc.block_at(info.source.start).unwrap();
    assert_eq!(doc.text(info.label), Some("签 *中😀"));
    assert_eq!(doc.text(info.source), Some("[签 *中😀][r]"));
    assert_eq!(depths(&doc, "中"), (0, 0, false));
    doc.append("\n\n[r]: /目标&amp; \"标&amp;题\"\n").unwrap();
    doc.finish();
    assert_eq!(*doc.link_info(id).unwrap(), info);
    assert_eq!(doc.block_at(info.source.start), Some(block));
    assert_eq!(doc.link_destination(id), Some("/目标&amp;"));
    let span = info.source;
    let mut cursor = doc.display_cursor(span).unwrap();
    let mut peer = doc.display_cursor(span).unwrap();
    let limit = WorkLimit {
        bytes: 6,
        queries: 1,
        objects: 1,
        ..Default::default()
    };
    assert_eq!(
        cursor
            .next(&doc, &mut Budget::new(WorkLimit::default()))
            .unwrap(),
        DisplayStep::BudgetExhausted
    );
    let mut shown = String::new();
    while cursor.source_position() < span.end {
        let mut budget = Budget::new(limit);
        let DisplayStep::Run(run) = cursor.next(&doc, &mut budget).unwrap() else {
            panic!("no progress")
        };
        assert_eq!(doc.text(run.source), Some(run.text));
        assert_eq!(run.replacement, None);
        assert_eq!((run.emphasis_depth, run.strong_depth), (0, 0));
        if !run.style.contains(Style::HIDDEN) {
            shown.push_str(run.text);
        }
        let used = budget.used();
        assert!(used.bytes <= 6 && used.queries <= 1 && used.objects <= 1);
        assert_eq!(
            peer.next(&doc, &mut budget).unwrap(),
            DisplayStep::BudgetExhausted
        );
        assert_eq!(peer.source_position(), span.start);
        assert_eq!(
            doc.decoded_link_destination(id)
                .unwrap()
                .advance(&mut budget),
            Err(Completion::BudgetExhausted)
        );
        assert_eq!(
            doc.decoded_link_title(id).unwrap().advance(&mut budget),
            Err(Completion::BudgetExhausted)
        );
        assert_eq!(budget.used(), used);
    }
    assert_eq!(shown, "签 *中😀");
}

#[test]
fn inherited_labels_drop_openers_only_when_the_late_header_fails() {
    for prefix in ["", "> ", "- "] {
        let continuation = if prefix == "- " { "  " } else { prefix };
        for inner in ["*", "**", "_", "~", "~~"] {
            let prior = format!("{prefix}a [old {inner}inner\n");
            let row = format!("{continuation}end] {}|b\n", "中".repeat(400));
            for ending in [
                "-|-\n".to_string(),
                format!("wrong{inner}\n"),
                String::new(),
            ] {
                let doc = parse(&format!("{prior}{row}{continuation}{ending}"));
                let success = ending == "-|-\n";
                assert_eq!(
                    doc.tables().len(),
                    usize::from(success),
                    "{inner:?} {ending:?}"
                );
                assert_eq!(doc.links().len(), usize::from(!success));
                assert_eq!(depths(&doc, "inner"), (0, 0, false));
                assert_eq!(depths(&doc, "end"), (0, 0, false));
                if success {
                    let early = parse(&format!(
                        "{prior}{continuation}|end] {}|b\n{continuation}{ending}",
                        "中".repeat(400)
                    ));
                    assert_eq!(depths(&early, "end"), depths(&doc, "end"));
                    assert_eq!(early.links().len(), doc.links().len());
                }
            }
        }
        // No bounded shortcut can be published for this inherited long label.
        // Its staged explicit destination must end the label on fallback too.
        let prior = format!("{prefix}a [old *inner {}\n", "x".repeat(1100));
        let row = format!("{continuation}end](u) {}|b\n", "中".repeat(400));
        for ending in ["-|-\n", "wrong*\n", ""] {
            let source = format!("{prior}{row}{continuation}{ending}");
            let doc = parse(&source);
            let success = ending == "-|-\n";
            assert_eq!(doc.tables().len(), usize::from(success));
            assert_eq!(doc.links().len(), usize::from(!success));
            assert_eq!(
                depths(&doc, "inner"),
                (0, 0, false),
                "{prefix:?} {ending:?}"
            );
            let mut streamed = Utf8Document::new();
            for bytes in source.as_bytes().chunks(7) {
                streamed.append(bytes).unwrap();
            }
            streamed.finish();
            assert_eq!(
                support::snapshot(streamed.document()),
                support::snapshot(&doc)
            );
        }
    }
}

#[test]
fn deep_containers_million_byte_labels_and_history_have_linear_cumulative_work() {
    let mut doc = Document::new();
    doc.append(&format!(
        "{}\n\n{}**前 [签 {}中",
        "[h](u) ".repeat(4096),
        ">".repeat(970),
        "*a ".repeat(4096)
    ))
    .unwrap();
    let before = doc.work();
    let history = *doc.link_info(LinkId(4095)).unwrap();
    let chunk = "中😀".repeat(143);
    for _ in 0..1000 {
        doc.append(&chunk).unwrap();
    }
    let after = doc.work();
    assert_eq!(
        after.inline_bytes - before.inline_bytes,
        (1000 * chunk.len()) as u64
    );
    assert_eq!(after.delimiter_steps, before.delimiter_steps);
    assert_eq!(after.delimiter_search_steps, before.delimiter_search_steps);
    assert_eq!(after.tree_visits, before.tree_visits);
    assert!(after.container_steps - before.container_steps < 2000);
    doc.append("](u) 后** 尾*\n\n").unwrap();
    doc.finish();
    assert_eq!(depths(&doc, "前"), (0, 1, false));
    assert_eq!(depths(&doc, "签"), (0, 1, false));
    assert_eq!(depths(&doc, "后"), (0, 1, false));
    assert_eq!(depths(&doc, "尾"), (0, 0, false));
    assert_eq!(*doc.link_info(LinkId(4095)).unwrap(), history);
    let work = doc.work();
    assert!(work.delimiter_steps - after.delimiter_steps <= 4110);
    assert!(work.inline_bytes <= doc.source().len() as u64);
    assert!(work.delimiter_steps <= 3 * doc.source().len() as u64);
    assert!(work.delimiter_search_steps <= 6 * work.delimiter_steps);
    assert!(work.tree_visits <= 24 * doc.source().len() as u64 * usize::BITS as u64);
    assert!(doc.annotation_arena_stats().nodes <= 4 * doc.source().len());
    for size in [1024, 4096, 16384] {
        let source = format!("p {}z", "[签 *中](u)* ".repeat(size));
        let doc = parse(&source);
        assert_eq!(doc.links().len(), size);
        assert!(doc.work().delimiter_steps <= 3 * source.len() as u64);
        assert!(doc.work().delimiter_search_steps <= 6 * doc.work().delimiter_steps);
        assert_eq!(depths(&doc, "中"), (0, 0, false));
    }
}

#[test]
fn all_utf8_splits_prefixes_and_eof_preserve_label_boundaries() {
    for seed in [String::new(), format!("{}\n", "x".repeat(1024))] {
        for tail in [
            "[签 *中😀](u)*\r\n",
            "**前 [签 _中][r] 后** 尾_\n\n[r]: /u\n",
            "![签 ~~中][] 尾~~\n",
            "> - [签 *中\n>   文](u)*\n",
            "[签 *中](bad *",
            "[签 *中 后*",
            "| [签 *中](u)* | b |\n| - | - |\n",
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
            let mut streaming = Utf8Document::new();
            streaming.append(seed.as_bytes()).unwrap();
            for end in 1..=tail.len() {
                streaming.append(&tail.as_bytes()[end - 1..end]).unwrap();
                let mut whole = Utf8Document::new();
                whole.append(seed.as_bytes()).unwrap();
                whole.append(&tail.as_bytes()[..end]).unwrap();
                assert_eq!(
                    support::snapshot(streaming.document()),
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
            }
        }
    }
}
