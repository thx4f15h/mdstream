use mdstream::*;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn visible(doc: &Document, source: Span) -> String {
    doc.display_runs_in(source)
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect()
}

fn whole(doc: &Document) -> String {
    visible(doc, Span::new(0, doc.source().len()))
}

#[test]
fn named_numeric_and_two_scalar_entities_preserve_original_source() {
    let source = "&amp; &AMP; &lt; &gt; &quot; &apos; &nbsp; &copy; &NotEqualTilde; &#35; &#x1F642; &#X4E2D; &#0; &#xD800; &#1114112; &#128;";
    let doc = parse(source);
    assert_eq!(
        whole(&doc),
        "& & < > \" ' \u{a0} © ≂\u{338} # 🙂 中 � � � \u{80}"
    );
    assert_eq!(doc.source(), source);
    let raw: String = doc
        .runs_in(Span::new(0, source.len()))
        .unwrap()
        .map(|r| r.text)
        .collect();
    assert_eq!(raw, source);
    for run in doc.display_runs_in(Span::new(0, source.len())).unwrap() {
        if let Some(original) = run.replacement {
            assert_eq!(original, run.source);
            assert!(doc.text(original).unwrap().starts_with('&'));
            assert!(run.text.len() <= 8);
            let selection = Selection::new(&doc, original).unwrap();
            assert_eq!(selection.text(&doc).unwrap(), &source[original.range()]);
        } else {
            assert_eq!(run.text.as_ptr(), doc.source()[run.source.range()].as_ptr());
        }
    }
}

#[test]
fn failed_incomplete_escaped_and_overlong_candidates_stay_literal() {
    for source in [
        "&amp",
        "&bogus;",
        "&Amp;",
        "&;",
        "&#;",
        "&#x;",
        "&#-1;",
        "&#x+41;",
        "&#12345678;",
        "&#x1234567;",
        "&abcdefghijklmnopqrstuvwxyzABCDEF;",
        "&中;",
    ] {
        assert_eq!(whole(&parse(source)), source, "{source}");
    }
    assert_eq!(
        whole(&parse(r"\&amp; &amp;amp; &&lt; &bad*ok*")),
        "&amp; &amp; &< &badok"
    );
    assert_eq!(whole(&parse("&#0000035; &#x000041;")), "# A");
    assert_eq!(whole(&parse("&CounterClockwiseContourIntegral;")), "∳");
}

#[test]
fn decoding_does_not_create_markdown_or_change_link_targets() {
    let doc = parse(
        "&#35; &#42;plain&#42; &NewLine; [a&amp;b](a&amp;b \"&quot;\") <https://a.test/?a&amp;b>",
    );
    assert_eq!(whole(&doc), "# *plain* \n a&b https://a.test/?a&amp;b");
    assert_eq!(doc.link_destination(LinkId(0)), Some("a&amp;b"));
    assert_eq!(doc.link_title(LinkId(0)), Some("&quot;"));
    assert_eq!(
        doc.link_destination(LinkId(1)),
        Some("https://a.test/?a&amp;b")
    );
    assert!(doc.headings().entries().is_empty());
    let linked = doc
        .display_runs_in(doc.link_info(LinkId(0)).unwrap().label)
        .unwrap()
        .collect::<Vec<_>>();
    assert!(linked.iter().all(|r| r.link == Some(LinkId(0))));
    assert!(
        linked
            .iter()
            .any(|r| r.text == "&" && r.replacement.is_some())
    );
    assert!(
        doc.display_runs_in(Span::new(0, doc.source().len()))
            .unwrap()
            .all(|r| !r.style.contains(Style::EMPHASIS))
    );
}

#[test]
fn code_whitespace_uses_normalized_endpoints_and_preserves_interior_spaces() {
    for (source, expected) in [
        ("x ` a `", "x a"),
        ("x `  a  `", "x  a "),
        ("x ` a`", "x  a"),
        ("x `a `", "x a "),
        ("x `   `", "x    "),
        ("x ` \n `", "x    "),
        ("x `\na\n`", "x a"),
        ("x `\r\na\r\n`", "x a"),
        ("x `a\r\nb\nc\rd`", "x a b c d"),
        ("x ` \t `", "x \t"),
        ("x `\u{a0}a\u{a0}`", "x \u{a0}a\u{a0}"),
        ("x `` ` ``", "x `"),
        ("x ` &amp; *a* `", "x &amp; *a*"),
        ("x ` \na", "x ` \na"),
        ("x `` \n`", "x `` \n`"),
    ] {
        assert_eq!(whole(&parse(source)), expected, "{source:?}");
    }
    let doc = parse("```text\n &amp; a  \nb\n```\n");
    assert_eq!(whole(&doc), " &amp; a  \nb\n");
    assert_eq!(whole(&parse("    &amp; a\n")), "&amp; a\n");
}

