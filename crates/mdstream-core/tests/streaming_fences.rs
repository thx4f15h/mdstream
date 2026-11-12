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
        .filter(|run| !run.style.contains(Style::HIDDEN))
        .map(|run| run.text)
        .collect()
}

#[test]
fn unbounded_runs_and_complete_info_use_physical_line_model() {
    for marker in ['`', '~'] {
        for length in [3, 1023, 1024, 4096, 1_000_000] {
            let fence = marker.to_string().repeat(length);
            for ending in ["\n", "\r\n", ""] {
                let info = format!("rs {} &amp;", "中".repeat(400));
                let source = format!("{fence} {info}\r\n*x*\n{fence}{ending}");
                let doc = parse(&source);
                assert_eq!(visible(&doc), "*x*\n", "{marker} x {length}");
                let BlockKind::Code { info: Some(span) } = doc.blocks()[0].kind else {
                    panic!("expected code at length {length}")
                };
                assert_eq!(doc.text(span), Some(info.as_str()));
                assert_eq!(doc.work().inline_bytes, 0);
            }
        }
    }
}

#[test]
fn late_backtick_rejects_once_and_exposes_following_inline_immediately() {
    let mut doc = Document::new();
    let raw = format!("```{}", "x".repeat(4096));
    doc.append(&raw).unwrap();
    assert_eq!(visible(&doc), raw);
    assert!(doc.blocks().is_empty());
    assert_eq!(doc.work().inline_bytes, 0);
    doc.append("``` **visible** ").unwrap();
    assert!(
        doc.blocks()
            .iter()
            .all(|b| !matches!(b.kind, BlockKind::Code { .. }))
    );
    assert!(
        doc.runs_in(Span::new(raw.len(), doc.source().len()))
            .unwrap()
            .any(|r| r.text == "visible" && r.style.contains(Style::STRONG))
    );
    assert_eq!(doc.work().inline_bytes, doc.source().len() as u64);
    let before = doc.work();
    for _ in 0..2048 {
        doc.append("x").unwrap();
    }
    assert_eq!(doc.work().classified_bytes, before.classified_bytes);
    assert_eq!(doc.work().inline_bytes - before.inline_bytes, 2048);
}

// Independent physical-line model: no Document calls, bounded classifier,
// display output, or production helper contributes to these expected values.
fn model_close(line: &str, marker: u8, minimum: usize, column: usize, virtuals: usize) -> bool {
    let bytes = line
        .trim_end_matches('\n')
        .trim_end_matches('\r')
        .as_bytes();
    let mut at = 0;
    let mut col = column;
    let mut indent = virtuals;
    while let Some(&byte @ (b' ' | b'\t')) = bytes.get(at) {
        let width = if byte == b'\t' { 4 - col % 4 } else { 1 };
        indent += width;
        col += width;
        at += 1;
    }
    if indent > 3 {
        return false;
    }
    let run = bytes[at..].iter().take_while(|&&b| b == marker).count();
    run >= minimum && bytes[at + run..].iter().all(u8::is_ascii_whitespace)
}

