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
fn fenced_tabs_match_independent_physical_column_model() {
    let mut cases = 0;
    for (before, prefix, seed) in [
        ("", "", ""),
        ("", "> ", ""),
        ("", ">> ", ""),
        ("", "> > ", ""),
        ("- seed\n", "  ", "seed\n"),
        ("> - seed\n", ">   ", "seed\n"),
    ] {
        for indent in 0..=3 {
            for whitespace in ["\t", " \t", "  \t", "   \t", "\t\t", " \t \t", "  \t  "] {
                let mut column = prefix.len();
                let mut removed = 0;
                let mut consumed = 0;
                let mut residual = 0;
                for byte in whitespace.bytes() {
                    if removed == indent {
                        break;
                    }
                    let width = if byte == b'\t' { 4 - column % 4 } else { 1 };
                    column += width;
                    consumed += 1;
                    let take = width.min(indent - removed);
                    removed += take;
                    residual = width - take;
                }
                for marker in ["```", "~~~~"] {
                    for ending in ["\n", "\r\n", ""] {
                        let source = format!(
                            "{before}{prefix}{}{marker} rs\n{prefix}{whitespace}中🙂 *x* &amp;\t{ending}",
                            " ".repeat(indent)
                        );
                        let expected = format!(
                            "{seed}{}{}中🙂 *x* &amp;\t{}",
                            " ".repeat(residual),
                            &whitespace[consumed..],
                            if ending.is_empty() { "" } else { "\n" }
                        );
                        let doc = parse(&source);
                        assert_eq!(visible(&doc), expected, "{source:?}");
                        assert_eq!(doc.source(), source);
                        assert!(matches!(
                            doc.blocks().last().unwrap().kind,
                            BlockKind::Code { .. }
                        ));
                        assert!(doc.links().is_empty());
                        cases += 1;
                    }
                }
            }
        }
    }
    assert_eq!(cases, 1008);
}

#[test]
fn partial_ancestor_tabs_and_literal_boundaries() {
    for (source, expected) in [
        ("- seed\n\t ```\n\t\t中\n  ```\n", "seed\n   中\n"),
        ("- seed\n\t```\n\t\t中\n  ```\n", "seed\n\t中\n"),
        ("   ```\n\t\n \t\r\n  \t\n```\n", " \n \n \n"),
        ("   ```\n\t```\n```\n", " ```\n"),
        ("```\n\tx\n```\n", "\tx\n"),
        ("   ```\nx\t\n   \tx\n```\n", "x\t\n\tx\n"),
        ("  $$\n\tx\n$$\n", "\tx\n"),
        (">   ```\n> \tx\n  ~~~\n\ty\n~~~\n", "x\n  y\n"),
    ] {
        assert_eq!(visible(&parse(source)), expected, "{source:?}");
    }
}

#[test]
fn all_splits_prefixes_eof_and_cache_invalidation() {
    for source in [
        "   ```rs\r\n \t中🙂\r\n\t\n```\n",
        ">>    ~~~\n>> \t\t中🙂\n>> ~~~",
        "- seed\n\t ```\n\t\t中🙂\n  ```\n",
        "   ```\n\t```\n```\n",
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
    doc.append("   ```\n\t中").unwrap();
    let key = doc.tile_key(TileId(0)).unwrap();
    let mut cursor = doc
        .display_cursor(Span::new(0, doc.source().len()))
        .unwrap();
    doc.append("\n").unwrap();
    assert_eq!(visible(&doc), " 中\n");
    assert_ne!(doc.tile_key(TileId(0)).unwrap(), key);
    assert_eq!(
        cursor.next(&doc, &mut Budget::new(WorkLimit::unlimited())),
        Err(LayoutError::StaleContinuation)
    );
}

#[test]
fn tab_mapping_slices_tiles_and_six_byte_budget() {
    let source = format!("   ```\n{}\n\t中🙂\tend\r\n```\n", "x".repeat(248));
    let doc = parse(&source);
    let tab = source.find('\t').unwrap();
    assert_eq!(tab, 256);
    let mut covered = 0;
    let mut replacements = 0;
    for run in doc.display_runs_in(Span::new(0, source.len())).unwrap() {
        assert_eq!(run.source.start, covered);
        covered = run.source.end;
        if let Some(mapped) = run.replacement {
            assert_eq!(mapped, Span::new(tab, tab + 1));
            assert_eq!(run.source, mapped);
            assert_eq!(run.text, " ");
            assert!(run.style.contains(Style::CODE));
            replacements += 1;
        } else {
            assert_eq!(run.text, &source[run.source.range()]);
        }
    }
    assert_eq!((covered, replacements), (source.len(), 1));
    let expected = format!("{}\n 中🙂\tend\n", "x".repeat(248));
    assert_eq!(visible(&doc), expected);
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
fn streaming_indent_crosses_window_and_million_byte_body_without_rescan() {
    let opener = format!("{}    ```\n", ">".repeat(1015));
    let prefix = format!("{}{}", "> ".repeat(7), ">".repeat(1008));
    let source = format!("{opener}{prefix}  \t\t中🙂\n");
    let whole = parse(&source);
    assert_eq!(visible(&whole), "  \t中🙂\n");
    for split in opener.len() + BLOCK_LOOKAHEAD - 2..=opener.len() + BLOCK_LOOKAHEAD + 2 {
        let mut doc = Document::new();
        doc.append(&source[..split]).unwrap();
        for ch in source[split..].chars() {
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
        }
        doc.finish();
        assert_eq!(support::snapshot(&doc), support::snapshot(&whole));
    }
    let mut doc = Document::new();
    doc.append(&format!("{opener}{prefix}  ")).unwrap();
    let before = doc.work();
    doc.append("\t").unwrap();
    assert_eq!(doc.work().container_bytes, before.container_bytes);
    assert_eq!(doc.work().container_steps, before.container_steps);
    assert_eq!(doc.work().classified_bytes, before.classified_bytes);
    doc.append(&"x".repeat(1_000_000)).unwrap();
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
    doc.finish();
    assert_eq!(visible(&doc), format!("  {}", "x".repeat(1_001_000)));
    let doc = parse(&format!("   ```\n{} ```\n", "\tx\n".repeat(4096)));
    assert_eq!(doc.blocks().len(), 1);
    assert_eq!(visible(&doc), " x\n".repeat(4096));
}
