use mdstream::*;

#[path = "support/shortcut_snapshot.rs"]
mod model;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn shown(doc: &Document, span: Span) -> String {
    doc.display_runs_in(span)
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect()
}

// Hand-written syntax and scalar truth, independent of annotation gates and
// snapshot normalization. Suffix text is opaque source, including entities.
#[test]
fn qualified_reference_syntax_follows_the_current_target() {
    for (source, label, target, syntax, hidden, literal, resolved) in [
        ("[a][b]", (1, 2), "b", (0, 1, 2, 6), &[][..], "[a][b]", "a"),
        ("[a][]", (1, 2), "a", (0, 1, 2, 5), &[][..], "[a][]", "a"),
        (
            "![*a*][b]",
            (2, 5),
            "b",
            (0, 2, 5, 9),
            &[(2, 3), (4, 5)][..],
            "![a][b]",
            "a",
        ),
        (
            "[中🙂][鍵]",
            (1, 8),
            "鍵",
            (0, 1, 8, 14),
            &[][..],
            "[中🙂][鍵]",
            "中🙂",
        ),
        (
            "[a\\]][b\\]]",
            (1, 4),
            "b\\]",
            (0, 1, 4, 10),
            &[(2, 3)][..],
            "[a]][b\\]]",
            "a]",
        ),
        (
            "[a][*b*&amp;\\!]",
            (1, 2),
            "*b*&amp;\\!",
            (0, 1, 2, 15),
            &[][..],
            "[a][*b*&amp;\\!]",
            "a",
        ),
    ] {
        let mut doc = Document::new();
        doc.append(&format!("{source}\n\n")).unwrap();
        let info = *doc.link_info(LinkId(0)).unwrap();
        assert_eq!(info.label, Span::new(label.0, label.1));
        assert_eq!(info.source, Span::new(0, source.len()));
        assert_eq!(info.state, SyntaxState::Sealed);
        let LinkTarget::Reference(symbol) = doc.links()[0] else {
            panic!()
        };
        assert_eq!(doc.references()[symbol.0].label, target);
        for phase in 0..4 {
            let update = match phase {
                1 if target != "a" => Some(doc.append("[a]: /a\n").unwrap()),
                2 => Some(doc.append(&format!("[{target}]: /u 'T'\n")).unwrap()),
                3 => Some(doc.append(&format!("[{target}]: /other\n")).unwrap()),
                _ => None,
            };
            let winner = phase >= 2;
            assert_eq!(doc.link_info(LinkId(0)), Some(&info));
            assert_eq!(doc.links()[0], LinkTarget::Reference(symbol));
            assert_eq!(
                shown(&doc, info.source),
                if winner {
                    resolved
                } else if source == "[a][*b*&amp;\\!]" {
                    "[a][b&amp;\\!]"
                } else {
                    literal
                }
            );
            for (at, ch) in source.char_indices() {
                let span = Span::new(at, at + ch.len_utf8());
                let syntax = (syntax.0 <= at && at < syntax.1) || (syntax.2 <= at && at < syntax.3);
                let permanent = hidden.iter().any(|&(a, b)| a <= at && at < b);
                let expected_hidden = permanent
                    || (syntax && winner)
                    || (source == "[a][*b*&amp;\\!]" && !winner && matches!(at, 4 | 6));
                let raw = doc.runs_in(span).unwrap().next().unwrap();
                let display = doc.display_runs_in(span).unwrap().next().unwrap();
                assert_eq!(
                    raw.style.contains(Style::HIDDEN),
                    expected_hidden,
                    "{source:?} {phase} {at}"
                );
                assert_eq!(display.style, raw.style);
                assert_eq!(raw.text, &source[span.start..span.end]);
                assert_eq!(display.text, raw.text);
                assert_eq!(display.replacement, None);
                assert_eq!(raw.link, (!syntax).then_some(LinkId(0)));
                assert_eq!(display.link, raw.link);
                if source == "![*a*][b]" && at == 3 {
                    assert_eq!(raw.emphasis_depth, 1);
                    assert!(raw.style.contains(Style::EMPHASIS));
                }
            }
            if phase == 3 {
                assert!(
                    !update
                        .unwrap()
                        .changes
                        .iter()
                        .any(|c| c.kind == ChangeKind::Visibility && c.source.start == 0)
                );
            }
        }
    }
}

#[test]
fn resolved_shortcut_can_upgrade_to_an_unresolved_full_with_the_same_id() {
    for suffix in ["[b]", "[]", "(u 'T')"] {
        let mut doc = Document::new();
        doc.append(&format!(
            "[a]: /a\n\n{}![a]",
            "x".repeat(BLOCK_LOOKAHEAD + 1)
        ))
        .unwrap();
        let old = *doc.link_info(LinkId(0)).unwrap();
        assert_eq!(doc.link_destination(LinkId(0)), Some("/a"));
        doc.append(suffix).unwrap();
        let info = *doc.link_info(LinkId(0)).unwrap();
        assert_eq!(doc.links().len(), 1);
        assert_eq!(info.label, old.label);
        assert_eq!(info.source.start, old.source.start);
        assert_eq!(info.source.end, old.source.end + suffix.len());
        assert_eq!(
            shown(&doc, info.source),
            if suffix == "[b]" { "![a][b]" } else { "a" }
        );
        if suffix == "[b]" {
            assert_eq!(doc.link_destination(LinkId(0)), None);
            doc.append("\n\n[b]: /b\n").unwrap();
            assert_eq!(shown(&doc, info.source), "a");
            assert_eq!(doc.link_info(LinkId(0)), Some(&info));
        }
    }
}

