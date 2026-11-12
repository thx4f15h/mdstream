//! Remaining container/tab semantics. Expected paths and columns are computed
//! without Document; core output is never used to build the expected model.
use mdstream::*;

#[path = "support/snapshot.rs"]
mod support;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn visible(doc: &Document) -> String {
    doc.display_runs_in(Span::new(0, doc.source().len()))
        .unwrap()
        .filter(|run| !run.style.contains(Style::HIDDEN))
        .map(|run| run.text)
        .collect()
}

// A deliberately different representation: expand only structural whitespace
// into individual column cells carrying the index of their original byte.
// Taking cells cannot consume or reinterpret any payload byte.
fn cells(ws: &str, start: usize) -> Vec<usize> {
    let mut out = Vec::new();
    for (index, byte) in ws.bytes().enumerate() {
        let width = if byte == b'\t' {
            4 - (start + out.len()) % 4
        } else {
            1
        };
        out.extend(std::iter::repeat_n(index, width));
    }
    out
}

fn tail(ws: &str, cells: &[usize], consumed: usize) -> String {
    if consumed == 0 {
        return ws.to_owned();
    }
    let byte = cells[consumed - 1];
    let residue = cells[consumed..].iter().take_while(|&&i| i == byte).count();
    format!("{}{}", " ".repeat(residue), &ws[byte + 1..])
}

fn whitespace() -> Vec<String> {
    let mut out = vec![String::new()];
    for n in 1..=6 {
        for mask in 0..1 << n {
            out.push(
                (0..n)
                    .map(|i| if mask & (1 << i) == 0 { ' ' } else { '\t' })
                    .collect(),
            );
        }
    }
    out
}

#[test]
fn incomplete_paths_match_independent_column_cells_and_close_actual_suffix() {
    let mut count = 0;
    for quotes in 0..=3 {
        let outer = "> ".repeat(quotes);
        for markers in [
            vec!["- "],
            vec!["-  "],
            vec!["1. "],
            vec!["- ", "- "],
            vec!["-  ", "12. "],
            vec!["1. ", "- ", "- "],
        ] {
            let first = format!("{outer}{}> # seed\n", markers.concat());
            for ws in whitespace() {
                let columns = cells(&ws, outer.len());
                let mut consumed = 0;
                let mut retained = quotes.checked_sub(1).map(ContainerId);
                for (i, marker) in markers.iter().enumerate() {
                    if columns.len() - consumed < marker.len() {
                        break;
                    }
                    consumed += marker.len();
                    retained = Some(ContainerId(quotes + 2 * i + 1));
                }
                let code = columns.len() - consumed >= 4;
                let expected = if code {
                    format!("seed\n{}# leaf\n", tail(&ws, &columns, consumed + 4))
                } else {
                    "seed\nleaf\n".to_owned()
                };
                let source = format!("{first}{outer}{ws}# leaf\n");
                let doc = parse(&source);
                let last = doc.blocks().last().unwrap();
                assert_eq!(last.container, retained, "{source:?}");
                assert_eq!(
                    last.kind,
                    if code {
                        BlockKind::Code { info: None }
                    } else {
                        BlockKind::Heading { level: 1 }
                    },
                    "{source:?}"
                );
                assert_eq!(visible(&doc), expected, "{source:?}");
                assert_eq!(doc.container_count(), quotes + markers.len() * 2 + 1);
                count += 1;
            }
        }
    }
    assert_eq!(count, 3048);
}

#[test]
fn leaf_code_uses_four_columns_in_new_and_continuing_scopes() {
    let mut count = 0;
    for outer in ["", "> ", "> > ", "- ", "1. ", "> - "] {
        for ws in whitespace() {
            let columns = cells(&ws, outer.len());
            if columns.len() < 4 {
                continue;
            }
            for previous in ["# seed", "*seed", "a|b"] {
                // List first-line padding is separately specified (1..4
                // separator columns); test its body after a seed physical line.
                let continuation = if outer.ends_with("- ") || outer == "1. " {
                    outer.replace("- ", "  ").replace("1. ", "   ")
                } else {
                    outer.to_owned()
                };
                let source = format!("{outer}{previous}\n{continuation}{ws}*raw*\tend\n");
                let doc = parse(&source);
                let expected_prefix = if previous == "# seed" {
                    "seed"
                } else if previous == "*seed" {
                    "*seed"
                } else {
                    "a|b"
                };
                let expected = format!("{expected_prefix}\n{}*raw*\tend\n", tail(&ws, &columns, 4));
                assert_eq!(visible(&doc), expected, "{source:?}");
                assert_eq!(
                    doc.blocks().last().unwrap().kind,
                    BlockKind::Code { info: None },
                    "{source:?}"
                );
                assert_eq!(
                    doc.blocks().last().unwrap().container,
                    doc.blocks()[0].container,
                    "{source:?}"
                );
                count += 1;
            }
        }
    }
    assert!(count > 1800);
    eprintln!("{count} independent leaf-column models");
}

