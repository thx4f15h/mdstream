use std::{fmt, ops::Range};

use crate::{Document, TileId, Update};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutError {
    ZeroEstimate,
    InvalidRange,
    StaleMeasurement,
    OutOfOrderUpdate,
}

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for LayoutError {}

/// The stable tile and row offset to keep anchored while estimates change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Anchor {
    pub tile: TileId,
    pub offset: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VisibleTile {
    pub tile: TileId,
    pub top: u128,
    pub height: u64,
}

/// Capture before measuring; any invalidation makes outstanding tickets stale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Measurement {
    tile: TileId,
    epoch: u64,
}

struct Node {
    children: [Option<usize>; 2],
    sum: u128,
    uniform: Option<u64>,
}

impl Node {
    fn uniform(height: u64, width: usize) -> Self {
        Self {
            children: [None; 2],
            sum: u128::from(height) * width as u128,
            uniform: Some(height),
        }
    }
}

/// A lazily assigned sum tree, independent of any renderer or terminal width.
/// Appending estimates, correcting heights, invalidating ranges, prefix sums and
/// row selection take O(log tiles); resetting after a width change takes O(1).
/// Existing nodes are reused after reset, so repeated resize cannot leak trees.
pub struct HeightIndex {
    nodes: Vec<Node>,
    root: usize,
    capacity: usize,
    len: usize,
    estimate: u64,
    total: u128,
    epoch: u64,
    revision: Option<u64>,
    visits: u64,
}

impl Default for HeightIndex {
    fn default() -> Self {
        Self::new(1).unwrap()
    }
}

impl HeightIndex {
    pub fn new(estimated_rows: u64) -> Result<Self, LayoutError> {
        if estimated_rows == 0 {
            return Err(LayoutError::ZeroEstimate);
        }
        Ok(Self {
            nodes: vec![Node::uniform(estimated_rows, 1)],
            root: 0,
            capacity: 1,
            len: 0,
            estimate: estimated_rows,
            total: 0,
            epoch: 0,
            revision: None,
            visits: 0,
        })
    }

    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn total_rows(&self) -> u128 {
        self.total
    }
    pub fn mutation_visits(&self) -> u64 {
        self.visits
    }
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Grow without visiting the new leaves. Shrinking is intentionally unsupported.
    pub fn grow_to(&mut self, len: usize) -> Result<(), LayoutError> {
        if len < self.len || len > isize::MAX as usize {
            return Err(LayoutError::InvalidRange);
        }
        while self.capacity < len {
            let right = self.nodes.len();
            self.nodes.push(Node::uniform(self.estimate, self.capacity));
            let root = self.nodes.len();
            self.nodes.push(Node {
                children: [Some(self.root), Some(right)],
                sum: self.nodes[self.root].sum + self.nodes[right].sum,
                uniform: None,
            });
            self.root = root;
            self.capacity *= 2;
        }
        self.total += (len - self.len) as u128 * u128::from(self.estimate);
        self.len = len;
        Ok(())
    }

    pub fn reset(&mut self, estimated_rows: u64) -> Result<(), LayoutError> {
        if estimated_rows == 0 {
            return Err(LayoutError::ZeroEstimate);
        }
        self.estimate = estimated_rows;
        self.assign(self.root, self.capacity, estimated_rows);
        self.total = self.len as u128 * u128::from(estimated_rows);
        self.epoch += 1;
        Ok(())
    }

    pub fn invalidate(&mut self, range: Range<usize>) -> Result<(), LayoutError> {
        if range.start > range.end || range.end > self.len {
            return Err(LayoutError::InvalidRange);
        }
        if range.is_empty() {
            return Ok(());
        }
        self.set_range(range, self.estimate);
        self.epoch += 1;
        Ok(())
    }

    /// Set an entire known-empty tile range to zero in O(log tiles). For example,
    /// Document::next_visible can identify a large hidden fence-info region.
    /// Resize/reset restores estimates; callers can rediscover hidden ranges
    /// lazily with another logarithmic jump.
    pub fn suppress(&mut self, range: Range<usize>) -> Result<(), LayoutError> {
        if range.start > range.end || range.end > self.len {
            return Err(LayoutError::InvalidRange);
        }
        if !range.is_empty() {
            self.set_range(range, 0);
            self.epoch += 1;
        }
        Ok(())
    }

    pub fn ticket(&self, tile: TileId) -> Option<Measurement> {
        (tile.0 < self.len).then_some(Measurement {
            tile,
            epoch: self.epoch,
        })
    }

    /// Zero is a valid measured height (hidden syntax / continuation-only tile).
    pub fn measure(&mut self, ticket: Measurement, rows: u64) -> Result<(), LayoutError> {
        if ticket.epoch != self.epoch {
            return Err(LayoutError::StaleMeasurement);
        }
        if ticket.tile.0 >= self.len {
            return Err(LayoutError::InvalidRange);
        }
        self.set_range(ticket.tile.0..ticket.tile.0 + 1, rows);
        Ok(())
    }

    pub fn height(&self, tile: TileId) -> Option<u64> {
        if tile.0 >= self.len {
            return None;
        }
        Some(self.sum(self.root, 0, self.capacity, tile.0..tile.0 + 1) as u64)
    }

    /// Height of tiles before `end` (including `end == len`).
    pub fn prefix_rows(&self, end: usize) -> Option<u128> {
        (end <= self.len).then(|| self.sum(self.root, 0, self.capacity, 0..end))
    }

