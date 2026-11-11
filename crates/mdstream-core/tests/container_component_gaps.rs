use mdstream::*;
#[path = "support/snapshot.rs"]
mod support;

fn parse(source: &str) -> Document {
    let mut d = Document::new();
    d.append(source).unwrap();
    d.finish();
    d
}

fn shown(d: &Document) -> String {
    d.display_runs_in(Span::new(0, d.source().len()))
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect()
}

#[test]
fn container_component_gaps_have_independent_endings_and_single_line_metadata() {
    for (first, next) in [("> ", "> "), ("- ", "  "), ("> - > ", ">   > ")] {
        for image in ["", "!"] {
            for eol in ["\n", "\r\n"] {
                for (body, target, title) in [
                    (format!("({eol}{next}u)"), "u", None),
                    (format!("(u{eol}{next})"), "u", None),
                    (format!("(u{eol}{next}'t')"), "u", Some("t")),
                    (format!("(u 't'{eol}{next})"), "u", Some("t")),
                    (
                        format!("({eol}{next}<a b>{eol}{next}'t'{eol}{next})"),
                        "a b",
                        Some("t"),
                    ),
                    (format!("({eol}{next})"), "", None),
                ] {
                    let source = format!("{first}{image}[中🙂]{body}\n");
                    let d = parse(&source);
                    assert_eq!(d.link_destination(LinkId(0)), Some(target), "{source:?}");
                    assert_eq!(d.link_title(LinkId(0)), title, "{source:?}");
                    let info = d.link_info(LinkId(0)).unwrap();
                    assert_eq!(info.source, Span::new(first.len(), source.len() - 1));
                    assert_eq!(
                        info.kind,
                        if image.is_empty() {
                            LinkKind::Link
                        } else {
                            LinkKind::Image
                        }
                    );
                    assert_eq!(info.state, SyntaxState::Sealed);
                    assert_eq!(d.text(info.label), Some("中🙂"));
                    assert_eq!(shown(&d), "中🙂\n");
                }
            }
        }
    }
}

#[test]
fn container_gap_title_body_newline_is_rejected_and_failure_prefix_is_opaque() {
    for image in ["", "!"] {
        for eol in ["\n", "\r\n"] {
            for (tail, display) in [
                ("'a&amp;'x)\n", "'a&amp;'x)\n"),
                ("'a&amp;", "'a&amp;"),
                ("'\\* *中*'&amp;)\n", "'\\* *中*'&)\n"),
            ] {
                let source = format!("> {image}[a](u{eol}> {tail}");
                let d = parse(&source);
                assert_eq!(d.link_destination(LinkId(0)), None);
                assert_eq!(shown(&d), format!("a(u\n{display}"), "{source:?}");
                assert_eq!(
                    d.link_info(LinkId(0)).unwrap().source,
                    Span::new(2, 2 + image.len() + 3)
                );
            }
            let source = format!("> {image}[a](u{eol}> 't{eol}> rest')\n");
            let d = parse(&source);
            assert_eq!(d.link_destination(LinkId(0)), None, "B must not enable C");
        }
    }
}

