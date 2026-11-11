use mdstream::*;
#[allow(dead_code)]
#[path = "support/shortcut_hashes.rs"]
mod shortcut_hashes;
#[path = "support/snapshot.rs"]
mod support;

fn cases() -> Vec<String> {
    let mut cases: Vec<String> = [
        "plain 中🙂 &amp; \\* *a* **b** _c_ ~~d~~\r\nnext\n",
        "[Straße][] [a][ STRASSE ] [ ][] [x](u 't') ![i](<url>)\n",
        "[*a* unmatched*] _tail_ <https://x.y> <a@b.co> [^签]\n",
        "`x`` a\r\nb `` tail $x$ $$中$$ &NotEqualTilde;\n",
        "[a](u\n 't')\n",
        "*prior [a](u\nend*and | b\n-|-\n",
        "   root\n  next\n",
        "a\n===\n",
        "# head [a] *b*\n",
        "> quoted [a]\n> *b*\n",
        "- item [a]\n  *b*\n",
        "> a\nlazy [a]\n",
        "> a\n>\tpartial\n",
        "```\n[a]\n```\n",
        "$$\n[a]\n$$\n",
        "[^f]: [a]\n",
        "a | b\n-|-\n[a] | $x$\n",
        "a | b\nwrong [x]\n",
        "[r]: /u 'title'\n\n[r]\n",
        "[r]:\n/u\n\n[r]\n",
        "[r\n label]: /u\n\n[r label]\n",
        "[r\n invalid text\n",
        "[r]: /u 'pending\ncontinued\n",
        "[r|x\nfailed\n",
        "[r]: /u\n 'title'\n[r]\n",
        "[r]: /u\ninvalid [r]\n",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    for prior in ["*prior [a](u\n", "[a](u\n", "`x``a\n", "$x\n", "[old *x\n"] {
        for end in ["-| -\n", "wrong\n"] {
            cases.push(format!("{prior}end*and {} | b\n{end}", "x".repeat(1100)));
        }
    }
    for label in [
        "x".repeat(999),
        "x".repeat(1000),
        "İ".repeat(499),
        " \t\u{2003}".into(),
    ] {
        cases.push(format!("*outside [{label}][] tail* [a][{label}]\n"));
    }
    cases
}

fn ends(source: &str) -> Vec<usize> {
    if source.len() <= 180 {
        (0..=source.len()).collect()
    } else {
        [
            0,
            1,
            3,
            16,
            999,
            1023,
            1024,
            1025,
            source.len() - 2,
            source.len() - 1,
            source.len(),
        ]
        .into_iter()
        .filter(|&n| n <= source.len())
        .collect()
    }
}

fn observed(source: &[u8], bytewise: bool) -> u64 {
    let mut stream = Utf8Document::new();
    for chunk in source.chunks(if bytewise { 1 } else { source.len().max(1) }) {
        stream.append(chunk).unwrap();
    }
    stream.finish();
    if matches!(source, b"[a](u\n 't')" | b"[a](u\n 't')\n") {
        // The only two newly legal sources in this frozen corpus. Rule 6.3
        // permits the LF separating the complete target and title. Assert the
        // raw object model independently before checking its reviewed snapshot.
        let doc = stream.document();
        assert_eq!(doc.links(), [LinkTarget::Direct(Span::new(4, 5))]);
        assert_eq!(doc.link_destination(LinkId(0)), Some("u"));
        assert_eq!(doc.link_title(LinkId(0)), Some("t"));
        assert_eq!(
            doc.link_info(LinkId(0)),
            Some(&LinkInfo {
                kind: LinkKind::Link,
                label: Span::new(1, 2),
                source: Span::new(0, 11),
                title: Some(Span::new(8, 9)),
                state: SyntaxState::Sealed,
            })
        );
        let visible: String = doc
            .display_runs_in(Span::new(0, source.len()))
            .unwrap()
            .filter(|r| !r.style.contains(Style::HIDDEN))
            .map(|r| r.text)
            .collect();
        assert_eq!(visible, if source.len() == 11 { "a" } else { "a\n" });
        assert_eq!(doc.references().len(), 1);
        assert_eq!(doc.references()[0].first_use, Some(0));
        assert_eq!(doc.references()[0].destination, None);
    }
    let snapshot = format!("{:?}", support::snapshot(stream.document()));
    snapshot.bytes().fold(0xcbf29ce484222325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    })
}