    pub fn locate(&self, row: u128) -> Option<Anchor> {
        if row >= self.total {
            return None;
        }
        let (mut id, mut lo, mut hi, mut rest) = (self.root, 0, self.capacity, row);
        loop {
            let node = &self.nodes[id];
            if let Some(height) = node.uniform {
                // A zero subtree cannot contain a row selected from a positive sum.
                let tile = lo + (rest / u128::from(height)) as usize;
                return Some(Anchor {
                    tile: TileId(tile),
                    offset: (rest % u128::from(height)) as u64,
                });
            }
            let mid = lo + (hi - lo) / 2;
            let left = node.children[0].unwrap();
            let right = node.children[1].unwrap();
            if rest < self.nodes[left].sum {
                id = left;
                hi = mid;
            } else {
                rest -= self.nodes[left].sum;
                id = right;
                lo = mid;
            }
        }
    }

    pub fn row_of(&self, anchor: Anchor) -> Option<u128> {
        let height = self.height(anchor.tile)?;
        Some(
            self.prefix_rows(anchor.tile.0)?
                + u128::from(anchor.offset.min(height.saturating_sub(1))),
        )
    }

    /// O((returned tiles + 1) log tiles), including arbitrarily long zero-height
    /// regions. `max_tiles` bounds allocation and work for any viewport request.
    pub fn viewport(
        &self,
        rows: Range<u128>,
        overscan_rows: u64,
        max_tiles: usize,
    ) -> Vec<VisibleTile> {
        if rows.start >= rows.end || max_tiles == 0 {
            return Vec::new();
        }
        let mut row = rows.start.saturating_sub(u128::from(overscan_rows));
        let end = rows
            .end
            .saturating_add(u128::from(overscan_rows))
            .min(self.total);
        let mut visible = Vec::new();
        while row < end && visible.len() < max_tiles {
            let Some(anchor) = self.locate(row) else {
                break;
            };
            let top = row - u128::from(anchor.offset);
            let height = self.height(anchor.tile).unwrap();
            visible.push(VisibleTile {
                tile: anchor.tile,
                top,
                height,
            });
            row = top + u128::from(height);
        }
        visible
    }

    /// Attach to a document or recover after skipped updates. Estimates for the
    /// entire document are installed without enumerating its tiles.
    pub fn rebuild(&mut self, document: &Document) -> Result<(), LayoutError> {
        if document.tile_count() < self.len {
            return Err(LayoutError::InvalidRange);
        }
        self.grow_to(document.tile_count())?;
        self.reset(self.estimate)?;
        self.revision = Some(document.revision());
        Ok(())
    }

    /// Consume each update immediately against the document that produced it.
    /// Late reference definitions reset estimates in O(1), without walking uses.
    pub fn apply(&mut self, document: &Document, update: &Update) -> Result<(), LayoutError> {
        let previous = self.revision.unwrap_or(0);
        if update.revision != document.revision()
            || update.revision < previous
            || update.revision > previous + 1
            || self.len > document.tile_count()
        {
            return Err(LayoutError::OutOfOrderUpdate);
        }
        self.grow_to(document.tile_count())?;
        if !update.references_changed.is_empty() {
            self.reset(self.estimate)?;
        } else {
            for &span in &update.dirty {
                self.invalidate(document.tiles_overlapping(span))?;
            }
        }
        self.revision = Some(update.revision);
        Ok(())
    }

    fn assign(&mut self, id: usize, width: usize, height: u64) {
        self.visits += 1;
        self.nodes[id].sum = width as u128 * u128::from(height);
        self.nodes[id].uniform = Some(height);
    }

    fn push(&mut self, id: usize, width: usize) {
        if let Some(height) = self.nodes[id].uniform.take() {
            for side in 0..2 {
                if let Some(child) = self.nodes[id].children[side] {
                    self.assign(child, width / 2, height);
                } else {
                    let child = self.nodes.len();
                    self.nodes.push(Node::uniform(height, width / 2));
                    self.nodes[id].children[side] = Some(child);
                }
            }
        }
    }

    fn set_range(&mut self, range: Range<usize>, height: u64) {
        let old = self.sum(self.root, 0, self.capacity, range.clone());
        self.total = self.total - old + (range.end - range.start) as u128 * u128::from(height);
        self.update(self.root, 0, self.capacity, range, height);
    }

    fn update(&mut self, id: usize, lo: usize, hi: usize, range: Range<usize>, height: u64) {
        self.visits += 1;
        if range.start <= lo && hi <= range.end {
            self.assign(id, hi - lo, height);
            return;
        }
        self.push(id, hi - lo);
        let mid = lo + (hi - lo) / 2;
        for (side, start, end) in [(0, lo, mid), (1, mid, hi)] {
            if range.start < end && start < range.end {
                self.update(
                    self.nodes[id].children[side].unwrap(),
                    start,
                    end,
                    range.clone(),
                    height,
                );
            }
        }
        self.nodes[id].sum = self.nodes[id]
            .children
            .iter()
            .map(|id| self.nodes[id.unwrap()].sum)
            .sum();
    }

    fn sum(&self, id: usize, lo: usize, hi: usize, range: Range<usize>) -> u128 {
        let start = lo.max(range.start);
        let end = hi.min(range.end);
        if start >= end {
            return 0;
        }
        let node = &self.nodes[id];
        if start == lo && end == hi {
            return node.sum;
        }
        if let Some(height) = node.uniform {
            return (end - start) as u128 * u128::from(height);
        }
        let mid = lo + (hi - lo) / 2;
        self.sum(node.children[0].unwrap(), lo, mid, range.clone())
            + self.sum(node.children[1].unwrap(), mid, hi, range)
    }
}
