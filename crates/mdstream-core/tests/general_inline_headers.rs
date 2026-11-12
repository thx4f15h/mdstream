#[path = "support/header_confirmation.rs"]
mod scope_composition;
use mdstream::*;

fn parse(source: &str) -> Document {
    let mut d = Document::new();
    d.append(source).unwrap();
    d.finish();
    d
}
fn shown(d: &Document, source: Span) -> Vec<(char, Style, usize, usize)> {
    d.display_runs_in(source)
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .flat_map(|r| {
            r.text
                .chars()
                .map(move |c| (c, r.style, r.emphasis_depth, r.strong_depth))
        })
        .collect()
}
fn active_links(d: &Document, source: Span) -> Vec<(LinkKind, String)> {
    let mut resolver = d.link_resolver();
    d.links_in(source)
        .filter_map(|(id, info)| {
            let mut activity = d.link_activity(id).unwrap();
            (activity
                .advance_with(d, &mut resolver, &mut Budget::new(WorkLimit::unlimited()))
                .unwrap()
                == Some(LinkDisposition::Active))
            .then(|| (info.kind, d.link_url(id).unwrap().into_owned()))
        })
        .collect()
}
fn active_maths(d: &Document, source: Span) -> Vec<(MathKind, String)> {
    let mut cursor = d.maths_in(source).unwrap();
    let mut result = Vec::new();
    while let Some((id, math)) = cursor
        .advance(&mut Budget::new(WorkLimit::unlimited()))
        .unwrap()
    {
        assert_eq!(d.math_state(id), Some(SyntaxState::Sealed));
        let mut payload = d.math_payload(id).unwrap();
        let mut text = String::new();
        while let Some(chunk) = payload
            .advance(&mut Budget::new(WorkLimit::unlimited()))
            .unwrap()
        {
            text.push_str(chunk.text);
        }
        result.push((math.kind, text));
    }
    result
}
fn footnotes(d: &Document, source: Span) -> Vec<String> {
    let mut cursor = d.footnote_references_in(source).unwrap();
    let mut result = Vec::new();
    while let Some((id, r)) = cursor
        .advance(&mut Budget::new(WorkLimit::unlimited()))
        .unwrap()
    {
        assert_eq!(d.footnote_reference_state(id), Some(SyntaxState::Sealed));
        result.push(d.footnote(r.symbol).unwrap().label.clone());
    }
    result
}

#[test]
fn general_confirmation_finishes_the_real_old_scope_and_parses_independent_cells() {
    for (prior, first) in [
        ("*prior\n", "end*x* "),
        ("`prior\n", "end` *x* "),
        ("$prior\n", "end$ *x* "),
        ("[prior\n", "end](u*x*) *x* "),
        ("![outer [prior\n", "end](u*x*)](image) *x* "),
        ("`[old](old) *old* &amp;\n", "end` *x* "),
        ("[prior *a**b\n", "end](u*x*) *x* "),
    ] {
        for (open, prefix) in [("", ""), ("> ", "> "), ("- ", "  "), ("> - [x] ", ">   ")] {
            let prior = format!("{open}{prior}");
            let header = format!(
                "{first}[h](v) $q$ [^n] &amp; {}|[t](w)\n",
                "中🙂".repeat(180)
            );
            let source = format!("{prior}{prefix}{header}{prefix}---|---\n{prefix}x|y\n");
            let d = parse(&source);
            assert_eq!(d.tables().len(), 1, "{source:?}");
            scope_composition::assert_scope_composition(&d);
            assert_eq!(d.source(), source);
        }
    }
}

#[test]
fn pending_row_objects_are_provisional_and_unchanged_events_reuse_their_ids() {
    for success in [false, true] {
        let mut d = Document::new();
        d.append("*prior\n").unwrap();
        d.append(&format!(
            "end*x* [h](v) $q$ [^n] &amp; {}",
            "x".repeat(1800)
        ))
        .unwrap();
        let h = d
            .links_in(Span::new(7, d.source().len()))
            .find(|(_, i)| d.text(i.label) == Some("h"))
            .unwrap()
            .0;
        let before = *d.link_info(h).unwrap();
        assert_eq!(before.state, SyntaxState::Provisional);
        assert_eq!(d.math_state(MathId(0)), Some(SyntaxState::Provisional));
        assert_eq!(
            d.footnote_reference_state(FootnoteReferenceId(0)),
            Some(SyntaxState::Provisional)
        );
        assert!(
            shown(&d, Span::new(0, d.source().len()))
                .iter()
                .any(|(c, _, _, _)| *c == 'h')
        );
        d.append("|b\n").unwrap();
        d.append(if success { "---|---\n" } else { "wrong\n" })
            .unwrap();
        d.finish();
        assert_eq!(d.tables().len(), usize::from(success));
        let after = *d.link_info(h).unwrap();
        assert_eq!(after.source, before.source);
        assert_eq!(after.label, before.label);
        assert_eq!(after.state, SyntaxState::Sealed);
        assert_eq!(d.math_state(MathId(0)), Some(SyntaxState::Sealed));
        assert_eq!(
            d.footnote_reference_state(FootnoteReferenceId(0)),
            Some(SyntaxState::Sealed)
        );
    }
}

