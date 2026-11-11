use mdstream::{BlockId, BlockKind, Document, Span, SyntaxState};

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

#[test]
fn pending_multiline_label_never_publishes_ghost_objects() {
    for (first, second, target, containers) in [
        ("[Foo\n", " bar]:\n", "/url\n", 0),
        ("> [Foo\n", ">   bar]:\n", "> /url\n", 1),
        ("- > [Foo\n", "  >   bar]:\n", "  > /url\n", 3),
        (">\t[Foo\n", ">\tbar]:\n", ">\t/url\n", 1),
    ] {
        let mut doc = Document::new();
        doc.append(first).unwrap();
        let block = doc.blocks()[0].clone();
        doc.append(second).unwrap();
        assert!(doc.links().is_empty(), "{first:?}{second:?}");
        assert!(doc.headings().entries().is_empty());
        assert_eq!(doc.container_count(), containers);
        assert_eq!(doc.blocks().len(), 1);
        assert_eq!(doc.blocks()[0].container, block.container);
        assert_eq!(doc.blocks()[0].state, SyntaxState::Growing);
        assert_eq!(doc.source_fold_limit(), 0);
        let update = doc.append(target).unwrap();
        assert!(update.new_blocks.is_empty());
        assert!(update.new_containers.is_empty());
        assert!(update.new_links.is_empty());
        assert!(matches!(
            doc.block(BlockId(0)).unwrap().kind,
            BlockKind::Definition { .. }
        ));
        assert_eq!(doc.blocks()[0].source, Span::new(0, doc.source().len()));
        doc.finish();
        assert_eq!(doc.blocks()[0].state, SyntaxState::Sealed);
    }
}

#[path = "support/link_metadata.rs"]
mod metadata;
#[path = "support/snapshot.rs"]
mod support;
use mdstream::{
    Budget, ChangeKind, DisplayStep, FoldView, LinkId, SourceFoldRegistration, Style, SymbolId,
    TileId, Update, Utf8Document, WorkLimit,
};

const PATHS: &[(&str, &str)] = &[
    ("", ""),
    ("> ", "> "),
    ("- ", "  "),
    ("> - ", ">   "),
    ("- > ", "  > "),
    ("- - ", "    "),
    (">\t", ">\t"),
    ("- - ", "\t"),
    ("- > ", "\t>\t"),
    ("> > ", ">\t>\t"),
];

fn defined(doc: &Document) -> usize {
    doc.references()
        .iter()
        .filter(|r| r.destination.is_some())
        .count()
}

fn chunk_equivalence(source: &str) {
    let whole = parse(source);
    let expected = support::snapshot(&whole);
    for split in (0..=source.len()).filter(|&i| source.is_char_boundary(i)) {
        let mut doc = Document::new();
        doc.append(&source[..split]).unwrap();
        doc.append(&source[split..]).unwrap();
        doc.finish();
        assert_eq!(
            support::snapshot(&doc),
            expected,
            "split {split}: {source:?}"
        );
    }
    let mut doc = Document::new();
    for (at, ch) in source.char_indices() {
        doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
        let prefix = &source[..at + ch.len_utf8()];
        let mut bulk = Document::new();
        bulk.append(prefix).unwrap();
        assert_eq!(
            support::snapshot(&doc),
            support::snapshot(&bulk),
            "prefix {prefix:?}"
        );
        bulk.finish();
        assert_eq!(support::snapshot(&bulk), support::snapshot(&parse(prefix)));
        let mut eof = Document::new();
        eof.append(&source[..at]).unwrap();
        eof.append(&source[at..at + ch.len_utf8()]).unwrap();
        eof.finish();
        assert_eq!(support::snapshot(&eof), support::snapshot(&bulk));
    }
    doc.finish();
    assert_eq!(support::snapshot(&doc), expected);
    for split in 0..=source.len() {
        let mut utf8 = Utf8Document::new();
        utf8.append(&source.as_bytes()[..split]).unwrap();
        utf8.append(&source.as_bytes()[split..]).unwrap();
        utf8.finish();
        assert_eq!(support::snapshot(utf8.document()), expected);
    }
}

