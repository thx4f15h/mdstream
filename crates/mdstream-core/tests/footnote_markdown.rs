use mdstream::*;
fn unlimited() -> Budget {
    Budget::new(WorkLimit::unlimited())
}
fn tiny() -> Budget {
    Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 6,
        dependencies: 1,
        recovery: 1,
        tiles: 1,
    })
}
fn body_text(doc: &Document, symbol: FootnoteId) -> String {
    let view = doc.footnote_body(symbol).unwrap();
    let mut cursor = view.display_cursor();
    let mut out = String::new();
    let mut frames = 0;
    loop {
        frames += 1;
        assert!(frames < 10000);
        match cursor.next(view, &mut tiny()).unwrap() {
            BodyDisplayStep::Run(run) => {
                if !run.style.contains(Style::HIDDEN) {
                    out.push_str(run.text);
                }
            }
            BodyDisplayStep::BudgetExhausted => {}
            BodyDisplayStep::Complete => break,
        }
    }
    out
}
#[test]
fn real_body_reuses_inline_and_block_markdown() {
    let mut d = Document::new();
    d.append("[^n]: *em* **strong** ~~strike~~ `code` &amp; \\* $x$\n\n    # Heading\n\n    - item\n    - other\n").unwrap();
    d.finish();
    assert_eq!(
        body_text(&d, FootnoteId(0)),
        "em strong strike code & * x\n\nHeading\n\nitem\nother"
    );
    assert_eq!(d.blocks().len(), 1);
    assert!(d.links().is_empty());
    assert!(d.maths().is_empty());
}
#[test]
fn closed_body_reads_a_late_host_link_definition() {
    let mut d = Document::new();
    d.append("[^n]: [label][r]\nafter\n").unwrap();
    assert_eq!(body_text(&d, FootnoteId(0)), "[label][r]");
    d.append("\n[r]: /late\n").unwrap();
    assert_eq!(body_text(&d, FootnoteId(0)), "label");
}
#[test]
fn body_definition_resolves_an_earlier_root_use() {
    let mut d = Document::new();
    d.append("[label][r]\n\n[^n]: [r]: /inside\n").unwrap();
    d.finish();
    assert_eq!(d.link_destination(LinkId(0)), Some("/inside"));
    assert_eq!(body_text(&d, FootnoteId(0)), "");
}
#[test]
fn only_nested_note_marker_is_literal() {
    let mut d = Document::new();
    d.append("[^n]: [^b]: *em* &amp; [link](/u) [^c] $x$\n")
        .unwrap();
    d.finish();
    assert_eq!(body_text(&d, FootnoteId(0)), "[^b]: em & link [^c] x");
    assert_eq!(
        d.footnotes()
            .iter()
            .map(|n| n.label.as_str())
            .collect::<Vec<_>>(),
        ["n", "c"]
    );
    assert_eq!(d.footnotes()[1].number, Some(1));
    assert!(d.footnote_references().is_empty());
}

