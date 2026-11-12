use mdstream::*;

#[path = "support/snapshot.rs"]
mod snapshot;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn visible(doc: &Document) -> String {
    doc.display_runs_in(Span::new(0, doc.source().len()))
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect()
}

// Deliberately naive, independent whole-line model. Expand tabs into physical
// columns first, then rescan the remaining suffix at every level. Production
// must never use this algorithm: its quadratic cost is useful as an oracle.
fn model(source: &str) -> (Vec<&'static str>, &'static str) {
    let raw = source.strip_suffix('\n').unwrap_or(source);
    let raw = if source.ends_with('\n') {
        raw.strip_suffix('\r').unwrap_or(raw)
    } else {
        raw
    };
    let mut expanded = String::new();
    for byte in raw.bytes() {
        if byte == b'\t' {
            expanded.extend(std::iter::repeat_n(' ', 4 - expanded.len() % 4));
        } else {
            expanded.push(char::from(byte));
        }
    }
    let mut rest = expanded.as_str();
    let mut scopes = Vec::new();
    loop {
        let trim = rest.trim_start_matches(' ');
        let indent = rest.len() - trim.len();
        if trim.bytes().all(|b| b.is_ascii_whitespace()) {
            return (scopes, "blank");
        }
        if indent >= 4 {
            return (scopes, "code");
        }
        let bytes = trim.as_bytes();
        if matches!(bytes[0], b'*' | b'-' | b'_')
            && bytes.iter().filter(|&&b| b == bytes[0]).count() >= 3
            && bytes.iter().all(|&b| b == bytes[0] || b == b' ')
        {
            return (scopes, "rule");
        }
        if let Some(quoted) = trim.strip_prefix('>') {
            scopes.push("quote");
            rest = quoted.strip_prefix(' ').unwrap_or(quoted);
            continue;
        }
        let marker = if matches!(bytes[0], b'-' | b'+' | b'*') {
            1
        } else {
            let digits = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
            if (1..=9).contains(&digits)
                && bytes.get(digits).is_some_and(|b| matches!(b, b'.' | b')'))
            {
                digits + 1
            } else {
                0
            }
        };
        if marker > 0 && (marker == bytes.len() || bytes[marker] == b' ') {
            scopes.extend(["list", "item"]);
            let tail = &trim[marker..];
            let body = tail.trim_start_matches(' ');
            if body.is_empty() {
                return (scopes, "blank");
            }
            if tail.len() - body.len() >= 5 {
                return (scopes, "code");
            }
            rest = body;
            continue;
        }
        if trim.starts_with("$$") && trim[2..].bytes().all(|b| b == b' ') {
            return (scopes, "math");
        }
        let hashes = bytes.iter().take_while(|&&b| b == b'#').count();
        if (1..=6).contains(&hashes) && (hashes == bytes.len() || bytes[hashes] == b' ') {
            return (scopes, "heading");
        }
        if trim.starts_with("```") || trim.starts_with("~~~") {
            return (scopes, "code");
        }
        return (scopes, "text");
    }
}

fn production_shape(doc: &Document) -> (Vec<&'static str>, &'static str) {
    let scopes = (0..doc.container_count())
        .map(|i| match doc.container(ContainerId(i)).unwrap().kind {
            ContainerKind::Quote => "quote",
            ContainerKind::List { .. } => "list",
            ContainerKind::Item { .. } => "item",
        })
        .collect();
    let leaf = match doc.blocks()[0].kind {
        BlockKind::Blank => "blank",
        BlockKind::Code { .. } => "code",
        BlockKind::ThematicBreak => "rule",
        BlockKind::Heading { .. } => "heading",
        BlockKind::Math { .. } => "math",
        _ => "text",
    };
    (scopes, leaf)
}

