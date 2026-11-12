use mdstream::*;
fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}
fn shown(doc: &Document) -> String {
    doc.display_runs_in(Span::new(0, doc.source().len()))
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect()
}
fn disposition(doc: &Document, id: LinkId) -> LinkDisposition {
    doc.link_activity(id)
        .unwrap()
        .advance(doc, &mut Budget::new(WorkLimit::unlimited()))
        .unwrap()
        .unwrap()
}
fn objects(doc: &Document) -> Vec<(LinkId, LinkKind, String)> {
    doc.links_in(Span::new(0, doc.source().len()))
        .filter(|&(id, _)| disposition(doc, id) == LinkDisposition::Active)
        .map(|(id, info)| (id, info.kind, doc.link_destination(id).unwrap().to_string()))
        .collect()
}
#[test]
fn nested_candidates_cover_the_official_structures_and_source_parents() {
    for (source, visible, kinds) in [
        (
            "[link [foo [bar]]](/uri)\n",
            "link [foo [bar]]\n",
            vec![LinkKind::Link],
        ),
        (
            "[link [foo [bar]]][ref]\n\n[ref]: /uri\n",
            "link [foo [bar]]\n\n",
            vec![LinkKind::Link],
        ),
        (
            "[![moon](moon.jpg)](/uri)\n",
            "moon\n",
            vec![LinkKind::Link, LinkKind::Image],
        ),
        (
            "[![moon](moon.jpg)][ref]\n\n[ref]: /uri\n",
            "moon\n\n",
            vec![LinkKind::Link, LinkKind::Image],
        ),
        (
            "![[[foo](uri1)](uri2)](uri3)\n",
            "[foo](uri2)\n",
            vec![LinkKind::Image, LinkKind::Link],
        ),
        (
            "![foo ![bar](/url)](/url2)\n",
            "foo bar\n",
            vec![LinkKind::Image, LinkKind::Image],
        ),
        (
            "![foo [bar](/url)](/url2)\n",
            "foo bar\n",
            vec![LinkKind::Image, LinkKind::Link],
        ),
    ] {
        let doc = parse(source);
        assert_eq!(shown(&doc), visible, "{source:?}");
        let actual = objects(&doc);
        assert_eq!(
            actual.iter().map(|o| o.1).collect::<Vec<_>>(),
            kinds,
            "{source:?}"
        );
        if actual.len() == 2 {
            assert_eq!(doc.link_parent(actual[1].0), Some(actual[0].0));
        }
        for (source_at, _) in source.char_indices() {
            let window = Span::new(source_at, (source_at + 1).min(source.len()));
            let mut expected: Vec<_> = (0..doc.links().len())
                .map(LinkId)
                .filter(|&id| {
                    let s = doc.link_info(id).unwrap().source;
                    s.start < window.end && window.start < s.end
                })
                .collect();
            expected.sort_by_key(|&id| (doc.link_info(id).unwrap().source.start, id.0));
            assert_eq!(
                doc.links_in(window).map(|(id, _)| id).collect::<Vec<_>>(),
                expected
            );
        }
    }
}
#[test]
fn inner_links_disable_every_link_ancestor_through_images_and_keep_suffix_priority() {
    for (source, visible, targets) in [
        (
            "[foo [bar](/uri)](/uri)\n",
            "[foo bar](/uri)\n",
            vec!["/uri"],
        ),
        (
            "[foo *[bar [baz](/uri)](/uri)*](/uri)\n",
            "[foo [bar baz](/uri)](/uri)\n",
            vec!["/uri"],
        ),
        ("[![a [b](u)](v)](w)", "[a b](w)", vec!["v", "u"]),
        (
            "[foo *bar [baz][ref]*][ref]\n\n[ref]: /uri\n",
            "[foo bar baz]ref\n\n",
            vec!["/uri", "/uri"],
        ),
        (
            "[a [b][r]][x](u)\n\n[r]: /r\n[x]: /x\n",
            "[a b]x\n\n",
            vec!["/r", "u"],
        ),
    ] {
        let doc = parse(source);
        assert_eq!(shown(&doc), visible, "{source:?}");
        assert_eq!(
            objects(&doc)
                .iter()
                .map(|o| o.2.as_str())
                .collect::<Vec<_>>(),
            targets,
            "{source:?}"
        );
    }
}
#[test]
fn late_definitions_change_selection_without_visiting_or_replacing_old_objects() {
    let mut doc = Document::new();
    doc.append("[a [r]](u)\n\n").unwrap();
    let ids: Vec<_> = doc
        .links_in(Span::new(0, doc.source().len()))
        .map(|(id, info)| (id, *info, doc.links()[id.0]))
        .collect();
    assert_eq!(shown(&doc), "a [r]\n\n");
    let before = doc.work();
    let update = doc.append("[r]: /r\n").unwrap();
    assert_eq!(shown(&doc), "[a r](u)\n\n");
    for (id, info, target) in ids {
        assert_eq!(doc.link_info(id), Some(&info));
        assert_eq!(doc.links()[id.0], target);
    }
    assert_eq!(doc.work().link_index_steps, before.link_index_steps);
    assert_eq!(doc.work().link_gate_steps, before.link_gate_steps);
    assert!(
        update
            .changes
            .iter()
            .any(|c| c.kind == ChangeKind::Visibility && c.source.start == 0)
    );
    assert_eq!(
        objects(&doc)
            .iter()
            .map(|o| o.2.as_str())
            .collect::<Vec<_>>(),
        ["/r"]
    );
}

