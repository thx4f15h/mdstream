//! Independent expectations for the deferred GFM bare-address recognizer.
use mdstream::*;
#[path = "support/snapshot.rs"]
mod support;
use support::snapshot;

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}
fn labels(doc: &Document) -> Vec<(&str, String)> {
    (0..doc.links().len())
        .map(|n| {
            let id = LinkId(n);
            (
                doc.text(doc.link_info(id).unwrap().label).unwrap(),
                doc.link_url(id).unwrap().into_owned(),
            )
        })
        .collect()
}

#[test]
fn independent_domain_and_boundary_matrix() {
    let mut count = 0;
    for before in ["", " ", "(", "*", "_", "~", "a", "/", ":", "中"] {
        for domain in [
            "a.b",
            "a-b.c",
            "a_b.c.d",
            "a.b_c",
            "a_b.c",
            "a..b",
            "a",
            "a.b.",
            "例子.测试",
        ] {
            for scheme in ["www.", "http://", "https://"] {
                let source = format!("x {before}{scheme}{domain}/path\n");
                // Domain model is intentionally segment based, independent of
                // candidate modes, prefix probes and the core's trimming code.
                let host = if scheme == "www." {
                    format!("www.{domain}")
                } else {
                    domain.to_owned()
                };
                let parts: Vec<_> = host.split('.').collect();
                let valid = parts.len() >= 2
                    && parts.iter().all(|s| !s.is_empty())
                    && parts[parts.len() - 2..].iter().all(|s| !s.contains('_'));
                let boundary = matches!(before, "" | " " | "(" | "*" | "_" | "~");
                let doc = parse(&source);
                assert_eq!(
                    doc.links().len(),
                    usize::from(valid && boundary),
                    "{source:?}"
                );
                if valid && boundary {
                    let expected = format!("{scheme}{domain}/path");
                    assert_eq!(labels(&doc)[0].0, expected);
                    assert_eq!(
                        labels(&doc)[0].1,
                        if scheme == "www." {
                            format!("http://{expected}")
                        } else {
                            expected
                        }
                    );
                }
                count += 1;
            }
        }
    }
    assert_eq!(count, 270);
}

#[test]
fn independent_email_protocol_and_resource_matrix() {
    let mut count = 0;
    for local in ["a", "a.b-c_d+e", "_a", "+", ".", ""] {
        for domain in [
            "a.b", "a_b.c", "-a.b", "a.b-", "a.b_", "a..b", "a", "a.b.", "a.b...",
        ] {
            for scheme in ["", "mailto:", "xmpp:"] {
                let raw = format!("{scheme}{local}@{domain}");
                let doc = parse(&format!("x {raw}\n"));
                // The sole opening/closing underscores in this independent
                // matrix form emphasis before email text-node recognition.
                let wrapped = local == "_a" && domain == "a.b_";
                let semantic_scheme = if wrapped { "" } else { scheme };
                let local = if wrapped { "a" } else { local };
                let domain = if wrapped { "a.b" } else { domain };
                let end = domain.trim_end_matches('.');
                let valid = !local.is_empty()
                    && end.contains('.')
                    && !end.ends_with(['-', '_'])
                    && end.split('.').all(|p| !p.is_empty());
                assert_eq!(doc.links().len(), usize::from(valid), "{raw:?}");
                if valid {
                    let address = format!("{semantic_scheme}{local}@{end}");
                    assert_eq!(labels(&doc)[0].0, address);
                    assert_eq!(
                        labels(&doc)[0].1,
                        if semantic_scheme.is_empty() {
                            format!("mailto:{address}")
                        } else {
                            address
                        }
                    );
                }
                count += 1;
            }
        }
    }
    assert_eq!(count, 162);
    for (raw, expected) in [
        ("xmpp:a@b.c/res@node.test/other", "xmpp:a@b.c/res@node.test"),
        ("xmpp:a@b.c/res-more", "xmpp:a@b.c/res"),
        ("mailto:a@b.c/resource", "mailto:a@b.c"),
        ("hello@mail+xyz.example", ""),
        ("http:a@b.c", "a@b.c"),
        ("https:/a@b.c", "a@b.c"),
        ("prefixmailto:a@b.c", "a@b.c"),
        ("prefixxmpp:a@b.c/resource", "a@b.c"),
    ] {
        let doc = parse(raw);
        assert_eq!(labels(&doc).first().map_or("", |p| p.0), expected);
    }
}

#[test]
fn punctuation_parenthesis_and_entity_suffixes_have_independent_expectations() {
    for (tail, included) in [
        ("?!.,:*_~", ""),
        ("(x)))", "(x)"),
        ("(x))+ok", "(x))+ok"),
        ("a))", "a"),
        ("a))(", "a))("),
        ("(a)", "(a)"),
        ("&hl;", ""),
        ("&amp;", ""),
        ("&amp;x", "&amp;x"),
        ("&#123;", "&#123;"),
        ("&;", "&;"),
        ("&abc;&def;", ""),
        ("a&hl;?!", "a"),
        ("a&hl=x", "a&hl=x"),
    ] {
        let source = format!("www.a.test/{tail}\n");
        let doc = parse(&source);
        assert_eq!(
            labels(&doc)[0].0,
            format!("www.a.test/{included}"),
            "{source:?}"
        );
    }
    let doc = parse("www.a.test/x<https://b.test>\n");
    assert_eq!(
        labels(&doc),
        [
            ("www.a.test/x", "http://www.a.test/x".into()),
            ("https://b.test", "https://b.test".into())
        ]
    );
}

#[test]
fn unfinished_late_confirmation_and_rejection_publish_no_speculative_ids() {
    let prefix = format!("{} ", "x".repeat(BLOCK_LOOKAHEAD));
    for (body, tail, expected) in [
        ("a@b.c", "- ", None),
        ("a@b.c", "_ ", None),
        ("a@b.c", ". ", Some("a@b.c")),
        ("www.a.test/a))", "+ok ", Some("www.a.test/a))+ok")),
        ("www.a.test/a&hl", "; ", Some("www.a.test/a")),
        ("https://a.test/x", " ", Some("https://a.test/x")),
    ] {
        let mut doc = Document::new();
        doc.append(&prefix).unwrap();
        doc.append(body).unwrap();
        let block = doc.block_at(0).unwrap().0;
        let old = doc.tile_key(doc.tile_at(prefix.len()).unwrap()).unwrap();
        assert!(doc.links().is_empty());
        let update = doc.append(tail).unwrap();
        assert_eq!(doc.block_at(0).unwrap().0, block);
        assert_eq!(labels(&doc).first().map(|p| p.0), expected);
        if expected.is_some() {
            assert!(
                update
                    .dirty
                    .iter()
                    .any(|s| s.start <= prefix.len() && s.end > prefix.len())
            );
            assert_ne!(
                doc.tile_key(doc.tile_at(prefix.len()).unwrap()).unwrap(),
                old
            );
            assert_eq!(doc.link_info(LinkId(0)).unwrap().state, SyntaxState::Sealed);
            let info = *doc.link_info(LinkId(0)).unwrap();
            doc.append("[next](/x) ").unwrap();
            assert_eq!(doc.link_info(LinkId(0)), Some(&info));
            assert_eq!(doc.link_info(LinkId(1)).unwrap().kind, LinkKind::Link);
        }
        doc.finish();
        let finished = snapshot(&doc);
        doc.finish();
        assert_eq!(snapshot(&doc), finished);
    }
}

#[test]
fn existing_opaque_contexts_and_explicit_link_image_priority_are_preserved() {
    let source = "`www.a.test a@b.c` $www.a.test a@b.c$ <https://a.test> [www.a.test](https://target.test) ![a@b.c](/image) [www.a.test][r]\n\n[r]: /ref\n";
    let doc = parse(source);
    assert_eq!(doc.links().len(), 4);
    assert_eq!(
        labels(&doc)
            .iter()
            .map(|p| p.1.as_str())
            .collect::<Vec<_>>(),
        ["https://a.test", "https://target.test", "/image", "/ref"]
    );
    assert_eq!(doc.link_info(LinkId(2)).unwrap().kind, LinkKind::Image);
    for (source, count) in [
        ("www.a.test/`code`", 1),
        ("www.a.test/$math$", 1),
        ("www.a.test/[label](/target)", 2),
        ("\\_foo@b.c", 1),
        ("<https://a.test/a@b.c>", 1),
        ("[unfinished www.a.test", 0),
        ("```\nwww.a.test\n```\n", 0),
        ("    www.a.test\n", 0),
    ] {
        assert_eq!(parse(source).links().len(), count, "{source:?}");
    }
    let doc = parse("www.a.test/`code` $math$ [x](/y)");
    assert_eq!(doc.maths().len(), 1);
    assert!(
        doc.display_runs_in(Span::new(0, doc.source().len()))
            .unwrap()
            .any(|r| r.style.contains(Style::CODE))
    );
}