#[test]
fn independent_closer_model_covers_lengths_suffixes_scopes_and_tab_columns() {
    let mut cases = 0;
    for marker in ['`', '~'] {
        for minimum in [3, 1023, 1024, 4096] {
            for (before, prefix, column, virtuals, seed) in [
                ("", "", 0, 0, ""),
                ("", "> ", 2, 0, ""),
                ("", ">\t", 4, 2, ""),
                ("- seed\n", "\t", 4, 2, "seed\n"),
                ("- - seed\n", "\t", 4, 0, "seed\n"),
            ] {
                for length in [minimum - 1, minimum, minimum + 1, minimum + 2048] {
                    for indent in ["", " ", "  ", "\t", " \t", "    "] {
                        for suffix in ["", " \t", "x", "`", " \t~", "\x0b"] {
                            let fence = marker.to_string().repeat(minimum);
                            let candidate = format!(
                                "{indent}{}{suffix}\r\n",
                                marker.to_string().repeat(length)
                            );
                            let closes =
                                model_close(&candidate, marker as u8, minimum, column, virtuals);
                            // Use a separate spelling for the opener, without
                            // a virtual indent that would alter body stripping.
                            let opener_prefix = if prefix == ">\t" {
                                "> "
                            } else if prefix == "\t" {
                                if before.starts_with("- -") {
                                    "    "
                                } else {
                                    "  "
                                }
                            } else {
                                prefix
                            };
                            let source =
                                format!("{before}{opener_prefix}{fence}\n{prefix}{candidate}");
                            let doc = parse(&source);
                            let expected = if closes {
                                seed.to_owned()
                            } else {
                                format!("{seed}{}{candidate}", " ".repeat(virtuals))
                                    .replace("\r\n", "\n")
                            };
                            assert_eq!(
                                visible(&doc),
                                expected,
                                "{marker} {minimum} {length} {prefix:?} {indent:?} {suffix:?}"
                            );
                            assert_eq!(
                                doc.blocks()
                                    .iter()
                                    .filter(|b| matches!(b.kind, BlockKind::Code { .. }))
                                    .count(),
                                1
                            );
                            cases += 1;
                        }
                    }
                }
            }
        }
    }
    assert_eq!(cases, 5760);
}

#[path = "support/snapshot.rs"]
mod support;

#[test]
fn every_utf8_split_and_scalar_prefix_agrees_before_and_after_finish() {
    for source in [
        format!(
            ">\t {} 中🙂 &amp;\r\n> \t中🙂\r\n> {} \r\n",
            "~".repeat(1024),
            "~".repeat(1030)
        ),
        format!(
            "- seed\n\t{}\r\n\t中🙂\n\t{}",
            "`".repeat(1024),
            "`".repeat(1024)
        ),
        format!("```{}中🙂` **tail**\r\n", "a".repeat(1024)),
        format!("~~~\n{} x\r\n中🙂\n~~~\n", "~".repeat(1100)),
        format!("> {}\n> literal\noutside\n", "`".repeat(1024)),
        format!("{} rs\r\nbody\n{}", "`".repeat(1024), "`".repeat(1023)),
    ] {
        let whole = support::snapshot(&parse(&source));
        for split in 0..=source.len() {
            let mut doc = Utf8Document::new();
            doc.append(&source.as_bytes()[..split]).unwrap();
            doc.append(&source.as_bytes()[split..]).unwrap();
            doc.finish();
            assert_eq!(
                support::snapshot(doc.document()),
                whole,
                "byte split {split}"
            );
        }
        let mut streamed = Document::new();
        for (at, ch) in source.char_indices() {
            streamed.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            let prefix = &source[..at + ch.len_utf8()];
            let mut batch = Document::new();
            batch.append(prefix).unwrap();
            assert_eq!(streamed.pending(), batch.pending());
            assert_eq!(support::snapshot(&streamed), support::snapshot(&batch));
            let mut eof = Document::new();
            for ch in prefix.chars() {
                eof.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            }
            eof.finish();
            assert_eq!(support::snapshot(&eof), support::snapshot(&parse(prefix)));
        }
    }
}