#[path = "support/snapshot.rs"]
mod support;
fn limits(bytes: usize) -> WorkLimit {
    WorkLimit {
        bytes,
        queries: 1,
        objects: 1,
        tiles: 1,
        recovery: 1,
        dependencies: 1,
    }
}
fn bounded(b: &Budget, l: WorkLimit) {
    let u = b.used();
    assert!(
        u.bytes <= l.bytes
            && u.queries <= l.queries
            && u.objects <= l.objects
            && u.tiles <= l.tiles
            && u.recovery <= l.recovery
            && u.dependencies <= l.dependencies
    );
}
fn semantics(doc: &Document) -> Vec<(LinkInfo, Option<LinkId>, LinkDisposition)> {
    let mut resolver = doc.link_resolver();
    (0..doc.links().len())
        .map(|i| {
            let id = LinkId(i);
            let d = doc
                .link_activity(id)
                .unwrap()
                .advance_with(doc, &mut resolver, &mut Budget::new(WorkLimit::unlimited()))
                .unwrap()
                .unwrap();
            (*doc.link_info(id).unwrap(), doc.link_parent(id), d)
        })
        .collect()
}
#[test]
fn arbitrary_utf8_byte_chunks_prefixes_and_container_boundaries_preserve_candidates() {
    let mut count = 0;
    for body in [
        "[前 [中 [🙂]]](/u)",
        "[![前 *中*](v)](u)",
        "![前 [中][r]](v)",
        "[前 [中][r]][x](u)",
        "![前 ![中](v)](bad z",
        "*[前 ![中 _文_](v)](u)*",
        "![前 [中](u)",
        "[前 ![中](v)](unfinished",
        "[前 [中][r]](u)\n\n[r]: /r\n",
    ] {
        for (prefix, separator) in [
            ("", "\n"),
            ("> ", "\r\n> "),
            ("- > ", "\n  > "),
            ("> ", "\n"),
        ] {
            let source = format!(
                "{prefix}{}\n",
                body.replace("前 ", &format!("前{separator}"))
            );
            let whole = parse(&source);
            for size in [1, 2, 3, 7, 31] {
                let mut bytes = Utf8Document::new();
                for chunk in source.as_bytes().chunks(size) {
                    bytes.append(chunk).unwrap();
                }
                bytes.finish();
                assert_eq!(
                    support::snapshot(bytes.document()),
                    support::snapshot(&whole),
                    "{source:?}, {size}"
                );
                assert_eq!(semantics(bytes.document()), semantics(&whole));
                count += 1;
            }
            let mut incremental = Document::new();
            for (at, ch) in source.char_indices() {
                let end = at + ch.len_utf8();
                incremental.append(&source[at..end]).unwrap();
                let mut once = Document::new();
                once.append(&source[..end]).unwrap();
                assert_eq!(
                    support::snapshot(&incremental),
                    support::snapshot(&once),
                    "prefix {end}, {source:?}"
                );
                assert_eq!(semantics(&incremental), semantics(&once));
                let mut split = Document::new();
                split.append(&source[..at]).unwrap();
                split.append(&source[at..end]).unwrap();
                split.finish();
                let eof = parse(&source[..end]);
                assert_eq!(support::snapshot(&split), support::snapshot(&eof));
                assert_eq!(semantics(&split), semantics(&eof));
                count += 1;
            }
        }
    }
    eprintln!("{count} byte-chunk/prefix/EOF comparisons");
}
#[test]
fn failed_ancestors_keep_real_children_and_syntax_scopes() {
    for (source, visible, kinds) in [
        ("![a ![b](v)](bad z", "![a b](bad z", vec![LinkKind::Image]),
        ("![a [b](v)", "![a b", vec![LinkKind::Link]),
        ("[a ![b](v)", "[a b", vec![LinkKind::Image]),
        ("[[b](v)](u)", "[b](u)", vec![LinkKind::Link]),
        (
            "![a [b](v)](u)",
            "a b",
            vec![LinkKind::Image, LinkKind::Link],
        ),
    ] {
        let doc = parse(source);
        assert_eq!(shown(&doc), visible, "{source:?}");
        assert_eq!(objects(&doc).iter().map(|o| o.1).collect::<Vec<_>>(), kinds);
    }
    for marker in ["*", "_", "**", "__", "~", "~~"] {
        let source = format!("p {marker}前 ![签 {marker}中 ![文](v)](u) 后{marker} 尾");
        let doc = parse(&source);
        let before = source.find('前').unwrap();
        let inside = source.find('中').unwrap();
        let after = source.find('后').unwrap();
        let style = |at| {
            let r = doc.runs_in(Span::new(at, at + 3)).unwrap().next().unwrap();
            (
                r.emphasis_depth,
                r.strong_depth,
                r.style.contains(Style::STRIKE),
            )
        };
        let outer = match marker {
            "*" | "_" => (1, 0, false),
            "**" | "__" => (0, 1, false),
            _ => (0, 0, true),
        };
        assert_eq!(style(before), outer, "{source:?}");
        assert_eq!(style(inside), outer, "{source:?}");
        assert_eq!(style(after), outer, "{source:?}");
    }
}
#[derive(Default)]
struct NestedSink {
    text: String,
    objects: Vec<(LinkId, LinkDisposition)>,
}
impl LayoutSink for NestedSink {
    fn begin_tile(&mut self, _: TileId) {}
    fn text(&mut self, r: RenderRun<'_>) {
        if !r.style.contains(Style::HIDDEN) {
            self.text.push_str(r.text);
        }
    }
    fn object(&mut self, _: LinkId, _: &LinkInfo) {
        panic!("selection must accompany callbacks");
    }
    fn object_with_disposition(&mut self, id: LinkId, _: &LinkInfo, d: LinkDisposition) {
        assert_ne!(d, LinkDisposition::Suppressed);
        self.objects.push((id, d));
    }
    fn end_tile(&mut self, _: TileId) -> u64 {
        1
    }
}
#[test]
fn display_layout_and_objects_share_paid_semantics_and_resolver_lifecycle() {
    for source in [
        "[a ![b](v)](u)",
        "![a [b][r]](u)\n\n[r]: /r\n",
        "[a [r]](u)\n\n[r]: /r\n",
        "[a [r]](u)",
        "[a [b][r]][x](u)\n\n[r]: /r\n[x]: /x\n",
    ] {
        let doc = parse(source);
        let span = Span::new(0, source.len());
        let expected = shown(&doc);
        let mut cursor = doc.display_cursor(span).unwrap();
        let mut resolver = doc.link_resolver();
        let mut output = String::new();
        let mut done = false;
        for _ in 0..source.len() * 80 + 100 {
            let l = limits(6);
            let mut budget = Budget::new(l);
            match cursor
                .next_with_resolver(&doc, &mut resolver, &mut budget)
                .unwrap()
            {
                DisplayStep::Run(r) => {
                    assert!(doc.text(r.source).is_some());
                    if r.replacement.is_none() {
                        assert_eq!(doc.text(r.source), Some(r.text));
                    }
                    if !r.style.contains(Style::HIDDEN) {
                        output.push_str(r.text);
                    }
                }
                DisplayStep::Complete => {
                    done = true;
                    break;
                }
                DisplayStep::BudgetExhausted => {}
            }
            bounded(&budget, l);
        }
        assert!(done, "display {source:?}");
        assert_eq!(output, expected);
        // Reuse the very same memo for selection and layout. Construction and
        // a zero-budget frame do not evaluate a remote descendant.
        let mut heights = HeightIndex::new(1).unwrap();
        heights.rebuild(&doc).unwrap();
        let mut pass = LayoutPass::new(&doc, &heights, TileId(0), 1000, true).unwrap();
        pass.swap_link_resolver(&mut resolver).unwrap();
        let mut sink = NestedSink::default();
        let mut zero = Budget::new(WorkLimit::default());
        assert_eq!(
            pass.advance(&doc, &mut heights, &mut zero, &mut sink)
                .unwrap(),
            Completion::BudgetExhausted
        );
        assert!(sink.text.is_empty() && sink.objects.is_empty());
        done = false;
        for _ in 0..source.len() * 100 + 100 {
            let l = limits(6);
            let mut b = Budget::new(l);
            let state = pass.advance(&doc, &mut heights, &mut b, &mut sink).unwrap();
            bounded(&b, l);
            if state == Completion::EndOfDocument {
                done = true;
                break;
            }
        }
        assert!(done, "layout {source:?}");
        assert_eq!(sink.text, expected);
        let wanted: Vec<_> = doc
            .links_in(span)
            .filter_map(|(id, _)| {
                let d = disposition(&doc, id);
                (d != LinkDisposition::Suppressed).then_some((id, d))
            })
            .collect();
        assert_eq!(sink.objects, wanted, "{source:?}");
        let mut old = pass.take_link_resolver();
        let n = old.memo_entries();
        let mut paid = 0;
        loop {
            let mut b = Budget::new(limits(0));
            let state = old.retire(&mut b);
            paid += b.used().objects;
            bounded(&b, limits(0));
            if state == Completion::Covered {
                break;
            }
        }
        assert_eq!(paid, n);
        old.reset(&doc).unwrap();
        assert_eq!(old.memo_entries(), 0);
    }
}
#[test]
fn remote_label_dependencies_are_resumable_and_invalidate_on_late_definition() {
    for child in ["[r]", "![r]"] {
        let mut doc = Document::new();
        doc.append(&format!("[{} {child}](u)\n\n", "a".repeat(7000)))
            .unwrap();
        let parent = doc.links_in(Span::new(0, 1)).next().unwrap().0;
        assert_eq!(disposition(&doc, parent), LinkDisposition::Active);
        let mut capture = DependencyCapture::new(&doc, TileId(0)).unwrap();
        let mut zero = Budget::new(WorkLimit::default());
        assert!(matches!(
            capture.advance(&doc, &mut zero).unwrap(),
            DependencyCaptureStep::BudgetExhausted
        ));
        assert_eq!(capture.memo_entries(), 0);
        let mut key = None;
        for _ in 0..10000 {
            let l = limits(6);
            let mut b = Budget::new(l);
            match capture.advance(&doc, &mut b).unwrap() {
                DependencyCaptureStep::Captured(k) => {
                    key = Some(k);
                    break;
                }
                DependencyCaptureStep::BudgetExhausted => {}
                DependencyCaptureStep::Complete => panic!(),
            }
            bounded(&b, l);
        }
        let key = key.expect("capture progresses");
        assert_eq!(capture.object_count(), 2);
        assert_eq!(
            key.validate(&doc, &mut Budget::new(WorkLimit::unlimited()))
                .unwrap(),
            CacheValidity::Valid
        );
        let n = capture.memo_entries();
        let mut paid = 0;
        loop {
            let mut b = Budget::new(limits(0));
            let c = capture.retire(&mut b);
            paid += b.used().objects;
            if c == Completion::Covered {
                break;
            }
        }
        assert_eq!(paid, n);
        let update = doc.append("[r]: /r\n").unwrap();
        assert!(
            update
                .changes
                .iter()
                .any(|c| c.source.start == 0 && c.kind == ChangeKind::Reference)
        );
        assert_eq!(
            key.validate(&doc, &mut Budget::new(WorkLimit::unlimited()))
                .unwrap(),
            CacheValidity::Changed
        );
        assert_eq!(
            disposition(&doc, parent),
            if child == "[r]" {
                LinkDisposition::Suppressed
            } else {
                LinkDisposition::Active
            }
        );
    }
}
#[test]
fn deep_candidates_and_million_byte_labels_have_linear_storage_and_work() {
    let depth = 16384;
    let source = format!("{}x{}", "![".repeat(depth), "][r]".repeat(depth));
    let doc = parse(&source);
    assert_eq!(doc.links().len(), depth);
    assert_eq!(doc.link_guard_stats().nodes, 0);
    let at = 2 * depth;
    let mut window = doc.link_window(Span::new(at, at + 1));
    let mut ids = Vec::new();
    let mut frames = 0;
    loop {
        frames += 1;
        let mut b = Budget::new(limits(6));
        match window.advance(&doc, &mut b).unwrap() {
            LinkStep::Object(id, _) => ids.push(id),
            LinkStep::Complete => break,
            LinkStep::BudgetExhausted => {}
        }
        bounded(&b, limits(6));
        assert!(frames < depth * 100);
    }
    assert_eq!(ids.len(), depth);
    assert!(window.work().index_visits < 10 * depth as u64 + 256);
    assert!(window.work().sort_steps < 400 * depth as u64);
    let stats = doc.link_index_stats();
    assert!(stats.nodes <= 3 * depth);
    assert!(stats.capacity_bytes <= 1024 * depth);
    let mut cursor = doc.display_cursor(Span::new(at, at + 1)).unwrap();
    let mut resolver = doc.link_resolver();
    let mut b = Budget::new(WorkLimit::unlimited());
    assert!(matches!(
        cursor
            .next_with_resolver(&doc, &mut resolver, &mut b)
            .unwrap(),
        DisplayStep::Run(_)
    ));
    let work = resolver.work();
    assert!(work.parent_steps <= depth as u64 + 1);
    assert!(work.owner_steps <= 3 * depth as u64 + 2);
    assert!(work.target_reads <= depth as u64 + 1);
    let mut growing = Document::new();
    growing
        .append(&format!("{}{}", "![".repeat(1024), "a".repeat(1_000_000)))
        .unwrap();
    let before = growing.work();
    let storage = growing.link_parser_stats();
    for _ in 0..1000 {
        growing.append("a").unwrap();
    }
    let after = growing.work();
    assert_eq!(after.link_label_steps, before.link_label_steps);
    assert_eq!(growing.link_parser_stats(), storage);
    growing.append(&"](u)".repeat(1024)).unwrap();
    growing.finish();
    assert_eq!(growing.links().len(), 1024);
    assert!(growing.work().link_label_steps < 20 * growing.source().len() as u64);
    assert!(growing.work().link_index_steps < 1000 * growing.links().len() as u64);
}

