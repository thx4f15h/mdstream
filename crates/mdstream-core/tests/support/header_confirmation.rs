//! Metamorphic scope-composition checks using the same parser, not an
//! independent semantic oracle. Handwritten/cmark expectations live separately.
use mdstream::*;
fn parse(s: &str) -> Document {
    let mut d = Document::new();
    d.append(s).unwrap();
    d.finish();
    d
}
fn shown(d: &Document, s: Span) -> Vec<(char, Style, usize, usize, Option<String>)> {
    d.display_runs_in(s)
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .flat_map(|r| {
            let link = r.link.and_then(|id| d.link_url(id)).map(|u| u.into_owned());
            r.text
                .chars()
                .map(move |c| (c, r.style, r.emphasis_depth, r.strong_depth, link.clone()))
        })
        .collect()
}
// Keep the canonical document's winning-definition context. Copying its full
// source is test-only and needed only when semantic definitions exist; this
// avoids reconstructing/escaping normalized labels or nested definition paths.
fn add_context(source: &mut String, canonical: &Document) {
    if canonical
        .references()
        .iter()
        .any(|r| r.destination.is_some())
        || canonical.footnotes().iter().any(|f| f.definition.is_some())
    {
        source.push_str("\n\n");
        source.push_str(canonical.source());
    }
}
pub fn inline_context(source: &str, canonical: &Document) -> (Document, Span) {
    const PREFIX: &str = "inline sentinel ";
    let mut input = String::from(PREFIX);
    input.push_str(source);
    let range = Span::new(PREFIX.len(), input.len());
    // Both boundaries are whitespace, just as an independent cell's edges.
    // The source range, never trim(), removes the ordinary-prose sentinel.
    input.push_str(" \n\n");
    add_context(&mut input, canonical);
    (parse(&input), range)
}
pub fn assert_scope_composition(d: &Document) {
    assert_eq!(d.tables().len(), 1, "{:?}", d.source());
    let table = &d.tables()[0];
    let old_range = Span::new(0, table.source.start);
    let mut old_source = d.text(old_range).unwrap().to_owned();
    old_source.push_str("\n\n");
    add_context(&mut old_source, d);
    let old = parse(&old_source);
    assert_eq!(
        shown(d, old_range),
        shown(&old, old_range),
        "old physical boundary in canonical reference context"
    );
    for row in table.rows() {
        for c in &row.cells {
            let (cell, range) = inline_context(d.text(c.source).unwrap(), d);
            assert_eq!(links(d, c.source), links(&cell, range));
            assert_eq!(maths(d, c.source), maths(&cell, range));
            assert_eq!(feet(d, c.source), feet(&cell, range));
            assert_eq!(
                shown(d, c.source),
                table_code_text(shown(&cell, range)),
                "inline cell {:?}",
                d.text(c.source)
            );
        }
    }
}

fn links(d: &Document, s: Span) -> Vec<(LinkKind, String, Option<String>, Option<String>)> {
    d.links_in(s)
        .filter(|(_, i)| i.state == SyntaxState::Sealed)
        .map(|(id, i)| {
            (
                i.kind,
                d.text(i.label).unwrap().to_owned(),
                d.link_url(id).map(|u| u.into_owned()),
                d.link_title(id).map(str::to_owned),
            )
        })
        .collect()
}
fn maths(d: &Document, s: Span) -> Vec<(MathKind, String)> {
    let mut c = d.maths_in(s).unwrap();
    let mut out = Vec::new();
    while let Some((id, m)) = c.advance(&mut Budget::new(WorkLimit::unlimited())).unwrap() {
        let mut payload = d.math_payload(id).unwrap();
        let mut text = String::new();
        while let Some(part) = payload
            .advance(&mut Budget::new(WorkLimit::unlimited()))
            .unwrap()
        {
            text.push_str(part.text);
        }
        out.push((m.kind, text));
    }
    out
}
fn feet(d: &Document, s: Span) -> Vec<String> {
    let mut c = d.footnote_references_in(s).unwrap();
    let mut out = Vec::new();
    while let Some((_, r)) = c.advance(&mut Budget::new(WorkLimit::unlimited())).unwrap() {
        out.push(d.footnote(r.symbol).unwrap().label.clone());
    }
    out
}

fn table_code_text(
    mut runs: Vec<(char, Style, usize, usize, Option<String>)>,
) -> Vec<(char, Style, usize, usize, Option<String>)> {
    // GFM table cells remove the backslash in an escaped pipe even in code;
    // ordinary standalone code deliberately preserves that backslash.
    let mut i = 0;
    while i + 1 < runs.len() {
        if runs[i].0 == '\\' && runs[i].1.contains(Style::CODE) && runs[i + 1].0 == '|' {
            runs.remove(i);
        }
        i += 1;
    }
    runs
}