#[test]
fn superseded_math_and_footnote_candidates_are_not_active_objects() {
    let source = format!(
        "$prior\nend$ [h](v) $q$ [^n] {}|b\n---|---\n",
        "x".repeat(1800)
    );
    let d = parse(&source);
    assert_eq!(d.tables().len(), 1);
    let cell = d.tables()[0].cell(0, 0).unwrap().source;
    let (expected, expected_range) = scope_composition::inline_context(d.text(cell).unwrap(), &d);
    assert_eq!(
        active_maths(&d, cell),
        active_maths(&expected, expected_range)
    );
    assert_eq!(footnotes(&d, cell), footnotes(&expected, expected_range));
    for i in 0..d.footnote_references().len() {
        if d.footnote_reference_state(FootnoteReferenceId(i)).is_none() {
            assert_eq!(
                d.resolve_footnote(
                    FootnoteReferenceId(i),
                    &mut Budget::new(WorkLimit::unlimited())
                )
                .unwrap(),
                None
            );
        }
    }
}

#[path = "support/snapshot.rs"]
mod snapshot;
#[test]
fn every_utf8_cut_and_prefix_eof_preserves_general_header_choices_and_raw_ids() {
    for (prior, first) in [
        ("*前\r\n", "末*x* "),
        ("`前\n", "末` *x* "),
        ("$前\n", "末$ *x* "),
        ("[前 *a**b\n", "末](u*x*) *x* "),
    ] {
        for ending in ["---|---\n", "wrong\n", ""] {
            let source = format!(
                "{prior}{first}[h](v) $q$ &amp; {}|中🙂\n{ending}",
                "x".repeat(1030)
            );
            let expected = parse(&source);
            let frozen = snapshot::snapshot(&expected);
            for cut in 0..=source.len() {
                let mut d = Utf8Document::new();
                d.append(&source.as_bytes()[..cut]).unwrap();
                d.append(&source.as_bytes()[cut..]).unwrap();
                d.finish();
                assert_eq!(
                    snapshot::snapshot(d.document()),
                    frozen,
                    "cut={cut} prior={prior:?} ending={ending:?}"
                );
            }
            let mut live = Utf8Document::new();
            for (at, ch) in source.char_indices() {
                live.append(&source.as_bytes()[at..at + ch.len_utf8()])
                    .unwrap();
                let prefix = &source[..at + ch.len_utf8()];
                let mut whole = Document::new();
                whole.append(prefix).unwrap();
                assert_eq!(
                    snapshot::snapshot(live.document()),
                    snapshot::snapshot(&whole),
                    "prefix={prefix:?}"
                );
                let mut eof = Utf8Document::new();
                for byte in prefix.bytes() {
                    eof.append(&[byte]).unwrap();
                }
                eof.finish();
                whole.finish();
                assert_eq!(
                    snapshot::snapshot(eof.document()),
                    snapshot::snapshot(&whole),
                    "EOF={prefix:?}"
                );
            }
            assert!(expected.work().header_replay_bytes <= source.len() as u64);
            assert!(
                expected.work().inline_bytes
                    <= source.len() as u64
                        + expected.work().header_replay_bytes
                        + expected.work().code_fallback_bytes
            );
        }
    }
}

