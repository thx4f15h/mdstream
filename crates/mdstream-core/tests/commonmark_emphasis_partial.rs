//! Offline scoring of CommonMark 0.31.2 Emphasis and strong emphasis subset (431–444).
//! The adapter reads public paragraph/display semantics, never Markdown syntax.
use mdstream::{
    BlockKind, Budget, DisplayStep, Document, LinkId, LinkKind, LinkTarget, Span, Style, WorkLimit,
};

#[path = "support/link_metadata.rs"]
mod link_metadata;

#[path = "support/snapshot.rs"]
mod support;
use support::snapshot;

include!("fixtures/commonmark_emphasis_partial.rs");

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

// Infer order only from public style transitions: an outer style must already
// be visible when an inner style begins. Simultaneous starts and crossing
// endings are ambiguous and rejected. Same-style depth is never reconstructed.
const STYLES: [(Style, &str); 2] = [(Style::EMPHASIS, "em"), (Style::STRONG, "strong")];

fn transition(html: &mut String, active: &mut Vec<usize>, style: Style) {
    let retained: Vec<_> = active
        .iter()
        .copied()
        .filter(|&i| style.contains(STYLES[i].0))
        .collect();
    assert_eq!(
        &active[..retained.len()],
        retained,
        "crossing style boundary"
    );
    let added: Vec<_> = (0..STYLES.len())
        .filter(|i| style.contains(STYLES[*i].0) && !retained.contains(i))
        .collect();
    assert!(added.len() <= 1, "ambiguous simultaneous style starts");
    for &i in active[retained.len()..].iter().rev() {
        html.push_str(&format!("</{}>", STYLES[i].1));
    }
    for &i in &added {
        html.push_str(&format!("<{}>", STYLES[i].1));
    }
    *active = retained;
    active.extend(added);
}

// Public syntax envelope supplies the styles outside a direct link. Inner
// styles are projected inside the anchor, never moved across its boundary.
fn link_outer_style(doc: &Document, id: LinkId) -> Style {
    let info = doc.link_info(id).unwrap();
    assert_eq!(info.kind, LinkKind::Link);
    assert!(matches!(doc.links()[id.0], LinkTarget::Direct(_)));
    assert!(
        info.source.start < info.label.start
            && info.label.start < info.label.end
            && info.label.end < info.source.end
    );
    let outer = doc
        .display_runs_in(info.source)
        .unwrap()
        .next()
        .unwrap()
        .style;
    for span in [
        Span::new(info.source.start, info.label.start),
        Span::new(info.label.end, info.source.end),
    ] {
        for run in doc.display_runs_in(span).unwrap() {
            assert!(run.style.contains(Style::HIDDEN));
            for (flag, _) in STYLES {
                assert_eq!(
                    run.style.contains(flag),
                    outer.contains(flag),
                    "nonuniform link envelope"
                );
            }
        }
    }
    outer
}