#[test]
fn independent_small_forests_select_innermost_links_through_every_image_path() {
    // A batch tree oracle is confined to the test. Production stores one parent
    // edge and shared predicates, never an ancestor chain or a text AST.
    struct Tree {
        image: bool,
        reference: bool,
        children: Vec<Tree>,
        key: usize,
    }
    impl Tree {
        fn source(&self, out: &mut String) {
            out.push_str(if self.image { "![" } else { "[" });
            out.push_str(&format!("t{}", self.key));
            for child in &self.children {
                out.push(' ');
                child.source(out);
            }
            out.push(']');
            out.push_str(&if self.reference {
                format!("[r{}]", self.key)
            } else {
                format!("(u{})", self.key)
            });
        }
        fn available_link(&self, defined: bool) -> bool {
            (!self.image && (!self.reference || defined))
                || self.children.iter().any(|c| c.available_link(defined))
        }
        fn shown(&self, defined: bool) -> String {
            let blocked = !self.image && self.children.iter().any(|c| c.available_link(defined));
            let active = (!self.reference || defined) && !blocked;
            let label = format!(
                "t{}{}",
                self.key,
                self.children
                    .iter()
                    .map(|c| format!(" {}", c.shown(defined)))
                    .collect::<String>()
            );
            if active {
                label
            } else {
                format!(
                    "{}[{label}]{}",
                    if self.image { "!" } else { "" },
                    if self.reference {
                        format!("[r{}]", self.key)
                    } else {
                        format!("(u{})", self.key)
                    }
                )
            }
        }
        fn active(&self, defined: bool, out: &mut Vec<(LinkKind, String)>) {
            if (!self.reference || defined)
                && !(!self.image && self.children.iter().any(|c| c.available_link(defined)))
            {
                out.push((
                    if self.image {
                        LinkKind::Image
                    } else {
                        LinkKind::Link
                    },
                    if self.reference {
                        format!("/r{}", self.key)
                    } else {
                        format!("u{}", self.key)
                    },
                ));
            }
            for c in &self.children {
                c.active(defined, out);
            }
        }
    }
    let mut cases = 0;
    for shape in [0, 1] {
        for bits in 0usize..16 {
            for refs in 0usize..4 {
                let leaf = |key: usize| Tree {
                    image: bits & (1 << key) != 0,
                    reference: refs & (1 << (key - 2)) != 0,
                    children: vec![],
                    key,
                };
                let middle = Tree {
                    image: bits & 2 != 0,
                    reference: false,
                    children: if shape == 0 {
                        vec![leaf(2), leaf(3)]
                    } else {
                        vec![leaf(2)]
                    },
                    key: 1,
                };
                let root = Tree {
                    image: bits & 1 != 0,
                    reference: false,
                    children: if shape == 0 {
                        vec![middle]
                    } else {
                        vec![middle, leaf(3)]
                    },
                    key: 0,
                };
                let mut source = String::new();
                root.source(&mut source);
                source.push_str("\n\n");
                let mut doc = Document::new();
                doc.append(&source).unwrap();
                for defined in [false, true] {
                    if defined {
                        doc.append("[r2]: /r2\n[r3]: /r3\n").unwrap();
                    }
                    assert_eq!(
                        shown(&doc),
                        format!("{}\n\n", root.shown(defined)),
                        "{source:?} defined={defined}"
                    );
                    let actual = objects(&doc)
                        .into_iter()
                        .map(|(_, k, u)| (k, u))
                        .collect::<Vec<_>>();
                    let mut expected = Vec::new();
                    root.active(defined, &mut expected);
                    assert_eq!(actual, expected, "{source:?} defined={defined}");
                    cases += 1;
                }
            }
        }
    }
    assert_eq!(cases, 256);
}