#[test]
fn naive_model_matches_short_and_unbounded_mixed_prefix_and_rule_priority() {
    for depth in [0, 1, 7, 513, 4097] {
        for unit in ["> ", ">\t", "- ", "- + ", "> - "] {
            let prefix = unit.repeat(depth);
            for tail in [
                "text",
                "# head",
                "$$",
                "  $$\t",
                "    code",
                "``` rs",
                "***",
                "- * * *",
                "_ _ _",
                "- - - x",
                "+",
                "123456789. x",
                "1234567890. x",
            ] {
                let source = format!("{prefix}{tail}\r\n");
                let expected = model(&source);
                let doc = parse(&source);
                assert_eq!(
                    production_shape(&doc),
                    expected,
                    "depth={depth} unit={unit:?} tail={tail:?}"
                );
                assert!(doc.work().classified_bytes < 16 * source.len() as u64);
                assert!(doc.work().container_bytes < 40 * source.len() as u64);
                assert!(doc.work().container_steps < 12 * source.len() as u64);
            }
        }
    }
    for count in [3, 1023, 1024, 8193] {
        for prefix in ["", "- ", "+ - ", "> - + "] {
            for marker in ["*", "-", "_"] {
                let source = format!("{prefix}{}\n", format!("{marker} ").repeat(count));
                assert_eq!(production_shape(&parse(&source)), model(&source));
            }
        }
    }
}

#[test]
fn whole_line_candidates_are_provisional_then_confirm_or_fail_once() {
    for prefix in ["", "> ", "- ", "> - "] {
        for tail in [
            "* ".repeat(4097),
            "_\t".repeat(2049),
            " ".repeat(8193),
            format!("$${}", "\t".repeat(8193)),
        ] {
            let raw = format!("{prefix}{tail}");
            let mut doc = Document::new();
            doc.append(&raw).unwrap();
            assert!(doc.blocks().is_empty());
            assert_eq!(doc.container_count(), 0);
            assert_eq!(visible(&doc), raw);
            assert!(doc.pending().len() <= BLOCK_LOOKAHEAD + 3);
            let update = doc.append("\r\n").unwrap();
            assert_eq!(production_shape(&doc), model(&format!("{raw}\r\n")));
            assert!(
                update
                    .dirty
                    .iter()
                    .any(|s| s.start == 0 && s.end >= raw.len())
            );
        }
    }
    for tail in ["* ".repeat(8193), format!("$${}", " ".repeat(8193))] {
        let mut doc = Document::new();
        doc.append(&tail).unwrap();
        doc.append("x **yes** ").unwrap();
        assert!(!doc.blocks().is_empty());
        assert!(
            tail.starts_with("$$")
                || doc
                    .runs_in(Span::new(tail.len(), doc.source().len()))
                    .unwrap()
                    .any(|r| r.text == "yes" && r.style.contains(Style::STRONG))
        );
        let before = doc.work();
        for _ in 0..4096 {
            doc.append("x").unwrap();
        }
        assert_eq!(doc.work().classified_bytes, before.classified_bytes);
        assert_eq!(doc.work().container_steps, before.container_steps);
        assert_eq!(doc.work().inline_bytes - before.inline_bytes, 4096);
    }
}

#[test]
fn new_deep_paths_reach_all_leaf_routes_and_exact_existing_scopes() {
    let prefix = "> ".repeat(4097);
    for leaf in [
        "# h\n",
        "[r]: /u\n",
        "[^n]: body\n",
        "``` rs\n",
        "$$\n",
        "    code\n",
        "a|b\n",
    ] {
        let mut doc = Document::new();
        doc.append(&format!("{prefix}{leaf}")).unwrap();
        assert_eq!(doc.container_count(), 4097, "{leaf}");
        assert_eq!(doc.blocks()[0].container, Some(ContainerId(4096)));
        if leaf == "a|b\n" {
            doc.append(&format!("{prefix}{}|-\n{prefix}x|y\n", "-".repeat(4097)))
                .unwrap();
            assert_eq!(doc.tables().len(), 1);
            assert_eq!(
                doc.blocks()[doc.tables()[0].block.0].container,
                Some(ContainerId(4096))
            );
        }
        if leaf == "$$\n" || leaf == "``` rs\n" {
            let close = if leaf == "$$\n" { "$$" } else { "```" };
            doc.append(&format!(
                "{prefix}literal\n{prefix}{close}{}\n",
                " ".repeat(4097)
            ))
            .unwrap();
            assert_eq!(visible(&doc), "literal\n");
        }
        doc.finish();
    }
    let mut doc = Document::new();
    doc.append(&format!("{prefix}title\n")).unwrap();
    let first = doc.blocks()[0].clone();
    doc.append(&format!("{prefix}{}\n", "=".repeat(8193)))
        .unwrap();
    assert_eq!(doc.headings().entries().len(), 1);
    assert_eq!(doc.blocks()[0].container, first.container);
    assert_eq!(visible(&doc), "title\n");
}

