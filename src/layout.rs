use std::{fmt, ops::Range};

use crate::annotations::Annotations;
use crate::{Document, DocumentId, Span, Style, TileId, Update};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutError {
    ZeroEstimate,
    InvalidRange,
    StaleMeasurement,
    OutOfOrderUpdate,
    WrongDocument,
    WrongView,
    WrongHistory,
    StaleContinuation,
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

#[derive(Clone, Debug)]
pub struct ViewportCursor {
    view: DocumentId,
    epoch: u64,
    revision: Option<u64>,
    start: u128,
    rows: u128,
    origin: Option<Anchor>,
    after: Option<TileId>,
}

#[derive(Clone, Debug)]
pub struct ViewportPage {
    pub completion: crate::Completion,
    pub next: Option<ViewportCursor>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeightState {
    Measured,
    Local,
    Estimated,
    Expired,
}

/// A measurement depends on the prefix that supplied its incoming layout state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Measurement {
    tile: TileId,
    epoch: u64,
    view: DocumentId,
    dependencies: Span,
    generation: u64,
    checkpoint: Option<crate::CheckpointToken>,
}

struct Node {
    children: [Option<usize>; 2],
    sum: u128,
    uniform: Option<u64>,
    zeros: usize,
    fill: Option<u64>,
    status: Option<HeightState>,
    // Coverage belongs to canonical tree intervals. Removing a mask must use
    // exactly the range that installed it; parent masks never overwrite children.
    covers: usize,
    open_count: usize,
    open_sum: u128,
    open_zeros: usize,
}

impl Node {
    fn uniform(height: u64, width: usize) -> Self {
        Self {
            children: [None; 2],
            sum: u128::from(height) * width as u128,
            uniform: Some(height),
            zeros: if height == 0 { width } else { 0 },
            fill: None,
            status: Some(HeightState::Estimated),
            covers: 0,
            open_count: width,
            open_sum: u128::from(height) * width as u128,
            open_zeros: if height == 0 { width } else { 0 },
        }
    }

    fn visible_sum(&self, height: Option<u64>, fill: Option<u64>) -> u128 {
        if self.covers > 0 {
            0
        } else if let Some(height) = height {
            self.open_count as u128 * u128::from(height)
        } else {
            self.open_sum + self.open_zeros as u128 * u128::from(fill.unwrap_or(0))
        }
    }
}

/// A lazily assigned sum tree, independent of any renderer or terminal width.
/// Appending estimates, correcting heights, invalidating ranges, prefix sums and
/// row selection take O(log tiles); resetting after a width change takes O(1).
/// Existing nodes are reused after reset, so repeated resize cannot leak trees.
pub struct HeightIndex {
    identity: DocumentId,
    document: Option<DocumentId>,
    nodes: Vec<Node>,
    root: usize,
    capacity: usize,
    len: usize,
    estimate: u64,
    total: u128,
    epoch: u64,
    layout_epoch: u64,
    generations: Annotations,
    revision: Option<u64>,
    visits: u64,
    allocations: u64,
}

impl Default for HeightIndex {
    fn default() -> Self {
        Self::new(1).unwrap()
    }
}

impl HeightIndex {
    pub(crate) fn document_id(&self) -> Option<DocumentId> {
        self.document
    }
    pub(crate) fn applied_revision(&self) -> Option<u64> {
        self.revision
    }
    pub(crate) fn version(&self) -> (DocumentId, u64) {
        (self.identity, self.epoch)
    }
    pub(crate) fn check_document(&self, document: &Document) -> Result<(), LayoutError> {
        if self.document.is_some_and(|id| id != document.id()) {
            return Err(LayoutError::WrongDocument);
        }
        if self.revision.is_some_and(|r| r != document.revision()) {
            return Err(LayoutError::OutOfOrderUpdate);
        }
        Ok(())
    }
    pub fn new(estimated_rows: u64) -> Result<Self, LayoutError> {
        if estimated_rows == 0 {
            return Err(LayoutError::ZeroEstimate);
        }
        Ok(Self {
            identity: DocumentId::default(),
            document: None,
            nodes: vec![Node::uniform(estimated_rows, 1)],
            root: 0,
            capacity: 1,
            len: 0,
            estimate: estimated_rows,
            total: 0,
            epoch: 0,
            layout_epoch: 0,
            generations: Annotations::default(),
            revision: None,
            visits: 0,
            allocations: 1,
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
    pub fn dependency_arena_stats(&self) -> crate::ArenaStats {
        self.generations.arena_stats()
    }

    pub fn dependency_mutation_visits(&self) -> u64 {
        self.generations.visits
    }

    pub fn arena_stats(&self) -> crate::ArenaStats {
        crate::ArenaStats {
            nodes: self.nodes.len(),
            capacity_bytes: self.nodes.capacity() * std::mem::size_of::<Node>(),
            allocation_calls: self.allocations,
        }
    }

    fn push_node(&mut self, node: Node) {
        if self.nodes.len() == self.nodes.capacity() {
            self.allocations += 1;
        }
        self.nodes.push(node);
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
            self.push_node(Node::uniform(self.estimate, self.capacity));
            let root = self.nodes.len();
            let mut node = Node::uniform(self.estimate, self.capacity * 2);
            node.children = [Some(self.root), Some(right)];
            node.uniform = None;
            node.status = None;
            self.push_node(node);
            self.recount(root);
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
        // Capacity beyond len is always uncovered and carries the new estimate.
        self.total = self.nodes[self.root].visible_sum(None, None)
            - (self.capacity - self.len) as u128 * u128::from(estimated_rows);
        self.epoch += 1;
        self.layout_epoch += 1;
        Ok(())
    }

    pub fn invalidate(&mut self, range: Range<usize>) -> Result<(), LayoutError> {
        if range.start > range.end || range.end > self.len {
            return Err(LayoutError::InvalidRange);
        }
        if range.is_empty() {
            return Ok(());
        }
        self.changed(range.clone());
        self.set_range(range, self.estimate, HeightState::Estimated);
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
            self.changed(range.clone());
            self.set_range(range, 0, HeightState::Measured);
        }
        Ok(())
    }

    pub fn ticket(&self, tile: TileId) -> Option<Measurement> {
        (tile.0 < self.len).then_some(Measurement {
            tile,
            epoch: self.layout_epoch,
            view: self.identity,
            dependencies: Span::new(0, tile.0 + 1),
            generation: self.generations.newest(Span::new(0, tile.0 + 1)),
            checkpoint: None,
        })
    }

    /// Zero is a valid measured height (hidden syntax / continuation-only tile).
    pub fn measure(&mut self, ticket: Measurement, rows: u64) -> Result<(), LayoutError> {
        if ticket.checkpoint.is_some() {
            return Err(LayoutError::StaleMeasurement);
        }
        self.measure_validated(ticket, rows, HeightState::Measured)
    }

    pub fn ticket_from<S>(
        &self,
        tile: TileId,
        checkpoints: &crate::CheckpointIndex<S>,
        input: TileId,
    ) -> Option<Measurement> {
        if input > tile
            || checkpoints.document_id() != self.document
            || checkpoints.applied_revision() != self.revision
        {
            return None;
        }
        let checkpoint = checkpoints.token(input)?;
        let mut ticket = self.ticket(tile)?;
        ticket.dependencies.start = input.0;
        ticket.generation = self.generations.newest(ticket.dependencies);
        ticket.checkpoint = Some(checkpoint);
        Some(ticket)
    }

    pub fn measure_from<S>(
        &mut self,
        ticket: Measurement,
        rows: u64,
        checkpoints: &crate::CheckpointIndex<S>,
    ) -> Result<(), LayoutError> {
        if ticket
            .checkpoint
            .is_some_and(|token| !checkpoints.validates(token))
        {
            return Err(LayoutError::StaleMeasurement);
        }
        self.measure_validated(ticket, rows, HeightState::Measured)
    }

    fn measure_validated(
        &mut self,
        ticket: Measurement,
        rows: u64,
        state: HeightState,
    ) -> Result<(), LayoutError> {
        if ticket.view != self.identity {
            return Err(LayoutError::WrongView);
        }
        if ticket.epoch != self.layout_epoch
            || self.generations.newest(ticket.dependencies) != ticket.generation
        {
            return Err(LayoutError::StaleMeasurement);
        }
        if ticket.tile.0 >= self.len {
            return Err(LayoutError::InvalidRange);
        }
        self.set_range(ticket.tile.0..ticket.tile.0 + 1, rows, state);
        Ok(())
    }

    pub fn height(&self, tile: TileId) -> Option<u64> {
        if tile.0 >= self.len {
            return None;
        }
        Some(self.visible_sum(self.root, 0, self.capacity, tile.0..tile.0 + 1, None, None) as u64)
    }

    /// Stored height before folding visibility, including expired estimates.
    pub fn base_height(&self, tile: TileId) -> Option<u64> {
        (tile.0 < self.len)
            .then(|| self.sum(self.root, 0, self.capacity, tile.0..tile.0 + 1, None) as u64)
    }

    /// Height of tiles before `end` (including `end == len`).
    pub fn prefix_rows(&self, end: usize) -> Option<u128> {
        (end <= self.len).then(|| self.visible_sum(self.root, 0, self.capacity, 0..end, None, None))
    }

    pub fn locate(&self, row: u128) -> Option<Anchor> {
        if row >= self.total {
            return None;
        }
        let (mut id, mut lo, mut hi, mut rest) = (self.root, 0, self.capacity, row);
        let mut fill = None;
        let mut assigned = None;
        loop {
            let node = &self.nodes[id];
            assigned = assigned.or(node.uniform);
            if let Some(height) = assigned
                && node.open_count == hi - lo
            {
                let height = if height == 0 {
                    fill.unwrap_or(0)
                } else {
                    height
                };
                // A zero subtree cannot contain a row selected from a positive sum.
                let tile = lo + (rest / u128::from(height)) as usize;
                return Some(Anchor {
                    tile: TileId(tile),
                    offset: (rest % u128::from(height)) as u64,
                });
            }
            if assigned == Some(0) {
                assigned = fill;
            }
            let mid = lo + (hi - lo) / 2;
            let left = node.children[0].unwrap();
            let right = node.children[1].unwrap();
            fill = node.fill.or(fill);
            let left_sum = self.nodes[left].visible_sum(assigned, fill);
            if rest < left_sum {
                id = left;
                hi = mid;
            } else {
                rest -= left_sum;
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
        let mut visible = Vec::new();
        self.viewport_into(rows, overscan_rows, max_tiles, &mut visible);
        visible
    }

    pub fn viewport_into(
        &self,
        rows: Range<u128>,
        overscan_rows: u64,
        max_tiles: usize,
        visible: &mut Vec<VisibleTile>,
    ) {
        visible.clear();
        if rows.start >= rows.end || max_tiles == 0 {
            return;
        }
        let mut row = rows.start.saturating_sub(u128::from(overscan_rows));
        let end = rows
            .end
            .saturating_add(u128::from(overscan_rows))
            .min(self.total);
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
    }

    /// Shared-budget variant. The output Vec is reused and a continuation keeps
    /// source-tile anchors rather than a stale absolute row after measurements.
    pub fn viewport_budgeted(
        &self,
        rows: Range<u128>,
        overscan: u64,
        budget: &mut crate::Budget,
        out: &mut Vec<VisibleTile>,
    ) -> ViewportPage {
        let start = rows.start.saturating_sub(u128::from(overscan));
        let end = if rows.start >= rows.end {
            start
        } else {
            rows.end.saturating_add(u128::from(overscan))
        };
        self.resume_viewport(
            ViewportCursor {
                view: self.identity,
                epoch: self.epoch,
                revision: self.revision,
                start,
                rows: end - start,
                origin: None,
                after: None,
            },
            budget,
            out,
        )
        .unwrap()
    }

    pub fn resume_viewport(
        &self,
        mut cursor: ViewportCursor,
        budget: &mut crate::Budget,
        out: &mut Vec<VisibleTile>,
    ) -> Result<ViewportPage, LayoutError> {
        use crate::{Completion, WorkLimit};
        if cursor.view != self.identity {
            return Err(LayoutError::WrongView);
        }
        if cursor.epoch != self.epoch || cursor.revision != self.revision {
            return Err(LayoutError::StaleContinuation);
        }
        out.clear();
        loop {
            if cursor.rows == 0 {
                return Ok(ViewportPage {
                    completion: Completion::Covered,
                    next: None,
                });
            }
            if !budget.charge(WorkLimit {
                queries: 1,
                tiles: 1,
                ..Default::default()
            }) {
                return Ok(ViewportPage {
                    completion: Completion::BudgetExhausted,
                    next: Some(cursor),
                });
            }
            let start = cursor
                .origin
                .and_then(|a| self.row_of(a))
                .unwrap_or(cursor.start);
            let row = cursor
                .after
                .and_then(|tile| self.prefix_rows(tile.0 + 1))
                .unwrap_or(start);
            if row >= start.saturating_add(cursor.rows) {
                return Ok(ViewportPage {
                    completion: Completion::Covered,
                    next: None,
                });
            }
            let Some(anchor) = self.locate(row) else {
                return Ok(ViewportPage {
                    completion: Completion::EndOfDocument,
                    next: None,
                });
            };
            cursor.origin.get_or_insert(anchor);
            let top = row - u128::from(anchor.offset);
            let height = self.height(anchor.tile).unwrap();
            out.push(VisibleTile {
                tile: anchor.tile,
                top,
                height,
            });
            cursor.after = Some(anchor.tile);
            let next_row = top + u128::from(height);
            if next_row >= start.saturating_add(cursor.rows) {
                return Ok(ViewportPage {
                    completion: Completion::Covered,
                    next: None,
                });
            }
            if next_row >= self.total {
                return Ok(ViewportPage {
                    completion: Completion::EndOfDocument,
                    next: None,
                });
            }
        }
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
        self.document = Some(document.id());
        Ok(())
    }

    /// Consume a consecutive update or batch against its final document revision.
    /// Keep old heights as estimates; propagate incoming layout invalidation from
    /// the earliest size-affecting change without visiting historical uses.
    pub fn apply(&mut self, document: &Document, update: &Update) -> Result<(), LayoutError> {
        self.apply_with_policy(document, update, crate::LayoutPolicy::default())
    }

    pub fn apply_with_policy(
        &mut self,
        document: &Document,
        update: &Update,
        policy: crate::LayoutPolicy,
    ) -> Result<(), LayoutError> {
        if !self.validate_update(document, update)? {
            self.document = Some(document.id());
            self.revision = Some(update.revision);
            return Ok(());
        }
        self.grow_to(document.tile_count())?;
        if let Some(start) = update.layout_start(policy)
            && let Some(tile) = document.tile_at(start)
        {
            self.expire(tile.0..self.len)?;
        }
        self.revision = Some(update.revision);
        self.document = Some(document.id());
        Ok(())
    }

    pub(crate) fn validate_update(
        &self,
        document: &Document,
        update: &Update,
    ) -> Result<bool, LayoutError> {
        let changed = update.validate(document, self.document, self.revision)?;
        if self.len > document.tile_count() {
            return Err(LayoutError::OutOfOrderUpdate);
        }
        Ok(changed)
    }

    fn changed(&mut self, range: Range<usize>) {
        self.epoch += 1;
        self.generations.apply(
            Span::new(range.start, range.end),
            Style::default(),
            None,
            self.epoch,
        );
    }

    /// Preserve old nonzero heights while reviving zero estimates in O(log T).
    pub fn expire(&mut self, range: Range<usize>) -> Result<(), LayoutError> {
        if range.start > range.end || range.end > self.len {
            return Err(LayoutError::InvalidRange);
        }
        if range.is_empty() {
            return Ok(());
        }
        let old = self.visible_sum(self.root, 0, self.capacity, range.clone(), None, None);
        self.changed(range.clone());
        self.expire_range(self.root, 0, self.capacity, range.clone());
        self.total =
            self.total - old + self.visible_sum(self.root, 0, self.capacity, range, None, None);
        Ok(())
    }

    pub fn height_state(&self, tile: TileId) -> Option<HeightState> {
        if tile.0 >= self.len {
            return None;
        }
        let (mut id, mut lo, mut hi) = (self.root, 0, self.capacity);
        loop {
            let node = &self.nodes[id];
            if let Some(status) = node.status {
                return Some(status);
            }
            let mid = lo + (hi - lo) / 2;
            if tile.0 < mid {
                id = node.children[0]?;
                hi = mid;
            } else {
                id = node.children[1]?;
                lo = mid;
            }
        }
    }

    pub fn measure_local(&mut self, ticket: Measurement, rows: u64) -> Result<(), LayoutError> {
        if ticket.checkpoint.is_some() {
            return Err(LayoutError::StaleMeasurement);
        }
        self.measure_validated(ticket, rows, HeightState::Local)
    }

    fn assign(&mut self, id: usize, width: usize, height: u64) {
        self.visits += 1;
        let node = &mut self.nodes[id];
        node.sum = width as u128 * u128::from(height);
        node.uniform = Some(height);
        node.zeros = if height == 0 { width } else { 0 };
        node.fill = None;
        node.status = Some(HeightState::Estimated);
        node.open_sum = node.open_count as u128 * u128::from(height);
        node.open_zeros = if height == 0 { node.open_count } else { 0 };
    }

    fn revive(&mut self, id: usize, height: u64) {
        let node = &mut self.nodes[id];
        if node.zeros > 0 {
            node.sum += node.zeros as u128 * u128::from(height);
            node.open_sum += node.open_zeros as u128 * u128::from(height);
            node.open_zeros = 0;
            node.zeros = 0;
            if node.uniform == Some(0) {
                node.uniform = Some(height);
            } else {
                node.fill = Some(height);
            }
        }
    }

    fn push(&mut self, id: usize, width: usize) {
        if let Some(height) = self.nodes[id].uniform.take() {
            for side in 0..2 {
                if let Some(child) = self.nodes[id].children[side] {
                    self.assign(child, width / 2, height);
                } else {
                    let child = self.nodes.len();
                    self.push_node(Node::uniform(height, width / 2));
                    self.nodes[id].children[side] = Some(child);
                }
            }
        }
        let fill = self.nodes[id].fill.take();
        let status = self.nodes[id].status.take();
        for child in self.nodes[id].children.into_iter().flatten() {
            if let Some(height) = fill {
                self.revive(child, height);
            }
            if let Some(status) = status {
                self.nodes[child].status = Some(status);
            }
        }
    }

    fn recount(&mut self, id: usize) {
        let children = self.nodes[id].children;
        self.nodes[id].sum = children.iter().map(|id| self.nodes[id.unwrap()].sum).sum();
        self.nodes[id].zeros = children
            .iter()
            .map(|id| self.nodes[id.unwrap()].zeros)
            .sum();
        let (mut count, mut sum, mut zeros) = (0, 0, 0);
        for child in children.into_iter().flatten() {
            let node = &self.nodes[child];
            if node.covers == 0 {
                count += node.open_count;
                sum += node.open_sum;
                zeros += node.open_zeros;
            }
        }
        self.nodes[id].open_count = count;
        self.nodes[id].open_sum = sum;
        self.nodes[id].open_zeros = zeros;
    }

    fn set_range(&mut self, range: Range<usize>, height: u64, status: HeightState) {
        let old = self.visible_sum(self.root, 0, self.capacity, range.clone(), None, None);
        self.update(self.root, 0, self.capacity, range.clone(), height, status);
        self.total =
            self.total - old + self.visible_sum(self.root, 0, self.capacity, range, None, None);
    }

    /// Internal paired mask operation. FoldView owns each installed range.
    pub(crate) fn cover(&mut self, range: Range<usize>, covered: bool) {
        assert!(range.start <= range.end && range.end <= self.len);
        if range.is_empty() {
            return;
        }
        let old = self.visible_sum(self.root, 0, self.capacity, range.clone(), None, None);
        self.cover_range(self.root, 0, self.capacity, range.clone(), covered);
        self.total =
            self.total - old + self.visible_sum(self.root, 0, self.capacity, range, None, None);
    }

    fn cover_range(&mut self, id: usize, lo: usize, hi: usize, range: Range<usize>, covered: bool) {
        self.visits += 1;
        if range.start <= lo && hi <= range.end {
            if covered {
                self.nodes[id].covers += 1;
            } else {
                self.nodes[id].covers = self.nodes[id].covers.checked_sub(1).expect("paired mask");
            }
            return;
        }
        self.push(id, hi - lo);
        let mid = lo + (hi - lo) / 2;
        for (side, start, end) in [(0, lo, mid), (1, mid, hi)] {
            if range.start < end && start < range.end {
                self.cover_range(
                    self.nodes[id].children[side].unwrap(),
                    start,
                    end,
                    range.clone(),
                    covered,
                );
            }
        }
        self.recount(id);
    }

    fn visible_sum(
        &self,
        id: usize,
        lo: usize,
        hi: usize,
        range: Range<usize>,
        assigned: Option<u64>,
        fill: Option<u64>,
    ) -> u128 {
        let start = lo.max(range.start);
        let end = hi.min(range.end);
        let node = &self.nodes[id];
        if start >= end || node.covers > 0 {
            return 0;
        }
        if start == lo && end == hi {
            return node.visible_sum(assigned, fill);
        }
        let assigned = assigned.or(node.uniform).map(|height| {
            if height == 0 {
                fill.unwrap_or(0)
            } else {
                height
            }
        });
        if let Some(height) = assigned
            && node.open_count == hi - lo
        {
            return (end - start) as u128 * u128::from(height);
        }
        let fill = node.fill.or(fill);
        let mid = lo + (hi - lo) / 2;
        self.visible_sum(
            node.children[0].unwrap(),
            lo,
            mid,
            range.clone(),
            assigned,
            fill,
        ) + self.visible_sum(node.children[1].unwrap(), mid, hi, range, assigned, fill)
    }

    fn update(
        &mut self,
        id: usize,
        lo: usize,
        hi: usize,
        range: Range<usize>,
        height: u64,
        status: HeightState,
    ) {
        self.visits += 1;
        if range.start <= lo && hi <= range.end {
            self.assign(id, hi - lo, height);
            self.nodes[id].status = Some(status);
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
                    status,
                );
            }
        }
        self.recount(id);
    }

    fn expire_range(&mut self, id: usize, lo: usize, hi: usize, range: Range<usize>) {
        self.visits += 1;
        if range.start <= lo && hi <= range.end {
            self.revive(id, self.estimate);
            self.nodes[id].status = Some(HeightState::Expired);
            return;
        }
        self.push(id, hi - lo);
        let mid = lo + (hi - lo) / 2;
        for (side, start, end) in [(0, lo, mid), (1, mid, hi)] {
            if range.start < end && start < range.end {
                self.expire_range(
                    self.nodes[id].children[side].unwrap(),
                    start,
                    end,
                    range.clone(),
                );
            }
        }
        self.recount(id);
    }

    fn sum(&self, id: usize, lo: usize, hi: usize, range: Range<usize>, fill: Option<u64>) -> u128 {
        let start = lo.max(range.start);
        let end = hi.min(range.end);
        if start >= end {
            return 0;
        }
        let node = &self.nodes[id];
        if start == lo && end == hi {
            return node.sum + u128::from(fill.unwrap_or(0)) * node.zeros as u128;
        }
        if let Some(height) = node.uniform {
            let height = if height == 0 {
                fill.unwrap_or(0)
            } else {
                height
            };
            return (end - start) as u128 * u128::from(height);
        }
        let mid = lo + (hi - lo) / 2;
        let fill = node.fill.or(fill);
        self.sum(node.children[0].unwrap(), lo, mid, range.clone(), fill)
            + self.sum(node.children[1].unwrap(), mid, hi, range, fill)
    }
}

#[cfg(test)]
mod folding_tests {
    use super::*;

    #[test]
    fn masks_assignments_expiration_resize_and_growth_match_array() {
        let mut heights = HeightIndex::new(3).unwrap();
        let mut values = Vec::<u64>::new();
        let mut masks = Vec::<Range<usize>>::new();
        let mut seed = 0x48a7_1cf9_u64;
        let mut estimate = 3;
        for step in 0..4000 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let n = values.len();
            let a = (seed >> 32) as usize % (n + 1);
            let b = (seed >> 16) as usize % (n + 1);
            let range = a.min(b)..a.max(b);
            match seed % 8 {
                0 => {
                    let end = n + 1 + step % 5;
                    heights.grow_to(end).unwrap();
                    values.resize(end, estimate);
                }
                1 => {
                    heights.cover(range.clone(), true);
                    masks.push(range);
                }
                2 if !masks.is_empty() => {
                    let range = masks.swap_remove(a % masks.len());
                    heights.cover(range, false);
                }
                3 => {
                    heights.expire(range.clone()).unwrap();
                    for h in &mut values[range] {
                        if *h == 0 {
                            *h = estimate;
                        }
                    }
                }
                4 => {
                    estimate = 1 + a as u64 % 7;
                    heights.reset(estimate).unwrap();
                    values.fill(estimate);
                }
                5 => {
                    heights.suppress(range.clone()).unwrap();
                    values[range].fill(0);
                }
                6 if a < n => {
                    let h = b as u64 % 9;
                    heights
                        .measure(heights.ticket(TileId(a)).unwrap(), h)
                        .unwrap();
                    values[a] = h;
                }
                _ => {
                    heights.invalidate(range.clone()).unwrap();
                    values[range].fill(estimate);
                }
            }
            let mut prefix = 0;
            for (i, &base) in values.iter().enumerate() {
                let visible = if masks.iter().any(|r| r.contains(&i)) {
                    0
                } else {
                    base
                };
                assert_eq!(
                    heights.base_height(TileId(i)),
                    Some(base),
                    "base step {step} tile {i}"
                );
                assert_eq!(
                    heights.height(TileId(i)),
                    Some(visible),
                    "step {step} tile {i}"
                );
                assert_eq!(heights.prefix_rows(i), Some(prefix));
                if visible > 0 {
                    assert_eq!(
                        heights.locate(prefix),
                        Some(Anchor {
                            tile: TileId(i),
                            offset: 0
                        })
                    );
                    assert_eq!(
                        heights.locate(prefix + u128::from(visible) - 1),
                        Some(Anchor {
                            tile: TileId(i),
                            offset: visible - 1
                        })
                    );
                }
                prefix += u128::from(visible);
            }
            assert_eq!(heights.total_rows(), prefix, "step {step}");
            assert_eq!(heights.locate(prefix), None);
        }
    }

    #[test]
    fn billion_tile_masks_are_logarithmic_and_reuse_nodes() {
        let mut heights = HeightIndex::default();
        heights.grow_to(1_000_000_000).unwrap();
        let start = heights.mutation_visits();
        heights.cover(100..900_000_000, true);
        heights.cover(200..800_000_000, true);
        let nodes = heights.node_count();
        assert!(heights.mutation_visits() - start < 1000);
        heights.cover(100..900_000_000, false);
        assert_eq!(heights.height(TileId(500)), Some(0));
        assert_eq!(heights.base_height(TileId(500)), Some(1));
        for _ in 0..100 {
            heights.cover(100..900_000_000, true);
            heights.reset(2).unwrap();
            heights.expire(0..1_000_000_000).unwrap();
            heights.cover(100..900_000_000, false);
        }
        assert!(heights.node_count() <= nodes + 120);
        heights.cover(200..800_000_000, false);
        assert_eq!(heights.total_rows(), 2_000_000_000);
    }
}