fn parse(s: &str) -> Document {
    let mut d = Document::new();
    d.append(s).unwrap();
    d.finish();
    d
}
fn resolution(view: BodyView<'_>, local: usize) -> FootnoteResolution {
    match view
        .resolve_footnote(view.qualify(FootnoteReferenceId(local)), &mut unlimited())
        .unwrap()
    {
        BodyResolveStep::Resolved(Some(r)) => r,
        _ => panic!("expected active occurrence"),
    }
}
#[test]
fn body_references_share_global_numbers_and_host_eof() {
    let mut d = Document::new();
    d.append("[^a]: [^b] [^a] [^b]\noutside\n").unwrap();
    let view = d.footnote_body(FootnoteId(0)).unwrap();
    let first = resolution(view, 0);
    assert_eq!(first.number, 1);
    assert_eq!(first.target, FootnoteTarget::Unresolved);
    assert_eq!(resolution(view, 1).number, 2);
    assert_eq!(resolution(view, 2).number, 1);
    assert!(d.footnote_references().is_empty());
    d.append("\n[^b]: [^a]\n").unwrap();
    d.finish();
    let view = d.footnote_body(FootnoteId(0)).unwrap();
    assert!(matches!(
        resolution(view, 0).target,
        FootnoteTarget::Defined(_)
    ));
    let b = d.footnote_body(first.symbol).unwrap();
    assert_eq!(resolution(b, 0).number, 2);
    let mut missing = Document::new();
    missing.append("[^n]: [^missing]\noutside\n").unwrap();
    assert_eq!(
        resolution(missing.footnote_body(FootnoteId(0)).unwrap(), 0).target,
        FootnoteTarget::Unresolved
    );
    missing.finish();
    assert_eq!(
        resolution(missing.footnote_body(FootnoteId(0)).unwrap(), 0).target,
        FootnoteTarget::Missing
    );
}
#[test]
fn duplicate_notes_do_not_publish_child_symbols() {
    let d = parse("[^n]: *winner*\n[^n]: [^evil] [x][bad]\n\n    [bad]: /evil\n");
    assert_eq!(body_text(&d, FootnoteId(0)), "winner");
    assert_eq!(d.footnotes().len(), 1);
    assert!(d.references().is_empty());
}
#[test]
fn definition_winners_follow_original_positions_across_namespaces() {
    for (source, expected) in [
        ("[l][r]\n\n[r]: /root\n\n[^n]: [r]: /body\n", "/root"),
        ("[l][r]\n\n[^n]: [r]: /body\n[r]: /root\n", "/body"),
        ("[l][r]\n\n[^n]: [r]: /first\n    [r]: /second\n", "/first"),
    ] {
        let d = parse(source);
        assert_eq!(d.link_destination(LinkId(0)), Some(expected), "{source}");
    }
}
#[test]
fn nested_markers_do_not_make_the_rest_of_body_opaque() {
    let d = parse("[^n]: [^b]: *em*\n\n    # Head\n\n    [^c]: **strong**\n");
    assert_eq!(
        body_text(&d, FootnoteId(0)),
        "[^b]: em\n\nHead\n\n[^c]: strong"
    );
    assert_eq!(d.footnotes().len(), 1);
    assert_eq!(d.footnote_body(FootnoteId(0)).unwrap().heading_count(), 1);
}
fn mappings(view: BodyView<'_>, address: BodyAddress) -> Vec<BodyMapping> {
    let mut t = view.map(address).unwrap();
    let mut out = Vec::new();
    let mut z = Budget::new(WorkLimit::default());
    assert!(matches!(
        t.advance(view, &mut z).unwrap(),
        BodyMapStep::BudgetExhausted | BodyMapStep::Complete
    ));
    assert_eq!(z.used(), WorkLimit::default());
    let mut frames = 0;
    loop {
        frames += 1;
        assert!(frames < 10000);
        let mut b = tiny();
        match t.advance(view, &mut b).unwrap() {
            BodyMapStep::Mapping(m) => out.push(m),
            BodyMapStep::BudgetExhausted => {}
            BodyMapStep::Complete => break,
        }
    }
    out
}
#[test]
fn root_body_mapping_preserves_original_and_partial_tabs() {
    let source = "- [^n]: first\r\n\t\t中🙂 &amp; ` a `\r\n";
    let d = parse(source);
    assert_eq!(d.source(), source);
    let view = d.footnote_body(FootnoteId(0)).unwrap();
    let ms = mappings(view, BodyAddress::Projected(view.range()));
    assert_eq!(
        ms.iter()
            .filter(|m| m.kind == BodyMappingKind::VirtualTab)
            .map(|m| (d.text(m.original).unwrap(), m.projected.span.len()))
            .collect::<Vec<_>>(),
        [("\t", 2)]
    );
    let mut projection = String::new();
    for m in &ms {
        match m.kind {
            BodyMappingKind::Raw => projection.push_str(d.text(m.original).unwrap()),
            BodyMappingKind::VirtualTab => projection.push_str(&"   "[..m.projected.span.len()]),
            _ => {}
        }
    }
    assert_eq!(projection, "first\r\n  中🙂 &amp; ` a `");
    let original = mappings(
        view,
        BodyAddress::Original {
            document: d.id(),
            span: d.blocks()[0].source,
        },
    );
    assert_eq!(original.first().unwrap().kind, BodyMappingKind::Hidden);
    assert_eq!(original.last().unwrap().kind, BodyMappingKind::Hidden);
    let mut at = 0;
    for m in original {
        assert_eq!(m.original.start, at);
        at = m.original.end;
    }
    assert_eq!(at, source.len());
    assert_eq!(body_text(&d, FootnoteId(0)), "first\n中🙂 & a");
}
#[test]
fn map_caret_bias_and_namespace_errors_are_explicit() {
    let d = parse("[^n]: first\n[^m]: next\n");
    let v = d.footnote_body(FootnoteId(0)).unwrap();
    let address = BodyAddress::Projected(BodySpan {
        body: v.key(),
        span: Span::new(0, 0),
    });
    for (bias, expected) in [(BodyBias::Left, 0), (BodyBias::Right, 6)] {
        let mut t = v.map_caret(address, bias).unwrap();
        loop {
            match t.advance(v, &mut tiny()).unwrap() {
                BodyMapStep::Mapping(m) => {
                    assert_eq!(m.original, Span::new(expected, expected));
                    break;
                }
                BodyMapStep::BudgetExhausted => {}
                _ => panic!(),
            }
        }
    }
    let other = d.footnote_body(FootnoteId(1)).unwrap();
    let mut task = v.map(BodyAddress::Projected(v.range())).unwrap();
    let mut z = Budget::new(WorkLimit::default());
    assert_eq!(task.advance(other, &mut z), Err(LayoutError::InvalidRange));
    assert_eq!(z.used(), WorkLimit::default());
    let foreign = parse("[^n]: first\n");
    assert_eq!(
        task.advance(foreign.footnote_body(FootnoteId(0)).unwrap(), &mut z),
        Err(LayoutError::WrongDocument)
    );
}
#[derive(Default)]
struct Sink {
    objects: Vec<(BodyId<LinkId>, LinkDisposition)>,
    bytes: usize,
}
impl BodyLayoutSink for Sink {
    fn begin_tile(&mut self, _: BodyId<TileId>) {}
    fn text(&mut self, r: BodyRun<'_>) {
        self.bytes += r.text.len();
    }
    fn object(&mut self, id: BodyId<LinkId>, _: BodyObject<&LinkInfo>, status: LinkDisposition) {
        self.objects.push((id, status));
    }
    fn end_tile(&mut self, _: BodyId<TileId>) -> u64 {
        1
    }
}
#[test]
fn actual_body_layout_search_dependencies_and_stale_tasks_share_budget() {
    let mut d = Document::new();
    d.append("[^n]: *hit* [label][r] [^missing]\noutside\n")
        .unwrap();
    let v = d.footnote_body(FootnoteId(0)).unwrap();
    let mut heights = BodyHeights::new(v, 1).unwrap();
    let ticket = heights.ticket(v, v.qualify(TileId(0))).unwrap().unwrap();
    let mut pass = BodyLayoutPass::new(v, &heights, v.qualify(TileId(0)), 100, false).unwrap();
    let mut sink = Sink::default();
    let mut z = Budget::new(WorkLimit::default());
    assert_eq!(
        pass.advance(v, &mut heights, &mut z, &mut sink).unwrap(),
        Completion::BudgetExhausted
    );
    assert_eq!(sink.bytes, 0);
    for frame in 0..10000 {
        let mut b = tiny();
        let status = pass.advance(v, &mut heights, &mut b, &mut sink).unwrap();
        assert!(b.used().queries <= 1 && b.used().objects <= 1 && b.used().bytes <= 6);
        if status != Completion::BudgetExhausted {
            break;
        }
        assert!(frame < 9999);
    }
    assert_eq!(sink.objects.len(), 1);
    assert_eq!(sink.objects[0].1, LinkDisposition::Unresolved);
    let mut search = v.display_search("hit").unwrap();
    let mut hits = Vec::new();
    for frame in 0..10000 {
        let mut b = tiny();
        let status = search.advance(v, &mut b, &mut hits).unwrap();
        if status != Completion::BudgetExhausted {
            break;
        }
        assert!(frame < 9999);
    }
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].body, v.key());
    let mut capture = v.dependency_capture(v.qualify(TileId(0))).unwrap();
    let key = loop {
        match capture.advance(v, &mut tiny()).unwrap() {
            BodyDependencyStep::Captured(k) => break k,
            BodyDependencyStep::BudgetExhausted => {}
            _ => panic!(),
        }
    };
    assert_eq!(
        key.validate(v, &mut unlimited()).unwrap(),
        CacheValidity::Valid
    );
    d.append("\n[r]: /late\n").unwrap();
    let v = d.footnote_body(FootnoteId(0)).unwrap();
    assert_eq!(
        pass.advance(v, &mut heights, &mut z, &mut sink),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(
        search.advance(v, &mut z, &mut hits),
        Err(LayoutError::StaleContinuation)
    );
    assert!(matches!(
        capture.advance(v, &mut z),
        Err(LayoutError::StaleContinuation)
    ));
    assert!(heights.measure(v, ticket, 9).is_err());
    assert_eq!(
        key.validate(v, &mut unlimited()).unwrap(),
        CacheValidity::Changed
    );
    let mut memo = pass.take_link_resolver();
    let n = memo.memo_entries();
    memo.retire(&mut z);
    assert_eq!(memo.memo_entries(), n);
    while memo.retire(&mut tiny()) != Completion::Covered {}
    assert_eq!(memo.memo_entries(), 0);
    while search.retire(&mut tiny()) != Completion::Covered {}
    while capture.retire(&mut tiny()) != Completion::Covered {}
}
#[test]
fn sealed_body_missing_dependency_observes_host_finish() {
    let mut d = Document::new();
    d.append("[^n]: [^missing]\noutside\n").unwrap();
    let v = d.footnote_body(FootnoteId(0)).unwrap();
    let mut capture = v.dependency_capture(v.qualify(TileId(0))).unwrap();
    let key = loop {
        match capture.advance(v, &mut tiny()).unwrap() {
            BodyDependencyStep::Captured(k) => break k,
            BodyDependencyStep::BudgetExhausted => {}
            _ => panic!(),
        }
    };
    d.finish();
    let v = d.footnote_body(FootnoteId(0)).unwrap();
    assert_eq!(
        key.validate(v, &mut unlimited()).unwrap(),
        CacheValidity::Changed
    );
    assert_eq!(resolution(v, 0).target, FootnoteTarget::Missing);
}

