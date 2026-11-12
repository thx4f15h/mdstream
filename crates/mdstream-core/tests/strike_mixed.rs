#![forbid(unsafe_code)]
use mdstream::*;

#[path = "support/strike_mixed_witnesses.rs"]
mod fixed;
#[path = "support/strike_mixed_model.rs"]
mod model;
#[path = "support/snapshot.rs"]
mod support;

const MICRO: WorkLimit = WorkLimit {
    queries: 1,
    objects: 1,
    bytes: 6,
    tiles: 1,
    recovery: 1,
    dependencies: 1,
};

fn parse(source: &str) -> Document {
    let mut d = Document::new();
    d.append(source).unwrap();
    d.finish();
    d
}
fn path(d: &Document, at: usize, resolver: &mut LinkResolver) -> Vec<StylePairId> {
    let mut cursor = d.style_path(at).unwrap();
    let before = resolver.style_work();
    assert!(matches!(
        cursor
            .advance(d, resolver, &mut Budget::new(WorkLimit::default()))
            .unwrap(),
        StylePathStep::BudgetExhausted
    ));
    assert_eq!(resolver.style_work(), before);
    let mut result = Vec::new();
    for _ in 0..1_000_000 {
        let mut b = Budget::new(MICRO);
        let step = cursor.advance(d, resolver, &mut b).unwrap();
        assert!(b.used().queries <= 1 && b.used().objects <= 1 && b.used().bytes <= 6);
        match step {
            StylePathStep::Pair(p) => result.push(p),
            StylePathStep::Complete => return result,
            StylePathStep::BudgetExhausted => (),
        }
    }
    panic!("style path did not explicitly complete");
}
fn visible(d: &Document) -> String {
    d.display_runs_in(Span::new(0, d.source().len()))
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Scalar {
    source: Span,
    replacement: Option<Span>,
    ch: char,
    pairs: Vec<StylePairId>,
}
fn scalars(d: &Document) -> Vec<Scalar> {
    let mut resolver = d.link_resolver();
    let mut cursor = d.display_cursor(Span::new(0, d.source().len())).unwrap();
    let mut result = Vec::new();
    let mut end = 0;
    for _ in 0..100_000 {
        let mut b = Budget::new(MICRO);
        match cursor.next_with_resolver(d, &mut resolver, &mut b).unwrap() {
            DisplayStep::Complete => {
                assert_eq!(end, d.source().len());
                return result;
            }
            DisplayStep::BudgetExhausted => (),
            DisplayStep::Run(r) => {
                assert_eq!(r.source.start, end);
                assert!(d.source().is_char_boundary(r.source.start));
                assert!(d.source().is_char_boundary(r.source.end));
                end = r.source.end;
                if r.style.contains(Style::HIDDEN) {
                    continue;
                }
                let pairs = path(d, r.source.start, &mut resolver);
                assert_eq!(r.style_pair, pairs.last().copied());
                for (offset, ch) in r.text.char_indices() {
                    result.push(Scalar {
                        source: r.replacement.unwrap_or(Span::new(
                            r.source.start + offset,
                            r.source.start + offset + ch.len_utf8(),
                        )),
                        replacement: r.replacement,
                        ch,
                        pairs: pairs.clone(),
                    });
                }
            }
        }
        assert!(b.used().queries <= 1 && b.used().objects <= 1 && b.used().bytes <= 6);
    }
    panic!("display cursor did not explicitly complete");
}

// Construct a syntax tree and its raw marker/source identities before parsing.
// Padding forces unambiguous nesting, including repeated same-kind strikes.
// This is a constructive oracle, not a Markdown lexer or a Document snapshot.
struct Witness {
    source: String,
    expected: Vec<Scalar>,
}
impl Witness {
    fn text(raw: &str, decoded: &str) -> Self {
        let replacement = (raw != decoded).then_some(Span::new(0, raw.len()));
        Self {
            source: raw.into(),
            expected: decoded
                .char_indices()
                .map(|(i, ch)| Scalar {
                    source: replacement.unwrap_or(Span::new(i, i + ch.len_utf8())),
                    replacement,
                    ch,
                    pairs: Vec::new(),
                })
                .collect(),
        }
    }
    fn shift(&mut self, by: usize) {
        for s in &mut self.expected {
            s.source.start += by;
            s.source.end += by;
            if let Some(r) = &mut s.replacement {
                r.start += by;
                r.end += by;
            }
            for p in &mut s.pairs {
                p.open.start += by;
                p.open.end += by;
                p.close.start += by;
                p.close.end += by;
            }
        }
    }
    fn surround(mut self, left: &str, right: &str) -> Self {
        self.shift(left.len());
        self.source = format!("{left}{}{right}", self.source);
        self
    }
    fn wrap(self, marker: &str) -> Self {
        let mut inner = self.surround(&format!("{marker}a "), &format!(" z{marker}"));
        let close = inner.source.len() - marker.len();
        let pairs = if marker == "***" {
            vec![
                StylePairId {
                    open: Span::new(0, 1),
                    close: Span::new(close + 2, close + 3),
                    kind: InlineStyleKind::Emphasis,
                },
                StylePairId {
                    open: Span::new(1, 3),
                    close: Span::new(close, close + 2),
                    kind: InlineStyleKind::Strong,
                },
            ]
        } else {
            vec![StylePairId {
                open: Span::new(0, marker.len()),
                close: Span::new(close, close + marker.len()),
                kind: match marker {
                    "~" | "~~" => InlineStyleKind::Strike,
                    "*" | "_" => InlineStyleKind::Emphasis,
                    "**" | "__" => InlineStyleKind::Strong,
                    _ => unreachable!(),
                },
            }]
        };
        let mut all = Witness::text("a ", "a ").expected;
        for s in &mut all {
            s.source.start += marker.len();
            s.source.end += marker.len();
        }
        all.extend(inner.expected);
        let mut tail = Witness::text(" z", " z").expected;
        for s in &mut tail {
            s.source.start += close - 2;
            s.source.end += close - 2;
        }
        all.extend(tail);
        for s in &mut all {
            let mut p = pairs.clone();
            p.append(&mut s.pairs);
            s.pairs = p;
        }
        inner.expected = all;
        inner
    }
    fn code(raw: &str) -> Self {
        let mut w = Self::text(raw, raw).surround("`", "`");
        let pair = StylePairId {
            open: Span::new(0, 1),
            close: Span::new(w.source.len() - 1, w.source.len()),
            kind: InlineStyleKind::Code,
        };
        for s in &mut w.expected {
            s.pairs.push(pair);
        }
        w
    }
}

#[test]
fn constructive_mixed_trees_have_exact_utf8_sources_and_ordered_pair_identities() {
    let markers = ["~", "~~", "*", "**", "_", "__", "***"];
    let mut count = 0;
    for a in markers {
        for b in markers {
            for c in markers {
                if ![a, b, c].iter().any(|m| m.starts_with('~')) {
                    continue;
                }
                for atom in 0..7 {
                    let w = match atom {
                        0 => Witness::text("中🙂", "中🙂"),
                        1 => Witness::text("&#126;", "~"),
                        2 => Witness::text("&NotEqualTilde;", "≂\u{338}"),
                        3 => Witness::code("~raw~ &amp;"),
                        4 => Witness::text("中", "中").surround("[", "](/u)"),
                        5 => Witness::text("中", "中").surround("![", "](/i)"),
                        _ => Witness::text("&amp;", "&"),
                    }
                    .wrap(c)
                    .wrap(b)
                    .wrap(a);
                    assert_eq!(scalars(&parse(&w.source)), w.expected, "{}", w.source);
                    count += 1;
                }
            }
        }
    }
    assert_eq!(count, 1526);
}

#[test]
fn crossing_and_partial_consumption_match_independent_quadratic_batch_model() {
    let tokens = [
        "~", "~~", "~~~", "*", "**", "***", "_", "__", "a", " ", "!", "\\~",
    ];
    let mut count = 0;
    for a in tokens {
        for b in tokens {
            for c in tokens {
                if ![a, b, c].iter().any(|s| s.contains('~')) {
                    continue;
                }
                let source = format!("p {a}x{b}y{c} q");
                let expected = model::pairs(&source);
                let d = parse(&source);
                let mut resolver = d.link_resolver();
                for (at, ch) in source.char_indices() {
                    let span = Span::new(at, at + ch.len_utf8());
                    let marker = expected.iter().any(|p| {
                        p.open.start <= at && at < p.open.end
                            || p.close.start <= at && at < p.close.end
                    });
                    let escaped = at > 0 && source.as_bytes()[at - 1] == b'\\' && ch == '~';
                    let hidden_escape = ch == '\\' && source.as_bytes().get(at + 1) == Some(&b'~');
                    let run = d.runs_in(span).unwrap().next().unwrap();
                    assert_eq!(
                        run.style.contains(Style::HIDDEN),
                        marker || hidden_escape,
                        "{source:?} {at}"
                    );
                    if !marker && !hidden_escape {
                        let expected_path: Vec<_> = expected
                            .iter()
                            .copied()
                            .filter(|p| p.open.end <= at && at < p.close.start)
                            .collect();
                        assert_eq!(
                            path(&d, at, &mut resolver),
                            expected_path,
                            "{source:?} {at} escaped={escaped}"
                        );
                    }
                }
                count += 1;
            }
        }
    }
    assert_eq!(count, 1216);
}

#[test]
fn ascii_strike_and_unicode_emphasis_flanking_are_distinct_and_entities_are_not_markers() {
    for (source, text, styled) in [
        ("p~!x!~q", "p!x!q", true),
        ("~\u{a0}x\u{2003}~", "\u{a0}x\u{2003}", true),
        ("~ x~", "~ x~", false),
        ("~x ~", "~x ~", false),
        ("\\~x\\~", "~x~", false),
        ("&#126;x~", "~x~", false),
        ("~x&#126;", "~x~", false),
        ("p ~~~x~~~ q", "p ~~~x~~~ q", false),
        ("p ~~~~x~~~~ q", "p ~~~~x~~~~ q", false),
    ] {
        let d = parse(source);
        assert_eq!(visible(&d), text, "{source:?}");
        let mut r = d.link_resolver();
        assert_eq!(
            path(&d, source.find('x').unwrap(), &mut r)
                .iter()
                .any(|p| p.kind == InlineStyleKind::Strike),
            styled
        );
    }
    for source in ["*\u{a0}x*", "*x\u{2003}*", "_中_文"] {
        let d = parse(source);
        let mut r = d.link_resolver();
        assert!(
            path(&d, source.find(['x', '中']).unwrap(), &mut r).is_empty(),
            "{source:?}"
        );
    }
    let source = "~~中 &NotEqualTilde; &#126; \\~ *e*~~";
    let d = parse(source);
    let actual = scalars(&d);
    assert_eq!(
        actual.iter().map(|s| s.ch).collect::<String>(),
        "中 ≂\u{338} ~ ~ e"
    );
    let entity = source.find('&').unwrap();
    let mapped: Vec<_> = actual.iter().filter(|s| s.source.start == entity).collect();
    assert_eq!(mapped.len(), 2);
    assert!(
        mapped
            .iter()
            .all(|s| s.source == Span::new(entity, entity + 15) && s.replacement == Some(s.source))
    );
    let pair = StylePairId {
        open: Span::new(0, 2),
        close: Span::new(source.len() - 2, source.len()),
        kind: InlineStyleKind::Strike,
    };
    assert!(actual.iter().all(|s| s.pairs.first() == Some(&pair)));
    let escape = source.find('\\').unwrap();
    assert!(actual.iter().any(|s| s.ch == '~'
        && s.source == Span::new(escape + 1, escape + 2)
        && s.replacement.is_none()));
}

#[test]
fn closed_code_and_failed_code_release_have_fixed_mixed_paths() {
    for (source, text, needle, kinds) in [
        (
            "~~a `~raw~` z~~",
            "a ~raw~ z",
            "raw",
            vec![InlineStyleKind::Strike, InlineStyleKind::Code],
        ),
        (
            "`~~*raw*~~`",
            "~~*raw*~~",
            "raw",
            vec![InlineStyleKind::Code],
        ),
        (
            "`oops ~~*x*~~",
            "`oops x",
            "x",
            vec![InlineStyleKind::Strike, InlineStyleKind::Emphasis],
        ),
        (
            "~~a `oops *x* z~~",
            "a `oops x z",
            "x",
            vec![InlineStyleKind::Strike, InlineStyleKind::Emphasis],
        ),
        (
            "`oops ~~a ``~raw~`` z~~",
            "`oops a ~raw~ z",
            "raw",
            vec![InlineStyleKind::Strike, InlineStyleKind::Code],
        ),
        (
            "~~a $~raw~$ z~~",
            "a ~raw~ z",
            "raw",
            vec![InlineStyleKind::Strike],
        ),
        // Failed math remains opaque in the current formal dialect; code releases.
        ("$oops ~~*raw*~~", "$oops ~~*raw*~~", "raw", vec![]),
    ] {
        let d = parse(source);
        assert_eq!(visible(&d), text, "{source:?}");
        let mut resolver = d.link_resolver();
        assert_eq!(
            path(&d, source.find(needle).unwrap(), &mut resolver)
                .iter()
                .map(|p| p.kind)
                .collect::<Vec<_>>(),
            kinds,
            "{source:?}"
        );
        assert!(d.work().code_fallback_bytes <= source.len() as u64);
    }
}

fn disposition(d: &Document, id: LinkId, resolver: &mut LinkResolver) -> LinkDisposition {
    let mut task = d.link_activity(id).unwrap();
    for _ in 0..100_000 {
        let mut b = Budget::new(MICRO);
        if let Some(state) = task.advance_with(d, resolver, &mut b).unwrap() {
            return state;
        }
        assert!(b.used().queries <= 1 && b.used().objects <= 1);
    }
    panic!("link activity did not complete");
}

#[test]
fn unresolved_active_and_suppressed_reference_labels_choose_the_actual_strike_scope() {
    for marker in ["~", "~~"] {
        for image in [false, true] {
            for parent in [false, true] {
                for kid in [false, true] {
                    let mut source = format!(
                        "{marker}pre {}[mid{marker} [kid] tail][r] end\n\n",
                        if image { "!" } else { "" }
                    );
                    if parent {
                        source.push_str("[r]: /r\n");
                    }
                    if kid {
                        source.push_str("[kid]: /k\n");
                    }
                    let d = parse(&source);
                    let mut resolver = d.link_resolver();
                    let id = d
                        .links_in(Span::new(0, source.len()))
                        .find(|(_, i)| d.text(i.label).unwrap().starts_with("mid"))
                        .unwrap()
                        .0;
                    let active = parent && (image || !kid);
                    let expected = if active {
                        LinkDisposition::Active
                    } else if kid && !image {
                        LinkDisposition::Suppressed
                    } else {
                        LinkDisposition::Unresolved
                    };
                    assert_eq!(disposition(&d, id, &mut resolver), expected, "{source:?}");
                    let close = source.find("mid").unwrap() + 3;
                    for needle in ["pre", "mid"] {
                        let expected = if active {
                            vec![]
                        } else {
                            vec![StylePairId {
                                open: Span::new(0, marker.len()),
                                close: Span::new(close, close + marker.len()),
                                kind: InlineStyleKind::Strike,
                            }]
                        };
                        assert_eq!(
                            path(&d, source.find(needle).unwrap(), &mut resolver),
                            expected,
                            "{source:?}"
                        );
                    }
                    let work = resolver.style_work();
                    for _ in 0..32 {
                        path(&d, source.find("pre").unwrap(), &mut resolver);
                    }
                    assert_eq!(
                        resolver.style_work(),
                        work,
                        "scope must not replay per glyph"
                    );
                }
            }
        }
    }
    for marker in ["~", "~~"] {
        for image in ["", "!"] {
            for source in [
                format!("{marker}pre {image}[mid{marker}](/u)"),
                format!("{image}[{marker}pre](/u) mid{marker}"),
            ] {
                let d = parse(&source);
                let mut r = d.link_resolver();
                assert!(
                    path(&d, source.find("pre").unwrap(), &mut r).is_empty(),
                    "{source:?}"
                );
                assert!(
                    path(&d, source.find("mid").unwrap(), &mut r).is_empty(),
                    "{source:?}"
                );
            }
        }
    }
}

#[test]
fn explicit_lazy_partial_tab_and_container_exit_share_only_the_formal_scope() {
    for m in ["~", "~~"] {
        for (body, matched) in [
            ("@left *em*\nright@!", true),
            ("@left *em*  \nright@!", true),
            ("@left *em*\\\r\nright@!", true),
            ("> @left *em*\n> right@!", true),
            ("> > @left *em*\n> right@!", true),
            ("> - > @left *em*\nright@!", true),
            ("- @left *em*\n  right@!", true),
            ("- > @left *em*\n\tright@!", true),
            ("@left *em*\nright@!\n---", true),
            ("@left *em*\n\nright@!", false),
            ("> @left *em*\n>\n> right@!", false),
            ("- @left *em*\n- right@!", false),
            ("- @left *em*\n  > child\n  right@!", false),
            ("> @left *em*\n# cut\nright@!", false),
            ("- @left *em*\n\t\tright@!", false),
            ("@left *em*|right@!\n-|-", false),
            ("| @left *em* | right@! |\n|---|---|", false),
        ] {
            let source = body.replace('@', m) + "\n";
            let d = parse(&source);
            let mut r = d.link_resolver();
            for needle in ["left", "right"] {
                assert_eq!(
                    path(&d, source.find(needle).unwrap(), &mut r)
                        .iter()
                        .any(|p| p.kind == InlineStyleKind::Strike),
                    matched,
                    "{source:?} {needle}"
                );
            }
            assert!(
                path(&d, source.find("em").unwrap(), &mut r)
                    .iter()
                    .any(|p| p.kind == InlineStyleKind::Emphasis)
            );
            if matched {
                let first = d.block_at(source.find("left").unwrap()).unwrap();
                let last = d.block_at(source.find("right").unwrap()).unwrap();
                assert_eq!(
                    d.blocks()[first.0].container,
                    d.blocks()[last.0].container,
                    "{source:?}"
                );
            }
        }
    }
}

#[test]
fn general_late_headers_finish_old_strikes_and_keep_cells_fresh_without_renumbering() {
    for m in ["~", "~~"] {
        for (open, prefix) in [("", ""), ("> ", "> "), ("- ", "  "), ("> - ", ">   ")] {
            for ending in ["---|---\n", "wrong\n", ""] {
                let mut d = Document::new();
                d.append(&format!("{open}{m}old\n")).unwrap();
                let old = d.blocks()[0].clone();
                d.append(&format!(
                    "{prefix}end{m}*x* [h](v) $q$ [^n] {}",
                    "s".repeat(1100)
                ))
                .unwrap();
                let id = d
                    .links_in(Span::new(0, d.source().len()))
                    .find(|(_, i)| d.text(i.label) == Some("h"))
                    .unwrap()
                    .0;
                let info = *d.link_info(id).unwrap();
                assert_eq!(info.state, SyntaxState::Provisional);
                d.append(&format!("|{m}c{m}\n{prefix}{ending}")).unwrap();
                d.finish();
                let table = ending == "---|---\n";
                assert_eq!(d.tables().len(), usize::from(table));
                assert_eq!(d.block_at(old.source.start), Some(BlockId(0)));
                assert_eq!(d.blocks()[0].source.start, old.source.start);
                assert_eq!(d.blocks()[0].kind, old.kind);
                assert_eq!(d.blocks()[0].container, old.container);
                assert_eq!(d.blocks()[0].state, SyntaxState::Sealed);
                if table || old.container.is_some() {
                    assert_eq!(d.blocks()[0].source, old.source);
                }
                assert_eq!(
                    *d.link_info(id).unwrap(),
                    LinkInfo {
                        state: SyntaxState::Sealed,
                        ..info
                    }
                );
                let mut r = d.link_resolver();
                let at = d.source().find("old").unwrap();
                let old_path = path(&d, at, &mut r);
                assert_eq!(old_path.len(), usize::from(!table), "{}", d.source());
                if !table {
                    let first = d.source().find(m).unwrap();
                    let close = d.source().find("end").unwrap() + 3;
                    assert_eq!(
                        old_path,
                        [StylePairId {
                            open: Span::new(first, first + m.len()),
                            close: Span::new(close, close + m.len()),
                            kind: InlineStyleKind::Strike
                        }]
                    );
                }
                assert_eq!(
                    path(&d, d.source().find("*x*").unwrap() + 1, &mut r)
                        .last()
                        .unwrap()
                        .kind,
                    InlineStyleKind::Emphasis
                );
                let c = d.source().find(&format!("{m}c{m}")).unwrap() + m.len();
                let cp = path(&d, c, &mut r);
                assert_eq!(cp.len(), 1);
                assert_eq!(
                    cp[0],
                    StylePairId {
                        open: Span::new(c - m.len(), c),
                        close: Span::new(c + 1, c + 1 + m.len()),
                        kind: InlineStyleKind::Strike
                    }
                );
                assert!(d.work().header_replay_bytes <= d.source().len() as u64);
            }
        }
    }
    // A LinkId spanning the old line really withdraws on confirmation. Local
    // same-source h survives with the original ID; it is not a filtered snapshot.
    let mut d = Document::new();
    d.append("[old ~~a\n").unwrap();
    d.append(&format!("end~~](u) [h](v) {}", "s".repeat(1100)))
        .unwrap();
    let old_id = d
        .links_in(Span::new(0, d.source().len()))
        .find(|(_, i)| i.source.start == 0)
        .unwrap()
        .0;
    let h = d
        .links_in(Span::new(0, d.source().len()))
        .find(|(_, i)| d.text(i.label) == Some("h"))
        .unwrap()
        .0;
    assert_eq!(d.link_info(old_id).unwrap().state, SyntaxState::Provisional);
    d.append("|~~c~~\n---|---\n").unwrap();
    d.finish();
    assert_eq!(d.tables().len(), 1, "old label header must confirm");
    assert!(d.link_info(old_id).is_some()); // raw allocation survives withdrawal
    let mut withdrawn_resolver = d.link_resolver();
    assert_eq!(
        disposition(&d, old_id, &mut withdrawn_resolver),
        LinkDisposition::Suppressed
    );
    assert_eq!(d.link_info(h).unwrap().state, SyntaxState::Sealed);
    let mut r = d.link_resolver();
    assert!(path(&d, d.source().find('a').unwrap(), &mut r).is_empty());
    assert_eq!(disposition(&d, h, &mut r), LinkDisposition::Active);
    let mut heights = HeightIndex::default();
    heights.rebuild(&d).unwrap();
    let mut pass = LayoutPass::new(&d, &heights, TileId(0), u64::MAX, false).unwrap();
    let mut sink = Sink::default();
    assert_eq!(
        pass.advance(
            &d,
            &mut heights,
            &mut Budget::new(WorkLimit::unlimited()),
            &mut sink
        )
        .unwrap(),
        Completion::EndOfDocument
    );
    assert!(!sink.objects.contains(&old_id));
    assert!(sink.objects.contains(&h));
}

#[test]
fn byte_splits_each_live_prefix_and_finish_preserve_raw_and_ordered_identities() {
    // This is an additional consistency check, not the independent oracle.
    for source in [
        "~~*中 &NotEqualTilde;*~~ ~尾~",
        "> - ~中\r\n尾~ *x*",
        "~~pre [mid~~ [kid] tail][r]\n\n[r]: /r\n[kid]: /k\n",
        "`oops ~~a ``~原~`` z~~ &amp;",
        "~~old\nend~~*x* [h](v)|~~中~~\n---|---\n",
        "~~old\nend~~*x* [h](v)|~~中~~\nwrong\n",
        "~unfinished &NotEqu \\",
    ] {
        let whole = parse(source);
        let expected = support::snapshot(&whole);
        let ordered = scalars(&whole);
        for cut in 0..=source.len() {
            let mut d = Utf8Document::new();
            d.append(&source.as_bytes()[..cut]).unwrap();
            d.append(&source.as_bytes()[cut..]).unwrap();
            d.finish();
            assert_eq!(
                support::snapshot(d.document()),
                expected,
                "cut {cut} {source:?}"
            );
            assert_eq!(
                scalars(d.document()),
                ordered,
                "ordered cut {cut} {source:?}"
            );
        }
        let mut stream = Utf8Document::new();
        for end in 1..=source.len() {
            stream.append(&source.as_bytes()[end - 1..end]).unwrap();
            let mut batch = Utf8Document::new();
            batch.append(&source.as_bytes()[..end]).unwrap();
            assert_eq!(
                support::snapshot(stream.document()),
                support::snapshot(batch.document()),
                "live {end} {source:?}"
            );
            assert_eq!(scalars(stream.document()), scalars(batch.document()));
            let mut finished = Utf8Document::new();
            for byte in source.as_bytes()[..end].iter() {
                finished.append(&[*byte]).unwrap();
            }
            finished.finish();
            batch.finish();
            assert_eq!(
                support::snapshot(finished.document()),
                support::snapshot(batch.document()),
                "EOF {end} {source:?}"
            );
            assert_eq!(scalars(finished.document()), scalars(batch.document()));
            assert_eq!(
                stream.pending_bytes().len(),
                source[..source.floor_char_boundary(end)]
                    .len()
                    .abs_diff(end)
            );
        }
        stream.finish();
        assert_eq!(scalars(stream.document()), ordered);
    }
}

#[derive(Default)]
struct Sink {
    text: String,
    runs: Vec<(Span, Style, Option<StylePairId>)>,
    objects: Vec<LinkId>,
}
impl LayoutSink for Sink {
    fn begin_tile(&mut self, _: TileId) {}
    fn text(&mut self, r: RenderRun<'_>) {
        if !r.style.contains(Style::HIDDEN) {
            self.text.push_str(r.text);
            self.runs.push((r.source, r.style, r.style_pair));
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
fn actual_layout_and_display_use_the_same_pairs_and_late_definition_rejects_old_tasks() {
    let mut d = Document::new();
    d.append("~~pre [mid~~ tail]\n\n").unwrap();
    let id = d.links_in(Span::new(0, d.source().len())).next().unwrap().0;
    let info = *d.link_info(id).unwrap();
    let mut resolver = d.link_resolver();
    let pairs = path(&d, 2, &mut resolver);
    assert_eq!(pairs.len(), 1);
    let mut old_cursor = d.display_cursor(Span::new(0, d.source().len())).unwrap();
    let mut heights = HeightIndex::default();
    heights.rebuild(&d).unwrap();
    let ticket = heights.ticket(TileId(0)).unwrap();
    let mut old_pass = LayoutPass::new(&d, &heights, TileId(0), u64::MAX, false).unwrap();
    let update = d.append("[mid~~ tail]: /u\n").unwrap();
    assert_eq!(*d.link_info(id).unwrap(), info);
    heights.apply(&d, &update).unwrap();
    assert_eq!(
        heights.measure(ticket, 1),
        Err(LayoutError::StaleMeasurement)
    );
    assert!(matches!(
        old_cursor.next(&d, &mut Budget::new(MICRO)),
        Err(LayoutError::StaleContinuation)
    ));
    assert!(matches!(
        old_pass.advance(
            &d,
            &mut heights,
            &mut Budget::new(MICRO),
            &mut Sink::default()
        ),
        Err(LayoutError::StaleContinuation)
    ));
    let entries = resolver.memo_entries();
    assert!(entries > 0);
    assert_eq!(
        resolver.retire(&mut Budget::new(WorkLimit::default())),
        Completion::BudgetExhausted
    );
    let mut paid = 0;
    for _ in 0..100_000 {
        let mut b = Budget::new(MICRO);
        let done = resolver.retire(&mut b);
        paid += b.used().objects;
        if done == Completion::Covered {
            break;
        }
    }
    assert_eq!(paid, entries);
    assert_eq!(resolver.memo_entries(), 0);
    resolver.reset(&d).unwrap();
    assert!(path(&d, 2, &mut resolver).is_empty());
    assert_eq!(disposition(&d, id, &mut resolver), LinkDisposition::Active);
    let update = d.finish();
    heights.apply(&d, &update).unwrap();
    let mut pass = LayoutPass::new(&d, &heights, TileId(0), u64::MAX, false).unwrap();
    let mut sink = Sink::default();
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
    let mut complete = false;
    for _ in 0..100_000 {
        let mut b = Budget::new(MICRO);
        let state = pass.advance(&d, &mut heights, &mut b, &mut sink).unwrap();
        assert!(
            b.used().queries <= 1
                && b.used().objects <= 1
                && b.used().bytes <= 6
                && b.used().tiles <= 1
                && b.used().recovery <= 1
                && b.used().dependencies <= 1
        );
        if state == Completion::EndOfDocument {
            complete = true;
            break;
        }
    }
    assert!(complete);
    assert_eq!(sink.text, "~~pre mid~~ tail\n\n");
    assert_eq!(sink.text, visible(&d));
    assert_eq!(sink.objects, [id]);
    let mut r = d.link_resolver();
    for (span, _, pair) in sink.runs {
        assert_eq!(path(&d, span.start, &mut r).last().copied(), pair);
    }
}

#[test]
fn giant_literal_runs_deep_mixed_paths_and_cancellation_pay_actual_work() {
    let mut literal = Document::new();
    literal.append("p ").unwrap();
    for _ in 0..1000 {
        literal.append(&"~".repeat(1000)).unwrap();
    }
    literal.append(" x").unwrap();
    literal.finish();
    assert_eq!(literal.work().inline_bytes, literal.source().len() as u64);
    assert_eq!(literal.work().delimiter_steps, 1); // one maximal run start
    assert_eq!(literal.confirmation_stats().style_index_entries, 0);
    assert_eq!(literal.confirmation_stats().style_event_entries, 0);
    eprintln!(
        "literal-1m: work={:?} annotations={:?}",
        literal.work(),
        literal.annotation_arena_stats()
    );

    let depth = 1024;
    let source = format!(
        "{}中{} [r]\n\n",
        "~~a **b ".repeat(depth),
        " c** z~~".repeat(depth)
    );
    let d = parse(&source);
    let mut resolver = d.link_resolver();
    let pairs = path(&d, source.find('中').unwrap(), &mut resolver);
    assert_eq!(pairs.len(), 2 * depth);
    for (i, p) in pairs.iter().enumerate() {
        assert_eq!(
            p.kind,
            if i % 2 == 0 {
                InlineStyleKind::Strike
            } else {
                InlineStyleKind::Strong
            }
        );
        assert_eq!(d.text(p.open), Some(if i % 2 == 0 { "~~" } else { "**" }));
        assert_eq!(d.text(p.close), d.text(p.open));
    }
    let work = resolver.style_work();
    for _ in 0..16 {
        assert_eq!(path(&d, source.find('中').unwrap(), &mut resolver), pairs);
    }
    assert_eq!(resolver.style_work(), work);
    assert!(work.events <= 4 * depth as u64 + 4);
    assert!(work.index_steps <= 32 * depth as u64);
    assert!(resolver.memo_entries() < 32 * depth);

    let mut partial = d.style_path(source.find('中').unwrap()).unwrap();
    for _ in 0..700 {
        partial
            .advance(&d, &mut resolver, &mut Budget::new(MICRO))
            .unwrap();
    }
    assert_eq!(
        partial.retire(&mut Budget::new(WorkLimit::default())),
        Completion::BudgetExhausted
    );
    let mut path_paid = 0;
    let mut complete = false;
    for _ in 0..depth * 4 {
        let mut b = Budget::new(MICRO);
        let done = partial.retire(&mut b);
        path_paid += b.used().objects;
        if done == Completion::Covered {
            complete = true;
            break;
        }
    }
    assert!(complete && path_paid > 500);
    let entries = resolver.memo_entries();
    let mut paid = 0;
    complete = false;
    assert_eq!(
        resolver.retire(&mut Budget::new(WorkLimit::default())),
        Completion::BudgetExhausted
    );
    for _ in 0..entries + 10 {
        let mut b = Budget::new(MICRO);
        let done = resolver.retire(&mut b);
        paid += b.used().objects;
        if done == Completion::Covered {
            complete = true;
            break;
        }
    }
    assert!(complete);
    assert_eq!(paid, entries);
    assert_eq!(resolver.memo_entries(), 0);
    eprintln!(
        "deep-mixed: source={} work={:?} confirmation={:?} query={work:?} path_retired={path_paid} memo_retired={paid}",
        source.len(),
        d.work(),
        d.confirmation_stats()
    );
}

#[test]
fn deep_conditional_guards_and_twenty_thousand_old_uses_have_no_definition_fanout() {
    let depth = 1024;
    let mut source = format!("{}~~中~~{}\n\n", "[a ".repeat(depth), "][r]".repeat(depth));
    source.push_str("[r]: /u\n");
    let d = parse(&source);
    let mut r = d.link_resolver();
    let pairs = path(&d, source.find('中').unwrap(), &mut r);
    assert_eq!(pairs.len(), 1);
    assert_eq!(pairs[0].kind, InlineStyleKind::Strike);
    let before = r.style_work();
    for _ in 0..64 {
        assert_eq!(path(&d, source.find('中').unwrap(), &mut r), pairs);
    }
    assert_eq!(r.style_work(), before);
    assert!(r.memo_entries() < 64 * depth);
    assert!(d.link_guard_stats().nodes < 8 * depth);
    assert!(d.link_guard_stats().capacity_bytes < 512 * depth);
    eprintln!(
        "deep-guards: work={:?} query={:?} style={:?} entries={} guards={:?} parser={:?}",
        d.work(),
        r.work(),
        r.style_work(),
        r.memo_entries(),
        d.link_guard_stats(),
        d.link_parser_stats()
    );
    let entries = r.memo_entries();
    let mut paid = 0;
    let mut complete = false;
    for _ in 0..entries + 8 {
        let mut b = Budget::new(MICRO);
        let done = r.retire(&mut b);
        paid += b.used().objects;
        if done == Completion::Covered {
            complete = true;
            break;
        }
    }
    assert!(complete);
    assert_eq!(paid, entries);

    let mut d = Document::new();
    d.append(&"~~pre [mid~~ tail][r]\n\n".repeat(20_000))
        .unwrap();
    let first = *d.link_info(LinkId(0)).unwrap();
    let last = *d.link_info(LinkId(19_999)).unwrap();
    let guards = d.link_guard_stats();
    let index = d.link_index_stats();
    let before = d.work();
    let stats = d.confirmation_stats();
    d.append("[r]: /u\n").unwrap();
    let defined = d.work();
    assert_eq!(defined.style_events, before.style_events);
    assert_eq!(defined.style_index_steps, before.style_index_steps);
    assert_eq!(defined.link_gate_steps, before.link_gate_steps);
    assert_eq!(defined.delimiter_steps, before.delimiter_steps);
    assert!(defined.tree_visits - before.tree_visits < 1000);
    assert_eq!(d.link_guard_stats(), guards);
    assert_eq!(d.link_index_stats(), index);
    assert_eq!(*d.link_info(LinkId(0)).unwrap(), first);
    assert_eq!(*d.link_info(LinkId(19_999)).unwrap(), last);
    assert_eq!(
        d.confirmation_stats().style_index_entries,
        stats.style_index_entries
    );
    // First force the ordinary-line classification; the short pending prefix
    // is not publishing inline input until the classification window fills.
    d.append(&format!("plain {}", "x".repeat(1100))).unwrap();
    let before_tail = d.work();
    let annotation = d.annotation_arena_stats();
    let parser = d.link_parser_stats();
    let styles = d.confirmation_stats();
    for _ in 0..1000 {
        d.append(&"x".repeat(1000)).unwrap();
    }
    let tail = d.work();
    assert_eq!(tail.inline_bytes - before_tail.inline_bytes, 1_000_000);
    assert_eq!(tail.style_events, before_tail.style_events);
    assert_eq!(tail.style_index_steps, before_tail.style_index_steps);
    assert_eq!(tail.delimiter_steps, before_tail.delimiter_steps);
    assert_eq!(tail.link_gate_steps, before_tail.link_gate_steps);
    assert_eq!(tail.tree_visits, before_tail.tree_visits);
    assert_eq!(d.annotation_arena_stats(), annotation);
    assert_eq!(d.link_parser_stats(), parser);
    assert_eq!(
        d.confirmation_stats().style_event_entries,
        styles.style_event_entries
    );
    assert_eq!(
        d.confirmation_stats().style_index_entries,
        styles.style_index_entries
    );
    d.finish();
    eprintln!(
        "old-20k: definition_delta_tree={} definition_delta_inline={} tail_delta_inline={} guards={guards:?} index={index:?} annotations={annotation:?} parser={parser:?} styles={styles:?}",
        defined.tree_visits - before.tree_visits,
        defined.inline_bytes - before.inline_bytes,
        tail.inline_bytes - before_tail.inline_bytes
    );
}

#[test]
fn punctuation_adjacent_counterexamples_check_raw_display_path_and_real_layout_separately() {
    for witness in fixed::CASES {
        let d = parse(witness.source);
        assert!(witness.cmark_html.contains("<del>a <"));
        assert_eq!(visible(&d), witness.formal_text);
        let mut resolver = d.link_resolver();
        for r in d.runs_in(Span::new(0, witness.source.len())).unwrap() {
            let expected: Vec<_> = witness
                .formal_pairs
                .iter()
                .copied()
                .filter(|p| p.open.end <= r.source.start && r.source.start < p.close.start)
                .collect();
            assert_eq!(
                r.style_pair,
                expected.last().copied(),
                "raw {:?} {}",
                r.source,
                witness.source
            );
            assert_eq!(
                path(&d, r.source.start, &mut resolver),
                expected,
                "path {:?} {}",
                r.source,
                witness.source
            );
        }
        for s in scalars(&d) {
            let expected: Vec<_> = witness
                .formal_pairs
                .iter()
                .copied()
                .filter(|p| p.open.end <= s.source.start && s.source.start < p.close.start)
                .collect();
            assert_eq!(s.pairs, expected, "display {s:?} {}", witness.source);
        }
        let mut heights = HeightIndex::default();
        heights.rebuild(&d).unwrap();
        let mut pass = LayoutPass::new(&d, &heights, TileId(0), u64::MAX, false).unwrap();
        let mut sink = Sink::default();
        let mut complete = false;
        for _ in 0..10000 {
            if pass
                .advance(&d, &mut heights, &mut Budget::new(MICRO), &mut sink)
                .unwrap()
                == Completion::EndOfDocument
            {
                complete = true;
                break;
            }
        }
        assert!(complete);
        assert_eq!(sink.text, witness.formal_text);
        for (span, _, pair) in sink.runs {
            assert_eq!(
                pair,
                witness
                    .formal_pairs
                    .iter()
                    .copied()
                    .rfind(|p| p.open.end <= span.start && span.start < p.close.start)
            );
        }
    }
}

#[test]
fn unresolved_image_full_suffix_differences_do_not_hide_a_strike_scope_failure() {
    for marker in ["~", "~~"] {
        for prefix in ["", "> ", "- ", "> - ", "- > "] {
            let source = format!("{prefix}{marker}a ![b{marker} [kid] c][r]\n");
            let d = parse(&source);
            let mut r = d.link_resolver();
            // The existing unresolved image/full fallback displays the alt.
            // cmark retains ![...][r]; the strike pair itself is identical.
            assert_eq!(visible(&d), "a b [kid] c\n");
            let open = prefix.len();
            let close = source.find('b').unwrap() + 1;
            let pair = StylePairId {
                open: Span::new(open, open + marker.len()),
                close: Span::new(close, close + marker.len()),
                kind: InlineStyleKind::Strike,
            };
            for needle in ["a !", "b"] {
                assert_eq!(path(&d, source.find(needle).unwrap(), &mut r), [pair]);
            }
            for needle in ["kid", " c"] {
                assert!(path(&d, source.find(needle).unwrap(), &mut r).is_empty());
            }
            let image = d
                .links_in(Span::new(0, source.len()))
                .find(|(_, i)| i.kind == LinkKind::Image)
                .unwrap()
                .0;
            assert_eq!(disposition(&d, image, &mut r), LinkDisposition::Unresolved);
        }
    }
}

#[test]
fn deep_missing_quote_continuation_and_million_plain_tail_do_not_walk_ancestors() {
    let prefix = ">".repeat(4097);
    let mut d = Document::new();
    d.append(&format!("{prefix} ~~a **中** [r] {}", "x".repeat(1100)))
        .unwrap();
    let first = d.block_at(prefix.len() + 1).unwrap();
    let container = d.blocks()[first.0].container;
    let before = d.work();
    let arena = d.container_arena_stats();
    let annotation = d.annotation_arena_stats();
    let events = d.confirmation_stats().style_event_entries;
    for _ in 0..1000 {
        d.append(&"x".repeat(1000)).unwrap();
    }
    let after = d.work();
    assert_eq!(after.inline_bytes - before.inline_bytes, 1_000_000);
    assert_eq!(after.container_bytes, before.container_bytes);
    assert_eq!(after.container_steps, before.container_steps);
    assert_eq!(after.delimiter_steps, before.delimiter_steps);
    assert_eq!(after.tree_visits, before.tree_visits);
    assert_eq!(d.container_arena_stats(), arena);
    assert_eq!(d.annotation_arena_stats(), annotation);
    assert_eq!(d.confirmation_stats().style_event_entries, events);
    d.append("\ntail~~!\n").unwrap();
    let continued = d.work();
    let tail = d.source().find("tail").unwrap();
    assert_eq!(d.blocks()[d.block_at(tail).unwrap().0].container, container);
    assert!(continued.container_steps - after.container_steps < 32);
    d.finish();
    let mut r = d.link_resolver();
    assert_eq!(
        path(&d, d.source().find('中').unwrap(), &mut r)
            .iter()
            .map(|p| p.kind)
            .collect::<Vec<_>>(),
        [InlineStyleKind::Strike, InlineStyleKind::Strong]
    );
    assert_eq!(
        path(&d, tail, &mut r)
            .iter()
            .map(|p| p.kind)
            .collect::<Vec<_>>(),
        [InlineStyleKind::Strike]
    );
    eprintln!(
        "deep-container-4097: arena={arena:?} annotation={annotation:?} tail_inline={} tail_container_steps={} missing_prefix_steps={}",
        after.inline_bytes - before.inline_bytes,
        after.container_steps - before.container_steps,
        continued.container_steps - after.container_steps
    );
}

#[test]
fn adjacent_strikes_keep_three_identities_and_zero_components_cannot_publish_text() {
    let source = "~a~[~b~](/u)~~c~~";
    let d = parse(source);
    let actual = scalars(&d);
    assert_eq!(actual.iter().map(|s| s.ch).collect::<String>(), "abc");
    for (s, (open, open_end, close, close_end)) in
        actual
            .iter()
            .zip([(0, 1, 2, 3), (4, 5, 6, 7), (12, 14, 15, 17)])
    {
        assert_eq!(
            s.pairs,
            [StylePairId {
                open: Span::new(open, open_end),
                close: Span::new(close, close_end),
                kind: InlineStyleKind::Strike
            }]
        );
    }
    for limit in [
        WorkLimit::default(),
        WorkLimit {
            queries: 0,
            ..MICRO
        },
        WorkLimit {
            objects: 0,
            ..MICRO
        },
        WorkLimit { bytes: 0, ..MICRO },
    ] {
        let mut cursor = d.display_cursor(Span::new(0, source.len())).unwrap();
        let mut resolver = d.link_resolver();
        assert_eq!(
            cursor
                .next_with_resolver(&d, &mut resolver, &mut Budget::new(limit))
                .unwrap(),
            DisplayStep::BudgetExhausted
        );
        assert_eq!(cursor.source_position(), 0);
        let mut text = String::new();
        let mut complete = false;
        for _ in 0..10000 {
            match cursor
                .next_with_resolver(&d, &mut resolver, &mut Budget::new(MICRO))
                .unwrap()
            {
                DisplayStep::Run(r) if !r.style.contains(Style::HIDDEN) => text.push_str(r.text),
                DisplayStep::Complete => {
                    complete = true;
                    break;
                }
                DisplayStep::Run(_) | DisplayStep::BudgetExhausted => (),
            }
        }
        assert!(complete);
        assert_eq!(text, "abc");
    }
}
