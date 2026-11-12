use mdtui_tui::render::plan::{PlanEventKind, TileSegmentKind, blank_lines_between, plan_visible};
use mdtui_tui::static_render::render_static;
use mdtui3::{BlockId, BlockKind, Document, Span, TILE_BYTES, TileId};

fn parse(source: &str) -> Document {
    let mut document = Document::new();
    document.append(source).unwrap();
    document.finish();
    document
}

fn plan_all(document: &Document) -> mdtui_tui::render::plan::BlockPlan {
    plan_visible(document, Span::new(0, document.source().len()), 0).unwrap()
}

fn static_lines(document: &Document, width: u16, height: u16) -> Vec<String> {
    let rendered = render_static(document, width, height).unwrap();
    let content = rendered.screen.content_area();
    (0..rendered.stats.rows)
        .map(|row| {
            let mut text = String::new();
            for column in content.x..content.right() {
                text.push_str(rendered.screen.cell(row, column).unwrap().text());
            }
            text.truncate(text.trim_end_matches(' ').len());
            text
        })
        .collect()
}

fn code_block_ids(document: &Document) -> Vec<BlockId> {
    document
        .blocks()
        .iter()
        .enumerate()
        .filter(|(_, block)| matches!(block.kind, BlockKind::Code { .. }))
        .map(|(index, _)| BlockId(index))
        .collect()
}