fn read_target(mut c: BodyTextCursor<'_>) -> (String, Vec<BodyAddress>) {
    let mut zero = Budget::new(WorkLimit::default());
    assert!(matches!(
        c.advance(&mut zero),
        Err(Completion::BudgetExhausted)
    ));
    assert_eq!(zero.used(), WorkLimit::default());
    let mut s = String::new();
    let mut source = Vec::new();
    for frame in 0..10000 {
        let mut b = tiny();
        match c.advance(&mut b) {
            Ok(Some(c)) => {
                s.push_str(c.text);
                source.push(c.source);
            }
            Ok(None) => return (s, source),
            Err(Completion::BudgetExhausted) => {}
            _ => panic!(),
        };
        assert!(b.used().queries <= 1 && b.used().objects <= 1 && b.used().bytes <= 6);
        assert!(frame < 9999)
    }
    unreachable!()
}
#[test]
fn canonical_targets_titles_and_direct_targets_have_distinct_typed_sources() {
    let source = "[root][r]\n\n- [^n]: [r]: /a\\+b?x=1&amp;y=2 'start\n\t\tend &amp;'\n- outside\n\n[^m]: [use][r] [direct](/inline \"t&amp;\")\n";
    let d = parse(source);
    let m = d.footnote_body(FootnoteId(1)).unwrap();
    let mut w = m.link_window(m.range()).unwrap();
    let mut seen = 0;
    loop {
        let (id, info) = match w.advance(m, &mut unlimited()).unwrap() {
            BodyLinkStep::Object(id, info) => (id, info),
            BodyLinkStep::Complete => break,
            _ => panic!(),
        };
        let label = read_target(m.source_cursor(info.span(info.value.label)).unwrap()).0;
        let expected = if label == "use" {
            "/a+b?x=1&y=2"
        } else if label == "direct" {
            "/inline"
        } else {
            continue;
        };
        let (value, addresses) = read_target(m.decoded_link_destination(id).unwrap().unwrap());
        assert_eq!(value, expected);
        if label == "use" {
            assert!(
                addresses
                    .iter()
                    .all(|a| matches!(a,BodyAddress::Original{document,..} if *document==d.id()))
            );
            assert_eq!(
                read_target(m.decoded_link_title(id).unwrap().unwrap()).0,
                "start\n  end &"
            );
        } else {
            assert!(
                addresses
                    .iter()
                    .all(|a| matches!(a,BodyAddress::Projected(s) if s.body==m.key()))
            );
            assert_eq!(
                read_target(m.decoded_link_title(id).unwrap().unwrap()).0,
                "t&"
            );
        }
        seen += 1;
    }
    assert_eq!(seen, 2);
    let mut root = d.decoded_link_title(LinkId(0)).unwrap();
    let mut out = String::new();
    loop {
        match root.advance(&mut tiny()) {
            Ok(Some(c)) => out.push_str(c.text),
            Ok(None) => break,
            Err(Completion::BudgetExhausted) => {}
            _ => panic!(),
        }
    }
    assert_eq!(out, "start\n  end &");
    assert!(d.link_title(LinkId(0)).unwrap().contains("\t\t"));
    assert_eq!(d.source(), source);
}
#[test]
fn multi_dependency_validation_progresses_one_fact_per_shared_frame() {
    let d = parse("[^n]: [a][x] [b][y] [^c] [^d]\n");
    let v = d.footnote_body(FootnoteId(0)).unwrap();
    let mut capture = v.dependency_capture(v.qualify(TileId(0))).unwrap();
    let key = loop {
        match capture.advance(v, &mut tiny()).unwrap() {
            BodyDependencyStep::Captured(k) => break k,
            BodyDependencyStep::BudgetExhausted => {}
            _ => panic!(),
        }
    };
    let mut task = key.validation();
    let mut z = Budget::new(WorkLimit::default());
    assert_eq!(
        task.advance(v, &mut z).unwrap(),
        CacheValidity::BudgetExhausted
    );
    assert_eq!(z.used(), WorkLimit::default());
    let mut frames = 0;
    loop {
        frames += 1;
        assert!(frames < 100);
        let mut b = tiny();
        match task.advance(v, &mut b).unwrap() {
            CacheValidity::Valid => break,
            CacheValidity::BudgetExhausted => assert_eq!(b.used().queries, 1),
            _ => panic!(),
        }
    }
    assert!(frames > 2);
}
#[test]
fn canonical_conditional_style_scope_uses_late_host_targets_and_shared_memo() {
    let mut d = Document::new();
    d.append("[^n]: *foo [bar* baz]\nend\n").unwrap();
    assert_eq!(body_text(&d, FootnoteId(0)), "foo [bar baz]");
    let old = d.footnote_body(FootnoteId(0)).unwrap();
    let mut cursor = old.display_cursor();
    let mut resolver = old.link_resolver();
    loop {
        if matches!(
            cursor
                .next_with_resolver(old, &mut resolver, &mut tiny())
                .unwrap(),
            BodyDisplayStep::Complete
        ) {
            break;
        }
    }
    d.append("\n[bar* baz]: /target\n").unwrap();
    let v = d.footnote_body(FootnoteId(0)).unwrap();
    let mut z = Budget::new(WorkLimit::default());
    assert!(matches!(
        cursor.next_with_resolver(v, &mut resolver, &mut z),
        Err(LayoutError::StaleContinuation)
    ));
    while resolver.retire(&mut tiny()) != Completion::Covered {}
    v.reset_link_resolver(&mut resolver).unwrap();
    assert_eq!(body_text(&d, FootnoteId(0)), "*foo bar* baz");
    let mut path = v
        .style_path(BodySpan {
            body: v.key(),
            span: Span::new(7, 7),
        })
        .unwrap()
        .unwrap();
    loop {
        match path.advance(v, &mut resolver, &mut tiny()).unwrap() {
            BodyStyleStep::Pair(_) => panic!("active label must isolate the old opener"),
            BodyStyleStep::BudgetExhausted => {}
            BodyStyleStep::Complete => break,
        }
    }
    while path.retire(&mut tiny()) != Completion::Covered {}
    while resolver.retire(&mut tiny()) != Completion::Covered {}
}
#[test]
fn nested_literal_marker_can_start_a_real_table_without_opaque_suffix() {
    let d = parse("[^n]: [^b]: *em*|[l](/u)\n    ---|---\n    x|y\n");
    let v = d.footnote_body(FootnoteId(0)).unwrap();
    assert_eq!(v.table_count(), 1);
    assert_eq!(d.footnotes().len(), 1);
    let text = body_text(&d, FootnoteId(0));
    assert!(text.contains("[^b]: em"));
    assert!(!text.contains("*em*"));
}

