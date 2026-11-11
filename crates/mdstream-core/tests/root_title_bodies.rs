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
        .filter(|run| !run.style.contains(Style::HIDDEN))
        .map(|run| run.text)
        .collect()
}

fn decoded(doc: &Document, id: LinkId) -> String {
    let mut reader = doc.decoded_link_title(id).unwrap();
    let mut text = String::new();
    while let Some(chunk) = reader
        .advance(&mut Budget::new(WorkLimit::unlimited()))
        .unwrap()
    {
        text.push_str(chunk.text);
    }
    text
}

#[test]
fn independent_root_body_model_delimiters_targets_components_and_raw_indentation() {
    let mut cases = 0;
    for image in ["", "!"] {
        for ending in ["\n", "\r\n"] {
            for (target, raw_target) in [
                ("u", "u"),
                ("a(b)c", "a(b)c"),
                ("a\\)b", "a\\)b"),
                ("<a b>", "a b"),
                ("<>", ""),
            ] {
                for (open, close, escaped) in [
                    ('\'', '\'', "\\'"),
                    ('"', '"', "\\\""),
                    ('(', ')', "\\(\\)"),
                ] {
                    for before in ["", ending] {
                        for between in [" ", ending] {
                            for after in ["", ending] {
                                for body in [
                                    format!("中🙂&amp;{ending}  tail{escaped}{ending}   end"),
                                    format!("{ending}   "),
                                    format!("a\\{ending}b"),
                                ] {
                                    let prefix = format!("{image}[中&amp;](");
                                    let source = format!(
                                        "{prefix}{before}{target}{between}{open}{body}{close}{after})\n"
                                    );
                                    let doc = parse(&source);
                                    let id = LinkId(0);
                                    assert_eq!(doc.links().len(), 1, "{source:?}");
                                    assert_eq!(
                                        doc.link_destination(id),
                                        Some(raw_target),
                                        "{source:?}"
                                    );
                                    assert_eq!(
                                        doc.link_title(id),
                                        Some(body.as_str()),
                                        "{source:?}"
                                    );
                                    let info = doc.link_info(id).unwrap();
                                    let start = prefix.len()
                                        + before.len()
                                        + target.len()
                                        + between.len()
                                        + 1;
                                    assert_eq!(
                                        info.title,
                                        Some(Span::new(start, start + body.len()))
                                    );
                                    assert_eq!(
                                        doc.text(info.title.unwrap()).unwrap().as_ptr(),
                                        doc.link_title(id).unwrap().as_ptr()
                                    );
                                    assert_eq!(info.source, Span::new(0, source.len() - 1));
                                    assert_eq!(info.state, SyntaxState::Sealed);
                                    assert_eq!(
                                        info.kind,
                                        if image.is_empty() {
                                            LinkKind::Link
                                        } else {
                                            LinkKind::Image
                                        }
                                    );
                                    let expected = body
                                        .replace("\r\n", "\n")
                                        .replace("&amp;", "&")
                                        .replace(escaped, &escaped.replace('\\', ""));
                                    assert_eq!(decoded(&doc, id), expected, "{source:?}");
                                    assert_eq!(visible(&doc), "中&\n");
                                    cases += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    assert_eq!(cases, 1440);
}

#[test]
fn blank_real_blocks_target_newlines_and_lone_cr_do_not_complete_a_title() {
    for image in ["", "!"] {
        for ending in ["\n", "\r\n"] {
            for body in [
                format!("first{ending}{ending}last"),
                format!("first{ending}  {ending}last"),
                format!("first{ending}# head{ending}last"),
                format!("first{ending}    code{ending}last"),
                format!("first{ending}> quote{ending}last"),
                format!("first{ending}- list{ending}last"),
                format!("first{ending}~~~{ending}last"),
                format!("first{ending}---{ending}last"),
                format!("first{ending}h|h{ending}-|-{ending}last"),
                "first\rlast".into(),
            ] {
                let source = format!("{image}[a](u '{body}')\n");
                let doc = parse(&source);
                assert_eq!(doc.link_destination(LinkId(0)), None, "{source:?}");
                assert_eq!(
                    doc.link_info(LinkId(0)).unwrap().source,
                    Span::new(0, image.len() + 3)
                );
            }
            for suffix in [
                format!("(<a{ending}b> 'x{ending}y')"),
                format!("(a({ending}b) 'x{ending}y')"),
                format!("(u (a{ending}(b)))"),
            ] {
                assert_eq!(
                    parse(&format!("{image}[a]{suffix}\n")).link_destination(LinkId(0)),
                    None
                );
            }
            // The entity newline is data and consumes no component allowance.
            let doc = parse(&format!("{image}[a](u{ending}'a&#10;{ending}b'{ending})\n"));
            assert_eq!(decoded(&doc, LinkId(0)), "a\n\nb");
            let doc = parse(&format!("{image}[a](u 'a{ending}b')\n---\n"));
            assert!(matches!(doc.blocks()[0].kind, BlockKind::Heading { .. }));
            assert_eq!(
                doc.link_title(LinkId(0)),
                Some(format!("a{ending}b").as_str())
            );
        }
    }
}

#[test]
fn new_body_opaque_failure_and_eof_have_explicit_raw_and_rejecting_byte_models() {
    for image in ["", "!"] {
        for ending in ["\n", "\r\n"] {
            for tail in ["", "'&amp;)\n"] {
                let body = format!("first{ending}中🙂&amp;\\*`x`*y*&#10;");
                let source = format!("{image}[a](u '{body}{tail}");
                let doc = parse(&source);
                let expected_tail = if tail.is_empty() { "" } else { "'&)\n" };
                assert_eq!(
                    visible(&doc),
                    format!("a(u 'first\n中🙂&amp;\\*`x`*y*&#10;{expected_tail}"),
                    "{source:?}"
                );
                assert_eq!(doc.links(), [LinkTarget::Reference(SymbolId(0))]);
                assert_eq!(
                    doc.link_info(LinkId(0)),
                    Some(&LinkInfo {
                        kind: if image.is_empty() {
                            LinkKind::Link
                        } else {
                            LinkKind::Image
                        },
                        label: Span::new(image.len() + 1, image.len() + 2),
                        source: Span::new(0, image.len() + 3),
                        title: None,
                        state: SyntaxState::Sealed,
                    })
                );
                assert_eq!(
                    doc.references(),
                    [Reference {
                        label: "a".into(),
                        destination: None,
                        title: None,
                        revision: 0,
                        first_use: Some(0)
                    }]
                );
                let start = source.find('中').unwrap();
                let span = Span::new(start, source.len() - tail.len());
                for run in doc.display_runs_in(span).unwrap() {
                    assert_eq!(run.replacement, None);
                    assert_eq!(run.text, doc.text(run.source).unwrap());
                    for style in [Style::HIDDEN, Style::CODE, Style::EMPHASIS] {
                        assert!(!run.style.contains(style));
                    }
                }
            }
        }
    }
}

#[test]
fn utf8_every_byte_split_and_every_prefix_eof_including_crlf_and_failed_body() {
    for source in [
        "![中🙂](\r\n<a b>\n'中&amp;\\'\r\n  🙂\n   '\r\n)\n",
        "[a](u (a\\(\nb\\)))\n",
        "[a](u 'first\n&amp;\\*`x`'&amp;)\n",
        "[a](u 'first\r\n&amp;\\*",
        "[a](u 'first\r",
        "[a](u 'first\n\nlast')\n",
        "[a](u 'first\n# head\nlast')\n",
        "[a](u 'first\nlast')|b\n-|-\n",
        "*old [a](u 'first\nend*and | b\n-|-\n",
    ] {
        let expected = support::snapshot(&parse(source));
        let mut bytewise = Utf8Document::new();
        for byte in source.bytes() {
            bytewise.append(&[byte]).unwrap();
        }
        bytewise.finish();
        assert_eq!(support::snapshot(bytewise.document()), expected);
        for split in 0..=source.len() {
            let mut stream = Utf8Document::new();
            stream.append(&source.as_bytes()[..split]).unwrap();
            stream.append(&source.as_bytes()[split..]).unwrap();
            stream.finish();
            assert_eq!(
                support::snapshot(stream.document()),
                expected,
                "split {split}: {source:?}"
            );
            let prefix = &source.as_bytes()[..split];
            let mut stream = Utf8Document::new();
            for byte in prefix {
                stream.append(&[*byte]).unwrap();
            }
            stream.finish();
            assert_eq!(
                support::snapshot(stream.document()),
                support::snapshot(&parse(&String::from_utf8_lossy(prefix))),
                "EOF {split}: {source:?}"
            );
        }
    }
}

struct ScopeCase {
    source: String,
    boundary: usize,
    complete: bool,
}

fn scope_cases() -> Vec<ScopeCase> {
    let mut cases = Vec::new();
    for ending in ["\n", "\r\n"] {
        for image in ["", "!"] {
            for tail in ["", "last')\n", "last'x)\n"] {
                let prior = format!("{image}[a](u 'first{ending}{}", tail.replace('\n', ending));
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
                        for decision in ["", "|[tail](v)\n{c}-|-\n", "|[tail](v)\n{c}wrong\n"] {
                            let body = "**中🙂** [z](v) ![i](q) &amp; \\* `c` ";
                            let row = format!(
                                "{marker}{body}{}",
                                "x".repeat(width - marker.len() - body.len())
                            );
                            let end = decision.replace("{c}", continuation).replace('\n', ending);
                            cases.push(ScopeCase {
                                source: format!("{prior}{row}{end}"),
                                boundary: prior.len(),
                                complete: tail == "last')\n",
                            });
                        }
                    }
                }
            }
        }
    }
    cases
}

fn scope_snapshot(doc: &Document, boundary: usize) -> support::Snapshot {
    let mut snapshot = support::snapshot(doc);
    snapshot.blocks.retain(|b| b.source.start >= boundary);
    snapshot.links.remove(0);
    snapshot.link_info.remove(0);
    snapshot.runs.retain(|r| r.0.start >= boundary);
    snapshot.display.retain(|r| r.0.start >= boundary);
    snapshot.depths.retain(|r| r.0.start >= boundary);
    snapshot
}

fn digest(text: &str) -> u64 {
    text.bytes().fold(0xcbf29ce484222325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    })
}

#[test]
fn body_scope_matches_real_prechange_container_maps_ids_and_table_structure() {
    let cases = scope_cases();
    let baseline: Vec<_> = include_str!("fixtures/root_title_scope_baseline.txt")
        .lines()
        .collect();
    assert_eq!(cases.len(), 864);
    assert_eq!(cases.len(), baseline.len());
    for (index, (case, expected)) in cases.iter().zip(baseline).enumerate() {
        let doc = parse(&case.source);
        assert_eq!(
            format!(
                "{:016x}",
                digest(&format!("{:?}", scope_snapshot(&doc, case.boundary)))
            ),
            expected,
            "case {index}: {:?}",
            case.source
        );
        assert_eq!(
            doc.link_destination(LinkId(0)),
            case.complete.then_some("u")
        );
        assert_eq!(
            doc.link_title(LinkId(0)),
            case.complete.then_some(if case.source.contains('\r') {
                "first\r\nlast"
            } else {
                "first\nlast"
            })
        );
        let mut stream = Utf8Document::new();
        for bytes in case.source.as_bytes().chunks(7) {
            stream.append(bytes).unwrap();
        }
        stream.finish();
        assert_eq!(
            support::snapshot(stream.document()),
            support::snapshot(&doc)
        );
    }
}

#[test]
fn body_pending_fold_updates_tickets_fixed_and_open_registration() {
    let mut doc = Document::new();
    doc.append(&format!("p {} [a](u 'first\n", "x".repeat(1100)))
        .unwrap();
    assert_eq!(doc.source_fold_limit(), 0);
    let mut view = FoldView::new(&doc, 1).unwrap();
    let mut merged = FoldView::new(&doc, 1).unwrap();
    assert!(SourceFoldRegistration::paragraph(&doc, &view, BlockId(0), 2).is_err());
    assert!(SourceFoldRegistration::new(&doc, &view, Span::new(0, 2)).is_err());
    let mut task = SourceFoldRegistration::open_paragraph(&doc, &view, BlockId(0), 2).unwrap();
    let mut id = None;
    for _ in 0..30 {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            ..WorkLimit::default()
        });
        let progress = task.advance(&doc, &mut view, &mut budget).unwrap();
        assert_eq!(budget.used().bytes, 0);
        if progress.id.is_some() {
            id = progress.id;
            break;
        }
    }
    let id = id.unwrap();
    view.set_collapsed(&doc, id, true).unwrap();
    let label_tile = doc
        .tile_at(doc.link_info(LinkId(0)).unwrap().label.start)
        .unwrap();
    let ticket = view.heights().ticket(label_tile).unwrap();
    let old_key = doc.tile_key(label_tile).unwrap();
    let mut reader = doc
        .display_cursor(Span::new(0, doc.source().len()))
        .unwrap();
    let mut batch = doc.append("last'\n").unwrap();
    view.apply(&doc, &batch).unwrap();
    assert!(view.fold_candidate(&doc, id).unwrap().unwrap().open);
    assert_eq!(doc.source_fold_limit(), 0);
    assert_eq!(doc.link_destination(LinkId(0)), None);
    let update = doc.append(")\n\n").unwrap();
    assert!(update.new_links.is_empty());
    view.apply(&doc, &update).unwrap();
    batch.merge(update).unwrap();
    merged.apply(&doc, &batch).unwrap();
    let rebuilt = FoldView::new(&doc, 1).unwrap();
    assert_eq!(
        merged.heights().total_rows(),
        rebuilt.heights().total_rows()
    );
    let candidate = view.fold_candidate(&doc, id).unwrap().unwrap();
    assert!(!candidate.open);
    assert!(view.is_collapsed(id));
    assert_eq!(candidate.source, doc.blocks()[0].source);
    assert!(doc.source_fold_limit() >= candidate.source.end);
    assert!(SourceFoldRegistration::paragraph(&doc, &merged, BlockId(0), 2).is_ok());
    assert_eq!(doc.link_destination(LinkId(0)), Some("u"));
    assert_eq!(doc.link_title(LinkId(0)), Some("first\nlast"));
    assert_ne!(doc.tile_key(label_tile).unwrap(), old_key);
    assert_eq!(view.measure(ticket, 9), Err(LayoutError::StaleMeasurement));
    let mut budget = Budget::new(WorkLimit::unlimited());
    assert_eq!(
        reader.next(&doc, &mut budget),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(budget.used(), WorkLimit::default());
    assert!(matches!(
        view.hidden_at(&doc, 1104, &mut budget).unwrap(),
        FoldLookup::Hidden(_)
    ));
    // Root prose can subsequently be promoted using the original structure
    // pipeline. The direct object keeps its source identity inside Setext.
    let heading = parse("[a](u 'first\nlast')\n---\n");
    assert!(matches!(
        heading.blocks()[0].kind,
        BlockKind::Heading { .. }
    ));
    assert_eq!(heading.link_destination(LinkId(0)), Some("u"));
    let table = parse("[a](u 'first\nlast')|b\n-|-\n");
    assert_eq!(table.tables().len(), 1);
    assert_eq!(table.link_destination(LinkId(0)), None);
}

#[test]
fn body_thousand_appends_and_late_definition_do_not_visit_old_objects() {
    let mut doc = Document::new();
    doc.append(&format!("{}\n\n", "[r] ".repeat(20_000)))
        .unwrap();
    let old_links = doc.links().to_vec();
    let old_infos: Vec<_> = (0..20_000)
        .map(|i| *doc.link_info(LinkId(i)).unwrap())
        .collect();
    let old_blocks = doc.blocks().to_vec();
    doc.append(&format!(
        "p {} ![new](u 'first\n{}",
        "x".repeat(1100),
        "t".repeat(1100)
    ))
    .unwrap();
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("t").unwrap();
    }
    let after = doc.work();
    assert_eq!(after.input_bytes - before.input_bytes, 1000);
    // The current physical line can still belong to a table. Only the old
    // grammar observer consumes it until the no-pipe line ending releases it.
    assert_eq!(after.inline_bytes, before.inline_bytes);
    assert_eq!(
        after.root_eligibility_bytes - before.root_eligibility_bytes,
        1000
    );
    assert_eq!(after.table_header_bytes - before.table_header_bytes, 1000);
    assert_eq!(after.label_bytes, before.label_bytes);
    assert_eq!(after.tree_visits, before.tree_visits);
    let update = doc.append("')\n\n").unwrap();
    assert!(update.new_links.is_empty());
    assert_eq!(doc.link_destination(LinkId(20_000)), Some("u"));
    assert_eq!(doc.link_title(LinkId(20_000)).unwrap().len(), 2106);
    let before = doc.work();
    let update = doc.append("[r]: /late 'title'\n").unwrap();
    assert_eq!(update.references_changed.len(), 1);
    assert!(doc.work().inline_bytes - before.inline_bytes < 64);
    assert!(doc.work().tree_visits - before.tree_visits < 1000);
    assert_eq!(&doc.links()[..20_000], old_links);
    assert_eq!(&doc.blocks()[..old_blocks.len()], old_blocks);
    for (i, info) in old_infos.iter().enumerate() {
        assert_eq!(doc.link_info(LinkId(i)), Some(info));
        assert_eq!(doc.link_destination(LinkId(i)), Some("/late"));
        assert_eq!(doc.link_title(LinkId(i)), Some("title"));
    }
    eprintln!("old-object work before={before:?} after={:?}", doc.work());
}

#[test]
fn body_limit_failure_does_not_replay_into_a_million_byte_tail() {
    for (body, closed) in [(" ", false), ("t", false), ("t", true)] {
        let mut doc = Document::new();
        let source = if body == " " {
            "[a](u 'first\n"
        } else {
            "[a](u 'first\r\n"
        };
        doc.append(source).unwrap();
        // Exceed both title and suffix limits. Once rejected, no old prefix
        // enters the scanner again, regardless of later plain appends or EOF.
        for _ in 0..10 {
            doc.append(&body.repeat(1000)).unwrap();
        }
        if closed {
            doc.append("')\n").unwrap();
        }
        let before_tail = doc.work();
        for _ in 0..1000 {
            doc.append(&"x".repeat(1000)).unwrap();
        }
        let after = doc.work();
        assert_eq!(after.input_bytes - before_tail.input_bytes, 1_000_000);
        assert!(after.inline_bytes - before_tail.inline_bytes <= 1_002_048);
        assert_eq!(after.label_bytes, before_tail.label_bytes);
        assert_eq!(after.entity_bytes, before_tail.entity_bytes);
        assert!(after.tree_visits - before_tail.tree_visits < 2000);
        doc.finish();
        let final_work = doc.work();
        assert!(final_work.inline_bytes <= final_work.input_bytes);
        assert!(final_work.classified_bytes <= 3 * final_work.input_bytes);
        assert!(final_work.root_eligibility_bytes <= final_work.input_bytes);
        assert!(final_work.root_eligibility_steps <= 24 * final_work.input_bytes);
        assert!(final_work.table_header_bytes <= final_work.input_bytes);
        assert_eq!(doc.links().len(), 1);
        assert_eq!(doc.link_destination(LinkId(0)), None);
        eprintln!("long suffix {body:?}/{closed}: {final_work:?}");
    }
}

#[test]
fn body_title_source_mapping_crlf_atomicity_and_shared_metadata_display_budgets() {
    for image in ["", "!"] {
        let source = format!(
            "{image}[中&amp;](<u&amp;> '中🙂&nGt;\\*\r\n  &#10;&amp;\n   ')\n\n~~~ &amp;\\*\nbody\n~~~\n"
        );
        let doc = parse(&source);
        let info = doc.link_info(LinkId(0)).unwrap();
        let raw = info.title.unwrap();
        let before = support::snapshot(&doc);
        // Independent exact mapping, including raw indentation on closing line.
        let mut cursor = doc.decoded_link_title(LinkId(0)).unwrap();
        let mut at = raw.start;
        for (spelling, text) in [
            ("中", "中"),
            ("🙂", "🙂"),
            ("&nGt;", "≫\u{20d2}"),
            ("\\*", "*"),
            ("\r\n", "\n"),
            (" ", " "),
            (" ", " "),
            ("&#10;", "\n"),
            ("&amp;", "&"),
            ("\n", "\n"),
            (" ", " "),
            (" ", " "),
            (" ", " "),
        ] {
            if spelling == "\r\n" || spelling == "🙂" {
                let mut tiny = Budget::new(WorkLimit {
                    bytes: 1,
                    ..WorkLimit::unlimited()
                });
                assert_eq!(cursor.advance(&mut tiny), Err(Completion::BudgetExhausted));
                assert_eq!(cursor.source_position(), at);
                assert_eq!(tiny.used(), WorkLimit::default());
            }
            let chunk = cursor
                .advance(&mut Budget::new(WorkLimit::unlimited()))
                .unwrap()
                .unwrap();
            assert_eq!(chunk.source, Span::new(at, at + spelling.len()));
            assert_eq!(chunk.text, text);
            assert_eq!(doc.text(chunk.source), Some(spelling));
            at += spelling.len();
        }
        assert_eq!(at, raw.end);
        assert_eq!(
            cursor.advance(&mut Budget::new(WorkLimit::default())),
            Ok(None)
        );
        let block = doc
            .blocks()
            .iter()
            .position(|b| matches!(b.kind, BlockKind::Code { .. }))
            .unwrap();
        let mut code = doc.code_info(BlockId(block)).unwrap();
        let mut title = doc.decoded_link_title(LinkId(0)).unwrap();
        let mut destination = doc.decoded_link_destination(LinkId(0)).unwrap();
        let mut display = doc.display_cursor(info.label).unwrap();
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
        ] {
            let mut budget = Budget::new(limit);
            let positions = [
                title.source_position(),
                destination.source_position(),
                code.source_position(),
            ];
            assert_eq!(title.advance(&mut budget), Err(Completion::BudgetExhausted));
            assert_eq!(
                destination.advance(&mut budget),
                Err(Completion::BudgetExhausted)
            );
            assert_eq!(code.advance(&mut budget), Err(Completion::BudgetExhausted));
            assert_eq!(
                display.next(&doc, &mut budget).unwrap(),
                DisplayStep::BudgetExhausted
            );
            assert_eq!(
                positions,
                [
                    title.source_position(),
                    destination.source_position(),
                    code.source_position()
                ]
            );
            assert_eq!(budget.used(), WorkLimit::default());
        }
        let mut output = [String::new(), String::new(), String::new(), String::new()];
        let mut done = [false; 4];
        for frame in 0..1000 {
            let bytes = if frame % 2 == 0 { 1 } else { 6 };
            let mut budget = Budget::new(WorkLimit {
                bytes,
                queries: 1,
                objects: 1,
                ..WorkLimit::default()
            });
            for i in (0..4).map(|i| (i + frame) % 4) {
                if done[i] {
                    continue;
                }
                match i {
                    0 => match title.advance(&mut budget) {
                        Ok(Some(c)) => output[i].push_str(c.text),
                        Ok(None) => done[i] = true,
                        Err(Completion::BudgetExhausted) => {}
                        other => panic!("{other:?}"),
                    },
                    1 => match destination.advance(&mut budget) {
                        Ok(Some(c)) => output[i].push_str(c.text),
                        Ok(None) => done[i] = true,
                        Err(Completion::BudgetExhausted) => {}
                        other => panic!("{other:?}"),
                    },
                    2 => match code.advance(&mut budget) {
                        Ok(Some(c)) => output[i].push_str(c.text),
                        Ok(None) => done[i] = true,
                        Err(Completion::BudgetExhausted) => {}
                        other => panic!("{other:?}"),
                    },
                    _ => match display.next(&doc, &mut budget).unwrap() {
                        DisplayStep::Run(r) => {
                            if !r.style.contains(Style::HIDDEN) {
                                output[i].push_str(r.text);
                            }
                        }
                        DisplayStep::Complete => done[i] = true,
                        DisplayStep::BudgetExhausted => {}
                    },
                }
            }
            assert!(budget.used().bytes <= bytes);
            assert!(budget.used().queries <= 1 && budget.used().objects <= 1);
            if done == [true; 4] {
                break;
            }
        }
        assert_eq!(done, [true; 4]);
        assert_eq!(output, ["中🙂≫\u{20d2}*\n  \n&\n   ", "u&", "&*", "中&"]);
        for (at, ch) in source.char_indices() {
            let span = Span::new(at, at + ch.len_utf8());
            assert_eq!(
                doc.runs_in(span)
                    .unwrap()
                    .map(|r| r.text)
                    .collect::<String>(),
                ch.to_string()
            );
        }
        assert_eq!(support::snapshot(&doc), before);
    }
}

#[test]
fn body_raw_limits_include_crlf_and_indent_without_a_definition_line_window() {
    for ending in ["\n", "\r\n"] {
        for image in ["", "!"] {
            for label in [999, 1000] {
                for length in [MAX_LINK_TITLE_BYTES, MAX_LINK_TITLE_BYTES + 1] {
                    let raw = format!("{}{ending}   ", "t".repeat(length - ending.len() - 3));
                    let doc = parse(&format!("{image}[{}](u '{raw}')\n", "a".repeat(label)));
                    assert_eq!(
                        doc.link_title(LinkId(0)),
                        (length == MAX_LINK_TITLE_BYTES).then_some(raw.as_str())
                    );
                    if length == MAX_LINK_TITLE_BYTES {
                        assert_eq!(doc.link_info(LinkId(0)).unwrap().label.len(), label);
                    }
                }
            }
            // All 4096 raw bytes accepted even when the first line exceeds the
            // definition classification window. Suffix includes final ')'.
            for extra in [0, 1] {
                let raw = format!(
                    "{}{ending}x",
                    "t".repeat(MAX_LINK_TITLE_BYTES - ending.len() - 1)
                );
                let spaces = MAX_LINK_SUFFIX_BYTES - MAX_DESTINATION_BYTES - raw.len() - 4 + extra;
                let doc = parse(&format!(
                    "{image}[a]({} '{raw}'{})\n",
                    "u".repeat(MAX_DESTINATION_BYTES),
                    " ".repeat(spaces)
                ));
                assert_eq!(
                    doc.link_title(LinkId(0)),
                    (extra == 0).then_some(raw.as_str())
                );
            }
        }
    }
}

#[test]
fn body_scope_lazy_partial_tabs_real_blocks_and_new_root_keep_original_routing() {
    for ending in ["\n", "\r\n"] {
        for prior_tail in ["", "last')\n", "last'x)\n"] {
            let prior = format!("[a](u 'first{ending}{}", prior_tail.replace('\n', ending));
            for tail in [
                format!(
                    "> [b](v 'x{ending}> y'){ending}lazy {}{ending}",
                    "x".repeat(1100)
                ),
                format!(
                    "- [b](v 'x{ending}  y'){ending}  {}{ending}",
                    "x".repeat(1100)
                ),
                format!(
                    "> - [b](v 'x{ending}>   y'){ending}>   {}{ending}",
                    "x".repeat(1100)
                ),
                format!(
                    ">\t[b](v 'x{ending}>\ty'){ending}>\t{}{ending}",
                    "x".repeat(1100)
                ),
            ] {
                let doc = parse(&format!("{prior}{tail}"));
                assert_eq!(doc.link_destination(LinkId(1)), None);
                assert_eq!(doc.link_title(LinkId(1)), None);
                assert!(doc.container_count() > 0);
            }
            for interruption in [
                ending.to_owned(),
                format!("> quote{ending}{ending}"),
                format!("- item{ending}{ending}"),
                format!("# head{ending}"),
                format!("~~~{ending}raw{ending}~~~{ending}"),
                format!("    code{ending}"),
            ] {
                let prefix = format!("{prior}{interruption}");
                let source = format!("{prefix}![b](v 'x{ending}y') {}", "x".repeat(1100));
                let doc = parse(&source);
                assert_eq!(doc.links().len(), 2);
                assert_eq!(doc.link_destination(LinkId(1)), Some("v"));
                assert_eq!(
                    doc.link_title(LinkId(1)),
                    Some(format!("x{ending}y").as_str())
                );
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
fn body_upgrade_is_atomic_keeps_id_reference_first_use_and_update_chain() {
    for image in ["", "!"] {
        for suffix in [
            "(u 'first\nlast'\n)",
            "(u 'first\nlast'x)",
            "(u 'first\nlast",
            "(u 'first\r",
            "(u 'first\n\nlast')",
        ] {
            let mut doc = Document::new();
            let mut batch = doc
                .append(&format!("p {} {image}[old]", "x".repeat(1100)))
                .unwrap();
            let id = LinkId(0);
            let original = *doc.link_info(id).unwrap();
            let LinkTarget::Reference(symbol) = doc.links()[0] else {
                panic!("expected shortcut")
            };
            let reference = doc.references()[symbol.0].clone();
            let block = BlockId(0);
            assert_eq!(original.state, SyntaxState::Provisional);
            let complete = suffix == "(u 'first\nlast'\n)";
            for ch in suffix.chars() {
                let update = doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
                assert!(update.new_links.is_empty());
                batch.merge(update).unwrap();
                if doc.link_info(id).unwrap().source == original.source {
                    assert_eq!(doc.links()[0], LinkTarget::Reference(symbol));
                    assert_eq!(doc.link_title(id), None);
                }
            }
            batch.merge(doc.finish()).unwrap();
            let info = doc.link_info(id).unwrap();
            assert_eq!(info.label, original.label);
            assert_eq!(info.kind, original.kind);
            assert_eq!(info.state, SyntaxState::Sealed);
            assert_eq!(doc.block(block).unwrap().source.start, 0);
            assert_eq!(doc.block(block).unwrap().container, None);
            assert_eq!(doc.references()[symbol.0], reference);
            assert_eq!(doc.links().len(), 1);
            assert_eq!(batch.new_links, 0..1);
            assert_eq!(doc.link_destination(id), complete.then_some("u"));
            assert_eq!(doc.link_title(id), complete.then_some("first\nlast"));
            if !complete {
                assert_eq!(info.source, original.source);
            }
        }
    }
}

#[test]
fn body_candidates_preserve_resolved_reference_title_and_first_definition() {
    for image in ["", "!"] {
        for ending in ["\n", "\r\n"] {
            for success in [false, true] {
                let mut doc = Document::new();
                doc.append(&format!(
                    "[r]: /ref 'old{ending}title'{ending}{ending}p {} {image}[r]",
                    "x".repeat(1100)
                ))
                .unwrap();
                let id = LinkId(0);
                let original = *doc.link_info(id).unwrap();
                let target = doc.links()[0];
                let reference = doc.references()[0].clone();
                assert_eq!(
                    doc.link_title(id),
                    Some(format!("old{ending}title").as_str())
                );
                let mut batch = doc.append(&format!("(u 'new{ending}")).unwrap();
                assert_eq!(doc.links()[0], target);
                assert_eq!(doc.link_info(id).unwrap().source, original.source);
                assert_eq!(doc.references()[0], reference);
                let update = doc
                    .append(if success { "body')\n\n" } else { "body'x)\n\n" })
                    .unwrap();
                assert!(update.new_links.is_empty());
                assert_eq!(batch.revision, update.base_revision);
                batch.merge(update).unwrap();
                assert_eq!(doc.links().len(), 1);
                assert_eq!(doc.link_info(id).unwrap().label, original.label);
                assert_eq!(
                    doc.link_destination(id),
                    Some(if success { "u" } else { "/ref" })
                );
                assert_eq!(
                    doc.link_title(id),
                    Some(
                        format!(
                            "{}{ending}{}",
                            if success { "new" } else { "old" },
                            if success { "body" } else { "title" }
                        )
                        .as_str()
                    )
                );
                assert_eq!(doc.references()[0], reference);
                doc.append("[r]: /ignored 'wrong'\n\n[r]\n").unwrap();
                doc.finish();
                assert_eq!(doc.link_destination(LinkId(1)), Some("/ref"));
                assert_eq!(decoded(&doc, LinkId(1)), "old\ntitle");
                assert_eq!(doc.references()[0], reference);
            }
        }
    }
}

#[test]
fn body_scope_long_utf8_all_splits_and_prefix_eof() {
    let cases = scope_cases();
    for index in [2, 95, 288, 574, 611, 863] {
        let source = &cases[index].source;
        let expected = support::snapshot(&parse(source));
        for split in 0..=source.len() {
            let mut stream = Utf8Document::new();
            stream.append(&source.as_bytes()[..split]).unwrap();
            stream.append(&source.as_bytes()[split..]).unwrap();
            stream.finish();
            assert_eq!(
                support::snapshot(stream.document()),
                expected,
                "{index}/{split}"
            );
            let mut prefix = Utf8Document::new();
            for chunk in source.as_bytes()[..split].chunks(7) {
                prefix.append(chunk).unwrap();
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
}

fn table_sources() -> Vec<String> {
    let mut sources = Vec::new();
    for ending in ["\n", "\r\n"] {
        for image in ["", "!"] {
            for prefix in ["", "*prior ", "`old ", "$old ", "[old "] {
                for decision in ["-|-", "wrong"] {
                    for row in ["end*and", "last')", "&amp; `cell` [z](v)"] {
                        sources.push(format!("{prefix}{image}[a](u 'first{ending}{row} {}|b{ending}{decision}{ending}", "x".repeat(1100)));
                    }
                }
            }
        }
    }
    sources
}

#[test]
fn body_old_eligibility_first_pipe_eager_opaque_and_table_decisions_match_head() {
    let baseline: Vec<_> = include_str!("fixtures/root_title_table_baseline.txt")
        .lines()
        .collect();
    let sources = table_sources();
    assert_eq!(sources.len(), 120);
    assert_eq!(sources.len(), baseline.len());
    for (index, (source, expected)) in sources.iter().zip(baseline).enumerate() {
        let doc = parse(source);
        let (tables, hash) = expected.split_once(' ').unwrap();
        assert_eq!(
            doc.tables().len().to_string(),
            tables,
            "table decision {index}"
        );
        match index % 30 {
            4 | 5 | 6 | 9 | 10 | 11 | 28 | 29 => {
                // Precisely these complete sources change: 12 successful body
                // upgrades and 20 opaque failures. No generic LF/table exemption.
                let success = matches!(index % 30, 4 | 10 | 28);
                let start = source.find("[a]").unwrap() - usize::from(source.contains("![a]"));
                let label = source.find("[a]").unwrap() + 1;
                let title_start = source.find("'first").unwrap() + 1;
                let title_end = source.find("last')").map(|pos| pos + 4);
                let expected_visible = if success {
                    format!("{}a{}", &source[..start], &source[title_end.unwrap() + 2..])
                } else {
                    format!("{}a{}", &source[..start], &source[label + 2..])
                }
                .replace("\r\n", "\n");
                assert_eq!(visible(&doc), expected_visible, "changed display {index}");
                assert_eq!(doc.links().len(), 1);
                assert_eq!(doc.link_destination(LinkId(0)), success.then_some("u"));
                assert_eq!(
                    doc.link_title(LinkId(0)),
                    title_end
                        .filter(|_| success)
                        .map(|end| &source[title_start..end])
                );
                assert_eq!(
                    doc.link_info(LinkId(0)),
                    Some(&LinkInfo {
                        kind: if source.contains("![a]") {
                            LinkKind::Image
                        } else {
                            LinkKind::Link
                        },
                        label: Span::new(label, label + 1),
                        source: Span::new(
                            start,
                            if success {
                                title_end.unwrap() + 2
                            } else {
                                label + 2
                            }
                        ),
                        title: title_end
                            .filter(|_| success)
                            .map(|end| Span::new(title_start, end)),
                        state: SyntaxState::Sealed,
                    })
                );
                assert_eq!(
                    doc.references(),
                    [Reference {
                        label: "a".into(),
                        destination: None,
                        title: None,
                        revision: 0,
                        first_use: Some(start)
                    }]
                );
            }
            _ => assert_eq!(
                format!("{:016x}", digest(&format!("{:?}", support::snapshot(&doc)))),
                hash,
                "unchanged {index}: {source:?}"
            ),
        }
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

#[test]
fn two_thousand_body_lines_keep_streaming_limits_and_linear_observer_work() {
    let body = "x\n".repeat(2000);
    let source = format!("[a](u '{body}')\n");
    let mut doc = Document::new();
    for chunk in source.as_bytes().chunks(3) {
        doc.append(std::str::from_utf8(chunk).unwrap()).unwrap();
    }
    doc.finish();
    assert_eq!(doc.link_title(LinkId(0)), Some(body.as_str()));
    assert_eq!(decoded(&doc, LinkId(0)), body);
    let work = doc.work();
    assert!(work.inline_bytes <= work.input_bytes);
    assert!(work.root_eligibility_bytes <= work.input_bytes);
    assert!(work.root_eligibility_steps <= 24 * work.input_bytes);
    assert!(work.classified_bytes <= 3 * work.input_bytes);
    eprintln!("2000 title lines: {work:?}");
}

#[test]
fn body_failure_and_eof_raw_source_display_obey_zero_one_six_and_query_object_limits() {
    for ending in ["\n", "\r\n"] {
        for image in ["", "!"] {
            for tail in ["", "'&amp;)\n"] {
                let source = format!("{image}[a](u 'first{ending}中🙂&amp;\\*`x`{tail}");
                let doc = parse(&source);
                let mut display = doc.display_cursor(Span::new(0, source.len())).unwrap();
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
                ] {
                    let mut budget = Budget::new(limit);
                    assert_eq!(
                        display.next(&doc, &mut budget).unwrap(),
                        DisplayStep::BudgetExhausted
                    );
                    assert_eq!(budget.used(), WorkLimit::default());
                }
                let mut shown = String::new();
                let mut done = false;
                for frame in 0..2000 {
                    let bytes = if frame % 2 == 0 { 1 } else { 6 };
                    let mut budget = Budget::new(WorkLimit {
                        bytes,
                        queries: 1,
                        objects: 1,
                        ..WorkLimit::default()
                    });
                    match display.next(&doc, &mut budget).unwrap() {
                        DisplayStep::Run(r) => {
                            assert!(doc.text(r.source).is_some());
                            if !r.style.contains(Style::HIDDEN) {
                                shown.push_str(r.text);
                            }
                        }
                        DisplayStep::Complete => {
                            done = true;
                            break;
                        }
                        DisplayStep::BudgetExhausted => {}
                    }
                    assert!(budget.used().bytes <= bytes);
                    assert!(budget.used().queries <= 1 && budget.used().objects <= 1);
                }
                assert!(done);
                assert_eq!(
                    shown,
                    format!(
                        "a(u 'first\n中🙂&amp;\\*`x`{}",
                        if tail.is_empty() { "" } else { "'&)\n" }
                    )
                );
                assert!(doc.decoded_link_title(LinkId(0)).is_none());
                assert!(doc.decoded_link_destination(LinkId(0)).is_none());
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
}
