#[path = "support/code_replay_changes.rs"]
mod code_changes;
use mdstream::*;
#[path = "support/container_title_model.rs"]
mod model;
#[allow(dead_code)]
#[path = "support/shortcut_hashes.rs"]
mod shortcut_hashes;
#[allow(dead_code)]
#[path = "support/shortcut_text_hashes.rs"]
mod shortcut_text;
#[path = "support/snapshot.rs"]
mod support;

#[test]
fn independent_container_title_body_metadata_and_decoded_source_model() {
    for m in model::successes() {
        model::assert_model(&model::parse(&m.source), &m);
    }
}

#[test]
fn container_titles_every_utf8_split_and_prefix_eof() {
    for m in model::successes() {
        let expected = model::parse(&m.source);
        for split in 0..=m.source.len() {
            let mut d = Utf8Document::new();
            d.append(&m.source.as_bytes()[..split]).unwrap();
            d.append(&m.source.as_bytes()[split..]).unwrap();
            d.finish();
            model::assert_model(d.document(), &m);
            assert_eq!(
                support::snapshot(d.document()),
                support::snapshot(&expected)
            );
        }
        let mut stream = Utf8Document::new();
        for end in 0..=m.source.len() {
            if end > 0 {
                stream.append(&m.source.as_bytes()[end - 1..end]).unwrap();
            }
            let mut eof = Utf8Document::new();
            eof.append(&m.source.as_bytes()[..end]).unwrap();
            eof.finish();
            if m.source.is_char_boundary(end) {
                assert_eq!(
                    support::snapshot(eof.document()),
                    support::snapshot(&model::parse(&m.source[..end]))
                );
            }
        }
        stream.finish();
        model::assert_model(stream.document(), &m);
    }
}

#[path = "support/container_title_changes.rs"]
mod changes;
#[allow(dead_code)]
#[path = "support/container_eligibility_cases.rs"]
mod corpus;
#[path = "support/nested_changes.rs"]
mod nested_changes;
include!("fixtures/container_title_controls_changes.rs");

fn shown(d: &Document) -> String {
    d.display_runs_in(Span::new(0, d.source().len()))
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect()
}

fn control_change_model(d: &Document, source: &str) {
    assert_eq!(d.links().len(), 1, "{source:?}");
    // These exact allowlisted sources all start with this same literal label.
    assert!(source.starts_with("> [a](u 'a\n> "));
    let suffix = if let Some(close) = source.find("')") {
        let title = Span::new(9, close);
        let gaps: Vec<_> = source[..close]
            .match_indices("\n> ")
            .map(|(at, _)| Span::new(at + 1, at + 3))
            .collect();
        changes::assert_plain(
            d,
            LinkId(0),
            Span::new(3, 4),
            Span::new(2, close + 2),
            Span::new(6, 7),
            title,
            &gaps,
        );
        &source[close + 2..]
    } else {
        assert_eq!(
            d.link_info(LinkId(0)),
            Some(&LinkInfo {
                kind: LinkKind::Link,
                label: Span::new(3, 4),
                source: Span::new(2, 5),
                title: None,
                state: SyntaxState::Sealed,
            })
        );
        assert_eq!(d.links(), &[LinkTarget::Reference(SymbolId(0))]);
        assert_eq!(model::chunks(d, LinkId(0)), None);
        &source[5..]
    };
    let mut expected = if source.contains("')") { "a" } else { "[a]" }.to_owned();
    for (i, line) in suffix.split('\n').enumerate() {
        if i > 0 {
            expected.push('\n');
        }
        let payload = line
            .strip_prefix("> ")
            .unwrap_or(if line == ">" { "" } else { line });
        expected.push_str(if payload == "-" { "" } else { payload });
    }
    if !source.ends_with('\n') {
        expected.truncate(expected.trim_end_matches(' ').len());
    }
    assert_eq!(shown(d), expected, "{source:?}");
}