#[test]
fn million_marker_and_info_scalar_appends_charge_only_new_probe_bytes() {
    for (prefix, first, continuation, invalid) in [
        ("", "~".repeat(1024), "~", false),
        ("", "`".repeat(1024), "`", false),
        ("", format!("```{}", "a".repeat(1024)), "x", true),
        ("", format!("~~~{}", "a".repeat(1024)), "`", false),
        (&">".repeat(980), "~".repeat(1024), "~", false),
    ] {
        let mut doc = Document::new();
        doc.append(&format!("{prefix}{first}")).unwrap();
        let before = doc.work();
        for _ in 0..1_000_000 {
            doc.append(continuation).unwrap();
        }
        let after = doc.work();
        assert_eq!(after.input_bytes - before.input_bytes, 1_000_000);
        assert_eq!(
            after.fence_candidate_bytes - before.fence_candidate_bytes,
            1_000_000
        );
        assert_eq!(after.fence_candidate_steps, before.fence_candidate_steps);
        assert_eq!(after.classified_bytes, before.classified_bytes);
        assert_eq!(after.inline_bytes, 0);
        assert_eq!(after.container_bytes, before.container_bytes);
        assert_eq!(after.container_steps, before.container_steps);
        assert_eq!(after.tree_visits, before.tree_visits);
        assert!(doc.pending().len() <= BLOCK_LOOKAHEAD + 3);
        if invalid {
            doc.append("`").unwrap();
            assert_eq!(doc.work().inline_bytes, doc.source().len() as u64);
            let rejected = doc.work();
            for _ in 0..1000 {
                doc.append("x").unwrap();
            }
            assert_eq!(
                doc.work().fence_candidate_bytes,
                rejected.fence_candidate_bytes
            );
            assert_eq!(doc.work().inline_bytes - rejected.inline_bytes, 1000);
        }
        doc.finish();
        assert_eq!(
            matches!(doc.blocks()[0].kind, BlockKind::Code { .. }),
            !invalid
        );
        assert_eq!(doc.work().fence_candidate_steps, 2);
        assert!(doc.work().fence_candidate_bytes <= doc.source().len() as u64 + 2);
        assert_eq!(doc.work().input_bytes, doc.source().len() as u64);
    }
}

#[derive(Default)]
struct Sink(String);
impl LayoutSink for Sink {
    fn begin_tile(&mut self, _: TileId) {}
    fn text(&mut self, run: RenderRun<'_>) {
        if !run.style.contains(Style::HIDDEN) {
            self.0.push_str(run.text);
        }
    }
    fn object(&mut self, _: LinkId, _: &LinkInfo) {
        panic!("literal code has no objects");
    }
    fn end_tile(&mut self, _: TileId) -> u64 {
        1
    }
}