// No source syntax, example number or oracle enters this adapter. Source reads
// below only distinguish a borrowed physical paragraph LF from decoded content.
fn project(doc: &Document) -> String {
    assert_eq!(doc.container_count(), 0);
    assert!(doc.headings().entries().is_empty());
    // Direct links retain an interned provisional reference label; validate
    // the resolved LinkTarget below instead of requiring an empty symbol table.
    assert!(doc.tables().is_empty() && doc.maths().is_empty() && doc.footnotes().is_empty());
    let tags = link_metadata::snapshot(doc);
    let mut seen = vec![false; doc.links().len()];
    let mut html = String::new();
    for block in doc.blocks() {
        match block.kind {
            BlockKind::Blank => continue,
            BlockKind::Paragraph => {}
            ref other => panic!("unscored block {other:?}"),
        }
        html.push_str("<p>");
        let mut active = Vec::new();
        let mut link = None;
        for run in doc.display_runs_in(block.source).unwrap() {
            if run.style.contains(Style::HIDDEN) {
                continue;
            }
            for unsupported in [Style::CODE, Style::STRIKE, Style::MATH] {
                assert!(!run.style.contains(unsupported), "unscored inline style");
            }
            if link.is_some() && link != run.link {
                transition(&mut html, &mut active, link_outer_style(doc, link.unwrap()));
                html.push_str("</a>");
                link = None;
            }
            if let Some(id) = run.link {
                let outer = link_outer_style(doc, id);
                for (flag, _) in STYLES {
                    assert!(
                        !outer.contains(flag) || run.style.contains(flag),
                        "lost outer link style"
                    );
                }
                let info = doc.link_info(id).unwrap();
                assert!(
                    block.source.start <= info.source.start && info.source.end <= block.source.end
                );
                assert!(info.label.start <= run.source.start && run.source.end <= info.label.end);
                if link != Some(id) {
                    assert!(!seen[id.0], "split object label");
                    seen[id.0] = true;
                    transition(&mut html, &mut active, outer);
                    html.push_str(tags[id.0].as_ref().unwrap());
                    link = Some(id);
                }
            }
            transition(&mut html, &mut active, run.style);
            let payload = if run.source.end == block.source.end
                && run.replacement.is_none()
                && active.is_empty()
                && link.is_none()
                && run.text.ends_with('\n')
                && doc.text(run.source).unwrap().ends_with('\n')
            {
                assert!(!run.style.contains(Style::HARD_BREAK));
                assert!(!run.style.contains(Style::SOFT_BREAK));
                run.text.strip_suffix('\n').unwrap()
            } else {
                run.text
            };
            for ch in payload.chars() {
                if ch == '\n' && run.style.contains(Style::HARD_BREAK) {
                    html.push_str("<br />");
                }
                match ch {
                    '&' => html.push_str("&amp;"),
                    '<' => html.push_str("&lt;"),
                    '>' => html.push_str("&gt;"),
                    '"' => html.push_str("&quot;"),
                    _ => html.push(ch),
                }
            }
        }
        if let Some(id) = link {
            transition(&mut html, &mut active, link_outer_style(doc, id));
            html.push_str("</a>");
        }
        for &i in active.iter().rev() {
            html.push_str(&format!("</{}>", STYLES[i].1));
        }
        html.push_str("</p>\n");
    }
    assert!(seen.into_iter().all(|s| s), "unprojected object");
    html
}

enum Assessment {
    Supported,
    Unsupported {
        reason: &'static str,
        observed: &'static str,
    },
}

// Independently reviewed observations, separate from the unchanged oracle.
fn assessment(number: usize) -> Assessment {
    match number {
        433..=441 => Assessment::Supported,
        431 => Assessment::Unsupported {
            reason: "triple closer cannot close strong and emphasis openers separately",
            observed: "<p>**foo *bar***</p>\n",
        },
        432 => Assessment::Unsupported {
            reason: "public strong bit loses repeated inner strong depth",
            observed: "<p><strong>foo <em>bar baz\nbim</em> bop</strong></p>\n",
        },
        442 => Assessment::Unsupported {
            reason: "double opener is not partially consumed by a single closer",
            observed: "<p>**foo*</p>\n",
        },
        443 => Assessment::Unsupported {
            reason: "double closer is not partially consumed by a single opener",
            observed: "<p>*foo**</p>\n",
        },
        444 => Assessment::Unsupported {
            reason: "triple opener is not partially consumed by a double closer",
            observed: "<p>***foo**</p>\n",
        },
        _ => panic!("unclassified example {number}"),
    }
}