#[test]
fn container_component_gaps_respect_actual_scope_and_syntax_roles() {
    for (source, success) in [
        ("> [a](u\n>    't')\n", true),
        ("> [a](u\n    't')\n", true),
        ("> [a](u\n>\t't')\n", true),
        ("> - > [a](u\n> 't')\n", true),
        ("> [a](u\n>     't')\n", false),
        ("> [a](u\n>\t  't')\n", false),
        ("> [a](u\n>\n> 't')\n", false),
        ("> [a](u\n> > 't')\n", false),
        ("> - [a](u\n> 't')\n", true),
        ("- [a](u\n- 't')\n", false),
        ("> [a](u\n> # 't')\n", false),
        ("> [a](u\n> ===\n> 't')\n", false),
        ("> [a](u\n> ```\n> 't')\n> ```\n", false),
        ("> [a](u\n> $$\n> 't')\n> $$\n", false),
        ("> [a](u\n> [^f]: 't')\n", false),
        ("> # [a](u\n> 't')\n", false),
        ("> [a](<u\n> rest>)\n", false),
        ("> [a](u(\n> rest))\n", false),
        ("> [a](u\\\n> 't')\n", false),
        ("> [a](u\r> 't')\n", false),
        ("> [a](u 't\n> rest')\n", false),
        ("> [a]\n> (u)\n", false),
        ("> [a](u|b\n> -|-\n> 't')|c\n", false),
        ("> `[a](u\n> )`\n", false),
        ("> $[a](u\n> )$\n", false),
        ("> [^f]: [a](u\n> 't')\n", false),
    ] {
        let d = parse(source);
        assert_eq!(
            d.link_destination(LinkId(0)),
            success.then_some("u"),
            "{source:?}"
        );
    }
}

#[test]
fn container_gap_utf8_chunks_and_every_prefix_eof_match_the_whole_model() {
    for source in [
        "> ![中🙂](\r\n>\t<a&amp;>\n> 't\\\''\r\n>\t)\n",
        "- [中](u\n  'a&amp;'&amp;)\n",
        "> [中](u\n> 't\n> rest')\n",
        "> [a](u\n> > 't')\n",
        "> [r\n> bad](\n> u)\n",
        "> ![中](u\r\n> 'a\\* *b*🙂",
    ] {
        let whole = support::snapshot(&parse(source));
        for split in 0..=source.len() {
            let mut d = Utf8Document::new();
            d.append(&source.as_bytes()[..split]).unwrap();
            d.append(&source.as_bytes()[split..]).unwrap();
            d.finish();
            assert_eq!(
                support::snapshot(d.document()),
                whole,
                "split={split}: {source:?}"
            );
        }
        for end in 0..=source.len() {
            let mut whole = Utf8Document::new();
            whole.append(&source.as_bytes()[..end]).unwrap();
            whole.finish();
            let mut bytes = Utf8Document::new();
            for byte in &source.as_bytes()[..end] {
                bytes.append(&[*byte]).unwrap();
            }
            bytes.finish();
            assert_eq!(
                support::snapshot(bytes.document()),
                support::snapshot(whole.document()),
                "end={end}: {source:?}"
            );
        }
    }
}

#[test]
fn structural_prefix_budget_is_separate_from_target_title_and_syntax_limits() {
    for len in [4095, 4096, 4097] {
        for target in [false, true] {
            let source = if target {
                format!("> [a](\n> {})\n", "d".repeat(len))
            } else {
                format!("> [a](u\n> '{}')\n", "t".repeat(len))
            };
            let d = parse(&source);
            assert_eq!(
                d.link_destination(LinkId(0)).is_some(),
                len <= 4096,
                "len={len} target={target}"
            );
            if len <= 4096 {
                assert_eq!(
                    d.link_destination(LinkId(0)).unwrap().len(),
                    if target { len } else { 1 }
                );
                if !target {
                    assert_eq!(d.link_title(LinkId(0)).unwrap().len(), len);
                }
            }
        }
    }
    // Exactly 8256 real syntax bytes. Absolute span is two bytes too long.
    for extra in [0, 1] {
        let source = format!(
            "> [a]({}{}\n> '{}' )\n",
            " ".repeat(59 + extra),
            "d".repeat(4096),
            "t".repeat(4096)
        );
        let d = parse(&source);
        assert_eq!(d.link_destination(LinkId(0)).is_some(), extra == 0);
    }
    // Prefix before opening a short title cannot be subtracted from title.len().
    let source = format!("{}[a](u\n{}'t')\n", "> ".repeat(80), "> ".repeat(80));
    let d = parse(&source);
    assert_eq!(d.link_title(LinkId(0)), Some("t"));
}