#[test]
fn real_predecessor_controls_and_all_eof_work_use_exact_source_allowlists() {
    let baseline: Vec<_> = include_str!("fixtures/container_title_controls_baseline.txt")
        .lines()
        .collect();
    let mut index = 0;
    for (case, source) in model::controls().iter().enumerate() {
        for end in corpus::ends(source) {
            for bytewise in [false, true] {
                let bytes = &source.as_bytes()[..end];
                let mut d = Utf8Document::new();
                for chunk in bytes.chunks(if bytewise { 1 } else { bytes.len().max(1) }) {
                    d.append(chunk).unwrap();
                }
                d.finish();
                let d = d.document();
                let original = shortcut_text::line(
                    "container_title_controls_baseline",
                    index,
                    bytes,
                    baseline[index],
                );
                let old: Vec<_> = original.split_whitespace().collect();
                assert_eq!(
                    (old[0], old[1], old[2]),
                    (
                        case.to_string().as_str(),
                        end.to_string().as_str(),
                        u8::from(bytewise).to_string().as_str()
                    )
                );
                let original = (
                    u64::from_str_radix(old[3], 16).unwrap(),
                    u64::from_str_radix(old[4], 16).unwrap(),
                );
                let expected = if let Some((source, hashes, public)) = CONTROL_TITLE_CHANGES
                    .iter()
                    .find(|entry| entry.0.as_bytes() == bytes)
                {
                    if *public {
                        control_change_model(d, source);
                    } else {
                        assert_eq!(
                            corpus::hash(&format!("{:?}", support::snapshot(d))),
                            original.0
                        );
                    }
                    hashes[usize::from(bytewise)]
                } else {
                    original
                };
                let expected = (
                    shortcut_hashes::source(d.source(), bytewise, expected.0),
                    nested_changes::work(d.source(), expected.1),
                );
                let expected = code_changes::pair(d.source(), expected);
                assert_eq!(
                    (
                        corpus::hash(&format!("{:?}", support::snapshot(d))),
                        corpus::hash(&corpus::old_work(d.work()))
                    ),
                    expected,
                    "case={case} end={end} bytewise={bytewise}"
                );
                index += 1;
            }
        }
    }
    assert_eq!(index, baseline.len());
}

#[test]
fn title_prefix_gap_steps_and_caps_count_real_source_only() {
    for m in model::successes() {
        let d = model::parse(&m.source);
        assert_eq!(
            d.work().link_title_gap_steps,
            m.gaps.len() as u64,
            "{:?}",
            m.source
        );
    }
    for ending in ["\n", "\r\n"] {
        for depth in [1, 16, 128] {
            let prefix = "> ".repeat(depth);
            for len in [4095, 4096, 4097] {
                let body = "x".repeat(len - 1 - ending.len());
                let source = format!("{prefix}[a](u 'a{ending}{prefix}{body}')\n");
                let d = model::parse(&source);
                assert_eq!(
                    d.link_destination(LinkId(0)),
                    (len <= 4096).then_some("u"),
                    "depth={depth} len={len}"
                );
            }
            for limit in [8255, 8256, 8257] {
                let spaces = limit - 8197;
                let source = format!(
                    "{prefix}[a]({}{} 'a{ending}{prefix}{}' )\n",
                    " ".repeat(spaces),
                    "d".repeat(4096),
                    "t".repeat(4096 - 1 - ending.len())
                );
                let d = model::parse(&source);
                assert_eq!(
                    d.link_destination(LinkId(0)).is_some(),
                    limit <= 8256,
                    "depth={depth} suffix={limit}"
                );
            }
        }
    }
    // The same large-prefix component input now has its complete real path.
    let prefix = "> ".repeat(1000);
    let d = model::parse(&format!(
        "{prefix}[a](\n{prefix}u\n{prefix}'t'\n{prefix})\n"
    ));
    assert_eq!(d.link_destination(LinkId(0)), Some("u"));
    let d = model::parse("> [a](\n>\tu\n> 'a\n> b')\n");
    assert_eq!(d.link_title(LinkId(0)), Some("a\n> b"));
    assert_eq!(d.work().link_title_gap_steps, 1);
}