fn check_case(number: usize, markdown: &str, expected: &str, doc: &Document) {
    match assessment(number) {
        Assessment::Supported => assert_eq!(project(doc), expected, "example {number}"),
        Assessment::Unsupported { observed, .. } => {
            let actual = project(doc);
            assert_ne!(
                actual, expected,
                "example {number} now matches; review classification"
            );
            assert_eq!(
                actual, observed,
                "example {number} changed; upstream {expected:?}"
            );
        }
    }
    assert_eq!(doc.source(), markdown);
    let mut end = 0;
    for run in doc.display_runs_in(Span::new(0, markdown.len())).unwrap() {
        assert_eq!(run.source.start, end);
        assert!(run.source.start < run.source.end);
        // These official examples have no replacements, including hidden syntax.
        assert_eq!(run.replacement, None);
        assert_eq!(run.text, &markdown[run.source.range()]);
        end = run.source.end;
    }
    assert_eq!(end, markdown.len());
}

#[test]
fn pinned_commonmark_emphasis_partial_report() {
    assert_eq!(EMPHASIS_CASES.len(), 14);
    let mut counts = [0; 3];
    for (index, &(number, markdown, expected)) in EMPHASIS_CASES.iter().enumerate() {
        assert_eq!(number, 431 + index);
        check_case(number, markdown, expected, &parse(markdown));
        let (category, reason) = match assessment(number) {
            Assessment::Supported => (0, "matches upstream"),
            Assessment::Unsupported { reason, .. } => (2, reason),
        };
        counts[category] += 1;
        eprintln!("example {number}: category {category} — {reason}");
    }
    assert_eq!(counts, [9, 0, 5]);
    eprintln!(
        "CommonMark 0.31.2 Emphasis subset 431–444: 9 supported, 0 deliberate differences, 5 unsupported; 445–481 not scored"
    );
}

#[test]
fn pinned_commonmark_emphasis_partial_every_split_prefix_and_eof() {
    let mut splits = 0;
    let mut prefixes = 0;
    for &(number, markdown, expected) in EMPHASIS_CASES {
        let whole = parse(markdown);
        for split in (0..=markdown.len()).filter(|&i| markdown.is_char_boundary(i)) {
            let mut doc = Document::new();
            doc.append(&markdown[..split]).unwrap();
            doc.append(&markdown[split..]).unwrap();
            doc.finish();
            assert_eq!(
                snapshot(&doc),
                snapshot(&whole),
                "example {number}, split {split}"
            );
            assert_eq!(doc.pending(), whole.pending());
            check_case(number, markdown, expected, &doc);
            splits += 1;
        }
        let mut doc = Document::new();
        for (offset, ch) in markdown.char_indices() {
            let end = offset + ch.len_utf8();
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            let mut prefix = Document::new();
            prefix.append(&markdown[..end]).unwrap();
            assert_eq!(snapshot(&doc), snapshot(&prefix), "prefix {end}");
            assert_eq!(doc.pending(), prefix.pending());
            let mut finished = Document::new();
            for c in markdown[..end].chars() {
                finished.append(c.encode_utf8(&mut [0; 4])).unwrap();
            }
            prefix.finish();
            finished.finish();
            assert_eq!(
                snapshot(&finished),
                snapshot(&prefix),
                "example {number}, EOF {end}"
            );
            assert_eq!(finished.pending(), prefix.pending());
            prefixes += 1;
        }
        doc.finish();
        check_case(number, markdown, expected, &doc);
    }
    assert_eq!((splits, prefixes), (224, 210));
}

