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
fn published_labels_reject_outer_closers_and_preserve_local_and_enclosing_pairs() {
    let mut cases = 0;
    for marker in ["*", "_", "**", "__", "***", "___", "****", "____"] {
        for image in ["", "!"] {
            for suffix in ["(u)", "[r]", "[]", ""] {
                let label = format!("签{marker}");
                let source =
                    format!("p {marker}前 {image}[{label}]{suffix} 后\n\n[r]: /u\n[{label}]: /u\n");
                let doc = parse(&source);
                assert_eq!(depths(&doc, "前"), (0, 0, false), "{source:?}");
                assert_eq!(depths(&doc, "签"), (0, 0, false), "{source:?}");
                assert_eq!(visible(&doc), format!("p {marker}前 {label} 后\n\n"));
                let source = format!(
                    "p {marker}前 {image}[签{marker} **中** _文_]{suffix} 后{marker}\n\n[r]: /u\n"
                );
                let doc = parse(&source);
                let outer = (marker.len() % 2, marker.len() / 2, false);
                assert_eq!(depths(&doc, "前"), outer, "{source:?}");
                assert_eq!(depths(&doc, "签"), outer, "{source:?}");
                assert_eq!(depths(&doc, "后"), outer, "{source:?}");
                assert_eq!(
                    depths(&doc, "中"),
                    (outer.0, outer.1 + 1, false),
                    "{source:?}"
                );
                assert_eq!(
                    depths(&doc, "文"),
                    (outer.0 + 1, outer.1, false),
                    "{source:?}"
                );
                cases += 1;
            }
        }
    }
    // The first two bytes of the closer satisfy a local strong opener; its
    // remainder must not consume the outside emphasis opener.
    for marker in ["*", "_"] {
        let source =
            format!("p {marker}前 [{marker}{marker}中{marker}{marker}{marker}](u) 后{marker}");
        let doc = parse(&source);
        assert_eq!(visible(&doc), format!("p 前 中{marker} 后"));
        assert_eq!(depths(&doc, "中"), (1, 1, false));
        assert_eq!(depths(&doc, "后"), (1, 0, false));
    }
    eprintln!("{cases} independent left-boundary models plus partial consumption");
}

