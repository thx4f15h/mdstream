use mdstream::*;

#[path = "support/strike_model.rs"]
mod model;
#[path = "support/snapshot.rs"]
mod support;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn flags(doc: &Document) -> Vec<(bool, bool)> {
    doc.runs_in(Span::new(0, doc.source().len()))
        .unwrap()
        .flat_map(|r| {
            std::iter::repeat_n(
                (
                    r.style.contains(Style::HIDDEN),
                    r.style.contains(Style::STRIKE),
                ),
                r.source.len(),
            )
        })
        .collect()
}

fn visible(doc: &Document) -> String {
    doc.display_runs_in(Span::new(0, doc.source().len()))
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect()
}

fn style_at(doc: &Document, needle: &str) -> Style {
    let at = doc.source().find(needle).unwrap();
    doc.runs_in(Span::new(at, at + needle.len()))
        .unwrap()
        .next()
        .unwrap()
        .style
}

#[test]
fn single_and_double_runs_match_independent_batch_model() {
    // Space, punctuation, word/Unicode intraword, mismatches, unfinished runs,
    // greedy same-kind pairing and crossing cancellation are all observable.
    let tokens = [
        "~", "~~", "~~~", "~~~~", "~a ", "~~a ", "x~", "x~~", " ", "!", "中", "\\~",
    ];
    for a in tokens {
        for b in tokens {
            for c in tokens {
                // Keep the oracle corpus away from paragraph-tail whitespace hiding.
                let source = format!("prefix {a}{b}{c} end");
                assert_eq!(flags(&parse(&source)), model::marks(&source), "{source:?}");
            }
        }
    }
    for source in [
        "~x~",
        "~~x~~",
        "~x~~",
        "~~x~",
        "~ x~",
        "~x ~",
        "~\t~",
        "~",
        "x ~~~x~~~",
        "x ~~~~x~~~~",
        "~a ~~b~~ c~",
        "~~a ~b~ c~~",
        "~a ~~b~ c~~",
        "~~a ~b~~ c~",
        "~a ~b~ c~",
        "~a\\~b~",
        "\\~x~",
        "~x\\~",
        "~中\u{a0}~",
        "x~中~y",
        "~!~",
    ] {
        assert_eq!(flags(&parse(source)), model::marks(source), "{source:?}");
    }
    for (source, expected) in [
        ("~a ~~b~~ c~", "a b c"),
        ("~~a ~b~ c~~", "a b c"),
        ("~a ~~b~ c~~", "a ~~b c~~"),
        ("~~a ~b~~ c~", "a ~b c~"),
        ("~a ~b~ c~", "a b c"),
        ("~a~b~c~", "abc"),
    ] {
        // Same-kind nesting remains greedy and the public bit is flat.
        assert_eq!(visible(&parse(source)), expected);
    }
}

#[test]
fn live_maximal_run_waits_for_next_byte_or_eof_without_partial_consumption() {
    for open in ["~", "~~", "~~~", "~~~~"] {
        for close in ["~", "~~", "~~~", "~~~~"] {
            for eof in [false, true] {
                let mut source = format!("{} {open}中😀", "x".repeat(BLOCK_LOOKAHEAD));
                let mut doc = Document::new();
                doc.append(&source).unwrap();
                let block = doc.block_at(0).unwrap();
                for byte in close.chars() {
                    source.push(byte);
                    doc.append(byte.encode_utf8(&mut [0; 4])).unwrap();
                    let mut whole = Document::new();
                    whole.append(&source).unwrap();
                    assert_eq!(support::snapshot(&doc), support::snapshot(&whole));
                    assert_eq!(doc.pending(), whole.pending());
                    assert!(!style_at(&doc, "中").contains(Style::STRIKE));
                    assert_eq!(doc.block_at(0), Some(block));
                }
                if !eof {
                    source.push('!');
                    doc.append("!").unwrap();
                    assert_eq!(
                        style_at(&doc, "中").contains(Style::STRIKE),
                        open == close && open.len() <= 2
                    );
                }
                doc.finish();
                assert_eq!(
                    flags(&doc),
                    model::marks(&source),
                    "{open:?} {close:?} EOF={eof}"
                );
                assert_eq!(doc.block_at(0), Some(block));
                assert_eq!(doc.work().inline_bytes, source.len() as u64);
            }
        }
    }
}

