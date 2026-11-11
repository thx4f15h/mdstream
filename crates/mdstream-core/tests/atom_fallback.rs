use mdstream::*;
#[path = "support/atom_fallback_model.rs"]
mod model;
#[path = "support/snapshot.rs"]
mod support;

macro_rules! red_target {
    ($name:ident, $index:expr) => {
        #[test]
        fn $name() {
            let (source, expected) = model::TARGETS[$index];
            assert_eq!(model::shown(&model::parse(source)), expected);
        }
    };
}
red_target!(eof_entity, 0);
red_target!(invalid_entity, 1);
red_target!(eof_escape, 2);
red_target!(body_eof, 3);
red_target!(body_invalid, 4);
red_target!(quote_eof, 5);
red_target!(partial_tab, 6);
red_target!(reject_normal, 7);
red_target!(reject_bracket, 8);
red_target!(numeric, 9);
red_target!(independent_cell, 10);

#[test]
fn non_targets_match_actual_predecessor() {
    let baseline = include_str!("fixtures/atom_fallback_baseline.txt");
    let cases = model::controls();
    assert_eq!(baseline.lines().count(), cases.len());
    for (source, expected) in cases.iter().zip(baseline.lines()) {
        let actual = model::hash(&format!("{:?}", support::snapshot(&model::parse(source))));
        assert_eq!(format!("{actual:016x}"), expected, "{source:?}");
    }
}

#[test]
fn independent_atoms_all_roles_delimiters_and_physical_endings() {
    for (prefix, continuation) in [("", ""), ("> ", "> "), ("- ", "  ")] {
        for image in ["", "!"] {
            for ending in ["\n", "\r\n"] {
                for (open, close) in [("'", "'"), ("\"", "\""), ("(", ")")] {
                    let body = "中🙂&amp;&#x1F642;&nGt;\\*\\[\\!\\_\\~\\`\\$\\&amp;&unknown;";
                    let expected = "中🙂&🙂≫⃒*[!_~`$&amp;&unknown;";
                    for tail in ["".to_owned(), format!("{close}x)&amp;\\*")] {
                        let source = format!(
                            "{prefix}{image}[a](u {open}x{ending}{continuation}{body}{tail}"
                        );
                        let d = model::parse(&source);
                        let after = if tail.is_empty() {
                            String::new()
                        } else {
                            format!("{close}x)&*")
                        };
                        assert_eq!(model::shown(&d), format!("a(u {open}x\n{expected}{after}"));
                        assert_eq!(d.links(), &[LinkTarget::Reference(SymbolId(0))]);
                        let info = d.link_info(LinkId(0)).unwrap();
                        assert_eq!(info.state, SyntaxState::Sealed);
                        assert_eq!(info.title, None);
                        assert_eq!(
                            info.source,
                            Span::new(prefix.len(), prefix.len() + image.len() + 3)
                        );
                        assert_eq!(d.references()[0].first_use, Some(prefix.len()));
                        assert_eq!(d.references()[0].revision, 0);
                        for (raw, text) in [("&amp;", "&"), ("&#x1F642;", "🙂"), ("&nGt;", "≫⃒")]
                        {
                            let at = source.find(raw).unwrap();
                            let span = Span::new(at, at + raw.len());
                            let runs: Vec<_> = d.display_runs_in(span).unwrap().collect();
                            assert_eq!(runs.iter().map(|r| r.text).collect::<String>(), text);
                            assert!(runs.iter().all(|r| r.replacement == Some(span)));
                            // A scalar-sized source query retains the full replacement.
                            let run = d
                                .display_runs_in(Span::new(at + 1, at + 2))
                                .unwrap()
                                .next()
                                .unwrap();
                            assert_eq!(run.replacement, Some(span));
                            assert_eq!(run.text, ""); // replacement text only at its start
                        }
                        assert_eq!(
                            d.runs_in(Span::new(0, source.len()))
                                .unwrap()
                                .map(|r| r.text)
                                .collect::<String>(),
                            source
                        );
                    }
                }
            }
        }
    }
    for source in ["# [a](&amp; x)", "| [a](&amp; x) | b |\n| - | - |\n"] {
        let d = model::parse(source);
        assert!(model::shown(&d).starts_with("a(& x)"));
        assert_eq!(d.link_destination(LinkId(0)), None);
    }
}