#[test]
fn definition_grammar_bounds_survive_unbounded_prefix_and_trimmed_whitespace() {
    let prefix = "> ".repeat(600);
    for ending in ["", "\n", "\r\n"] {
        for gap in [" ".repeat(8193), "\t".repeat(4097)] {
            let source = format!("{prefix}[r]:{gap}/url 'title'{gap}{ending}");
            let doc = parse(&source);
            assert_eq!(doc.references().len(), 1);
            assert_eq!(
                doc.references()[0].destination.and_then(|s| doc.text(s)),
                Some("/url")
            );
            assert_eq!(visible(&doc), "");
        }
        let doc = parse(&format!(
            "{prefix}[{}]: /u{ending}",
            "a".repeat(MAX_LABEL_BYTES + 1)
        ));
        assert!(doc.references().iter().all(|r| r.destination.is_none()));
        let doc = parse(&format!(
            "{prefix}[r]: {}{ending}",
            "u".repeat(MAX_DESTINATION_BYTES + 1)
        ));
        assert!(doc.references().iter().all(|r| r.destination.is_none()));
    }
    for (start, next, expected) in [
        ("[r]:\n", "/u", Some("/u")),
        ("[r]: /u\n", "'title'", Some("/u")),
        ("[r\n", "last]: /u", Some("/u")),
        ("[r]: /u 'open\n", "close'", Some("/u")),
    ] {
        let padding = if start.contains("open") { 2000 } else { 9000 };
        let source = format!("{prefix}{start}{prefix}{next}{}\n", " ".repeat(padding));
        let doc = parse(&source);
        assert_eq!(
            doc.references()
                .iter()
                .find_map(|r| r.destination.and_then(|s| doc.text(s))),
            expected,
            "{start} {next}"
        );
    }
}

#[test]
fn every_utf8_byte_cut_and_every_prefix_eof_matches_physical_line_decisions() {
    let sources = [
        format!("{}# 中🙂\r\n", "> ".repeat(515)),
        format!("- {}x *中*\n", "* ".repeat(520)),
        format!("title\n{}\r\n", "=".repeat(1030)),
        format!("$${}x &amp;\n", " ".repeat(1030)),
        format!("{}[r]: /中 '🙂'\r\n", "> ".repeat(515)),
    ];
    for source in sources {
        let expected = snapshot::snapshot(&parse(&source));
        for cut in 0..=source.len() {
            let mut stream = Utf8Document::new();
            stream.append(&source.as_bytes()[..cut]).unwrap();
            stream.append(&source.as_bytes()[cut..]).unwrap();
            stream.finish();
            assert_eq!(snapshot::snapshot(stream.document()), expected, "cut={cut}");
        }
        let mut live = Document::new();
        for (at, ch) in source.char_indices() {
            live.append(&source[at..at + ch.len_utf8()]).unwrap();
            let prefix = &source[..at + ch.len_utf8()];
            let mut once = Document::new();
            once.append(prefix).unwrap();
            assert_eq!(
                snapshot::snapshot(&live),
                snapshot::snapshot(&once),
                "live {at}"
            );
            let mut finished = Utf8Document::new();
            for byte in prefix.as_bytes() {
                finished.append(&[*byte]).unwrap();
            }
            finished.finish();
            assert_eq!(
                snapshot::snapshot(finished.document()),
                snapshot::snapshot(&parse(prefix)),
                "EOF {at}"
            );
        }
    }
}