#[test]
fn raw_mapping_shared_budgets_layout_and_old_measurements_survive_only_until_resolution() {
    for (opener, suffix, expected) in [
        (
            format!(">  {} rs &amp;", "`".repeat(4096)),
            "\r\n> \t中🙂 &amp;\r\n> ```\n> ",
            " 中🙂 &amp;\n```\n",
        ),
        (format!("```{}", "a".repeat(4096)), "``` **tail**\n", ""),
    ] {
        let mut doc = Document::new();
        doc.append(&opener).unwrap();
        let original = doc.blocks().to_vec();
        let key = doc.tile_key(TileId(0)).unwrap();
        let mut cursor = doc.display_cursor(Span::new(0, opener.len())).unwrap();
        let mut heights = HeightIndex::default();
        heights.rebuild(&doc).unwrap();
        let ticket = heights.ticket(TileId(0)).unwrap();
        let mut stale =
            LayoutPass::new(&doc, &heights, TileId(0), usize::MAX as u64, false).unwrap();
        let mut update = doc.append(suffix).unwrap();
        if opener.starts_with('>') {
            update
                .merge(doc.append(&format!("{}\n", "`".repeat(4096))).unwrap())
                .unwrap();
        }
        assert!(
            update
                .dirty
                .iter()
                .any(|s| s.start == 0 && s.end >= opener.len())
        );
        assert_ne!(doc.tile_key(TileId(0)).unwrap(), key);
        assert!(doc.blocks().starts_with(&original));
        heights.apply(&doc, &update).unwrap();
        assert_eq!(
            heights.measure(ticket, 1),
            Err(LayoutError::StaleMeasurement)
        );
        assert_eq!(
            cursor.next(&doc, &mut Budget::new(WorkLimit::unlimited())),
            Err(LayoutError::StaleContinuation)
        );
        assert_eq!(
            stale.advance(
                &doc,
                &mut heights,
                &mut Budget::new(WorkLimit::unlimited()),
                &mut Sink::default()
            ),
            Err(LayoutError::StaleContinuation)
        );
        let update = doc.finish();
        heights.apply(&doc, &update).unwrap();
        let expected = if expected.is_empty() {
            format!("{} tail\n", "a".repeat(4096))
        } else {
            expected.to_owned()
        };
        assert_eq!(visible(&doc), expected);
        let mut covered = 0;
        for run in doc
            .display_runs_in(Span::new(0, doc.source().len()))
            .unwrap()
        {
            assert_eq!(run.source.start, covered);
            covered = run.source.end;
            if run.replacement.is_none() {
                assert_eq!(run.text, doc.text(run.source).unwrap());
            }
        }
        assert_eq!(covered, doc.source().len());
        let sliced: String = doc
            .source()
            .char_indices()
            .flat_map(|(at, ch)| {
                doc.display_runs_in(Span::new(at, at + ch.len_utf8()))
                    .unwrap()
            })
            .filter(|r| !r.style.contains(Style::HIDDEN))
            .map(|r| r.text)
            .collect();
        assert_eq!(sliced, expected);
        let mut display = doc
            .display_cursor(Span::new(0, doc.source().len()))
            .unwrap();
        let mut output = String::new();
        while display.source_position() < doc.source().len() {
            let pos = display.source_position();
            assert_eq!(
                display
                    .next(&doc, &mut Budget::new(WorkLimit::default()))
                    .unwrap(),
                DisplayStep::BudgetExhausted
            );
            assert_eq!(display.source_position(), pos);
            let mut budget = Budget::new(WorkLimit {
                bytes: 6,
                queries: 1,
                objects: 1,
                ..Default::default()
            });
            // A prior consumer spends one byte from this very same budget.
            assert!(budget.charge(WorkLimit {
                bytes: 1,
                ..Default::default()
            }));
            match display.next(&doc, &mut budget).unwrap() {
                DisplayStep::Run(run) => {
                    if !run.style.contains(Style::HIDDEN) {
                        output.push_str(run.text);
                    }
                }
                DisplayStep::BudgetExhausted => {
                    // The predecessor can leave too little room for a UTF-8
                    // scalar plus the cursor's two lookahead bytes.
                    assert_eq!(display.source_position(), pos);
                    let mut fresh = Budget::new(WorkLimit {
                        bytes: 6,
                        queries: 1,
                        objects: 1,
                        ..Default::default()
                    });
                    let DisplayStep::Run(run) = display.next(&doc, &mut fresh).unwrap() else {
                        panic!("fresh frame must progress")
                    };
                    if !run.style.contains(Style::HIDDEN) {
                        output.push_str(run.text);
                    }
                }
                other => panic!("unexpected display step: {other:?}"),
            }
            assert!(budget.used().bytes <= 6);
        }
        assert_eq!(output, expected);
        let mut pass = LayoutPass::new(&doc, &heights, TileId(0), u64::MAX, false).unwrap();
        let mut sink = Sink::default();
        for _ in 0..doc.source().len() * 4 {
            let mut budget = Budget::new(WorkLimit {
                bytes: 6,
                queries: 6,
                objects: 6,
                tiles: 6,
                recovery: 6,
                dependencies: 6,
            });
            assert!(budget.charge(WorkLimit {
                queries: 1,
                ..Default::default()
            }));
            match pass
                .advance(&doc, &mut heights, &mut budget, &mut sink)
                .unwrap()
            {
                Completion::EndOfDocument => break,
                Completion::BudgetExhausted => (),
                other => panic!("unexpected layout step {other:?}"),
            }
            assert!(budget.used().bytes <= 6);
        }
        assert_eq!(pass.source_position(), doc.source().len());
        // LayoutPass delivers raw RenderRun bytes; DisplayCursor above owns
        // the existing partial-tab replacement contract.
        let raw_expected = if opener.starts_with('>') {
            "\t中🙂 &amp;\n```\n"
        } else {
            &expected
        };
        assert_eq!(sink.0, raw_expected);
    }
}