#[test]
fn objects_keep_existing_priority_and_failed_candidates_stay_opaque() {
    for (source, text, code, maths, links) in [
        ("~a `~raw~` b~", "a ~raw~ b", true, 0, 0),
        ("`~raw~` ~yes~", "~raw~ yes", true, 0, 0),
        ("`~raw~ ~unfinished", "`~raw~ ~unfinished", false, 0, 0),
        ("~a $~raw~$ b~", "a ~raw~ b", false, 1, 0),
        ("$~raw~$ ~yes~", "~raw~ yes", false, 1, 0),
        ("$~raw~ ~unfinished", "$~raw~ ~unfinished", false, 0, 0),
        ("[~label~](/~dest~)", "label", false, 0, 1),
        ("~[label](/~dest~)~", "label", false, 0, 1),
        ("~[label~](/u)", "~label~", false, 0, 1),
        ("[~label](/u)~", "~label~", false, 0, 1),
        ("[~label~", "[label", false, 0, 0),
        ("~*em* **strong**~", "em strong", false, 0, 0),
    ] {
        let doc = parse(source);
        assert_eq!(visible(&doc), text, "{source:?}");
        assert_eq!(doc.maths().len(), maths);
        assert_eq!(doc.links().len(), links);
        assert_eq!(
            doc.runs_in(Span::new(0, source.len()))
                .unwrap()
                .any(|r| r.style.contains(Style::CODE)),
            code
        );
        if source.contains("label") {
            assert_eq!(
                style_at(&doc, "label").contains(Style::STRIKE),
                !matches!(source, "[~label](/u)~" | "~[label~](/u)")
            );
        }
        if source.starts_with('`') || source.starts_with('$') {
            assert!(!style_at(&doc, "raw").contains(Style::STRIKE));
        }
        if source.starts_with("~a ") {
            assert!(style_at(&doc, "raw").contains(Style::STRIKE));
        }
    }
}

#[test]
fn scope_boundaries_keep_single_runs_local() {
    for (source, matched) in [
        ("~one\ntwo~\n", true),
        ("> ~one\n> two~\n", true),
        ("> > ~one\n>> two~\n", true),
        ("> - ~one\n>   two~\n", true),
        ("> - ~one\ntwo~\n", true),
        ("~one\n\ntwo~\n", false),
        ("~one\n# two~\n", false),
        ("- ~one\n- two~\n", false),
        ("> ~one\n\n> two~\n", false),
        ("~one|two~\n-|-\n", false),
        ("a|b\n-|-\n~one|two~\n", false),
        ("~one|two~\nwrong\n", true),
    ] {
        let doc = parse(source);
        assert_eq!(
            style_at(&doc, "one").contains(Style::STRIKE),
            matched,
            "{source:?}"
        );
        assert_eq!(
            style_at(&doc, "two").contains(Style::STRIKE),
            matched,
            "{source:?}"
        );
    }
}

#[test]
fn arbitrary_utf8_chunks_live_prefixes_and_eof_keep_full_semantics() {
    for seed in [String::new(), format!("{} ", "x".repeat(BLOCK_LOOKAHEAD))] {
        for source in [
            "~中😀~ ~~双~~ ~错~~ ~未完",
            "~a ~~中~ c~~ \\~ ~!~ ~\u{a0}~\n",
            "> - ~中\r\n>   `~原~` $~式~$ [~签~](/~目标~) 😀~\n",
            "> ~先\n> 后~ |~中~\n> -|-\n",
            "~先\n后~next |中\n-|-\n",
        ] {
            let expected = parse(&format!("{seed}{source}"));
            for split in 0..=source.len() {
                let mut stream = Utf8Document::new();
                stream.append(seed.as_bytes()).unwrap();
                stream.append(&source.as_bytes()[..split]).unwrap();
                stream.append(&source.as_bytes()[split..]).unwrap();
                stream.finish();
                assert_eq!(
                    support::snapshot(stream.document()),
                    support::snapshot(&expected),
                    "{source:?} byte {split}"
                );
                assert_eq!(stream.document().pending(), expected.pending());
            }
            let mut stream = Utf8Document::new();
            stream.append(seed.as_bytes()).unwrap();
            for end in 1..=source.len() {
                stream.append(&source.as_bytes()[end - 1..end]).unwrap();
                let mut whole = Utf8Document::new();
                whole.append(seed.as_bytes()).unwrap();
                whole.append(&source.as_bytes()[..end]).unwrap();
                assert_eq!(
                    support::snapshot(stream.document()),
                    support::snapshot(whole.document())
                );
                assert_eq!(stream.document().pending(), whole.document().pending());
                assert_eq!(stream.pending_bytes(), whole.pending_bytes());
                let mut prefix = Utf8Document::new();
                prefix.append(seed.as_bytes()).unwrap();
                for byte in &source.as_bytes()[..end] {
                    prefix.append(&[*byte]).unwrap();
                }
                prefix.finish();
                whole.finish();
                assert_eq!(
                    support::snapshot(prefix.document()),
                    support::snapshot(whole.document()),
                    "EOF {end}"
                );
                assert_eq!(prefix.document().pending(), whole.document().pending());
            }
            stream.finish();
            assert_eq!(
                support::snapshot(stream.document()),
                support::snapshot(&expected)
            );
        }
    }
}

