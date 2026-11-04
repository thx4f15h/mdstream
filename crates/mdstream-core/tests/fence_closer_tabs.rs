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
fn closers_match_independent_column_and_length_model() {
    let mut cases = 0;
    // Prefix, opener prefix, seed, visible seed, and unused prefix-tab columns.
    for (prefix, opener, before, seed, residual) in [
        ("", "", "", "", 0),
        ("> ", "> ", "", "", 0),
        (">> ", ">> ", "", "", 0),
        ("> > ", "> > ", "", "", 0),
        (">>> ", ">>> ", "", "", 0),
        ("  ", "  ", "- seed\n", "seed\n", 0),
        ("   ", "   ", "1. seed\n", "seed\n", 0),
        (">   ", ">   ", "> - seed\n", "seed\n", 0),
        ("\t", "  ", "- seed\n", "seed\n", 2),
        (">\t", "> ", "", "", 2),
    ] {
        for whitespace in ["", " ", "   ", "\t", " \t", "  \t", "\t ", "\t\t"] {
            let mut column = 0;
            for byte in prefix.bytes() {
                column += if byte == b'\t' { 4 - column % 4 } else { 1 };
            }
            let start_column = column;
            for byte in whitespace.bytes() {
                column += if byte == b'\t' { 4 - column % 4 } else { 1 };
            }
            let indent = residual + column - start_column;
            for marker in ['`', '~'] {
                for length in [2, 3, 4, 33] {
                    for ending in ["\n", "\r\n", ""] {
                        let delimiter = marker.to_string().repeat(length);
                        let source = format!(
                            "{before}{opener}{} rs\n{opener}中🙂\n{prefix}{whitespace}{delimiter}\t {ending}",
                            marker.to_string().repeat(3)
                        );
                        let closes = indent <= 3 && length >= 3;
                        let expected = if closes {
                            format!("{seed}中🙂\n")
                        } else {
                            format!(
                                "{seed}中🙂\n{}{whitespace}{delimiter}\t {}",
                                " ".repeat(residual),
                                if ending.is_empty() { "" } else { "\n" }
                            )
                        };
                        let doc = parse(&source);
                        assert_eq!(visible(&doc), expected, "{source:?}");
                        assert!(matches!(
                            doc.blocks().last().unwrap().kind,
                            BlockKind::Code { .. }
                        ));
                        assert_eq!(doc.source(), source);
                        cases += 1;
                    }
                }
            }
        }
    }
    assert_eq!(cases, 1920);
}

#[test]
fn scope_math_failure_and_window_boundaries() {
    for (source, expected) in [
        ("> ```\n> x\n> \t```\n> # after\n", "x\nafter\n"),
        ("> ```\n> \t~~~\n> ```\n", "\t~~~\n"),
        ("> ```\n> \t```x\n> ```\n", "\t```x\n"),
        ("> ```\n> \t``` `\n> ```\n", "\t``` `\n"),
        ("> ```\n> \t\t```\n> ```\n", "\t\t```\n"),
        ("```\n\t```\n```\n", "\t```\n"),
        ("> $$\n> \t$$\n> $$\n", ""),
        ("> ```\n> x\n\t```\n", "x\n```\n"),
        ("- seed\n  ```\n  x\n- next\n", "seed\nx\nnext\n"),
    ] {
        assert_eq!(visible(&parse(source)), expected, "{source:?}");
    }
    // The candidate must be complete inside the existing classification window.
    for length in [
        BLOCK_LOOKAHEAD - 5,
        BLOCK_LOOKAHEAD - 4,
        BLOCK_LOOKAHEAD,
        2048,
    ] {
        let delimiter = "~".repeat(length);
        let source = format!("> ~~~\n> x\n> \t{delimiter}\n> # after\n");
        let closes = 3 + length < BLOCK_LOOKAHEAD;
        let expected = if closes {
            "x\nafter\n".to_owned()
        } else {
            format!("x\n\t{delimiter}\n# after\n")
        };
        assert_eq!(visible(&parse(&source)), expected, "length {length}");
    }
}