#[test]
fn missing_prefix_ends_fences_and_paragraph_state_before_reentry() {
    for (source, expected) in [
        ("- > ```\n\t# cut\n\t> after\n", "cut\nafter\n"),
        ("- > $$\n\t# cut\n\t> after\n", "cut\nafter\n"),
        ("- > # seed\n\ttext\n\t> again\n", "seed\n  text\nagain\n"),
        ("- > *seed\n\t# cut\n\t> end*\n", "*seed\ncut\nend*\n"),
        ("- > $seed\n\t# cut\n\t> end$\n", "$seed\ncut\nend$\n"),
        ("- > [seed\n\t# cut\n\t> end](u)\n", "[seed\ncut\nend](u)\n"),
    ] {
        let doc = parse(source);
        assert_eq!(visible(&doc), expected, "{source:?}");
        let first = doc.blocks()[0].container.unwrap();
        let last = doc.blocks().last().unwrap().container.unwrap();
        assert_ne!(first, last, "closed quotes must not reopen: {source:?}");
        assert_eq!(
            doc.container(first).unwrap().parent,
            doc.container(last).unwrap().parent
        );
        assert!(doc.maths().is_empty());
        assert!(doc.links().is_empty());
        assert!(
            doc.display_runs_in(Span::new(0, source.len()))
                .unwrap()
                .all(|r| !r.style.contains(Style::EMPHASIS))
        );
    }
    let mut doc = Document::new();
    doc.append("- > *old\n").unwrap();
    let original = doc.blocks()[0].clone();
    let quote = original.container.unwrap();
    let item = doc.container(quote).unwrap().parent.unwrap();
    let update = doc.append("\t# cut\n").unwrap();
    assert_eq!(update.closed_containers, [quote]);
    assert_eq!(update.new_containers.len(), 0);
    assert_eq!(doc.blocks()[0], original);
    assert_eq!(doc.blocks()[1].container, Some(item));
    let update = doc.append("\t> new*\n").unwrap();
    assert_eq!(update.new_containers.len(), 1);
    assert_ne!(doc.blocks()[2].container, Some(quote));
}

#[test]
fn incomplete_path_header_definition_and_footnote_candidates_consume_once() {
    for (suffix, inline) in [
        ("\ttext\n", 4 + 2 + 5),
        ("\t# cut\n", 4 + 4),
        ("\t\tcode\n", 4 + 2 + "\tcode\n".len()),
        ("\t\n", 4),
        ("\t[ref]: /u\n", 4 + 2 + "[ref]: /u\n".len()),
        ("\t[^f]: body\n", 4),
        ("\t$$\n\tmath\n\t$$\n", 4),
    ] {
        let source = format!("- > a|b\n{suffix}");
        let doc = parse(&source);
        assert!(doc.tables().is_empty(), "{source:?}");
        assert_eq!(doc.source(), source);
        assert_eq!(
            doc.work().inline_bytes,
            inline as u64,
            "no duplicate raw/virtual input: {source:?}"
        );
    }
    let doc = parse("- > # seed\n\ta|b\n\t-|-\n\t[x]: /u\n");
    assert_eq!(doc.tables().len(), 1);
    assert_eq!(
        doc.blocks()[1].container,
        doc.container(doc.blocks()[0].container.unwrap())
            .unwrap()
            .parent
    );
}