#[test]
fn late_closure_preserves_ids_raw_mapping_and_shared_query_budget() {
    let mut doc = Document::new();
    doc.append(&format!(
        "> - ~中\\~😀 [签](/~目标~ \"标题\") $式$ `code` {}",
        "x".repeat(2048)
    ))
    .unwrap();
    let block = doc.block_at(0).unwrap();
    let container = doc.blocks()[block.0].container;
    let link = *doc.link_info(LinkId(0)).unwrap();
    let math = doc.maths()[0];
    let tile = doc.tile_key(TileId(0)).unwrap();
    assert!(!style_at(&doc, "中").contains(Style::STRIKE));
    doc.append("尾~!\n").unwrap();
    doc.finish();
    assert_eq!(doc.block_at(0), Some(block));
    assert_eq!(doc.blocks()[block.0].container, container);
    assert_eq!(*doc.link_info(LinkId(0)).unwrap(), link);
    assert_eq!(doc.maths()[0], math);
    assert_ne!(doc.tile_key(TileId(0)).unwrap(), tile);
    assert!(style_at(&doc, "中").contains(Style::STRIKE));
    assert!(style_at(&doc, "签").contains(Style::STRIKE));
    assert!(style_at(&doc, "code").contains(Style::CODE));
    assert!(style_at(&doc, "code").contains(Style::STRIKE));
    assert_eq!(doc.link_destination(LinkId(0)), Some("/~目标~"));
    let span = Span::new(0, doc.source().len());
    let expected: Vec<_> = doc
        .display_runs_in(span)
        .unwrap()
        .map(|r| (r.source, r.style, r.link, r.text.to_owned()))
        .collect();
    let mut end = 0;
    for (source, _, _, text) in &expected {
        assert_eq!(source.start, end);
        assert_eq!(doc.text(*source), Some(text.as_str()));
        end = source.end;
    }
    assert_eq!(end, span.end);
    for (at, ch) in doc.source().char_indices() {
        let slice = Span::new(at, at + ch.len_utf8());
        let run = doc.display_runs_in(slice).unwrap().next().unwrap();
        let whole = expected
            .iter()
            .find(|r| r.0.start <= at && at < r.0.end)
            .unwrap();
        assert_eq!(run.source, slice);
        assert_eq!((run.style, run.link), (whole.1, whole.2));
        assert_eq!(run.text, doc.text(slice).unwrap());
    }
    let mut cursor = doc.display_cursor(span).unwrap();
    let mut peer = doc.display_cursor(span).unwrap();
    assert_eq!(
        cursor
            .next(&doc, &mut Budget::new(WorkLimit::default()))
            .unwrap(),
        DisplayStep::BudgetExhausted
    );
    assert_eq!(cursor.source_position(), 0);
    let mut text = String::new();
    while cursor.source_position() < span.end {
        let mut budget = Budget::new(WorkLimit {
            bytes: 6,
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        let before = cursor.source_position();
        let DisplayStep::Run(run) = cursor.next(&doc, &mut budget).unwrap() else {
            panic!("no progress")
        };
        assert_eq!(run.source.start, before);
        assert!(cursor.source_position() > before);
        assert_eq!(run.text, doc.text(run.source).unwrap());
        if !run.style.contains(Style::HIDDEN) {
            text.push_str(run.text);
        }
        let used = budget.used();
        let mut target = doc.decoded_link_destination(LinkId(0)).unwrap();
        let target_before = target.source_position();
        assert_eq!(
            target.advance(&mut budget),
            Err(Completion::BudgetExhausted)
        );
        assert_eq!(target.source_position(), target_before);
        assert_eq!(
            peer.next(&doc, &mut budget).unwrap(),
            DisplayStep::BudgetExhausted
        );
        assert_eq!(peer.source_position(), 0);
        assert_eq!(budget.used(), used);
        assert!(used.bytes <= 6 && used.queries <= 1 && used.objects <= 1);
    }
    assert_eq!(text, visible(&doc));
    assert_eq!(
        cursor
            .next(&doc, &mut Budget::new(WorkLimit::default()))
            .unwrap(),
        DisplayStep::Complete
    );
    let slash = doc.source().find('\\').unwrap();
    assert!(flags(&doc)[slash].0);
    assert!(!flags(&doc)[slash + 1].0);
}

#[test]
fn deep_million_byte_headers_and_historical_mixed_openers_have_linear_work() {
    let prefix = ">".repeat(970);
    for success in [false, true] {
        let mut doc = Document::new();
        doc.append(&format!("{}\n\n", "[p](u) ".repeat(4096)))
            .unwrap();
        let historical = *doc.link_info(LinkId(4095)).unwrap();
        let before = doc.work();
        doc.append(&format!("{prefix}{}\n", "~old ~~old ".repeat(2048)))
            .unwrap();
        // 4096 maximal runs, each pushed once, then closed/popped once below.
        assert_eq!(doc.work().delimiter_steps - before.delimiter_steps, 8192);
        let block = doc.block_at(doc.source().len() - 1).unwrap();
        let before = doc.work();
        doc.append(&format!(
            "{prefix}{}[h](v) $m$ `code` {}",
            "end~~ end~ ".repeat(2048),
            "x".repeat(2048)
        ))
        .unwrap();
        assert!(doc.work().delimiter_steps - before.delimiter_steps <= 8198);
        let local = *doc.link_info(LinkId(4096)).unwrap();
        let math = doc.maths()[0];
        let before = doc.work();
        let annotations = doc.annotation_arena_stats();
        let containers = doc.container_arena_stats();
        for _ in 0..1000 {
            doc.append(&"x".repeat(1000)).unwrap();
        }
        let after = doc.work();
        assert_eq!(after.inline_bytes - before.inline_bytes, 1_000_000);
        assert_eq!(
            after.table_header_bytes - before.table_header_bytes,
            1_000_000
        );
        assert_eq!(after.delimiter_steps, before.delimiter_steps);
        assert_eq!(after.classified_bytes, before.classified_bytes);
        assert_eq!(after.container_bytes, before.container_bytes);
        assert_eq!(after.container_steps, before.container_steps);
        assert_eq!(after.heading_steps, before.heading_steps);
        assert_eq!(after.label_bytes, before.label_bytes);
        assert_eq!(after.tree_visits, before.tree_visits);
        assert_eq!(doc.annotation_arena_stats(), annotations);
        assert_eq!(doc.container_arena_stats(), containers);
        doc.append("|b\n").unwrap();
        doc.append(&format!(
            "{prefix}{}\n",
            if success { "-|-" } else { "wrong" }
        ))
        .unwrap();
        assert_eq!(doc.tables().len(), usize::from(success));
        assert_eq!(doc.work().delimiter_steps - after.delimiter_steps, 4096);
        assert_eq!(doc.work().label_bytes, after.label_bytes);
        assert_eq!(*doc.link_info(LinkId(4095)).unwrap(), historical);
        assert_eq!(*doc.link_info(LinkId(4096)).unwrap(), local);
        assert_eq!(doc.maths()[0], math);
        assert_eq!(doc.block_at(historical.source.start), Some(BlockId(0)));
        assert!(doc.blocks().get(block.0).is_some());
        doc.finish();
        assert_eq!(doc.work().delimiter_steps - after.delimiter_steps, 4096);
        assert!(doc.work().inline_bytes <= doc.source().len() as u64);
        assert!(doc.work().delimiter_steps <= 3 * doc.source().len() as u64);
    }
}