#[test]
fn cell_block_like_prefixes_remain_inline_in_scope_composition_checks() {
    for text in [
        "# head",
        "> quote",
        "- item",
        "1. numbered",
        "```rust",
        "~~~",
        "[r]: /target",
        "[^n]: note",
        "---",
        "[r] *tail*",
        "trailing\\ ",
    ] {
        let early = parse(&format!("| {text} | right |\n|---|---|\n|{text}| body |\n"));
        scope_composition::assert_scope_composition(&early);
        for row in early.tables()[0].rows() {
            let c = row.cells[0].source;
            let actual: String = shown(&early, c).iter().map(|r| r.0).collect();
            let expected = if text == "[r] *tail*" {
                "[r] tail"
            } else if text == "trailing\\ " {
                "trailing\\"
            } else {
                text
            };
            assert_eq!(actual, expected, "literal block prefix {text:?}");
        }
        let late = parse(&format!(
            "*prior\nend*x* {}|{text}\n---|---\n",
            "z".repeat(1100)
        ));
        scope_composition::assert_scope_composition(&late);
        let second = late.tables()[0].cell(0, 1).unwrap().source;
        let actual: String = shown(&late, second).iter().map(|r| r.0).collect();
        assert_eq!(
            actual,
            if text == "[r] *tail*" {
                "[r] tail"
            } else if text == "trailing\\ " {
                "trailing\\"
            } else {
                text
            }
        );
    }
}

#[test]
fn split_scope_comparison_preserves_late_canonical_reference_context() {
    let source = format!(
        "*prior [r* z]\nend*x* {}|*foo [bar* baz]\n---|---\n\n[r* z]: /old\n[bar* baz]: /cell\n",
        "z".repeat(1100)
    );
    let d = parse(&source);
    scope_composition::assert_scope_composition(&d);
    let old_end = source.find('\n').unwrap();
    assert_eq!(
        shown(&d, Span::new(0, old_end))
            .iter()
            .map(|r| r.0)
            .collect::<String>(),
        "*prior r* z"
    );
    let cell = d.tables()[0].cell(0, 1).unwrap().source;
    assert_eq!(
        shown(&d, cell).iter().map(|r| r.0).collect::<String>(),
        "*foo bar* baz"
    );
    assert_eq!(
        active_links(&d, Span::new(0, old_end)),
        [(LinkKind::Link, "/old".to_owned())]
    );
    assert_eq!(
        active_links(&d, cell),
        [(LinkKind::Link, "/cell".to_owned())]
    );
}

#[test]
fn four_general_header_witnesses_have_fixed_handwritten_semantics() {
    let pad = "z".repeat(1100);
    for (prior, head, visible, maths, has_link, x_depth) in [
        (
            "*prior\n",
            "end*x* [h](v) $q$ &amp; ",
            "endx h q & ",
            vec![(MathKind::Inline, "q".to_owned())],
            true,
            1,
        ),
        (
            "`prior\n",
            "end` *x* [h](v) $q$ &amp; ",
            "end` x h q & ",
            vec![(MathKind::Inline, "q".to_owned())],
            true,
            1,
        ),
        // Unclosed dollar content retains the project's existing opaque fallback.
        (
            "$prior\n",
            "end$ *x* [h](v) &amp; ",
            "end$ *x* [h](v) &amp; ",
            vec![],
            false,
            0,
        ),
        (
            "[prior\n",
            "end](u*x*) *x* [h](v) &amp; ",
            "end](ux) x h & ",
            vec![],
            true,
            1,
        ),
    ] {
        let source = format!("{prior}{head}{pad}|b\n---|---\nx|y\n");
        let d = parse(&source);
        assert_eq!(d.tables().len(), 1);
        assert_eq!(d.source(), source);
        assert_eq!(
            shown(&d, Span::new(0, prior.len()))
                .iter()
                .map(|r| r.0)
                .collect::<String>(),
            prior
        );
        assert!(shown(&d, Span::new(0, prior.len())).iter().all(
            |r| !r.1.contains(Style::EMPHASIS)
                && !r.1.contains(Style::CODE)
                && !r.1.contains(Style::MATH)
        ));
        let cell = d.tables()[0].cell(0, 0).unwrap().source;
        assert_eq!(
            shown(&d, cell).iter().map(|r| r.0).collect::<String>(),
            format!("{visible}{pad}")
        );
        assert_eq!(
            active_links(&d, cell),
            if has_link {
                vec![(LinkKind::Link, "v".to_owned())]
            } else {
                vec![]
            }
        );
        assert_eq!(active_maths(&d, cell), maths);
        let xs: Vec<_> = shown(&d, cell).into_iter().filter(|r| r.0 == 'x').collect();
        assert!(!xs.is_empty());
        assert!(xs.iter().all(|r| r.2 == x_depth && r.3 == 0));
    }
}