#[test]
fn repeated_rejections_and_plain_prefixes_do_not_reacquire_candidates() {
    let mut doc = Document::new();
    for _ in 0..128 {
        doc.append(&format!("```{}", "a".repeat(4096))).unwrap();
        let before = doc.work();
        doc.append("``` **tail**\n\n").unwrap();
        assert_eq!(
            doc.work().fence_candidate_steps - before.fence_candidate_steps,
            1
        );
    }
    assert_eq!(doc.work().fence_candidate_steps, 256);
    assert_eq!(doc.work().inline_bytes, doc.source().len() as u64 - 128);
    assert!(doc.work().fence_candidate_bytes <= doc.source().len() as u64 + 256);
    for prefix in ["normal ", "`` ", "~~~x`", "    ~~~"] {
        // The tilde spelling remains a possible opener; all other prefixes
        // must enter the ordinary or indented-code stream immediately.
        let mut doc = Document::new();
        doc.append(&format!("{prefix}{}", "x".repeat(4096)))
            .unwrap();
        assert_eq!(doc.blocks().is_empty(), prefix == "~~~x`");
        let before = doc.work();
        for _ in 0..1000 {
            doc.append("x").unwrap();
        }
        assert_eq!(doc.work().classified_bytes, before.classified_bytes);
        if prefix == "normal " || prefix == "`` " {
            assert_eq!(doc.work().inline_bytes - before.inline_bytes, 1000);
        }
    }
    let mut doc = Document::new();
    let seed = "- seed\n";
    doc.append(seed).unwrap();
    let old = doc.blocks()[0].clone();
    doc.append(&format!("\t{}\n\t中🙂\n- next\n", "~".repeat(4096)))
        .unwrap();
    doc.finish();
    assert_eq!(doc.blocks()[0], old);
    assert_eq!(doc.blocks()[1].container, old.container);
    assert_eq!(visible(&doc), "seed\n中🙂\nnext\n");
}

#[test]
fn structural_prefix_can_leave_one_marker_byte_and_finish_resolves_only_once() {
    for depth in [980, 1021, 1022, 1023] {
        let prefix = ">".repeat(depth);
        let mut doc = Document::new();
        doc.append(&format!("{prefix}seed\n")).unwrap();
        let container = doc.blocks()[0].container;
        doc.append(&format!("{prefix}{}", "`".repeat(4096)))
            .unwrap();
        assert_eq!(doc.blocks().len(), 1);
        assert_eq!(doc.pending(), Span::new(depth + 5, depth + 5 + 1024));
        let before = doc.work();
        doc.append("rs\r").unwrap();
        assert_eq!(
            doc.work().fence_candidate_bytes - before.fence_candidate_bytes,
            3
        );
        let update = doc
            .append(&format!("\n{prefix}\t中🙂\n{prefix}{}", "`".repeat(4097)))
            .unwrap();
        assert_eq!(doc.blocks()[1].container, container);
        assert_eq!(doc.blocks()[1].state, SyntaxState::Growing);
        assert!(!update.new_blocks.is_empty());
        let update = doc.finish();
        assert_eq!(doc.blocks()[1].container, container);
        assert_eq!(doc.blocks()[1].state, SyntaxState::Sealed);
        assert!(update.new_blocks.is_empty());
        assert_eq!(doc.work().fence_candidate_steps, 4);
        let before = doc.work();
        assert!(doc.finish().dirty.is_empty());
        assert_eq!(doc.work(), before);
    }
    // The independent math dialect still requires a bounded complete line.
    for source in [
        format!("$${}\n", " ".repeat(4096)),
        format!("$$\nx\n$${}\n", " ".repeat(4096)),
    ] {
        let doc = parse(&source);
        assert_eq!(doc.work().fence_candidate_steps, 0);
    }
    let doc = parse(&format!(
        "{}\nbody\n{}\n",
        "~".repeat(4096),
        "~".repeat(4096)
    ));
    let mut cursor = doc
        .display_cursor(Span::new(0, doc.source().len()))
        .unwrap();
    let mut one = Budget::new(WorkLimit {
        bytes: 1,
        queries: 1,
        objects: 1,
        ..Default::default()
    });
    assert_eq!(
        cursor.next(&doc, &mut one).unwrap(),
        DisplayStep::BudgetExhausted
    );
    assert_eq!(cursor.source_position(), 0);
    assert_eq!(one.used(), WorkLimit::default());
}