include!("fixtures/root_eligibility_baseline.rs");

#[test]
fn root_eligibility_preserves_real_head_snapshots_including_utf8_and_prefix_eof() {
    let mut index = 0;
    for (case, source) in cases().iter().enumerate() {
        for end in ends(source) {
            for bytewise in [false, true] {
                assert_eq!(
                    observed(&source.as_bytes()[..end], bytewise),
                    shortcut_hashes::root(
                        case,
                        end,
                        bytewise,
                        match (case, end) {
                            // Exact complete sources above, not an LF exemption.
                            (4, 11) => 0xc578db03a65f9c0e,
                            (4, 12) => 0x239d8b333a6535cb,
                            _ => BASELINE[index],
                        }
                    ),
                    "case={case} end={end} bytewise={bytewise}"
                );
                index += 1;
            }
        }
    }
    assert_eq!(index, BASELINE.len());
}

#[test]
fn root_eligibility_work_is_separate_and_only_observes_actual_root_payload() {
    let mut doc = Document::new();
    doc.append(&format!("   ordinary {}", "x".repeat(1100)))
        .unwrap();
    assert_eq!(doc.work().root_eligibility_bytes, doc.work().inline_bytes);
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("x").unwrap();
    }
    assert_eq!(
        doc.work().root_eligibility_bytes - before.root_eligibility_bytes,
        1000
    );
    assert_eq!(doc.work().inline_bytes - before.inline_bytes, 1000);
    assert_eq!(
        doc.work().root_eligibility_steps - before.root_eligibility_steps,
        1000
    );
    doc.finish();
    for source in [
        "> a [b]\n",
        "- a [b]\n",
        "# a [b]\n",
        "```\n[b]\n```\n",
        "$$\n[b]\n$$\n",
        "[^f]: [b]\n",
        "a|b\n-|-\n",
        "[r]: /u\n",
    ] {
        let mut doc = Document::new();
        doc.append(source).unwrap();
        doc.finish();
        assert_eq!(doc.work().root_eligibility_bytes, 0, "{source:?}");
    }
}

#[test]
fn root_eligibility_counts_million_byte_and_fallback_work_without_history_visits() {
    let mut doc = Document::new();
    for i in 0..20_000 {
        doc.append(&format!("[r{i}]: /u\n")).unwrap();
    }
    doc.append("\n").unwrap();
    let history = doc.references().to_vec();
    doc.append(&format!("root `x``{}", "a".repeat(1_000_000)))
        .unwrap();
    let before = doc.work();
    for _ in 0..1000 {
        doc.append("b").unwrap();
    }
    let grown = doc.work();
    assert_eq!(
        grown.root_eligibility_bytes - before.root_eligibility_bytes,
        1000
    );
    assert_eq!(
        grown.root_eligibility_steps - before.root_eligibility_steps,
        1000
    );
    assert_eq!(grown.inline_bytes - before.inline_bytes, 1000);
    assert_eq!(grown.tree_visits, before.tree_visits);
    assert_eq!(doc.references(), history);
    doc.finish();
    assert_eq!(doc.references(), history);
    assert!(doc.work().root_eligibility_bytes <= doc.source().len() as u64);
    for runs in [128, 1024, 8192] {
        let source = format!("root `x{}", "``a```b``c ".repeat(runs));
        let mut doc = Document::new();
        doc.append(&source).unwrap();
        doc.finish();
        let work = doc.work();
        assert_eq!(work.root_eligibility_bytes, source.len() as u64);
        assert!(work.root_eligibility_steps <= 24 * source.len() as u64);
        assert!(
            work.root_eligibility_steps
                >= work.inline_bytes + work.delimiter_steps + work.delimiter_search_steps
        );
    }
}