#[test]
fn multiline_paths_targets_titles_raw_spans_and_first_winner() {
    for &(first, next) in PATHS {
        for eol in ["\n", "\r\n"] {
            for target_next in [false, true] {
                for title_next in [false, true] {
                    let mut source = format!("{first}[Straße{eol}{next} 中\\[\\]\\\\]:");
                    if target_next {
                        source.push_str(&format!("{eol}{next}    "));
                    }
                    source.push_str(" <a&amp;中%20> ");
                    if title_next {
                        source.push_str(&format!("{eol}{next}    "));
                    }
                    source.push_str(&format!("'t&amp;🙂\\*'{eol}"));
                    let end = source.len();
                    source.push_str(&format!(
                        "{next}[STRASSE 中\\[\\]\\\\]: /ignored{eol}{next}'ignored'{eol}"
                    ));
                    source.push_str("\n[x][strasse 中\\[\\]\\\\]\n");
                    let doc = parse(&source);
                    assert_eq!(defined(&doc), 1, "{source:?}");
                    assert_eq!(doc.links().len(), 1);
                    assert_eq!(doc.link_destination(LinkId(0)), Some("a&amp;中%20"));
                    assert_eq!(doc.link_title(LinkId(0)), Some("t&amp;🙂\\*"));
                    assert_eq!(doc.blocks()[0].source, Span::new(0, end));
                    assert!(matches!(doc.blocks()[0].kind, BlockKind::Definition { .. }));
                    for at in (0..end).filter(|&at| source.is_char_boundary(at)) {
                        assert_eq!(doc.block_at(at), Some(BlockId(0)));
                    }
                    let reference = &doc.references()[0];
                    assert_eq!(reference.label, "strasse 中\\[\\]\\\\");
                    assert_eq!(
                        doc.text(reference.destination.unwrap()),
                        doc.link_destination(LinkId(0))
                    );
                    assert_eq!(
                        doc.text(reference.title.unwrap()),
                        doc.link_title(LinkId(0))
                    );
                    assert_eq!(reference.revision, if title_next { 2 } else { 1 });
                    assert!(
                        doc.display_runs_in(Span::new(0, end))
                            .unwrap()
                            .all(|r| r.style.contains(Style::HIDDEN))
                    );
                    metadata::snapshot(&doc);
                }
            }
        }
    }
}

#[test]
fn multiline_every_utf8_split_prefix_eof_and_crlf() {
    for &(first, next) in PATHS {
        for eol in ["\n", "\r\n"] {
            for tail in [
                "b]: <中🙂> 't&amp;'",
                "b]:",
                "b] : /bad",
                "b[bad]: /bad",
                "b]: <bad",
                "b]: /url 'bad",
                "b",
                "\\]b]: /url",
                "\\\\]: /url",
            ] {
                let source = format!("{first}[É{eol}{next}{tail}{eol}");
                chunk_equivalence(&source);
            }
        }
    }
    for source in [
        "[\nfoo\n]: /url\nbar\n",
        "[a\\\r\nb]:\r\n/url\r\n 't'\r\n",
        "seed\n[a\nb]: /url\n",
    ] {
        chunk_equivalence(source);
    }
}

#[test]
fn multiline_raw_byte_limit_counts_eol_and_excludes_prefixes_and_virtual_columns() {
    for eol in ["\n", "\r\n"] {
        for &(first, next) in PATHS {
            for size in [998, 999, 1000] {
                let suffix = "a".repeat(size - eol.len() - 1);
                let source = format!("{first}[a{eol}{next}{suffix}]: /url\n");
                let doc = parse(&source);
                assert_eq!(
                    defined(&doc),
                    usize::from(size <= 999),
                    "size={size} {first:?} {next:?}"
                );
                if size <= 999 {
                    assert_eq!(doc.work().label_bytes, size as u64);
                    assert_eq!(doc.work().inline_bytes, 0);
                }
            }
        }
    }
    // 969 explicit ancestors per line, 500 fragments, only 999 raw bytes.
    let prefix = format!("{}>\t", ">".repeat(968));
    let mut source = format!("{prefix}[a\n");
    for _ in 0..498 {
        source.push_str(&format!("{prefix}a\n"));
    }
    source.push_str(&format!("{prefix}a]: /url\n"));
    let doc = parse(&source);
    assert_eq!(defined(&doc), 1);
    assert_eq!(doc.work().label_bytes, 999);
    assert_eq!(doc.container_count(), 969);
    assert_eq!(doc.blocks().len(), 1);
    assert_eq!(doc.work().inline_bytes, 0);
    assert!(doc.work().classified_bytes < 12 * source.len() as u64);
    assert!(doc.work().container_bytes < 20 * source.len() as u64);
    eprintln!("deep success bytes={} work={:?}", source.len(), doc.work());
}