#[test]
fn rejected_info_keeps_ordinary_header_eligibility_with_one_payload_consumption() {
    for (before, prefix, continuation) in [("", "", ""), ("", "> ", "> "), ("- seed\n", "\t", "  ")]
    {
        let raw = format!("{prefix}```{}", "a".repeat(1100));
        let tail = format!("` | b\r\n{continuation}--- | ---\r\n{continuation}x | y\n");
        let source = format!("{before}{raw}{tail}");
        for split in [
            before.len() + 1023,
            before.len() + 1024,
            before.len() + raw.len(),
            before.len() + raw.len() + 1,
            before.len() + raw.len() + 3,
            before.len() + raw.len() + 6,
            source.len(),
        ] {
            let mut doc = Document::new();
            doc.append(&source[..split]).unwrap();
            let ids = doc.blocks().len();
            doc.append(&source[split..]).unwrap();
            doc.finish();
            assert_eq!(doc.tables().len(), 1, "split {split}, {prefix:?}");
            let table = &doc.tables()[0];
            assert_eq!(table.columns.len(), 2);
            assert_eq!(table.rows().len(), 2);
            assert_eq!(
                doc.text(table.cell(0, 0).unwrap().source),
                Some(format!("```{}`", "a".repeat(1100)).as_str())
            );
            assert_eq!(doc.text(table.cell(1, 0).unwrap().source), Some("x"));
            assert_eq!(doc.text(table.cell(1, 1).unwrap().source), Some("y"));
            assert_eq!(table.source, Span::new(before.len(), source.len()));
            assert!(doc.work().inline_bytes <= source.len() as u64);
            assert!(doc.work().table_header_bytes <= source.len() as u64);
            assert!(doc.blocks().len() >= ids);
        }
        let mut doc = Document::new();
        doc.append(&format!("{before}{raw}` | b")).unwrap();
        let header = doc.blocks().last().unwrap().clone();
        let id = BlockId(doc.blocks().len() - 1);
        let update = doc
            .append(&format!("\r\n{continuation}--- | ---\n"))
            .unwrap();
        assert_eq!(doc.tables()[0].block, id);
        assert_eq!(doc.blocks()[id.0].container, header.container);
        assert_eq!(update.new_tables.len(), 1);
        assert_eq!(update.changed_blocks.start, id.0);
        assert!(
            update
                .dirty
                .iter()
                .any(|s| s.start <= header.source.start && s.end >= header.source.end)
        );
    }
}

#[test]
fn provisional_tail_is_visible_and_dirties_only_new_text_until_the_single_decision() {
    for closing in [false, true] {
        let mut doc = Document::new();
        if closing {
            doc.append("~~~\nbody\n").unwrap();
        }
        let start = doc.source().len();
        doc.append(&"~".repeat(4096)).unwrap();
        let pending = doc.pending();
        assert_eq!(pending, Span::new(start, start + BLOCK_LOOKAHEAD));
        let key = doc.tile_key(TileId(1)).unwrap();
        let mut heights = HeightIndex::default();
        heights.rebuild(&doc).unwrap();
        let ticket = heights.ticket(TileId(1)).unwrap();
        let update = doc.append(" ").unwrap();
        assert_eq!(doc.pending(), pending);
        assert!(update.dirty.iter().all(|s| s.start >= start + 4096));
        heights.apply(&doc, &update).unwrap();
        assert_eq!(doc.tile_key(TileId(1)).unwrap(), key);
        heights.measure(ticket, 1).unwrap();
        let raw = format!(
            "{}{} ",
            if closing { "body\n" } else { "" },
            "~".repeat(4096)
        );
        assert_eq!(visible(&doc), raw);
        let mut pass = LayoutPass::new(&doc, &heights, TileId(0), u64::MAX, false).unwrap();
        let mut sink = Sink::default();
        assert_eq!(
            pass.advance(
                &doc,
                &mut heights,
                &mut Budget::new(WorkLimit::unlimited()),
                &mut sink
            )
            .unwrap(),
            Completion::EndOfDocument
        );
        assert_eq!(sink.0, raw);
        let ids = doc.blocks().len();
        let ticket = heights.ticket(TileId(1)).unwrap();
        let update = doc.append("\n").unwrap();
        assert!(
            update
                .dirty
                .iter()
                .any(|s| s.start == start && s.end == start + 4098)
        );
        heights.apply(&doc, &update).unwrap();
        assert_eq!(
            heights.measure(ticket, 1),
            Err(LayoutError::StaleMeasurement)
        );
        assert_eq!(doc.blocks().len(), if closing { ids } else { ids + 1 });
        assert_eq!(visible(&doc), if closing { "body\n" } else { "" });
    }
}