#[test]
fn old_container_table_roles_survive_new_gap_and_sticky_divergence() {
    for (first, next, header) in [
        ("> ", "> ", true),
        ("- ", "  ", true),
        ("> - > ", ">   > ", true),
        ("> ", ">\t", true),
        ("> ", "", false),
    ] {
        for width in [1023, 1024, 1025, 1100] {
            for (prior, closer, eligible) in [
                ("*prior [a](u", "end*and", false),
                ("_prior [a](u", "end_and", true),
                ("~~prior [a](u", "end~~and", false),
                ("*prior [a](u", "end* ", true),
                ("_prior [a](u", "end_ ", true),
                ("~~prior [a](u", "end~~ ", true),
                ("[a](u", "end", true),
                ("`prior", "end`", true),
                ("$prior", "end$", true),
                ("[prior", "end](u)", true),
                ("`prior", "*end*`", true),
                ("$prior", "*end*$", true),
            ] {
                let source = format!(
                    "{first}{prior}\n{next}{closer}{} | b\n{next}-|-\n",
                    "x".repeat(width)
                );
                let d = parse(&source);
                assert_eq!(
                    d.tables().len(),
                    usize::from(header && eligible),
                    "{first:?}/{next:?} width={width} prior={prior:?} closer={closer:?}"
                );
                if prior.contains("[a]") {
                    assert_eq!(d.link_destination(LinkId(0)), None);
                }
            }
        }
    }
}