#[test]
fn excluded_candidates_and_failed_suffixes_keep_the_previous_dialect() {
    for (source, expected, count) in [
        ("[][b]", "", 1),
        ("![][b]", "", 1),
        ("[ ][b]", " ", 1),
        ("![][]", "![][]", 0),
        ("[a](u 't')", "a", 1),
        ("[a][b", "[a][b", 1),
        ("[a][ ]", "[a][ ]", 1),
    ] {
        let doc = parse(source);
        assert_eq!(shown(&doc, Span::new(0, source.len())), expected);
        assert_eq!(doc.links().len(), count);
    }
    for n in [999, 1000] {
        let label = "x".repeat(n);
        let source = format!("[{label}][b]");
        let doc = parse(&source);
        assert_eq!(doc.links().len(), 1);
        assert_eq!(
            shown(&doc, Span::new(0, source.len())),
            if n == 999 { source.clone() } else { label }
        );
        assert_eq!(
            doc.link_info(LinkId(0)).unwrap().source,
            Span::new(0, source.len())
        );
    }
}

#[test]
fn utf8_cuts_prefix_eof_and_scratch_keep_full_reference_semantics() {
    for &(source, _, _) in BASELINE.iter().filter(|(source, _, _)| source.len() < 180) {
        let expected = model::describe(&parse(source), &[], true);
        for cut in 0..=source.len() {
            let mut doc = Utf8Document::new();
            doc.append(&source.as_bytes()[..cut]).unwrap();
            doc.append(b"").unwrap();
            doc.append(&source.as_bytes()[cut..]).unwrap();
            doc.finish();
            assert_eq!(model::describe(doc.document(), &[], true), expected);
            let mut bytes = Utf8Document::new();
            for b in source.as_bytes()[..cut].chunks(1) {
                bytes.append(b).unwrap();
            }
            bytes.finish();
            let mut once = Utf8Document::new();
            once.append(&source.as_bytes()[..cut]).unwrap();
            once.finish();
            assert_eq!(
                model::describe(bytes.document(), &[], true),
                model::describe(once.document(), &[], true)
            );
        }
        let doc = parse(source);
        let span = Span::new(0, source.len());
        let mut scratch = QueryScratch::default();
        assert_eq!(
            doc.runs_in(span).unwrap().collect::<Vec<_>>(),
            doc.runs_in_with_scratch(span, &mut scratch)
                .unwrap()
                .collect::<Vec<_>>()
        );
        assert_eq!(
            doc.display_runs_in(span).unwrap().collect::<Vec<_>>(),
            doc.display_runs_in_with_scratch(span, &mut scratch)
                .unwrap()
                .collect::<Vec<_>>()
        );
    }
}

include!("fixtures/reference_boundary_baseline.rs");

#[test]
fn predecessor_corpus_all_structures_payloads_and_scalar_mappings_are_frozen() {
    for &(source, syntax, expected) in BASELINE {
        let desc = model::describe(&parse(source), syntax, true);
        let hash = desc.bytes().fold(0xcbf29ce484222325u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
        });
        assert_eq!(hash, expected, "{source:?}");
    }
}

#[test]
fn full_reference_winner_mutation_does_not_visit_historical_uses() {
    let mut costs = Vec::new();
    for history in [1, 20_000] {
        let mut doc = Document::new();
        doc.append(&"[a][b] ".repeat(history)).unwrap();
        doc.append("\n\n").unwrap();
        let before = doc.work();
        doc.append("[b]: /u\n").unwrap();
        let after = doc.work();
        assert_eq!(after.inline_bytes, before.inline_bytes);
        assert_eq!(after.label_bytes - before.label_bytes, 1);
        assert!(after.tree_visits - before.tree_visits < 100);
        costs.push(after.tree_visits - before.tree_visits);
        assert_eq!(doc.links().len(), history);
    }
    assert!(costs[1] <= costs[0] + 40);
}

#[test]
fn winner_orders_title_continuations_and_duplicates_preserve_original_metadata() {
    for definition_first in [false, true] {
        for a_first in [false, true] {
            let mut doc = Document::new();
            if definition_first {
                doc.append("[b]: /b\n\n").unwrap();
            }
            doc.append("[a][b] ![a][b] [a][]\n\n").unwrap();
            let infos: Vec<_> = (0..3).map(|i| *doc.link_info(LinkId(i)).unwrap()).collect();
            for target in if a_first { ["a", "b"] } else { ["b", "a"] } {
                doc.append(&format!("[{target}]: /{target}\n")).unwrap();
                for (i, info) in infos.iter().enumerate() {
                    assert_eq!(doc.link_info(LinkId(i)), Some(info));
                    let winner = doc.link_destination(LinkId(i)).is_some();
                    let literal = if i == 0 {
                        "[a][b]"
                    } else if i == 1 {
                        "![a][b]"
                    } else {
                        "[a][]"
                    };
                    assert_eq!(shown(&doc, info.source), if winner { "a" } else { literal });
                }
                let title = doc.append(" 'T'\n").unwrap();
                assert!(
                    !title
                        .changes
                        .iter()
                        .any(|c| c.kind == ChangeKind::Visibility
                            && c.source.start <= infos[2].source.end)
                );
                let duplicate = doc.append(&format!("[{target}]: /duplicate\n")).unwrap();
                assert!(duplicate.references_changed.is_empty());
                assert!(
                    !duplicate
                        .changes
                        .iter()
                        .any(|c| c.kind == ChangeKind::Visibility
                            && c.source.start <= infos[2].source.end)
                );
            }
            for (i, info) in infos.iter().enumerate() {
                assert_eq!(doc.link_info(LinkId(i)), Some(info));
                assert_eq!(info.title, None);
            }
        }
    }
}