#[test]
fn splits_prefixes_eof_and_late_closure_keep_identity() {
    for source in [
        "> ```rs\r\n> 中🙂\r\n> \t```\r\n> # after\n",
        "1. seed\n   ~~~\n   中🙂\n   \t~~~~",
        "- seed\n  ```\n\t```\n- next\n",
        "> ```\n>\t\t```\n> ```\n",
        "> $$\n> \t$$\n> $$\n",
    ] {
        let whole = support::snapshot(&parse(source));
        for split in (0..=source.len()).filter(|&at| source.is_char_boundary(at)) {
            let mut doc = Document::new();
            doc.append(&source[..split]).unwrap();
            doc.append(&source[split..]).unwrap();
            doc.finish();
            assert_eq!(support::snapshot(&doc), whole, "split {split}: {source:?}");
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
            for ch in text.chars() {
                eof.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            }
            eof.finish();
            assert_eq!(support::snapshot(&eof), support::snapshot(&parse(text)));
        }
    }
    let mut doc = Document::new();
    doc.append("> ```\n> 中\n> \t```").unwrap();
    let container = doc.blocks()[0].container;
    let key = doc.tile_key(TileId(0)).unwrap();
    let mut cursor = doc
        .display_cursor(Span::new(0, doc.source().len()))
        .unwrap();
    doc.append("\n> # after\n").unwrap();
    doc.finish();
    assert_eq!(visible(&doc), "中\nafter\n");
    assert_eq!(doc.blocks()[0].container, container);
    assert_eq!(doc.blocks()[1].container, container);
    assert!(matches!(
        doc.blocks()[1].kind,
        BlockKind::Heading { level: 1 }
    ));
    assert_ne!(doc.tile_key(TileId(0)).unwrap(), key);
    assert_eq!(
        cursor.next(&doc, &mut Budget::new(WorkLimit::unlimited())),
        Err(LayoutError::StaleContinuation)
    );
}

#[test]
fn mapping_tiles_slices_and_six_byte_budget_hide_closer() {
    let source = format!("> ```\n> {}\n> \t```\r\n> 中🙂\n", "x".repeat(245));
    let doc = parse(&source);
    let tab = source.find('\t').unwrap();
    assert_eq!(tab, 256);
    let expected = format!("{}\n中🙂\n", "x".repeat(245));
    assert_eq!(visible(&doc), expected);
    let mut covered = 0;
    for run in doc.display_runs_in(Span::new(0, source.len())).unwrap() {
        assert_eq!(run.source.start, covered);
        assert_eq!(run.text, &source[run.source.range()]);
        assert!(run.replacement.is_none());
        if run.source.start <= tab && tab < run.source.end {
            assert!(run.style.contains(Style::HIDDEN));
        }
        covered = run.source.end;
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
    let tiled: String = (0..doc.tile_count())
        .flat_map(|i| doc.display_runs(TileId(i)).unwrap())
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    assert_eq!(tiled, expected);
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
fn deep_long_bodies_and_failed_closers_do_not_rescan() {
    let prefix = format!("{} ", ">".repeat(1000));
    let mut doc = Document::new();
    doc.append(&format!(
        "{prefix}~~~\n{prefix}{}\n{prefix}\t{}",
        "x".repeat(1_000_000),
        "~".repeat(2048)
    ))
    .unwrap();
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("~").unwrap();
    }
    let after = doc.work();
    assert_eq!(after.input_bytes - before.input_bytes, 1000);
    assert_eq!(after.classified_bytes, before.classified_bytes);
    assert_eq!(after.container_bytes, before.container_bytes);
    assert_eq!(after.container_steps, before.container_steps);
    assert_eq!(after.inline_bytes, before.inline_bytes);
    assert_eq!(after.tree_visits, before.tree_visits);
    doc.append(&format!("\n{prefix}\t~~~\n{prefix}# after\n"))
        .unwrap();
    doc.finish();
    assert_eq!(
        visible(&doc),
        format!("{}\n\t{}\nafter\n", "x".repeat(1_000_000), "~".repeat(3048))
    );
    assert_eq!(doc.container_count(), 1000);
    assert_eq!(doc.blocks().len(), 2);
    assert!(doc.work().classified_bytes < 30 * BLOCK_LOOKAHEAD as u64);
    let source = "> ~~~\n> x\n> \t~~~\n".repeat(4096);
    let doc = parse(&source);
    assert_eq!(doc.blocks().len(), 4096);
    assert_eq!(visible(&doc), "x\n".repeat(4096));
    assert!(doc.work().classified_bytes < 8 * source.len() as u64);
    assert!(doc.work().container_steps < 8 * source.len() as u64);
}