#[test]
fn newline_container_and_cell_boundaries_never_join_addresses() {
    for source in [
        "www.\ncom\n",
        "> www.\n> com\n",
        "- a@b.\n  c\n",
        "> a@b.\noutside\n",
        "www.\n\ncom\n",
        "a@b.\n# c\n",
    ] {
        assert!(parse(source).links().is_empty(), "{source:?}");
    }
    for source in [
        "> www.a.test\n> a@b.c\n",
        "- www.a.test\n  a@b.c\n",
        "www.a.test|a@b.c\n---|---\nwww.x.test|mailto:a@b.c\n",
        "old\nwww.a.test|a@b.c\n---|---\n",
        "old\nwww.a.test|a@b.c\nnot a table\n",
    ] {
        let whole = parse(source);
        assert!(!whole.links().is_empty(), "{source:?}");
        for split in (0..=source.len()).filter(|&i| source.is_char_boundary(i)) {
            let mut doc = Document::new();
            doc.append(&source[..split]).unwrap();
            doc.append(&source[split..]).unwrap();
            doc.finish();
            assert_eq!(
                snapshot(&doc),
                snapshot(&whole),
                "{source:?}, split {split}"
            );
        }
    }
}

#[test]
fn arbitrary_utf8_bytes_prefixes_eof_and_failures_are_chunk_invariant() {
    for source in [
        "中 *www.a.test/中😀(a))*.\r\n",
        "_a.b-c_d@a.b_ / mailto:a@b.c/\n",
        "foo_www.a.test/x&hl; www.a_b.c/d $a@b.c$\n",
        "www.a.test/x`code` ![www.b.test](/x) xmpp:a@b.c/r@d/next\n",
        "*foo_bar* http:/bad www.no-domain a@b..c &amp;\n",
    ] {
        let whole = parse(source);
        let mut bytewise = Utf8Document::new();
        for byte in source.bytes() {
            bytewise.append(&[byte]).unwrap();
        }
        bytewise.finish();
        assert_eq!(snapshot(bytewise.document()), snapshot(&whole));
        let mut doc = Document::new();
        for (at, ch) in source.char_indices() {
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            let mut prefix = Document::new();
            prefix.append(&source[..at + ch.len_utf8()]).unwrap();
            assert_eq!(snapshot(&doc), snapshot(&prefix));
            let mut bytes = Utf8Document::new();
            for byte in source[..at + ch.len_utf8()].bytes() {
                bytes.append(&[byte]).unwrap();
            }
            bytes.finish();
            prefix.finish();
            assert_eq!(snapshot(bytes.document()), snapshot(&prefix));
        }
        doc.finish();
        assert_eq!(snapshot(&doc), snapshot(&whole));
    }
}

const SIX: WorkLimit = WorkLimit {
    bytes: 6,
    queries: 1,
    objects: 1,
    tiles: 0,
    recovery: 0,
    dependencies: 0,
};
#[test]
fn raw_mapping_tiles_destinations_and_shared_budget() {
    let source = format!(
        "{} www.a.test/中😀?a&amp;x a+b@c.d mailto:x@y.z xmpp:x@y.z/r@t\n",
        "x".repeat(251)
    );
    let doc = parse(&source);
    for n in 0..doc.links().len() {
        let id = LinkId(n);
        let info = doc.link_info(id).unwrap();
        assert_eq!(info.source, info.label);
        let mut target = doc.decoded_link_destination(id).unwrap();
        let start = target.source_position();
        let mut raw = String::new();
        assert_eq!(
            target.advance(&mut Budget::new(WorkLimit::default())),
            Err(Completion::BudgetExhausted)
        );
        assert_eq!(target.source_position(), start);
        while target.source_position() < info.label.end {
            let mut budget = Budget::new(SIX);
            let old = target.source_position();
            let chunk = target.advance(&mut budget).unwrap().unwrap();
            assert_eq!(chunk.source.start, old);
            assert_eq!(doc.text(chunk.source), Some(chunk.text));
            raw.push_str(chunk.text);
            let mut display = doc.display_cursor(info.label).unwrap();
            assert_eq!(
                display.next(&doc, &mut budget).unwrap(),
                DisplayStep::BudgetExhausted
            );
            assert_eq!(display.source_position(), info.label.start);
        }
        assert_eq!(raw, doc.link_destination(id).unwrap());
    }
    let mut display = doc.display_cursor(Span::new(0, source.len())).unwrap();
    let mut text = String::new();
    while display.source_position() < source.len() {
        let old = display.source_position();
        let mut budget = Budget::new(SIX);
        let DisplayStep::Run(run) = display.next(&doc, &mut budget).unwrap() else {
            panic!("stalled");
        };
        assert_eq!(run.source.start, old);
        assert_eq!(doc.text(run.source), Some(run.text));
        text.push_str(run.text);
    }
    assert_eq!(text, source);
    for n in 0..doc.tile_count() {
        for run in doc.runs(TileId(n)).unwrap() {
            assert_eq!(doc.text(run.source), Some(run.text));
        }
    }
}

#[test]
fn million_byte_growth_sealing_and_history_have_linear_candidate_work() {
    for lead in ["www.a.test/", "https://a.test/", "a_", "xmpp:a@b.c/"] {
        let mut doc = Document::new();
        doc.append(lead).unwrap();
        doc.append(&"x".repeat(1_000_000)).unwrap();
        let block = doc.block_at(0).unwrap().0;
        for _ in 0..1000 {
            let before = doc.work();
            doc.append("x").unwrap();
            assert!(doc.work().bare_autolink_bytes - before.bare_autolink_bytes <= 1);
            assert_eq!(doc.work().inline_bytes - before.inline_bytes, 1);
        }
        let before = doc.work();
        doc.append(" \n").unwrap();
        assert_eq!(doc.block_at(0).unwrap().0, block);
        assert!(
            doc.work().bare_autolink_bytes - before.bare_autolink_bytes
                <= 8 * doc.source().len() as u64
        );
        assert_eq!(doc.work().inline_bytes - before.inline_bytes, 2);
        doc.finish();
        assert!(doc.work().bare_autolink_bytes <= 10 * doc.source().len() as u64);
    }
    let mut doc = Document::new();
    doc.append(&"> ".repeat(970)).unwrap();
    doc.append(&"x".repeat(1_000_000)).unwrap();
    doc.append(" www.a.test/x ").unwrap();
    let info = *doc.link_info(LinkId(0)).unwrap();
    for _ in 0..4096 {
        doc.append("a@b.c www.a.test/x \n").unwrap();
    }
    let before = doc.work();
    doc.append("tail ").unwrap();
    assert!(doc.work().bare_autolink_bytes - before.bare_autolink_bytes <= 32);
    assert_eq!(doc.link_info(LinkId(0)), Some(&info));
    doc.finish();
    assert!(doc.work().bare_autolink_bytes <= 10 * doc.source().len() as u64);
}

#[test]
fn late_long_header_confirmation_failure_and_eof_keep_object_order() {
    for old in ["", "old\n", "> old\n"] {
        for separator in ["---|---\n", "not a separator\n", ""] {
            let quote = if old.starts_with('>') { "> " } else { "" };
            let source = format!(
                "{old}{quote}www.a.test/{}|a@b.c\n{quote}{separator}",
                "x".repeat(3000)
            );
            let whole = parse(&source);
            let mut doc = Document::new();
            for chunk in source.as_bytes().chunks(17) {
                doc.append(std::str::from_utf8(chunk).unwrap()).unwrap();
            }
            doc.finish();
            assert_eq!(snapshot(&doc), snapshot(&whole));
            assert_eq!(
                doc.tables().len(),
                usize::from(separator.starts_with("---")),
                "{old:?} {separator:?}"
            );
            let objects: Vec<_> = doc.links_in(Span::new(0, source.len())).collect();
            assert!(
                objects
                    .windows(2)
                    .all(|p| p[0].1.source.end <= p[1].1.source.start)
            );
            assert!(doc.work().bare_autolink_bytes <= 10 * source.len() as u64);
        }
    }
}

