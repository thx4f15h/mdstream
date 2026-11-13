use mdstream::*;
include!("fixtures/indented_blank_model.rs");
fn parse(source: &str) -> Document {
    let mut d = Document::new();
    d.append(source).unwrap();
    d.finish();
    d
}
fn codes(d: &Document) -> Vec<String> {
    d.blocks()
        .iter()
        .filter(|b| matches!(b.kind, BlockKind::Code { .. }))
        .map(|b| {
            d.display_runs_in(b.source)
                .unwrap()
                .filter(|r| !r.style.contains(Style::HIDDEN))
                .map(|r| r.text)
                .collect()
        })
        .collect()
}
type Snapshot = (Vec<Block>, Vec<String>, Vec<(Span, String, bool)>);
fn snapshot(d: &Document) -> Snapshot {
    (
        d.blocks().to_vec(),
        codes(d),
        d.display_runs_in(Span::new(0, d.source().len()))
            .unwrap()
            .map(|r| (r.source, r.text.to_owned(), r.style.contains(Style::HIDDEN)))
            .collect(),
    )
}
#[test]
fn hand_authored_semantics_and_complete_source_partition() {
    for &(name, source, expected) in CASES {
        let d = parse(source);
        assert_eq!(codes(&d), expected, "{name}");
        let mut at = 0;
        for (i, b) in d.blocks().iter().enumerate() {
            assert_eq!(b.source.start, at, "{name}");
            assert_eq!(b.state, SyntaxState::Sealed);
            for p in b.source.range() {
                assert_eq!(d.block_at(p), Some(BlockId(i)), "{name}")
            }
            at = b.source.end
        }
        assert_eq!(at, source.len(), "{name}");
        assert!(d.indented_code_pending().is_none());
        assert_eq!(d.source(), source)
    }
}
#[test]
fn byte_cuts_prefix_eof_and_empty_appends() {
    for &(_, source, _) in CASES {
        let whole = snapshot(&parse(source));
        for i in 0..=source.len() {
            let mut d = Utf8Document::new();
            d.append(&source.as_bytes()[..i]).unwrap();
            d.append(&[]).unwrap();
            d.append(&source.as_bytes()[i..]).unwrap();
            d.finish();
            assert_eq!(snapshot(d.document()), whole, "{source:?} cut{i}");
            let mut p = Utf8Document::new();
            for b in &source.as_bytes()[..i] {
                p.append(&[*b]).unwrap();
            }
            p.finish();
            let prefix = String::from_utf8_lossy(&source.as_bytes()[..i]);
            assert_eq!(
                snapshot(p.document()),
                snapshot(&parse(&prefix)),
                "{source:?} prefix{i}"
            )
        }
    }
}
#[test]
fn pending_owner_stability_and_spacing_decide_before_siblings() {
    for first in ["    a\n", ">     a\n", "-     a\n", "> -     a\n"] {
        let blank = if first.starts_with('>') {
            ">\n>      \n"
        } else {
            "\n      \n"
        };
        let mut d = Document::new();
        d.append(first).unwrap();
        let id = BlockId(0);
        let source = d.block(id).unwrap().source;
        let u = d.append(blank).unwrap();
        assert!(u.new_blocks.is_empty());
        assert_eq!(d.block(id).unwrap().source, source);
        assert_eq!(d.block(id).unwrap().state, SyntaxState::Growing);
        let p = d.indented_code_pending().unwrap();
        assert_eq!(p.owner, id);
        assert_eq!(p.source, Span::new(first.len(), first.len() + blank.len()));
        for at in p.source.range() {
            assert_eq!(d.block_at(at), None)
        }
        assert_eq!(d.next_visible(p.source.start), None);
        assert!(
            d.display_runs_in(p.source)
                .unwrap()
                .all(|r| r.style.contains(Style::HIDDEN))
        );
        assert!(!d.text(p.source).unwrap().is_empty());
        let end = d.finish();
        assert!(end.changed_blocks.contains(&0));
        assert_eq!(d.block(id).unwrap().source, source);
        assert_eq!(d.block(id).unwrap().state, SyntaxState::Sealed);
        assert_eq!(d.blocks().len(), 3);
        assert!(d.indented_code_pending().is_none());
    }
    for (source, expected) in [
        ("-     a\n\n      b\n", ListTightness::Tight),
        ("-     a\n\n- b\n", ListTightness::Loose),
        ("-     a\n\n  text\n", ListTightness::Loose),
        ("- before\n\n      a\n\n      b\n", ListTightness::Loose),
    ] {
        let d = parse(source);
        assert_eq!(
            d.container(ContainerId(0)).unwrap().list_tightness,
            Some(expected),
            "{source}"
        )
    }
}
fn tiny() -> Budget {
    Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 6,
        tiles: 1,
        recovery: 1,
        dependencies: 1,
    })
}
fn cursor_visible(d: &Document, span: Span) -> String {
    let mut c = d.display_cursor(span).unwrap();
    let at = c.source_position();
    assert_eq!(
        c.next(d, &mut Budget::new(WorkLimit::default())).unwrap(),
        DisplayStep::BudgetExhausted
    );
    assert_eq!(at, c.source_position());
    let mut text = String::new();
    for _ in 0..(span.len() + 1) * 30 {
        match c.next(d, &mut tiny()).unwrap() {
            DisplayStep::Run(r) => {
                if !r.style.contains(Style::HIDDEN) {
                    text.push_str(r.text)
                }
            }
            DisplayStep::Complete => return text,
            DisplayStep::BudgetExhausted => {}
        }
    }
    panic!("micro cursor stalled")
}
#[test]
fn real_pending_visibility_payload_mapping_and_update_tickets() {
    let mut d = Document::new();
    d.enable_display_shape_effects().unwrap();
    let a = d.append(">\t\t界🙂\r\n").unwrap();
    let id = BlockId(0);
    let old = d.block(id).unwrap().source;
    let mut h = HeightIndex::new(1).unwrap();
    h.rebuild(&d).unwrap();
    let ticket = h.ticket(TileId(0)).unwrap();
    h.measure(ticket, 1).unwrap();
    let mut cp = CheckpointIndex::new();
    cp.rebuild(&d);
    cp.insert(TileId(0), 7usize).unwrap();
    let token = cp.token(TileId(0)).unwrap();
    let b = d.append(">\t\t\r\n").unwrap();
    h.apply(&d, &b).unwrap();
    cp.apply(&d, &b).unwrap();
    assert!(!cp.validates(token));
    assert!(h.measure(ticket, 1).is_err());
    let p = d.indented_code_pending().unwrap();
    assert_eq!(cursor_visible(&d, p.source), "");
    assert_eq!(d.text(p.source), Some(">\t\t\r\n"));
    assert_eq!(d.block(id).unwrap().source, old);
    assert!(
        b.display_shape
            .events()
            .iter()
            .any(|e| matches!(e.kind, DisplayShapeEffectKind::Structure { .. }))
    );
    let c = d.append(">\t\té\t尾 \r\n").unwrap();
    assert!(c.new_blocks.is_empty());
    assert!(c.changed_blocks.contains(&0));
    assert!(d.indented_code_pending().is_none());
    assert_eq!(
        cursor_visible(&d, d.block(id).unwrap().source),
        "  界🙂\n  \n  é\t尾 \n"
    );
    assert_eq!(codes(&d), ["  界🙂\n  \n  é\t尾 \n"]);
    let source = d.block(id).unwrap().source;
    assert_eq!(source, Span::new(0, d.source().len()));
    let fold = d.fold_candidate(FoldId::Code(id)).unwrap();
    assert_eq!(fold.source, source);
    let mut merged = a;
    merged.merge(b).unwrap();
    merged.merge(c).unwrap();
    assert_eq!(merged.appended, source);
    let before = d.work();
    let _ = cursor_visible(&d, source);
    assert_eq!(before, d.work());
}
#[test]
fn typed_body_pending_and_child_deltas_share_the_actual_machine() {
    let mut d = Document::new();
    d.enable_display_shape_effects().unwrap();
    d.append("[^n]:     a\n    \n").unwrap();
    // The outer projection flushes its final LF only when the next admitted line arrives.
    let u = d.append("    \n").unwrap();
    let v = d.footnote_body(FootnoteId(0)).unwrap();
    let p = v
        .indented_code_pending()
        .expect("actual child blank ledger");
    let id = p.id(p.value.owner);
    let source = p.span(p.value.source);
    assert!(v.block(id).unwrap().is_some());
    assert_eq!(v.code_blank_storage().records, 1);
    let mut heights = BodyHeights::new(v, 1).unwrap();
    let mut cps = BodyCheckpoints::<usize>::new(v);
    assert!(!u.body_updates.is_empty());
    let u = d.append("        b\n").unwrap();
    let v = d.footnote_body(FootnoteId(0)).unwrap();
    heights.apply(v, &u).unwrap();
    cps.apply(v, &u).unwrap();
    assert!(v.indented_code_pending().is_some());
    let u = d.append("outside\n").unwrap();
    let v = d.footnote_body(FootnoteId(0)).unwrap();
    heights.apply(v, &u).unwrap();
    cps.apply(v, &u).unwrap();
    assert!(v.indented_code_pending().is_none());
    let mut cur = v.display_cursor();
    let mut text = String::new();
    for _ in 0..200 {
        match cur.next(v, &mut tiny()).unwrap() {
            BodyDisplayStep::Run(r) => {
                if !r.style.contains(Style::HIDDEN) {
                    text.push_str(r.text)
                }
            }
            BodyDisplayStep::Complete => break,
            BodyDisplayStep::BudgetExhausted => {}
        }
    }
    assert_eq!(text, "a\n\n\nb");
    assert_eq!(
        d.footnote_body_work().engine.code_blank_admissions,
        v.work().code_blank_admissions
    );
    let _ = source;
    let u = d.finish();
    let v = d.footnote_body(FootnoteId(0)).unwrap();
    heights.apply(v, &u).unwrap();
    cps.apply(v, &u).unwrap();
    assert!(v.indented_code_pending().is_none());
}
#[test]
fn deep_omitted_blanks_retire_once_and_ordinary_prose_remains_eager() {
    for depth in [0, 1000] {
        let mut d = Document::new();
        d.append(&format!("{}    a\n", "- ".repeat(depth))).unwrap();
        let before = d.work();
        for _ in 0..10000 {
            d.append("\n").unwrap();
        }
        let pending = d.work();
        assert_eq!(
            pending.code_blank_admissions - before.code_blank_admissions,
            10000
        );
        assert_eq!(pending.code_blank_retirements, 0);
        assert!(pending.container_steps - before.container_steps < 40000);
        assert!(d.code_blank_storage().capacity_bytes < 10000 * 128);
        d.append(&format!("{}    b\n", "  ".repeat(depth))).unwrap();
        assert_eq!(d.blocks().len(), 1);
        assert_eq!(d.work().code_blank_decisions, 1);
        assert_eq!(d.work().code_blank_retirements, 10000);
        assert_eq!(d.code_blank_storage().records, 0);
        assert_eq!(d.work().inline_bytes, 0);
        assert_eq!(d.work().root_eligibility_bytes, 0);
        assert_eq!(d.work().container_eligibility_bytes, 0);
    }
    let mut d = Document::new();
    d.append(&"p".repeat(100000)).unwrap();
    assert_eq!(d.work().inline_bytes, 100000);
    assert_eq!(cursor_visible(&d, Span::new(0, 1)), "p");
    assert_eq!(d.code_blank_storage(), CodeBlankStorage::default());
}
#[test]
fn exact_handwritten_raw_crlf_unicode_and_partial_tab_mapping() {
    for (source, expected) in [
        (
            "    a\r\n\r\n    b\r\n",
            vec![
                ("a", Span::new(4, 5)),
                ("\n", Span::new(6, 7)),
                ("\n", Span::new(8, 9)),
                ("b", Span::new(13, 14)),
                ("\n", Span::new(15, 16)),
            ],
        ),
        (
            ">\t\ta\n>\t\t\n>\t\tb\n",
            vec![
                ("  ", Span::new(2, 3)),
                ("a", Span::new(3, 4)),
                ("\n", Span::new(4, 5)),
                ("  ", Span::new(7, 8)),
                ("\n", Span::new(8, 9)),
                ("  ", Span::new(11, 12)),
                ("b", Span::new(12, 13)),
                ("\n", Span::new(13, 14)),
            ],
        ),
        (
            "    界🙂\n \t \n    é\t尾 \n",
            vec![
                ("界", Span::new(4, 7)),
                ("🙂", Span::new(7, 11)),
                ("\n", Span::new(11, 12)),
                (" ", Span::new(14, 15)),
                ("\n", Span::new(15, 16)),
                ("é", Span::new(20, 22)),
                ("\t", Span::new(22, 23)),
                ("尾", Span::new(23, 26)),
                (" ", Span::new(26, 27)),
                ("\n", Span::new(27, 28)),
            ],
        ),
    ] {
        let d = parse(source);
        let mut got = vec![];
        for r in d
            .display_runs_in(Span::new(0, source.len()))
            .unwrap()
            .filter(|r| !r.style.contains(Style::HIDDEN))
        {
            if let Some(replacement) = r.replacement {
                got.push((r.text.to_owned(), replacement));
            } else {
                for (offset, ch) in r.text.char_indices() {
                    got.push((
                        ch.to_string(),
                        Span::new(
                            r.source.start + offset,
                            r.source.start + offset + ch.len_utf8(),
                        ),
                    ))
                }
            }
        }
        let expected = expected
            .iter()
            .map(|(s, p)| (s.to_string(), *p))
            .collect::<Vec<_>>();
        assert_eq!(got, expected, "{source:?}");
    }
}
#[test]
fn partial_tab_blank_retains_omitted_lists_but_not_missing_quotes() {
    for (source, expected) in [
        ("- - -     a\n\t\n          b\n", vec!["a\n\nb\n"]),
        ("> - - -     a\n>\t\n>           b\n", vec!["a\n\nb\n"]),
        ("- > - -     a\n\t\n  > - -     b\n", vec!["a\n", "b\n"]),
        ("- - -     a\n\t\n- sibling\n", vec!["a\n"]),
    ] {
        let d = parse(source);
        assert_eq!(codes(&d), expected, "{source:?}");
        for split in 0..=source.len() {
            let mut streamed = Document::new();
            streamed.append(&source[..split]).unwrap();
            streamed.append(&source[split..]).unwrap();
            streamed.finish();
            assert_eq!(snapshot(&streamed), snapshot(&d), "{source:?} cut{split}");
        }
        if source.ends_with("- sibling\n") {
            assert_eq!(
                d.container(ContainerId(0)).unwrap().list_tightness,
                Some(ListTightness::Loose)
            );
            let blank = d
                .blocks()
                .iter()
                .find(|b| b.kind == BlockKind::Blank)
                .unwrap();
            assert_eq!(blank.container, Some(ContainerId(5)));
        }
    }
}
#[test]
fn nested_blank_indentation_matches_independent_physical_column_oracle() {
    let whitespace =
        (0..=20)
            .map(|n| " ".repeat(n))
            .chain((1..=8).map(|n| "\t".repeat(n)))
            .chain((0..=3).flat_map(|a| {
                (0..=3).map(move |b| format!("{}\t{}", " ".repeat(a), " ".repeat(b)))
            }))
            .collect::<Vec<_>>();
    for depth in 1..=10 {
        for quote in [false, true] {
            let q = if quote { "> " } else { "" };
            for blank in &whitespace {
                let source = format!(
                    "{q}{}    a\n{q}{blank}\n{q}{}    b\n",
                    "- ".repeat(depth),
                    "  ".repeat(depth)
                );
                let mut column = if quote { 2 } else { 0 };
                let target = column + 2 * depth + 4;
                let mut end = 0;
                while end < blank.len() && column < target {
                    column += if blank.as_bytes()[end] == b'\t' {
                        4 - column % 4
                    } else {
                        1
                    };
                    end += 1;
                }
                let retained = format!(
                    "{}{}",
                    " ".repeat(column.saturating_sub(target)),
                    &blank[end..]
                );
                let expected = format!("a\n{retained}\nb\n");
                let d = parse(&source);
                assert_eq!(
                    codes(&d),
                    [expected],
                    "depth={depth} quote={quote} blank={blank:?}"
                );
                assert_eq!(d.blocks().len(), 1);
            }
        }
    }
}