#[test]
fn million_scalar_appends_and_deep_lifecycle_have_actual_linear_work() {
    for unit in ["x", ">", "*", " ", "="] {
        let mut doc = Document::new();
        if unit == "=" {
            doc.append("title\n").unwrap();
        }
        let mut update = Update::default();
        for _ in 0..1_000_000 {
            doc.append_into(unit, &mut update).unwrap();
        }
        if unit == "x" {
            assert_eq!(doc.work().inline_bytes, 1_000_000);
            assert!(!doc.blocks().is_empty());
        } else {
            assert_eq!(doc.container_count(), 0);
        }
        doc.append("\n").unwrap();
        doc.finish();
        let n = doc.source().len() as u64;
        let w = doc.work();
        assert_eq!(w.input_bytes, n);
        assert!(w.structural_prefix_bytes < 3 * n);
        assert!(w.structural_prefix_steps < 7 * n);
        assert!(w.classified_bytes < 12 * n);
        assert!(w.container_bytes < 35 * n);
        assert!(w.container_steps < 6 * n);
        eprintln!("million {unit:?}: {w:?}");
        drop(doc);
    }
    let mut doc = Document::new();
    doc.append(&format!("{}head\n", "> ".repeat(8193))).unwrap();
    let before = doc.work();
    for _ in 0..4096 {
        doc.append("lazy\n").unwrap();
    }
    let after = doc.work();
    assert!(after.container_steps - before.container_steps < 4096 * 6);
    assert_eq!(doc.container_count(), 8193);
    doc.finish();
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(move || drop(doc))
        .unwrap()
        .join()
        .unwrap();
}

#[derive(Default)]
struct Sink(String);
impl LayoutSink for Sink {
    fn begin_tile(&mut self, _: TileId) {}
    fn text(&mut self, run: RenderRun<'_>) {
        if !run.style.contains(Style::HIDDEN) {
            self.0.push_str(run.text);
        }
    }
    fn object(&mut self, _: LinkId, _: &LinkInfo) {}
    fn end_tile(&mut self, _: TileId) -> u64 {
        1
    }
}