#[test]
fn a_late_definition_does_not_visit_old_bare_or_reference_objects() {
    let mut doc = Document::new();
    doc.append(&"[r] www.a.test a@b.c \n".repeat(4096)).unwrap();
    let links = doc.links().to_vec();
    let infos: Vec<_> = (0..links.len())
        .map(|n| *doc.link_info(LinkId(n)).unwrap())
        .collect();
    let before = doc.work();
    doc.append("\n[r]: /winner\n").unwrap();
    let after = doc.work();
    assert_eq!(after.bare_autolink_bytes, before.bare_autolink_bytes);
    assert_eq!(after.bare_eligibility_bytes, before.bare_eligibility_bytes);
    assert_eq!(doc.links(), links);
    for (n, info) in infos.iter().enumerate() {
        assert_eq!(doc.link_info(LinkId(n)), Some(info));
    }
    assert_eq!(doc.link_url(LinkId(0)).as_deref(), Some("/winner"));
    doc.finish();
}

#[test]
fn adversarial_suffix_summaries_and_incomplete_entities_are_linear() {
    for (tail, kept) in [
        ("&a;".repeat(100_000), "www.a.test/"),
        (")?_*~".repeat(100_000), "www.a.test/"),
        ("(x))".repeat(100_000), ""),
    ] {
        let mut doc = Document::new();
        doc.append("www.a.test/").unwrap();
        doc.append(&tail).unwrap();
        for _ in 0..1000 {
            let before = doc.work();
            doc.append(".").unwrap();
            assert_eq!(
                doc.work().bare_autolink_bytes - before.bare_autolink_bytes,
                1
            );
            assert!(doc.links().is_empty());
        }
        doc.finish();
        if !kept.is_empty() {
            assert_eq!(labels(&doc)[0].0, kept);
        }
        assert!(doc.work().bare_autolink_bytes <= 10 * doc.source().len() as u64);
        assert!(doc.work().bare_eligibility_bytes <= 10 * doc.source().len() as u64);
    }
    for raw in ["www.a.test/x&", "www.a.test/x&hl", "www.a.test/x&hl=x"] {
        let doc = parse(raw);
        assert_eq!(labels(&doc)[0].0, raw);
    }
}

#[test]
fn review_email_uses_emphasis_text_nodes_and_preserves_literal_local_underscores() {
    for (source, label, visible, style) in [
        (
            "_foo@bar.com_",
            Some("foo@bar.com"),
            "foo@bar.com",
            Style::EMPHASIS,
        ),
        (
            "__foo@bar.com__",
            Some("foo@bar.com"),
            "foo@bar.com",
            Style::STRONG,
        ),
        (
            "*foo@bar.com*",
            Some("foo@bar.com"),
            "foo@bar.com",
            Style::EMPHASIS,
        ),
        (
            "~~foo@bar.com~~",
            Some("foo@bar.com"),
            "foo@bar.com",
            Style::STRIKE,
        ),
        (
            "_foo@bar.com",
            Some("_foo@bar.com"),
            "_foo@bar.com",
            Style::default(),
        ),
        (
            "__foo@bar.com",
            Some("__foo@bar.com"),
            "__foo@bar.com",
            Style::default(),
        ),
        (
            "foo_@bar.com",
            Some("foo_@bar.com"),
            "foo_@bar.com",
            Style::default(),
        ),
        ("foo@bar.com_", None, "foo@bar.com_", Style::default()),
        ("foo@bar.com__", None, "foo@bar.com__", Style::default()),
        ("_foo@bar.com-_", None, "foo@bar.com-", Style::EMPHASIS),
        (
            "__foo@bar.com_",
            Some("foo@bar.com"),
            "_foo@bar.com",
            Style::EMPHASIS,
        ),
        (
            "_foo@bar.com__",
            Some("foo@bar.com"),
            "foo@bar.com_",
            Style::EMPHASIS,
        ),
        ("_a_@bar.com", None, "a@bar.com", Style::default()),
        ("_@bar.com_", None, "@bar.com", Style::EMPHASIS),
        (
            "_@bar.com",
            Some("_@bar.com"),
            "_@bar.com",
            Style::default(),
        ),
        (
            "_foo@bar.com_foo",
            Some("_foo@bar.com_foo"),
            "_foo@bar.com_foo",
            Style::default(),
        ),
    ] {
        let doc = parse(source);
        assert_eq!(labels(&doc).first().map(|p| p.0), label, "{source:?}");
        let shown: String = doc
            .display_runs_in(Span::new(0, source.len()))
            .unwrap()
            .filter(|r| !r.style.contains(Style::HIDDEN))
            .map(|r| r.text)
            .collect();
        assert_eq!(shown, visible, "{source:?}");
        if let Some(label) = label {
            assert_eq!(labels(&doc)[0].1, format!("mailto:{label}"));
            let info = doc.link_info(LinkId(0)).unwrap();
            for run in doc.display_runs_in(info.label).unwrap() {
                assert!(!run.style.contains(Style::HIDDEN), "{source:?}");
                assert!(run.style.contains(style), "{source:?}");
            }
        }
    }
}

#[test]
fn review_url_domain_validation_precedes_path_punctuation_trimming() {
    for (source, label) in [
        ("www.example.com_", None),
        ("https://example.com_", None),
        ("http://example.com__", None),
        ("www.example.com_..", None),
        ("https://example.com_.", None),
        ("https://example.com_/path_", None),
        ("_www.example.com_", None),
        ("__https://example.com__", None),
        ("www.example.com/a_", Some("www.example.com/a")),
        ("https://example.com/a_", Some("https://example.com/a")),
        ("www.ex_ample.com", None),
        ("www.ex_ample.a.com", Some("www.ex_ample.a.com")),
    ] {
        let doc = parse(source);
        assert_eq!(labels(&doc).first().map(|p| p.0), label, "{source:?}");
    }
}

#[test]
fn review_email_late_closers_and_real_sequence_boundaries() {
    for (source, expected) in [
        ("_foo@bar.com rest_", vec!["foo@bar.com"]),
        ("_foo@bar.com\nend_", vec!["foo@bar.com"]),
        ("__foo@bar.com rest__", vec!["foo@bar.com"]),
        ("_word foo@bar.com_", vec!["foo@bar.com"]),
        ("_foo@bar.com\n\nend_", vec!["_foo@bar.com"]),
        ("_foo@bar.com\n# end_", vec!["_foo@bar.com"]),
        ("> _foo@bar.com\n> end_", vec!["foo@bar.com"]),
        ("> _foo@bar.com\nend_", vec!["foo@bar.com"]),
        ("- __foo@bar.com\n  end__", vec!["foo@bar.com"]),
        ("_foo@bar.com [x](/x) rest_", vec!["foo@bar.com", "x"]),
        ("_@bar.com rest_", vec![]),
        ("a._foo@bar.com rest_", vec!["foo@bar.com"]),
        (
            "_foo@bar.com old\na@b.c|x\n---|---\n",
            vec!["_foo@bar.com", "a@b.c"],
        ),
    ] {
        let doc = parse(source);
        let ordered: Vec<_> = doc
            .links_in(Span::new(0, source.len()))
            .map(|(_, info)| doc.text(info.label).unwrap())
            .collect();
        assert_eq!(ordered, expected, "{source:?}");
        for (id, info) in doc.links_in(Span::new(0, source.len())) {
            assert_eq!(
                doc.next_drawable(info.source.start),
                Some(info.source.start)
            );
            if doc.text(info.label).unwrap().contains('@') {
                assert_eq!(
                    doc.link_url(id).unwrap(),
                    format!("mailto:{}", doc.text(info.label).unwrap())
                );
            }
        }
    }
}

