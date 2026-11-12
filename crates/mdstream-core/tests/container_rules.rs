use mdstream::*;

#[path = "support/snapshot.rs"]
mod support;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn rules(doc: &Document) -> Vec<&Block> {
    doc.blocks()
        .iter()
        .filter(|block| block.kind == BlockKind::ThematicBreak)
        .collect()
}

#[test]
fn container_rule_model_covers_markers_spacing_and_failures() {
    let mut cases = 0;
    // `+` cannot merge with the payload into an outer thematic break. The
    // separate precedence test covers `- --` and the enclosing scope instead.
    for prefix in ["> ", "+ ", "> + ", "- > ", "1. ", "> > "] {
        for marker in ['*', '-', '_'] {
            for separator in ["", " ", "\t"] {
                for count in 1..=6 {
                    for suffix in ["", " x"] {
                        let text = vec![marker.to_string(); count].join(separator);
                        let source = format!("{prefix}{text}{suffix} \t\r\n");
                        let doc = parse(&source);
                        let expected = usize::from(count >= 3 && suffix.is_empty());
                        assert_eq!(rules(&doc).len(), expected, "{source:?}");
                        if expected == 1 {
                            let block = rules(&doc)[0];
                            assert_eq!(block.source, Span::new(0, source.len()));
                            assert!(block.container.is_some());
                            assert_eq!(block.state, SyntaxState::Sealed);
                        }
                        cases += 1;
                    }
                }
            }
        }
    }
    assert_eq!(cases, 648);
}

#[test]
fn rules_preserve_scope_precedence_and_opaque_code() {
    for (source, rule_count, containers, level, code_count) in [
        ("- Foo\n- * * *\n", 1, 3, None, 0),
        ("> - a\n> - - -\n", 1, 3, None, 0),
        ("> - a\n>   ***\n", 1, 3, None, 0),
        ("> a\n> ---\n", 0, 1, Some(2), 0),
        ("> a\n> - - -\n", 1, 1, None, 0),
        ("> a\n> ***\n", 1, 1, None, 0),
        ("> a\n***\n", 1, 1, None, 0),
        ("> ```\n> ***\n> ```\n", 0, 1, None, 1),
        (">     ***\n", 0, 1, None, 1),
        (">    ***\n", 1, 1, None, 0),
        ("> **x**\n", 0, 1, None, 0),
        ("> * * x\n", 0, 5, None, 0),
        ("- --\n", 1, 0, None, 0),
        ("> - --\n", 1, 1, None, 0),
        ("> - **\n", 0, 3, None, 0),
        ("> ---x***\n", 0, 1, None, 0),
    ] {
        let doc = parse(source);
        assert_eq!(rules(&doc).len(), rule_count, "{source:?}");
        assert_eq!(doc.container_count(), containers, "{source:?}");
        assert_eq!(doc.headings().entries().first().map(|h| h.level), level);
        assert_eq!(
            doc.blocks()
                .iter()
                .filter(|b| matches!(b.kind, BlockKind::Code { .. }))
                .count(),
            code_count,
            "{source:?}"
        );
    }
    let doc = parse("- Foo\n- * * *\n");
    let first_item = doc.blocks()[0].container.unwrap();
    let second_item = rules(&doc)[0].container.unwrap();
    assert_ne!(first_item, second_item);
    assert_eq!(
        doc.container(first_item).unwrap().parent,
        doc.container(second_item).unwrap().parent
    );
    let doc = parse("> - a\n> - - -\n");
    let scope = rules(&doc)[0].container.unwrap();
    assert_eq!(doc.container(scope).unwrap().kind, ContainerKind::Quote);
    let doc = parse("> a | b\n> - | -\n> ***\n");
    assert_eq!(doc.tables().len(), 1);
    assert_eq!(rules(&doc).len(), 1);
    assert_eq!(
        doc.blocks()[doc.tables()[0].block.0].container,
        rules(&doc)[0].container
    );
}

#[test]
fn container_rules_all_splits_prefixes_and_eof() {
    for source in [
        "- Foo\n- * * *\n",
        "> - a\n> - - -\n",
        "> - a\n>   ***\n",
        "> a\n> ---\n",
        "> a\n> - - -\n",
        "> a\n***\n",
        "> ***",
        "> **",
        "> * * * x\n",
        "> ___ \t\r\n",
        "> a | b\n> - | -\n> ***\n",
        "> ```\n> ***\n> ```\n",
        ">     ***\n",
        ">    ***\n",
        "- > 中🙂 \t\n  > ***\n  > next\n",
        "> - * * *\n> - next\n",
        "> - a\n>\n> - - -\n",
    ] {
        let whole = support::snapshot(&parse(source));
        for split in (0..=source.len()).filter(|&at| source.is_char_boundary(at)) {
            let mut doc = Document::new();
            doc.append(&source[..split]).unwrap();
            doc.append(&source[split..]).unwrap();
            doc.finish();
            assert_eq!(support::snapshot(&doc), whole, "{source:?} split {split}");
        }
        let mut doc = Document::new();
        for (at, ch) in source.char_indices() {
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            let mut prefix = Document::new();
            prefix.append(&source[..at + ch.len_utf8()]).unwrap();
            assert_eq!(support::snapshot(&doc), support::snapshot(&prefix));
            assert_eq!(doc.pending(), prefix.pending());
        }
        doc.finish();
        assert_eq!(support::snapshot(&doc), whole);
    }
}