#[test]
fn whole_candidate_validation_precedes_any_publication() {
    for bad in [
        "<",
        "[",
        "]",
        "!",
        "*",
        "_",
        "~",
        "`",
        "$",
        "&half",
        "\\",
        "\\\nnext",
        "\\\r\nnext",
    ] {
        let source = format!("[a](u '&amp;\\*{bad}");
        let d = model::parse(&source);
        assert_eq!(
            model::shown(&d),
            format!("a(u '&amp;\\*{bad}").replace("\r\n", "\n"),
            "{source:?}"
        );
    }
    // Neither half atom can use the current rejection byte to close itself.
    for (source, shown) in [
        ("[a](u '&amp';&amp;", "a(u '&amp';&"),
        ("[a](u '&amp;\\'x)", "a(u '&'x)"),
        ("[a](u '&amp;'\\*", "a(u '&'*"),
        ("[a](&bogus; x)", "a(&bogus; x)"),
        ("[a](&bad-\\* x)", "a(&bad-* x)"),
        ("[a](\\&amp; x)", "a(&amp; x)"),
    ] {
        assert_eq!(model::shown(&model::parse(source)), shown, "{source:?}");
    }
}

#[test]
fn every_utf8_split_prefix_eof_empty_append_and_finish() {
    let mut cases: Vec<_> = model::TARGETS.iter().map(|(s, _)| s.to_string()).collect();
    cases.extend([
        "> [中🙂](u 'x\r\n>\t&amp;\\*'x)&amp; [b](v)".into(),
        "[中](u '&amp;`x`'x)".into(),
        "[中](&amp;&half".into(),
        "[中](u '&amp;\\\r\nnext'x)".into(),
    ]);
    for source in cases {
        let full = support::snapshot(&model::parse(&source));
        for split in 0..=source.len() {
            let mut d = Utf8Document::new();
            d.append(&source.as_bytes()[..split]).unwrap();
            d.append(b"").unwrap();
            d.append(&source.as_bytes()[split..]).unwrap();
            d.finish();
            d.finish();
            assert_eq!(
                support::snapshot(d.document()),
                full,
                "{source:?} split={split}"
            );
            let expected = model::parse(&String::from_utf8_lossy(&source.as_bytes()[..split]));
            let mut eof = Utf8Document::new();
            for byte in &source.as_bytes()[..split] {
                eof.append(&[*byte]).unwrap();
            }
            eof.finish();
            assert_eq!(
                support::snapshot(eof.document()),
                support::snapshot(&expected)
            );
        }
    }
}

#[test]
fn shared_display_budgets_source_slices_and_replacements() {
    for (source, expected) in model::TARGETS {
        let d = model::parse(source);
        let span = Span::new(0, source.len());
        for bytes in [1, 6] {
            let mut c = d.display_cursor(span).unwrap();
            if bytes == 1 {
                assert_eq!(
                    c.next(
                        &d,
                        &mut Budget::new(WorkLimit {
                            bytes: 1,
                            ..WorkLimit::unlimited()
                        })
                    )
                    .unwrap(),
                    DisplayStep::BudgetExhausted
                );
                assert_eq!(c.source_position(), 0);
                continue;
            }
            let mut shown = String::new();
            for _ in 0..source.len() * 20 + 20 {
                let at = c.source_position();
                let mut zero = Budget::new(WorkLimit::default());
                let step = c.next(&d, &mut zero).unwrap();
                if step == DisplayStep::Complete {
                    break;
                }
                assert_eq!(step, DisplayStep::BudgetExhausted);
                assert_eq!(c.source_position(), at);
                // Query and object limits are shared with the source byte budget.
                for limit in [
                    WorkLimit {
                        bytes: 6,
                        ..Default::default()
                    },
                    WorkLimit {
                        queries: 1,
                        bytes: 6,
                        ..Default::default()
                    },
                ] {
                    assert_eq!(
                        c.next(&d, &mut Budget::new(limit)).unwrap(),
                        DisplayStep::BudgetExhausted
                    );
                    assert_eq!(c.source_position(), at);
                }
                let mut budget = Budget::new(WorkLimit {
                    queries: 1,
                    objects: 1,
                    bytes,
                    ..WorkLimit::unlimited()
                });
                if let DisplayStep::Run(run) = c.next(&d, &mut budget).unwrap()
                    && !run.style.contains(Style::HIDDEN)
                {
                    shown.push_str(run.text);
                }
                assert!(budget.used().bytes <= bytes);
            }
            assert_eq!(c.source_position(), source.len());
            assert_eq!(shown, *expected);
            assert_eq!(
                c.next(&d, &mut Budget::new(WorkLimit::default())).unwrap(),
                DisplayStep::Complete
            );
        }
    }
}