#[test]
fn general_header_transactions_preserve_host_numbering_definition_growth_and_raw_ids() {
    let mut d = Document::new();
    d.append("[^n]\n\n[^n]: $prior\n").unwrap();
    d.append(&format!(
        "    end$ [h](v) $q$ [^n] [^c] {}",
        "x".repeat(1800)
    ))
    .unwrap();
    let v = d.footnote_body(FootnoteId(0)).unwrap();
    assert_eq!(
        v.math_state(v.qualify(MathId(0))).unwrap(),
        Some(SyntaxState::Provisional)
    );
    assert_eq!(resolution(v, 0).number, 1);
    assert_eq!(resolution(v, 1).number, 2);
    d.append("|b\n    ---|---\n[^d]\n").unwrap();
    d.finish();
    let v = d.footnote_body(FootnoteId(0)).unwrap();
    assert_eq!(v.table_count(), 1);
    assert_eq!(v.math_state(v.qualify(MathId(0))).unwrap(), None);
    assert_eq!(
        v.footnote_reference_state(v.qualify(FootnoteReferenceId(0)))
            .unwrap(),
        None
    );
    assert!(matches!(
        v.resolve_footnote(v.qualify(FootnoteReferenceId(0)), &mut tiny())
            .unwrap(),
        BodyResolveStep::Resolved(None)
    ));
    let mut refs = v.footnote_references_in(v.range()).unwrap();
    assert!(refs.advance(&mut tiny()).unwrap().is_none());
    assert_eq!(
        d.footnotes()
            .iter()
            .map(|n| (n.label.as_str(), n.number))
            .collect::<Vec<_>>(),
        [("n", Some(1)), ("c", None), ("d", Some(2))]
    );
    let def = d.footnote(FootnoteId(0)).unwrap().definition.unwrap();
    let bytes = d.block(def.block).unwrap().source.len();
    assert_eq!(
        d.footnote(FootnoteId(0)).unwrap().revision,
        bytes as u64 + 3
    );
    let mut maths = v.maths_in(v.range()).unwrap();
    let (id, math) = maths.advance(&mut unlimited()).unwrap().unwrap();
    assert_eq!(math.body, v.key());
    let mut payload = v.math_payload(id).unwrap().unwrap();
    let mut text = String::new();
    while let Some(c) = payload.advance(&mut tiny()).unwrap() {
        text.push_str(c.text)
    }
    assert_eq!(text, " [h](v) ");
    assert!(maths.advance(&mut unlimited()).unwrap().is_none());
}
#[test]
fn code_failure_recovery_and_late_table_keep_active_body_reference_source_order() {
    let d = parse("[^n]\n\n[^n]: ` [^a]\n    [^b]\n");
    let v = d.footnote_body(FootnoteId(0)).unwrap();
    let mut refs = v.footnote_references_in(v.range()).unwrap();
    let mut last = None;
    for (expected, number) in [("a", 2), ("b", 3)] {
        let (id, r) = refs.advance(&mut unlimited()).unwrap().unwrap();
        assert_eq!(d.footnote(r.symbol).unwrap().label, expected);
        assert_eq!(resolution(v, id.local().0).number, number);
        if let Some(prior) = last {
            assert_eq!(v.next_footnote_reference(prior).unwrap(), Some(id));
        }
        last = Some(id);
    }
    assert!(refs.advance(&mut unlimited()).unwrap().is_none());
    assert!(d.footnote_body_work().engine.code_fallback_bytes > 0);
}
#[test]
fn body_work_counts_input_finish_recovery_without_query_hidden_parse() {
    let mut d = Document::new();
    d.append(&format!("[^n]: `{}", "*a ".repeat(20_000)))
        .unwrap();
    let before = d.footnote_body_work();
    d.finish();
    let after = d.footnote_body_work();
    assert!(after.engine.code_fallback_bytes - before.engine.code_fallback_bytes >= 60_000);
    let v = d.footnote_body(FootnoteId(0)).unwrap();
    let mut c = v.display_cursor();
    let mut z = Budget::new(WorkLimit::default());
    assert!(matches!(
        c.next(v, &mut z).unwrap(),
        BodyDisplayStep::BudgetExhausted
    ));
    for _ in 0..100 {
        let mut b = tiny();
        let _ = c.next(v, &mut b).unwrap();
        assert!(b.used().bytes <= 6 && b.used().queries <= 1 && b.used().objects <= 1);
    }
    assert_eq!(d.footnote_body_work(), after);
}