#[test]
fn replacements_cross_tiles_and_arbitrary_source_slices_without_duplication() {
    let source = format!(
        "{}&NotEqualTilde; `{}`",
        "x".repeat(TILE_BYTES - 2),
        "a\r\nb ".repeat(80)
    );
    let doc = parse(&source);
    let expected = whole(&doc);
    let tiled: String = (0..doc.tile_count())
        .flat_map(|i| doc.display_runs(TileId(i)).unwrap())
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    assert_eq!(tiled, expected);
    let mut scalar_slices = String::new();
    for (start, c) in source.char_indices() {
        let span = Span::new(start, start + c.len_utf8());
        let runs = doc.display_runs_in(span).unwrap().collect::<Vec<_>>();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].source, span);
        if let Some(original) = runs[0].replacement {
            assert!(original.start <= span.start && span.end <= original.end);
        }
        if !runs[0].style.contains(Style::HIDDEN) {
            scalar_slices.push_str(runs[0].text);
        }
    }
    assert_eq!(scalar_slices, expected);
    let entity_start = TILE_BYTES - 2;
    let continuation = doc
        .display_runs_in(Span::new(entity_start + 1, entity_start + 2))
        .unwrap()
        .next()
        .unwrap();
    assert_eq!(continuation.text, "");
    assert_eq!(
        continuation.replacement,
        Some(Span::new(
            entity_start,
            entity_start + "&NotEqualTilde;".len()
        ))
    );
}

#[test]
fn each_live_prefix_and_each_chunk_split_have_identical_display_mapping() {
    let prefix = "x".repeat(BLOCK_LOOKAHEAD);
    for suffix in [
        "&amp; &#x1f642; &NotEqualTilde; &bad*em*; &amp\n",
        " `  中\r\n🙂  ` &#35; `` ` `` `unfinished\n",
        "\n\n| a&amp;b | ` x ` |\n|---|---|\n| &#x41; | `a\\|b` |\n",
        "\n\ntext &copy; ` a `\n===\n",
    ] {
        let source = format!("{prefix}{suffix}");
        let expected = parse(&source);
        for split in suffix.char_indices().map(|(i, _)| prefix.len() + i) {
            let mut doc = Document::new();
            doc.append(&source[..split]).unwrap();
            doc.append(&source[split..]).unwrap();
            doc.finish();
            assert_eq!(
                doc.display_runs_in(Span::new(0, source.len()))
                    .unwrap()
                    .collect::<Vec<_>>(),
                expected
                    .display_runs_in(Span::new(0, source.len()))
                    .unwrap()
                    .collect::<Vec<_>>()
            );
        }
        let mut doc = Document::new();
        doc.append(&prefix).unwrap();
        for (offset, c) in suffix.char_indices() {
            doc.append(c.encode_utf8(&mut [0; 4])).unwrap();
            let end = prefix.len() + offset + c.len_utf8();
            let mut single = Document::new();
            single.append(&source[..end]).unwrap();
            assert_eq!(
                doc.display_runs_in(Span::new(0, end))
                    .unwrap()
                    .collect::<Vec<_>>(),
                single
                    .display_runs_in(Span::new(0, end))
                    .unwrap()
                    .collect::<Vec<_>>(),
                "{suffix:?} at {offset}"
            );
        }
    }
}

#[test]
fn late_entity_and_code_confirmation_invalidate_old_layout_even_with_fixed_styles() {
    for (pending, closing) in [("&amp", ";"), ("` a ", "` ")] {
        let mut doc = Document::new();
        let prefix = "x".repeat(BLOCK_LOOKAHEAD);
        let mut heights = HeightIndex::default();
        let policy = LayoutPolicy {
            style_affects_size: false,
            ..Default::default()
        };
        let update = doc.append(&format!("{prefix}{pending}")).unwrap();
        heights.apply_with_policy(&doc, &update, policy).unwrap();
        let tile = doc.tile_at(prefix.len()).unwrap();
        let old_key = doc.tile_key(tile).unwrap();
        let ticket = heights.ticket(tile).unwrap();
        heights.measure(ticket, 20).unwrap();
        let update = doc.append(closing).unwrap();
        assert_ne!(old_key, doc.tile_key(tile).unwrap());
        assert!(
            update
                .changes
                .iter()
                .any(|c| c.kind == ChangeKind::Text && c.source.start < update.appended.start)
        );
        heights.apply_with_policy(&doc, &update, policy).unwrap();
        assert!(heights.measure(ticket, 5).is_err());
    }
}

