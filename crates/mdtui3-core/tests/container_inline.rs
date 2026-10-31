use mdtui3::*;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn visible(doc: &Document, span: Span) -> String {
    doc.display_runs_in(span)
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect()
}

fn whole(doc: &Document) -> String {
    visible(doc, Span::new(0, doc.source().len()))
}

fn style(doc: &Document, at: usize) -> Style {
    let end = at + doc.source()[at..].chars().next().unwrap().len_utf8();
    doc.runs_in(Span::new(at, end))
        .unwrap()
        .next()
        .unwrap()
        .style
}

fn breaks(doc: &Document) -> Vec<(usize, bool)> {
    doc.runs_in(Span::new(0, doc.source().len()))
        .unwrap()
        .filter_map(|r| {
            let hard = r.style.contains(Style::HARD_BREAK);
            (hard || r.style.contains(Style::SOFT_BREAK)).then_some((r.source.start, hard))
        })
        .collect()
}

#[test]
fn explicit_continuations_share_delimiters_but_keep_physical_leaf_ids() {
    for (first, next) in [
        ("> ", "> "),
        ("- [x] ", "  "),
        ("> - ", ">   "),
        ("12) ", "    "),
    ] {
        let source = format!("{first}**中 _one\n{next}two_ ~~three\n{next}four~~ end**\n");
        let doc = parse(&source);
        assert_eq!(doc.blocks().len(), 3);
        assert!(
            doc.blocks()
                .iter()
                .all(|b| b.container == doc.blocks()[0].container)
        );
        assert!(doc.blocks().iter().all(|b| b.state == SyntaxState::Sealed));
        assert_eq!(whole(&doc), "中 one\ntwo three\nfour end\n");
        for word in ["中", "one", "two", "three", "four", "end"] {
            assert!(style(&doc, source.find(word).unwrap()).contains(Style::STRONG));
        }
        for word in ["one", "two"] {
            assert!(style(&doc, source.find(word).unwrap()).contains(Style::EMPHASIS));
        }
        for word in ["three", "four"] {
            assert!(style(&doc, source.find(word).unwrap()).contains(Style::STRIKE));
        }
        assert_eq!(breaks(&doc).len(), 2);
        let mut start = 0;
        for (i, line) in source.split_inclusive('\n').enumerate() {
            assert_eq!(doc.blocks()[i].source, Span::new(start, start + line.len()));
            assert_eq!(doc.block_at(start), Some(BlockId(i)));
            start += line.len();
        }
    }
}

#[test]
fn container_switches_and_explicit_blank_lines_discard_old_openers() {
    for source in [
        "> *a\n\nout*\n",
        "*root\n> end*\n",
        "> *a\n> > b*\n>\n> c*\n",
        "> > *a\n>\n> b*\n> > c*\n",
        "- *a\n- b*\n",
        "1. *a\n2. b*\n",
        "- *a\n  - b*\n  \n  c*\n",
        "> *a\n>\n> b*\n",
        "- *a\n  \n  b*\n",
        "> *a\n\n> b*\n",
        "> - *a\n> - b*\n",
    ] {
        let doc = parse(source);
        assert!(
            doc.runs_in(Span::new(0, source.len()))
                .unwrap()
                .all(|r| !r.style.contains(Style::EMPHASIS)),
            "{source:?}"
        );
        assert!(breaks(&doc).is_empty(), "{source:?}");
    }
    let doc = parse("> > **a\n>> b**\n");
    assert_eq!(whole(&doc), "a\nb\n"); // Equivalent spelling, same exact ID.
}

#[test]
fn block_interruptions_end_inline_sequences_including_opaque_candidates() {
    for middle in [
        "> # heading\n",
        "> ```\n> raw\n> ```\n",
        ">     raw\n",
        "> [r]: /url\n",
        "> [^n]: raw\n",
        "> a|b\n> -|-\n>\n",
    ] {
        for (open, close, forbidden) in [
            ("*open", "end*", Style::EMPHASIS),
            ("`open", "end`", Style::CODE),
        ] {
            let source = format!("> {open}\n{middle}> {close}\n");
            let doc = parse(&source);
            let at = source.find("open").unwrap();
            assert!(!style(&doc, at).contains(forbidden), "{source:?}");
            assert!(
                !style(&doc, source.rfind("end").unwrap()).contains(forbidden),
                "{source:?}"
            );
            assert!(
                !breaks(&doc)
                    .iter()
                    .any(|(at, _)| *at == source.find('\n').unwrap())
            );
        }
    }
}

