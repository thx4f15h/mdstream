use std::fmt;

use mdtui3::{Block, BlockId, BlockKind, ContainerKind, Document, Heading};

use crate::render::code::{CodeError, render_code_block};
use crate::render::heading::{HeadingError, render_heading};
use crate::render::list::{ListError, render_list_item};
use crate::render::paragraph::{ParagraphError, render_paragraph, render_quote_paragraph};
pub use crate::render::plan::{GroupClass, blank_lines_between};
use crate::render::plan::{GroupKey, group_identity};
use crate::render::table::{TableError, render_table};
use crate::render::text::TextError;
use crate::render::thematic::{ThematicError, is_thematic_break_block, render_thematic_break};
use crate::render::visibility::is_hidden_block;
use crate::screen::{Screen, ScreenError};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RenderStats {
    pub rows: u16,
    pub content_rows: u16,
    pub blank_rows: u16,
    pub groups: usize,
    pub math_blocks_skipped: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StaticRenderError {
    Prepared(String),
    MissingHeading(BlockId),
    InvalidContainer(BlockId),
    RowOverflow,
    ColumnOverflow,
    Heading(HeadingError),
    Paragraph(ParagraphError),
    List(ListError),
    Code(CodeError),
    Table(TableError),
    Thematic(ThematicError),
    Screen(ScreenError),
}

impl fmt::Display for StaticRenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Prepared(error) => formatter.write_str(error),
            Self::MissingHeading(block) => {
                write!(formatter, "heading block {} is missing", block.0)
            }
            Self::InvalidContainer(block) => {
                write!(formatter, "block {} has an invalid container", block.0)
            }
            Self::RowOverflow => formatter.write_str("static output exceeds terminal rows"),
            Self::ColumnOverflow => {
                formatter.write_str("static output exceeds the supported column width")
            }
            Self::Heading(error) => error.fmt(formatter),
            Self::Paragraph(error) => error.fmt(formatter),
            Self::List(error) => error.fmt(formatter),
            Self::Code(error) => error.fmt(formatter),
            Self::Table(error) => error.fmt(formatter),
            Self::Thematic(error) => error.fmt(formatter),
            Self::Screen(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for StaticRenderError {}

impl StaticRenderError {
    #[must_use]
    pub fn is_row_overflow(&self) -> bool {
        matches!(
            self,
            Self::RowOverflow
                | Self::Screen(ScreenError::RowOutOfBounds(_))
                | Self::Heading(
                    HeadingError::Text(TextError::Screen(ScreenError::RowOutOfBounds(_)))
                        | HeadingError::Screen(ScreenError::RowOutOfBounds(_)),
                )
                | Self::Paragraph(
                    ParagraphError::RowOverflow
                        | ParagraphError::Text(TextError::Screen(ScreenError::RowOutOfBounds(_)))
                        | ParagraphError::Screen(ScreenError::RowOutOfBounds(_)),
                )
                | Self::List(
                    ListError::RowOverflow
                        | ListError::Text(TextError::Screen(ScreenError::RowOutOfBounds(_)))
                        | ListError::Screen(ScreenError::RowOutOfBounds(_)),
                )
                | Self::Code(
                    CodeError::RowOverflow
                        | CodeError::Text(TextError::Screen(ScreenError::RowOutOfBounds(_)))
                        | CodeError::Screen(ScreenError::RowOutOfBounds(_)),
                )
                | Self::Table(
                    TableError::RowOverflow | TableError::Screen(ScreenError::RowOutOfBounds(_)),
                )
                | Self::Thematic(ThematicError::Screen(ScreenError::RowOutOfBounds(_)))
        )
    }
}

macro_rules! impl_from_render_error {
    ($variant:ident, $error:ty) => {
        impl From<$error> for StaticRenderError {
            fn from(error: $error) -> Self {
                Self::$variant(error)
            }
        }
    };
}

impl_from_render_error!(Heading, HeadingError);
impl_from_render_error!(Paragraph, ParagraphError);
impl_from_render_error!(List, ListError);
impl_from_render_error!(Code, CodeError);
impl_from_render_error!(Table, TableError);
impl_from_render_error!(Thematic, ThematicError);
impl_from_render_error!(Screen, ScreenError);

pub fn render_static_screen(
    document: &Document,
    screen: &mut Screen,
) -> Result<RenderStats, StaticRenderError> {
    screen.clear_back();
    if document.math_object_stats().active > 0 {
        use super::first_frame::{FirstFrame, FirstFrameConfig, FirstFrameStatus};
        let config = FirstFrameConfig {
            top_tile: mdtui3::TileId(0),
            tile_limit: document.tile_count().max(1),
            width: screen.render_width(),
            row_limit: u64::from(screen.rows()) + 1,
        };
        let mut first = FirstFrame::new(document, config)
            .map_err(|e| StaticRenderError::Prepared(e.to_string()))?;
        loop {
            let step = first
                .advance(
                    document,
                    &mut mdtui3::Budget::new(mdtui3::WorkLimit::unlimited()),
                )
                .map_err(|e| StaticRenderError::Prepared(e.to_string()))?;
            if matches!(
                step.status,
                FirstFrameStatus::Covered | FirstFrameStatus::EndOfDocument
            ) {
                break;
            }
        }
        if first.rows() > u64::from(screen.rows()) {
            return Err(StaticRenderError::RowOverflow);
        }
        first
            .write_to_screen(screen, 0)
            .map_err(|e| StaticRenderError::Prepared(e.to_string()))?;
        let rows = first.rows() as u16;
        let content_rows = first.lines().iter().filter(|l| l.source.is_some()).count() as u16;
        return Ok(RenderStats {
            rows,
            content_rows,
            blank_rows: rows - content_rows,
            groups: first.plan().blocks.len(),
            math_blocks_skipped: 0,
        });
    }
    let groups = collect_groups(document)?;
    let mut stats = RenderStats::default();
    let mut previous = None;

    for group in groups {
        if let Some(previous) = previous {
            advance_rows(
                &mut stats,
                blank_lines_between(previous, group.class),
                screen.rows(),
            )?;
        }
        let start_row = stats.rows;
        let rendered = render_group(
            document,
            screen,
            start_row,
            &group,
            &mut stats.math_blocks_skipped,
        )?;
        stats.content_rows = stats
            .content_rows
            .checked_add(rendered)
            .ok_or(StaticRenderError::RowOverflow)?;
        stats.rows = stats
            .rows
            .checked_add(rendered)
            .ok_or(StaticRenderError::RowOverflow)?;
        if stats.rows > screen.rows() {
            return Err(StaticRenderError::RowOverflow);
        }
        stats.groups = stats
            .groups
            .checked_add(1)
            .ok_or(StaticRenderError::RowOverflow)?;
        previous = Some(group.class);
    }

    Ok(stats)
}

fn advance_rows(
    stats: &mut RenderStats,
    rows: u16,
    available: u16,
) -> Result<(), StaticRenderError> {
    stats.blank_rows = stats
        .blank_rows
        .checked_add(rows)
        .ok_or(StaticRenderError::RowOverflow)?;
    stats.rows = stats
        .rows
        .checked_add(rows)
        .ok_or(StaticRenderError::RowOverflow)?;
    if stats.rows > available {
        return Err(StaticRenderError::RowOverflow);
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BlockGroup {
    key: GroupKey,
    class: GroupClass,
    start: usize,
    end: usize,
}

fn collect_groups(document: &Document) -> Result<Vec<BlockGroup>, StaticRenderError> {
    let mut groups: Vec<BlockGroup> = Vec::new();
    for (index, block) in document.blocks().iter().enumerate() {
        if is_hidden_block(block) || block.kind == BlockKind::Blank {
            continue;
        }
        let block_id = BlockId(index);
        let (key, class) = group_identity(document, block_id, block)
            .ok_or(StaticRenderError::InvalidContainer(block_id))?;
        if let Some(group) = groups.last_mut()
            && group.key == key
        {
            group.end = index + 1;
            continue;
        }
        groups.push(BlockGroup {
            key,
            class,
            start: index,
            end: index + 1,
        });
    }
    Ok(groups)
}

fn render_group(
    document: &Document,
    screen: &mut Screen,
    start_row: u16,
    group: &BlockGroup,
    math_blocks_skipped: &mut usize,
) -> Result<u16, StaticRenderError> {
    let mut rows = 0_u16;
    for (index, block) in document.blocks()[group.start..group.end].iter().enumerate() {
        let block_id = BlockId(group.start + index);
        let row = start_row
            .checked_add(rows)
            .ok_or(StaticRenderError::RowOverflow)?;
        let rendered = match block.kind {
            BlockKind::Paragraph => {
                if is_item_block(document, block) {
                    continue;
                }
                render_paragraph(document, block_id, block, screen, row)?
            }
            BlockKind::Heading { .. } => {
                let heading = heading_for_block(document, block_id)
                    .ok_or(StaticRenderError::MissingHeading(block_id))?;
                render_heading(document, heading, screen, row)?
            }
            BlockKind::Quote { .. } => {
                if is_thematic_break_block(document, block)? {
                    render_thematic_break(document, block_id, block, screen, row)?
                } else {
                    render_quote_paragraph(document, block_id, block, screen, row)?
                }
            }
            BlockKind::ListItem { .. } => render_list_item(document, block_id, block, screen, row)?,
            BlockKind::Code { .. } => render_code_block(document, block_id, block, screen, row)?,
            BlockKind::Table { .. } => render_table(document, block_id, block, screen, row)?,
            BlockKind::ThematicBreak => {
                render_thematic_break(document, block_id, block, screen, row)?
            }
            BlockKind::Math { .. } => {
                *math_blocks_skipped = math_blocks_skipped
                    .checked_add(1)
                    .ok_or(StaticRenderError::RowOverflow)?;
                0
            }
            BlockKind::Blank
            | BlockKind::Definition { .. }
            | BlockKind::FootnoteDefinition { .. } => 0,
        };
        rows = rows
            .checked_add(rendered)
            .ok_or(StaticRenderError::RowOverflow)?;
    }
    Ok(rows)
}

fn heading_for_block(document: &Document, block_id: BlockId) -> Option<&Heading> {
    let headings = document.headings().entries();
    let index = headings.partition_point(|heading| heading.block < block_id);
    headings
        .get(index)
        .filter(|heading| heading.block == block_id)
}

fn is_item_block(document: &Document, block: &Block) -> bool {
    block
        .container
        .and_then(|container| document.container(container))
        .is_some_and(|container| matches!(container.kind, ContainerKind::Item { .. }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen::ContentArea;

    fn parse(source: &str) -> Document {
        let mut document = Document::new();
        document.append(source).unwrap();
        document.finish();
        document
    }

    fn render(source: &str, rows: u16, cols: u16) -> (Document, Screen, RenderStats) {
        let document = parse(source);
        let mut screen = Screen::new(rows, cols, ContentArea::new(0, cols)).unwrap();
        let stats = render_static_screen(&document, &mut screen).unwrap();
        (document, screen, stats)
    }

    fn row_text(screen: &Screen, row: u16) -> String {
        let content = screen.content_area();
        let mut text = String::new();
        for col in content.x..content.right() {
            text.push_str(&screen.cell(row, col).unwrap().text().collect_text());
        }
        text.trim_end().to_owned()
    }

    fn assert_sources_on_content_rows(screen: &Screen, stats: RenderStats) {
        for row in 0..stats.rows {
            assert!(screen.line_source(row).is_some(), "row {row}");
        }
    }

    #[test]
    fn spacing_matrix_is_exhaustive() {
        use GroupClass::{List, Other, ParagraphLike};
        let classes = [ParagraphLike, List, Other];
        let expected = [[1_u16, 0, 1], [0, 0, 1], [1, 1, 1]];
        for (left, row) in classes.iter().copied().zip(expected) {
            for (right, blank) in classes.iter().copied().zip(row) {
                assert_eq!(
                    blank_lines_between(left, right),
                    blank,
                    "{left:?}->{right:?}"
                );
            }
        }
    }

    #[test]
    fn paragraph_spacing_has_one_blank_and_no_outer_edges() {
        let (_, screen, stats) = render("one\n\ntwo\n", 4, 20);
        assert_eq!(
            stats,
            RenderStats {
                rows: 3,
                content_rows: 2,
                blank_rows: 1,
                groups: 2,
                math_blocks_skipped: 0,
            }
        );
        assert_eq!(row_text(&screen, 0), "one");
        assert_eq!(row_text(&screen, 1), "");
        assert_eq!(row_text(&screen, 2), "two");
        assert!(screen.line_source(0).is_some());
        assert!(screen.line_source(1).is_none());
        assert!(screen.line_source(2).is_some());
    }

    #[test]
    fn paragraph_list_and_list_list_boundaries_are_tight() {
        let (_, screen, stats) = render("intro\n- one\n- two\n\n1. three\n", 6, 30);
        assert_eq!(stats.blank_rows, 0);
        assert_eq!(stats.content_rows, 4);
        assert_eq!(stats.groups, 3);
        assert_sources_on_content_rows(&screen, stats);
        assert_eq!(row_text(&screen, 0), "intro");
        assert_eq!(row_text(&screen, 1), "• one");
        assert_eq!(row_text(&screen, 2), "• two");
        assert_eq!(row_text(&screen, 3), "1. three");
    }

    #[test]
    fn code_and_rule_attach_to_lists_without_blank_rows() {
        let source = "```text\ncode\n```\n- item\n\n---\n- item two\n";
        let (_, screen, stats) = render(source, 8, 30);
        assert_eq!(stats.blank_rows, 0);
        assert_eq!(stats.content_rows, 5);
        assert_eq!(stats.groups, 4);
        assert_sources_on_content_rows(&screen, stats);
        assert_eq!(row_text(&screen, 0), "[text]");
        assert_eq!(row_text(&screen, 1), "  code");
        assert_eq!(row_text(&screen, 2), "• item");
        assert_eq!(row_text(&screen, 3), "──────────────────────────────");
        assert_eq!(row_text(&screen, 4), "• item two");
    }

    #[test]
    fn other_blocks_keep_one_blank_around_paragraph_like_blocks() {
        let source = "# H\n\npara\n\n> quote\n\npara\n\n| A |\n| --- |\n| 1 |\n\npara\n";
        let (_, screen, stats) = render(source, 20, 40);
        assert_eq!(stats.blank_rows, 5);
        assert_eq!(stats.content_rows, 10);
        assert_eq!(stats.groups, 6);
        for row in [1, 3, 5, 7, 13] {
            assert!(screen.line_source(row).is_none(), "row {row}");
        }
        assert_eq!(row_text(&screen, 0), "H");
        assert_eq!(row_text(&screen, 4), "│ quote");
        assert_eq!(row_text(&screen, 8), "┌───┐");
        assert_eq!(row_text(&screen, 14), "para");
    }

    #[test]
    fn quoted_subtree_renders_every_block_kind_without_inner_blank_rows() {
        let source = concat!(
            "> first paragraph\n",
            ">\n",
            "> second paragraph\n",
            ">\n",
            "> # In quote\n",
            ">\n",
            "> - item one\n",
            "> - item two\n",
            ">\n",
            "> ```text\n",
            "> code line\n",
            "> ```\n",
            ">\n",
            "> | A | B |\n",
            "> | --- | --- |\n",
            "> | 1 | 2 |\n",
            ">\n",
            "> ---\n",
        );
        let (_, screen, stats) = render(source, 20, 40);
        assert_eq!(stats.groups, 1);
        assert_eq!(stats.blank_rows, 0);
        assert_eq!(stats.content_rows, 13);
        assert_sources_on_content_rows(&screen, stats);
        assert_eq!(row_text(&screen, 0), "│ first paragraph");
        assert_eq!(row_text(&screen, 1), "│ second paragraph");
        assert_eq!(row_text(&screen, 2), "In quote");
        assert_eq!(row_text(&screen, 3), "• item one");
        assert_eq!(row_text(&screen, 5), "[text]");
        assert_eq!(row_text(&screen, 7), "┌───┬───┐");
        assert_eq!(
            row_text(&screen, 12),
            "────────────────────────────────────────"
        );
    }

    #[test]
    fn top_level_list_subtree_has_no_inner_blank_rows() {
        let source = "- one\n  - nested\n- two\n1. three\n2. four\n";
        let (_, screen, stats) = render(source, 8, 40);
        assert_eq!(stats.blank_rows, 0);
        assert_eq!(stats.content_rows, 5);
        assert_eq!(stats.groups, 2);
        assert_sources_on_content_rows(&screen, stats);
        assert_eq!(row_text(&screen, 0), "• one");
        assert_eq!(row_text(&screen, 1), "  • nested");
        assert_eq!(row_text(&screen, 2), "• two");
        assert_eq!(row_text(&screen, 3), "1. three");
        assert_eq!(row_text(&screen, 4), "2. four");
    }

    #[test]
    fn list_item_continuation_paragraph_is_not_rendered_twice() {
        let (_, screen, stats) = render("- first line\n  second line\n- next item\n", 4, 40);
        assert_eq!(stats.groups, 1);
        assert_eq!(stats.content_rows, 2);
        assert_eq!(stats.blank_rows, 0);
        assert_eq!(row_text(&screen, 0), "• first line second line");
        assert_eq!(row_text(&screen, 1), "• next item");
        assert_sources_on_content_rows(&screen, stats);
    }

    #[test]
    fn hidden_definitions_do_not_create_groups_or_extra_spacing() {
        let source = "one\n\n[d]: https://example.com\n\n[^n]: footnote\n\ntwo\n";
        let (_, screen, stats) = render(source, 4, 40);
        assert_eq!(stats.groups, 2);
        assert_eq!(stats.content_rows, 2);
        assert_eq!(stats.blank_rows, 1);
        assert_eq!(row_text(&screen, 0), "one");
        assert_eq!(row_text(&screen, 1), "");
        assert_eq!(row_text(&screen, 2), "two");
    }

    #[test]
    fn image_block_contributes_content_and_group_spacing() {
        let (_, screen, stats) = render("one\n\n![x](y)\n\ntwo\n", 6, 20);
        assert_eq!(stats.groups, 3);
        assert_eq!(stats.content_rows, 3);
        assert_eq!(stats.blank_rows, 2);
        assert_eq!(stats.rows, 5);
        assert_eq!(
            (0..5).map(|row| row_text(&screen, row)).collect::<Vec<_>>(),
            ["one", "", "x", "", "two"]
        );
        assert!(screen.line_source(1).is_none());
        assert!(screen.line_source(3).is_none());
        assert_eq!(
            screen.cell(2, 0).unwrap().link_id(),
            Some(mdtui3::LinkId(0))
        );
    }

    #[test]
    fn spacing_fixture_matches_expected_rows_and_blank_sources() {
        let source = include_str!("../../tests/fixtures/spec/spacing.md");
        let (_, screen, stats) = render(source, 80, 80);
        assert_eq!(stats.groups, 19);
        assert_eq!(stats.content_rows, 29);
        assert_eq!(stats.blank_rows, 12);
        assert_eq!(stats.rows, 41);
        let expected_blank_rows = [1, 3, 16, 18, 20, 22, 24, 30, 32, 35, 37, 39];
        for row in 0..stats.rows {
            let expected_blank = expected_blank_rows.contains(&row);
            assert_eq!(
                screen.line_source(row).is_none(),
                expected_blank,
                "row {row}"
            );
        }
        assert_eq!(row_text(&screen, 0), "Paragraph A.");
        assert_eq!(row_text(&screen, 4), "Paragraph before a list.");
        assert_eq!(row_text(&screen, 5), "• bullet attached to the paragraph");
        assert_eq!(row_text(&screen, 17), "Heading");
        assert_eq!(row_text(&screen, 21), "│ Quoted paragraph.");
        assert_eq!(row_text(&screen, 25), "┌─────┬───────┐");
        assert_eq!(row_text(&screen, 40), "Paragraph after rule.");
    }

    #[test]
    fn insufficient_rows_return_an_explicit_error() {
        let document = parse("one\n\ntwo\n");
        let mut screen = Screen::new(1, 20, ContentArea::new(0, 20)).unwrap();
        assert_eq!(
            render_static_screen(&document, &mut screen),
            Err(StaticRenderError::RowOverflow)
        );
    }

    #[test]
    fn math_blocks_use_the_real_prepared_object_path() {
        let (_, screen, stats) = render("$$\nx\n$$\n", 2, 20);
        assert_eq!(stats.math_blocks_skipped, 0);
        assert_eq!(stats.content_rows, 1);
        assert_eq!(screen.cell(0, 0).unwrap().text().collect_text(), "x");
        assert!(screen.line_source(0).is_some());
    }
}