#[test]
fn provisional_mapping_dirty_identity_old_tickets_and_real_budgeted_layout() {
    for tail in [
        "* ".repeat(520),
        format!("$${}", " ".repeat(1040)),
        "> ".repeat(520),
    ] {
        let mut doc = Document::new();
        doc.append("old [a](u)\n\n").unwrap();
        let old = doc.blocks().to_vec();
        let old_link = *doc.link_info(LinkId(0)).unwrap();
        let start = doc.source().len();
        let mut view = FoldView::new(&doc, 1).unwrap();
        let update = doc.append(&tail).unwrap();
        view.apply(&doc, &update).unwrap();
        let mut heights = HeightIndex::default();
        heights.rebuild(&doc).unwrap();
        let tile = TileId(start / TILE_BYTES);
        let ticket = heights.ticket(tile).unwrap();
        let mut cursor = doc
            .display_cursor(Span::new(start, doc.source().len()))
            .unwrap();
        let mut stale = LayoutPass::new(&doc, &heights, tile, u64::MAX, false).unwrap();
        let update = doc.append("\r\n").unwrap();
        assert!(
            update
                .dirty
                .iter()
                .any(|s| s.start <= start && s.end >= start + tail.len())
        );
        view.apply(&doc, &update).unwrap();
        heights.apply(&doc, &update).unwrap();
        assert_eq!(
            heights.measure(ticket, 1),
            Err(LayoutError::StaleMeasurement)
        );
        assert_eq!(
            cursor.next(&doc, &mut Budget::new(WorkLimit::unlimited())),
            Err(LayoutError::StaleContinuation)
        );
        assert_eq!(
            stale.advance(
                &doc,
                &mut heights,
                &mut Budget::new(WorkLimit::unlimited()),
                &mut Sink::default()
            ),
            Err(LayoutError::StaleContinuation)
        );
        let update = doc.finish();
        view.apply(&doc, &update).unwrap();
        heights.apply(&doc, &update).unwrap();
        assert!(doc.blocks().starts_with(&old));
        assert_eq!(doc.link_info(LinkId(0)), Some(&old_link));
        let mut coverage = 0;
        for run in doc
            .display_runs_in(Span::new(0, doc.source().len()))
            .unwrap()
        {
            assert_eq!(run.source.start, coverage);
            coverage = run.source.end;
            if run.replacement.is_none() {
                assert_eq!(run.text, doc.text(run.source).unwrap());
            }
        }
        assert_eq!(coverage, doc.source().len());
        let expected = visible(&doc);
        let mut display = doc
            .display_cursor(Span::new(0, doc.source().len()))
            .unwrap();
        let mut output = String::new();
        while display.source_position() < doc.source().len() {
            for bytes in [0, 1] {
                let mut tiny = Budget::new(WorkLimit {
                    bytes,
                    queries: 1,
                    objects: 1,
                    ..Default::default()
                });
                let pos = display.source_position();
                assert_eq!(
                    display.next(&doc, &mut tiny).unwrap(),
                    DisplayStep::BudgetExhausted
                );
                assert_eq!(display.source_position(), pos);
            }
            let mut budget = Budget::new(WorkLimit {
                bytes: 6,
                queries: 6,
                objects: 6,
                ..Default::default()
            });
            assert!(budget.charge(WorkLimit {
                queries: 1,
                ..Default::default()
            }));
            match display.next(&doc, &mut budget).unwrap() {
                DisplayStep::Run(run) => {
                    if !run.style.contains(Style::HIDDEN) {
                        output.push_str(run.text)
                    }
                }
                DisplayStep::BudgetExhausted => (),
                other => panic!("{other:?}"),
            }
            assert!(budget.used().bytes <= 6);
        }
        assert_eq!(output, expected);
        let mut pass = LayoutPass::new(&doc, &heights, TileId(0), u64::MAX, false).unwrap();
        let mut sink = Sink::default();
        for _ in 0..doc.source().len() * 8 {
            let mut budget = Budget::new(WorkLimit {
                bytes: 6,
                queries: 6,
                objects: 6,
                tiles: 6,
                recovery: 6,
                dependencies: 6,
            });
            assert!(budget.charge(WorkLimit {
                queries: 1,
                ..Default::default()
            }));
            if pass
                .advance(&doc, &mut heights, &mut budget, &mut sink)
                .unwrap()
                == Completion::EndOfDocument
            {
                break;
            }
            assert!(budget.used().bytes <= 6);
        }
        assert_eq!(pass.source_position(), doc.source().len());
    }
}

#[test]
fn long_task_padding_fences_and_partial_tabs_preserve_real_source_identity() {
    let p = "> ".repeat(4097);
    for marker in ["```", "~~~"] {
        for prefix in [p.clone(), format!("{p}- >\t"), format!("{p}- ")] {
            let continuation = if prefix.ends_with("- ") {
                format!("{p}  ")
            } else if prefix.ends_with("- >\t") {
                format!("{p}  >\t")
            } else {
                prefix.clone()
            };
            let source =
                format!("{prefix}{marker}rust\n{continuation}code\n{continuation}{marker}\n");
            let doc = parse(&source);
            assert!(matches!(
                doc.blocks()[0].kind,
                BlockKind::Code { info: Some(_) }
            ));
            if let BlockKind::Code { info: Some(info) } = doc.blocks()[0].kind {
                assert_eq!(doc.text(info), Some("rust"));
            }
            assert_eq!(visible(&doc), "code\n");
            assert!(doc.container_count() >= 4097);
            let mut stream = Utf8Document::new();
            for b in source.bytes() {
                stream.append(&[b]).unwrap();
            }
            stream.finish();
            assert_eq!(
                snapshot::snapshot(stream.document()),
                snapshot::snapshot(&doc)
            );
        }
    }
    let source = format!("{p}- [x]{}中🙂\n", " \t".repeat(4097));
    let doc = parse(&source);
    assert_eq!(visible(&doc), "中🙂\n");
    assert!(matches!(
        doc.blocks()[0].kind,
        BlockKind::ListItem { task: Some(_), .. }
    ));
    let task = match doc
        .container(ContainerId(doc.container_count() - 1))
        .unwrap()
        .kind
    {
        ContainerKind::Item {
            task: Some(task), ..
        } => task,
        _ => panic!("task"),
    };
    assert_eq!(doc.text(task.source), Some("[x]"));
}