#[test]
fn soft_and_hard_breaks_wait_for_a_classified_same_scope_payload() {
    for (marker, hard, visible_marker) in [
        ("", false, ""),
        (" ", false, ""),
        ("  ", true, ""),
        ("\\", true, ""),
        ("\\\\", false, "\\"),
    ] {
        for eol in ["\n", "\r\n"] {
            let mut doc = Document::new();
            let source = format!("> a{marker}{eol}");
            doc.append(&source).unwrap();
            let newline = source.len() - 1;
            let key = doc.tile_key(TileId(0)).unwrap();
            assert!(breaks(&doc).is_empty());
            doc.append("> b").unwrap();
            assert!(breaks(&doc).is_empty());
            let update = doc.append(eol).unwrap();
            assert_eq!(breaks(&doc), [(newline, hard)]);
            assert_ne!(doc.tile_key(TileId(0)).unwrap(), key);
            assert!(
                update
                    .dirty
                    .iter()
                    .any(|s| s.start <= newline && newline < s.end)
            );
            assert_eq!(whole(&doc), format!("a{visible_marker}\nb\n"));
            doc.finish();
            assert_eq!(breaks(&doc), [(newline, hard)]);
        }
    }
    for tail in ["", ">\n", "> # h\n", "# outside\n", "> - sibling\n"] {
        let doc = parse(&format!("> a\\\n{tail}"));
        assert!(breaks(&doc).is_empty());
        assert_eq!(visible(&doc, doc.blocks()[0].source), "a\\\n");
    }
}

#[test]
fn code_normalization_uses_payload_endpoints_and_keeps_exact_mapping() {
    for (source, expected) in [
        ("> `a\n> b`\n", "a b\n"),
        ("> ` a\n> `\n", "a\n"),
        ("> ` a\r\n> `\r\n", "a\n"),
        ("> `\r\n> 中🙂\r\n> `\n", "中🙂\n"),
        ("> `  a\n>  `\n", " a \n"),
        ("> ` \n>  `\n", "   \n"),
        ("> `` a\n> ` b ``\n", "a ` b\n"),
        (
            "> `&amp; *a*\n> [x](url) $m$`\n",
            "&amp; *a* [x](url) $m$\n",
        ),
        ("- ` a\r\n  `", "a"),
    ] {
        let doc = parse(source);
        assert_eq!(whole(&doc), expected, "{source:?}");
        assert!(breaks(&doc).is_empty());
        assert!(doc.links().is_empty());
        assert!(doc.maths().is_empty());
        let mut sliced = String::new();
        for (start, c) in source.char_indices() {
            let span = Span::new(start, start + c.len_utf8());
            for run in doc.display_runs_in(span).unwrap() {
                if let Some(replacement) = run.replacement {
                    assert!(
                        replacement.start <= run.source.start && run.source.end <= replacement.end
                    );
                }
                if !run.style.contains(Style::HIDDEN) {
                    sliced.push_str(run.text);
                }
            }
        }
        assert_eq!(sliced, expected);
    }
}

#[test]
fn failed_table_candidates_continue_once_but_confirmed_tables_isolate_cells() {
    for source in [
        "> *one\n> two|three\n> four*\n",
        "> *one|two\n> three*\n",
        "> *one\n> two*|three",
    ] {
        let doc = parse(source);
        assert!(doc.tables().is_empty());
        assert!(style(&doc, source.find("one").unwrap()).contains(Style::EMPHASIS));
        assert_eq!(
            doc.work().inline_bytes as usize,
            source.len() - 2 * doc.blocks().len()
        );
    }
    let doc = parse("> *one\n> two*|three\n> -|-\n> *cell|other*\n");
    assert_eq!(doc.tables().len(), 1);
    assert!(
        doc.runs_in(Span::new(0, doc.source().len()))
            .unwrap()
            .all(|r| !r.style.contains(Style::EMPHASIS))
    );
    assert!(breaks(&doc).is_empty());
    // A rejected header can itself precede a confirmed header.
    let doc = parse("> *a|b\n> c*|d\n> -|-\n");
    assert_eq!(visible(&doc, doc.blocks()[0].source), "*a|b\n");
    assert_eq!(doc.tables()[0].block, BlockId(1));
}