#[test]
fn multiline_invalid_labels_and_existing_paragraphs_never_define() {
    for &(first, next) in PATHS {
        for (head, tail) in [
            ("[ ", "\t]: /url"),
            ("[a", "b] : /url"),
            ("[a", "b]: <bad"),
            ("[a", "b]: /url 'bad"),
            ("[a", "b[bad]: /url"),
            ("[a", "b]"),
            ("[a", "\\\\] : /url"),
            ("[a", "\\]"),
        ] {
            let source = format!("{first}{head}\r\n{next}{tail}\r\n");
            assert_eq!(defined(&parse(&source)), 0, "{source:?}");
        }
        let source = format!("{first}seed\n{next}[a\n{next}b]: /url\n");
        assert_eq!(defined(&parse(&source)), 0, "{source:?}");
    }
}

#[test]
fn multiline_pending_folds_updates_and_shared_metadata_microbudgets() {
    for &(first, next) in PATHS {
        for success in [false, true] {
            let mut doc = Document::new();
            doc.append("[x][é 中]\n\n").unwrap();
            let old = *doc.link_info(LinkId(0)).unwrap();
            let start = doc.source().len();
            let mut view = FoldView::new(&doc, 1).unwrap();
            let mut batched = FoldView::new(&doc, 1).unwrap();
            let mut merged = Update::default();
            let mut blocks = doc.blocks().to_vec();
            let ticket = view.heights().ticket(TileId(0)).unwrap();
            view.measure(ticket, 3).unwrap();
            let batched_ticket = batched.heights().ticket(TileId(0)).unwrap();
            batched.measure(batched_ticket, 3).unwrap();
            for (i, chunk) in [
                format!("{first}[É\r\n"),
                format!("{next} 中]:\r\n"),
                format!("{next}<中&amp;\\*>\r\n"),
                format!(
                    "{next}'🙂&amp;\\*'{}\r\n",
                    if success { "" } else { " bad" }
                ),
            ]
            .iter()
            .enumerate()
            {
                let update = doc.append(chunk).unwrap();
                blocks.extend_from_slice(&doc.blocks()[update.new_blocks.clone()]);
                for b in update.changed_blocks.clone() {
                    blocks[b] = doc.blocks()[b].clone();
                }
                assert_eq!(blocks, doc.blocks());
                view.apply(&doc, &update).unwrap();
                merged.merge(update).unwrap();
                assert_eq!(doc.link_info(LinkId(0)), Some(&old));
                if i < 3 {
                    assert_eq!(doc.source_fold_limit(), start);
                }
                if i < 2 {
                    assert_eq!(doc.links().len(), 1);
                    assert_eq!(doc.blocks().len(), 3);
                    assert!(
                        SourceFoldRegistration::new(&doc, &view, doc.blocks()[2].source).is_err()
                    );
                }
            }
            let update = doc.finish();
            view.apply(&doc, &update).unwrap();
            merged.merge(update).unwrap();
            batched.apply(&doc, &merged).unwrap();
            assert_eq!(view.heights().total_rows(), batched.heights().total_rows());
            for at in (0..doc.source().len()).filter(|&at| doc.source().is_char_boundary(at)) {
                assert_eq!(
                    view.hidden_at(&doc, at, &mut Budget::new(WorkLimit::unlimited()))
                        .unwrap(),
                    batched
                        .hidden_at(&doc, at, &mut Budget::new(WorkLimit::unlimited()))
                        .unwrap()
                );
            }
            assert!(view.measure(ticket, 3).is_err());
            assert_eq!(doc.link_destination(LinkId(0)), Some("中&amp;\\*"));
            assert_eq!(doc.link_title(LinkId(0)), success.then_some("🙂&amp;\\*"));
            assert!(
                merged
                    .changes
                    .iter()
                    .any(|c| c.kind == ChangeKind::Structure && c.source.start == start)
            );
            let mut display = doc
                .display_cursor(Span::new(0, doc.source().len()))
                .unwrap();
            let mut destination = doc.decoded_link_destination(LinkId(0)).unwrap();
            let mut title = doc.decoded_link_title(LinkId(0));
            let (mut dest, mut tit) = (String::new(), String::new());
            let mut visible = String::new();
            let mut steps = 0;
            loop {
                steps += 1;
                assert!(steps < 20 * doc.source().len());
                let mut budget = Budget::new(metadata::SIX_BYTES);
                match display.next(&doc, &mut budget).unwrap() {
                    DisplayStep::Run(run) => {
                        if !run.style.contains(Style::HIDDEN) {
                            visible.push_str(run.text);
                        }
                    }
                    DisplayStep::BudgetExhausted | DisplayStep::Complete => (),
                }
                if let Ok(Some(chunk)) = destination.advance(&mut budget) {
                    dest.push_str(chunk.text);
                }
                if let Some(cursor) = &mut title
                    && let Ok(Some(chunk)) = cursor.advance(&mut budget)
                {
                    tit.push_str(chunk.text);
                }
                assert!(
                    budget.used().bytes <= 6
                        && budget.used().queries <= 1
                        && budget.used().objects <= 1
                );
                if display.source_position() == doc.source().len()
                    && destination.source_position()
                        == doc
                            .references()
                            .iter()
                            .find(|r| r.destination.is_some())
                            .unwrap()
                            .destination
                            .unwrap()
                            .end
                    && title.as_ref().is_none_or(|cursor| {
                        cursor.source_position()
                            == doc
                                .references()
                                .iter()
                                .find(|r| r.destination.is_some())
                                .unwrap()
                                .title
                                .unwrap()
                                .end
                    })
                {
                    break;
                }
            }
            assert_eq!(dest, "中&*");
            assert_eq!(tit, if success { "🙂&*" } else { "" });
            let bulk: String = doc
                .display_runs_in(Span::new(0, doc.source().len()))
                .unwrap()
                .filter(|r| !r.style.contains(Style::HIDDEN))
                .map(|r| r.text)
                .collect();
            assert_eq!(visible, bulk);
            metadata::snapshot(&doc);
        }
    }
}