#[test]
fn unpublished_labels_release_summaries_as_ordinary_emphasis_once() {
    for marker in ["*", "_", "**", "__"] {
        let expected = (marker.len() % 2, marker.len() / 2, false);
        for tail in ["", " [new](u)", " <https://x>"] {
            let source = format!("p {marker}前 [签{marker}{tail}");
            let doc = parse(&source);
            assert_eq!(depths(&doc, "前"), expected, "{source:?}");
            assert_eq!(depths(&doc, "签"), expected, "{source:?}");
        }
        for suffix in ["(u)", "[r]", "", "(bad z", "[missing"] {
            let source = format!(
                "p {marker}前 [签{}{marker}]{suffix}\n\n[r]: /u\n",
                "x".repeat(1100)
            );
            let published = matches!(suffix, "(u)" | "[r]");
            let doc = parse(&source);
            assert_eq!(doc.links().len(), usize::from(published));
            for size in [1, 7, 1024] {
                let mut streaming = Utf8Document::new();
                for bytes in source.as_bytes().chunks(size) {
                    streaming.append(bytes).unwrap();
                }
                streaming.finish();
                assert_eq!(
                    support::snapshot(streaming.document()),
                    support::snapshot(&doc),
                    "{marker} {suffix} chunks {size}"
                );
            }
            assert_eq!(
                depths(&doc, "签"),
                if published { (0, 0, false) } else { expected },
                "{marker} {suffix}"
            );
        }
        for suffix in ["", "(bad z", "[missing]", "[]"] {
            let source = format!("p {marker}前 [签{marker}]{suffix}");
            let doc = parse(&source);
            assert_eq!(doc.links().len(), 1);
            assert_eq!(depths(&doc, "签"), (0, 0, false), "{source:?}");
        }
    }
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
    for (label_source, label_text, link_source) in [
        ("[签 *中😀][r]*", "签 *中😀", "[签 *中😀][r]"),
        ("*前 [签* 中😀][r]", "签* 中😀", "[签* 中😀][r]"),
        ("__前 [签__ 中😀][r]", "签__ 中😀", "[签__ 中😀][r]"),
    ] {
        let mut doc = Document::new();
        doc.append(&format!(
            "p {} {label_source} {}",
            "x".repeat(1024),
            "z".repeat(64)
        ))
        .unwrap();
        let id = LinkId(0);
        let info = *doc.link_info(id).unwrap();
        let block = doc.block_at(info.source.start).unwrap();
        assert_eq!(doc.text(info.label), Some(label_text));
        assert_eq!(doc.text(info.source), Some(link_source));
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
        assert_eq!(shown, label_text);
    }
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
    // Resolve each deferred summary, push its opener, then discard at the edge.
    assert!(work.delimiter_steps - after.delimiter_steps <= 3 * 4096 + 14);
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
            "*前 [签*😀](u) 后*\r\n",
            "__前 [签__ _中_😀][r] 后__\n\n[r]: /u\n",
            "*前 [**中***😀][] 后*\n",
            "_前 [签_😀](bad z",
            "**前 [签**😀",
            "*前 [签*😀 [new](u)",
            "*前 [签*😀 <https://x>",
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

#[test]
fn inherited_left_closers_resolve_only_after_header_confirmation() {
    for marker in ["*", "_", "**", "__"] {
        for prefix in ["", "> ", "- "] {
            let continuation = if prefix == "- " { "  " } else { prefix };
            for ending in ["-|-\n", "wrong\n", ""] {
                for suffix in ["", "(u)", "[r]"] {
                    let source = format!(
                        "{prefix}p {marker}前 [签{marker}\n{continuation}末 _中_]{suffix} {}|b\n{continuation}{ending}",
                        "x".repeat(1100)
                    );
                    let doc = parse(&source);
                    let table = ending == "-|-\n";
                    assert_eq!(
                        doc.tables().len(),
                        usize::from(table),
                        "{marker} {prefix} {suffix} {ending:?}"
                    );
                    assert_eq!(doc.links().len(), usize::from(!table || suffix == "[r]"));
                    let prior = if table {
                        (marker.len() % 2, marker.len() / 2, false)
                    } else {
                        (0, 0, false)
                    };
                    assert_eq!(
                        depths(&doc, "前"),
                        prior,
                        "{marker:?} {prefix:?} {suffix:?} {ending:?}"
                    );
                    assert_eq!(
                        depths(&doc, "签"),
                        prior,
                        "{marker:?} {prefix:?} {suffix:?} {ending:?}"
                    );
                    assert_eq!(depths(&doc, "中"), (1, 0, false));
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
    }
}

#[test]
fn many_outside_openers_cannot_be_searched_or_consumed_by_label_closers() {
    for size in [1024, 4096, 16384] {
        let source = format!(
            "p {}前 [签 {}中](u) 后*",
            "*a _b ".repeat(size),
            "字* 字_ ".repeat(size)
        );
        let doc = parse(&source);
        assert_eq!(depths(&doc, "前"), (1, 0, false));
        assert_eq!(depths(&doc, "签"), (1, 0, false));
        assert_eq!(depths(&doc, "后"), (1, 0, false));
        let work = doc.work();
        assert!(work.inline_bytes <= source.len() as u64);
        assert!(work.delimiter_steps <= 3 * source.len() as u64);
        assert!(work.delimiter_search_steps <= 6 * work.delimiter_steps);
        assert!(work.tree_visits <= 24 * source.len() as u64 * usize::BITS as u64);
        assert!(doc.annotation_arena_stats().nodes <= 4 * source.len());
    }
}

fn check_replaced_inherited_label(replacement: &str) {
    let mut failures = Vec::new();
    for marker in ["*", "_", "**", "__"] {
        let styled = (marker.len() % 2, marker.len() / 2, false);
        for padding in [0, 1100] {
            for ending in ["-|-\n", "wrong\n", ""] {
                let table = ending == "-|-\n";
                for (prior, row, old_depth, end_depth) in [
                    (
                        format!("p {marker}before [inside{marker}\n"),
                        format!("{replacement} end"),
                        styled,
                        (0, 0, false),
                    ),
                    (
                        format!("p [old {marker}inside\n"),
                        format!("{replacement} end{marker}"),
                        if table { (0, 0, false) } else { styled },
                        if table { (0, 0, false) } else { styled },
                    ),
                    (
                        format!("p {marker}before [inside{marker}\n"),
                        format!("{marker}cell{marker} {replacement} end"),
                        styled,
                        (0, 0, false),
                    ),
                    (
                        format!("p {marker}before [inside!{marker}?\n"),
                        format!("{replacement} end"),
                        styled,
                        (0, 0, false),
                    ),
                ] {
                    let source = format!("{prior}{row} {}|b\n{ending}", "x".repeat(padding));
                    let doc = parse(&source);
                    // The model keeps pairs wholly in the old prose, while a
                    // confirmed first cell cannot close an old prose opener.
                    if doc.tables().len() != usize::from(table)
                        || depths(&doc, "inside") != old_depth
                        || depths(&doc, "end") != end_depth
                        || (prior.contains("before") && depths(&doc, "before") != styled)
                        || (row.contains("cell") && depths(&doc, "cell") != styled)
                    {
                        failures.push(format!(
                            "{prior:?} {row:?} padding={padding} ending={ending:?}: tables={}, inside={:?}, end={:?}",
                            doc.tables().len(), depths(&doc, "inside"), depths(&doc, "end")
                        ));
                    }
                    assert_eq!(doc.links().len(), 1);
                    let info = doc.link_info(LinkId(0)).unwrap();
                    assert_eq!(doc.text(info.source), Some(replacement));
                    let snapshot = support::snapshot(&doc);
                    for size in [1, 7] {
                        let mut streamed = Utf8Document::new();
                        for bytes in source.as_bytes().chunks(size) {
                            streamed.append(bytes).unwrap();
                        }
                        streamed.finish();
                        assert_eq!(support::snapshot(streamed.document()), snapshot);
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn replaced_inherited_label_by_bracket_preserves_summary_line_boundaries() {
    check_replaced_inherited_label("[new](u)");
    check_replaced_inherited_label("[new签😀](u)");
}

#[test]
fn replaced_inherited_label_by_autolink_preserves_summary_line_boundaries() {
    check_replaced_inherited_label("<https://x>");
}