#[test]
fn opaque_failure_and_eof_preserve_reference_and_rejecting_byte_once() {
    for image in ["", "!"] {
        for ending in ["\n", "\r\n"] {
            for (tail, visible) in [("'x)&amp; *after*\n", "'x)& after\n"), ("", "")] {
                let source = format!("> {image}[中](u 'a{ending}> &amp;\\* *b*{tail}");
                let d = model::parse(&source);
                assert_eq!(d.link_destination(LinkId(0)), None);
                assert_eq!(d.link_title(LinkId(0)), None);
                assert_eq!(model::chunks(&d, LinkId(0)), None);
                assert_eq!(
                    d.link_info(LinkId(0)),
                    Some(&LinkInfo {
                        kind: if image.is_empty() {
                            LinkKind::Link
                        } else {
                            LinkKind::Image
                        },
                        label: Span::new(3 + image.len(), 6 + image.len()),
                        source: Span::new(2, 7 + image.len()),
                        title: None,
                        state: SyntaxState::Sealed,
                    })
                );
                assert_eq!(
                    shown(&d),
                    format!("{image}[中](u 'a\n&amp;\\* *b*{visible}")
                );
                for split in 0..=source.len() {
                    let mut stream = Utf8Document::new();
                    stream.append(&source.as_bytes()[..split]).unwrap();
                    stream.append(&source.as_bytes()[split..]).unwrap();
                    stream.finish();
                    assert_eq!(support::snapshot(stream.document()), support::snapshot(&d));
                }
            }
        }
    }
}

#[test]
fn resolved_reference_title_gaps_remain_until_final_success_and_next_candidate_starts_empty() {
    for ending in ["\n", "\r\n"] {
        for tail in [
            format!("> b'){ending}"),
            format!("> b'x){ending}"),
            "> b".into(),
            format!(">{ending}> b'){ending}"),
            format!("> > b'){ending}"),
        ] {
            let mut d = Document::new();
            d.append(&format!(
                "> [r]: /old 'old{ending}> title'{ending}{ending}> [r](u 'a{ending}"
            ))
            .unwrap();
            let reference = d.references()[0].clone();
            let before = *d.link_info(LinkId(0)).unwrap();
            let old = model::chunks(&d, LinkId(0));
            assert_eq!(d.link_destination(LinkId(0)), Some("/old"));
            let success = tail == format!("> b'){ending}");
            let mut batch = None;
            for (i, ch) in tail.char_indices() {
                let u = d.append(ch.encode_utf8(&mut [0; 4])).unwrap();
                assert!(u.new_links.is_empty());
                if let Some(batch) = &mut batch {
                    Update::merge(batch, u).unwrap();
                } else {
                    batch = Some(u);
                }
                if !success || i < tail.find(')').unwrap() {
                    assert_eq!(d.link_info(LinkId(0)).unwrap().source, before.source);
                    assert_eq!(model::chunks(&d, LinkId(0)), old);
                }
                assert_eq!(d.references()[0], reference);
            }
            d.finish();
            assert_eq!(
                d.link_destination(LinkId(0)),
                Some(if success { "u" } else { "/old" })
            );
            assert_eq!(d.references()[0], reference);
            if !success {
                assert_eq!(model::chunks(&d, LinkId(0)), old);
            }
            let mut next = Document::new();
            next.append(&format!(
                "> [a](u 'a{ending}> b'x){ending}{ending}> [b](v 'one'){ending}"
            ))
            .unwrap();
            next.finish();
            assert_eq!(next.link_title(LinkId(1)), Some("one"));
            assert_eq!(
                model::chunks(&next, LinkId(1))
                    .unwrap()
                    .iter()
                    .map(|c| c.1.as_str())
                    .collect::<String>(),
                "one"
            );
        }
    }
}

