#![forbid(unsafe_code)]
use mdstream::*;

#[path = "support/snapshot.rs"]
mod support;
include!("fixtures/commonmark_backslash_escapes.rs");

fn parse(source: &str) -> Document {
    let mut d = Document::new();
    d.append(source).unwrap();
    d.finish();
    d
}
fn shown(d: &Document) -> String {
    d.display_runs_in(Span::new(0, d.source().len()))
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect()
}
fn styled(d: &Document, style: Style) -> String {
    d.display_runs_in(Span::new(0, d.source().len()))
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN) && r.style.contains(style))
        .map(|r| r.text)
        .collect()
}

#[test]
fn official_backslash_example_14_core_semantics() {
    let (_, source, _) = BACKSLASH_CASES.iter().find(|(n, _, _)| *n == 14).unwrap();
    let expected = "*not emphasized*\n<br/> not a tag\n[not a link](/foo)\n`not code`\n1. not a list\n* not a list\n# not a heading\n[foo]: /url \"not a reference\"\n&ouml; not a character entity\n";
    let d = parse(source);
    assert_eq!(shown(&d), expected);
    assert!(d.links().is_empty());
    assert_eq!(styled(&d, Style::EMPHASIS), "");
    assert_eq!(styled(&d, Style::CODE), "");
}

#[test]
fn live_preview_and_late_closer_have_only_the_code_branch() {
    let lead = "p".repeat(BLOCK_LOOKAHEAD + 3) + " ";
    let body = "` &amp; \\* *em* ~~strike~~ [x](/u) ![y](/v) http://a.b $m$ [^n] ``inner`` end";
    let source = lead.clone() + body;
    let mut d = Document::new();
    d.append(&source).unwrap();
    assert_eq!(shown(&d), source);
    assert!(d.links().is_empty());
    assert!(d.maths().is_empty());
    assert!(d.footnote_references().is_empty());
    for style in [
        Style::EMPHASIS,
        Style::STRONG,
        Style::STRIKE,
        Style::CODE,
        Style::MATH,
    ] {
        assert_eq!(styled(&d, style), "");
    }
    d.append("` tail").unwrap();
    d.finish();
    assert_eq!(shown(&d), lead + &body[1..] + " tail");
    assert_eq!(styled(&d, Style::CODE), &body[1..]);
    assert!(d.links().is_empty());
    assert!(d.maths().is_empty());
    assert!(d.footnote_references().is_empty());
}

#[test]
fn independent_expected_all_normal_grammars_and_closed_priority() {
    let lead = "p".repeat(BLOCK_LOOKAHEAD + 3) + " ";
    let body = "` &amp; \\* *em* **strong** ~~strike~~ [link](/u) ![alt](/v) <https://a.b> http://c.d $m$ [^n] ``*literal* &amp;`` end";
    let source = lead.clone() + body;
    let mut d = Document::new();
    d.append(&source).unwrap();
    assert_eq!(shown(&d), source);
    assert!(d.links().is_empty());
    assert!(d.maths().is_empty());
    assert!(d.footnote_references().is_empty());
    for style in [
        Style::EMPHASIS,
        Style::STRONG,
        Style::STRIKE,
        Style::CODE,
        Style::MATH,
    ] {
        assert_eq!(styled(&d, style), "");
    }
    d.finish();
    assert_eq!(
        shown(&d),
        lead.clone()
            + "` & * em strong strike link alt https://a.b http://c.d m [^n] *literal* &amp; end"
    );
    assert_eq!(styled(&d, Style::EMPHASIS), "em");
    assert_eq!(styled(&d, Style::STRONG), "strong");
    assert_eq!(styled(&d, Style::STRIKE), "strike");
    assert_eq!(styled(&d, Style::CODE), "*literal* &amp;");
    assert_eq!(active(&d).len(), 4);
    assert_eq!(d.maths().len(), 1);
    assert_eq!(d.footnote_references().len(), 1);
    let mut closed = Document::new();
    closed.append(&source).unwrap();
    closed.append("` tail").unwrap();
    closed.finish();
    assert_eq!(shown(&closed), lead + &body[1..] + " tail");
    assert!(closed.links().is_empty());
    assert!(closed.maths().is_empty());
    assert!(closed.footnote_references().is_empty());
    assert_eq!(styled(&closed, Style::EMPHASIS), "");
}