#[test]
fn emphasis_partial_mapping_slices_and_six_byte_budget() {
    for &(_, markdown, _) in EMPHASIS_CASES {
        let doc = parse(markdown);
        let span = Span::new(0, markdown.len());
        let expected: String = doc
            .display_runs_in(span)
            .unwrap()
            .filter(|r| !r.style.contains(Style::HIDDEN))
            .map(|r| r.text)
            .collect();
        let mut sliced = String::new();
        for (start, ch) in markdown.char_indices() {
            for run in doc
                .display_runs_in(Span::new(start, start + ch.len_utf8()))
                .unwrap()
            {
                assert_eq!(run.source, Span::new(start, start + ch.len_utf8()));
                assert_eq!(run.replacement, None);
                assert_eq!(run.text, &markdown[run.source.range()]);
                if !run.style.contains(Style::HIDDEN) {
                    sliced.push_str(run.text);
                }
            }
        }
        assert_eq!(sliced, expected);
        let expected_styles: Vec<_> = doc
            .display_runs_in(span)
            .unwrap()
            .flat_map(|r| {
                r.text
                    .char_indices()
                    .map(move |(i, _)| (r.source.start + i, r.style))
            })
            .collect();
        let mut actual_styles = Vec::new();
        let mut cursor = doc.display_cursor(span).unwrap();
        let mut peer = doc.display_cursor(span).unwrap();
        assert_eq!(
            cursor
                .next(&doc, &mut Budget::new(WorkLimit::default()))
                .unwrap(),
            DisplayStep::BudgetExhausted
        );
        assert_eq!(cursor.source_position(), 0);
        let mut output = String::new();
        while cursor.source_position() < markdown.len() {
            let old = cursor.source_position();
            let mut budget = Budget::new(WorkLimit {
                bytes: 6,
                queries: 1,
                objects: 1,
                ..Default::default()
            });
            let DisplayStep::Run(run) = cursor.next(&doc, &mut budget).unwrap() else {
                panic!("no progress");
            };
            assert_eq!(run.source.start, old);
            assert_eq!(run.replacement, None);
            assert_eq!(run.text, &markdown[run.source.range()]);
            assert!(cursor.source_position() > old);
            actual_styles.extend(
                run.text
                    .char_indices()
                    .map(|(i, _)| (run.source.start + i, run.style)),
            );
            let used = budget.used();
            assert_eq!(
                peer.next(&doc, &mut budget).unwrap(),
                DisplayStep::BudgetExhausted
            );
            assert_eq!(peer.source_position(), 0);
            assert_eq!(budget.used(), used);
            assert!(
                budget.used().bytes <= 6
                    && budget.used().queries <= 1
                    && budget.used().objects <= 1
            );
            if !run.style.contains(Style::HIDDEN) {
                output.push_str(run.text);
            }
            if cursor.source_position() < markdown.len() {
                assert_eq!(
                    cursor.next(&doc, &mut budget).unwrap(),
                    DisplayStep::BudgetExhausted
                );
            }
        }
        assert_eq!(output, expected);
        assert_eq!(actual_styles, expected_styles);
        assert_eq!(
            cursor
                .next(&doc, &mut Budget::new(WorkLimit::default()))
                .unwrap(),
            DisplayStep::Complete
        );
    }
}

#[test]
fn nesting_projection_preserves_visible_runs_and_rejects_ambiguous_order() {
    for (source, expected) in [
        ("", ""),
        (
            "_中 **😀** 尾_\n",
            "<p><em>中 <strong>😀</strong> 尾</em></p>\n",
        ),
        ("*a**b*\n", "<p><em>a**b</em></p>\n"),
        ("*a  b\tc*\n", "<p><em>a  b\tc</em></p>\n"),
        ("*a\nb*\n", "<p><em>a\nb</em></p>\n"),
        ("*a  \nb*\n", "<p><em>a<br />\nb</em></p>\n"),
        ("*a\n\nb*\n", "<p>*a</p>\n<p>b*</p>\n"),
        ("*&lt;&amp;&quot;*\n", "<p><em>&lt;&amp;&quot;</em></p>\n"),
        ("&#10;\n", "<p>\n</p>\n"),
        // The uniform envelope is projectable, but still lacks nested em depth.
        (
            "*foo [*bar*](/url)*\n",
            "<p><em>foo <a href=\"/url\">bar</a></em></p>\n",
        ),
    ] {
        assert_eq!(project(&parse(source)), expected);
    }
    let baseline = project(&parse("*a **b** c*\n"));
    for changed in [
        "a **b** c\n",
        "*a b c*\n",
        "*a **x** c*\n",
        "*a **b**  c*\n",
    ] {
        assert_ne!(project(&parse(changed)), baseline);
    }
    let doc = parse("*a **b** c*\n");
    let both = doc
        .display_runs_in(Span::new(0, doc.source().len()))
        .unwrap()
        .find(|r| r.style.contains(Style::EMPHASIS) && r.style.contains(Style::STRONG))
        .unwrap()
        .style;
    assert!(
        std::panic::catch_unwind(|| transition(&mut String::new(), &mut Vec::new(), both)).is_err()
    );
    assert!(
        std::panic::catch_unwind(|| transition(&mut String::new(), &mut vec![0, 1], Style::STRONG))
            .is_err()
    );
    for source in [
        "# h\n",
        "> q\n",
        "- a\n",
        "***\n",
        "    code\n",
        "~~~\nx\n~~~\n",
        "a|b\n--|--\n",
        "`code`\n",
        "~~s~~\n",
        "$m$\n",
        "*foo [bar*](/url)\n",
        "[r]\n\n[r]: /url\n",
        "![a](/x)\n",
        "[r]: /x\n",
        "[^f]: footnote\n",
    ] {
        assert!(
            std::panic::catch_unwind(|| project(&parse(source))).is_err(),
            "accepted {source:?}"
        );
    }
}

