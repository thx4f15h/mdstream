//! Offline scoring of the complete GFM 0.29-gfm Task list items section (279–280).
//! The narrow projection reads public container/task metadata and display runs;
//! it does not identify task syntax or receive the example number. Only tight
//! bullet lists with one physical prose leaf per item are scored here.
use mdstream::{
    Block, BlockId, BlockKind, Container, ContainerId, ContainerKind, Document, Heading,
    ListMarker, ListTightness, Span, Style, TaskMarker,
};

include!("fixtures/gfm_tasks.rs");

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn escape(ch: char, out: &mut String) {
    match ch {
        '&' => out.push_str("&amp;"),
        '<' => out.push_str("&lt;"),
        '>' => out.push_str("&gt;"),
        '"' => out.push_str("&quot;"),
        _ => out.push(ch),
    }
}

fn inline(doc: &Document, source: Span) -> String {
    let mut chars = Vec::new();
    for run in doc.display_runs_in(source).unwrap() {
        if run.style.contains(Style::HIDDEN) {
            continue;
        }
        assert!(run.link.is_none());
        for unsupported in [Style::STRIKE, Style::MATH] {
            assert!(!run.style.contains(unsupported), "unscored inline style");
        }
        chars.extend(run.text.chars().map(|ch| (ch, run.style)));
    }
    // One physical prose terminator is structural. Never trim spaces/tabs,
    // unescape text, or remove internal line breaks.
    if chars.last().is_some_and(|(ch, _)| *ch == '\n') {
        chars.pop();
    }
    let mut html = String::new();
    let mut tags = Vec::new();
    for (ch, style) in chars {
        let next: Vec<_> = [
            (Style::STRONG, "strong"),
            (Style::EMPHASIS, "em"),
            (Style::CODE, "code"),
        ]
        .into_iter()
        .filter_map(|(bit, tag)| style.contains(bit).then_some(tag))
        .collect();
        let shared = tags.iter().zip(&next).take_while(|(a, b)| a == b).count();
        for tag in tags[shared..].iter().rev() {
            html.push_str(&format!("</{tag}>"));
        }
        for tag in &next[shared..] {
            html.push_str(&format!("<{tag}>"));
        }
        tags = next;
        if ch == '\n' && style.contains(Style::HARD_BREAK) {
            html.push_str("<br />");
        }
        escape(ch, &mut html);
    }
    for tag in tags.iter().rev() {
        html.push_str(&format!("</{tag}>"));
    }
    html
}

fn checkbox(doc: &Document, task: TaskMarker, block: Span, html: &mut String) {
    assert!(block.start <= task.source.start && task.source.end < block.end);
    assert_eq!(task.source.end - task.source.start, 3);
    let marker = doc.text(task.source).unwrap();
    assert!(if task.checked {
        matches!(marker, "[x]" | "[X]")
    } else {
        marker == "[ ]"
    });
    assert!(
        doc.runs_in(task.source)
            .unwrap()
            .all(|run| run.style.contains(Style::HIDDEN))
    );

    // The public model hides marker padding as well as the marker. These two
    // official inputs have exactly one ASCII separator; verify its source and
    // hidden mapping, then retain it after the semantic checkbox. Do not
    // normalize arbitrary whitespace into a space or recover hidden body text.
    let separator = Span::new(task.source.end, task.source.end + 1);
    assert_eq!(doc.text(separator), Some(" "), "unscored task padding");
    assert!(!doc.source()[separator.end..].starts_with([' ', '\t', '\r', '\n']));
    assert!(
        doc.runs_in(separator)
            .unwrap()
            .all(|run| run.style.contains(Style::HIDDEN))
    );
    html.push_str(if task.checked {
        "<input checked=\"\" disabled=\"\" type=\"checkbox\">"
    } else {
        "<input disabled=\"\" type=\"checkbox\">"
    });
    html.push_str(doc.text(separator).unwrap());
}

fn close_container(doc: &Document, id: ContainerId, html: &mut String) {
    html.push_str(match doc.container(id).unwrap().kind {
        ContainerKind::List { .. } => "</ul>\n",
        ContainerKind::Item { .. } => "</li>\n",
        other => panic!("container outside task projection: {other:?}"),
    });
}