#[test]
fn independent_expected_tick_reachability_and_old_context() {
    for (source, expected, code, links) in [
        ("q ```` x\\``a` end", "q ```` x`a end", "a", 0),
        ("q ```` x\\```a`` end", "q ```` x`a end", "a", 0),
        ("q ` $``x``$ end", "q ` ``x`` end", "", 0),
        ("q ` [x](/``u``) end", "q ` x end", "", 1),
        (
            "q ```` \\``a [x](/`u) end",
            "q ```` `a [x](/u) end",
            "a [x](/",
            0,
        ),
        ("q ` ``a $b``$ end", "q ` a $b$ end", "a $b", 0),
        ("q *old ` new* end", "q old ` new end", "", 0),
        ("q [old ` new](/u) end", "q old ` new end", "", 1),
        (
            "q ` x``\\* &amp;`` end",
            "q ` x\\* &amp; end",
            "\\* &amp;",
            0,
        ),
    ] {
        let d = parse(source);
        assert_eq!(shown(&d), expected, "{source:?}");
        assert_eq!(styled(&d, Style::CODE), code, "{source:?}");
        assert_eq!(active(&d).len(), links, "{source:?}");
    }
}

#[test]
fn independent_expected_real_scope_boundaries_and_partial_tabs() {
    for source in [
        "> q ` &amp; *x*\n> [y](/u)\n",
        "- q ` &amp; *x*\n  [y](/u)\n",
        "> - q ` &amp; *x*\n>   [y](/u)\n",
    ] {
        let d = parse(source);
        assert_eq!(shown(&d), "q ` & x\ny\n", "{source:?}");
        assert_eq!(active(&d).len(), 1);
    }
    for source in ["- q ` *x\n\ty* &amp;\n", "- q ` *x\n    y* &amp;\n"] {
        let d = parse(source);
        assert_eq!(shown(&d), "q ` x\n  y &\n", "{source:?}");
    }
    for source in [
        "q ` &amp; *x*\n\n[y](/u)\n",
        "q ` &amp; *x*\n# [y](/u)\n",
        "- q ` &amp; *x*\n- [y](/u)\n",
    ] {
        let d = parse(source);
        assert!(shown(&d).starts_with("q ` & x\n"), "{source:?}");
        assert!(shown(&d).ends_with("y\n"), "{source:?}");
        assert_eq!(active(&d).len(), 1);
    }
    let d = parse("a|b\n-|-\nq ` &amp; *x*|[y](/u)\n");
    assert_eq!(shown(&d), "ab\nq ` & xy\n");
    assert_eq!(active(&d).len(), 1);
}

#[test]
fn all_utf8_splits_bytewise_and_every_prefix_eof() {
    for source in [
        "中 ` &amp; \\* *字🙂* [链](/u) `` 内部& `` 尾\n",
        "> [旧 ` &amp; *中🙂*](/u)\r\n> 尾\r\n",
        "- q ```` \\``中🙂` $``x``$\n\t尾 &amp;\n",
        "q ` [x][r] &amp;\n\n[r]: /u\n",
    ] {
        let expected = support::snapshot(&parse(source));
        for split in (0..=source.len()).filter(|&at| source.is_char_boundary(at)) {
            let mut d = Document::new();
            d.append(&source[..split]).unwrap();
            d.append(&source[split..]).unwrap();
            d.finish();
            assert_eq!(support::snapshot(&d), expected, "split={split}: {source:?}");
        }
        let mut bytes = Utf8Document::new();
        for byte in source.bytes() {
            bytes.append(&[byte]).unwrap();
        }
        bytes.finish();
        assert_eq!(support::snapshot(bytes.document()), expected);
        let mut stream = Document::new();
        for (at, ch) in source.char_indices() {
            stream.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            let end = at + ch.len_utf8();
            let mut prefix = Document::new();
            prefix.append(&source[..end]).unwrap();
            assert_eq!(support::snapshot(&stream), support::snapshot(&prefix));
            let mut finished = Utf8Document::new();
            for byte in source[..end].bytes() {
                finished.append(&[byte]).unwrap();
            }
            finished.finish();
            prefix.finish();
            assert_eq!(
                support::snapshot(finished.document()),
                support::snapshot(&prefix)
            );
        }
    }
}

