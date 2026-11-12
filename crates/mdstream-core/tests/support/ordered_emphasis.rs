//! HTML encoding from real ordered syntax identities, never Markdown lexing.
use mdstream::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Node {
    Style(StylePairId),
    Link(LinkId),
}
fn tag(node: Node) -> &'static str {
    match node {
        Node::Link(_) => "a",
        Node::Style(id) => match id.kind {
            InlineStyleKind::Emphasis => "em",
            InlineStyleKind::Strong => "strong",
            InlineStyleKind::Strike => "del",
            InlineStyleKind::Code => "code",
        },
    }
}
#[path = "link_metadata.rs"]
pub(crate) mod metadata;

pub fn project(doc: &Document, allowed: Style, allow_links: bool) -> String {
    assert_eq!(doc.container_count(), 0);
    assert!(
        doc.headings().entries().is_empty()
            && doc.tables().is_empty()
            && doc.maths().is_empty()
            && doc.footnotes().is_empty()
    );
    if !allow_links {
        assert!(doc.links().is_empty() && doc.references().is_empty());
    }
    for i in 0..doc.links().len() {
        assert_eq!(doc.link_info(LinkId(i)).unwrap().kind, LinkKind::Link);
        assert!(matches!(doc.links()[i], LinkTarget::Direct(_)));
    }
    let openings = metadata::snapshot(doc);
    let mut resolver = doc.link_resolver();
    let mut html = String::new();
    for block in doc.blocks() {
        match block.kind {
            BlockKind::Blank => continue,
            BlockKind::Paragraph => (),
            ref kind => panic!("unscored block {kind:?}"),
        }
        html.push_str("<p>");
        let mut active: Vec<Node> = Vec::new();
        for run in doc.display_runs_in(block.source).unwrap() {
            if run.style.contains(Style::HIDDEN) {
                continue;
            }
            assert!(!run.style.contains(Style::MATH));
            for flag in [Style::EMPHASIS, Style::STRONG, Style::STRIKE, Style::CODE] {
                assert!(
                    !run.style.contains(flag) || allowed.contains(flag),
                    "unscored style"
                );
            }
            let mut path = doc.style_path(run.source.start).unwrap();
            let mut nodes = Vec::new();
            loop {
                match path
                    .advance(doc, &mut resolver, &mut Budget::new(WorkLimit::unlimited()))
                    .unwrap()
                {
                    StylePathStep::Pair(id) => nodes.push((id.open.start, Node::Style(id))),
                    StylePathStep::Complete => break,
                    StylePathStep::BudgetExhausted => panic!("unlimited style query paused"),
                }
            }
            assert_eq!(
                nodes
                    .iter()
                    .filter(
                        |(_, n)| matches!(n,Node::Style(p) if p.kind==InlineStyleKind::Emphasis)
                    )
                    .count(),
                run.emphasis_depth
            );
            assert_eq!(
                nodes
                    .iter()
                    .filter(|(_, n)| matches!(n,Node::Style(p) if p.kind==InlineStyleKind::Strong))
                    .count(),
                run.strong_depth
            );
            if let Some(id) = run.link {
                assert!(allow_links);
                let info = doc.link_info(id).unwrap();
                nodes.push((info.source.start, Node::Link(id)));
            }
            nodes.sort_by_key(|(at, _)| *at);
            let current: Vec<_> = nodes.into_iter().map(|(_, n)| n).collect();
            let common = active
                .iter()
                .zip(&current)
                .take_while(|(a, b)| a == b)
                .count();
            for &node in active[common..].iter().rev() {
                html.push_str(&format!("</{}>", tag(node)));
            }
            for &node in &current[common..] {
                if let Node::Link(id) = node {
                    html.push_str(openings[id.0].as_ref().unwrap());
                } else {
                    html.push_str(&format!("<{}>", tag(node)));
                }
            }
            active = current;
            let payload = if run.source.end == block.source.end
                && run.replacement.is_none()
                && active.is_empty()
                && run.text.ends_with('\n')
                && doc.text(run.source).unwrap().ends_with('\n')
            {
                assert!(
                    !run.style.contains(Style::HARD_BREAK)
                        && !run.style.contains(Style::SOFT_BREAK)
                );
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
        for node in active.into_iter().rev() {
            html.push_str(&format!("</{}>", tag(node)));
        }
        html.push_str("</p>\n");
    }
    html
}
