//! Complete CommonMark 0.31.2 Precedence (42), observed through public APIs.
//! Container/leaf boundaries and published code styles determine the output.
use mdstream::{
    BlockKind, Budget, ContainerId, ContainerKind, DisplayStep, Document, ListMarker,
    ListTightness, Span, Style, Utf8Document, WorkLimit,
};

#[path = "support/snapshot.rs"]
mod support;
use support::snapshot;

include!("fixtures/commonmark_precedence.rs");

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn inline(doc: &Document, source: Span) -> String {
    let mut chars = Vec::new();
    for run in doc.display_runs_in(source).unwrap() {
        if run.style.contains(Style::HIDDEN) {
            continue;
        }
        assert!(run.link.is_none());
        for unsupported in [Style::EMPHASIS, Style::STRONG, Style::STRIKE, Style::MATH] {
            assert!(!run.style.contains(unsupported), "unscored inline style");
        }
        // Only a borrowed physical leaf terminator is structural. Decoded
        // entity LF and visible whitespace remain content; never trim them.
        let text = if run.replacement.is_none()
            && run.source.end == source.end
            && run.text.ends_with('\n')
            && doc.text(run.source).unwrap().ends_with('\n')
        {
            assert!(!run.style.contains(Style::SOFT_BREAK));
            assert!(!run.style.contains(Style::HARD_BREAK));
            assert!(!run.style.contains(Style::CODE));
            run.text.strip_suffix('\n').unwrap()
        } else {
            run.text
        };
        chars.extend(text.chars().map(|ch| (ch, run.style)));
    }
    let mut html = String::new();
    let mut code = false;
    for (ch, style) in chars {
        let next = style.contains(Style::CODE);
        if code != next {
            html.push_str(if next { "<code>" } else { "</code>" });
            code = next;
        }
        if ch == '\n' && style.contains(Style::HARD_BREAK) {
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
    if code {
        html.push_str("</code>");
    }
    html
}

// Narrow, independent projection: flat tight bullet lists with one prose leaf
// per item, or root paragraphs. Reject any structure this adapter cannot score.
fn project(doc: &Document) -> String {
    assert!(doc.headings().entries().is_empty());
    assert!(doc.links().is_empty() && doc.references().is_empty());
    assert!(doc.tables().is_empty() && doc.maths().is_empty() && doc.footnotes().is_empty());
    let mut html = String::new();
    let mut active_list = None;
    let mut seen = vec![false; doc.container_count()];
    for block in doc.blocks() {
        if block.kind == BlockKind::Blank {
            continue;
        }
        let list = if let Some(item_id) = block.container {
            assert!(matches!(
                block.kind,
                BlockKind::ListItem {
                    number: None,
                    task: None,
                    ..
                }
            ));
            let item = doc.container(item_id).unwrap();
            assert!(matches!(
                item.kind,
                ContainerKind::Item {
                    number: None,
                    task: None
                }
            ));
            assert!(!seen[item_id.0], "unscored multi-leaf item");
            seen[item_id.0] = true;
            let list_id = item.parent.unwrap();
            let list = doc.container(list_id).unwrap();
            assert!(matches!(
                list.kind,
                ContainerKind::List {
                    marker: ListMarker::Bullet(_)
                }
            ));
            assert_eq!(list.parent, None);
            assert_eq!(list.list_tightness, Some(ListTightness::Tight));
            Some(list_id)
        } else {
            assert_eq!(block.kind, BlockKind::Paragraph);
            None
        };
        if list != active_list {
            if active_list.is_some() {
                html.push_str("</ul>\n");
            }
            if let Some(id) = list {
                assert!(!seen[id.0], "reopened list");
                seen[id.0] = true;
                html.push_str("<ul>\n");
            }
            active_list = list;
        }
        html.push_str(if list.is_some() { "<li>" } else { "<p>" });
        html.push_str(&inline(doc, block.source));
        html.push_str(if list.is_some() { "</li>\n" } else { "</p>\n" });
    }
    if active_list.is_some() {
        html.push_str("</ul>\n");
    }
    assert!(seen.into_iter().all(|seen| seen), "unscored container");
    html
}

fn check_mapping(doc: &Document, source: &str) {
    assert_eq!(doc.source(), source);
    let mut end = 0;
    for run in doc.display_runs_in(Span::new(0, source.len())).unwrap() {
        assert_eq!(run.source.start, end);
        assert!(run.source.end > end);
        assert_eq!(run.replacement, None);
        assert_eq!(run.text, &source[run.source.range()]);
        end = run.source.end;
    }
    assert_eq!(end, source.len());
}

#[test]
fn pinned_commonmark_precedence_report() {
    assert_eq!(PRECEDENCE_CASES.len(), 1);
    let (number, markdown, expected) = PRECEDENCE_CASES[0];
    assert_eq!(number, 42);
    let doc = parse(markdown);
    assert_eq!(project(&doc), expected);
    assert_eq!(doc.container_count(), 3);
    assert_eq!(doc.blocks().len(), 2);
    assert_ne!(doc.blocks()[0].container, doc.blocks()[1].container);
    assert_eq!(
        doc.container(ContainerId(0)).unwrap().source,
        Span::new(0, markdown.len())
    );
    for run in doc.runs_in(Span::new(0, markdown.len())).unwrap() {
        assert!(
            !run.style.contains(Style::CODE),
            "code crossed an item boundary"
        );
    }
    check_mapping(&doc, markdown);
    eprintln!("example {number}: supported — block boundaries precede code span matching");
    eprintln!(
        "CommonMark 0.31.2 Precedence (42): 1 supported, 0 deliberate differences, 0 unsupported"
    );
}

#[test]
fn precedence_every_split_scalar_byte_prefix_and_eof() {
    for source in [PRECEDENCE_CASES[0].1, "- `一\n- 二`\n", "- `one\n- two`"] {
        let whole = parse(source);
        for split in 0..=source.len() {
            let mut stream = Utf8Document::new();
            stream.append(&source.as_bytes()[..split]).unwrap();
            stream.append(&source.as_bytes()[split..]).unwrap();
            stream.finish();
            assert!(stream.pending_bytes().is_empty());
            assert_eq!(
                snapshot(stream.document()),
                snapshot(&whole),
                "split {split}"
            );
            assert_eq!(project(stream.document()), project(&whole));
        }
        let mut doc = Document::new();
        let mut stream = Utf8Document::new();
        for end in 1..=source.len() {
            stream.append(&source.as_bytes()[end - 1..end]).unwrap();
            let mut boundary = end;
            while !source.is_char_boundary(boundary) {
                boundary -= 1;
            }
            assert_eq!(stream.pending_bytes(), &source.as_bytes()[boundary..end]);
            let mut prefix = Document::new();
            prefix.append(&source[..boundary]).unwrap();
            assert_eq!(snapshot(stream.document()), snapshot(&prefix));
            if source.is_char_boundary(end) {
                let start = source[..end].char_indices().next_back().unwrap().0;
                doc.append(&source[start..end]).unwrap();
                assert_eq!(snapshot(&doc), snapshot(&prefix));
                assert_eq!(doc.pending(), prefix.pending());
                // EOF at every scalar boundary must agree with an independent
                // single append, including unmatched and partially read ticks.
                let mut finished = Document::new();
                for ch in source[..end].chars() {
                    finished.append(ch.encode_utf8(&mut [0; 4])).unwrap();
                }
                finished.finish();
                prefix.finish();
                assert_eq!(snapshot(&prefix), snapshot(&finished));
            }
        }
        doc.finish();
        stream.finish();
        assert_eq!(snapshot(&doc), snapshot(&whole));
        assert_eq!(snapshot(stream.document()), snapshot(&whole));
        check_mapping(&doc, source);
    }
}

#[test]
fn precedence_projection_observes_boundaries_code_and_whitespace() {
    for (source, expected) in [
        (PRECEDENCE_CASES[0].1, PRECEDENCE_CASES[0].2),
        (
            "- `一\n- 二`\n",
            "<ul>\n<li>`一</li>\n<li>二`</li>\n</ul>\n",
        ),
        (
            "- `one`\n- `two`\n",
            "<ul>\n<li><code>one</code></li>\n<li><code>two</code></li>\n</ul>\n",
        ),
        ("`one\n\ntwo`\n", "<p>`one</p>\n<p>two`</p>\n"),
        (
            "- one  two\n- <&\"\n",
            "<ul>\n<li>one  two</li>\n<li>&lt;&amp;&quot;</li>\n</ul>\n",
        ),
        ("a&#10;", "<p>a\n</p>\n"),
        ("a&#10;\n", "<p>a\n</p>\n"),
        ("a&#32;", "<p>a </p>\n"),
        ("a&#32;\n", "<p>a </p>\n"),
    ] {
        assert_eq!(project(&parse(source)), expected, "{source:?}");
    }
    for source in [
        "> a\n",
        "1. a\n",
        "- [x] a\n",
        "- a\n\n- b\n",
        "- - a\n",
        "- a\n  b\n",
        "# a\n",
        "    a\n",
        "*a*\n",
        "~~a~~\n",
        "[a](b)\n",
        "$a$\n",
    ] {
        assert!(
            std::panic::catch_unwind(|| project(&parse(source))).is_err(),
            "accepted {source:?}"
        );
    }
}

#[test]
fn precedence_source_mapping_and_shared_display_budget() {
    for source in [PRECEDENCE_CASES[0].1, "- `一\n- 二`\n"] {
        let doc = parse(source);
        check_mapping(&doc, source);
        let span = Span::new(0, source.len());
        let visible: String = doc
            .display_runs_in(span)
            .unwrap()
            .filter(|r| !r.style.contains(Style::HIDDEN))
            .map(|r| r.text)
            .collect();
        let mut sliced = String::new();
        for (start, ch) in source.char_indices() {
            for run in doc
                .display_runs_in(Span::new(start, start + ch.len_utf8()))
                .unwrap()
            {
                assert_eq!(run.source, Span::new(start, start + ch.len_utf8()));
                assert_eq!(run.text, &source[run.source.range()]);
                if !run.style.contains(Style::HIDDEN) {
                    sliced.push_str(run.text);
                }
            }
        }
        assert_eq!(sliced, visible);
        let mut cursor = doc.display_cursor(span).unwrap();
        let mut rival = doc.display_cursor(span).unwrap();
        assert_eq!(
            cursor
                .next(&doc, &mut Budget::new(WorkLimit::default()))
                .unwrap(),
            DisplayStep::BudgetExhausted
        );
        assert_eq!(cursor.source_position(), 0);
        let mut output = String::new();
        while cursor.source_position() < source.len() {
            let old = cursor.source_position();
            let mut budget = Budget::new(WorkLimit {
                bytes: 6,
                queries: 1,
                objects: 1,
                ..Default::default()
            });
            let DisplayStep::Run(run) = cursor.next(&doc, &mut budget).unwrap() else {
                panic!("positive budget made no progress");
            };
            assert_eq!(run.source.start, old);
            assert_eq!(run.text, &source[run.source.range()]);
            assert!(cursor.source_position() > old);
            if !run.style.contains(Style::HIDDEN) {
                output.push_str(run.text);
            }
            assert_eq!(
                rival.next(&doc, &mut budget).unwrap(),
                DisplayStep::BudgetExhausted
            );
            assert_eq!(rival.source_position(), 0);
            assert!(
                budget.used().bytes <= 6
                    && budget.used().queries <= 1
                    && budget.used().objects <= 1
            );
        }
        assert_eq!(output, visible);
        assert_eq!(
            cursor
                .next(&doc, &mut Budget::new(WorkLimit::default()))
                .unwrap(),
            DisplayStep::Complete
        );
    }
}
