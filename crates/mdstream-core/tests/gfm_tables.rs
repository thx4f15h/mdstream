//! Offline scoring of the complete GFM 0.29-gfm Tables section (198–205).
//! This narrow projection uses public table/cell metadata and display runs. It
//! never reparses Markdown, trims display text, or receives an example number.
//! It is a test adapter, not a library HTML renderer.
use mdstream::{
    Alignment, Block, BlockId, BlockKind, Container, ContainerId, ContainerKind, Document, Heading,
    Span, Style, Table,
};

include!("fixtures/gfm_tables.rs");

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

fn inline(doc: &Document, source: Span, paragraph: bool) -> String {
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
    // One physical paragraph terminator is structural. Cell spans already
    // exclude padding in the public model; never trim or unescape their text.
    if paragraph && chars.last().is_some_and(|(ch, _)| *ch == '\n') {
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

fn table_html(doc: &Document, table: &Table) -> String {
    assert!(!table.rows().is_empty());
    assert_eq!(table.rows()[0].cells.len(), table.columns.len());
    let mut html = "<table>\n<thead>\n".to_owned();
    for (row_index, row) in table.rows().iter().enumerate() {
        assert!(row.complete);
        assert!(row.cells.len() <= table.columns.len());
        if row_index == 1 {
            html.push_str("<tbody>\n");
        }
        html.push_str("<tr>\n");
        let tag = if row_index == 0 { "th" } else { "td" };
        for (col_index, column) in table.columns.iter().enumerate() {
            let align = match column.alignment {
                Alignment::Default => "",
                Alignment::Left => " align=\"left\"",
                Alignment::Center => " align=\"center\"",
                Alignment::Right => " align=\"right\"",
            };
            html.push_str(&format!("<{tag}{align}>"));
            // Missing cells are documented implicit empties, not a parser
            // failure repaired by this adapter. Excess cells must be absent.
            if let Some(cell) = table.cell(row_index, col_index) {
                html.push_str(&inline(doc, cell.source, false));
            }
            html.push_str(&format!("</{tag}>\n"));
        }
        html.push_str("</tr>\n");
        if row_index == 0 {
            html.push_str("</thead>\n");
        }
    }
    if table.rows().len() > 1 {
        html.push_str("</tbody>\n");
    }
    html.push_str("</table>\n");
    html
}

fn project(doc: &Document) -> String {
    assert!(doc.links().is_empty());
    assert!(doc.maths().is_empty());
    assert!(doc.headings().entries().is_empty());
    let mut html = String::new();
    let mut tables = 0;
    let mut quote = None;
    let mut seen = vec![false; doc.container_count()];
    for (index, block) in doc.blocks().iter().enumerate() {
        if quote != block.container {
            if quote.is_some() {
                html.push_str("</blockquote>\n");
            }
            if let Some(id) = block.container {
                let node = doc.container(id).unwrap();
                assert_eq!(node.kind, ContainerKind::Quote);
                assert!(node.parent.is_none(), "unscored nested container");
                assert!(!seen[id.0], "reopened sealed quote");
                seen[id.0] = true;
                html.push_str("<blockquote>\n");
            }
            quote = block.container;
        }
        match block.kind {
            BlockKind::Table { id } => {
                assert!(block.container.is_none(), "unscored container table");
                assert_eq!(id.0, tables);
                let table = doc.table(id).unwrap();
                assert_eq!(table.block, BlockId(index));
                assert_eq!(table.source, block.source);
                html.push_str(&table_html(doc, table));
                tables += 1;
            }
            BlockKind::Paragraph | BlockKind::Quote { .. } => {
                // The selected examples have a single physical quote leaf.
                // Do not silently treat multi-line container prose as separate
                // paragraphs; it needs a wider adapter if added to this suite.
                assert!(
                    block.container.is_none()
                        || index == 0
                        || doc.blocks()[index - 1].container != block.container
                );
                html.push_str("<p>");
                html.push_str(&inline(doc, block.source, true));
                html.push_str("</p>\n");
            }
            BlockKind::Blank => {}
            ref other => panic!("block outside Tables projection: {other:?}"),
        }
    }
    if quote.is_some() {
        html.push_str("</blockquote>\n");
    }
    assert_eq!(tables, doc.tables().len(), "unscored table");
    assert!(seen.into_iter().all(|seen| seen), "unscored container");
    html
}

fn check_case(number: usize, markdown: &str, expected: &str, actual: &str) {
    // Initial independent observation: all eight examples match upstream.
    // There are no deliberate differences or unsupported cases in this subset.
    // A mismatch must fail; never derive a new expectation from library output.
    assert!((198..=205).contains(&number), "unclassified example");
    assert_eq!(actual, expected, "example {number}, input {markdown:?}");
}

#[test]
fn pinned_gfm_tables_report() {
    assert_eq!(TABLE_CASES.len(), 8);
    let mut supported = 0;
    for (index, &(number, markdown, expected)) in TABLE_CASES.iter().enumerate() {
        assert_eq!(number, 198 + index);
        check_case(number, markdown, expected, &project(&parse(markdown)));
        let reason = match number {
            198 => "header, delimiter and body rows",
            199 => "center/right alignment and optional outer pipes",
            200 => "escaped pipes in text, code and strong emphasis",
            201 => "quote interrupts the table",
            202 => "pipe-free sparse row and blank-line termination",
            203 => "mismatched header/delimiter columns remain a paragraph",
            204 => "implicit empty cells and ignored excess cells",
            205 => "header-only table has no body",
            _ => unreachable!(),
        };
        supported += 1;
        eprintln!("example {number}: supported — {reason}");
    }
    assert_eq!(supported, 8);
    eprintln!(
        "GFM 0.29-gfm Tables (198–205): {supported} supported, 0 deliberate differences, 0 unsupported; other GFM sections not scored by this test"
    );
}

#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    blocks: Vec<Block>,
    headings: Vec<Heading>,
    containers: Vec<Container>,
    tables: Vec<Table>,
    runs: Vec<(Span, Option<BlockId>, Style)>,
    display: Vec<(Span, Option<Span>, String, Style)>,
    pending: Span,
}

fn snapshot(doc: &Document) -> Snapshot {
    assert!(doc.links().is_empty());
    assert!(doc.maths().is_empty());
    Snapshot {
        blocks: doc.blocks().to_vec(),
        headings: doc.headings().entries().to_vec(),
        containers: (0..doc.container_count())
            .map(|i| doc.container(ContainerId(i)).unwrap())
            .collect(),
        tables: doc.tables().to_vec(),
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
fn pinned_gfm_tables_every_split_and_scalar_prefix() {
    let mut splits = 0;
    let mut prefixes = 0;
    for &(number, markdown, expected) in TABLE_CASES {
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
    assert_eq!(splits, 361);
    assert_eq!(prefixes, 353);
    eprintln!(
        "8 examples: {splits} single splits and {prefixes} scalar prefixes preserve classified semantics, table identities, cell ranges and source metadata"
    );
}

#[test]
fn table_projection_preserves_cells_alignment_styles_and_boundaries() {
    // Handwritten adapter checks are separate from the untouched official data.
    let base = project(&parse("a|b\n-|-\nx|y\n"));
    assert_eq!(
        base,
        "<table>\n<thead>\n<tr>\n<th>a</th>\n<th>b</th>\n</tr>\n</thead>\n<tbody>\n<tr>\n<td>x</td>\n<td>y</td>\n</tr>\n</tbody>\n</table>\n"
    );
    for different in [
        "a|b\n:-|-\nx|y\n", // left alignment is distinct from default
        "a|b\n-:|-\nx|y\n",
        "a|b\n:-:|-\nx|y\n",
        "b|a\n-|-\nx|y\n", // column/header order
        "a|b\n-|-\ny|x\n",
        "a|b\n-|-\nx\n", // sparse cell is not the existing y
        "a|b\n-|-\n",    // header-only is not an empty body row
        "a|b\n-|-\n| | |\n",
        "a|b\n-|-\n*x*|y\n",
        "a|b\n-|-\n**x**|y\n",
        "a|b\n-|-\n`x`|y\n",
        "a|b\n-|-\nx|y\nz|w\n",   // additional row must survive
        "a|b\n-|-\nx|y\n\nz|w\n", // paragraph after table
    ] {
        assert_ne!(base, project(&parse(different)), "input {different:?}");
    }
    assert_eq!(base, project(&parse("a|b\n-|-\nx|y|ignored\n")));
    assert!(project(&parse("a|b\n-|-\nx\n")).contains("<td>x</td>\n<td></td>"));
    assert!(!project(&parse("a|b\n-|-\n")).contains("<tbody>"));
    assert!(project(&parse("a|b\n-|-\n| | |\n")).contains("<tbody>"));
    assert!(project(&parse("a|b\n-|-\nx  y|z\n")).contains("<td>x  y</td>"));
    assert!(project(&parse("a|b\n-|-\nx\ty|z\n")).contains("<td>x\ty</td>"));
    assert!(
        project(&parse("a|b\n-|-\n&#32;x&#32;|&amp;\n")).contains("<td> x </td>\n<td>&amp;</td>")
    );
    assert!(project(&parse("a|b\n-|-\n*`x`*|z\n")).contains("<td><em><code>x</code></em></td>"));
    assert_eq!(project(&parse("  a\t\n")), "<p>  a\t</p>\n");
    assert_eq!(project(&parse("a\nb\n")), "<p>a\nb</p>\n");
    assert_eq!(project(&parse("a\n\nb\n")), "<p>a</p>\n<p>b</p>\n");
    assert_eq!(project(&parse("& < \\\"\n")), "<p>&amp; &lt; &quot;</p>\n");
    assert_eq!(
        project(&parse("> a\n\n> b\n")),
        "<blockquote>\n<p>a</p>\n</blockquote>\n<blockquote>\n<p>b</p>\n</blockquote>\n"
    );
}