#[test]
fn all_utf8_splits_byte_splits_and_live_prefixes_preserve_complete_state() {
    for source in [
        "- > *中🙂\n\t# cut\n\t> end*\n",
        "> - > # 中\r\n>\tplain\n>\t> again\n",
        "- - > ```\n\t# cut\n\t+ sibling\n",
        "- > a|b\n\t# cut\n\t> c|d\n\t> -|-\n",
        "- > # seed\n\t[ref]: /u\n\t[^f]: body\n\t\tmore\n",
        "> *seed\n> \t\t*raw*\t中\n",
    ] {
        let expected = support::snapshot(&parse(source));
        for split in 0..=source.len() {
            let mut doc = Utf8Document::new();
            doc.append(&source.as_bytes()[..split]).unwrap();
            doc.append(&source.as_bytes()[split..]).unwrap();
            doc.finish();
            assert_eq!(
                support::snapshot(doc.document()),
                expected,
                "byte split {split}: {source:?}"
            );
        }
        let mut doc = Document::new();
        for (at, ch) in source.char_indices() {
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            let mut whole = Document::new();
            whole.append(&source[..at + ch.len_utf8()]).unwrap();
            assert_eq!(doc.pending(), whole.pending());
            assert_eq!(support::snapshot(&doc), support::snapshot(&whole));
        }
        doc.finish();
        assert_eq!(support::snapshot(&doc), expected);
    }
}

#[test]
fn new_paragraphs_and_quote_first_lines_keep_residual_mapping_at_failed_candidates() {
    for outer in ["", "> ", "> > "] {
        for marker in ["- ", "-  ", "1. ", "- - "] {
            for ws in whitespace() {
                let columns = cells(&ws, outer.len());
                let targets: Vec<_> = if marker == "- - " {
                    vec![2, 2]
                } else {
                    vec![marker.len()]
                };
                let mut consumed = 0;
                let mut retained = outer
                    .len()
                    .checked_div(2)
                    .unwrap()
                    .checked_sub(1)
                    .map(ContainerId);
                for (i, width) in targets.iter().enumerate() {
                    if columns.len() - consumed < *width {
                        break;
                    }
                    consumed += *width;
                    retained = Some(ContainerId(outer.len() / 2 + 2 * i + 1));
                }
                let code = columns.len() - consumed >= 4;
                let source = format!("{outer}{marker}> # seed\n{outer}{ws}plain\tend\n");
                let doc = parse(&source);
                let expected_tail = if code {
                    tail(&ws, &columns, consumed + 4)
                } else if retained.is_none() {
                    String::new() // The documented root paragraph 0..3 space trim.
                } else {
                    tail(&ws, &columns, consumed)
                };
                assert_eq!(
                    visible(&doc),
                    format!("seed\n{expected_tail}plain\tend\n"),
                    "{source:?}"
                );
                assert_eq!(
                    doc.blocks().last().unwrap().container,
                    retained,
                    "{source:?}"
                );
            }
        }
    }
    // Invalid reference labels fall back to prose. Quote separator columns
    // are still structural; the failed candidate must not regain a raw tab.
    for label in [" ".to_owned(), "x".repeat(1000)] {
        let source = format!(">\t[{label}]: /u\n");
        let doc = parse(&source);
        assert_eq!(visible(&doc), format!("  [{label}]: /u\n"));
        assert_eq!(doc.container_count(), 1);
        assert!(doc.references().is_empty());
    }
}

#[test]
fn heterogeneous_empty_and_misaligned_siblings_use_paths_and_paragraph_eligibility() {
    for ancestor in ["- ", "-  ", "1. "] {
        for ws in whitespace() {
            let columns = cells(&ws, 0);
            // A two-column old child marker followed by one separator gives
            // a body boundary at ancestor+3. Different sibling indentation
            // must stay below that boundary and at most three columns.
            if columns.len() < ancestor.len() || columns.len() > ancestor.len() + 2 {
                continue;
            }
            let indent = columns.len() - ancestor.len();
            for old in ["-", "+", "1.", "2)"] {
                for new in ["-", "+", "*", "7.", "3)"] {
                    for blank in [false, true] {
                        for content in [" body", ""] {
                            let first = format!(
                                "{ancestor}# parent\n{} {old} child\n",
                                " ".repeat(ancestor.len())
                            );
                            let separator = if blank { "\n" } else { "" };
                            let source = format!("{first}{separator}{ws}{new}{content}\n");
                            let doc = parse(&source);
                            let same = old == new
                                || old.ends_with('.') && new.ends_with('.')
                                || old.ends_with(')') && new.ends_with(')');
                            // Old marker's extra indentation was one column.
                            // A different empty marker cannot interrupt prose;
                            // any other candidate creates a sibling/list.
                            let lazy = !same && !blank && content.is_empty();
                            let inner = doc.blocks().last().unwrap().container.unwrap();
                            if lazy {
                                assert_eq!(inner, doc.blocks()[1].container.unwrap(), "{source:?}");
                            } else {
                                assert_ne!(inner, doc.blocks()[1].container.unwrap(), "{source:?}");
                                let list = doc.container(inner).unwrap().parent.unwrap();
                                assert_eq!(
                                    doc.container(list).unwrap().parent,
                                    doc.blocks()[0].container,
                                    "{source:?}"
                                );
                            }
                            assert!(doc.work().container_steps <= 40 * source.len() as u64);
                            assert!(indent <= 2);
                        }
                    }
                }
            }
        }
    }
}

