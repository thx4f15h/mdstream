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

#[test]
fn long_fences_match_length_and_indent_model() {
    let mut cases = 0;
    for marker in ['`', '~'] {
        for length in [3, 32, 33, 63, 255, 1000, 1023, 1024, 4096] {
            let fence = marker.to_string().repeat(length);
            for indent in 0..=3 {
                for (before, prefix, seed) in [
                    ("", "", ""),
                    ("", "> ", ""),
                    ("- seed\n", "  ", "seed\n"),
                    ("- seed\n", "\t", "seed\n"),
                ] {
                    // A tab after a two-column Item leaves two virtual spaces.
                    if prefix == "\t" && indent > 1 {
                        continue;
                    }
                    for ending in ["\n", "\r\n", ""] {
                        let source = format!(
                            "{before}{prefix}{}{fence} rs &amp;\n{prefix}{}中🙂 *x*\n{prefix}{fence}{ending}",
                            " ".repeat(indent),
                            " ".repeat(indent)
                        );
                        let doc = parse(&source);
                        assert_eq!(visible(&doc), format!("{seed}中🙂 *x*\n"), "{source:?}");
                        let BlockKind::Code { info: Some(info) } =
                            doc.blocks().last().unwrap().kind
                        else {
                            panic!("expected code: {source:?}")
                        };
                        assert_eq!(&doc.source()[info.range()], "rs &amp;");
                        assert!(doc.links().is_empty());
                        cases += 1;
                    }
                }
            }
        }
    }
    assert_eq!(cases, 756);
}

#[test]
fn window_edges_require_a_complete_run_and_keep_closer_rules() {
    for marker in ['`', '~'] {
        for prefix in ["", "> ", ">> "] {
            let length = BLOCK_LOOKAHEAD - prefix.len() - 1;
            let fence = marker.to_string().repeat(length);
            for ending in ["\n", ""] {
                let source = format!("{prefix}{fence}\n{prefix}x\n{prefix}{fence}{ending}");
                assert_eq!(visible(&parse(&source)), "x\n");
            }
            // Prefix + delimiter fills the window; the probe confirms at LF.
            let fence = marker.to_string().repeat(length + 1);
            let source = format!("{prefix}{fence}\n{prefix}body\n");
            assert_eq!(visible(&parse(&source)), "body\n");
            // Info and the run are confirmed at the actual physical line end.
            let fence = marker.to_string().repeat(length);
            let source = format!("{prefix}{fence} x\n{prefix}body\n{prefix}{fence}\n");
            assert_eq!(visible(&parse(&source)), "body\n");
        }
        let fence = marker.to_string().repeat(33);
        for body in [
            marker.to_string().repeat(32),
            format!("{fence}x"),
            format!("    {fence}"),
            if marker == '`' {
                "~".repeat(33)
            } else {
                "`".repeat(33)
            },
            format!("{}x", marker.to_string().repeat(BLOCK_LOOKAHEAD)),
        ] {
            let source = format!("{fence}\n{body}\n{fence}\nafter\n");
            let doc = parse(&source);
            assert_eq!(visible(&doc), format!("{body}\nafter\n"));
            assert_eq!(doc.blocks().len(), 2);
        }
        let source = format!("{fence}\nx\n{fence}{marker}\t \r\nafter\n");
        assert_eq!(visible(&parse(&source)), "x\nafter\n");
    }
    let source = format!("{}bad`\ntext\n", "`".repeat(33));
    assert!(
        parse(&source)
            .blocks()
            .iter()
            .all(|b| !matches!(b.kind, BlockKind::Code { .. }))
    );
}