impl FoldLayoutSink for NestedSink {
    fn placeholder(&mut self, _: FoldPlaceholder) {
        self.text.push('◇');
    }
}
#[test]
fn overlay_restarts_do_not_duplicate_nested_objects_and_search_uses_real_display_text() {
    let source = format!(
        "[a {} ![b](v) {}](u) tail\n",
        "x".repeat(1500),
        "y".repeat(1500)
    );
    let doc = parse(&source);
    let mut view = FoldView::new(&doc, 1).unwrap();
    let raw = Span::new(100, 200);
    let mut registration = SourceFoldRegistration::overlapping(&doc, &view, raw).unwrap();
    let fold = loop {
        let mut b = Budget::new(limits(6));
        let p = registration.advance(&doc, &mut view, &mut b).unwrap();
        if let Some(id) = p.id {
            break id;
        }
    };
    view.set_collapsed(&doc, fold, true).unwrap();
    let mut pass = LayoutPass::folded(&doc, &view, TileId(0), u64::MAX).unwrap();
    let mut sink = NestedSink::default();
    let mut complete = false;
    for _ in 0..30000 {
        let l = limits(6);
        let mut b = Budget::new(l);
        let c = pass
            .advance_folded(&doc, &mut view, &mut b, &mut sink)
            .unwrap();
        bounded(&b, l);
        if c == Completion::EndOfDocument {
            complete = true;
            break;
        }
    }
    assert!(complete);
    assert_eq!(sink.objects.len(), 2);
    assert_ne!(sink.objects[0].0, sink.objects[1].0);
    let doc = parse("[a [r]](u)\n\n[r]: /r\n");
    let expected = shown(&doc);
    assert_eq!(expected, "[a r](u)\n\n");
    let mut search = DisplaySearch::new(&doc, "a r").unwrap();
    let mut hits = Vec::new();
    let mut complete = false;
    for _ in 0..1000 {
        let l = limits(6);
        let mut b = Budget::new(l);
        let c = search.advance(&doc, &mut b, &mut hits).unwrap();
        bounded(&b, l);
        if c == Completion::EndOfDocument {
            complete = true;
            break;
        }
    }
    assert!(complete);
    assert_eq!(hits.len(), 1);
    assert_eq!(doc.text(hits[0].source), Some("a [r"));
    let mut resolver = search.take_link_resolver();
    let n = resolver.memo_entries();
    assert!(n > 0);
    let mut paid = 0;
    loop {
        let mut b = Budget::new(limits(0));
        let done = resolver.retire(&mut b);
        paid += b.used().objects;
        if done == Completion::Covered {
            break;
        }
    }
    assert_eq!(paid, n);
}