#[test]
fn budgeted_cursor_resumes_with_tiny_shared_budgets_and_checks_revision() {
    let doc = parse("中 &NotEqualTilde; ` a\r\n🙂 b ` &#x41;");
    let mut cursor = doc
        .display_cursor(Span::new(0, doc.source().len()))
        .unwrap();
    let limit = WorkLimit {
        bytes: 6,
        queries: 1,
        objects: 1,
        ..Default::default()
    };
    let mut output = String::new();
    while cursor.source_position() < doc.source().len() {
        let old = cursor.source_position();
        let mut budget = Budget::new(limit);
        match cursor.next(&doc, &mut budget).unwrap() {
            DisplayStep::Run(run) => {
                assert_eq!(run.source.start, old);
                if !run.style.contains(Style::HIDDEN) {
                    output.push_str(run.text);
                }
            }
            other => panic!("no progress: {other:?}"),
        }
        assert!(cursor.source_position() > old);
        assert!(budget.used().bytes <= limit.bytes);
        if cursor.source_position() < doc.source().len() {
            assert_eq!(
                cursor.next(&doc, &mut budget).unwrap(),
                DisplayStep::BudgetExhausted
            );
        }
    }
    assert_eq!(output, whole(&doc));
    assert_eq!(
        cursor
            .next(&doc, &mut Budget::new(WorkLimit::default()))
            .unwrap(),
        DisplayStep::Complete
    );
    let mut cursor = doc.display_cursor(Span::new(0, 3)).unwrap();
    for limit in [
        WorkLimit::default(),
        WorkLimit {
            bytes: 4,
            ..WorkLimit::unlimited()
        },
    ] {
        let mut budget = Budget::new(limit);
        assert_eq!(
            cursor.next(&doc, &mut budget).unwrap(),
            DisplayStep::BudgetExhausted
        );
        assert_eq!(cursor.source_position(), 0);
        assert_eq!(budget.used(), WorkLimit::default());
    }
    assert_eq!(
        cursor.next(&Document::new(), &mut Budget::new(limit)),
        Err(LayoutError::WrongDocument)
    );
    let mut growing = Document::new();
    growing.append("a").unwrap();
    let mut cursor = growing.display_cursor(Span::new(0, 1)).unwrap();
    growing.append("b").unwrap();
    assert_eq!(
        cursor.next(&growing, &mut Budget::new(limit)),
        Err(LayoutError::StaleContinuation)
    );
    assert!(doc.display_cursor(Span::new(1, 3)).is_none());
    assert!(doc.display_runs_in(Span::new(3, 1)).is_none());
}

#[test]
fn million_byte_code_closure_and_small_window_do_not_scan_old_body() {
    let mut doc = Document::new();
    doc.append(&format!("x ` {} ", "a".repeat(1_000_000)))
        .unwrap();
    let old = doc.work();
    doc.append("` ").unwrap();
    let after = doc.work();
    assert_eq!(after.inline_bytes - old.inline_bytes, 2);
    assert!(after.tree_visits - old.tree_visits < 500);
    let span = Span::new(500_000, 500_064);
    let mut runs = doc.display_runs_in(span).unwrap();
    let run = runs.next().unwrap();
    assert_eq!(run.text, "a".repeat(64));
    assert_eq!(run.replacement, None);
    assert!(runs.next().is_none());
    assert!(runs.annotation_visits() < 200);
    for size in [4096, 1_000_000] {
        let doc = parse(&format!("x `{}` &amp;", "a\r\nb ".repeat(size / 6)));
        let mut cursor = doc.display_cursor(Span::new(10, 50)).unwrap();
        let mut budget = Budget::new(WorkLimit {
            queries: 2,
            objects: 2,
            bytes: 20,
            ..Default::default()
        });
        while matches!(cursor.next(&doc, &mut budget).unwrap(), DisplayStep::Run(_)) {}
        assert!(budget.used().bytes <= 20);
        assert!(budget.used().queries <= 2);
        assert!(cursor.source_position() <= 50);
    }
}

#[test]
fn adversarial_failed_entities_remain_single_pass() {
    let source = format!(
        "{}{}{}",
        "&a".repeat(10_000),
        "&abcdefghijklmnopqrstuvwxzy0123456789;".repeat(1_000),
        "&#12345678;&unknown;".repeat(1_000)
    );
    let mut doc = Document::new();
    for chunk in source.as_bytes().chunks(71) {
        doc.append(std::str::from_utf8(chunk).unwrap()).unwrap();
    }
    doc.finish();
    assert_eq!(whole(&doc), source);
    assert_eq!(doc.work().inline_bytes, source.len() as u64);
    assert!(doc.work().entity_bytes <= source.len() as u64);
}
