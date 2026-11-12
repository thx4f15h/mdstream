mod support;

use std::fmt::Write as _;

use mdtui_tui::render::document::RenderStats;
use mdtui_tui::screen::{Screen, StyleRole};
use mdtui_tui::static_render::{StaticRender, render_static};
use mdtui3::Utf8Document;
use support::snapshot::snapshot_settings;

const HEIGHT: u16 = 512;
const WIDTHS: [u16; 4] = [1, 20, 80, 120];

const FIXTURES: &[(&str, &str)] = &[
    ("spec_headings", include_str!("fixtures/spec/headings.md")),
    (
        "spec_paragraphs",
        include_str!("fixtures/spec/paragraphs.md"),
    ),
    ("spec_inline", include_str!("fixtures/spec/inline.md")),
    ("spec_links", include_str!("fixtures/spec/links.md")),
    ("spec_images", include_str!("fixtures/spec/images.md")),
    ("spec_quotes", include_str!("fixtures/spec/quotes.md")),
    ("spec_lists", include_str!("fixtures/spec/lists.md")),
    ("spec_code", include_str!("fixtures/spec/code.md")),
    ("spec_overwide", include_str!("fixtures/spec/overwide.md")),
    ("spec_hr", include_str!("fixtures/spec/hr.md")),
    ("spec_tables", include_str!("fixtures/spec/tables.md")),
    ("spec_spacing", include_str!("fixtures/spec/spacing.md")),
    ("spec_footnotes", include_str!("fixtures/spec/footnotes.md")),
    ("spec_html", include_str!("fixtures/spec/html.md")),
    (
        "fixture_links_anchors",
        include_str!("fixtures/links/anchors.md"),
    ),
    (
        "fixture_stream_large",
        include_str!("fixtures/stream/large.md"),
    ),
    (
        "fixture_stream_hidden_blocks",
        include_str!("fixtures/stream/hidden-blocks.md"),
    ),
    (
        "fixture_sidebar_many_headings",
        include_str!("fixtures/sidebar/many-headings.md"),
    ),
    (
        "fixture_sidebar_huge_setext",
        include_str!("fixtures/sidebar/huge-setext.md"),
    ),
];

fn parse(source: &str) -> mdtui3::Document {
    let mut document = Utf8Document::new();
    document.append(source.as_bytes()).unwrap();
    document.finish();
    document.into_document()
}

fn row_text(screen: &Screen, row: u16) -> String {
    let content = screen.content_area();
    let mut text = String::new();
    for column in content.x..content.right() {
        let cell = screen.cell(row, column).expect("row cell must exist");
        text.push_str(cell.text());
    }
    text.truncate(text.trim_end_matches(' ').len());
    text.replace('\t', "\\t")
}

#[test]
fn all_fixtures_have_stable_cell_snapshots_at_all_target_widths() {
    assert_eq!(FIXTURES.len(), 19);
    assert_eq!(FIXTURES.len() * WIDTHS.len(), 76);
    let settings = snapshot_settings();
    settings.bind(|| {
        for (name, source) in FIXTURES {
            let document = parse(source);
            let mut snapshot = String::new();
            for width in WIDTHS {
                let rendered = render_static(&document, width, HEIGHT)
                    .unwrap_or_else(|error| panic!("{name} at width {width}: {error}"));
                writeln!(
                    snapshot,
                    "==== width={width} rows={} content={} blank={} groups={} math={} ====",
                    rendered.stats.rows,
                    rendered.stats.content_rows,
                    rendered.stats.blank_rows,
                    rendered.stats.groups,
                    rendered.stats.math_blocks_skipped,
                )
                .unwrap();
                snapshot.push_str(&dump_render(&rendered));
            }
            insta::assert_snapshot!(name.to_string(), snapshot);
        }
    });
}

#[test]
fn spacing_fixture_stats_match_p25_oracle() {
    let document = parse(include_str!("fixtures/spec/spacing.md"));
    let rendered = render_static(&document, 80, HEIGHT).unwrap();
    assert_eq!(
        rendered.stats,
        RenderStats {
            rows: 41,
            content_rows: 29,
            blank_rows: 12,
            groups: 19,
            math_blocks_skipped: 0,
        }
    );
}