#[test]
fn caps_do_not_close_half_atoms_and_recorded_data_is_bounded() {
    for count in [8255, 8256, 8257] {
        let source = format!("[a]({}&amp;", " ".repeat(count - 5));
        let d = model::parse(&source);
        let expected = format!(
            "a({}{}",
            " ".repeat(count - 5),
            if count <= 8256 { "&" } else { "&amp;" }
        );
        assert_eq!(model::shown(&d), expected, "count={count}");
        assert!(d.work().link_fallback_bytes <= 2 * 8256);
    }
    for entity in [
        format!("&{};", "a".repeat(31)),
        format!("&{};", "a".repeat(32)),
        "&bogus;".into(),
    ] {
        let d = model::parse(&format!("[a]({entity} x)"));
        assert_eq!(model::shown(&d), format!("a({entity} x)"));
    }
    for scalar in ["中", "🙂"] {
        let source = format!("[a](u '{}&amp;", scalar.repeat(4096 / scalar.len()));
        let d = model::parse(&source);
        assert_eq!(
            d.runs_in(Span::new(0, source.len()))
                .unwrap()
                .map(|r| r.text)
                .collect::<String>(),
            source
        );
        assert!(
            d.display_runs_in(Span::new(0, source.len()))
                .unwrap()
                .all(|r| d.text(r.source).is_some())
        );
    }
}

#[test]
fn real_table_confirmation_cancellation_first_pipe_and_eager_guard() {
    for prefix in ["", "> "] {
        for decision in ["-|-", "wrong"] {
            let source = format!("{prefix}[a](u 'x\n{prefix}&amp;' | b\n{prefix}{decision}\n");
            let d = model::parse(&source);
            assert_eq!(
                d.tables().len(),
                usize::from(decision == "-|-"),
                "{source:?}"
            );
            if decision == "-|-" {
                // Deferred first-cell confirmation finishes the old paragraph,
                // then ordinary independent-cell entity handling owns the row.
                assert_eq!(model::shown(&d), "a(u 'x\n&'b\n");
            } else {
                assert_eq!(model::shown(&d), "a(u 'x\n&' | b\nwrong\n");
            }
            let mut stream = Utf8Document::new();
            for b in source.as_bytes() {
                stream.append(&[*b]).unwrap();
            }
            stream.finish();
            assert_eq!(support::snapshot(stream.document()), support::snapshot(&d));
        }
        for row in [
            "[old\nend](u 't')",
            "[old\nend](u '&amp;')",
            "`old\nend`",
            "$old\nend$",
        ] {
            // Their complete predecessor snapshots are in the frozen corpus.
            // The inherited label's syntax guard cancels before '&' publishes.
            let source = format!(
                "{prefix}{} | b\n{prefix}-|-\n",
                row.replace('\n', &format!("\n{prefix}"))
            );
            let d = model::parse(&source);
            let mut stream = Utf8Document::new();
            for b in source.as_bytes() {
                stream.append(&[*b]).unwrap();
            }
            stream.finish();
            assert_eq!(support::snapshot(stream.document()), support::snapshot(&d));
        }
    }
    // The pipe terminates the independent first cell. Its following syntax
    // never belongs to the failed destination trace or creates duplicate entries.
    let d = model::parse("[a](u '&amp;'x) | &amp; [b](v)\n-|-\n");
    assert_eq!(d.tables().len(), 1);
    assert_eq!(model::shown(&d), "a(u '&'x)& b\n");
    assert_eq!(d.links().len(), 2);
    assert_eq!(d.link_destination(LinkId(1)), Some("v"));
}

