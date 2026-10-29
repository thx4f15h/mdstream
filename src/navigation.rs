use crate::{Anchor, Document, HeadingId, HeightIndex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavAction {
    LineUp,
    LineDown,
    PageUp,
    PageDown,
    Home,
    End,
    Heading(HeadingId),
    NextHeading { max_level: u8 },
    PreviousHeading { max_level: u8 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScrollTarget {
    /// Top row using the current mix of measured and estimated heights.
    pub top: u128,
    pub anchor: Option<Anchor>,
    /// Exact source destination for heading jumps, even within one source tile.
    /// The renderer refines its row using local layout; this is not a pixel offset.
    pub source: Option<usize>,
}

#[derive(Default)]
pub struct Navigator {
    anchor: Option<Anchor>,
    source: Option<usize>,
}

impl Navigator {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn top(&self, heights: &HeightIndex) -> u128 {
        self.anchor.and_then(|a| heights.row_of(a)).unwrap_or(0)
    }

    /// Refresh the viewport anchor after the renderer refines a source jump.
    /// The heading source target is kept so consecutive jumps within one tile
    /// visit distinct headings, rather than repeatedly selecting the same one.
    pub fn reconcile(
        &mut self,
        heights: &HeightIndex,
        row: u128,
        viewport_rows: u64,
    ) -> ScrollTarget {
        self.set(heights, row, viewport_rows, self.source)
    }

    /// Supply the renderer's row-to-source mapping after ordinary scrolling.
    /// Without it, heading navigation uses the selected tile's start as an
    /// estimate; source positions within a wrapped tile require actual layout.
    pub fn reconcile_source(
        &mut self,
        heights: &HeightIndex,
        row: u128,
        viewport_rows: u64,
        source: usize,
    ) -> ScrollTarget {
        self.set(heights, row, viewport_rows, Some(source))
    }

    pub fn navigate(
        &mut self,
        document: &Document,
        heights: &HeightIndex,
        action: NavAction,
        viewport_rows: u64,
    ) -> Option<ScrollTarget> {
        let page = u128::from(viewport_rows.max(1));
        let current = self
            .top(heights)
            .min(heights.total_rows().saturating_sub(page));
        let offset = self
            .source
            .or_else(|| {
                self.anchor
                    .and_then(|a| document.tile(a.tile))
                    .map(|s| s.start)
            })
            .unwrap_or(0);
        let heading = match action {
            NavAction::Heading(id) => Some(id),
            NavAction::NextHeading { max_level } => {
                Some(document.headings().next(offset, max_level)?)
            }
            NavAction::PreviousHeading { max_level } => {
                Some(document.headings().previous(offset, max_level)?)
            }
            _ => None,
        };
        let (row, source) = if let Some(id) = heading {
            let heading = document.headings().get(id)?;
            let tile = document.tile_at(heading.source.start)?;
            (heights.prefix_rows(tile.0)?, Some(heading.source.start))
        } else {
            let row = match action {
                NavAction::LineUp => current.saturating_sub(1),
                NavAction::LineDown => current.saturating_add(1),
                NavAction::PageUp => current.saturating_sub(page),
                NavAction::PageDown => current.saturating_add(page),
                NavAction::Home => 0,
                NavAction::End => heights.total_rows(),
                _ => unreachable!(),
            };
            (row, None)
        };
        Some(self.set(heights, row, viewport_rows, source))
    }

    fn set(
        &mut self,
        heights: &HeightIndex,
        row: u128,
        viewport_rows: u64,
        source: Option<usize>,
    ) -> ScrollTarget {
        let top = row.min(
            heights
                .total_rows()
                .saturating_sub(u128::from(viewport_rows.max(1))),
        );
        self.anchor = heights.locate(top);
        self.source = source;
        ScrollTarget {
            top,
            anchor: self.anchor,
            source,
        }
    }
}