#[test]
fn real_frame_memo_is_moved_and_retired_after_revision_changes() {
    let mut doc = Document::new();
    doc.append(&format!("[a {}](u)\n\n", "[r] ".repeat(512)))
        .unwrap();
    let mut heights = HeightIndex::new(1).unwrap();
    heights.rebuild(&doc).unwrap();
    let mut pass = LayoutPass::new(&doc, &heights, TileId(0), u64::MAX, true).unwrap();
    let mut sink = NestedSink::default();
    let mut frames = 0;
    while sink.objects.is_empty() {
        frames += 1;
        let l = limits(6);
        let mut b = Budget::new(l);
        pass.advance(&doc, &mut heights, &mut b, &mut sink).unwrap();
        bounded(&b, l);
        assert!(frames < 12000);
    }
    let mut memo = pass.take_link_resolver();
    assert!(memo.memo_entries() > 1000);
    let old = memo.memo_entries();
    drop(pass);
    doc.append("x").unwrap();
    let mut zero = Budget::new(WorkLimit::default());
    assert_eq!(memo.retire(&mut zero), Completion::BudgetExhausted);
    assert_eq!(memo.memo_entries(), old);
    let mut paid = 0;
    for left in (0..old).rev() {
        let l = limits(0);
        let mut b = Budget::new(l);
        let c = memo.retire(&mut b);
        bounded(&b, l);
        paid += b.used().objects;
        assert_eq!(memo.memo_entries(), left);
        assert_eq!(
            c,
            if left == 0 {
                Completion::Covered
            } else {
                Completion::BudgetExhausted
            }
        );
    }
    assert_eq!(paid, old);
    assert_eq!(memo.work().retired_entries, old as u64);
    memo.reset(&doc).unwrap();
    let mut next = doc.display_cursor(Span::new(0, 1)).unwrap();
    next.swap_link_resolver(&mut memo).unwrap();
    assert_eq!(memo.memo_entries(), 0);
}
#[test]
fn historical_objects_do_not_enlarge_sparse_or_cross_tile_window_work() {
    for history in [0, 20000] {
        let mut doc = Document::new();
        doc.append(&"![old](u) ".repeat(history)).unwrap();
        let start = doc.source().len();
        doc.append(&format!("[{} ![中](v)](u)\n", "a".repeat(TILE_BYTES * 2)))
            .unwrap();
        doc.finish();
        let point = start + TILE_BYTES;
        let mut cursor = doc.link_window(Span::new(point, point + 1));
        let mut out = Vec::new();
        loop {
            let l = limits(6);
            let mut b = Budget::new(l);
            match cursor.advance(&doc, &mut b).unwrap() {
                LinkStep::Object(id, _) => out.push(id),
                LinkStep::Complete => break,
                LinkStep::BudgetExhausted => {}
            }
            bounded(&b, l);
        }
        assert_eq!(out.len(), 1);
        assert_eq!(doc.link_info(out[0]).unwrap().source.start, start);
        assert!(
            cursor.work().index_visits
                <= 16 * (usize::BITS - doc.links().len().max(1).leading_zeros()) as u64 + 32
        );
        let stats = doc.link_index_stats();
        assert!(stats.nodes <= 3 * doc.links().len());
        assert!(stats.capacity_bytes <= 1024 * doc.links().len().max(4));
        let n = doc.source().len() as u64;
        assert!(doc.work().link_index_steps <= 100 * n);
        assert!(doc.work().link_label_steps <= 20 * n);
        eprintln!(
            "history={history}, window={:?}, index={stats:?}, parser={:?}, guards={:?}",
            cursor.work(),
            doc.link_parser_stats(),
            doc.link_guard_stats()
        );
    }
}

