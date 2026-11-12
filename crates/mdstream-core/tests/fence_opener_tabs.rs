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
fn openers_use_physical_tab_columns_and_preserve_info() {
    let mut cases = 0;
    for (before, prefix, body_prefix, seed, residual) in [
        ("", "> ", "> ", "", 0),
        ("", ">> ", ">> ", "", 0),
        ("", "> > ", "> > ", "", 0),
        ("", ">>> ", ">>> ", "", 0),
        ("- seed\n", "  ", "  ", "seed\n", 0),
        ("1. seed\n", "   ", "   ", "seed\n", 0),
        ("> - seed\n", ">   ", ">   ", "seed\n", 0),
        ("- seed\n", "\t", "  ", "seed\n", 2),
        ("> seed\n", ">\t", "> ", "seed\n", 2),
        ("- - seed\n", "\t", "    ", "seed\n", 0),
    ] {
        for whitespace in ["", " ", "   ", "\t", " \t", "  \t", "\t ", "\t\t"] {
            let mut column = 0;
            for byte in prefix.bytes() {
                column += if byte == b'\t' { 4 - column % 4 } else { 1 };
            }
            let start = column;
            for byte in whitespace.bytes() {
                column += if byte == b'\t' { 4 - column % 4 } else { 1 };
            }
            let indent = residual + column - start;
            for marker in ["```", "~~~~", "~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~"] {
                for ending in ["\n", "\r\n", ""] {
                    let source = format!(
                        "{before}{prefix}{whitespace}{marker} rs &amp;\n{body_prefix}    中🙂 *x*\t{ending}"
                    );
                    let doc = parse(&source);
                    let info = doc.blocks().iter().find_map(|block| match block.kind {
                        BlockKind::Code {
                            info: Some(info), ..
                        } => Some(info),
                        _ => None,
                    });
                    assert_eq!(info.is_some(), indent <= 3, "{source:?}");
                    if let Some(info) = info {
                        assert_eq!(&source[info.range()], "rs &amp;");
                        assert_eq!(
                            visible(&doc),
                            format!(
                                "{seed}{}中🙂 *x*\t{}",
                                " ".repeat(4 - indent),
                                if ending.is_empty() { "" } else { "\n" }
                            ),
                            "{source:?}"
                        );
                    }
                    assert_eq!(doc.source(), source);
                    cases += 1;
                }
            }
        }
    }
    assert_eq!(cases, 720);
}

#[test]
fn failed_candidates_scopes_math_and_window_boundaries() {
    for (source, has_info) in [
        ("> \t```rs\n", true),
        ("> \t```rs`\n", false),
        ("> \t~~ rs\n", false),
        ("> \t~~~rs`\n", true),
        ("> \t\t```rs\n", false),
        ("\t```rs\n", false),
        ("> \t$$\n", false),
        ("> seed\n>\t ```rs\n", true),
        ("> seed\n>\t  ```rs\n", false),
        ("- > seed\n\t```rs\n", false),
    ] {
        let doc = parse(source);
        assert_eq!(
            doc.blocks()
                .iter()
                .any(|b| matches!(b.kind, BlockKind::Code { info: Some(_), .. })),
            has_info,
            "{source:?}"
        );
        assert_eq!(
            doc.blocks()
                .iter()
                .any(|b| matches!(b.kind, BlockKind::Math { .. })),
            source == "> \t$$\n",
            "{source:?}"
        );
    }
    for (source, expected) in [
        ("> \t```\n> \tx\n> ```\n", "x\n"),
        ("> \t~~~\n> \t x\n> ~~~\n", " x\n"),
        ("> \t$$\n", ""),
        ("> \t# title\n", "title\n"),
        ("> \t```\n# outside\n", "outside\n"),
        ("- seed\n  \t```\n  \tx\n- next\n", "seed\nx\nnext\n"),
    ] {
        assert_eq!(visible(&parse(source)), expected, "{source:?}");
    }
    // Prefix and raw tab consume three bytes of the existing shared window.
    for length in [
        BLOCK_LOOKAHEAD - 5,
        BLOCK_LOOKAHEAD - 4,
        BLOCK_LOOKAHEAD - 3,
        BLOCK_LOOKAHEAD,
        2048,
    ] {
        let marker = "~".repeat(length);
        let source = format!("> \t{marker}\n>   x\n");
        let doc = parse(&source);
        assert!(
            doc.blocks()
                .iter()
                .any(|b| matches!(b.kind, BlockKind::Code { .. })),
            "length {length}"
        );
        assert_eq!(visible(&doc), "x\n");
    }
}