#[test]
fn empty_info_fence_is_tui_owned_and_plans_its_label() {
    let source = "```rust\n```\n";
    let document = parse(source);
    let plan = plan_all(&document);
    let code = code_block_ids(&document)[0];
    let block = document.block(code).unwrap();
    let tile = document.tile_at(block.source.start).unwrap();
    let segment = plan.segment(tile).unwrap();

    assert_eq!(segment.kind, TileSegmentKind::TuiOwned);
    assert!(segment.blocks.contains(&code));
    let labels: Vec<_> = plan
        .events
        .iter()
        .filter_map(|event| match &event.kind {
            PlanEventKind::CodeLabel { label } => Some(label.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(labels, ["[rust]"]);
    assert_partition_with_static(&plan, &document, 80, 4, &["[rust]"]);
}

#[test]
fn same_tile_paragraph_and_empty_fence_keep_source_order_and_spacing() {
    let source = "before\n```rust\n```\nafter\n";
    let document = parse(source);
    let plan = plan_all(&document);
    let code = code_block_ids(&document)[0];
    let tile = document.tile_at(0).unwrap();
    let segment = plan.segment(tile).unwrap();

    assert_eq!(segment.kind, TileSegmentKind::Drawable);
    assert_eq!(
        segment.blocks,
        (0..document.blocks().len())
            .map(BlockId)
            .collect::<Vec<_>>()
    );
    assert!(segment.blocks.contains(&code));
    let event_kinds: Vec<_> = plan.events.iter().map(|event| &event.kind).collect();
    assert_eq!(
        event_kinds,
        [
            &PlanEventKind::Spacing { blank_lines: 1 },
            &PlanEventKind::CodeLabel {
                label: "[rust]".to_owned()
            },
            &PlanEventKind::Spacing { blank_lines: 1 },
        ]
    );
    assert_partition_with_static(
        &plan,
        &document,
        80,
        6,
        &["before", "", "[rust]", "", "after"],
    );
    assert_eq!(
        render_static(&document, 80, 6).unwrap().stats.blank_rows,
        plan.total_blank_rows()
    );
}

#[test]
fn visible_paragraph_and_heading_tiles_are_drawable() {
    let document = parse("# Heading\n\nparagraph\n");
    let plan = plan_all(&document);
    assert!(!plan.segments.is_empty());
    assert!(
        plan.segments
            .iter()
            .all(|segment| segment.kind == TileSegmentKind::Drawable)
    );
}

#[test]
fn pure_definition_tile_is_zero_row_tui_owned() {
    let source = "[d]: https://example.com\n";
    let document = parse(source);
    let plan = plan_all(&document);
    let block = document
        .blocks()
        .iter()
        .position(|block| matches!(block.kind, BlockKind::Definition { .. }))
        .map(BlockId)
        .expect("definition block");
    let tile = document
        .tile_at(document.block(block).unwrap().source.start)
        .unwrap();
    let segment = plan.segment(tile).unwrap();

    assert_eq!(segment.kind, TileSegmentKind::ZeroRow);
    assert!(segment.blocks.contains(&block));
    assert!(plan.events_for_tile(tile).next().is_none());
}

#[test]
fn empty_alt_image_object_keeps_the_tile_drawable() {
    let document = parse("![](image.png)\n");
    let plan = plan_all(&document);
    let segment = &plan.segments[0];
    assert_eq!(
        segment.drawable_at,
        document.next_drawable(segment.source.start)
    );
    assert!(
        segment
            .drawable_at
            .is_some_and(|position| position < segment.source.end)
    );
    assert_eq!(segment.kind, TileSegmentKind::Drawable);
}

#[test]
fn requested_range_and_overscan_limit_planned_blocks_and_tiles() {
    let prefix = "A".repeat(TILE_BYTES * 2);
    let middle = "M".repeat(TILE_BYTES * 3);
    let suffix = "Z".repeat(TILE_BYTES * 2);
    let source = format!("{prefix}\n\n{middle}\n\n{suffix}\n");
    let document = parse(&source);
    let middle_start = source.find(&middle).unwrap();
    let middle_block = document.block_at(middle_start).unwrap();
    let prefix_block = document.block_at(0).unwrap();
    let suffix_block = document.block_at(source.len() - 1).unwrap();
    let visible_tile = document.tile_at(middle_start + TILE_BYTES + 1).unwrap();
    let visible = document.tile(visible_tile).unwrap();

    let tight = plan_visible(&document, visible, 0).unwrap();
    assert_eq!(tight.tile_start, visible_tile);
    assert_eq!(tight.tile_end, TileId(visible_tile.0 + 1));
    assert_eq!(tight.segments.len(), 1);
    assert_eq!(tight.segments[0].source, visible);
    assert_eq!(tight.blocks.len(), 1);
    assert_eq!(tight.blocks[0].id, middle_block);
    assert!(!tight.blocks[0].group_start);
    assert!(tight.blocks.iter().all(|block| block.id == middle_block));
    assert!(!tight.blocks.iter().any(|block| block.id == prefix_block));
    assert!(!tight.blocks.iter().any(|block| block.id == suffix_block));

    let overscan = plan_visible(&document, visible, 1).unwrap();
    assert_eq!(
        (overscan.tile_start.0, overscan.tile_end.0),
        (visible_tile.0 - 1, visible_tile.0 + 2)
    );
    assert_eq!(overscan.segments.len(), 3);
    for (index, segment) in overscan.segments.iter().enumerate() {
        assert_eq!(segment.tile.0, overscan.tile_start.0 + index);
        assert_eq!(segment.source, document.tile(segment.tile).unwrap());
    }
    assert!(overscan.blocks.iter().any(|block| block.id == middle_block));
    assert!(!overscan.blocks.iter().any(|block| block.id == suffix_block));
}

#[test]
fn hidden_block_fixture_plans_all_labels_and_matches_static_text() {
    let source = include_str!("fixtures/stream/hidden-blocks.md");
    let document = parse(source);
    let plan = plan_all(&document);
    let labels: Vec<_> = plan
        .events
        .iter()
        .filter_map(|event| match &event.kind {
            PlanEventKind::CodeLabel { label } => Some(label.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(labels, ["[rust]", "[python]", "[text]"]);

    let lines = static_lines(&document, 80, 100);
    assert_eq!(
        render_static(&document, 80, 100).unwrap().stats.blank_rows,
        plan.total_blank_rows()
    );
    for label in labels {
        assert!(lines.iter().any(|line| line == label), "missing {label:?}");
    }
    assert_eq!(
        plan.blocks
            .iter()
            .filter(|block| matches!(block.kind, BlockKind::Code { .. }))
            .count(),
        3
    );
}

#[test]
fn spacing_events_match_p25_oracle_and_static_blank_rows() {
    let source = include_str!("fixtures/spec/spacing.md");
    let document = parse(source);
    let plan = plan_all(&document);
    assert_eq!(plan.groups.len(), 19);
    assert_eq!(plan.total_blank_rows(), 12);
    for pair in plan.groups.windows(2) {
        assert_eq!(
            pair[1].spacing_before,
            blank_lines_between(pair[0].class, pair[1].class)
        );
    }
    let rendered = render_static(&document, 80, 512).unwrap();
    assert_eq!(rendered.stats.blank_rows, plan.total_blank_rows());
}

#[test]
fn thematic_break_event_is_shared_with_static_rendering() {
    let document = parse("---\n");
    let plan = plan_all(&document);
    assert!(
        plan.events.iter().any(|event| {
            event.kind == PlanEventKind::ThematicBreak && event.block == BlockId(0)
        })
    );
    let lines = static_lines(&document, 10, 2);
    assert_eq!(lines, ["──────────"]);
}

#[test]
fn plan_is_revision_bound() {
    let mut document = Document::new();
    document.append("one\n").unwrap();
    let plan = plan_all(&document);
    assert!(plan.matches(&document));
    document.append("two\n").unwrap();
    assert!(!plan.matches(&document));
}

fn assert_partition_with_static(
    plan: &mdtui_tui::render::plan::BlockPlan,
    document: &Document,
    width: u16,
    height: u16,
    expected_lines: &[&str],
) {
    let labels: Vec<_> = plan
        .events
        .iter()
        .filter_map(|event| match &event.kind {
            PlanEventKind::CodeLabel { label } => Some(label.as_str()),
            _ => None,
        })
        .collect();
    let lines = static_lines(document, width, height);
    for label in labels {
        assert_eq!(lines.iter().filter(|line| *line == label).count(), 1);
    }
    assert_eq!(lines, expected_lines);
}