#[test]
fn review_all_byte_splits_live_prefixes_and_eof_cover_mixed_boundaries() {
    for tail in [
        "_foo@bar.com_",
        "__foo@bar.com__",
        "*foo@bar.com*",
        "~~foo@bar.com~~",
        "_foo@bar.com",
        "__foo@bar.com",
        "foo_@bar.com",
        "foo@bar.com_",
        "_foo@bar.com-_",
        "__foo@bar.com_",
        "_foo@bar.com__",
        "_a_@bar.com",
        "_@bar.com_",
        "_@bar.com",
        "_foo@bar.com_foo",
        "_foo@bar.com rest_",
        "_foo@bar.com\nend_",
        "__foo@bar.com rest__",
        "_word foo@bar.com_",
        "_foo@bar.com [x](/x) rest_",
        "a._foo@bar.com rest_",
        "_foo@bar.com\n\nend_",
        "_foo@bar.com\n# end_",
        "_foo@bar.com 中😀_",
        "www.example.com_",
        "https://example.com_",
        "https://example.com/a_",
        "_www.example.com_",
        "www.ex_ample.com",
        "www.ex_ample.a.com",
        "_mailto:foo@bar.com_",
        "__mailto:foo@bar.com__",
        "_xmpp:foo@bar.com_",
        "_mailto:foo@bar.com rest_",
        "__xmpp:foo@bar.com rest__",
        "_mailto:foo@bar.com\nrest_",
        "_xmpp:foo@bar.com/resource_",
        "_xmpp:foo@bar.com/resource\nrest_",
        "mailto:foo@bar.com_",
        "xmpp:foo@bar.com_",
        "mailto:",
        "xmpp:",
        "_mailto:",
        "mailto:_foo@bar.com_",
        "xmpp:_foo@bar.com_",
    ] {
        let lead = format!("{} ", "p".repeat(BLOCK_LOOKAHEAD));
        let source = format!("{lead}{tail}");
        let whole = parse(&source);
        for split in 0..=source.len() {
            let mut doc = Utf8Document::new();
            doc.append(&source.as_bytes()[..split]).unwrap();
            doc.append(&source.as_bytes()[split..]).unwrap();
            doc.finish();
            assert_eq!(
                snapshot(doc.document()),
                snapshot(&whole),
                "{tail:?}, split {split}"
            );
        }
        let mut doc = Document::new();
        doc.append(&lead).unwrap();
        for (at, ch) in tail.char_indices() {
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            let prefix = &source[..lead.len() + at + ch.len_utf8()];
            let mut expected = Document::new();
            expected.append(prefix).unwrap();
            assert_eq!(snapshot(&doc), snapshot(&expected), "{prefix:?}");
            let mut bytes = Utf8Document::new();
            for byte in prefix.bytes() {
                bytes.append(&[byte]).unwrap();
            }
            bytes.finish();
            expected.finish();
            assert_eq!(
                snapshot(bytes.document()),
                snapshot(&expected),
                "EOF {prefix:?}"
            );
        }
        doc.finish();
        assert_eq!(snapshot(&doc), snapshot(&whole));
    }
    for source in [
        "> _foo@bar.com\n> end_",
        "> _foo@bar.com\nend_",
        "- __foo@bar.com\n  end__",
    ] {
        let whole = parse(source);
        for split in 0..=source.len() {
            let mut d = Utf8Document::new();
            d.append(&source.as_bytes()[..split]).unwrap();
            d.append(&source.as_bytes()[split..]).unwrap();
            d.finish();
            assert_eq!(snapshot(d.document()), snapshot(&whole));
        }
    }
}