#[test]
fn multiline_deep_failures_stream_million_byte_tails_once_and_never_refresh_history() {
    for depth in [0, 32, 128, 970] {
        let prefix = ">".repeat(depth);
        let mut doc = Document::new();
        doc.append(&format!("{prefix}[a\n")).unwrap();
        for _ in 0..40 {
            doc.append(&format!("{prefix}a\n")).unwrap();
        }
        let pending = doc.work();
        doc.append(&format!("{prefix}{}", "x".repeat(1_000_000)))
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
        assert!(before.classified_bytes < 15 * doc.source().len() as u64);
        assert!(before.inline_bytes < 2 * doc.source().len() as u64);
        eprintln!("failure depth={depth} pending={pending:?} final={before:?}");
    }
    let mut doc = Document::new();
    doc.append(&format!("{}\n\n", "[x][a b] ".repeat(20_000)))
        .unwrap();
    let links = doc.links().to_vec();
    let infos: Vec<_> = (0..links.len())
        .map(|i| *doc.link_info(LinkId(i)).unwrap())
        .collect();
    let before = doc.work();
    let update = doc.append("[A\nB]: /late\n 'title'\n").unwrap();
    assert_eq!(doc.links(), links);
    assert_eq!(update.references_changed, [SymbolId(1), SymbolId(1)]);
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert!(doc.work().tree_visits - before.tree_visits < 200);
    assert!(update.changes.len() < 12);
    for (i, info) in infos.iter().enumerate() {
        assert_eq!(doc.link_info(LinkId(i)), Some(info));
    }
    assert_eq!(doc.link_destination(LinkId(19_999)), Some("/late"));
    for i in 0..4096 {
        doc.append(&format!("[n{i}\nx]: /url\n")).unwrap();
    }
    let before = doc.work();
    let update = doc.append("[a\nb]: /ignored\n 'ignored'\n").unwrap();
    assert!(update.references_changed.is_empty());
    assert_eq!(doc.link_title(LinkId(0)), Some("title"));
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert!(doc.work().tree_visits - before.tree_visits < 200);
    eprintln!(
        "late definition delta classified={} inline={} tree={} label={}",
        doc.work().classified_bytes - before.classified_bytes,
        doc.work().inline_bytes - before.inline_bytes,
        doc.work().tree_visits - before.tree_visits,
        doc.work().label_bytes - before.label_bytes
    );
}