#[test]
fn multiline_setext_promotion_preserves_committed_inline_semantics_then_closes() {
    let mut doc = Document::new();
    doc.append("> *first\n> last*\n").unwrap();
    let before = doc.work();
    let update = doc.append("> ---\n").unwrap();
    assert_eq!(update.new_headings, 0..1);
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    let heading = &doc.headings().entries()[0];
    assert_eq!(heading.block, BlockId(0));
    assert_eq!(doc.text(heading.content), Some("*first\n> last*"));
    assert_eq!(visible(&doc, heading.content), "first\nlast");
    assert!(style(&doc, 3).contains(Style::EMPHASIS));
    assert_eq!(breaks(&doc).len(), 1); // Already confirmed before promotion.
    let doc = parse("> *first\n> last\n> ---\n> end*\n");
    assert!(
        doc.runs_in(Span::new(0, doc.source().len()))
            .unwrap()
            .all(|r| !r.style.contains(Style::EMPHASIS))
    );
    let doc = parse("> *one\n> two*|three\n> ---\n");
    assert_eq!(doc.headings().entries().len(), 1);
    assert_eq!(whole(&doc), "one\ntwo|three\n");
}

#[test]
fn labels_and_math_continue_while_footnotes_keep_physical_boundaries() {
    let doc = parse(
        "> [cross\n> label](url)\n> ![image\n> label](img)\n> $*opaque\n> tail$\n> [^open\n> note]\n",
    );
    assert_eq!(doc.links().len(), 2);
    assert_eq!(doc.link_destination(LinkId(0)), Some("url"));
    assert_eq!(doc.link_destination(LinkId(1)), Some("img"));
    assert_eq!(doc.maths().len(), 1);
    assert_eq!(doc.text(doc.maths()[0].content), Some("*opaque\n> tail"));
    assert!(doc.footnote_references().is_empty());
    assert!(
        doc.runs_in(Span::new(0, doc.source().len()))
            .unwrap()
            .all(|r| !r.style.contains(Style::EMPHASIS))
    );
    let doc = parse("> *[bad](unclosed\n> plain* [ok][r] $x$ [^n]\n> [r]: /late\n> [^n]: raw\n");
    assert_eq!(doc.links().len(), 2);
    assert_eq!(doc.link_destination(LinkId(0)), None);
    assert_eq!(doc.link_destination(LinkId(1)), Some("/late"));
    assert!(style(&doc, doc.source().find("plain").unwrap()).contains(Style::EMPHASIS));
    assert_eq!(doc.maths().len(), 1);
    assert_eq!(doc.footnote_references().len(), 1);
    for suffix in ["$x$", "[ok](url)", "[^n]"] {
        let doc = parse(&format!("> {suffix}"));
        assert_eq!(
            whole(&doc),
            match suffix {
                "$x$" => "x",
                "[ok](url)" => "ok",
                _ => "[^n]",
            }
        );
    }
}

#[test]
fn million_byte_continuations_and_deep_prefixes_never_rescan_body_or_ancestors() {
    for delimiter in ["*", "`"] {
        let prefix = ">".repeat(1000);
        let mut doc = Document::new();
        doc.append(&format!("{prefix}{delimiter}{}", "x".repeat(1_000_000)))
            .unwrap();
        let before = doc.work();
        for _ in 0..1000 {
            doc.append("x").unwrap();
        }
        let after = doc.work();
        assert_eq!(after.inline_bytes - before.inline_bytes, 1000);
        assert_eq!(after.classified_bytes, before.classified_bytes);
        assert_eq!(after.container_steps, before.container_steps);
        assert_eq!(after.container_bytes, before.container_bytes);
        let before = doc.work();
        doc.append(&format!("\n{prefix}end{delimiter}\n")).unwrap();
        let after = doc.work();
        assert_eq!(after.inline_bytes - before.inline_bytes, 6);
        assert!(after.classified_bytes - before.classified_bytes <= 1100);
        assert!(after.container_steps - before.container_steps <= 1001);
        assert!(after.delimiter_steps - before.delimiter_steps <= 3);
        assert!(after.tree_visits - before.tree_visits < 1500);
        assert!(style(&doc, 1001).contains(if delimiter == "*" {
            Style::EMPHASIS
        } else {
            Style::CODE
        }));
        let before = doc.work();
        doc.finish();
        assert_eq!(doc.work().inline_bytes, before.inline_bytes);
        assert_eq!(doc.container_count(), 1000);
    }
}