#[test]
fn review_late_publication_keeps_stable_ids_order_dirty_mapping_and_budgets() {
    let mut doc = Document::new();
    doc.append(&format!(
        "{} _foo@bar.com [x](/x) rest",
        "p".repeat(BLOCK_LOOKAHEAD)
    ))
    .unwrap();
    assert_eq!(labels(&doc), [("x", "/x".into())]);
    let first = *doc.link_info(LinkId(0)).unwrap();
    let email_at = doc.source().find("_foo").unwrap();
    let tile = doc.tile_at(email_at).unwrap();
    let key = doc.tile_key(tile).unwrap();
    let mut stale = doc
        .display_cursor(Span::new(email_at, email_at + 4))
        .unwrap();
    let update = doc.append("_ ").unwrap();
    assert_eq!(doc.link_info(LinkId(0)), Some(&first));
    assert_eq!(doc.link_destination(LinkId(1)), Some("foo@bar.com"));
    assert_eq!(
        doc.link_url(LinkId(1)).as_deref(),
        Some("mailto:foo@bar.com")
    );
    assert!(
        update
            .dirty
            .iter()
            .any(|s| s.start <= email_at + 1 && s.end >= email_at + 12)
    );
    assert_ne!(doc.tile_key(tile).unwrap(), key);
    let mut zero = Budget::new(WorkLimit::default());
    assert_eq!(
        stale.next(&doc, &mut zero),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(zero.used(), WorkLimit::default());
    let info = *doc.link_info(LinkId(1)).unwrap();
    doc.append("[z](/z) more ").unwrap();
    assert_eq!(doc.link_info(LinkId(0)), Some(&first));
    assert_eq!(doc.link_info(LinkId(1)), Some(&info));
    assert_eq!(
        doc.links_in(Span::new(0, doc.source().len()))
            .map(|p| p.0)
            .collect::<Vec<_>>(),
        [LinkId(1), LinkId(0), LinkId(2)]
    );
    let mut cursor = doc.decoded_link_destination(LinkId(1)).unwrap();
    let mut raw = String::new();
    while cursor.source_position() < info.label.end {
        let mut budget = Budget::new(SIX);
        let chunk = cursor.advance(&mut budget).unwrap().unwrap();
        assert_eq!(doc.text(chunk.source), Some(chunk.text));
        raw.push_str(chunk.text);
        let mut peer = doc.display_cursor(info.label).unwrap();
        assert_eq!(
            peer.next(&doc, &mut budget).unwrap(),
            DisplayStep::BudgetExhausted
        );
    }
    assert_eq!(raw, "foo@bar.com");
    doc.finish();
    #[derive(Default)]
    struct Sink(Vec<LinkId>);
    impl LayoutSink for Sink {
        fn begin_tile(&mut self, _: TileId) {}
        fn text(&mut self, _: RenderRun<'_>) {}
        fn object(&mut self, id: LinkId, _: &LinkInfo) {
            self.0.push(id);
        }
        fn end_tile(&mut self, _: TileId) -> u64 {
            0
        }
    }
    let mut heights = HeightIndex::default();
    heights.rebuild(&doc).unwrap();
    let mut pass = LayoutPass::new(&doc, &heights, TileId(0), 1, true).unwrap();
    let mut sink = Sink::default();
    for _ in 0..10000 {
        let mut budget = Budget::new(WorkLimit {
            bytes: 6,
            queries: 2,
            objects: 1,
            tiles: 1,
            recovery: 1,
            dependencies: 0,
        });
        if pass
            .advance(&doc, &mut heights, &mut budget, &mut sink)
            .unwrap()
            == Completion::EndOfDocument
        {
            break;
        }
    }
    assert_eq!(sink.0, [LinkId(1), LinkId(0), LinkId(2)]);
    assert_eq!(pass.source_position(), doc.source().len());
}

#[test]
fn review_pending_opener_owners_and_late_source_index_have_cumulative_bounds() {
    let mut doc = Document::new();
    doc.append(&format!("{} ", "p".repeat(BLOCK_LOOKAHEAD)))
        .unwrap();
    for n in 0..4096 {
        doc.append(&format!("_a{n}@b.c ")).unwrap();
    }
    assert!(doc.links().is_empty());
    for _ in 0..4096 {
        let before = doc.work();
        doc.append("z_ ").unwrap();
        assert!(doc.work().bare_autolink_bytes - before.bare_autolink_bytes <= 64);
        assert!(
            doc.work().tree_visits - before.tree_visits
                <= 512 * (doc.source().len().ilog2() as u64 + 1)
        );
    }
    assert_eq!(doc.links().len(), 4096);
    let ordered: Vec<_> = doc
        .links_in(Span::new(0, doc.source().len()))
        .map(|(id, _)| doc.link_destination(id).unwrap())
        .collect();
    for (n, actual) in ordered.iter().enumerate() {
        assert_eq!(*actual, format!("a{n}@b.c"));
    }
    let infos: Vec<_> = (0..doc.links().len())
        .map(|n| *doc.link_info(LinkId(n)).unwrap())
        .collect();
    doc.append(&"x".repeat(1_000_000)).unwrap();
    for _ in 0..1000 {
        let before = doc.work();
        doc.append("x").unwrap();
        assert_eq!(
            doc.work().bare_autolink_bytes - before.bare_autolink_bytes,
            1
        );
    }
    doc.finish();
    for (n, info) in infos.iter().enumerate() {
        assert_eq!(doc.link_info(LinkId(n)), Some(info));
    }
    assert!(doc.work().bare_autolink_bytes <= 16 * doc.source().len() as u64);
    assert!(doc.work().bare_eligibility_bytes <= 16 * doc.source().len() as u64);

    let mut d = Document::new();
    d.append(&format!("{} _foo@bar.com ", "p".repeat(BLOCK_LOOKAHEAD)))
        .unwrap();
    d.append(&"[r] a@b.c ".repeat(4096)).unwrap();
    let count = d.links().len();
    let old = *d.link_info(LinkId(0)).unwrap();
    let before = d.work();
    d.append("tail_ ").unwrap();
    assert_eq!(d.links().len(), count + 1);
    assert_eq!(d.link_info(LinkId(0)), Some(&old));
    assert_eq!(d.link_destination(LinkId(count)), Some("foo@bar.com"));
    assert!(d.work().bare_autolink_bytes - before.bare_autolink_bytes <= 64);
    assert!(
        d.work().tree_visits - before.tree_visits <= 512 * (d.source().len().ilog2() as u64 + 1)
    );
    assert_eq!(d.work().label_bytes, before.label_bytes);
    d.finish();
}

#[test]
fn review_inherited_header_pairs_settle_email_only_after_header_decision() {
    for scheme in ["", "mailto:", "xmpp:"] {
        for ending in ["\n", " | b\nnot delimiter\n", " | b\n- | -\n"] {
            let address = format!("{scheme}foo@bar.com");
            let target = if scheme.is_empty() {
                format!("mailto:{address}")
            } else {
                address.clone()
            };
            let source = format!("_prior\n{address}_ {}{ending}", "x".repeat(1100));
            let whole = parse(&source);
            let check = |d: &Document| {
                if d.tables().is_empty() {
                    assert_eq!(labels(d), [(address.as_str(), target.clone())]);
                } else {
                    assert_eq!(d.tables().len(), 1);
                    assert!(d.links().is_empty());
                }
                assert!(d.work().bare_autolink_bytes <= 16 * d.source().len() as u64);
            };
            check(&whole);
            if ending != " | b\n- | -\n" {
                assert!(whole.tables().is_empty());
            }
            for split in 0..=source.len() {
                let mut d = Document::new();
                d.append(&source[..split]).unwrap();
                d.append(&source[split..]).unwrap();
                d.finish();
                check(&d);
                assert_eq!(snapshot(&d), snapshot(&whole), "{ending:?} {split}");
            }
            let mut live = Document::new();
            for (at, ch) in source.char_indices() {
                live.append(ch.encode_utf8(&mut [0; 4])).unwrap();
                let prefix = &source[..at + 1];
                let mut bulk = Document::new();
                bulk.append(prefix).unwrap();
                assert_eq!(snapshot(&live), snapshot(&bulk));
                let mut bytes = Utf8Document::new();
                for byte in prefix.bytes() {
                    bytes.append(&[byte]).unwrap();
                }
                bytes.finish();
                bulk.finish();
                assert_eq!(snapshot(bytes.document()), snapshot(&bulk));
                if prefix.contains("foo@bar.com_") {
                    check(&bulk);
                }
            }
            live.finish();
            assert_eq!(snapshot(&live), snapshot(&whole));
        }
    }
}

#[test]
fn review_protocols_share_email_text_node_and_late_marker_rules() {
    for (source, label, url) in [
        (
            "_mailto:foo@bar.com_",
            Some("mailto:foo@bar.com"),
            Some("mailto:foo@bar.com"),
        ),
        (
            "__mailto:foo@bar.com__",
            Some("mailto:foo@bar.com"),
            Some("mailto:foo@bar.com"),
        ),
        (
            "_xmpp:foo@bar.com_",
            Some("xmpp:foo@bar.com"),
            Some("xmpp:foo@bar.com"),
        ),
        (
            "_mailto:foo@bar.com rest_",
            Some("mailto:foo@bar.com"),
            Some("mailto:foo@bar.com"),
        ),
        (
            "__xmpp:foo@bar.com rest__",
            Some("xmpp:foo@bar.com"),
            Some("xmpp:foo@bar.com"),
        ),
        (
            "_mailto:foo@bar.com\nrest_",
            Some("mailto:foo@bar.com"),
            Some("mailto:foo@bar.com"),
        ),
        (
            "_xmpp:foo@bar.com/resource_",
            Some("xmpp:foo@bar.com/resource"),
            Some("xmpp:foo@bar.com/resource"),
        ),
        (
            "_xmpp:foo@bar.com/resource\nrest_",
            Some("xmpp:foo@bar.com/resource"),
            Some("xmpp:foo@bar.com/resource"),
        ),
        ("mailto:foo@bar.com_", None, None),
        ("xmpp:foo@bar.com_", None, None),
        ("_mailto:foo@bar.com-_", None, None),
        (
            "mailto:_foo@bar.com_",
            Some("foo@bar.com"),
            Some("mailto:foo@bar.com"),
        ),
        (
            "xmpp:_foo@bar.com_",
            Some("foo@bar.com"),
            Some("mailto:foo@bar.com"),
        ),
    ] {
        let doc = parse(source);
        assert_eq!(labels(&doc).first().map(|p| p.0), label, "{source:?}");
        assert_eq!(doc.link_url(LinkId(0)).as_deref(), url, "{source:?}");
        if source.starts_with('_') && label.is_some() {
            let info = doc.link_info(LinkId(0)).unwrap();
            let style = if source.starts_with("__") {
                Style::STRONG
            } else {
                Style::EMPHASIS
            };
            assert!(
                doc.display_runs_in(info.label)
                    .unwrap()
                    .all(|r| r.style.contains(style) && !r.style.contains(Style::HIDDEN)),
                "{source:?}"
            );
        }
    }
}

#[test]
fn review_basic_family_boundaries_and_failed_email_recovery() {
    // Handwritten expectations: fixed 6.9 domain/resource rules and official
    // .13's protocol word boundary. Do not inherit its over-rewind of a literal
    // prefix before a scheme or its second-@ restart inside an XMPP resource.
    for (source, label, url) in [
        ("www.foo", "www.foo", "http://www.foo"),
        ("http://foo", "", ""),
        ("https://foo", "", ""),
        ("prefixmailto:a@b.c", "a@b.c", "mailto:a@b.c"),
        ("prefixxmpp:a@b.c/resource", "a@b.c", "mailto:a@b.c"),
        ("0mailto:a@b.c", "a@b.c", "mailto:a@b.c"),
        ("prefix_mailto:a@b.c", "mailto:a@b.c", "mailto:a@b.c"),
        ("prefix-xmpp:a@b.c", "xmpp:a@b.c", "xmpp:a@b.c"),
        ("(mailto:a@b.c)", "mailto:a@b.c", "mailto:a@b.c"),
        ("xmpp:a@b.c/", "", ""),
        ("xmpp:a@b.c/.", "", ""),
        ("xmpp:a@b.c/..", "", ""),
        ("xmpp:a@b.c/r..", "xmpp:a@b.c/r", "xmpp:a@b.c/r"),
        (
            "xmpp:a@b.c/r@d.e/bin",
            "xmpp:a@b.c/r@d.e",
            "xmpp:a@b.c/r@d.e",
        ),
        ("xmpp:a@b.c/r2", "xmpp:a@b.c/r2", "xmpp:a@b.c/r2"),
        ("a@b@c.d", "b@c.d", "mailto:b@c.d"),
        ("a@@c.d", "", ""),
        ("@a@b.c", "a@b.c", "mailto:a@b.c"),
        ("a@b.c_@d.e", "b.c_@d.e", "mailto:b.c_@d.e"),
        ("mailto:a@b@c.d", "b@c.d", "mailto:b@c.d"),
        ("xmpp:a@b@c.d/resource", "b@c.d", "mailto:b@c.d"),
        ("_a@b@c.d rest_", "b@c.d", "mailto:b@c.d"),
    ] {
        let whole = parse(source);
        assert_eq!(
            labels(&whole).first().map_or("", |p| p.0),
            label,
            "{source:?}"
        );
        assert_eq!(
            whole.link_url(LinkId(0)).as_deref().unwrap_or(""),
            url,
            "{source:?}"
        );
        for split in 0..=source.len() {
            let mut bytes = Utf8Document::new();
            bytes.append(&source.as_bytes()[..split]).unwrap();
            bytes.append(&source.as_bytes()[split..]).unwrap();
            bytes.finish();
            assert_eq!(
                snapshot(bytes.document()),
                snapshot(&whole),
                "{source:?} {split}"
            );
        }
        let lead = format!("{} ", "p".repeat(BLOCK_LOOKAHEAD));
        let mut live = Document::new();
        live.append(&lead).unwrap();
        for end in 1..=source.len() {
            live.append(&source[end - 1..end]).unwrap();
            let mut bulk = Document::new();
            bulk.append(&format!("{lead}{}", &source[..end])).unwrap();
            assert_eq!(snapshot(&live), snapshot(&bulk));
            let mut bytes = Utf8Document::new();
            for byte in bulk.source().bytes() {
                bytes.append(&[byte]).unwrap();
            }
            bytes.finish();
            bulk.finish();
            assert_eq!(snapshot(bytes.document()), snapshot(&bulk));
        }
        live.finish();
        assert_eq!(labels(&live), labels(&whole));
    }

    // Rebase a failed local/domain at each new @ using fixed state. No old
    // prefix can be rescanned while this unbounded token is still growing.
    let mut doc = Document::new();
    doc.append(&format!("{} ", "p".repeat(BLOCK_LOOKAHEAD)))
        .unwrap();
    doc.append(&"a@".repeat(100_000)).unwrap();
    for _ in 0..1000 {
        let before = doc.work();
        doc.append("a@").unwrap();
        assert_eq!(
            doc.work().bare_autolink_bytes - before.bare_autolink_bytes,
            2
        );
        assert!(doc.links().is_empty());
    }
    doc.append("b.c ").unwrap();
    assert_eq!(labels(&doc), [("a@b.c", "mailto:a@b.c".into())]);
    doc.finish();
    assert!(doc.work().bare_autolink_bytes <= 16 * doc.source().len() as u64);
    assert!(doc.work().bare_eligibility_bytes <= 16 * doc.source().len() as u64);
}

#[test]
fn failed_web_candidates_release_internal_addresses_without_recursive_rescans() {
    for (source, label, url) in [
        (
            "http://foo_bar@bar.com",
            "foo_bar@bar.com",
            "mailto:foo_bar@bar.com",
        ),
        ("https://foo_bar/path/a@b.c", "a@b.c", "mailto:a@b.c"),
        ("www._a@b.c", "www._a@b.c", "mailto:www._a@b.c"),
        ("http://a._www.c", "www.c", "http://www.c"),
        ("http://a._www.c/a@b.c", "www.c/a@b.c", "http://www.c/a@b.c"),
        ("https://a_/(www.foo)", "www.foo", "http://www.foo"),
        ("https://a_/?mailto:a@b.c", "mailto:a@b.c", "mailto:a@b.c"),
        (
            "http://_foo@bar.com rest_",
            "foo@bar.com",
            "mailto:foo@bar.com",
        ),
        ("http://a._www.c_ rest_", "", ""),
        (
            "https://a.b/a@b.c",
            "https://a.b/a@b.c",
            "https://a.b/a@b.c",
        ),
        (
            "http://a_/(https://x.y/a))",
            "https://x.y/a",
            "https://x.y/a",
        ),
        ("http://a._www.b_._www.c", "www.c", "http://www.c"),
    ] {
        let whole = parse(source);
        assert_eq!(
            labels(&whole).first().map_or("", |p| p.0),
            label,
            "{source:?}"
        );
        assert_eq!(
            whole.link_url(LinkId(0)).as_deref().unwrap_or(""),
            url,
            "{source:?}"
        );
        assert_eq!(whole.links().len(), usize::from(!label.is_empty()));
        for split in 0..=source.len() {
            let mut bytes = Utf8Document::new();
            bytes.append(&source.as_bytes()[..split]).unwrap();
            bytes.append(&source.as_bytes()[split..]).unwrap();
            bytes.finish();
            assert_eq!(snapshot(bytes.document()), snapshot(&whole));
        }
        let lead = format!("{} ", "p".repeat(BLOCK_LOOKAHEAD));
        let mut live = Document::new();
        live.append(&lead).unwrap();
        for end in 1..=source.len() {
            live.append(&source[end - 1..end]).unwrap();
            let mut bulk = Document::new();
            bulk.append(&format!("{lead}{}", &source[..end])).unwrap();
            assert_eq!(snapshot(&live), snapshot(&bulk));
            bulk.finish();
            let mut bytes = Utf8Document::new();
            for byte in bulk.source().bytes() {
                bytes.append(&[byte]).unwrap();
            }
            bytes.finish();
            assert_eq!(snapshot(bytes.document()), snapshot(&bulk));
        }
        live.finish();
        assert_eq!(labels(&live), labels(&whole));
        assert!(live.work().bare_autolink_bytes <= 40 * live.source().len() as u64);
    }
    let mut doc = Document::new();
    doc.append(&format!("{} ", "p".repeat(BLOCK_LOOKAHEAD)))
        .unwrap();
    doc.append(&"http://a_/(".repeat(4096)).unwrap();
    doc.append("www.foo/").unwrap();
    for _ in 0..1000 {
        let before = doc.work();
        doc.append("x").unwrap();
        assert_eq!(
            doc.work().bare_autolink_bytes - before.bare_autolink_bytes,
            1
        );
        assert!(doc.links().is_empty());
    }
    doc.append(" ").unwrap();
    assert_eq!(
        labels(&doc),
        [(
            doc.text(doc.link_info(LinkId(0)).unwrap().label).unwrap(),
            format!("http://www.foo/{}", "x".repeat(1000))
        )]
    );
    assert_eq!(doc.links().len(), 1);
    assert_eq!(
        doc.text(doc.link_info(LinkId(0)).unwrap().label).unwrap(),
        format!("www.foo/{}", "x".repeat(1000))
    );
    doc.finish();
    assert!(doc.work().bare_autolink_bytes <= 40 * doc.source().len() as u64);
    assert!(doc.work().bare_eligibility_bytes <= 40 * doc.source().len() as u64);
}

#[test]
fn failed_web_recovery_family_matrix_and_valid_outer_suppression() {
    // Independent GFM 6.9 expectations; the same inner syntax must work in
    // every failed Web family, and be suppressed by every valid outer family.
    for (inner, label, target) in [
        ("a@b.c", "a@b.c", "mailto:a@b.c"),
        ("a@b@c.d", "b@c.d", "mailto:b@c.d"),
        ("a@@c.d", "", ""),
        ("www.a.b", "www.a.b", "http://www.a.b"),
        ("www.a_b.c", "", ""),
        ("www.a_b.c.d", "www.a_b.c.d", "http://www.a_b.c.d"),
        (
            "https://a.b/a@b.c",
            "https://a.b/a@b.c",
            "https://a.b/a@b.c",
        ),
        ("https://a_b.c/a@b.c", "a@b.c", "mailto:a@b.c"),
        ("mailto:a@b.c", "mailto:a@b.c", "mailto:a@b.c"),
        ("xmpp:a@b.c/r@d.e", "xmpp:a@b.c/r@d.e", "xmpp:a@b.c/r@d.e"),
        ("xmpp:a@b.c/", "", ""),
        ("xmpp:a@b@c.d/r", "b@c.d", "mailto:b@c.d"),
        ("prefixmailto:a@b.c", "a@b.c", "mailto:a@b.c"),
        ("http:/a@b.c", "a@b.c", "mailto:a@b.c"),
        ("http://a._www.b_._www.c", "www.c", "http://www.c"),
    ] {
        for outer in ["http://bad_/(", "https://bad_/?(", "www.bad_/("] {
            let source = format!("{outer}{inner}");
            let doc = parse(&source);
            let expected = if label.is_empty() {
                vec![]
            } else {
                vec![(label, target.to_owned())]
            };
            assert_eq!(labels(&doc), expected, "{source:?}");
        }
        // A final slash is plain and keeps all of the inner text in the URL.
        for outer in ["http://a.b/", "https://例.子/", "www.a.b/"] {
            let source = format!("{outer}{inner}/");
            let doc = parse(&source);
            let target = if outer.starts_with("www.") {
                format!("http://{source}")
            } else {
                source.clone()
            };
            assert_eq!(labels(&doc), [(source.as_str(), target)], "{source:?}");
        }
    }
    let source = "http://bad_/(a@b.c)/?mailto:d@e.f/(http://bad_/x@y.z)/www.a_b.c http://bad_/q@r.s https://a._www.t/u@v.w";
    assert_eq!(
        labels(&parse(source)),
        [
            ("a@b.c", "mailto:a@b.c".into()),
            ("mailto:d@e.f", "mailto:d@e.f".into()),
            ("x@y.z", "mailto:x@y.z".into()),
            ("q@r.s", "mailto:q@r.s".into()),
            ("www.t/u@v.w", "http://www.t/u@v.w".into()),
        ]
    );
}

#[test]
fn failed_web_recovery_preserves_opaque_and_ordinary_inline_priority() {
    for (source, expected, images) in [
        ("`http://bad_/a@b.c`", vec![], 0),
        ("$http://bad_/a@b.c$", vec![], 0),
        (
            "[http://bad_/a@b.c](/x)",
            vec![("http://bad_/a@b.c", "/x")],
            0,
        ),
        (
            "![http://bad_/a@b.c](/img)",
            vec![("http://bad_/a@b.c", "/img")],
            1,
        ),
        ("http://bad_/[a@b.c](/x)", vec![("a@b.c", "/x")], 0),
        (
            "http://bad_/`a@b.c`/d@e.f",
            vec![("d@e.f", "mailto:d@e.f")],
            0,
        ),
        (
            "http://bad_/$a@b.c$/d@e.f",
            vec![("d@e.f", "mailto:d@e.f")],
            0,
        ),
        ("www.bad_/&amp;a@b.c", vec![("a@b.c", "mailto:a@b.c")], 0),
        (
            "<http://bad_/a@b.c>",
            vec![("http://bad_/a@b.c", "http://bad_/a@b.c")],
            0,
        ),
        (
            "http://bad_/a@b.c *word* &amp;",
            vec![("a@b.c", "mailto:a@b.c")],
            0,
        ),
        (
            "[r](http://bad_/a@b.c)",
            vec![("r", "http://bad_/a@b.c")],
            0,
        ),
        (
            "![r](http://bad_/a@b.c)",
            vec![("r", "http://bad_/a@b.c")],
            1,
        ),
    ] {
        let whole = parse(source);
        assert_eq!(
            labels(&whole),
            expected
                .into_iter()
                .map(|(a, b)| (a, b.into()))
                .collect::<Vec<_>>(),
            "{source:?}"
        );
        assert_eq!(
            whole
                .links_in(Span::new(0, source.len()))
                .filter(|(_, info)| info.kind == LinkKind::Image)
                .count(),
            images,
            "{source:?}"
        );
        for split in 0..=source.len() {
            let mut bytes = Utf8Document::new();
            bytes.append(&source.as_bytes()[..split]).unwrap();
            bytes.append(&source.as_bytes()[split..]).unwrap();
            bytes.finish();
            assert_eq!(
                snapshot(bytes.document()),
                snapshot(&whole),
                "{source:?} {split}"
            );
        }
    }
}

#[test]
fn failed_web_recovery_utf8_prefixes_eof_and_real_container_boundaries() {
    for (source, expected) in [
        ("http://_foo@bar.com rest_", vec!["foo@bar.com"]),
        ("http://_foo@bar.com\nend_", vec!["foo@bar.com"]),
        ("http://_foo@bar.com\n\nend_", vec!["_foo@bar.com"]),
        ("> http://_foo@bar.com\n> end_", vec!["foo@bar.com"]),
        ("> http://_foo@bar.com\nend_", vec!["foo@bar.com"]),
        ("- http://__foo@bar.com\n  end__", vec!["foo@bar.com"]),
        (
            "> - http://a_/(https://例.子/中😀)\n  lazy",
            vec!["https://例.子/中😀"],
        ),
        ("http://bad_/a@\nb.c", vec![]),
        ("> http://bad_/a@\n> b.c", vec![]),
        ("http://bad_/www.\nfoo", vec![]),
        ("http://bad_/mailto:a@", vec![]),
        ("http://bad_/(www.foo_)", vec![]),
    ] {
        let whole = parse(source);
        let ordered: Vec<_> = whole
            .links_in(Span::new(0, source.len()))
            .map(|(_, info)| whole.text(info.label).unwrap())
            .collect();
        assert_eq!(ordered, expected, "{source:?}");
        for split in 0..=source.len() {
            let mut bytes = Utf8Document::new();
            bytes.append(&source.as_bytes()[..split]).unwrap();
            bytes.append(&source.as_bytes()[split..]).unwrap();
            bytes.finish();
            assert_eq!(
                snapshot(bytes.document()),
                snapshot(&whole),
                "{source:?} {split}"
            );
        }
        // Force genuine live parsing beyond block lookahead, including every
        // Unicode scalar prefix and every prefix's byte-at-a-time EOF result.
        let lead = format!("{} ", "p".repeat(BLOCK_LOOKAHEAD));
        let mut live = Document::new();
        live.append(&lead).unwrap();
        for (at, ch) in source.char_indices() {
            live.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            let prefix = format!("{lead}{}", &source[..at + ch.len_utf8()]);
            let mut bulk = Document::new();
            bulk.append(&prefix).unwrap();
            assert_eq!(snapshot(&live), snapshot(&bulk), "{prefix:?}");
            let mut bytes = Utf8Document::new();
            for byte in prefix.bytes() {
                bytes.append(&[byte]).unwrap();
            }
            bytes.finish();
            bulk.finish();
            assert_eq!(snapshot(bytes.document()), snapshot(&bulk), "{prefix:?}");
        }
    }
}

#[test]
fn failed_web_recovery_late_header_decisions_settle_email_in_the_right_scope() {
    // An invalid host's own closing underscore can settle the inherited pair
    // before the email. Its final underscore then stays literal and rejects it.
    for prefix in ["http://bad_/", "www.bad_/", "https://a._/?"] {
        let source = format!("_prior\n{prefix}foo@bar.com_ tail\n");
        assert!(parse(&source).links().is_empty(), "{source:?}");
    }
    // Letters surround the invalid host underscore so it cannot consume
    // the inherited opener before the actual address's closing underscore.
    for prefix in ["http://a_b/", "www.a_b/", "https://a_b/?"] {
        for scheme in ["", "mailto:", "xmpp:"] {
            for ending in ["\n", " | b\nnot delimiter\n", " | b\n- | -\n"] {
                let address = format!("{scheme}foo@bar.com");
                let target = if scheme.is_empty() {
                    format!("mailto:{address}")
                } else {
                    address.clone()
                };
                let source = format!("_prior\n{prefix}{address}_ {}{ending}", "x".repeat(1100));
                let whole = parse(&source);
                if whole.tables().is_empty() {
                    assert_eq!(labels(&whole), [(address.as_str(), target)], "{source:?}");
                } else {
                    assert_eq!(whole.tables().len(), 1);
                    assert!(whole.links().is_empty(), "{source:?}");
                }
                for split in [
                    0,
                    3,
                    7,
                    source.find("foo@").unwrap(),
                    source.find(" | ").unwrap_or(source.len()),
                    source.len(),
                ] {
                    let mut bytes = Utf8Document::new();
                    bytes.append(&source.as_bytes()[..split]).unwrap();
                    bytes.append(&source.as_bytes()[split..]).unwrap();
                    bytes.finish();
                    assert_eq!(
                        snapshot(bytes.document()),
                        snapshot(&whole),
                        "{prefix} {scheme} {ending:?} {split}"
                    );
                }
                let mut live = Utf8Document::new();
                for byte in source.bytes() {
                    live.append(&[byte]).unwrap();
                }
                live.finish();
                assert_eq!(snapshot(live.document()), snapshot(&whole));
            }
        }
    }
}

#[test]
fn failed_web_recovery_late_objects_mapping_shared_budget_and_real_layout() {
    let local = "f".repeat(2 * TILE_BYTES + 3);
    let mut doc = Document::new();
    doc.append(&format!(
        "{} http://a_b/_{local}@bar.com [x](/x) rest",
        "p".repeat(BLOCK_LOOKAHEAD)
    ))
    .unwrap();
    assert_eq!(labels(&doc), [("x", "/x".into())]);
    let explicit = *doc.link_info(LinkId(0)).unwrap();
    let email_at = doc.source().find(&local).unwrap();
    let tile = doc.tile_at(email_at).unwrap();
    let key = doc.tile_key(tile).unwrap();
    let mut heights = HeightIndex::default();
    heights.rebuild(&doc).unwrap();
    let ticket = heights.ticket(tile).unwrap();
    heights.measure(ticket, 3).unwrap();
    let ticket = heights.ticket(tile).unwrap();
    let mut stale = doc
        .display_cursor(Span::new(email_at, email_at + local.len()))
        .unwrap();
    let mut stale_pass = LayoutPass::new(&doc, &heights, TileId(0), 1, true).unwrap();
    let update = doc.append("_ http://a_b/(www.例子/中😀) ").unwrap();
    heights.apply(&doc, &update).unwrap();
    assert_eq!(doc.link_info(LinkId(0)), Some(&explicit));
    assert_eq!(
        doc.link_url(LinkId(1)).unwrap(),
        format!("mailto:{local}@bar.com")
    );
    assert_eq!(doc.link_url(LinkId(2)).unwrap(), "http://www.例子/中😀");
    assert!(
        update
            .dirty
            .iter()
            .any(|s| s.start <= email_at && s.end >= email_at + local.len() + 8)
    );
    assert_ne!(doc.tile_key(tile).unwrap(), key);
    assert_eq!(
        heights.measure(ticket, 4),
        Err(LayoutError::StaleMeasurement)
    );
    let mut zero = Budget::new(WorkLimit::default());
    assert_eq!(
        stale.next(&doc, &mut zero),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(zero.used(), WorkLimit::default());

    #[derive(Default)]
    struct Sink {
        ids: Vec<LinkId>,
        bytes: usize,
    }
    impl LayoutSink for Sink {
        fn begin_tile(&mut self, _: TileId) {}
        fn text(&mut self, run: RenderRun<'_>) {
            self.bytes += run.text.len();
        }
        fn object(&mut self, id: LinkId, _: &LinkInfo) {
            self.ids.push(id);
        }
        fn end_tile(&mut self, _: TileId) -> u64 {
            0
        }
    }
    let mut sink = Sink::default();
    assert_eq!(
        stale_pass.advance(&doc, &mut heights, &mut zero, &mut sink),
        Err(LayoutError::StaleContinuation)
    );
    assert!(sink.ids.is_empty());
    assert_eq!(sink.bytes, 0);
    for (id, limit) in [(LinkId(1), 1), (LinkId(1), 6), (LinkId(2), 6)] {
        let info = doc.link_info(id).unwrap();
        assert_eq!(info.source, info.label);
        let mut target = doc.decoded_link_destination(id).unwrap();
        let mut raw = String::new();
        assert_eq!(target.advance(&mut zero), Err(Completion::BudgetExhausted));
        assert_eq!(target.source_position(), info.label.start);
        while target.source_position() < info.label.end {
            let mut budget = Budget::new(WorkLimit {
                bytes: limit,
                ..SIX
            });
            let old = target.source_position();
            let chunk = target.advance(&mut budget).unwrap().unwrap();
            assert_eq!(chunk.source.start, old);
            assert_eq!(doc.text(chunk.source), Some(chunk.text));
            raw.push_str(chunk.text);
            let mut peer = doc.display_cursor(info.label).unwrap();
            assert_eq!(
                peer.next(&doc, &mut budget).unwrap(),
                DisplayStep::BudgetExhausted
            );
            assert_eq!(peer.source_position(), info.label.start);
            assert!(budget.used().bytes <= limit);
        }
        assert_eq!(raw, doc.link_destination(id).unwrap());
    }
    let update = doc.finish();
    heights.apply(&doc, &update).unwrap();
    let ordered = doc
        .links_in(Span::new(0, doc.source().len()))
        .map(|(id, _)| id)
        .collect::<Vec<_>>();
    assert_eq!(ordered, [LinkId(1), LinkId(0), LinkId(2)]);
    for n in 0..doc.tile_count() {
        for run in doc.runs(TileId(n)).unwrap() {
            assert_eq!(doc.text(run.source), Some(run.text));
        }
    }
    let mut pass = LayoutPass::new(&doc, &heights, TileId(0), 1, true).unwrap();
    assert_eq!(
        pass.advance(&doc, &mut heights, &mut zero, &mut sink),
        Ok(Completion::BudgetExhausted)
    );
    assert_eq!(pass.source_position(), 0);
    // Text and object consumers use the same actual frame budget. A one-query
    // budget may stop before an object; two queries let the paid successor
    // lookup and delivery proceed. No history enumeration is hidden here.
    for step in 0..10000 {
        let mut budget = Budget::new(WorkLimit {
            bytes: if step % 2 == 0 { 1 } else { 6 },
            queries: 2,
            objects: 1,
            tiles: 1,
            recovery: 1,
            dependencies: 0,
        });
        let result = pass
            .advance(&doc, &mut heights, &mut budget, &mut sink)
            .unwrap();
        assert!(budget.used().bytes <= if step % 2 == 0 { 1 } else { 6 });
        assert!(budget.used().objects <= 1);
        if result == Completion::EndOfDocument {
            break;
        }
    }
    assert_eq!(pass.source_position(), doc.source().len());
    assert_eq!(sink.ids, ordered);
    assert!(sink.bytes > local.len());
}

#[test]
fn failed_web_recovery_million_byte_nested_prefixes_and_history_are_bounded() {
    for depth in [1, 16, 256, 4096, 65536] {
        let mut doc = Document::new();
        doc.append(&format!("{} ", "p".repeat(BLOCK_LOOKAHEAD)))
            .unwrap();
        let first = doc.work();
        doc.append(&"http://a_/(".repeat(depth)).unwrap();
        doc.append("www.foo/").unwrap();
        doc.append(&"x".repeat(1_000_000)).unwrap();
        for _ in 0..64 {
            let before = doc.work();
            doc.append("x").unwrap();
            assert_eq!(
                doc.work().bare_autolink_bytes - before.bare_autolink_bytes,
                1
            );
            assert!(doc.links().is_empty());
        }
        let before = doc.work();
        doc.append(" ").unwrap();
        let work = doc.work().bare_autolink_bytes - before.bare_autolink_bytes;
        let input = doc.source().len() as u64;
        eprintln!(
            "failed-Web operations: depth={depth} input={input} seal={work} total={}",
            doc.work().bare_autolink_bytes - first.bare_autolink_bytes
        );
        assert_eq!(doc.links().len(), 1);
        let label = doc.link_destination(LinkId(0)).unwrap();
        assert_eq!(label.len(), "www.foo/".len() + 1_000_000 + 64);
        assert!(label.starts_with("www.foo/"));
        assert!(work <= 20 * input, "depth={depth}: {work} / {input}");
        doc.finish();
        assert!(doc.work().bare_autolink_bytes - first.bare_autolink_bytes <= 24 * input);
        assert!(doc.work().bare_eligibility_bytes <= 24 * input);
    }
    // Many starts in one host must share suffix validation too. In particular,
    // do not copy .13's >10-segment shortcut that accepts invalid underscores.
    for (body, expected) in [
        (
            format!("http://a._{}www.c", "www._".repeat(100_000)),
            "www.c",
        ),
        (format!("http://a_b/{}b.c", "a@".repeat(100_000)), "a@b.c"),
        (
            format!("http://a._www.c{}", "?&a;".repeat(100_000)),
            "www.c",
        ),
    ] {
        let doc = parse(&body);
        assert_eq!(doc.links().len(), 1);
        assert_eq!(doc.link_destination(LinkId(0)), Some(expected));
        assert!(doc.work().bare_autolink_bytes <= 24 * body.len() as u64);
        assert!(doc.work().bare_eligibility_bytes <= 24 * body.len() as u64);
    }
    let mut doc = Document::new();
    doc.append(&format!("{} ", "> ".repeat(970))).unwrap();
    doc.append(&"x".repeat(1_000_000)).unwrap();
    doc.append(" http://a_b/a@b.c ").unwrap();
    let first = *doc.link_info(LinkId(0)).unwrap();
    for _ in 0..4096 {
        doc.append("[r] http://a_b/d@e.f \n").unwrap();
    }
    let links = doc.links().to_vec();
    let infos = (0..links.len())
        .map(|n| *doc.link_info(LinkId(n)).unwrap())
        .collect::<Vec<_>>();
    // Leave the deep container before defining the old reference in root scope.
    doc.append("\n\n").unwrap();
    let before = doc.work();
    doc.append("[r]: /winner\n").unwrap();
    assert_eq!(doc.work().bare_autolink_bytes, before.bare_autolink_bytes);
    assert_eq!(
        doc.work().bare_eligibility_bytes,
        before.bare_eligibility_bytes
    );
    assert_eq!(doc.links(), links);
    assert_eq!(doc.link_info(LinkId(0)), Some(&first));
    assert_eq!(doc.link_url(LinkId(1)).unwrap(), "/winner");
    for (n, info) in infos.iter().enumerate() {
        assert_eq!(doc.link_info(LinkId(n)), Some(info));
    }
    let before = doc.work();
    doc.append("http://a_b/z@q.r ").unwrap();
    assert!(doc.work().bare_autolink_bytes - before.bare_autolink_bytes <= 20 * 32);
    assert_eq!(doc.work().label_bytes, before.label_bytes);
    doc.finish();
    assert!(doc.work().bare_autolink_bytes <= 24 * doc.source().len() as u64);
}
