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

fn codes(doc: &Document) -> Vec<String> {
    let mut result = Vec::new();
    let mut active = false;
    for run in doc
        .display_runs_in(Span::new(0, doc.source().len()))
        .unwrap()
    {
        if run.style.contains(Style::HIDDEN) {
            continue;
        }
        if run.style.contains(Style::CODE) {
            if !active {
                result.push(String::new());
            }
            result.last_mut().unwrap().push_str(run.text);
            active = true;
        } else {
            active = false;
        }
    }
    result
}

#[test]
fn failed_outer_recovers_codes_but_keeps_other_syntax_literal() {
    for (source, expected, spans) in [
        ("`foo``bar``\n", "`foobar\n", vec!["bar"]),
        ("a `x`` 中🙂 `` y", "a `x中🙂 y", vec!["中🙂"]),
        ("`x``  `` y", "`x   y", vec!["  "]),
        ("`x`` a\r\nb `` y", "`xa b y", vec!["a b"]),
        ("`x``a```b`` y", "`xa```b y", vec!["a```b"]),
        ("`x```a``b`` y", "`x```ab y", vec!["b"]),
        ("`x``a`` y```b``` z", "`xa yb z", vec!["a", "b"]),
        ("`*x* &amp; ``a`` $y$", "`*x* &amp; a $y$", vec!["a"]),
        ("`x``a``y`", "x``a``y", vec!["x``a``y"]),
        ("`x``a``y` `z", "x``a``y `z", vec!["x``a``y"]),
        ("`x\\``a``", "`x\\a", vec!["a"]),
        ("> `x``a\n> b`` y\n", "`xa b y\n", vec!["a b"]),
        ("- `x``a`` y\n", "`xa y\n", vec!["a"]),
        ("# `x``a`` y\n", "`xa y\n", vec!["a"]),
        ("`x``a`` y\n===\n", "`xa y\n", vec!["a"]),
        ("a|b\n-| -\n`x``a``|c\n", "ab\n`xac\n", vec!["a"]),
    ] {
        let doc = parse(source);
        assert_eq!(visible(&doc), expected, "{source:?}");
        assert_eq!(codes(&doc), spans, "{source:?}");
        assert!(doc.links().is_empty());
        assert!(doc.maths().is_empty());
    }
}

#[test]
fn late_header_overflow_and_crossing_pairs_do_not_leak() {
    let long = "x".repeat(BLOCK_LOOKAHEAD + 7);
    for row in [
        format!("{long}{} | c", "``b".repeat(40)),
        format!("{long}` | c"),
    ] {
        let doc = parse(&format!("`x``a``\n{row}\n- | -\n"));
        assert_eq!(doc.tables().len(), 1);
        assert_eq!(codes(&doc).first().map(String::as_str), Some("a"));
    }
    let doc = parse(&format!("`x``a\n{long}`` | c\n- | -\n"));
    assert_eq!(doc.tables().len(), 1);
    assert!(codes(&doc).is_empty());
    let doc = parse(&format!("`x``a\n{long}`` | c\nfailed\n"));
    assert!(doc.tables().is_empty());
    assert_eq!(codes(&doc), [format!("a {long}")]);
}

#[test]
fn fallback_mapping_tiles_and_shared_budget_remain_exact() {
    // Later virtual-space mappings must not change an earlier pair's trim.
    let tabs = parse("- `x`` a\n\t`` y\n\tz\n");
    let spaces = parse("- `x`` a\n    `` y\n    z\n");
    assert_eq!(visible(&tabs), visible(&spaces));
    assert_eq!(codes(&tabs), ["a  "]);
    let source = format!("`x`` {}中🙂\r\n b `` tail", "a".repeat(300));
    let doc = parse(&source);
    let expected = format!("`x{}中🙂 b tail", "a".repeat(300));
    let tiled: String = (0..doc.tile_count())
        .flat_map(|i| doc.display_runs(TileId(i)).unwrap())
        .filter(|run| !run.style.contains(Style::HIDDEN))
        .map(|run| run.text)
        .collect();
    assert_eq!(tiled, expected);
    let mut cursor = doc.display_cursor(Span::new(0, source.len())).unwrap();
    let mut output = String::new();
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
                assert!(budget.used().bytes <= 6);
                assert_eq!(run.source.start, position);
                if let Some(original) = run.replacement {
                    assert!(original.start <= run.source.start && run.source.end <= original.end);
                    assert!(matches!(&source[original.range()], " " | "\r\n"));
                    assert!(run.style.contains(Style::CODE));
                } else {
                    assert_eq!(run.text, &source[run.source.range()]);
                }
                if !run.style.contains(Style::HIDDEN) {
                    output.push_str(run.text);
                }
            }
            DisplayStep::Complete => break,
            other => panic!("six bytes must advance: {other:?}"),
        }
    }
    assert_eq!(output, expected);
}