#[test]
fn repeated_strong_depth_and_link_scope_are_scored_separately() {
    let nested = parse("**foo *bar **baz**\nbim* bop**\n");
    let flat = parse("**foo *bar baz\nbim* bop**\n");
    assert_eq!(project(&nested), project(&flat));
    let &(_, _, official) = EMPHASIS_CASES.iter().find(|c| c.0 == 432).unwrap();
    assert_ne!(project(&nested), official);
    for (source, expected) in [
        (
            "**foo [*bar*](/url)**\n",
            "<p><strong>foo <a href=\"/url\"><em>bar</em></a></strong></p>\n",
        ),
        (
            "[a *b* c](/url)\n",
            "<p><a href=\"/url\">a <em>b</em> c</a></p>\n",
        ),
        (
            "*foo [bar](/url)*\n",
            "<p><em>foo <a href=\"/url\">bar</a></em></p>\n",
        ),
    ] {
        assert_eq!(project(&parse(source)), expected);
    }
    let expected = project(&parse("**foo [*bar*](/url)**\n"));
    for changed in [
        "**foo [bar](/url)**\n",
        "**foo [*bar*](/other)**\n",
        "foo [*bar*](/url)\n",
    ] {
        assert_ne!(project(&parse(changed)), expected);
    }
}

#[test]
fn nested_late_closers_preserve_identity_and_literal_partial_runs() {
    let body = "中😀x ".repeat(300);
    for (open, inner, inner_close, close, expected) in [
        (
            "_",
            "__",
            "__",
            "_",
            format!("<em>{body}<strong>bar</strong> tail</em>"),
        ),
        ("_", "_", "_", "_", format!("<em>{body}bar tail</em>")),
        (
            "*",
            "**",
            "**",
            "*",
            format!("<em>{body}<strong>bar</strong> tail</em>"),
        ),
        ("**", "*", "", "***", format!("**{body}*bar tail***")),
        ("**", "", "", "*", format!("**{body}bar tail*")),
        ("*", "", "", "**", format!("*{body}bar tail**")),
        ("***", "", "", "**", format!("***{body}bar tail**")),
    ] {
        let prefix = format!("{open}{body}{inner}bar");
        let suffix = format!("{inner_close} tail{close}!\n");
        let mut doc = Document::new();
        doc.append(&prefix).unwrap();
        let block = doc.block_at(0).unwrap();
        let mut current = prefix.clone();
        for ch in suffix.chars() {
            current.push(ch);
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            let mut whole = Document::new();
            whole.append(&current).unwrap();
            assert_eq!(snapshot(&doc), snapshot(&whole));
            assert_eq!(doc.pending(), whole.pending());
            assert_eq!(doc.block_at(0).unwrap(), block);
        }
        doc.finish();
        assert_eq!(snapshot(&doc), snapshot(&parse(&current)));
        assert_eq!(doc.block_at(0).unwrap(), block);
        assert_eq!(project(&doc), format!("<p>{expected}!</p>\n"));
    }
}