#[test]
fn row_overflow_is_reported_by_the_static_api() {
    let document = parse("one\n\ntwo\n");
    let error = render_static(&document, 20, 1).unwrap_err();
    assert!(error.is_row_overflow(), "unexpected error: {error}");
}

fn dump_render(render: &StaticRender) -> String {
    let mut output = String::new();
    for row in 0..render.stats.rows {
        write!(output, "{row:04} |{}|", row_text(&render.screen, row)).unwrap();
        let styles = style_ranges(&render.screen, row);
        if !styles.is_empty() {
            write!(output, " styles={}", styles.join(",")).unwrap();
        }
        let links = link_ranges(&render.screen, row);
        if !links.is_empty() {
            write!(output, " links={}", links.join(",")).unwrap();
        }
        let wide = wide_cells(&render.screen, row);
        if !wide.is_empty() {
            write!(output, " wide={}", wide.join(",")).unwrap();
        }
        output.push('\n');
    }
    output
}

fn style_ranges(screen: &Screen, row: u16) -> Vec<String> {
    let content = screen.content_area();
    let mut ranges: Vec<(u16, u16, String)> = Vec::new();
    for column in content.x..content.right() {
        let cell = screen.cell(row, column).expect("row cell must exist");
        let Some(label) = style_label(cell.style()) else {
            continue;
        };
        if let Some((_, end, previous)) = ranges.last_mut()
            && *end == column
            && *previous == label
        {
            *end += 1;
        } else {
            ranges.push((column, column + 1, label));
        }
    }
    ranges
        .into_iter()
        .map(|(start, end, label)| format!("{start}-{end}:{label}"))
        .collect()
}

fn link_ranges(screen: &Screen, row: u16) -> Vec<String> {
    let content = screen.content_area();
    let mut ranges: Vec<(u16, u16, String)> = Vec::new();
    for column in content.x..content.right() {
        let cell = screen.cell(row, column).expect("row cell must exist");
        let Some(link) = cell.link_id() else {
            continue;
        };
        let label = format!("{link:?}");
        if let Some((_, end, previous)) = ranges.last_mut()
            && *end == column
            && *previous == label
        {
            *end += 1;
        } else {
            ranges.push((column, column + 1, label));
        }
    }
    ranges
        .into_iter()
        .map(|(start, end, label)| format!("{start}-{end}:{label}"))
        .collect()
}

fn wide_cells(screen: &Screen, row: u16) -> Vec<String> {
    let content = screen.content_area();
    let mut cells = Vec::new();
    for column in content.x..content.right() {
        let cell = screen.cell(row, column).expect("row cell must exist");
        if cell.width() == 2 {
            cells.push(format!("base@{column}:2"));
        } else if cell.is_continuation() {
            cells.push(format!("cont@{column}"));
        }
    }
    cells
}

fn style_label(style: StyleRole) -> Option<String> {
    let mut labels = Vec::new();
    if let Some(level) = style.heading_level() {
        labels.push(format!("H{level}"));
    }
    for (flag, label) in [
        (StyleRole::STRONG, "strong"),
        (StyleRole::EMPHASIS, "emphasis"),
        (StyleRole::STRIKE, "strike"),
        (StyleRole::CODE, "code"),
        (StyleRole::MATH, "math"),
        (StyleRole::LINK, "link"),
        (StyleRole::QUOTE, "quote"),
        (StyleRole::LIST_MARKER, "list-marker"),
        (StyleRole::RULE, "rule"),
        (StyleRole::TABLE_BORDER, "table-border"),
        (StyleRole::META, "meta"),
        (StyleRole::TASK_CHECKED, "task-checked"),
    ] {
        if style.contains(flag) {
            labels.push(label.to_owned());
        }
    }
    (!labels.is_empty()).then(|| labels.join("+"))
}