#[test]
fn splits_prefixes_eof_and_late_opener_keep_identity() {
    for source in [
        "> \t```rs\r\n>   中🙂\r\n> ```\r\n> # after\n",
        "1. seed\n   \t~~~\n   中🙂\n   ~~~~",
        "- seed\n  \t```\n\t中🙂\n  ```\n- next\n",
        "> seed\n>\t ```\n> \t中🙂\n> ```\n",
        "> $$\n> \t$$\n> $$\n",
        "> \t```bad`info\n> after\n",
        "> \t\t~~~\n> after\n",
        "- > seed\n\t~~~rs\n",
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
    doc.append("> seed\n> \t```").unwrap();
    let container = doc.blocks()[0].container;
    let key = doc.tile_key(TileId(0)).unwrap();
    let mut cursor = doc
        .display_cursor(Span::new(0, doc.source().len()))
        .unwrap();
    doc.append(" rs\n>   中\n> ```\n> # after\n").unwrap();
    doc.finish();
    assert_eq!(visible(&doc), "seed\n中\nafter\n");
    assert_eq!(doc.blocks()[0].container, container);
    assert_eq!(doc.blocks()[1].container, container);
    assert!(matches!(
        doc.blocks()[2].kind,
        BlockKind::Heading { level: 1 }
    ));
    assert_ne!(doc.tile_key(TileId(0)).unwrap(), key);
    assert_eq!(
        cursor.next(&doc, &mut Budget::new(WorkLimit::unlimited())),
        Err(LayoutError::StaleContinuation)
    );
}

#[test]
fn mapping_tiles_slices_and_six_byte_budget_hide_opener() {
    let source = format!("> {}\n> \t``` rs\r\n>   中🙂\n> ```\n", "x".repeat(251));
    let doc = parse(&source);
    let tab = source.find('\t').unwrap();
    assert_eq!(tab, 256);
    let expected = format!("{}\n中🙂\n", "x".repeat(251));
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
fn deep_long_body_appends_do_not_rescan_prefix_or_old_text() {
    let prefix = format!("{} ", ">".repeat(1000));
    let mut doc = Document::new();
    doc.append(&format!("{prefix}\t~~~\n{prefix}{}", "x".repeat(1_000_000)))
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
    assert_eq!(after.inline_bytes, before.inline_bytes);
    assert_eq!(after.tree_visits, before.tree_visits);
    doc.append(&format!("\n{prefix}~~~\n{prefix}# after\n"))
        .unwrap();
    doc.finish();
    assert_eq!(visible(&doc), format!("{}\nafter\n", "x".repeat(1_001_000)));
    assert_eq!(doc.container_count(), 1000);
    assert_eq!(doc.blocks().len(), 2);
    assert!(doc.work().classified_bytes < 30 * BLOCK_LOOKAHEAD as u64);
    // A long run keeps a single raw candidate until LF.
    let mut failed = Document::new();
    failed
        .append(&format!("{prefix}\t{}", "~".repeat(2048)))
        .unwrap();
    let before = failed.work();
    for _ in 0..1000 {
        failed.append("~").unwrap();
    }
    let after = failed.work();
    assert_eq!(after.classified_bytes, before.classified_bytes);
    assert_eq!(after.container_bytes, before.container_bytes);
    assert_eq!(after.container_steps, before.container_steps);
    failed.append(" rs\n").unwrap();
    failed.finish();
    assert!(
        failed
            .blocks()
            .iter()
            .any(|b| matches!(b.kind, BlockKind::Code { .. }))
    );
    let source = "> \t~~~\n>   x\n> ~~~\n".repeat(4096);
    let doc = parse(&source);
    assert_eq!(doc.blocks().len(), 4096);
    assert_eq!(visible(&doc), "x\n".repeat(4096));
    assert!(doc.work().classified_bytes < 10 * source.len() as u64);
    assert!(doc.work().container_steps < 8 * source.len() as u64);
}