#[test]
fn failed_candidate_preserves_existing_table_identity_and_success_interrupts_it() {
    for valid in [false, true] {
        let mut doc = Document::new();
        doc.append("h | z\n--- | ---\n").unwrap();
        let block = doc.tables()[0].block;
        doc.append(&format!("```{}", "a".repeat(4096))).unwrap();
        assert_eq!(doc.tables()[0].rows().len(), 1);
        let update = doc
            .append(if valid {
                "\r\nx | y\n"
            } else {
                "` | b\r\nx | y\n"
            })
            .unwrap();
        assert_eq!(doc.tables()[0].block, block);
        if valid {
            assert_eq!(doc.blocks().len(), 2);
            assert!(matches!(doc.blocks()[1].kind, BlockKind::Code { .. }));
            assert_eq!(doc.tables()[0].rows().len(), 1);
        } else {
            assert_eq!(doc.blocks().len(), 1);
            assert_eq!(doc.tables()[0].rows().len(), 3);
            assert_eq!(
                doc.text(doc.tables()[0].cell(1, 1).unwrap().source),
                Some("b")
            );
            assert_eq!(
                doc.text(doc.tables()[0].cell(2, 0).unwrap().source),
                Some("x")
            );
            assert!(update.tables_changed.contains(&TableId(0)));
            assert!(doc.work().inline_bytes <= doc.source().len() as u64);
        }
    }
}

fn model_open(line: &str) -> Option<(u8, usize, &str)> {
    let line = line.strip_suffix('\n').unwrap_or(line);
    let line = line.strip_suffix('\r').unwrap_or(line);
    let marker = *line.as_bytes().first()?;
    if marker != b'`' && marker != b'~' {
        return None;
    }
    let length = line.bytes().take_while(|&byte| byte == marker).count();
    let info = &line[length..];
    if length < 3 || (marker == b'`' && info.contains('`')) {
        return None;
    }
    Some((marker, length, info.trim_ascii()))
}

#[test]
fn independent_opener_model_confirms_full_info_or_rejects_backticks_in_any_late_position() {
    let mut cases = 0;
    for marker in ['`', '~'] {
        for length in [1, 2, 3, 1023, 1024, 4096] {
            for info in [
                String::new(),
                "rs".to_owned(),
                "a".repeat(4096),
                format!("{}`", "a".repeat(4096)),
                format!("{} 中🙂 \\+ &amp;\t", " ".repeat(4096)),
            ] {
                for eol in ["\n", "\r\n", ""] {
                    let source = format!("{}{info}{eol}", marker.to_string().repeat(length));
                    let expected = model_open(&source);
                    let doc = parse(&source);
                    let code = doc.blocks().iter().find_map(|b| {
                        if let BlockKind::Code { info } = b.kind {
                            Some(info)
                        } else {
                            None
                        }
                    });
                    assert_eq!(
                        code.is_some(),
                        expected.is_some(),
                        "{marker} {length} {}",
                        info.len()
                    );
                    if let Some((_, _, raw_info)) = expected {
                        assert_eq!(
                            code.unwrap()
                                .map(|span| doc.text(span).unwrap())
                                .unwrap_or(""),
                            raw_info
                        );
                    }
                    assert!(doc.work().inline_bytes <= source.len() as u64);
                    cases += 1;
                }
            }
        }
    }
    assert_eq!(cases, 180);
}