#[test]
fn long_continuation_classification_and_failed_prefixes_are_never_retried() {
    let mut doc = Document::new();
    doc.append("> *start\n").unwrap();
    doc.append(&format!("> {}", "中".repeat(500))).unwrap();
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("🙂").unwrap();
    }
    let after = doc.work();
    assert_eq!(after.inline_bytes - before.inline_bytes, 4000);
    assert_eq!(after.classified_bytes, before.classified_bytes);
    assert_eq!(after.container_bytes, before.container_bytes);
    doc.append("*\r\n").unwrap();
    assert!(style(&doc, 3).contains(Style::EMPHASIS));
    assert_eq!(breaks(&doc).len(), 1);
    let doc = parse(&format!("> *start\n> [r]: /{}*\n", "x".repeat(2000)));
    assert!(doc.references().iter().all(|r| r.destination.is_none()));
    assert!(style(&doc, 3).contains(Style::EMPHASIS));
    let doc = parse(&format!("> `start\n> {}\n> end`\n", "x|".repeat(600)));
    assert!(doc.tables().is_empty());
    assert!(style(&doc, 3).contains(Style::CODE));
}

#[test]
fn many_open_delimiters_and_deep_list_continuations_have_linear_lifetimes() {
    let depth = 250;
    let mut doc = Document::new();
    doc.append(&format!("{}*first\n", "- ".repeat(depth)))
        .unwrap();
    let next = format!("{}*more\n", " ".repeat(2 * depth));
    for _ in 0..240 {
        doc.append(&next).unwrap();
    }
    assert_eq!(doc.container_count(), 2 * depth);
    let before = doc.work();
    doc.append(&format!("{}end*\n", " ".repeat(2 * depth)))
        .unwrap();
    assert_eq!(
        doc.work().container_steps - before.container_steps,
        depth as u64 // One active frame owns each List/Item pair.
    );
    assert!(doc.work().container_bytes - before.container_bytes < next.len() as u64 * 5);
    let before = doc.work();
    doc.finish();
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert!(doc.work().delimiter_steps - before.delimiter_steps <= 241);
    assert!(doc.work().delimiter_steps <= 3 * 242);

    let mut doc = Document::new();
    for _ in 0..4096 {
        doc.append("> *old\n").unwrap();
    }
    doc.append("> `tail").unwrap(); // Bounded pending line, then a live long line.
    doc.append(&"x".repeat(BLOCK_LOOKAHEAD)).unwrap();
    let before = doc.work();
    doc.append("x").unwrap();
    let after = doc.work();
    assert_eq!(after.inline_bytes - before.inline_bytes, 1);
    assert_eq!(after.classified_bytes, before.classified_bytes);
    assert_eq!(after.delimiter_steps, before.delimiter_steps);
    assert_eq!(after.container_steps, before.container_steps);
    doc.append("\n> end`\n").unwrap();
    let before = doc.work();
    doc.finish();
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert_eq!(doc.work().delimiter_steps - before.delimiter_steps, 4096);
}

#[test]
fn generated_scope_sequences_match_independent_visible_text_and_style_model() {
    let mut source = String::new();
    let mut expected = String::new();
    let mut probes = Vec::new();
    let mut seed = 431_u64;
    for i in 0..400 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let depth = (seed as usize % 12) + 1;
        let prefix = ">".repeat(depth);
        let a = format!("a{i}");
        let b = format!("b{i}");
        let same = seed & 16 == 0;
        let first = format!("{prefix} *{a}\n");
        probes.push((source.len() + prefix.len() + 2, same));
        source.push_str(&first);
        let next = if same {
            format!("{prefix} {b}*\n")
        } else {
            format!("{prefix}> {b}*\n")
        };
        probes.push((source.len() + prefix.len() + if same { 1 } else { 2 }, same));
        source.push_str(&next);
        source.push('\n');
        if same {
            expected.push_str(&format!("{a}\n{b}\n\n"));
        } else {
            expected.push_str(&format!("*{a}\n{b}*\n\n"));
        }
    }
    let doc = parse(&source);
    assert_eq!(whole(&doc), expected);
    for (at, expected) in probes {
        assert_eq!(style(&doc, at).contains(Style::EMPHASIS), expected);
    }
    assert!(doc.work().inline_bytes <= source.len() as u64);
    assert!(doc.work().container_bytes < 6 * source.len() as u64);
    assert!(doc.work().delimiter_steps < source.len() as u64);
}