#[test]
fn splits_prefixes_eof_and_scope_transitions_are_stable() {
    let sources = [
        format!("{}rs\r\n中🙂\r\n{}\n", "`".repeat(33), "`".repeat(34)),
        format!(
            "- seed\n\t{}\n\t中🙂\n  {}\n- next\n",
            "~".repeat(63),
            "~".repeat(63)
        ),
        format!("> {}\n> 中\noutside\n", "`".repeat(33)),
        format!("{}\n中\n{}", "~".repeat(1023), "~".repeat(1023)),
        format!("> {}\n中\n", "`".repeat(1022)),
    ];
    for source in sources {
        let whole = support::snapshot(&parse(&source));
        for split in (0..=source.len()).filter(|&at| source.is_char_boundary(at)) {
            let mut doc = Document::new();
            doc.append(&source[..split]).unwrap();
            doc.append(&source[split..]).unwrap();
            doc.finish();
            assert_eq!(support::snapshot(&doc), whole, "split {split}");
        }
        let mut doc = Document::new();
        for (at, ch) in source.char_indices() {
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            let text = &source[..at + ch.len_utf8()];
            let mut prefix = Document::new();
            prefix.append(text).unwrap();
            assert_eq!(support::snapshot(&doc), support::snapshot(&prefix));
            assert_eq!(doc.pending(), prefix.pending());
            let mut eof = Document::new();
            eof.append(text).unwrap();
            eof.finish();
            let mut chunked_eof = Document::new();
            for chunk in text.chars() {
                chunked_eof.append(chunk.encode_utf8(&mut [0; 4])).unwrap();
            }
            chunked_eof.finish();
            assert_eq!(support::snapshot(&chunked_eof), support::snapshot(&eof));
        }
    }
}

#[test]
fn mapping_slices_and_budget_preserve_literal_code() {
    let fence = "`".repeat(255);
    let source = format!(">  {fence} rust\n> \t中🙂 &amp;\r\n> {fence}\n");
    let doc = parse(&source);
    let expected = " 中🙂 &amp;\n";
    assert_eq!(visible(&doc), expected);
    let mut covered = 0;
    for run in doc.display_runs_in(Span::new(0, source.len())).unwrap() {
        assert_eq!(run.source.start, covered);
        covered = run.source.end;
        if let Some(mapped) = run.replacement {
            assert_eq!(&source[mapped.range()], "\t");
            assert_eq!(run.text, " ");
        } else {
            assert_eq!(run.text, &source[run.source.range()]);
        }
    }
    assert_eq!(covered, source.len());
    let sliced: String = source
        .char_indices()
        .flat_map(|(at, ch)| {
            doc.display_runs_in(Span::new(at, at + ch.len_utf8()))
                .unwrap()
        })
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    assert_eq!(sliced, expected);
    let mut cursor = doc.display_cursor(Span::new(0, source.len())).unwrap();
    let mut output = String::new();
    while cursor.source_position() < source.len() {
        let position = cursor.source_position();
        assert_eq!(
            cursor
                .next(&doc, &mut Budget::new(WorkLimit::default()))
                .unwrap(),
            DisplayStep::BudgetExhausted
        );
        assert_eq!(cursor.source_position(), position);
        let mut budget = Budget::new(WorkLimit {
            bytes: 6,
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        let DisplayStep::Run(run) = cursor.next(&doc, &mut budget).unwrap() else {
            panic!("must advance")
        };
        assert!(budget.used().bytes <= 6);
        if !run.style.contains(Style::HIDDEN) {
            output.push_str(run.text);
        }
    }
    assert_eq!(output, expected);
}

#[test]
fn long_success_and_failure_do_not_rescan_old_input() {
    for (prefix, marker_count, recognized) in [
        ("".to_owned(), 1023, true),
        (">".repeat(980), 40, true),
        (">".repeat(980), 44, true),
        (">".repeat(1000), 23, true),
        (">".repeat(1000), 24, true),
        ("".to_owned(), 1024, true),
    ] {
        let mut doc = Document::new();
        let opener = format!("{prefix}{}", "~".repeat(marker_count));
        for ch in opener.chars() {
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
        }
        doc.append(&format!("\n{prefix}{}\n", "x".repeat(1_000_000)))
            .unwrap();
        // Start another long body line, then measure only its extension.
        doc.append(&format!("{prefix}{}", "x".repeat(2048)))
            .unwrap();
        let before = doc.work();
        for _ in 0..1000 {
            doc.append("x").unwrap();
        }
        let after = doc.work();
        assert_eq!(after.input_bytes - before.input_bytes, 1000);
        assert_eq!(after.classified_bytes, before.classified_bytes);
        assert_eq!(after.container_bytes, before.container_bytes);
        assert_eq!(after.container_steps, before.container_steps);
        if recognized {
            assert_eq!(after.inline_bytes, before.inline_bytes);
        } else {
            assert!(after.inline_bytes - before.inline_bytes <= 1001);
        }
        doc.finish();
        assert_eq!(
            matches!(doc.blocks()[0].kind, BlockKind::Code { .. }),
            recognized
        );
        assert!(doc.work().classified_bytes < 12 * BLOCK_LOOKAHEAD as u64);
    }
}
