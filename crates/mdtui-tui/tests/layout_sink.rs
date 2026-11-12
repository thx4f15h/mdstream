use std::mem::size_of;

use mdtui_tui::render::checkpoint::{CheckpointPosition, RenderCheckpointStore};
use mdtui_tui::render::display::DisplayScanner;
use mdtui_tui::render::plan::{BlockPlan, TileSegmentKind, plan_visible};
use mdtui_tui::render::sink::TileLayoutSink;
use mdtui_tui::screen::{ContentArea, Screen};
use mdtui_tui::static_render::render_static;
use mdtui3::{
    Budget, Completion, Document, HeightIndex, LayoutPass, LayoutSink, Span, TILE_BYTES, TileId,
    WorkLimit,
};

fn parse(source: &str) -> Document {
    let mut document = Document::new();
    document.append(source).unwrap();
    document.finish();
    document
}

fn unlimited() -> Budget {
    Budget::new(WorkLimit::unlimited())
}

fn plan_source(document: &Document, plan: &BlockPlan) -> Span {
    if plan.tile_start.0 == plan.tile_end.0 {
        return Span::new(0, 0);
    }
    let start = document.tile(plan.tile_start).unwrap().start;
    let end = document.tile(TileId(plan.tile_end.0 - 1)).unwrap().end;
    Span::new(start, end)
}

fn prepare(document: &Document, width: u16) -> (BlockPlan, TileLayoutSink) {
    let plan = plan_visible(document, Span::new(0, document.source().len()), 0).unwrap();
    let span = plan_source(document, &plan);
    let mut scanner = DisplayScanner::new(document, span).unwrap();
    assert_eq!(
        scanner.advance(document, &mut unlimited()),
        Ok(mdtui_tui::render::display::ScanStatus::Complete)
    );
    let sink = TileLayoutSink::new(document, &plan, &scanner, width).unwrap();
    (plan, sink)
}

fn render_all_tiles(plan: &BlockPlan, sink: &mut TileLayoutSink) -> Vec<(TileId, u64)> {
    let mut rows = Vec::new();
    for segment in &plan.segments {
        let count = match segment.kind {
            TileSegmentKind::Drawable => {
                sink.begin_tile(segment.tile);
                sink.end_tile(segment.tile)
            }
            TileSegmentKind::TuiOwned | TileSegmentKind::ZeroRow => {
                sink.render_tui_owned_tile(segment.tile)
            }
        };
        rows.push((segment.tile, count));
    }
    rows
}

fn assert_static_equivalent(sink: &TileLayoutSink, document: &Document, width: u16, height: u16) {
    let expected = render_static(document, width, height).unwrap();
    assert_eq!(sink.lines().len(), usize::from(expected.stats.rows));
    let rows = u16::try_from(sink.lines().len())
        .unwrap()
        .max(expected.stats.rows)
        .max(1);
    let mut actual = Screen::new(rows, width, ContentArea::new(0, width)).unwrap();
    assert_eq!(
        sink.write_to_screen(&mut actual, 0),
        Ok(expected.stats.rows)
    );
    for row in 0..expected.stats.rows {
        assert_eq!(
            actual.line_source(row),
            expected.screen.line_source(row),
            "row {row}"
        );
        for column in 0..width {
            assert_eq!(
                actual.cell(row, column),
                expected.screen.cell(row, column),
                "row {row} column {column}"
            );
        }
    }
}

#[test]
fn single_tile_paragraph_matches_static_cell_grid() {
    let document = parse("hello **world** and `code`\n");
    let (plan, mut sink) = prepare(&document, 40);
    let rows = render_all_tiles(&plan, &mut sink);
    assert_eq!(rows.iter().map(|(_, rows)| rows).sum::<u64>(), 1);
    assert_static_equivalent(&sink, &document, 40, 8);
}

