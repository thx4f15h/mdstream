use mdtui3::*;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn read(doc: &Document, id: MathId, mapped: bool, bytes: usize) -> (String, Vec<Span>) {
    let math = doc.math(id).unwrap();
    let mut cursor = if mapped {
        doc.math_payload(id)
    } else {
        doc.math_content(id)
    }
    .unwrap();
    let mut text = String::new();
    let mut gaps = Vec::new();
    let mut position = math.content.start;
    let mut charged = 0;
    loop {
        let mut budget = Budget::new(WorkLimit {
            bytes,
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        let Some(chunk) = cursor.advance(&mut budget).unwrap() else {
            break;
        };
        assert_eq!(chunk.source.start, position);
        assert!(chunk.source.end > position);
        assert!(chunk.source.len() <= TILE_BYTES);
        assert_eq!(budget.used().bytes, chunk.source.len());
        assert_eq!(budget.used().queries, 1);
        assert_eq!(budget.used().objects, 1);
        if chunk.text.is_empty() {
            gaps.push(chunk.source);
        } else {
            assert_eq!(doc.text(chunk.source), Some(chunk.text));
        }
        text.push_str(chunk.text);
        charged += budget.used().bytes;
        position = cursor.source_position();
        assert_eq!(position, chunk.source.end);
    }
    assert_eq!(position, math.content.end);
    assert_eq!(charged, math.content.len());
    assert_eq!(
        cursor.advance(&mut Budget::new(WorkLimit::default())),
        Ok(None)
    );
    (text, gaps)
}

fn payload(doc: &Document, id: usize) -> String {
    read(doc, MathId(id), true, 4).0
}

fn visible(doc: &Document, span: Span) -> String {
    doc.display_runs_in(span)
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect()
}

#[test]
fn explicit_and_lazy_continuations_keep_math_identity_and_leaf_ranges() {
    for (first, next) in [
        ("> ", "> "),
        ("> - ", ">   "),
        ("12) ", "    "),
        ("> > ", ">> "),
        ("> - ", ""),
    ] {
        for delimiter in ["$", "$$"] {
            let mut doc = Document::new();
            doc.append(&format!("{first}{delimiter}中 &amp;\\\r\n"))
                .unwrap();
            let old = doc.blocks()[0].clone();
            let update = doc.append(&format!("{next}🙂{delimiter}\n")).unwrap();
            assert_eq!(update.new_maths, 0..1, "{first:?} / {next:?}");
            assert_eq!(doc.blocks()[0], old);
            assert_eq!(doc.blocks()[1].container, old.container);
            assert_eq!(doc.math(MathId(0)).unwrap().source.start, first.len());
            assert_eq!(
                doc.math(MathId(0)).unwrap().source.end,
                doc.source().len() - 1
            );
            let math = *doc.math(MathId(0)).unwrap();
            assert_eq!(payload(&doc, 0), "中 &amp;\\\r\n🙂");
            assert_eq!(
                read(&doc, MathId(0), false, 4).0,
                format!("中 &amp;\\\r\n{next}🙂")
            );
            assert_eq!(visible(&doc, math.content), "中 &amp;\\\n🙂");
            assert!(doc.links().is_empty());
            assert!(doc.finish().new_maths.is_empty());
            assert_eq!(*doc.math(MathId(0)).unwrap(), math);
        }
    }
}

#[test]
fn exact_delimiters_escapes_and_opaque_content_survive_prefix_removal() {
    for (body, expected) in [
        ("$a\\\n> $b", "a\\\n"),
        ("$a\\$\n> b$", "a\\$\nb"),
        ("$$a$\n> b$$", "a$\nb"),
        ("$a$$\n> b$", "a$$\nb"),
        (
            "$$a\r\n>  &amp; **x** [x](u) `code`\\alpha_1$$",
            "a\r\n &amp; **x** [x](u) `code`\\alpha_1",
        ),
    ] {
        let doc = parse(&format!("> {body}\n"));
        assert_eq!(doc.maths().len(), 1, "{body:?}");
        assert_eq!(payload(&doc, 0), expected);
        assert!(doc.links().is_empty());
        assert!(doc.footnote_references().is_empty());
    }
    let doc = parse("> **![a $中\r\n> 🙂$ b](u)**\n");
    assert_eq!(payload(&doc, 0), "中\r\n🙂");
    assert_eq!(doc.link_destination(LinkId(0)), Some("u"));
    for r in doc.runs_in(doc.maths()[0].content).unwrap() {
        assert!(r.style.contains(Style::MATH));
        assert!(r.style.contains(Style::STRONG));
        assert_eq!(r.link, Some(LinkId(0)));
    }
}

#[test]
fn scope_and_block_interruptions_end_candidates_without_replaying_body() {
    for source in [
        "> $open\n\n> end$\n",
        "> $open\n>\n> end$\n",
        "> $open\n> > end$\n",
        "- $open\n- end$\n",
        "> - $open\n> - end$\n",
        "> $open\n# end$\n",
        "> # $open\n> end$\n",
        "> $open\n> ---\n> end$\n",
        "> $open\n> ```\n> raw\n> ```\n> end$\n",
        "> $open\n>     raw\n> end$\n",
        "> $open\n> [r]: /u\n> end$\n",
        "> $open\n> [^r]: raw\n> end$\n",
        "> $open\n> h|h\n> -|-\n> end$|z\n",
    ] {
        assert!(parse(source).maths().is_empty(), "{source:?}");
    }
    for end in ["", "\n", "\n> $$mismatch\n", "\n> # stop\n"] {
        let doc = parse(&format!("> $**opaque**\n> [x](u) &amp;{end}"));
        assert!(doc.maths().is_empty());
        assert!(doc.links().is_empty());
        let at = doc.source().find("[x]").unwrap();
        assert_eq!(
            visible(&doc, Span::new(at, at + "[x](u) &amp;".len())),
            "[x](u) &amp;"
        );
    }
}

#[test]
fn setext_promotion_keeps_confirmed_mapping_and_later_ids_are_stable() {
    let mut doc = Document::new();
    doc.append("> $a\n> b$\n").unwrap();
    let math = *doc.math(MathId(0)).unwrap();
    let update = doc.append("> ---\n> $next\n> end$\n").unwrap();
    assert_eq!(update.new_headings, 0..1);
    assert_eq!(update.new_maths, 1..2);
    assert_eq!(doc.headings().entries()[0].block, BlockId(0));
    assert_eq!(doc.maths()[0], math);
    assert_eq!(payload(&doc, 0), "a\nb");
    assert_eq!(payload(&doc, 1), "next\nend");
    doc.finish();
    assert_eq!(doc.maths()[0], math);
}

#[test]
fn shared_budget_pauses_atomically_at_scalars_and_omitted_prefixes() {
    let doc = parse("> - $🙂\r\n>   中$\n");
    let mut cursor = doc.math_payload(MathId(0)).unwrap();
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
        WorkLimit {
            bytes: 3,
            ..WorkLimit::unlimited()
        },
    ] {
        let pos = cursor.source_position();
        let mut budget = Budget::new(limit);
        assert_eq!(
            cursor.advance(&mut budget),
            Err(Completion::BudgetExhausted)
        );
        assert_eq!(cursor.source_position(), pos);
        assert_eq!(budget.used(), WorkLimit::default());
    }
    let mut budget = Budget::new(WorkLimit {
        queries: 2,
        objects: 2,
        bytes: 6,
        ..Default::default()
    });
    let mut objects = doc.maths_in(doc.maths()[0].source).unwrap();
    assert_eq!(objects.advance(&mut budget).unwrap().unwrap().0, MathId(0));
    assert_eq!(cursor.advance(&mut budget).unwrap().unwrap().text, "🙂\r\n");
    let pos = cursor.source_position();
    assert_eq!(
        cursor.advance(&mut budget),
        Err(Completion::BudgetExhausted)
    );
    assert_eq!(cursor.source_position(), pos);
    let mut budget = Budget::new(WorkLimit {
        bytes: 1,
        queries: 1,
        objects: 1,
        ..Default::default()
    });
    let omitted = cursor.advance(&mut budget).unwrap().unwrap();
    assert_eq!(omitted.source, Span::new(pos, pos + 1));
    assert_eq!(omitted.text, "");
    assert_eq!(budget.used().bytes, 1);
    assert!(doc.math_payload(MathId(1)).is_none());
    assert_eq!(payload(&doc, 0), "🙂\r\n中");
}

#[test]
fn raw_crlf_escapes_and_literal_markers_do_not_follow_general_hidden_styles() {
    let doc = parse("> $a  \r\n>  b\\$c$\n>\n> h|h\n> -|-\n> $a\\|b$|$ c $\n");
    assert_eq!(payload(&doc, 0), "a  \r\n b\\$c");
    assert_eq!(payload(&doc, 1), "a\\|b");
    assert_eq!(payload(&doc, 2), " c ");
    assert_eq!(visible(&doc, doc.maths()[1].content), "a|b");
    for i in 0..doc.maths().len() {
        assert_eq!(
            read(&doc, MathId(i), false, TILE_BYTES).0,
            doc.text(doc.maths()[i].content).unwrap()
        );
    }
}

#[test]
fn short_and_long_table_confirmation_isolate_math_but_failed_headers_continue() {
    for count in [4, 1040] {
        for success in [false, true] {
            let source = format!(
                "> $$old\n> {}|$local$\n>{}\n> tail$$\n",
                "x".repeat(count),
                if success { " -|-" } else { " wrong" }
            );
            let doc = parse(&source);
            assert_eq!(doc.tables().len(), usize::from(success));
            assert_eq!(doc.maths().len(), 1);
            if success {
                assert_eq!(payload(&doc, 0), "local");
            } else {
                assert_eq!(
                    payload(&doc, 0),
                    format!("old\n{}|$local$\nwrong\ntail", "x".repeat(count))
                );
            }
            assert!(doc.work().inline_bytes <= source.len() as u64 + doc.work().table_bytes);
        }
    }
    for close in ["$", "$$", "$$$"] {
        let doc = parse(&format!(
            "> $$old\n> {}{close}|$local$\n> -|-\n",
            "x".repeat(1040)
        ));
        assert_eq!(doc.tables().len(), 1); // Pending run at the pipe is discarded.
        assert_eq!(payload(&doc, 0), "local");
    }
    let doc = parse(&format!(
        "> $old\n> {}|b\n> {}x|tail$\n",
        "x".repeat(1040),
        "-".repeat(1040)
    ));
    assert!(doc.tables().is_empty());
    assert_eq!(
        payload(&doc, 0),
        format!("old\n{}|b\n{}x|tail", "x".repeat(1040), "-".repeat(1040))
    );
}

#[test]
fn late_close_invalidates_old_measurements_caches_and_search_and_preserves_fold_state() {
    let mut doc = Document::new();
    let mut batch = doc.append(&format!("> $中{}\n", "x".repeat(2000))).unwrap();
    let old = doc.blocks()[0].clone();
    let mut heights = HeightIndex::default();
    let mut checkpoints = CheckpointIndex::<usize>::new();
    apply_layout_update(&doc, &batch, &mut heights, &mut checkpoints).unwrap();
    checkpoints.insert(TileId(1), 42).unwrap();
    let ticket = heights.ticket(TileId(0)).unwrap();
    let key = DependencyKey::capture(&doc, TileId(0), &mut Budget::new(WorkLimit::unlimited()))
        .unwrap()
        .unwrap();
    let mut search = DisplaySearch::new(&doc, "中").unwrap();
    let mut view = FoldView::new(&doc, 1).unwrap();
    let fold = FoldId::Container(ContainerId(0));
    view.set_collapsed(&doc, fold, true).unwrap();
    let update = doc.append("> tail$\n").unwrap();
    assert_eq!(update.new_maths, 0..1);
    assert_eq!(doc.blocks()[0], old);
    apply_layout_update(&doc, &update, &mut heights, &mut checkpoints).unwrap();
    view.apply(&doc, &update).unwrap();
    batch.merge(update).unwrap();
    assert!(heights.measure(ticket, 99).is_err());
    assert!(checkpoints.get(TileId(1)).is_none());
    assert_eq!(
        key.validate(&doc, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap(),
        CacheValidity::Changed
    );
    assert_eq!(
        search
            .advance(
                &doc,
                &mut Budget::new(WorkLimit::unlimited()),
                &mut Vec::new()
            )
            .unwrap_err(),
        LayoutError::StaleContinuation
    );
    assert!(view.is_collapsed(fold));
    let end = doc.finish();
    heights.apply(&doc, &end).unwrap();
    view.apply(&doc, &end).unwrap();
    batch.merge(end).unwrap();
    assert_eq!(batch.new_maths, 0..1);
    let mut rebuilt = HeightIndex::default();
    rebuilt.apply(&doc, &batch).unwrap();
    assert_eq!(heights.total_rows(), rebuilt.total_rows());
    let mut search = DisplaySearch::new(&doc, "x\ntail").unwrap();
    let mut hits = Vec::new();
    search
        .advance(&doc, &mut Budget::new(WorkLimit::unlimited()), &mut hits)
        .unwrap();
    assert_eq!(hits.len(), 1);
}

#[test]
fn deep_million_byte_math_only_consumes_new_payload_and_folded_layout_skips_it() {
    for (first, next) in [
        (">".repeat(1000), ">".repeat(1000)),
        ("- ".repeat(250), " ".repeat(500)),
    ] {
        let mut doc = Document::new();
        doc.append(&format!("{first}$中{}", "x".repeat(1_000_000)))
            .unwrap();
        let before = doc.work();
        let arena = doc.annotation_arena_stats();
        for _ in 0..1000 {
            doc.append("x").unwrap();
        }
        let after = doc.work();
        assert_eq!(after.inline_bytes - before.inline_bytes, 1000);
        assert_eq!(after.classified_bytes, before.classified_bytes);
        assert_eq!(after.math_steps, before.math_steps);
        assert_eq!(after.container_steps, before.container_steps);
        assert_eq!(
            doc.annotation_arena_stats().allocation_calls,
            arena.allocation_calls
        );
        let before = doc.work();
        doc.append(&format!("\r\n{next}end$\n")).unwrap();
        let after = doc.work();
        assert_eq!(after.inline_bytes - before.inline_bytes, 7);
        assert!(after.tree_visits - before.tree_visits < 2000);
        assert!(after.container_steps - before.container_steps <= 1000);
        assert_eq!(doc.maths().len(), 1);
        assert_eq!(
            read(&doc, MathId(0), true, TILE_BYTES).0,
            format!("中{}\r\nend", "x".repeat(1_001_000))
        );
        doc.finish();
        assert!(doc.work().inline_bytes <= doc.source().len() as u64);
        struct Hidden;
        impl LayoutSink for Hidden {
            fn begin_tile(&mut self, _: TileId) {}
            fn text(&mut self, _: RenderRun<'_>) {
                panic!("hidden math read")
            }
            fn object(&mut self, _: LinkId, _: &LinkInfo) {
                panic!("hidden object read")
            }
            fn end_tile(&mut self, _: TileId) -> u64 {
                1
            }
        }
        impl FoldLayoutSink for Hidden {
            fn placeholder(&mut self, _: FoldPlaceholder) {}
        }
        let mut view = FoldView::new(&doc, 1).unwrap();
        view.set_collapsed(&doc, FoldId::Container(ContainerId(0)), true)
            .unwrap();
        let mut pass = LayoutPass::folded(&doc, &view, TileId(0), 10).unwrap();
        let mut budget = Budget::new(WorkLimit {
            bytes: 0,
            ..WorkLimit::unlimited()
        });
        assert_eq!(
            pass.advance_folded(&doc, &mut view, &mut budget, &mut Hidden)
                .unwrap(),
            Completion::EndOfDocument
        );
        assert_eq!(budget.used().bytes, 0);
    }
}

#[test]
fn thousands_of_lazy_lines_and_old_objects_do_not_increase_append_work() {
    let mut doc = Document::new();
    doc.append(&format!("{}$a\n", ">".repeat(1000))).unwrap();
    let before = doc.work();
    for _ in 0..4096 {
        doc.append("x\n").unwrap();
    }
    assert_eq!(doc.work().inline_bytes - before.inline_bytes, 8192);
    assert_eq!(doc.work().container_steps - before.container_steps, 4096);
    doc.append("end$\n").unwrap();
    assert_eq!(
        read(&doc, MathId(0), true, TILE_BYTES).0,
        format!("a\n{}end", "x\n".repeat(4096))
    );
    let mut doc = Document::new();
    for _ in 0..4096 {
        doc.append("> $a\n> b$\n>\n").unwrap();
    }
    let before = doc.work();
    doc.append("> $中\n> 🙂$\n").unwrap();
    assert_eq!(doc.work().inline_bytes - before.inline_bytes, 11);
    assert_eq!(doc.work().math_steps - before.math_steps, 2);
    assert!(doc.work().tree_visits - before.tree_visits < 1000);
    assert_eq!(payload(&doc, 4096), "中\n🙂");
    assert_eq!(payload(&doc, 0), "a\nb");
}

#[test]
fn generated_containers_match_prefix_free_math_and_source_range_models() {
    let tokens = [
        "a", "$$", "$$$", "\\$", "\\\\", "&amp;", "[x](u)", "**x**", "`code`", "🙂",
    ];
    for i in 0..400 {
        let depth = i % 16 + 1;
        let prefix = ">".repeat(depth);
        let delimiter = if i % 2 == 0 { "$" } else { "$$" };
        let body = format!(
            "{delimiter}中{}\r\n{} tail{delimiter}\n",
            tokens[i % 10],
            tokens[i / 10 % 10]
        );
        let source = body
            .split_inclusive('\n')
            .map(|l| format!("{prefix}{l}"))
            .collect::<String>();
        let plain = parse(&body);
        let doc = parse(&source);
        assert_eq!(doc.maths().len(), plain.maths().len());
        for (index, (math, old)) in doc.maths().iter().zip(plain.maths()).enumerate() {
            let map =
                |at: usize| at + (body[..at].bytes().filter(|b| *b == b'\n').count() + 1) * depth;
            assert_eq!(math.kind, old.kind);
            assert_eq!(
                math.source,
                Span::new(map(old.source.start), map(old.source.end))
            );
            assert_eq!(
                math.content,
                Span::new(map(old.content.start), map(old.content.end))
            );
            assert_eq!(
                payload(&doc, index),
                read(&plain, MathId(index), false, 4).0
            );
        }
        assert!(doc.work().inline_bytes <= source.len() as u64);
    }
}