#[test]
fn fallback_updates_fold_rebuild_tickets_and_stale_reader() {
    let mut d = Document::new();
    d.append(&format!("p {} [a](u '&amp;", "x".repeat(1100)))
        .unwrap();
    let mut view = FoldView::new(&d, 1).unwrap();
    let mut merged = FoldView::new(&d, 1).unwrap();
    let mut registration =
        SourceFoldRegistration::open_paragraph(&d, &view, BlockId(0), 2).unwrap();
    let mut id = None;
    for _ in 0..30 {
        let progress = registration
            .advance(&d, &mut view, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap();
        if progress.id.is_some() {
            id = progress.id;
            break;
        }
    }
    let id = id.unwrap();
    view.set_collapsed(&d, id, true).unwrap();
    let tile = d.tile_at(d.source().len() - 1).unwrap();
    let ticket = view.heights().ticket(tile).unwrap();
    let mut reader = d.display_cursor(Span::new(0, d.source().len())).unwrap();
    let mut batch = None;
    for part in ["'", "x)", " &amp;\n", "\n"] {
        let update = d.append(part).unwrap();
        view.apply(&d, &update).unwrap();
        if let Some(batch) = &mut batch {
            Update::merge(batch, update).unwrap();
        } else {
            batch = Some(update);
        }
    }
    merged.apply(&d, &batch.unwrap()).unwrap();
    let rebuilt = FoldView::new(&d, 1).unwrap();
    assert_eq!(
        merged.heights().total_rows(),
        rebuilt.heights().total_rows()
    );
    assert!(!view.fold_candidate(&d, id).unwrap().unwrap().open);
    assert!(SourceFoldRegistration::paragraph(&d, &merged, BlockId(0), 2).is_ok());
    assert!(SourceFoldRegistration::new(&d, &merged, d.blocks()[0].source).is_ok());
    assert!(matches!(
        reader.next(&d, &mut Budget::new(WorkLimit::unlimited())),
        Err(LayoutError::StaleContinuation)
    ));
    assert!(view.measure(ticket, 9).is_err());
    assert!(model::shown(&d).ends_with("a(u '&'x) &\n\n"));
    assert_eq!(d.links().len(), 1);
    assert_eq!(d.link_info(LinkId(0)).unwrap().state, SyntaxState::Sealed);
}

#[test]
fn source_prefix_envelopes_and_overlapping_candidates_have_linear_visits() {
    let prefix = "> ".repeat(100);
    let source = format!(
        "{prefix}[a](u 'x\n{}{prefix}&amp;",
        format!("{prefix}b\n").repeat(2000)
    );
    let d = model::parse(&source);
    assert!(source.len() > 400_000);
    assert_eq!(model::shown(&d), format!("a(u 'x\n{}&", "b\n".repeat(2000)));
    let work = d.work();
    assert_eq!(work.inline_bytes, 4014);
    assert_eq!(work.link_fallback_bytes, 2 * (4014 - 4));
    assert_eq!(work.link_fallback_steps, 4010 + 2 * 2002);
    for depth in [1, 8, 64, 512, 1000] {
        let source = format!("{}&amp;", "[a](".repeat(depth));
        let d = model::parse(&source);
        assert_eq!(d.links().len(), 1);
        assert_eq!(d.work().inline_bytes, source.len() as u64);
        if depth > 1 {
            assert_eq!(d.work().link_fallback_bytes, 1); // first '[' disqualifies
            assert_eq!(
                model::shown(&d),
                format!("a({}&amp;", "[a](".repeat(depth - 1))
            );
        }
        assert!(d.work().link_fallback_bytes <= 2 * d.work().inline_bytes);
        assert!(d.work().link_fallback_steps <= 3 * d.work().inline_bytes);
    }
}

#[test]
fn old_history_does_not_add_fallback_work_and_ids_remain_stable() {
    let mut d = Document::new();
    d.append(&format!("{}\n\n", "x".repeat(1_000_000))).unwrap();
    for i in 0..20_000 {
        d.append(&format!("[r{i}]: /v\n")).unwrap();
    }
    d.append("\n[a](&amp; x)\n\n").unwrap();
    let id = LinkId(0);
    let info = *d.link_info(id).unwrap();
    let first = d.references().last().unwrap().first_use;
    let before = d.work();
    d.append("[a]: /old 'T'\n[a]: /new\n\n").unwrap();
    assert_eq!(d.link_info(id), Some(&info));
    assert_eq!(d.references().last().unwrap().first_use, first);
    assert_eq!(d.references().last().unwrap().revision, 1);
    assert_eq!(d.link_destination(id), Some("/old"));
    assert_eq!(d.work().link_fallback_bytes, before.link_fallback_bytes);
    let before = d.work();
    d.append("[b](&amp; x)\n\n").unwrap();
    assert_eq!(
        d.work().link_fallback_bytes - before.link_fallback_bytes,
        12
    );
    assert_eq!(d.work().link_fallback_steps - before.link_fallback_steps, 8);
    assert!(d.work().inline_bytes - before.inline_bytes < 20);
    d.append("[c](u 'C') [d](&amp; x)\n").unwrap();
    d.finish();
    assert_eq!(d.link_destination(LinkId(2)), Some("u"));
    assert_eq!(d.link_title(LinkId(2)), Some("C"));
    assert_eq!(d.link_destination(LinkId(3)), None);
}