#[test]
fn cross_tile_line_is_counted_only_by_its_ending_tile_and_keeps_line_source() {
    let source = format!("{}\n", "a".repeat(TILE_BYTES * 2 + 10));
    let document = parse(&source);
    let (plan, mut sink) = prepare(&document, 80);
    assert!(plan.segments.len() >= 3);
    let mut rows_per_tile = Vec::new();
    let mut checkpoint_after_first = None;
    for (index, segment) in plan.segments.iter().enumerate() {
        let count = if segment.kind == TileSegmentKind::Drawable {
            sink.begin_tile(segment.tile);
            sink.end_tile(segment.tile)
        } else {
            sink.render_tui_owned_tile(segment.tile)
        };
        rows_per_tile.push(count);
        if index == 0 {
            checkpoint_after_first = Some(sink.checkpoint());
        }
    }
    assert!(
        rows_per_tile[..rows_per_tile.len() - 1]
            .iter()
            .all(|rows| *rows == 0)
    );
    assert!(*rows_per_tile.last().unwrap() > 0);

    let mut actual = Screen::new(
        rows_per_tile.iter().sum::<u64>() as u16,
        80,
        ContentArea::new(0, 80),
    )
    .unwrap();
    sink.write_to_screen(&mut actual, 0).unwrap();
    assert_eq!(
        actual.line_source(0),
        Some(mdtui_tui::screen::LineSource::new(
            Span::new(0, 1),
            mdtui3::BlockId(0)
        ))
    );
    assert_static_equivalent(&sink, &document, 80, 16);

    let checkpoint = checkpoint_after_first.unwrap();
    assert_eq!(checkpoint.position, CheckpointPosition::BlockInner);
    assert_eq!(checkpoint.block, Some(mdtui3::BlockId(0)));
    let mut store = RenderCheckpointStore::new(&document);
    store
        .checkpoint_after_tile(TileId(0), checkpoint, &document)
        .unwrap();
    assert_eq!(store.exact(TileId(1)), Some(&checkpoint));
}

#[test]
fn same_tile_paragraph_and_empty_fence_merge_in_source_order() {
    let document = parse("before\n```rust\n```\nafter\n");
    let (plan, mut sink) = prepare(&document, 40);
    let rows = render_all_tiles(&plan, &mut sink);
    assert_eq!(rows.iter().map(|(_, rows)| rows).sum::<u64>(), 5);
    assert_eq!(sink.stats().events_consumed, 3);
    assert_static_equivalent(&sink, &document, 40, 12);
}

#[test]
fn tui_owned_empty_info_fence_returns_label_row() {
    let document = parse("```rust\n```\n");
    let (plan, mut sink) = prepare(&document, 40);
    assert_eq!(plan.segments[0].kind, TileSegmentKind::TuiOwned);
    assert_eq!(sink.render_tui_owned_tile(TileId(0)), 1);
    assert_static_equivalent(&sink, &document, 40, 4);
}

#[test]
fn heading_code_table_and_list_match_static_cells() {
    for (source, width, height) in [
        ("# Heading **bold**\n", 40, 8),
        (
            "> quoted text that wraps across the narrow content width\n",
            20,
            12,
        ),
        ("---\n", 20, 4),
        ("```rust\nline one\nline two\n```\n", 40, 12),
        ("| A | B |\n| --- | --- |\n| 1 | 2 |\n", 40, 12),
        ("- first item\n- second item\n", 40, 12),
    ] {
        let document = parse(source);
        let (plan, mut sink) = prepare(&document, width);
        render_all_tiles(&plan, &mut sink);
        assert_static_equivalent(&sink, &document, width, height);
    }
}

#[test]
fn links_objects_and_wide_cells_survive_layout() {
    let document = parse("a [link](https://example.com) 🙂 end\n");
    let (plan, mut sink) = prepare(&document, 40);
    render_all_tiles(&plan, &mut sink);
    assert_static_equivalent(&sink, &document, 40, 8);
    let mut screen = Screen::new(1, 40, ContentArea::new(0, 40)).unwrap();
    let rows = u16::try_from(sink.lines().len()).unwrap();
    sink.write_to_screen(&mut screen, 0).unwrap();
    assert!(
        (0..rows).any(|row| (0..40).any(|column| screen
            .cell(row, column)
            .unwrap()
            .link_id()
            .is_some()))
    );
    assert!(
        (0..rows).any(|row| (0..40).any(|column| screen.cell(row, column).unwrap().width() == 2))
    );
}

#[test]
fn layout_pass_callbacks_do_not_query_document_or_duplicate_segments() {
    let document = parse(&format!(
        "a &amp; b <https://example.com> `a  \t b` 🙂\n\n{}\n",
        "word ".repeat(100)
    ));
    let (plan, mut sink) = prepare(&document, 40);
    let mut heights = HeightIndex::default();
    heights.rebuild(&document).unwrap();
    let mut pass = LayoutPass::new(&document, &heights, plan.tile_start, u64::MAX, false).unwrap();
    assert_eq!(
        pass.advance(&document, &mut heights, &mut unlimited(), &mut sink)
            .unwrap(),
        Completion::EndOfDocument
    );
    let stats = sink.stats();
    assert!(stats.begin_tile_callbacks > 0);
    assert!(stats.raw_text_callbacks > 0);
    assert!(stats.object_callbacks > 0);
    assert!(stats.segments_consumed > 0);
    assert_eq!(stats.document_queries_in_callbacks, 0);
    assert_eq!(stats.rows, sink.lines().len() as u64);
    assert!(size_of::<TileLayoutSink>() > 0);
    assert_static_equivalent(&sink, &document, 40, 64);
}