#[test]
fn container_body_updates_merge_with_rebuild_and_open_folds_keep_identity() {
    for image in ["", "!"] {
        for table in [false, true] {
            let mut d = Document::new();
            d.append(&format!("> p {} {image}[a](u 'a\r\n", "x".repeat(1100)))
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
            let header = format!(">\t b') {} | b\r\n", "x".repeat(1100));
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
            assert_eq!(d.link_title(LinkId(0)), (!table).then_some("a\r\n>\t b"));
            if !table {
                assert_eq!(
                    model::chunks(&d, LinkId(0))
                        .unwrap()
                        .iter()
                        .map(|c| c.1.as_str())
                        .collect::<String>(),
                    "a\n b"
                );
            }
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
fn direct_gap_reader_pauses_inside_prefix_and_keeps_crlf_utf8_atomic() {
    for m in model::successes() {
        let d = model::parse(&m.source);
        let mut c = d.decoded_link_title(LinkId(0)).unwrap();
        let mut actual = Vec::new();
        let mut skipped = 0;
        for _ in 0..1000 {
            let at = c.source_position();
            for limit in [
                WorkLimit::default(),
                WorkLimit {
                    queries: 0,
                    ..WorkLimit::unlimited()
                },
                WorkLimit {
                    objects: 0,
                    ..WorkLimit::unlimited()
                },
                WorkLimit {
                    bytes: 0,
                    ..WorkLimit::unlimited()
                },
            ] {
                let mut b = Budget::new(limit);
                if at == m.info.title.unwrap().end {
                    assert_eq!(c.advance(&mut b), Ok(None));
                } else {
                    assert_eq!(c.advance(&mut b), Err(Completion::BudgetExhausted));
                }
                assert_eq!(b.used(), WorkLimit::default());
                assert_eq!(c.source_position(), at);
            }
            if m.gaps.iter().any(|gap| gap.start <= at && at < gap.end) {
                let mut b = Budget::new(WorkLimit {
                    bytes: 1,
                    queries: 1,
                    objects: 1,
                    ..WorkLimit::default()
                });
                if at + 1 == m.info.title.unwrap().end {
                    assert_eq!(c.advance(&mut b), Ok(None));
                } else {
                    assert_eq!(c.advance(&mut b), Err(Completion::BudgetExhausted));
                }
                assert_eq!(c.source_position(), at + 1);
                assert_eq!(b.used().bytes, 1);
                assert_eq!(b.used().queries, 1);
                assert_eq!(b.used().objects, 0);
                skipped += 1;
                continue;
            }
            if at < m.info.title.unwrap().end
                && (m.source[at..].starts_with("\r\n")
                    || m.source[at..].chars().next().unwrap().len_utf8() > 1)
            {
                let mut b = Budget::new(WorkLimit {
                    bytes: 1,
                    ..WorkLimit::unlimited()
                });
                assert_eq!(c.advance(&mut b), Err(Completion::BudgetExhausted));
                assert_eq!(b.used(), WorkLimit::default());
            }
            match c.advance(&mut Budget::new(WorkLimit {
                bytes: 6,
                queries: 1,
                objects: 1,
                ..WorkLimit::default()
            })) {
                Ok(Some(chunk)) => actual.push((chunk.source, chunk.text.to_owned())),
                Ok(None) => break,
                Err(Completion::BudgetExhausted) => (),
                Err(e) => panic!("{e:?}"),
            }
        }
        assert_eq!(actual, m.chunks);
        assert_eq!(skipped, m.gaps.iter().map(|g| g.len()).sum::<usize>());
        assert_eq!(c.source_position(), m.info.title.unwrap().end);
    }
    let d = model::parse("> [a](u '')\n");
    let mut c = d.decoded_link_title(LinkId(0)).unwrap();
    let mut b = Budget::new(WorkLimit::default());
    assert_eq!(c.advance(&mut b), Ok(None));
    assert_eq!(b.used(), WorkLimit::default());
}

#[test]
fn title_destination_code_info_and_display_share_zero_one_six_frames() {
    let d =
        model::parse("> [中](p&amp;中 'a\r\n>\t &nGt;🙂\\*\r\n>   ')\n\n~~~ t&#10;\nbody\n~~~\n");
    let mut title = d.decoded_link_title(LinkId(0)).unwrap();
    let mut target = d.decoded_link_destination(LinkId(0)).unwrap();
    let block = BlockId(
        d.blocks()
            .iter()
            .position(|b| matches!(b.kind, BlockKind::Code { .. }))
            .unwrap(),
    );
    let mut info = d.code_info(block).unwrap();
    let mut display = d.display_cursor(Span::new(0, d.source().len())).unwrap();
    let mut done = [false; 4];
    let mut texts = [String::new(), String::new(), String::new(), String::new()];
    for frame in 0..10_000 {
        let bytes = if frame % 3 == 0 {
            0
        } else if frame % 3 == 1 {
            1
        } else {
            6
        };
        let mut b = Budget::new(WorkLimit {
            bytes,
            queries: 1,
            objects: 1,
            ..WorkLimit::default()
        });
        for turn in 0..4 {
            let i = (frame + turn) % 4;
            if done[i] {
                continue;
            }
            if i == 3 {
                match display.next(&d, &mut b).unwrap() {
                    DisplayStep::Run(r) => {
                        if !r.style.contains(Style::HIDDEN) {
                            texts[i].push_str(r.text)
                        }
                    }
                    DisplayStep::Complete => done[i] = true,
                    DisplayStep::BudgetExhausted => (),
                }
            } else {
                let result = match i {
                    0 => title.advance(&mut b).map(|c| c.map(|c| c.text.to_owned())),
                    1 => target.advance(&mut b).map(|c| c.map(|c| c.text.to_owned())),
                    _ => info.advance(&mut b).map(|c| c.map(|c| c.text.to_owned())),
                };
                match result {
                    Ok(Some(c)) => texts[i].push_str(&c),
                    Ok(None) => done[i] = true,
                    Err(Completion::BudgetExhausted) => (),
                    Err(e) => panic!("{e:?}"),
                }
            }
        }
        assert!(b.used().bytes <= bytes && b.used().queries <= 1 && b.used().objects <= 1);
        if bytes == 0 {
            assert_eq!(b.used(), WorkLimit::default());
        }
        if done == [true; 4] {
            break;
        }
    }
    assert_eq!(done, [true; 4]);
    assert_eq!(
        texts,
        [
            "a\n ≫\u{20d2}🙂*\n  ".to_owned(),
            "p&中".to_owned(),
            "t\n".to_owned(),
            shown(&d)
        ]
    );
}

#[test]
fn thousand_title_lines_and_appends_after_old_definitions_and_million_body_are_linear() {
    let mut d = Document::new();
    let mut old = String::new();
    for i in 0..20_000 {
        old.push_str(&format!("[r{i}]: /u{i}\n"));
    }
    old.push('\n');
    old.push_str(&"x".repeat(1_000_000));
    d.append(&old).unwrap();
    let references = d.references().to_vec();
    let before = d.work();
    for _ in 0..1000 {
        d.append("x").unwrap();
    }
    let after = d.work();
    assert_eq!(after.inline_bytes - before.inline_bytes, 1000);
    assert_eq!(
        after.root_eligibility_bytes - before.root_eligibility_bytes,
        1000
    );
    assert_eq!(after.tree_visits, before.tree_visits);
    assert_eq!(after.classified_bytes, before.classified_bytes);
    assert_eq!(after.container_steps, before.container_steps);
    assert_eq!(after.link_title_gap_steps, before.link_title_gap_steps);
    d.append("\n\n> [a](u 'a\n").unwrap();
    let before = d.work();
    for _ in 0..1000 {
        d.append("> x\n").unwrap();
    }
    d.append("> ')\n").unwrap();
    d.finish();
    assert_eq!(&d.references()[..20_000], references);
    assert_eq!(d.link_destination(LinkId(0)), Some("u"));
    let text = model::chunks(&d, LinkId(0))
        .unwrap()
        .iter()
        .map(|c| c.1.as_str())
        .collect::<String>();
    assert_eq!(text, format!("a\n{}", "x\n".repeat(1000)));
    let after = d.work();
    assert_eq!(
        after.link_title_gap_steps - before.link_title_gap_steps,
        1001
    );
    assert!(after.inline_bytes - before.inline_bytes <= 3004);
    assert!(after.container_eligibility_bytes - before.container_eligibility_bytes <= 3004);
    assert_eq!(after.label_bytes, before.label_bytes);
    assert_eq!(after.label_fold_steps, before.label_fold_steps);
    assert!(after.container_bytes - before.container_bytes <= 12 * 4010);
    assert!(after.tree_visits - before.tree_visits <= 300_000);
}

#[test]
fn virtual_columns_never_spend_real_title_cap_and_backslash_resets_at_ending() {
    for prefix in [">\t", " >\t", "   >\t"] {
        for ending in ["\n", "\r\n"] {
            for len in [4095, 4096, 4097] {
                let source = format!(
                    "> [a](u 'a{ending}{prefix}{}')\n",
                    "x".repeat(len - 1 - ending.len())
                );
                let d = model::parse(&source);
                assert_eq!(
                    d.link_destination(LinkId(0)),
                    (len <= 4096).then_some("u"),
                    "{prefix:?} {ending:?} {len}"
                );
            }
            let source = format!("> [a](u 'a\\{ending}{prefix}')\n");
            let d = model::parse(&source);
            assert_eq!(
                model::chunks(&d, LinkId(0))
                    .unwrap()
                    .iter()
                    .map(|c| c.1.as_str())
                    .collect::<String>(),
                "a\\\n"
            );
        }
    }
}

include!("fixtures/container_title_work_changes.rs");

mod table_changes {
    include!("fixtures/container_title_tables_changes.rs");
    pub fn exact(source: &str, bytewise: bool) -> Option<(u64, u64)> {
        TABLE_TITLE_CHANGES
            .iter()
            .find(|entry| entry.0 == source)
            .map(|entry| entry.1[usize::from(bytewise)])
    }
}

#[test]
fn old_header_eligibility_at_window_and_late_pipe_keeps_real_table_shapes() {
    let baseline: Vec<_> = include_str!("fixtures/container_title_tables_baseline.txt")
        .lines()
        .collect();
    let mut index = 0;
    for source in model::table_cases() {
        for bytewise in [false, true] {
            let mut d = Utf8Document::new();
            for part in source
                .as_bytes()
                .chunks(if bytewise { 1 } else { source.len() })
            {
                d.append(part).unwrap();
            }
            d.finish();
            let d = d.document();
            let original = shortcut_text::line(
                "container_title_tables_baseline",
                index,
                source.as_bytes(),
                baseline[index],
            );
            let old: Vec<_> = original.split_whitespace().collect();
            let expected = if let Some(expected) = table_changes::exact(&source, bytewise) {
                let open = source.find("[a]").unwrap();
                let t = open + 7;
                changes::assert_plain(
                    d,
                    LinkId(0),
                    Span::new(open + 1, open + 2),
                    Span::new(open, open + 14),
                    Span::new(open + 4, open + 5),
                    Span::new(t, t + 5),
                    &[Span::new(t + 2, t + 4)],
                );
                let inside: String = d
                    .display_runs_in(Span::new(open, open + 14))
                    .unwrap()
                    .filter(|r| !r.style.contains(Style::HIDDEN))
                    .map(|r| r.text)
                    .collect();
                assert_eq!(inside, "a");
                expected
            } else {
                (
                    u64::from_str_radix(old[3], 16).unwrap(),
                    u64::from_str_radix(old[4], 16).unwrap(),
                )
            };
            assert_eq!(
                (
                    corpus::hash(&format!("{:?}", support::snapshot(d))),
                    corpus::hash(&corpus::old_work(d.work()))
                ),
                code_changes::pair(
                    d.source(),
                    (expected.0, nested_changes::work(d.source(), expected.1))
                ),
                "{index}"
            );
            let full_work = format!("{:?}", d.work())
                .split(", ")
                .filter(|field| {
                    !field.starts_with("link_title_gap_steps:")
                        && !field.starts_with("code_fallback_")
                        && !field.starts_with("fence_candidate_")
                        && !field.starts_with("structural_prefix_")
                        && !field.starts_with("link_fallback_")
                        && !field.starts_with("bare_")
                        && !field.starts_with("link_label_steps:")
                        && !field.starts_with("link_index_steps:")
                        && !field.starts_with("link_gate_steps:")
                })
                .collect::<Vec<_>>()
                .join(", ");
            let work_expected = if let Some((_, expected, public)) = TABLE_FULL_WORK_CHANGES
                .iter()
                .find(|entry| entry.0 == source)
            {
                if !public {
                    // Exact confirmed headers keep the old public snapshot;
                    // deferred finalization adds one observer step, no bytes.
                    assert_eq!(d.link_destination(LinkId(0)), None);
                    assert_eq!(d.link_title(LinkId(0)), None);
                    assert_eq!(model::chunks(d, LinkId(0)), None);
                    assert_eq!(
                        corpus::hash(&format!("{:?}", support::snapshot(d))),
                        u64::from_str_radix(old[3], 16).unwrap()
                    );
                }
                expected[usize::from(bytewise)]
            } else {
                u64::from_str_radix(old[6], 16).unwrap()
            };
            assert_eq!(
                corpus::hash(&full_work),
                code_changes::work(d.source(), nested_changes::work(d.source(), work_expected)),
                "all old Work {index}"
            );
            assert_eq!(
                format!("{:016x}", corpus::hash(&model::table_shape(d))),
                format!(
                    "{:016x}",
                    code_changes::snapshot(d.source(), u64::from_str_radix(old[5], 16).unwrap())
                ),
                "shape {index}"
            );
            index += 1;
        }
    }
    assert_eq!(index, baseline.len());
}

#[test]
fn depth1000_extends_the_exact_predecessor_shape_to_the_complete_quote_path() {
    let prefix = "> ".repeat(1000);
    let source = format!("{prefix}[a](u 'a\n{prefix}b')\n");
    let d = model::parse(&source);
    // Exact semantic delta from the frozen 512-quote predecessor shape.
    assert_eq!(
        format!("{:016x}", corpus::hash(&model::table_shape(&d))),
        structural_shape_delta(&source, "0a581438d4262f7d")
    );
    assert_eq!(d.container_count(), 1000);
    assert!(d.blocks().iter().all(
        |b| b.kind == BlockKind::Quote { depth: 1000 } && b.container == Some(ContainerId(999))
    ));
    changes::assert_plain(
        &d,
        LinkId(0),
        Span::new(2001, 2002),
        Span::new(2000, 4012),
        Span::new(2004, 2005),
        Span::new(2007, 4010),
        &[Span::new(2009, 4009)],
    );
    assert_eq!(
        d.link_title(LinkId(0)),
        Some(format!("a\n{prefix}b").as_str())
    );
    assert_eq!(
        model::chunks(&d, LinkId(0))
            .unwrap()
            .iter()
            .map(|c| c.1.as_str())
            .collect::<String>(),
        "a\nb"
    );
    assert_eq!(d.work().link_title_gap_steps, 1);
}

fn structural_shape_delta(source: &str, old: &str) -> &'static str {
    assert_eq!(old, "0a581438d4262f7d");
    let prefix = "> ".repeat(1000);
    assert_eq!(source, format!("{prefix}[a](u 'a\n{prefix}b')\n"));
    "cedd14f277b1c416"
}