#[test]
fn rule_confirmation_closes_inline_and_invalidates_provisional_display() {
    let mut doc = Document::new();
    doc.append("> [中](u) \t\n> **").unwrap();
    let links = doc.links().to_vec();
    let first = doc.blocks()[0].clone();
    let key = doc.tile_key(TileId(0)).unwrap();
    let mut cursor = doc
        .display_cursor(Span::new(0, doc.source().len()))
        .unwrap();
    assert!(rules(&doc).is_empty());
    doc.append("*").unwrap();
    assert!(rules(&doc).is_empty());
    let update = doc.finish();
    assert_eq!(rules(&doc).len(), 1);
    assert_eq!(doc.links(), links);
    assert_eq!(doc.blocks()[0].container, first.container);
    assert_eq!(doc.blocks()[0].source, first.source);
    assert!(!update.dirty.is_empty());
    assert_ne!(doc.tile_key(TileId(0)).unwrap(), key);
    assert_eq!(
        cursor.next(&doc, &mut Budget::new(WorkLimit::unlimited())),
        Err(LayoutError::StaleContinuation)
    );
    let visible: String = doc
        .display_runs_in(Span::new(0, doc.source().len()))
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    assert_eq!(visible, "中\n***");
    assert!(doc.finish().dirty.is_empty());
}

#[test]
fn rule_prefix_mapping_and_shared_budget_progress() {
    let source = "> - 中🙂 \t\n>   * * * \t\r\n>   next\n";
    let doc = parse(source);
    assert_eq!(rules(&doc).len(), 1);
    assert_eq!(rules(&doc)[0].container, doc.blocks()[0].container);
    let mut cursor = doc.display_cursor(Span::new(0, source.len())).unwrap();
    let mut visible = String::new();
    loop {
        let position = cursor.source_position();
        if position < source.len() {
            assert_eq!(
                cursor
                    .next(&doc, &mut Budget::new(WorkLimit::default()))
                    .unwrap(),
                DisplayStep::BudgetExhausted
            );
            assert_eq!(cursor.source_position(), position);
        }
        let mut budget = Budget::new(WorkLimit {
            bytes: 6,
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        match cursor.next(&doc, &mut budget).unwrap() {
            DisplayStep::Run(run) => {
                assert_eq!(run.replacement, None);
                assert_eq!(run.text, &source[run.source.range()]);
                assert!(budget.used().bytes <= 6);
                if !run.style.contains(Style::HIDDEN) {
                    visible.push_str(run.text);
                }
            }
            DisplayStep::Complete => break,
            other => panic!("six bytes must advance: {other:?}"),
        }
    }
    assert_eq!(visible, "中🙂\n* * * \t\nnext\n");
}

#[test]
fn bounded_rule_suffix_and_long_failure_never_rescan_old_input() {
    for depth in [1, 32, 128, 400] {
        for tail in ["***\n", "* * * x\n"] {
            let source = format!("{}{tail}", "> ".repeat(depth));
            let doc = parse(&source);
            assert_eq!(rules(&doc).len(), usize::from(tail == "***\n"));
            assert!(doc.work().container_bytes < source.len() as u64 * 30);
            assert!(doc.work().container_steps < source.len() as u64 * 10);
        }
        let source = format!("> {}x\n", "* ".repeat(depth));
        let doc = parse(&source);
        assert!(rules(&doc).is_empty());
        assert!(doc.work().container_bytes < source.len() as u64 * 30);
        let source = format!("> {}* * *\n", "+ ".repeat(depth));
        let doc = parse(&source);
        assert_eq!(rules(&doc).len(), 1);
        assert_eq!(doc.container_count(), depth * 2 + 1);
        assert!(doc.work().container_bytes < source.len() as u64 * 30);
    }
    let mut doc = Document::new();
    doc.append(&format!("> {}", "* ".repeat(500_000))).unwrap();
    assert!(rules(&doc).is_empty());
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("*").unwrap();
    }
    let grown = doc.work();
    assert_eq!(grown.classified_bytes, before.classified_bytes);
    assert_eq!(grown.container_bytes, before.container_bytes);
    assert_eq!(grown.container_steps, before.container_steps);
    doc.finish();
    assert_eq!(rules(&doc).len(), 1);
    assert!(doc.work().container_bytes - grown.container_bytes < 30 * doc.source().len() as u64);
    let mut doc = Document::new();
    for _ in 0..4096 {
        doc.append("> ***\n").unwrap();
    }
    doc.finish();
    assert_eq!(rules(&doc).len(), 4096);
    assert_eq!(doc.container_count(), 1);
    assert!(doc.work().container_bytes < doc.source().len() as u64 * 30);
}

#[test]
fn rules_require_physical_line_completion_without_a_window_limit() {
    for end in ["", "\n", "\r\n"] {
        for len in [BLOCK_LOOKAHEAD - 1, BLOCK_LOOKAHEAD, BLOCK_LOOKAHEAD + 1] {
            let source = format!("> {}{end}", "*".repeat(len - 2 - end.len()));
            let expected = 1;
            let whole = parse(&source);
            assert_eq!(rules(&whole).len(), expected, "length {len}, end {end:?}");
            for split in [1, 2, len - 2, len - 1, len] {
                let mut doc = Document::new();
                doc.append(&source[..split]).unwrap();
                doc.append(&source[split..]).unwrap();
                doc.finish();
                assert_eq!(support::snapshot(&doc), support::snapshot(&whole));
            }
        }
    }
}