#[test]
fn long_code_line_is_counted_at_the_tile_containing_its_line_end() {
    let code = "c".repeat(TILE_BYTES * 2 + 31);
    let source = format!("```\n{code}\n```\n");
    let document = parse(&source);
    let line_start = source.find(&code).unwrap();
    let line_end = line_start + code.len();
    let owner = document.tile_at(line_end).unwrap();
    assert!(owner.0 >= 2);

    let (plan, mut sink) = prepare(&document, 120);
    let rows = render_all_tiles(&plan, &mut sink);
    assert_eq!(rows.iter().map(|(_, rows)| rows).sum::<u64>(), 1);
    for (tile, count) in rows {
        assert_eq!(count, u64::from(tile == owner), "tile {tile:?}");
    }
    assert_static_equivalent(&sink, &document, 120, 8);
}

#[test]
fn each_code_line_is_owned_by_its_own_ending_tile() {
    let first = "a".repeat(TILE_BYTES + 17);
    let second = "b".repeat(TILE_BYTES + 33);
    let source = format!("```\n{first}\n{second}\n```\n");
    let document = parse(&source);
    let first_start = source.find(&first).unwrap();
    let second_start = source.find(&second).unwrap();
    let first_owner = document.tile_at(first_start + first.len()).unwrap();
    let second_owner = document.tile_at(second_start + second.len()).unwrap();
    assert!(first_owner < second_owner);

    let (plan, mut sink) = prepare(&document, 120);
    let rows = render_all_tiles(&plan, &mut sink);
    let first_rows = rows
        .iter()
        .find(|(tile, _)| *tile == first_owner)
        .map_or(0, |(_, rows)| *rows);
    let second_rows = rows
        .iter()
        .find(|(tile, _)| *tile == second_owner)
        .map_or(0, |(_, rows)| *rows);
    assert_eq!((first_rows, second_rows), (1, 1));
    assert_eq!(rows.iter().map(|(_, rows)| rows).sum::<u64>(), 2);
    assert_static_equivalent(&sink, &document, 120, 8);
}

#[test]
fn table_partition_rows_follow_header_and_body_line_end_tiles() {
    let source = format!(
        "| {} | {} |\n| --- | --- |\n| {} | {} |\n| {} | {} |\n",
        "H".repeat(TILE_BYTES + 11),
        "I".repeat(TILE_BYTES + 13),
        "a".repeat(TILE_BYTES + 17),
        "b".repeat(TILE_BYTES + 19),
        "c".repeat(TILE_BYTES + 23),
        "d".repeat(TILE_BYTES + 29),
    );
    let document = parse(&source);
    let table_block = document
        .blocks()
        .iter()
        .find_map(|block| match block.kind {
            mdtui3::BlockKind::Table { id } => Some((block, id)),
            _ => None,
        })
        .expect("table block");
    let table = document.table(table_block.1).unwrap();
    assert_eq!(table.rows().len(), 3);
    let header_owner = document
        .tile_at(table.rows()[0].source.end.saturating_sub(1))
        .unwrap();
    let body1_owner = document
        .tile_at(table.rows()[1].source.end.saturating_sub(1))
        .unwrap();
    let body2_owner = document
        .tile_at(table.rows()[2].source.end.saturating_sub(1))
        .unwrap();
    assert!(header_owner < body1_owner && body1_owner < body2_owner);

    let (plan, mut sink) = prepare(&document, 120);
    let rows = render_all_tiles(&plan, &mut sink);
    let count = |tile| {
        rows.iter()
            .find(|(candidate, _)| *candidate == tile)
            .map_or(0, |(_, rows)| *rows)
    };
    assert_eq!(count(header_owner), 3);
    assert_eq!(count(body1_owner), 1);
    assert_eq!(count(body2_owner), 2);
    assert_eq!(rows.iter().map(|(_, rows)| rows).sum::<u64>(), 6);
    assert_static_equivalent(&sink, &document, 120, 16);
}

#[test]
fn list_item_continuations_are_owned_by_the_item_container_end_tile() {
    let continuation = "word ".repeat(160);
    let source = format!("- first\n  {continuation}\n");
    let document = parse(&source);
    let item = document
        .blocks()
        .iter()
        .find(|block| matches!(block.kind, mdtui3::BlockKind::ListItem { .. }))
        .expect("list item");
    let container = item.container.expect("list item container");
    let container_end = document.container(container).unwrap().source.end;
    let owner = document.tile_at(container_end.saturating_sub(1)).unwrap();
    assert!(owner.0 >= 2);

    let (plan, mut sink) = prepare(&document, 40);
    let rows = render_all_tiles(&plan, &mut sink);
    for (tile, count) in &rows {
        if *tile == owner {
            assert!(*count > 0);
        } else {
            assert_eq!(*count, 0, "tile {tile:?}");
        }
    }
    assert_static_equivalent(&sink, &document, 40, 64);
}