#[test]
fn conditional_full_suffix_chains_retain_both_ordinary_label_branches() {
    for defined in [false, true] {
        for tail in ["[y]", "[y](v)", "[y][z]", "[y][z](v)"] {
            let source = format!(
                "[a [b][r]][x]{tail}\n\n[x]: /x\n[y]: /y\n[z]: /z\n{}",
                if defined { "[r]: /r\n" } else { "" }
            );
            let doc = parse(&source);
            let actual = objects(&doc)
                .iter()
                .map(|o| o.2.clone())
                .collect::<Vec<_>>();
            let expected = if defined {
                match tail {
                    "[y]" | "[y](v)" => vec!["/r", "/y"],
                    "[y][z]" => vec!["/r", "/y", "/z"],
                    _ => vec!["/r", "/y", "v"],
                }
            } else {
                match tail {
                    "[y]" => vec!["/x", "/y"],
                    "[y](v)" => vec!["/x", "v"],
                    _ => vec!["/x", "/z"],
                }
            };
            assert_eq!(actual, expected, "{source:?}");
            let mut stream = Utf8Document::new();
            for byte in source.as_bytes().chunks(1) {
                stream.append(byte).unwrap();
            }
            stream.finish();
            assert_eq!(semantics(stream.document()), semantics(&doc));
        }
    }
    // A suffix alternative can be the only effective child of another link.
    // Its source parent must receive the shared blocker summary, too.
    let source = "[outer [a [b][r]][x][y]](u)\n\n[y]: /y\n";
    let doc = parse(source);
    assert_eq!(
        objects(&doc)
            .iter()
            .map(|o| o.2.as_str())
            .collect::<Vec<_>>(),
        ["/y"]
    );
}