fn project(doc: &Document) -> String {
    assert!(doc.links().is_empty());
    assert!(doc.maths().is_empty());
    assert!(doc.tables().is_empty());
    assert!(doc.headings().entries().is_empty());
    let mut html = String::new();
    let mut stack = Vec::new();
    let mut seen = vec![false; doc.container_count()];
    let mut items = vec![false; doc.container_count()];
    for block in doc.blocks() {
        let BlockKind::ListItem {
            number: None, task, ..
        } = block.kind
        else {
            panic!("block outside task projection: {:?}", block.kind);
        };
        let item = block.container.expect("missing item container");
        assert_eq!(
            doc.container(item).unwrap().kind,
            ContainerKind::Item { number: None, task }
        );
        let mut path = Vec::new();
        let mut parent = Some(item);
        while let Some(id) = parent {
            path.push(id);
            parent = doc.container(id).unwrap().parent;
        }
        path.reverse();
        let shared = stack.iter().zip(&path).take_while(|(a, b)| a == b).count();
        while stack.len() > shared {
            close_container(doc, stack.pop().unwrap(), &mut html);
        }
        for &id in &path[shared..] {
            let node = doc.container(id).unwrap();
            assert!(!seen[id.0], "reopened sealed container");
            seen[id.0] = true;
            match node.kind {
                ContainerKind::List {
                    marker: ListMarker::Bullet(_),
                } => {
                    assert_eq!(
                        node.list_tightness,
                        Some(ListTightness::Tight),
                        "unscored loose list"
                    );
                    if let Some(parent) = node.parent {
                        assert!(matches!(
                            doc.container(parent).unwrap().kind,
                            ContainerKind::Item { .. }
                        ));
                        assert!(items[parent.0], "unscored item without initial prose");
                        html.push('\n');
                    }
                    html.push_str("<ul>\n");
                }
                ContainerKind::Item { number: None, .. } => {
                    assert!(matches!(
                        doc.container(node.parent.unwrap()).unwrap().kind,
                        ContainerKind::List {
                            marker: ListMarker::Bullet(_)
                        }
                    ));
                    html.push_str("<li>");
                }
                other => panic!("container outside task projection: {other:?}"),
            }
            stack.push(id);
        }
        // Multiple physical prose leaves would need continuation/paragraph
        // handling. Reject them, instead of silently concatenating their text.
        assert!(!items[item.0], "unscored multi-line item");
        items[item.0] = true;
        if let Some(task) = task {
            checkbox(doc, task, block.source, &mut html);
        }
        html.push_str(&inline(doc, block.source));
    }
    while let Some(id) = stack.pop() {
        close_container(doc, id, &mut html);
    }
    assert!(seen.into_iter().all(|seen| seen), "unscored container");
    for (index, rendered) in items.into_iter().enumerate() {
        assert_eq!(
            rendered,
            matches!(
                doc.container(ContainerId(index)).unwrap().kind,
                ContainerKind::Item { .. }
            )
        );
    }
    html
}

fn check_case(number: usize, markdown: &str, expected: &str, actual: &str) {
    // No known deliberate difference or unsupported example in this subset.
    // A mismatch fails; upstream expectations never come from library output.
    assert!((279..=280).contains(&number), "unclassified example");
    assert_eq!(actual, expected, "example {number}, input {markdown:?}");
}

#[test]
fn pinned_gfm_tasks_report() {
    assert_eq!(TASK_CASES.len(), 2);
    let mut supported = 0;
    for (index, &(number, markdown, expected)) in TASK_CASES.iter().enumerate() {
        assert_eq!(number, 279 + index);
        let actual = project(&parse(markdown));
        check_case(number, markdown, expected, &actual);
        let reason = match number {
            279 => "unchecked and checked sibling task items",
            280 => "nested tasks, independent checked states and parent boundaries",
            _ => unreachable!(),
        };
        supported += 1;
        eprintln!("example {number}: supported — {reason}");
    }
    assert_eq!(supported, 2);
    eprintln!(
        "GFM 0.29-gfm Task list items (279–280): {supported} supported, 0 deliberate differences, 0 unsupported; other task/list rules not scored by this test"
    );
}

#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    blocks: Vec<Block>,
    headings: Vec<Heading>,
    containers: Vec<Container>,
    runs: Vec<(Span, Option<BlockId>, Style)>,
    display: Vec<(Span, Option<Span>, String, Style)>,
    pending: Span,
}

fn snapshot(doc: &Document) -> Snapshot {
    assert!(doc.links().is_empty());
    assert!(doc.maths().is_empty());
    assert!(doc.tables().is_empty());
    Snapshot {
        blocks: doc.blocks().to_vec(),
        headings: doc.headings().entries().to_vec(),
        containers: (0..doc.container_count())
            .map(|i| doc.container(ContainerId(i)).unwrap())
            .collect(),
        runs: doc
            .runs_in(Span::new(0, doc.source().len()))
            .unwrap()
            .map(|r| (r.source, r.block, r.style))
            .collect(),
        display: doc
            .display_runs_in(Span::new(0, doc.source().len()))
            .unwrap()
            .map(|r| (r.source, r.replacement, r.text.to_owned(), r.style))
            .collect(),
        pending: doc.pending(),
    }
}

