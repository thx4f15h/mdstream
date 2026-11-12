use mdstream::*;
struct Case {
    name: &'static str,
    body: &'static str,
    counts: &'static [(&'static str, usize)],
    visible: Option<&'static str>,
}
include!("support/footnote_markdown_cases.rs");
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
fn parse(s: &str) -> Document {
    let mut d = Document::new();
    d.append(s).unwrap();
    d.finish();
    d
}
fn shown(v: BodyView<'_>) -> String {
    let mut c = v.display_cursor();
    let mut s = String::new();
    loop {
        match c.next(v, &mut unlimited()).unwrap() {
            BodyDisplayStep::Run(r) => {
                assert_eq!(r.source.body, v.key());
                if !r.style.contains(Style::HIDDEN) {
                    s.push_str(r.text)
                }
            }
            BodyDisplayStep::Complete => break,
            _ => panic!(),
        }
    }
    s
}
fn projected(v: BodyView<'_>) -> String {
    let mut c = v.source_cursor(v.range()).unwrap();
    let mut out = String::new();
    while let Some(chunk) = c.advance(&mut unlimited()).unwrap() {
        assert!(matches!(chunk.source,BodyAddress::Projected(s) if s.body==v.key()));
        out.push_str(chunk.text)
    }
    out
}
fn frame(body: &str, open: &str, prefix: &str, eol: &str) -> String {
    let mut parts = body.split('\n');
    let mut out = open.to_owned();
    out.push_str(parts.next().unwrap());
    for line in parts {
        out.push_str(if eol.is_empty() { "\n" } else { eol });
        out.push_str(prefix);
        out.push_str(line)
    }
    out.push_str(eol);
    out
}
fn counts(v: BodyView<'_>) -> Vec<(&'static str, usize)> {
    let mut links = 0;
    let mut images = 0;
    let mut unresolved = 0;
    let mut window = v.link_window(v.range()).unwrap();
    let mut resolver = v.link_resolver();
    loop {
        match window.advance(v, &mut unlimited()).unwrap() {
            BodyLinkStep::Object(id, info) => {
                assert_eq!(info.body, v.key());
                let mut task = v.link_activity(id).unwrap().unwrap();
                let status = task
                    .advance(v, &mut resolver, &mut unlimited())
                    .unwrap()
                    .unwrap();
                if status != LinkDisposition::Suppressed {
                    links += 1;
                    images += usize::from(info.value.kind == LinkKind::Image);
                    unresolved += usize::from(status == LinkDisposition::Unresolved);
                }
            }
            BodyLinkStep::Complete => break,
            _ => panic!(),
        }
    }
    let mut c = v.maths_in(v.range()).unwrap();
    let mut maths = 0;
    while c.advance(&mut unlimited()).unwrap().is_some() {
        maths += 1
    }
    let mut f = v.footnote_references_in(v.range()).unwrap();
    let mut refs = 0;
    let mut symbols = Vec::new();
    while let Some((id, r)) = f.advance(&mut unlimited()).unwrap() {
        assert_eq!(id.body(), v.key());
        assert_eq!(r.source.body, v.key());
        refs += 1;
        symbols.push(r.symbol)
    }
    symbols.sort();
    symbols.dedup();
    let mut code = 0;
    let mut mathblock = 0;
    let mut definitions = 0;
    let mut paras = 0;
    let mut rule = 0;
    for i in 0..v.block_count() {
        match v.block(v.qualify(BlockId(i))).unwrap().unwrap().value.kind {
            BlockKind::Code { .. } => code += 1,
            BlockKind::Math { .. } => mathblock += 1,
            BlockKind::Definition { .. } => definitions += 1,
            BlockKind::Paragraph => paras += 1,
            BlockKind::ThematicBreak => rule += 1,
            _ => {}
        }
    }
    let tasks = (0..v.container_count())
        .filter(|i| {
            matches!(
                v.container(v.qualify(ContainerId(*i)))
                    .unwrap()
                    .unwrap()
                    .value
                    .kind,
                ContainerKind::Item { task: Some(_), .. }
            )
        })
        .count();
    vec![
        ("links", links),
        ("images", images),
        ("unresolved_links", unresolved),
        ("maths", maths),
        ("footnote_references", refs),
        ("footnotes", symbols.len()),
        ("code_blocks", code),
        ("math_blocks", mathblock),
        ("definitions", definitions),
        ("paragraphs", paras),
        ("thematic_breaks", rule),
        ("headings", v.heading_count()),
        ("tables", v.table_count()),
        ("task_items", tasks),
    ]
}
#[test]
fn authored_markdown_oracles_compose_with_all_outer_scopes_and_line_endings() {
    for c in CASES {
        for (open, prefix) in [
            ("[^n]: ", "    "),
            ("> [^n]: ", ">     "),
            ("- [^n]: ", "      "),
            ("> - [^n]: ", ">       "),
        ] {
            for eol in ["\n", "\r\n", ""] {
                let source = frame(c.body, open, prefix, eol);
                let d = parse(&source);
                assert_eq!(d.source(), source);
                assert_eq!(d.footnote_body_count(), 1);
                let v = d.footnote_body(FootnoteId(0)).unwrap();
                assert_eq!(
                    projected(v),
                    c.body
                        .replace('\n', if eol.is_empty() { "\n" } else { eol }),
                    "{}",
                    c.name
                );
                let actual = counts(v);
                for &(key, expected) in c.counts {
                    if let Some((_, value)) = actual.iter().find(|(k, _)| *k == key) {
                        assert_eq!(*value, expected, "{} {key} {source:?}", c.name);
                    }
                }
                if let Some(expected) = c.visible {
                    assert_eq!(shown(v), expected, "{} {source:?}", c.name);
                }
                assert!(
                    d.maths().is_empty()
                        && d.tables().is_empty()
                        && d.headings().entries().is_empty()
                        && d.footnote_references().is_empty()
                );
                assert!(matches!(
                    d.blocks()[0].kind,
                    BlockKind::FootnoteDefinition { .. }
                ));
            }
        }
    }
}
fn snapshot(d: &Document) -> String {
    let mut out = format!("{:?}|{:?}|", d.blocks(), d.footnotes());
    for i in 0..d.footnotes().len() {
        if let Some(v) = d.footnote_body(FootnoteId(i)) {
            out.push_str(&format!(
                "{:?}|{:?}|{:?}|",
                projected(v),
                shown(v),
                counts(v)
            ));
            for j in 0..v.block_count() {
                out.push_str(&format!(
                    "{:?}|",
                    v.block(v.qualify(BlockId(j))).unwrap().unwrap().value
                ));
            }
        }
    }
    out
}
#[test]
fn utf8_every_byte_cut_bytewise_and_each_prefix_eof_preserve_body_semantics() {
    let cases = [
        "[^n]: *中🙂* &amp; `a` [x][r] $q$\n\n    # 标题\n\n[r]: /late\n",
        "> - [^n]: first\r\n>\t\t*中🙂*\r\n>   \r\n>       [^n]: **literal** [^c]\r\n",
        "- [^n]: `old\n\t\tend` [^c] |b\n      ---|---\n- outside\n",
        "[^n]: first\r\n    second\r\n    tail\r",
        "[^n]: [r]: /u '中\n    🙂'\n[r]: /late\n",
    ];
    for source in cases {
        let expected = snapshot(&parse(source));
        for cut in 0..=source.len() {
            let mut d = Utf8Document::new();
            d.append(&source.as_bytes()[..cut]).unwrap();
            d.append(&source.as_bytes()[cut..]).unwrap();
            d.finish();
            assert_eq!(snapshot(d.document()), expected, "cut {cut}: {source:?}");
        }
        for end in 0..=source.len() {
            let bytes = &source.as_bytes()[..end];
            let expected = snapshot(&parse(&String::from_utf8_lossy(bytes)));
            let mut d = Utf8Document::new();
            for b in bytes {
                d.append(&[*b]).unwrap();
            }
            d.finish();
            assert_eq!(snapshot(d.document()), expected, "prefix {end}: {source:?}");
        }
    }
}
#[test]
fn canonical_body_number_is_not_its_independent_parser_number() {
    let d = parse("[^n]\n\n[^n]: [^c]\n[^c]: [^n]\n");
    let v = d.footnote_body(FootnoteId(0)).unwrap();
    assert_eq!(d.footnotes()[0].number, Some(1));
    match v
        .resolve_footnote(v.qualify(FootnoteReferenceId(0)), &mut tiny())
        .unwrap()
    {
        BodyResolveStep::Resolved(Some(r)) => {
            assert_eq!(r.number, 2);
            assert_eq!(r.symbol, FootnoteId(1));
        }
        _ => panic!(),
    }
    let c = d.footnote_body(FootnoteId(1)).unwrap();
    match c
        .resolve_footnote(c.qualify(FootnoteReferenceId(0)), &mut tiny())
        .unwrap()
    {
        BodyResolveStep::Resolved(Some(r)) => assert_eq!(r.number, 1),
        _ => panic!(),
    }
}