#[derive(Default)]
struct Sink {
    text: String,
    objects: Vec<LinkId>,
}
impl LayoutSink for Sink {
    fn begin_tile(&mut self, _: TileId) {}
    fn text(&mut self, r: RenderRun<'_>) {
        if !r.style.contains(Style::HIDDEN) {
            self.text.push_str(r.text);
        }
    }
    fn object(&mut self, id: LinkId, _: &LinkInfo) {
        self.objects.push(id);
    }
    fn end_tile(&mut self, _: TileId) -> u64 {
        1
    }
}

#[test]
fn failed_replay_invalidates_old_tickets_readers_and_actual_layout_pass() {
    let source = "p".repeat(BLOCK_LOOKAHEAD + 3) + " ` &amp; *x* [y](/u) end";
    let mut d = Document::new();
    d.append(&source).unwrap();
    let mut heights = HeightIndex::new(1).unwrap();
    heights.rebuild(&d).unwrap();
    let entity_at = source.find('&').unwrap();
    let tile = d.tile_at(entity_at).unwrap();
    let ticket = heights.ticket(tile).unwrap();
    let key = d.tile_key(tile).unwrap();
    let dependency = DependencyKey::capture(&d, tile, &mut Budget::new(WorkLimit::unlimited()))
        .unwrap()
        .unwrap();
    let mut cursor = d.display_cursor(Span::new(0, source.len())).unwrap();
    let mut old_pass = LayoutPass::new(&d, &heights, tile, u64::MAX, false).unwrap();
    let update = d.finish();
    assert_ne!(d.tile_key(tile).unwrap(), key);
    assert!(
        update
            .dirty
            .iter()
            .any(|s| s.start <= entity_at && entity_at < s.end)
    );
    heights
        .apply_with_policy(
            &d,
            &update,
            LayoutPolicy {
                style_affects_size: false,
                ..LayoutPolicy::default()
            },
        )
        .unwrap();
    assert_eq!(
        dependency.validate(&d, &mut Budget::new(WorkLimit::unlimited())),
        Ok(CacheValidity::Changed)
    );
    assert!(heights.measure(ticket, 9).is_err());
    assert_eq!(
        cursor.next(&d, &mut Budget::new(WorkLimit::unlimited())),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(
        old_pass.advance(
            &d,
            &mut heights,
            &mut Budget::new(WorkLimit::unlimited()),
            &mut Sink::default()
        ),
        Err(LayoutError::StaleContinuation)
    );
    let mut pass = LayoutPass::new(&d, &heights, TileId(0), u64::MAX, false).unwrap();
    let mut sink = Sink::default();
    let mut done = false;
    for _ in 0..source.len() * 10 {
        let pos = pass.source_position();
        assert_eq!(
            pass.advance(
                &d,
                &mut heights,
                &mut Budget::new(WorkLimit::default()),
                &mut sink
            )
            .unwrap(),
            Completion::BudgetExhausted
        );
        assert_eq!(pass.source_position(), pos);
        let mut budget = Budget::new(WorkLimit {
            bytes: 6,
            queries: 6,
            objects: 1,
            tiles: 1,
            recovery: 1,
            dependencies: 1,
        });
        let status = pass
            .advance(&d, &mut heights, &mut budget, &mut sink)
            .unwrap();
        assert!(budget.used().bytes <= 6);
        assert!(budget.used().objects <= 1);
        if status == Completion::EndOfDocument {
            done = true;
            break;
        }
    }
    assert!(done);
    assert_eq!(sink.objects, [LinkId(0)]);
}

fn active(d: &Document) -> Vec<LinkId> {
    let mut resolver = d.link_resolver();
    let mut budget = Budget::new(WorkLimit::unlimited());
    let mut window = d.link_window(Span::new(0, d.source().len()));
    let mut out = Vec::new();
    loop {
        match window.advance(d, &mut budget).unwrap() {
            LinkStep::Object(id, _) => {
                let mut activity = d.link_activity(id).unwrap();
                if activity
                    .advance_with(d, &mut resolver, &mut budget)
                    .unwrap()
                    == Some(LinkDisposition::Active)
                {
                    out.push(id);
                }
            }
            LinkStep::Complete => break,
            LinkStep::BudgetExhausted => panic!("unlimited budget"),
        }
    }
    out
}

#[test]
fn long_mixed_grammar_releases_once_and_preserves_historical_ids() {
    let unit = " &amp; \\* *em* ~~gone~~ [x](/u) ![y](/v) http://a.b ``&amp;*c*``";
    let rendered = " & * em gone x y http://a.b &amp;*c*";
    let mut d = Document::new();
    d.append("[keep](/old)\n\n").unwrap();
    let old = *d.link_info(LinkId(0)).unwrap();
    let start = d.source().len();
    let body = "q `".to_owned() + &unit.repeat(2048);
    d.append(&body).unwrap();
    assert_eq!(active(&d).len(), 1);
    let before = d.work();
    d.append("\n\n").unwrap();
    let released = d.work();
    assert_eq!(d.link_info(LinkId(0)), Some(&old));
    assert_eq!(
        shown(&d),
        "keep\n\nq `".to_owned() + &rendered.repeat(2048) + "\n\n"
    );
    assert_eq!(active(&d).len(), 1 + 3 * 2048);
    assert_eq!(styled(&d, Style::EMPHASIS), "em".repeat(2048));
    assert_eq!(styled(&d, Style::STRIKE), "gone".repeat(2048));
    assert_eq!(styled(&d, Style::CODE), "&amp;*c*".repeat(2048));
    assert_eq!(
        released.code_fallback_bytes - before.code_fallback_bytes,
        (body.len() - 3 + 1) as u64
    );
    assert!(released.code_fallback_steps - before.code_fallback_steps < 8 * body.len() as u64);
    assert!(released.code_fallback_comparisons - before.code_fallback_comparisons < 32 * 4096);
    assert!(released.inline_bytes - before.inline_bytes < 4);
    let work = d.work();
    d.append("[r]: /late\n\n").unwrap();
    assert_eq!(d.work().code_fallback_bytes, work.code_fallback_bytes);
    assert_eq!(d.link_info(LinkId(0)), Some(&old));
    assert_eq!(
        d.source().get(start..start + body.len()),
        Some(body.as_str())
    );
}

#[test]
fn nested_images_and_late_references_use_active_selection_after_replay() {
    let mut d = Document::new();
    d.append("q ` [outer ![alt](/img)](/u) [old][r]\n\n")
        .unwrap();
    let ids = active(&d);
    assert_eq!(ids.len(), 2); // nested image and its direct link parent
    let infos: Vec<_> = (0..d.links().len())
        .map(|i| *d.link_info(LinkId(i)).unwrap())
        .collect();
    let before = d.work();
    d.append("[r]: /late\n\n").unwrap();
    d.finish();
    assert_eq!(active(&d).len(), 3);
    for (i, info) in infos.iter().enumerate() {
        assert_eq!(d.link_info(LinkId(i)), Some(info));
    }
    assert_eq!(d.work().code_fallback_bytes, before.code_fallback_bytes);
    assert_eq!(shown(&d), "q ` outer alt old\n\n\n");
    let parent = active(&d)
        .into_iter()
        .find(|&id| d.link_destination(id) == Some("/u"))
        .unwrap();
    let image = active(&d)
        .into_iter()
        .find(|&id| d.link_destination(id) == Some("/img"))
        .unwrap();
    assert_eq!(d.link_parent(image), Some(parent));
}

#[test]
fn late_header_confirm_or_cancel_finishes_only_the_real_scope() {
    let long = "x".repeat(BLOCK_LOOKAHEAD + 9);
    for prefix in ["", "> "] {
        for delimiter in ["-|-", "wrong"] {
            let source = format!(
                "{prefix}*old ` &amp; end*\n{prefix}{long} [x](/u) | &amp;\n{prefix}{delimiter}\n"
            );
            let d = parse(&source);
            assert_eq!(d.tables().len(), usize::from(delimiter == "-|-"));
            assert_eq!(active(&d).len(), 1);
            let expected = if delimiter == "-|-" {
                format!("old ` & end\n{long} x&\n")
            } else {
                format!("old ` & end\n{long} x | &\nwrong\n")
            };
            assert_eq!(shown(&d), expected, "{source:?}");
            assert_eq!(styled(&d, Style::EMPHASIS), "old ` & end");
            for split in [
                0,
                source.find('\n').unwrap() + 1,
                source.len() - 2,
                source.len(),
            ] {
                let mut stream = Document::new();
                stream.append(&source[..split]).unwrap();
                stream.append(&source[split..]).unwrap();
                stream.finish();
                assert_eq!(support::snapshot(&stream), support::snapshot(&d));
            }
        }
    }
}

#[test]
fn mapped_utf8_entity_code_virtual_payload_shares_zero_one_six_budgets() {
    let source = format!(
        "- {} ` &amp; 中🙂 `` a\r\n\tb `` [x](/u)\n",
        "p".repeat(TILE_BYTES - 8)
    );
    let d = parse(&source);
    assert_eq!(active(&d).len(), 1);
    let expected = shown(&d);
    let tiled: String = (0..d.tile_count())
        .flat_map(|i| d.display_runs(TileId(i)).unwrap())
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    assert_eq!(tiled, expected);
    let mut cursor = d.display_cursor(Span::new(0, source.len())).unwrap();
    let mut resolver = d.link_resolver();
    let mut text = String::new();
    let mut completed = false;
    for _ in 0..source.len() * 20 {
        let pos = cursor.source_position();
        if pos == source.len() {
            completed = true;
            break;
        }
        let mut zero = Budget::new(WorkLimit::default());
        assert_eq!(
            cursor
                .next_with_resolver(&d, &mut resolver, &mut zero)
                .unwrap(),
            DisplayStep::BudgetExhausted
        );
        assert_eq!(cursor.source_position(), pos);
        assert_eq!(zero.used(), WorkLimit::default());
        for bytes in [1, 6] {
            let mut budget = Budget::new(WorkLimit {
                bytes,
                queries: 1,
                objects: 1,
                ..Default::default()
            });
            match cursor
                .next_with_resolver(&d, &mut resolver, &mut budget)
                .unwrap()
            {
                DisplayStep::Run(run) => {
                    assert!(run.source.start < run.source.end);
                    if let Some(span) = run.replacement {
                        assert!(span.start <= run.source.start && run.source.end <= span.end);
                        assert!(matches!(d.text(span), Some("&amp;" | "\r\n" | "\t" | " ")));
                    } else {
                        assert_eq!(d.text(run.source), Some(run.text));
                    }
                    if !run.style.contains(Style::HIDDEN) {
                        text.push_str(run.text);
                    }
                }
                DisplayStep::Complete => {
                    completed = true;
                    break;
                }
                DisplayStep::BudgetExhausted => {}
            }
            assert!(budget.used().bytes <= bytes);
            assert!(budget.used().queries <= 1);
            assert!(budget.used().objects <= 1);
        }
        if completed {
            break;
        }
    }
    assert!(completed);
    assert_eq!(text, expected);
}

#[test]
fn deep_million_byte_growth_and_many_distinct_runs_have_counted_bounds() {
    let mut d = Document::new();
    let prefix = "> ".repeat(970);
    d.append(&(prefix.clone() + "q ` " + &"a".repeat(1_000_000)))
        .unwrap();
    let before = d.work();
    for _ in 0..1000 {
        d.append("b").unwrap();
    }
    let grown = d.work();
    assert_eq!(grown.inline_bytes - before.inline_bytes, 1000);
    assert_eq!(grown.code_fallback_bytes, before.code_fallback_bytes);
    assert_eq!(
        grown.code_fallback_comparisons,
        before.code_fallback_comparisons
    );
    assert_eq!(grown.tree_visits, before.tree_visits);
    assert!(grown.code_fallback_steps - before.code_fallback_steps <= 1000);
    d.append(" &amp; *x* [y](/u)\n\n").unwrap();
    let done = d.work();
    assert_eq!(active(&d).len(), 1);
    assert!(done.code_fallback_bytes <= d.source().len() as u64);
    assert!(done.code_fallback_steps <= 4 * d.source().len() as u64);
    assert_eq!(
        done.code_fallback_bytes,
        done.code_fallback_eligibility_bytes
    );
    assert!(shown(&d).ends_with(" & x y\n\n"));
    for count in [64usize, 256, 1024, 4096] {
        let mut source = "q `".to_owned();
        for i in 0..count {
            source.push(' ');
            source.push_str(&"`".repeat(2 + (i * 37) % count));
            source.push_str(" &amp; *x*");
        }
        let mut d = Document::new();
        d.append(&source).unwrap();
        let before = d.work();
        d.finish();
        let work = d.work();
        assert_eq!(work.inline_bytes, before.inline_bytes);
        assert_eq!(work.code_fallback_bytes, (source.len() - 3) as u64);
        assert!(work.code_fallback_steps <= 8 * source.len() as u64);
        assert!(work.code_fallback_comparisons <= 8 * count as u64 * (count.ilog2() + 2) as u64);
        assert_eq!(styled(&d, Style::CODE), "");
        assert_eq!(styled(&d, Style::EMPHASIS), "x".repeat(count));
        eprintln!(
            "runs={count} bytes={} replay={} steps={} comparisons={}",
            source.len(),
            work.code_fallback_bytes,
            work.code_fallback_steps,
            work.code_fallback_comparisons
        );
    }
    let count = 8192;
    let mut source = "q `".to_owned();
    let mut expected = "q `".to_owned();
    for i in 0..count {
        let ticks = "`".repeat(2 + i % 31);
        source.push_str(&format!(" {ticks}*c* &amp;{ticks} *e* &amp; [x](/u)"));
        expected.push_str(" *c* &amp; e & x");
    }
    let doc = parse(&source);
    assert_eq!(shown(&doc), expected);
    assert_eq!(styled(&doc, Style::CODE), "*c* &amp;".repeat(count));
    assert_eq!(styled(&doc, Style::EMPHASIS), "e".repeat(count));
    assert_eq!(active(&doc).len(), count);
    let work = doc.work();
    assert_eq!(work.code_fallback_bytes, (source.len() - 3) as u64);
    assert!(work.code_fallback_steps <= 4 * source.len() as u64);
    assert!(work.code_fallback_comparisons <= 8 * 16384 * 15);
    eprintln!(
        "mixed_runs=16384 bytes={} replay={} steps={} comparisons={}",
        source.len(),
        work.code_fallback_bytes,
        work.code_fallback_steps,
        work.code_fallback_comparisons
    );
}

#[test]
fn every_live_utf8_prefix_matches_forced_literal_primary_and_closures_remain_opaque() {
    let lead = "p".repeat(BLOCK_LOOKAHEAD + 7) + " q ";
    let ticks = "```````";
    let escaped = "\\`".repeat(7);
    let body = " &amp; \\* *中🙂* ~~gone~~ [x](/u) ![y](/v) http://a.b `` a `` $m$ [^n]";
    let mut stream = Document::new();
    stream.append(&(lead.clone() + ticks)).unwrap();
    for (at, ch) in body.char_indices() {
        stream.append(ch.encode_utf8(&mut [0; 4])).unwrap();
        let end = at + ch.len_utf8();
        let source = lead.clone() + ticks + &body[..end];
        assert_eq!(shown(&stream), source);
        assert!(stream.links().is_empty());
        assert!(stream.maths().is_empty());
        assert!(stream.footnote_references().is_empty());
        let released = parse(&source);
        let ordinary = parse(&(lead.clone() + &escaped + &body[..end]));
        assert_eq!(shown(&released), shown(&ordinary), "prefix={end}");
        for style in [
            Style::EMPHASIS,
            Style::STRONG,
            Style::STRIKE,
            Style::CODE,
            Style::MATH,
        ] {
            assert_eq!(
                styled(&released, style),
                styled(&ordinary, style),
                "prefix={end}"
            );
        }
        assert_eq!(active(&released).len(), active(&ordinary).len());
        assert_eq!(released.maths().len(), ordinary.maths().len());
        assert_eq!(
            released.footnote_references().len(),
            ordinary.footnote_references().len()
        );
        let closed = parse(&(source + " " + ticks));
        assert!(closed.links().is_empty());
        assert!(closed.maths().is_empty());
        assert!(closed.footnote_references().is_empty());
        assert_eq!(styled(&closed, Style::EMPHASIS), "");
    }
}

#[test]
fn inherited_thousands_of_label_frames_are_reused_without_recursive_replay() {
    let source = "q ".to_owned() + &"[".repeat(5000) + "` &amp; *x*" + &"](/u)".repeat(5000);
    let d = parse(&source);
    assert_eq!(active(&d).len(), 1);
    assert_eq!(styled(&d, Style::EMPHASIS), "x");
    assert_eq!(styled(&d, Style::CODE), "");
    assert!(!shown(&d).contains("&amp;"));
    let w = d.work();
    assert!(w.code_fallback_bytes <= source.len() as u64);
    assert!(w.code_fallback_steps <= 2 * source.len() as u64);
    assert!(w.link_label_steps <= 20 * source.len() as u64);
}

#[test]
fn late_definition_selects_replayed_objects_under_one_shared_budget_and_retires_memo() {
    let mut d = Document::new();
    d.append("q ` [parent [r]](/u) [x](/x)\n\n").unwrap();
    let before = active(&d);
    let urls: Vec<_> = before
        .iter()
        .map(|&id| d.link_destination(id).unwrap())
        .collect();
    assert_eq!(urls, ["/u", "/x"]);
    let infos: Vec<_> = (0..d.links().len())
        .map(|i| *d.link_info(LinkId(i)).unwrap())
        .collect();
    let mut resolver = d.link_resolver();
    for id in before {
        d.link_activity(id)
            .unwrap()
            .advance_with(&d, &mut resolver, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap();
    }
    let work = d.work();
    d.append("[r]: /late\n\n").unwrap();
    assert_eq!(d.work().code_fallback_bytes, work.code_fallback_bytes);
    let n = resolver.memo_entries();
    resolver.retire(&mut Budget::new(WorkLimit::default()));
    assert_eq!(resolver.memo_entries(), n);
    let mut retired = false;
    for _ in 0..n + 2 {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        if resolver.retire(&mut budget) == Completion::Covered {
            retired = true;
            break;
        }
    }
    assert!(retired);
    resolver.reset(&d).unwrap();
    let mut window = d.link_window(Span::new(0, d.source().len()));
    let mut pending: Option<(LinkId, LinkActivity)> = None;
    let mut selected = Vec::new();
    let mut complete = false;
    for _ in 0..1000 {
        let mut budget = Budget::new(WorkLimit {
            bytes: 6,
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        if let Some((id, mut activity)) = pending.take() {
            match activity
                .advance_with(&d, &mut resolver, &mut budget)
                .unwrap()
            {
                Some(LinkDisposition::Active) => selected.push(id),
                Some(_) => (),
                None => pending = Some((id, activity)),
            }
        }
        if pending.is_none() {
            match window.advance(&d, &mut budget).unwrap() {
                LinkStep::Object(id, _) => pending = Some((id, d.link_activity(id).unwrap())),
                LinkStep::Complete => {
                    complete = true;
                    break;
                }
                LinkStep::BudgetExhausted => (),
            }
        }
        assert!(
            budget.used().queries <= 1 && budget.used().objects <= 1 && budget.used().bytes <= 6
        );
    }
    assert!(complete);
    assert_eq!(selected, active(&d));
    let urls: Vec<_> = selected
        .iter()
        .map(|&id| d.link_destination(id).unwrap())
        .collect();
    assert_eq!(urls, ["/late", "/x"]);
    for (i, info) in infos.iter().enumerate() {
        assert_eq!(d.link_info(LinkId(i)), Some(info));
    }
}

#[test]
fn replayed_multiline_suffixes_keep_prefix_events_and_original_metadata_mapping() {
    for (first, continuation) in [("", ""), ("> ", "> "), ("- ", "  "), ("> - ", ">   ")] {
        let lead = "p".repeat(BLOCK_LOOKAHEAD + 1);
        let source = format!(
            "{first}{lead} q ` [x](\n{continuation}/u\n{continuation}'a\n{continuation}b')\n"
        );
        let control = source.replacen('`', "\\`", 1);
        let d = parse(&source);
        let ordinary = parse(&control);
        assert_eq!(shown(&d), shown(&ordinary));
        let ids = active(&d);
        assert_eq!(ids.len(), 1, "{first:?}");
        assert_eq!(d.link_destination(ids[0]), Some("/u"));
        let mut title = d.decoded_link_title(ids[0]).unwrap();
        let mut text = String::new();
        for _ in 0..source.len() * 2 {
            let mut budget = Budget::new(WorkLimit {
                bytes: 6,
                queries: 1,
                objects: 1,
                ..Default::default()
            });
            match title.advance(&mut budget) {
                Ok(Some(chunk)) => {
                    assert!(d.text(chunk.source).is_some());
                    text.push_str(chunk.text);
                }
                Ok(None) => break,
                Err(Completion::BudgetExhausted) => (),
                other => panic!("metadata {other:?}"),
            }
            assert!(budget.used().bytes <= 6);
        }
        assert_eq!(text, "a\nb", "{first:?}");
        assert_eq!(d.link_title(ids[0]).unwrap(), format!("a\n{continuation}b"));
        let mut bytewise = Utf8Document::new();
        for byte in source.bytes() {
            bytewise.append(&[byte]).unwrap();
        }
        bytewise.finish();
        assert_eq!(
            support::snapshot(bytewise.document()),
            support::snapshot(&d)
        );
        let math_source = format!("{first}{lead} q ` $a\n{continuation}b$\n");
        let math = parse(&math_source);
        assert_eq!(math.maths().len(), 1);
        let mut payload = math.math_payload(MathId(0)).unwrap();
        let mut text = String::new();
        while let Some(chunk) = payload
            .advance(&mut Budget::new(WorkLimit::unlimited()))
            .unwrap()
        {
            assert!(math.text(chunk.source).is_some());
            text.push_str(chunk.text);
        }
        assert_eq!(text, "a\nb");
    }
}