#[test]
fn mixed_late_close_preserves_styles_link_identity_and_shared_budget() {
    for (outer, outer_tag, inner, inner_tag) in
        [("*", "em", "**", "strong"), ("**", "strong", "*", "em")]
    {
        let body = "中😀x ".repeat(300);
        let source = format!("{outer}{body}{inner}inner{inner} [{inner}label{inner}](/url)");
        let mut doc = Document::new();
        doc.append(&source).unwrap();
        doc.append(" tail").unwrap();
        let block = doc.block_at(0).unwrap();
        let info = *doc.link_info(LinkId(0)).unwrap();
        let target = doc.links()[0];
        assert_eq!(
            project(&doc),
            format!(
                "<p>{outer}{body}<{inner_tag}>inner</{inner_tag}> <a href=\"/url\"><{inner_tag}>label</{inner_tag}></a> tail</p>\n"
            )
        );
        doc.append(outer).unwrap();
        doc.append("!\n").unwrap();
        assert_eq!(
            project(&doc),
            format!(
                "<p><{outer_tag}>{body}<{inner_tag}>inner</{inner_tag}> <a href=\"/url\"><{inner_tag}>label</{inner_tag}></a> tail</{outer_tag}>!</p>\n"
            )
        );
        assert_eq!(doc.block_at(0).unwrap(), block);
        assert_eq!(doc.link_info(LinkId(0)), Some(&info));
        assert_eq!(doc.links()[0], target);
        doc.finish();
        assert_eq!(
            snapshot(&doc),
            snapshot(&parse(&format!("{source} tail{outer}!\n")))
        );
        let mut display = doc.display_cursor(info.label).unwrap();
        let mut destination = doc.decoded_link_destination(LinkId(0)).unwrap();
        let mut shown = String::new();
        let mut url = String::new();
        let mut display_done = false;
        let mut destination_done = false;
        let mut frames = 0;
        while !display_done || !destination_done {
            frames += 1;
            assert!(frames <= info.source.len() * 4 + 8);
            let mut budget = Budget::new(link_metadata::SIX_BYTES);
            if !display_done {
                match display.next(&doc, &mut budget).unwrap() {
                    DisplayStep::Run(run) => {
                        assert!(run.style.contains(if outer == "*" {
                            Style::EMPHASIS
                        } else {
                            Style::STRONG
                        }));
                        if !run.style.contains(Style::HIDDEN) {
                            assert!(run.style.contains(if inner == "*" {
                                Style::EMPHASIS
                            } else {
                                Style::STRONG
                            }));
                            shown.push_str(run.text);
                        }
                    }
                    DisplayStep::Complete => display_done = true,
                    other => panic!("fresh display frame failed: {other:?}"),
                }
            }
            let old = destination.source_position();
            if !destination_done {
                match destination.advance(&mut budget) {
                    Ok(Some(chunk)) => {
                        assert_eq!(chunk.source.start, old);
                        assert_eq!(doc.text(chunk.source), Some(chunk.text));
                        url.push_str(chunk.text);
                    }
                    Ok(None) => destination_done = true,
                    Err(mdstream::Completion::BudgetExhausted) => {
                        assert_eq!(destination.source_position(), old)
                    }
                    Err(other) => panic!("unexpected {other:?}"),
                }
            }
            assert!(
                budget.used().bytes <= 6
                    && budget.used().queries <= 1
                    && budget.used().objects <= 1
            );
        }
        assert_eq!(shown, "label");
        assert_eq!(url, "/url");
    }
}