#[test]
fn pinned_gfm_tasks_every_split_and_scalar_prefix() {
    let mut splits = 0;
    let mut prefixes = 0;
    for &(number, markdown, expected) in TASK_CASES {
        let whole = parse(markdown);
        for split in (0..=markdown.len()).filter(|&split| markdown.is_char_boundary(split)) {
            let mut doc = Document::new();
            doc.append(&markdown[..split]).unwrap();
            doc.append(&markdown[split..]).unwrap();
            doc.finish();
            assert_eq!(
                snapshot(&doc),
                snapshot(&whole),
                "example {number}, split {split}"
            );
            check_case(number, markdown, expected, &project(&doc));
            splits += 1;
        }
        let mut doc = Document::new();
        for (offset, ch) in markdown.char_indices() {
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            let mut prefix = Document::new();
            prefix.append(&markdown[..offset + ch.len_utf8()]).unwrap();
            assert_eq!(
                snapshot(&doc),
                snapshot(&prefix),
                "example {number}, prefix {offset}"
            );
            prefixes += 1;
        }
        doc.finish();
        check_case(number, markdown, expected, &project(&doc));
        assert_eq!(
            snapshot(&doc),
            snapshot(&whole),
            "example {number}, scalars"
        );
    }
    assert_eq!(splits, 66);
    assert_eq!(prefixes, 64);
    eprintln!(
        "2 examples: {splits} single splits and {prefixes} scalar prefixes preserve classified semantics, container identities, task marker ranges/states and source metadata"
    );
}

#[test]
fn task_projection_preserves_state_nesting_text_and_styles() {
    let checked = project(&parse("- [x] a\n"));
    assert_eq!(
        checked,
        "<ul>\n<li><input checked=\"\" disabled=\"\" type=\"checkbox\"> a</li>\n</ul>\n"
    );
    assert_eq!(checked, project(&parse("- [X] a\n")));
    for different in [
        "- [ ] a\n",          // checked state
        "- a\n",              // no task marker
        "- [x] b\n",          // body
        "- [x] a\n- [ ] b\n", // sibling count
        "- [x] *a*\n",
        "- [x] **a**\n",
        "- [x] `a`\n",
    ] {
        assert_ne!(checked, project(&parse(different)), "{different:?}");
    }
    assert_eq!(project(&parse("- a\n")), "<ul>\n<li>a</li>\n</ul>\n");
    let nested = project(&parse("- [x] a\n  - [ ] b\n- [x] c\n"));
    assert_eq!(
        nested,
        "<ul>\n<li><input checked=\"\" disabled=\"\" type=\"checkbox\"> a\n<ul>\n<li><input disabled=\"\" type=\"checkbox\"> b</li>\n</ul>\n</li>\n<li><input checked=\"\" disabled=\"\" type=\"checkbox\"> c</li>\n</ul>\n"
    );
    for different in [
        "- [x] a\n- [ ] b\n- [x] c\n",     // flatten
        "- [x] a\n  - [ ] b\n  - [x] c\n", // different parent
        "- [x] a\n  - [x] b\n- [x] c\n",   // child state
        "- [x] c\n  - [ ] b\n- [x] a\n",   // sibling text order
        "- [x] a\n  - [ ] b\n+ [x] c\n",   // new root list
    ] {
        assert_ne!(nested, project(&parse(different)), "{different:?}");
    }
    assert!(project(&parse("- [x] a  b\tc\n")).contains(" a  b\tc</li>"));
    assert!(project(&parse("- [x] a  \n")).contains(" a</li>"));
    assert!(project(&parse("- [x] &#32;a&#32;\n")).contains("  a </li>"));
    assert!(project(&parse("- [x] & < \\\"\n")).contains(" &amp; &lt; &quot;</li>"));
    assert!(project(&parse("- [x] *`a`*\n")).contains(" <em><code>a</code></em></li>"));
}

#[test]
fn task_projection_rejects_unscored_structure_and_padding() {
    // Do not accidentally report conformance after dropping unknown semantics
    // or synthesizing a standard-looking separator from arbitrary padding.
    for source in [
        "plain\n",
        "> - [x] a\n",
        "1. [x] a\n",
        "- [x] a\n\n- [ ] b\n",
        "- [x] a\n  continued\n",
        "- [x]\ta\n",
        "- [x]  a\n",
    ] {
        assert!(
            std::panic::catch_unwind(|| project(&parse(source))).is_err(),
            "{source:?}"
        );
    }
}