#[test]
fn multiline_closing_pipe_line_table_priority_and_four_column_targets() {
    for (first, next, residue) in [
        ("", "", 0),
        ("> ", "> ", 0),
        (">\t", ">\t", 2),
        ("- - ", "\t", 0),
        ("- > ", "\t>\t", 2),
    ] {
        for padding in 0..6 {
            let source = format!("{first}[a\n{next}b|c]:\n{next}{}-|-\n", " ".repeat(padding));
            let doc = parse(&source);
            let table = !next.contains('\t') || residue + padding <= 3;
            assert_eq!(doc.tables().len(), usize::from(table), "{source:?}");
            assert_eq!(defined(&doc), usize::from(!table), "{source:?}");
            chunk_equivalence(&source);
        }
        let source = format!("{first}[a\n{next}b\\|c]:\n{next}-|-\n");
        let doc = parse(&source);
        assert!(doc.tables().is_empty());
        assert_eq!(defined(&doc), 1);
    }
}

#[test]
fn multiline_open_fold_registration_and_old_ids_survive_until_real_resolution() {
    for &(first, next) in PATHS {
        for tail in ["c]: /url\n", "c[bad]: /url\n", "===\n", "x|y\n-|-\n"] {
            let mut doc = Document::new();
            doc.append("[old](/old)\n\n").unwrap();
            let old = *doc.link_info(LinkId(0)).unwrap();
            let start = doc.source().len();
            let update = doc.append(&format!("{first}[a\n")).unwrap();
            let id = BlockId(update.new_blocks.start);
            let scope = doc.block(id).unwrap().container;
            let count = doc.container_count();
            let mut view = FoldView::new(&doc, 1).unwrap();
            let mut registration = if scope.is_some() {
                SourceFoldRegistration::container_paragraph(&doc, &view, id, 0).unwrap()
            } else {
                SourceFoldRegistration::open_paragraph(&doc, &view, id, 0).unwrap()
            };
            let fold = registration
                .advance(&doc, &mut view, &mut Budget::new(WorkLimit::unlimited()))
                .unwrap()
                .id
                .unwrap();
            view.set_collapsed(&doc, fold, true).unwrap();
            for chunk in [format!("{next}b\n"), format!("{next}{tail}")] {
                let update = doc.append(&chunk).unwrap();
                view.apply(&doc, &update).unwrap();
                assert_eq!(doc.link_info(LinkId(0)), Some(&old));
                assert_eq!(doc.block(id).unwrap().container, scope);
                assert_eq!(doc.block_at(start), Some(id));
                assert_eq!(doc.container_count(), count);
            }
            let update = doc.finish();
            view.apply(&doc, &update).unwrap();
            if tail.starts_with("c]:") || tail == "===\n" {
                assert!(view.fold_candidate(&doc, fold).unwrap().is_none());
            }
            assert_eq!(doc.link_info(LinkId(0)), Some(&old));
        }
    }
}