#[test]
fn million_byte_candidate_and_bounded_matching_do_not_rescan() {
    let mut doc = Document::new();
    doc.append(&format!(
        "{} `x``{}",
        "> ".repeat(1000),
        "a".repeat(1_000_000)
    ))
    .unwrap();
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("b").unwrap();
    }
    let grown = doc.work();
    assert_eq!(grown.inline_bytes - before.inline_bytes, 1000);
    assert_eq!(grown.classified_bytes, before.classified_bytes);
    assert_eq!(grown.container_bytes, before.container_bytes);
    assert_eq!(grown.tree_visits, before.tree_visits);
    assert_eq!(grown.delimiter_steps, before.delimiter_steps);
    doc.append("`` ").unwrap();
    let before_finish = doc.work();
    doc.finish();
    let sealed = doc.work();
    assert_eq!(sealed.inline_bytes, before_finish.inline_bytes);
    assert_eq!(sealed.classified_bytes, before_finish.classified_bytes);
    assert_eq!(sealed.container_bytes, before_finish.container_bytes);
    assert_eq!(sealed.delimiter_steps - before_finish.delimiter_steps, 1);
    assert!(sealed.tree_visits - before_finish.tree_visits < 1000);
    assert_eq!(
        codes(&doc),
        [format!("{}{}", "a".repeat(1_000_000), "b".repeat(1000))]
    );

    let mut source = format!("`q{}", "x".repeat(BLOCK_LOOKAHEAD));
    for len in 2..=33 {
        source.push_str(&format!("{}x", "`".repeat(len)));
    }
    let mut doc = Document::new();
    doc.append(&source).unwrap();
    let before = doc.work();
    doc.finish();
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert_eq!(
        doc.work().delimiter_steps - before.delimiter_steps,
        32 * 31 / 2
    );
    assert_eq!(visible(&doc), source);

    let source = "`x``a``\n\n".repeat(4096);
    let doc = parse(&source);
    assert_eq!(codes(&doc).len(), 4096);
    assert!(doc.work().inline_bytes <= source.len() as u64);
    assert!(doc.work().delimiter_steps < 8 * 4096);
}

#[test]
fn open_candidate_is_literal_and_finish_invalidates_old_queries() {
    let source = "`foo``bar`` ";
    let mut doc = Document::new();
    doc.append(source).unwrap();
    assert_eq!(visible(&doc), source);
    assert!(codes(&doc).is_empty());
    let key = doc.tile_key(TileId(0)).unwrap();
    let mut cursor = doc.display_cursor(Span::new(0, source.len())).unwrap();
    let update = doc.finish();
    assert_eq!(visible(&doc), "`foobar");
    assert_ne!(key, doc.tile_key(TileId(0)).unwrap());
    assert!(
        update
            .dirty
            .iter()
            .any(|span| span.start <= 6 && span.end >= 9)
    );
    assert_eq!(
        cursor.next(&doc, &mut Budget::new(WorkLimit::unlimited())),
        Err(LayoutError::StaleContinuation)
    );
    let before = support::snapshot(&doc);
    doc.finish();
    assert_eq!(support::snapshot(&doc), before);
}

#[test]
fn every_split_prefix_and_header_resolution_preserves_identity() {
    let long = "x".repeat(BLOCK_LOOKAHEAD + 7);
    let mut inputs = vec![
        "`foo``bar``\n".to_owned(),
        "> `x`` 中🙂\r\n> b `` y\n".to_owned(),
        "- `x`` a\n\t`` y\n\tz\n".to_owned(),
        "a `x``a`` y```b``` z\n\nnew\n".to_owned(),
        "- `x``a``\n- ``b``\n".to_owned(),
        "`x``a``\n===\n".to_owned(),
        "a|b\n-|-\n`x``a``|c\n".to_owned(),
    ];
    for row in [
        "b | c".to_owned(),
        format!("{long} | c"),
        format!("{long}` | c"),
        format!("{long}``b`` | c"),
    ] {
        for suffix in ["- | -\n", "not a delimiter\n"] {
            let source = format!("`x``a``\n{row}\n{suffix}");
            let doc = parse(&source);
            assert_eq!(doc.tables().len(), usize::from(suffix == "- | -\n"));
            if !row.contains('`') || suffix == "- | -\n" {
                assert_eq!(
                    codes(&doc).first().map(String::as_str),
                    Some("a"),
                    "{source:?}"
                );
            }
            inputs.push(source);
        }
    }
    for source in inputs {
        let whole = support::snapshot(&parse(&source));
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
            let mut prefix = Document::new();
            prefix.append(&source[..at + ch.len_utf8()]).unwrap();
            assert_eq!(doc.pending(), prefix.pending());
            assert_eq!(
                support::snapshot(&doc),
                support::snapshot(&prefix),
                "prefix {at}: {source:?}"
            );
        }
        doc.finish();
        assert_eq!(support::snapshot(&doc), whole);
    }
}

#[test]
fn bounded_run_model_and_overflow_keep_outer_precedence() {
    // Independent greedy oracle: the first later equal run closes each opener;
    // unmatched runs remain literal. A primary length of 1 never occurs inside.
    for count in 0..=7 {
        for mask in 0..1usize << count {
            let lengths: Vec<_> = (0..count).map(|i| 2 + ((mask >> i) & 1)).collect();
            let mut source = "`q".to_owned();
            for len in &lengths {
                source.push_str(&"`".repeat(*len));
                source.push('x');
            }
            let mut expected = "`q".to_owned();
            let mut i = 0;
            while i < count {
                if let Some(j) = (i + 1..count).find(|&j| lengths[j] == lengths[i]) {
                    expected.push('x');
                    for len in &lengths[i + 1..j] {
                        expected.push_str(&"`".repeat(*len));
                        expected.push('x');
                    }
                    expected.push('x');
                    i = j + 1;
                } else {
                    expected.push_str(&"`".repeat(lengths[i]));
                    expected.push('x');
                    i += 1;
                }
            }
            assert_eq!(visible(&parse(&source)), expected, "{source:?}");
        }
    }
    for runs in [31, 32, 33, 1000] {
        let source = format!("`q{}", "``x".repeat(runs));
        let doc = parse(&source);
        if runs <= 32 {
            assert_eq!(codes(&doc).len(), runs / 2);
        } else {
            assert_eq!(visible(&doc), source);
            assert!(codes(&doc).is_empty());
        }
        assert_eq!(visible(&parse(&format!("{source}`"))), source[1..]);
    }
}