fn assert_source_bounds(d: &Document) {
    let n = d.source().len() as u64;
    let w = d.work();
    let s = d.confirmation_stats();
    // Independent original-byte bounds: each decided physical row is replayed
    // once; at most three virtual columns replace one source tab.
    assert!(
        w.header_replay_bytes <= 4 * n,
        "replay {} > 4*{n}",
        w.header_replay_bytes
    );
    assert!(w.inline_bytes <= 8 * n, "{w:?}");
    assert!(w.code_fallback_bytes <= 8 * n, "{w:?}");
    assert!(w.style_suffix_bytes <= 8 * n, "{w:?}");
    assert!(
        w.style_events <= 16 * n && w.style_index_steps <= 24 * n,
        "{w:?}"
    );
    assert!(
        w.inline_checkpoint_steps + w.inline_rollback_steps <= 64 * n,
        "{w:?}"
    );
    let log = u64::BITS as u64 - n.leading_zeros() as u64 + 1;
    assert!(
        w.tree_visits + w.header_effect_steps <= 128 * n * log,
        "{w:?}"
    );
    let entries = s.inline_undo_peak
        + s.eligibility_undo_peak
        + s.annotation_undo_peak
        + s.style_undo_peak
        + s.parent_undo_peak
        + s.footnote_undo_peak
        + s.publication_peak;
    let bytes = s.inline_undo_bytes_peak
        + s.eligibility_undo_bytes_peak
        + s.annotation_undo_bytes_peak
        + s.style_undo_bytes_peak
        + s.parent_undo_bytes_peak;
    assert!(entries <= 128 * n as usize, "{s:?}");
    assert!(bytes <= 8192 * n as usize, "{s:?}");
    assert!(
        s.annotation_undo_peak <= 4 * n as usize,
        "first-write annotation records {s:?}"
    );
    assert!(
        s.style_event_peak <= 16 * n as usize && s.style_index_peak <= 16 * n as usize,
        "{s:?}"
    );
}

#[test]
fn cumulative_replay_and_actual_undo_index_peaks_have_original_byte_bounds() {
    for history in [0, 2048] {
        for depth in [0, 64] {
            for count in [64, 1024] {
                let p = "> ".repeat(depth);
                let mut d = Document::new();
                for _ in 0..history {
                    d.append("[hist](u) $h$ [^hist] *old*\n\n").unwrap();
                }
                let old = d.link_info(LinkId(0)).copied();
                for row in 0..16 {
                    let (prior, close) = match row % 4 {
                        0 => (format!("{}prior", "*old ".repeat(count)), "end*x* "),
                        1 => ("`prior".to_owned(), "end` *x* "),
                        2 => ("$prior".to_owned(), "end$ *x* "),
                        _ => ("[prior *a**b".to_owned(), "end](u*x*) *x* "),
                    };
                    d.append(&format!(
                        "{p}{prior}\n{p}{close}{} ",
                        "[h](v) $q$ [^n] &amp; *x* ".repeat(count)
                    ))
                    .unwrap();
                    assert_source_bounds(&d);
                    d.append("|b\n").unwrap();
                    d.append(&format!(
                        "{p}{}\n{p}\n",
                        if row % 2 == 0 { "---|---" } else { "wrong" }
                    ))
                    .unwrap();
                    assert_source_bounds(&d);
                }
                d.finish();
                assert_source_bounds(&d);
                assert_eq!(d.tables().len(), 8);
                if let Some(old) = old {
                    assert_eq!(d.link_info(LinkId(0)), Some(&old));
                }
                println!(
                    "confirmation source_bytes={} history={history} depth={depth} runs={count} work={:?} peak={:?}",
                    d.source().len(),
                    d.work(),
                    d.confirmation_stats()
                );
            }
        }
    }
}

#[test]
fn million_single_byte_appends_do_not_grow_an_endpoint_undo_log_or_replay_history() {
    let mut d = Document::new();
    d.append("*old\n").unwrap();
    d.append(&format!("end*x* [h](v) {}", "z".repeat(1100)))
        .unwrap();
    let before = d.confirmation_stats();
    let count = d.work();
    for _ in 0..1_000_000 {
        d.append("z").unwrap();
    }
    let grown = d.confirmation_stats();
    assert_eq!(d.work().header_replay_bytes, count.header_replay_bytes);
    assert_eq!(grown.inline_undo_peak, before.inline_undo_peak);
    assert_eq!(grown.eligibility_undo_peak, before.eligibility_undo_peak);
    d.append("|b\n---|---\n").unwrap();
    d.finish();
    assert_eq!(d.tables().len(), 1);
    assert_source_bounds(&d);
    println!(
        "million-append source_bytes={} work={:?} peak={:?}",
        d.source().len(),
        d.work(),
        d.confirmation_stats()
    );
}