#[test]
fn large_history_never_revisits_old_objects_during_ordinary_growth() {
    let mut doc = Document::new();
    for i in 0..20_000 {
        doc.append(&format!("[r{i}]: /u\n")).unwrap();
    }
    doc.append("\n").unwrap();
    let old = doc.references()[0].clone();
    let old_blocks = doc.blocks().len();
    let prefix = "> ".repeat(4097);
    doc.append(&format!("{prefix}body ")).unwrap();
    let before = doc.work();
    for _ in 0..1_000_000 {
        doc.append("x").unwrap();
    }
    let after = doc.work();
    assert_eq!(after.inline_bytes - before.inline_bytes, 1_000_000);
    assert_eq!(after.classified_bytes, before.classified_bytes);
    assert_eq!(after.container_steps, before.container_steps);
    assert_eq!(after.label_bytes, before.label_bytes);
    assert_eq!(doc.references()[0], old);
    assert_eq!(doc.blocks().len(), old_blocks + 1);
    doc.finish();
}

#[test]
fn excluded_root_structure_and_all_definition_title_delimiters_stream_correctly() {
    for (seed, head, inline) in [("", "    ", false), ("text\n", "14. ", true)] {
        let mut doc = Document::new();
        doc.append(seed).unwrap();
        doc.append(&format!("{head}{}", " ".repeat(8192))).unwrap();
        if !inline {
            doc.append("*").unwrap();
        }
        let before = doc.work();
        for _ in 0..100_000 {
            doc.append(if inline { " " } else { "*" }).unwrap();
        }
        assert!(!doc.blocks().is_empty());
        assert_eq!(doc.work().classified_bytes, before.classified_bytes);
        if inline {
            assert_eq!(doc.work().inline_bytes - before.inline_bytes, 100_000);
        } else {
            assert_eq!(doc.work().inline_bytes, 0);
        }
    }
    let p = "> ".repeat(4097);
    for title in ["'title'", "\"title\"", "(title)"] {
        for ending in ["", "\n", "\r\n"] {
            let source = format!("{p}[r]: /u {title}{}{ending}", " \t".repeat(8193));
            let doc = parse(&source);
            assert_eq!(
                doc.references()[0].destination.and_then(|s| doc.text(s)),
                Some("/u")
            );
            assert_eq!(
                doc.references()[0].title.and_then(|s| doc.text(s)),
                Some("title")
            );
            assert_eq!(visible(&doc), "");
        }
    }
}

#[test]
fn each_prefix_eof_has_an_independent_structural_shape() {
    for source in [
        format!("{}- * * *\t\n", ">".repeat(1030)),
        format!("{}>\t# heading\r\n", ">".repeat(1030)),
        format!("{}1. + text\n", "> ".repeat(520)),
        format!("{}$${}\r\n", "> ".repeat(10), " ".repeat(1030)),
    ] {
        for end in 1..=source.len() {
            let prefix = &source[..end];
            assert_eq!(
                production_shape(&parse(prefix)),
                model(prefix),
                "EOF end={end}"
            );
        }
    }
}