#[derive(Default)]
struct RawSink(String);
impl LayoutSink for RawSink {
    fn begin_tile(&mut self, _: TileId) {}
    fn text(&mut self, run: RenderRun<'_>) {
        if !run.style.contains(Style::HIDDEN) {
            self.0.push_str(run.text);
        }
    }
    fn object(&mut self, _: LinkId, _: &LinkInfo) {
        panic!("no link objects")
    }
    fn end_tile(&mut self, _: TileId) -> u64 {
        1
    }
}

#[test]
fn source_virtual_display_mapping_and_shared_budget_layout_staleness() {
    let mut doc = Document::new();
    let mut heights = HeightIndex::default();
    let update = doc.append("- > # seed\n").unwrap();
    heights.apply(&doc, &update).unwrap();
    let source = "\t\t中🙂\tend\n";
    let update = doc.append(source).unwrap();
    heights.apply(&doc, &update).unwrap();
    let update = doc.finish();
    heights.apply(&doc, &update).unwrap();
    let expected = "seed\n  中🙂\tend\n";
    assert_eq!(visible(&doc), expected);
    let tab = doc.source().find("\t\t").unwrap() + 1;
    let mapped: Vec<_> = doc
        .display_runs_in(Span::new(tab, tab + 1))
        .unwrap()
        .collect();
    assert_eq!(mapped.len(), 1);
    assert_eq!(mapped[0].text, "  ");
    assert_eq!(mapped[0].replacement, Some(Span::new(tab, tab + 1)));
    assert_eq!(doc.text(mapped[0].source), Some("\t"));
    let tiled: String = (0..doc.tile_count())
        .flat_map(|i| doc.display_runs(TileId(i)).unwrap())
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    assert_eq!(tiled, expected);
    let mut cursor = doc
        .display_cursor(Span::new(0, doc.source().len()))
        .unwrap();
    let mut out = String::new();
    loop {
        let pos = cursor.source_position();
        for bytes in [0, 1].into_iter().filter(|_| pos < doc.source().len()) {
            let mut budget = Budget::new(WorkLimit {
                bytes,
                ..WorkLimit::default()
            });
            assert_eq!(
                cursor.next(&doc, &mut budget).unwrap(),
                DisplayStep::BudgetExhausted
            );
            assert_eq!(cursor.source_position(), pos);
        }
        let mut budget = Budget::new(WorkLimit {
            bytes: 6,
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        assert!(budget.charge(WorkLimit {
            bytes: 1,
            ..Default::default()
        }));
        match cursor.next(&doc, &mut budget).unwrap() {
            DisplayStep::Run(run) => {
                if !run.style.contains(Style::HIDDEN) {
                    out.push_str(run.text);
                }
            }
            DisplayStep::Complete => break,
            DisplayStep::BudgetExhausted => {
                assert_eq!(cursor.source_position(), pos);
                let mut fresh = Budget::new(WorkLimit {
                    bytes: 6,
                    queries: 1,
                    objects: 1,
                    ..Default::default()
                });
                let DisplayStep::Run(run) = cursor.next(&doc, &mut fresh).unwrap() else {
                    panic!("fresh frame must advance")
                };
                if !run.style.contains(Style::HIDDEN) {
                    out.push_str(run.text);
                }
            }
        }
        assert!(budget.used().bytes <= 6);
    }
    assert_eq!(out, expected);
    let mut pass = LayoutPass::new(&doc, &heights, TileId(0), u64::MAX, false).unwrap();
    let mut sink = RawSink::default();
    loop {
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
        match pass
            .advance(&doc, &mut heights, &mut budget, &mut sink)
            .unwrap()
        {
            Completion::EndOfDocument => break,
            Completion::BudgetExhausted => (),
            other => panic!("unexpected {other:?}"),
        }
        assert!(budget.used().bytes <= 6);
    }
    assert_eq!(sink.0, "seed\n\t中🙂\tend\n"); // LayoutPass carries original RenderRun bytes.
    let mut live = Document::new();
    let mut heights = HeightIndex::default();
    let update = live.append("- > *old\n").unwrap();
    heights.apply(&live, &update).unwrap();
    let ticket = heights.ticket(TileId(0)).unwrap();
    let mut stale = live
        .display_cursor(Span::new(0, live.source().len()))
        .unwrap();
    let mut stale_pass = LayoutPass::new(&live, &heights, TileId(0), u64::MAX, false).unwrap();
    let update = live.append("\t# cut\n\t> end*\n").unwrap();
    assert!(!update.dirty.is_empty());
    heights.apply(&live, &update).unwrap();
    assert_eq!(
        stale.next(&live, &mut Budget::new(WorkLimit::unlimited())),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(
        stale_pass.advance(
            &live,
            &mut heights,
            &mut Budget::new(WorkLimit::unlimited()),
            &mut RawSink::default()
        ),
        Err(LayoutError::StaleContinuation)
    );
    // The old unmatched opener stays literal. Its tile can retain an unaffected
    // measurement when the interruption lies outside that tile's dependency.
    assert_eq!(
        heights.measure(ticket, 1),
        Err(LayoutError::StaleMeasurement)
    );
}

#[test]
fn deep_paths_million_byte_growth_lazy_blanks_and_siblings_count_actual_work() {
    // The last outer quote owns one separator column of this two-column tab.
    // One column cannot also satisfy the following two-column Item. The old
    // byte-aligned retry used to silently reuse List/Item/Quote here.
    let outer = ">".repeat(970);
    let mut split = Document::new();
    split.append(&format!("{outer}- > # seed\n")).unwrap();
    let update = split.append(&format!("{outer}\t> \n")).unwrap();
    assert_eq!(
        update.closed_containers,
        [ContainerId(972), ContainerId(971), ContainerId(970)]
    );
    assert_eq!(update.new_containers, 973..974);
    assert_eq!(
        split.container(ContainerId(973)).unwrap().parent,
        Some(ContainerId(969))
    );
    assert_eq!(split.blocks()[1].kind, BlockKind::Blank);
    assert_eq!(visible(&split), "seed\n\n");

    let prefix = ">".repeat(997);
    let mut doc = Document::new();
    doc.append(&format!("{prefix}- > # seed\n")).unwrap();
    assert_eq!(doc.container_count(), 1000);
    let update = doc.append(&format!("{prefix}\t# cut\n")).unwrap();
    assert_eq!(update.closed_containers.len(), 1);
    doc.append(&format!("{prefix}\t{}", "x".repeat(1_000_000)))
        .unwrap();
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("x").unwrap();
    }
    let after = doc.work();
    assert_eq!(after.inline_bytes - before.inline_bytes, 1000);
    assert_eq!(after.classified_bytes, before.classified_bytes);
    assert_eq!(after.container_bytes, before.container_bytes);
    assert_eq!(after.container_steps, before.container_steps);
    doc.append("\n").unwrap();
    let before = doc.work();
    for _ in 0..4096 {
        doc.append("lazy\n").unwrap();
    }
    let after = doc.work();
    assert!(after.container_steps - before.container_steps < 8 * 4096);
    assert!(after.container_bytes - before.container_bytes < 64 * 4096);
    assert_eq!(
        doc.blocks().last().unwrap().container,
        Some(ContainerId(998))
    );
    let update = doc.append("\n").unwrap();
    assert_eq!(update.closed_containers.len(), 999);
    let before = doc.work();
    for _ in 0..4096 {
        doc.append("\n").unwrap();
    }
    assert!(doc.work().container_steps - before.container_steps < 8 * 4096);
    doc.finish();
    assert!(doc.work().container_bytes < 32 * doc.source().len() as u64);
    assert!(doc.work().container_steps < 16 * doc.source().len() as u64);
    eprintln!("million/deep/lazy counters: {:?}", doc.work());

    let mut doc = Document::new();
    doc.append(&format!("{}a\n", "- ".repeat(500))).unwrap();
    let before = doc.work();
    for _ in 0..4096 {
        doc.append("\n").unwrap();
    }
    assert!(doc.work().container_steps - before.container_steps < 8 * 4096);
    let retained = "  ".repeat(498);
    for i in 0..512 {
        doc.append(&format!(
            "{retained}\t{} body\n",
            if i % 2 == 0 { "+" } else { "-" }
        ))
        .unwrap();
    }
    doc.finish();
    assert!(doc.work().container_bytes < 32 * doc.source().len() as u64);
    assert!(doc.work().container_steps < 16 * doc.source().len() as u64);
    assert!(doc.work().list_steps < 8 * doc.source().len() as u64);
    eprintln!("deep counters: {:?}", doc.work());
}

#[test]
fn mixed_quote_item_paths_match_independent_expanded_cell_interpreter() {
    let mut checked = 0;
    for depth in 1..=6 {
        for pattern in 0..1 << depth {
            let quotes: Vec<_> = (0..depth).map(|i| pattern & (1 << i) != 0).collect();
            let opening: String = quotes
                .iter()
                .map(|&q| if q { "> " } else { "- " })
                .collect();
            for provided in 0..=depth {
                for spelling in [" ", "  ", "\t", " \t", "\t ", "\t\t"] {
                    let mut raw = String::new();
                    for &quote in &quotes[..provided] {
                        if quote {
                            raw.push('>');
                        }
                        raw.push_str(spelling);
                    }
                    raw.push_str(spelling);
                    // Expand the bounded structural input into cells. Quote
                    // markers are ordinary cells; only tab cells share an owner.
                    let mut input = Vec::new();
                    for (byte, ch) in raw.bytes().enumerate() {
                        let width = if ch == b'\t' { 4 - input.len() % 4 } else { 1 };
                        input.extend(std::iter::repeat_n(
                            (if ch == b'\t' { b' ' } else { ch }, byte),
                            width,
                        ));
                    }
                    let mut at = 0;
                    let mut retained = None;
                    let mut id = 0;
                    for &quote in &quotes {
                        let start = at;
                        if quote {
                            let spaces = input[at..]
                                .iter()
                                .take_while(|&&(ch, _)| ch == b' ')
                                .count();
                            if spaces > 3 || input.get(at + spaces).map(|&(ch, _)| ch) != Some(b'>')
                            {
                                break;
                            }
                            at += spaces + 1;
                            if input.get(at).is_some_and(|&(ch, _)| ch == b' ') {
                                at += 1;
                            }
                            id += 1;
                        } else {
                            if input[at..]
                                .iter()
                                .take(2)
                                .filter(|&&(ch, _)| ch == b' ')
                                .count()
                                != 2
                            {
                                break;
                            }
                            at += 2;
                            id += 2;
                        }
                        assert!(at > start);
                        retained = Some(ContainerId(id - 1));
                    }
                    // Unconsumed explicit quote tokens create new containers;
                    // they are tested separately. Here assert closure/retention
                    // without deriving any expected path from Document.
                    if input[at..].iter().any(|&(ch, _)| ch != b' ') {
                        continue;
                    }
                    let code = input.len() - at >= 4;
                    let cut = at + if code { 4 } else { 0 };
                    let padding = if code {
                        let owner = input[cut - 1].1;
                        let residue = input[cut..]
                            .iter()
                            .take_while(|&&(_, byte)| byte == owner)
                            .count();
                        format!("{}{}", " ".repeat(residue), &raw[owner + 1..])
                    } else {
                        String::new()
                    };
                    let source = format!("{opening}# seed\n{raw}# leaf\n");
                    let doc = parse(&source);
                    assert_eq!(
                        doc.blocks().last().unwrap().container,
                        retained,
                        "{source:?}"
                    );
                    assert_eq!(
                        visible(&doc),
                        format!("seed\n{padding}{}\n", if code { "# leaf" } else { "leaf" }),
                        "{source:?}"
                    );
                    assert_eq!(
                        doc.container_count(),
                        quotes.iter().map(|&q| if q { 1 } else { 2 }).sum::<usize>(),
                        "no reopened IDs: {source:?}"
                    );
                    checked += 1;
                }
            }
        }
    }
    assert!(checked > 1500);
    eprintln!("{checked} independent mixed-path models");
}