#[test]
fn long_conditional_suffix_chains_have_one_candidate_per_label_and_shared_guards() {
    // Independent batch pairing: without r the outer consumes x, then y/z
    // pair and w stands alone; with r, x/y and z/w pair. Availability selects
    // active targets after pairing, matching the retained full-reference dialect.
    for mask in 0usize..16 {
        for defined in [false, true] {
            for wrapper in [false, true] {
                let body = "[a [b][r]][x][y][z][w]";
                let mut source = if wrapper {
                    format!("[ancestor {body}](u)\n\n")
                } else {
                    format!("{body}\n\n")
                };
                for (i, key) in ["x", "y", "z", "w"].iter().enumerate() {
                    if mask & (1 << i) != 0 {
                        source.push_str(&format!("[{key}]: /{key}\n"));
                    }
                }
                if defined {
                    source.push_str("[r]: /r\n");
                }
                let selected = if defined {
                    vec![("r", true), ("y", mask & 2 != 0), ("w", mask & 8 != 0)]
                } else {
                    vec![
                        ("x", mask & 1 != 0),
                        ("z", mask & 4 != 0),
                        ("w", mask & 8 != 0),
                    ]
                };
                let mut expected: Vec<_> = selected
                    .into_iter()
                    .filter(|(_, yes)| *yes)
                    .map(|(key, _)| format!("/{key}"))
                    .collect();
                if wrapper && expected.is_empty() {
                    expected.push("u".to_owned());
                }
                assert_eq!(
                    objects(&parse(&source))
                        .iter()
                        .map(|o| o.2.clone())
                        .collect::<Vec<_>>(),
                    expected,
                    "{source:?}"
                );
            }
        }
    }
    for n in [1, 32, 4096] {
        let mut doc = Document::new();
        doc.append(&format!(
            "[a [r]][x]{}\n\n[x]: /x\n[y]: /y\n",
            "[y]".repeat(n)
        ))
        .unwrap();
        assert_eq!(doc.links().len(), n + 3);
        let active = |doc: &Document| {
            semantics(doc)
                .iter()
                .filter(|(_, _, d)| *d == LinkDisposition::Active)
                .count()
        };
        assert_eq!(active(&doc), 1 + n.div_ceil(2));
        let before = doc.work();
        let infos: Vec<_> = (0..doc.links().len())
            .map(|i| *doc.link_info(LinkId(i)).unwrap())
            .collect();
        doc.append("[r]: /r\n").unwrap();
        assert_eq!(active(&doc), 1 + (n + 2) / 2);
        assert_eq!(doc.work().link_index_steps, before.link_index_steps);
        assert_eq!(doc.work().link_gate_steps, before.link_gate_steps);
        for (i, info) in infos.iter().enumerate() {
            assert_eq!(doc.link_info(LinkId(i)), Some(info));
        }
        assert!(doc.work().link_label_steps < 30 * doc.source().len() as u64);
        assert!(doc.link_guard_stats().nodes <= doc.links().len());
    }
}