#[test]
fn late_closure_invalidates_old_measurements_and_keeps_fold_intentions() {
    let mut doc = Document::new();
    let first = doc.append(&format!("> **{}\n", "x".repeat(2000))).unwrap();
    let mut heights = HeightIndex::default();
    let mut checkpoints = CheckpointIndex::<usize>::new();
    apply_layout_update(&doc, &first, &mut heights, &mut checkpoints).unwrap();
    checkpoints.insert(TileId(1), 42).unwrap();
    let ticket = heights.ticket(TileId(0)).unwrap();
    let key = DependencyKey::capture(&doc, TileId(0), &mut Budget::new(WorkLimit::unlimited()))
        .unwrap()
        .unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let fold = FoldId::Container(ContainerId(0));
    view.set_collapsed(&doc, fold, true).unwrap();
    let old_block = doc.blocks()[0].clone();
    let update = doc.append("> tail**\n").unwrap();
    apply_layout_update(&doc, &update, &mut heights, &mut checkpoints).unwrap();
    view.apply(&doc, &update).unwrap();
    assert_eq!(doc.blocks()[0], old_block);
    assert!(heights.measure(ticket, 99).is_err());
    assert!(checkpoints.get(TileId(1)).is_none());
    assert_eq!(
        key.validate(&doc, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap(),
        CacheValidity::Changed
    );
    assert!(view.is_collapsed(fold));
    let mut merged = update;
    let end = doc.finish();
    view.apply(&doc, &end).unwrap();
    merged.merge(end).unwrap();
    let mut delayed = FoldView::new(&doc, 1).unwrap();
    delayed.set_collapsed(&doc, fold, true).unwrap();
    let mut budget = Budget::new(WorkLimit::unlimited());
    assert_eq!(
        view.hidden_at(&doc, 1000, &mut budget),
        delayed.hidden_at(&doc, 1000, &mut budget)
    );
    let mut rebuilt = HeightIndex::default();
    let mut batch = first;
    batch.merge(merged).unwrap();
    rebuilt.apply(&doc, &batch).unwrap();
    assert_eq!(rebuilt.total_rows(), heights.total_rows());
}

#[test]
fn budgeted_display_preserves_hidden_prefix_cost_and_atomic_pause() {
    let doc = parse("> - ` 中\r\n>   🙂\r\n>   ` &amp;\n");
    let span = Span::new(0, doc.source().len());
    let mut reader = doc.display_cursor(span).unwrap();
    let before = reader.source_position();
    let mut empty = Budget::new(WorkLimit::default());
    assert_eq!(
        reader.next(&doc, &mut empty).unwrap(),
        DisplayStep::BudgetExhausted
    );
    assert_eq!(reader.source_position(), before);
    assert_eq!(empty.used(), WorkLimit::default());
    let mut output = String::new();
    let mut charged = 0;
    for _ in 0..200 {
        let limit = WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 8,
            ..Default::default()
        };
        let mut budget = Budget::new(limit);
        match reader.next(&doc, &mut budget).unwrap() {
            DisplayStep::Run(run) => {
                if !run.style.contains(Style::HIDDEN) {
                    output.push_str(run.text);
                }
            }
            DisplayStep::Complete => break,
            step => panic!("unexpected {step:?}"),
        }
        assert!(budget.used().bytes <= limit.bytes);
        charged += budget.used().bytes;
    }
    assert_eq!(reader.source_position(), span.end);
    assert_eq!(output, "中 🙂 &\n");
    assert!(charged >= span.len());
    assert_eq!(output, whole(&doc));
}