#[test]
fn container_opaque_failure_source_display_and_decoded_controls_share_micro_budgets() {
    for image in ["", "!"] {
        for ending in ["\n", "\r\n"] {
            for tail in [format!("'&amp;){ending}"), String::new()] {
                let prefix = "[b](<p&amp;中> 't&#10;') ";
                let source = format!("> {prefix}{image}[a](u{ending}>\t'中🙂&#10;\\*{tail}");
                let doc = parse(&source);
                let whole = Span::new(0, source.len());
                let raw: String = doc.runs_in(whole).unwrap().map(|run| run.text).collect();
                assert_eq!(raw, source);
                let expected = format!(
                    "b a(u\n  '中🙂&#10;\\*{}",
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
fn container_stable_shortcut_and_reference_survive_pending_failure_eof_and_atomic_upgrade() {
    for image in ["", "!"] {
        for (tail, success) in [
            ("'t')\n", true),
            ("'t'x)\n", false),
            ("'t", false),
            ("'t\n> rest')\n", false),
        ] {
            let mut d = Document::new();
            d.append(&format!("> p {} {image}[old]", "x".repeat(1100)))
                .unwrap();
            let original = *d.link_info(LinkId(0)).unwrap();
            let target = d.links()[0];
            let reference = d.references()[0].clone();
            assert_eq!(original.state, SyntaxState::Provisional);
            let mut batch = d.append("(u\r\n").unwrap();
            for ch in format!("> {tail}").chars() {
                let update = d.append(ch.encode_utf8(&mut [0; 4])).unwrap();
                assert!(update.new_links.is_empty());
                batch.merge(update).unwrap();
                assert_eq!(d.references()[0], reference);
                if d.link_destination(LinkId(0)).is_none() {
                    assert_eq!(d.links()[0], target);
                    assert_eq!(d.link_info(LinkId(0)).unwrap().source, original.source);
                }
            }
            d.finish();
            assert_eq!(d.links().len(), 1);
            assert_eq!(d.link_destination(LinkId(0)), success.then_some("u"));
            assert_eq!(d.references()[0], reference);
            let info = d.link_info(LinkId(0)).unwrap();
            assert_eq!(info.label, original.label);
            assert_eq!(info.kind, original.kind);
            assert_eq!(info.state, SyntaxState::Sealed);
            if !success {
                assert_eq!(info.source, original.source);
            }
        }
    }
}

#[test]
fn container_deferred_work_only_visits_new_bytes_and_never_old_references_or_ancestors() {
    for table in [false, true] {
        let mut d = Document::new();
        d.append(&format!("{}\n\n> [a](u\n", "[r] ".repeat(20_000)))
            .unwrap();
        let references = d.references().to_vec();
        let old: Vec<_> = (0..20_000)
            .map(|i| *d.link_info(LinkId(i)).unwrap())
            .collect();
        d.append(&format!("> 't') {}", "x".repeat(1100))).unwrap();
        let before = d.work();
        for _ in 0..1000 {
            d.append("x").unwrap();
        }
        let after = d.work();
        assert_eq!(
            after.container_eligibility_bytes - before.container_eligibility_bytes,
            1000
        );
        assert_eq!(after.table_header_bytes - before.table_header_bytes, 1000);
        assert_eq!(after.inline_bytes, before.inline_bytes);
        assert_eq!(after.tree_visits, before.tree_visits);
        assert_eq!(after.container_steps, before.container_steps);
        assert_eq!(after.classified_bytes, before.classified_bytes);
        assert_eq!(d.references(), references);
        assert_eq!(d.link_destination(LinkId(20_000)), None);
        d.append(if table { " | b\n> -|-\n\n" } else { "\n\n" })
            .unwrap();
        assert_eq!(d.tables().len(), usize::from(table));
        assert_eq!(d.link_destination(LinkId(20_000)), (!table).then_some("u"));
        let before = d.work();
        d.append("[r]: /late\n").unwrap();
        assert!(d.work().inline_bytes - before.inline_bytes < 32);
        assert!(d.work().tree_visits - before.tree_visits < 1000);
        for (i, original) in old.iter().enumerate() {
            assert_eq!(d.link_info(LinkId(i)), Some(original));
        }
        d.finish();
        let w = d.work();
        assert!(w.container_eligibility_bytes <= w.input_bytes);
        assert!(w.container_eligibility_steps <= 24 * w.input_bytes);
        eprintln!("table={table} work={w:?}");
    }
    // A suffix can fail at its 4097th title byte, long before header release.
    for decision in ["", " | b\n> -|-\n"] {
        let mut d = Document::new();
        d.append(&format!("{}[a](u\n{}'", "> ".repeat(500), "> ".repeat(500)))
            .unwrap();
        d.append(&"x".repeat(1_000_000)).unwrap();
        let before = d.work();
        for _ in 0..1000 {
            d.append("x").unwrap();
        }
        let after = d.work();
        assert_eq!(after.inline_bytes, before.inline_bytes);
        assert_eq!(
            after.container_eligibility_bytes - before.container_eligibility_bytes,
            1000
        );
        assert_eq!(after.container_steps, before.container_steps);
        assert_eq!(after.tree_visits, before.tree_visits);
        assert_eq!(after.classified_bytes, before.classified_bytes);
        d.append(decision).unwrap();
        d.finish();
        assert_eq!(d.link_destination(LinkId(0)), None);
        let w = d.work();
        assert!(w.inline_bytes <= w.input_bytes);
        assert!(w.container_eligibility_bytes <= w.input_bytes);
        assert!(w.container_eligibility_steps <= 24 * w.input_bytes);
        eprintln!("million byte: {w:?}");
    }
}

#[test]
fn container_raw_metadata_and_source_display_slices_share_zero_one_and_six_byte_budgets() {
    for image in ["", "!"] {
        let source = format!(
            "> {image}[中&amp;\r\n> 🙂](\r\n>\t<a b%20中\\\\&amp;>\n> 't&#10;&amp;\\\''\r\n> )\n"
        );
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
fn container_gap_updates_merge_with_rebuild_and_open_folds_close_without_changing_identity() {
    for image in ["", "!"] {
        for table in [false, true] {
            let mut d = Document::new();
            d.append(&format!("> p {} {image}[a](u\r\n", "x".repeat(1100)))
                .unwrap();
            let first = BlockId(0);
            let original = *d.link_info(LinkId(0)).unwrap();
            let reference = d.references()[0].clone();
            let mut view = FoldView::new(&d, 1).unwrap();
            let mut merged = FoldView::new(&d, 1).unwrap();
            assert_eq!(d.source_fold_limit(), 0);
            assert!(SourceFoldRegistration::new(&d, &view, Span::new(0, 2)).is_err());
            let mut registration =
                SourceFoldRegistration::container_paragraph(&d, &view, first, 4).unwrap();
            let mut id = None;
            for _ in 0..40 {
                let mut b = Budget::new(WorkLimit {
                    queries: 1,
                    objects: 1,
                    ..WorkLimit::default()
                });
                id = registration.advance(&d, &mut view, &mut b).unwrap().id;
                assert_eq!(b.used().bytes, 0);
                if id.is_some() {
                    break;
                }
            }
            let id = id.unwrap();
            view.set_collapsed(&d, id, true).unwrap();
            let tile = d.tile_at(original.label.start).unwrap();
            let ticket = view.heights().ticket(tile).unwrap();
            let key = d.tile_key(tile).unwrap();
            let mut reader = d.display_cursor(original.label).unwrap();
            let header = format!(">\t't') {} | b\r\n", "x".repeat(1100));
            let mut batch = d.append(&header).unwrap();
            view.apply(&d, &batch).unwrap();
            assert_eq!(d.link_info(LinkId(0)), Some(&original));
            assert_eq!(d.link_destination(LinkId(0)), None);
            assert!(view.fold_candidate(&d, id).unwrap().unwrap().open);
            let update = d
                .append(if table {
                    "> -|-\r\n\n"
                } else {
                    "> wrong\r\n\n"
                })
                .unwrap();
            view.apply(&d, &update).unwrap();
            batch.merge(update).unwrap();
            merged.apply(&d, &batch).unwrap();
            let rebuilt = FoldView::new(&d, 1).unwrap();
            assert_eq!(
                merged.heights().total_rows(),
                rebuilt.heights().total_rows()
            );
            assert_eq!(d.tables().len(), usize::from(table));
            assert_eq!(d.link_destination(LinkId(0)), (!table).then_some("u"));
            assert_eq!(d.references()[0], reference);
            let info = d.link_info(LinkId(0)).unwrap();
            assert_eq!(info.label, original.label);
            assert_eq!(info.kind, original.kind);
            assert_eq!(info.state, SyntaxState::Sealed);
            if table {
                assert_eq!(info.source, original.source);
            }
            let candidate = view.fold_candidate(&d, id).unwrap().unwrap();
            assert!(!candidate.open);
            assert!(view.is_collapsed(id));
            assert!(candidate.source.end <= d.source_fold_limit());
            assert!(SourceFoldRegistration::container_paragraph(&d, &merged, first, 4).is_ok());
            assert_ne!(d.tile_key(tile).unwrap(), key);
            assert_eq!(view.measure(ticket, 9), Err(LayoutError::StaleMeasurement));
            let mut b = Budget::new(WorkLimit::unlimited());
            assert_eq!(reader.next(&d, &mut b), Err(LayoutError::StaleContinuation));
            assert_eq!(b.used(), WorkLimit::default());
        }
    }
}

#[test]
fn generated_container_components_keep_raw_spans_without_structural_or_virtual_metadata() {
    let gaps = ["", " \t ", "\n", "\r\n", " \r\n "];
    let mut cases = 0;
    for (first, next) in [
        ("> ", "> "),
        ("- ", "  "),
        ("> - > ", ">   > "),
        (">\t", ">\t"),
        ("> ", ""),
    ] {
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
                                let gap = |s: &str| s.replace('\n', &format!("\n{next}"));
                                let prefix = format!("{first}{image}[中&amp;🙂](");
                                let mut source = format!("{prefix}{}", gap(before));
                                let target_start =
                                    source.len() + usize::from(target.starts_with('<'));
                                source.push_str(target);
                                source.push_str(&gap(between));
                                let title_start = source.len() + 1;
                                source.push_str(title);
                                source.push_str(&gap(after));
                                source.push_str(")\n");
                                let d = parse(&source);
                                assert_eq!(
                                    d.links(),
                                    [LinkTarget::Direct(Span::new(
                                        target_start,
                                        target_start + raw.len()
                                    ))],
                                    "{source:?}"
                                );
                                assert_eq!(d.link_title(LinkId(0)), Some(raw_title), "{source:?}");
                                assert_eq!(d.link_destination(LinkId(0)), Some(raw));
                                let info = d.link_info(LinkId(0)).unwrap();
                                assert_eq!(
                                    info.title,
                                    Some(Span::new(title_start, title_start + raw_title.len()))
                                );
                                assert_eq!(info.source, Span::new(first.len(), source.len() - 1));
                                assert_eq!(
                                    info.label,
                                    Span::new(
                                        first.len() + image.len() + 1,
                                        first.len() + image.len() + 1 + "中&amp;🙂".len()
                                    )
                                );
                                assert_eq!(
                                    info.kind,
                                    if image.is_empty() {
                                        LinkKind::Link
                                    } else {
                                        LinkKind::Image
                                    }
                                );
                                assert_eq!(info.state, SyntaxState::Sealed);
                                let label: String = d
                                    .display_runs_in(info.source)
                                    .unwrap()
                                    .filter(|r| !r.style.contains(Style::HIDDEN))
                                    .map(|r| r.text)
                                    .collect();
                                assert_eq!(label, "中&🙂");
                                cases += 1;
                            }
                        }
                    }
                }
            }
        }
    }
    assert_eq!(cases, 18_000);
}

#[test]
fn deferred_container_source_releases_under_its_old_key_before_new_scope_classification() {
    for (first, next) in [
        ("> ", "> "),
        ("- ", "  "),
        ("> - > ", ">   > "),
        (">\t", ">\t"),
    ] {
        for tail in [
            "\nroot ",
            "- sibling ",
            "> > nested ",
            "> # heading ",
            "> ```\n> literal\n> ```\n",
            "> $$\n> literal\n> $$\n",
            "> [^f]: note ",
        ] {
            for completed in [false, true] {
                let source = format!(
                    "{first}[a](u\n{next}{}{} |b\n{tail}{}\n",
                    if completed { "'t') " } else { "'t'x) " },
                    "x".repeat(1100),
                    "x".repeat(1100)
                );
                let d = parse(&source);
                assert!(d.tables().is_empty());
                assert_eq!(d.link_destination(LinkId(0)), completed.then_some("u"));
                let mut bytes = Utf8Document::new();
                for chunk in source.as_bytes().chunks(17) {
                    bytes.append(chunk).unwrap();
                }
                bytes.finish();
                assert_eq!(support::snapshot(bytes.document()), support::snapshot(&d));
            }
        }
    }
}

#[test]
fn definition_owned_prefixes_and_deferred_virtual_groups_are_consumed_once() {
    for image in ["", "!"] {
        let source = format!(">\t{image}[r\n> bad](u\n>\t't')\n");
        let d = parse(&source);
        assert_eq!(d.link_destination(LinkId(0)), Some("u"));
        assert_eq!(d.link_title(LinkId(0)), Some("t"));
        let info = d.link_info(LinkId(0)).unwrap();
        assert_eq!(d.text(info.label), Some("r\n> bad"));
        assert_eq!(d.references()[0].label, "r bad");
        assert_eq!(d.references()[0].first_use, Some(2));
        assert_eq!(
            d.work().container_eligibility_bytes,
            (source.len() - 6 + 4) as u64
        );
        assert_eq!(
            d.runs_in(Span::new(0, source.len()))
                .unwrap()
                .map(|r| r.text)
                .collect::<String>(),
            source
        );
    }
}
