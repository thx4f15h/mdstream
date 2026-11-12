use std::{num::NonZeroUsize, ops::Range};

use crate::{Document, DocumentId, LayoutError, TileId, Update};

struct Node<S> {
    children: [Option<NonZeroUsize>; 2],
    valid: usize,
    state: Option<S>,
    tile: Option<TileId>,
    version: u64,
}

impl<S> Node<S> {
    fn child(&self, side: usize) -> Option<usize> {
        self.children[side].map(|index| index.get() - 1)
    }
}

impl<S> Default for Node<S> {
    fn default() -> Self {
        Self {
            children: [None; 2],
            valid: 0,
            state: None,
            tile: None,
            version: 0,
        }
    }
}

/// Sparse renderer-owned resume states, keyed by the tile they precede.
///
/// Invalidating a range only clears subtree counts; looking up a predecessor
/// skips invalid subtrees in O(log maximum tile index), even after millions of
/// cached states become stale. Invalid states are retained until overwritten or
/// explicitly drained/compacted or this index is dropped, so invalidation never
/// runs an unbounded number of user destructors. Without explicit compaction,
/// memory depends on positions ever cached, not invalidation count.
pub struct CheckpointIndex<S> {
    identity: DocumentId,
    serial: u64,
    document: Option<DocumentId>,
    nodes: Vec<Node<S>>,
    root: usize,
    capacity: usize,
    revision: Option<u64>,
    visits: u64,
    allocations: u64,
}

impl<S> Default for CheckpointIndex<S> {
    fn default() -> Self {
        Self {
            identity: DocumentId::default(),
            serial: 0,
            document: None,
            nodes: vec![Node::default()],
            root: 0,
            capacity: 1,
            revision: None,
            visits: 0,
            allocations: 1,
        }
    }
}

impl<S> CheckpointIndex<S> {
    pub(crate) fn document_id(&self) -> Option<DocumentId> {
        self.document
    }
    pub(crate) fn applied_revision(&self) -> Option<u64> {
        self.revision
    }

    /// Replace an input state and expire downstream cached layout together.
    /// The replaced payload is returned, not destroyed by invalidation.
    pub fn insert_for_layout(
        &mut self,
        heights: &mut crate::HeightIndex,
        tile: TileId,
        state: S,
    ) -> Result<Option<S>, LayoutError> {
        if self.document != heights.document_id() {
            return Err(LayoutError::WrongDocument);
        }
        if self.revision != heights.applied_revision() {
            return Err(LayoutError::OutOfOrderUpdate);
        }
        if tile.0 >= heights.len() {
            return Err(LayoutError::InvalidRange);
        }
        heights.expire(tile.0..heights.len())?;
        if tile.0 < self.capacity {
            self.invalidate(tile.0..self.capacity)?;
        }
        self.insert(tile, state)
    }
    pub fn new() -> Self {
        Self::default()
    }
    pub fn len(&self) -> usize {
        self.nodes[self.root].valid
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn arena_stats(&self) -> crate::ArenaStats {
        crate::ArenaStats {
            nodes: self.nodes.len(),
            capacity_bytes: self.nodes.capacity() * std::mem::size_of::<Node<S>>(),
            allocation_calls: self.allocations,
        }
    }

    fn push_node(&mut self, node: Node<S>) {
        if self.nodes.len() == self.nodes.capacity() {
            self.allocations += 1;
        }
        self.nodes.push(node);
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }
    pub fn mutation_visits(&self) -> u64 {
        self.visits
    }

    /// Take ownership for resumable, read-only maintenance. Valid payloads and
    /// tokens survive; stale payloads are returned by `advance`. No tree scan or
    /// payload destruction happens here. Finish before applying further updates.
    pub fn into_compaction(self) -> CheckpointCompaction<S> {
        let destination = Self {
            identity: self.identity,
            serial: self.serial,
            document: self.document,
            revision: self.revision,
            ..Self::default()
        };
        let root = self.root;
        CheckpointCompaction {
            source: self,
            destination,
            stack: vec![root],
            scratch_allocations: 1,
            visited: 0,
            moved: 0,
            retired: 0,
        }
    }

    /// Tile indices must be below `isize::MAX`, matching HeightIndex's maximum
    /// length and keeping the exclusive power-of-two capacity representable.
    pub fn insert(&mut self, tile: TileId, state: S) -> Result<Option<S>, LayoutError> {
        if tile.0 >= isize::MAX as usize {
            return Err(LayoutError::InvalidRange);
        }
        self.serial += 1;
        Ok(self.insert_versioned(tile, state, self.serial))
    }

    fn insert_versioned(&mut self, tile: TileId, state: S, version: u64) -> Option<S> {
        while self.capacity <= tile.0 {
            let root = self.nodes.len();
            self.push_node(Node {
                children: [NonZeroUsize::new(self.root + 1), None],
                valid: self.nodes[self.root].valid,
                state: None,
                tile: None,
                version: 0,
            });
            self.root = root;
            self.capacity *= 2;
        }
        self.set(self.root, 0, self.capacity, tile.0, (state, version))
    }

    pub fn token(&self, tile: TileId) -> Option<CheckpointToken> {
        if tile.0 >= self.capacity {
            return None;
        }
        let (mut id, mut lo, mut hi) = (self.root, 0, self.capacity);
        loop {
            let node = &self.nodes[id];
            if node.valid == 0 {
                return None;
            }
            if hi - lo == 1 {
                return Some(CheckpointToken {
                    index: self.identity,
                    tile,
                    version: node.version,
                });
            }
            let mid = lo + (hi - lo) / 2;
            if tile.0 < mid {
                id = node.child(0)?;
                hi = mid;
            } else {
                id = node.child(1)?;
                lo = mid;
            }
        }
    }

    pub fn validates(&self, token: CheckpointToken) -> bool {
        self.token(token.tile) == Some(token)
    }

    pub fn get(&self, tile: TileId) -> Option<&S> {
        self.previous(tile)
            .filter(|(found, _)| *found == tile)
            .map(|(_, state)| state)
    }

    /// Latest valid checkpoint at or before `tile`; never scans stale entries.
    pub fn previous(&self, tile: TileId) -> Option<(TileId, &S)> {
        self.find(self.root, 0, self.capacity, tile.0)
    }

    /// Refuse a remote checkpoint instead of implicitly replaying a huge prefix.
    pub fn previous_within(&self, tile: TileId, max_tiles: usize) -> Option<(TileId, &S)> {
        self.previous(tile)
            .filter(|(from, _)| tile.0 - from.0 <= max_tiles)
    }

    /// Source-byte recovery limit, charged together with the caller's frame.
    /// None permits a local, explicitly approximate start at the requested tile.
    pub fn recovery<'a>(
        &'a self,
        document: &Document,
        tile: TileId,
        max_bytes: usize,
        budget: &mut crate::Budget,
    ) -> Result<Option<(TileId, &'a S)>, LayoutError> {
        if self.document.is_some_and(|id| id != document.id()) {
            return Err(LayoutError::WrongDocument);
        }
        if self
            .revision
            .is_some_and(|revision| revision != document.revision())
        {
            return Err(LayoutError::OutOfOrderUpdate);
        }
        if !budget.charge(crate::WorkLimit {
            queries: 1,
            ..Default::default()
        }) {
            return Ok(None);
        }
        let Some(target) = document.tile(tile) else {
            return Err(LayoutError::InvalidRange);
        };
        Ok(self.previous(tile).filter(|(from, _)| {
            document
                .tile(*from)
                .is_some_and(|span| target.start - span.start <= max_bytes)
        }))
    }

    pub fn invalidate(&mut self, range: Range<usize>) -> Result<(), LayoutError> {
        if range.start > range.end {
            return Err(LayoutError::InvalidRange);
        }
        self.remove(self.root, 0, self.capacity, range);
        Ok(())
    }

    pub fn clear(&mut self) {
        self.visits += 1;
        self.nodes[self.root].valid = 0;
    }

    /// Recover after missed updates, or attach an empty cache to a document.
    pub fn rebuild(&mut self, document: &Document) {
        self.clear();
        self.revision = Some(document.revision());
        self.document = Some(document.id());
    }

    /// Conservative propagation of layout dependencies: all checkpoints at or
    /// after the first changed tile become invalid. Existing prefixes survive.
    /// Use this together with HeightIndex::apply after every append/finish.
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
        if let Some(start) = update.layout_start(policy)
            && let Some(tile) = document.tile_at(start)
            && tile.0 < self.capacity
        {
            self.invalidate(tile.0..self.capacity)?;
        }
        self.revision = Some(update.revision);
        self.document = Some(document.id());
        Ok(())
    }

    pub(crate) fn apply_hosted(
        &mut self,
        document: &Document,
        base: u64,
        revision: u64,
        start: Option<usize>,
    ) -> Result<(), LayoutError> {
        if self.document != Some(document.id()) {
            return Err(LayoutError::WrongDocument);
        }
        if self.revision != Some(base) || document.revision() != revision {
            return Err(LayoutError::OutOfOrderUpdate);
        }
        if let Some(start) = start
            && let Some(tile) = document.tile_at(start)
            && tile.0 < self.capacity
        {
            self.invalidate(tile.0..self.capacity)?;
        }
        self.revision = Some(revision);
        Ok(())
    }

    /// Explicitly evict payloads before a tile. Scan at most the shared object
    /// budget, returning ownership so user destructors run at the caller's chosen
    /// time. `cursor` is an arena position, reset to zero for a new collection pass.
    pub fn drain_before(
        &mut self,
        before: TileId,
        cursor: &mut usize,
        budget: &mut crate::Budget,
        out: &mut Vec<S>,
    ) -> crate::Completion {
        while *cursor < self.nodes.len() {
            if !budget.charge(crate::WorkLimit {
                objects: 1,
                queries: 1,
                ..Default::default()
            }) {
                return crate::Completion::BudgetExhausted;
            }
            let id = *cursor;
            *cursor += 1;
            if let Some(tile) = self.nodes[id].tile
                && tile < before
                && self.nodes[id].state.is_some()
            {
                self.invalidate(tile.0..tile.0 + 1).expect("one valid tile");
                out.push(self.nodes[id].state.take().unwrap());
            }
        }
        crate::Completion::Covered
    }

    pub(crate) fn validate_update(
        &self,
        document: &Document,
        update: &Update,
    ) -> Result<bool, LayoutError> {
        update.validate(document, self.document, self.revision)
    }

    fn set(&mut self, id: usize, lo: usize, hi: usize, tile: usize, state: (S, u64)) -> Option<S> {
        self.visits += 1;
        if hi - lo == 1 {
            self.nodes[id].valid = 1;
            self.nodes[id].tile = Some(TileId(tile));
            self.nodes[id].version = state.1;
            return self.nodes[id].state.replace(state.0);
        }
        // A cleared parent hides potentially stale children. Clear their counts
        // before reviving a single path, or insertion would resurrect old states.
        if self.nodes[id].valid == 0 {
            for child in self.nodes[id].children.into_iter().flatten() {
                self.nodes[child.get() - 1].valid = 0;
            }
        }
        let mid = lo + (hi - lo) / 2;
        let side = usize::from(tile >= mid);
        let child = match self.nodes[id].child(side) {
            Some(child) => child,
            None => {
                let child = self.nodes.len();
                self.push_node(Node::default());
                self.nodes[id].children[side] = NonZeroUsize::new(child + 1);
                child
            }
        };
        let old = if side == 0 {
            self.set(child, lo, mid, tile, state)
        } else {
            self.set(child, mid, hi, tile, state)
        };
        self.recount(id);
        old
    }

    fn recount(&mut self, id: usize) {
        self.nodes[id].valid = self.nodes[id]
            .children
            .into_iter()
            .flatten()
            .map(|child| self.nodes[child.get() - 1].valid)
            .sum();
    }

    fn remove(&mut self, id: usize, lo: usize, hi: usize, range: Range<usize>) {
        self.visits += 1;
        if self.nodes[id].valid == 0 || range.start >= hi || range.end <= lo {
            return;
        }
        if range.start <= lo && hi <= range.end {
            self.nodes[id].valid = 0;
            return;
        }
        let mid = lo + (hi - lo) / 2;
        for (side, start, end) in [(0, lo, mid), (1, mid, hi)] {
            if let Some(child) = self.nodes[id].child(side) {
                self.remove(child, start, end, range.clone());
            }
        }
        self.recount(id);
    }

    fn find(&self, id: usize, lo: usize, hi: usize, at: usize) -> Option<(TileId, &S)> {
        let node = &self.nodes[id];
        if node.valid == 0 || lo > at {
            return None;
        }
        if hi - lo == 1 {
            return node.state.as_ref().map(|state| (TileId(lo), state));
        }
        let mid = lo + (hi - lo) / 2;
        if let Some(right) = node.child(1)
            && let Some(found) = self.find(right, mid, hi, at)
        {
            return Some(found);
        }
        node.child(0).and_then(|left| self.find(left, lo, mid, at))
    }
}

/// A cache owned by a paused compaction. Queries see the union of the old and
/// new trees; writes and document updates resume after `finish`. Dropping this
/// task drops the cache (including remaining payloads), just like dropping an
/// index. Keep the task to pause without destruction. No `S: Clone` is required.
///
/// Each advance step charges one object and one query: either a DFS node visit
/// (a live leaf also performs two O(log T) tree operations), or one retired node.
/// For N historical nodes and L live states the complete pass costs
/// O(N + L log T), with O(log T) traversal scratch. Existing Vec growth remains
/// amortized; allocator work and caller-owned payload destruction are not time
/// bounded. The old arena remains allocated until its last node is retired,
/// so peak storage includes both arenas. There is no whole-tree final copy.
pub struct CheckpointCompaction<S> {
    source: CheckpointIndex<S>,
    destination: CheckpointIndex<S>,
    stack: Vec<usize>,
    scratch_allocations: u64,
    visited: usize,
    moved: usize,
    retired: usize,
}

/// Separate arena capacities and work counters for explicit cache maintenance.
/// External payload heaps and the caller's output Vec are not included.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CheckpointCompactionStats {
    pub source: crate::ArenaStats,
    pub destination: crate::ArenaStats,
    pub scratch: crate::ArenaStats,
    pub visited: usize,
    pub moved: usize,
    pub retired: usize,
    /// Cumulative tree work, including work before compaction began.
    pub mutation_visits: u64,
}

impl<S> CheckpointCompaction<S> {
    pub fn len(&self) -> usize {
        self.destination.len()
            + if self.stack.is_empty() {
                0
            } else {
                self.source.len()
            }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Same O(log T) predecessor semantics while either tree owns live states.
    pub fn previous(&self, tile: TileId) -> Option<(TileId, &S)> {
        let new = self.destination.previous(tile);
        let old = if self.stack.is_empty() {
            None
        } else {
            self.source.previous(tile)
        };
        match (old, new) {
            (Some(a), Some(b)) => Some(if a.0 > b.0 { a } else { b }),
            (a, b) => a.or(b),
        }
    }

    pub fn get(&self, tile: TileId) -> Option<&S> {
        self.previous(tile)
            .filter(|(id, _)| *id == tile)
            .map(|(_, state)| state)
    }

    pub fn token(&self, tile: TileId) -> Option<CheckpointToken> {
        self.destination.token(tile).or_else(|| {
            if self.stack.is_empty() {
                None
            } else {
                self.source.token(tile)
            }
        })
    }

    pub fn validates(&self, token: CheckpointToken) -> bool {
        self.token(token.tile) == Some(token)
    }

    pub fn stats(&self) -> CheckpointCompactionStats {
        CheckpointCompactionStats {
            source: self.source.arena_stats(),
            destination: self.destination.arena_stats(),
            scratch: crate::ArenaStats {
                nodes: self.stack.len(),
                capacity_bytes: self.stack.capacity() * std::mem::size_of::<usize>(),
                allocation_calls: self.scratch_allocations,
            },
            visited: self.visited,
            moved: self.moved,
            retired: self.retired,
            mutation_visits: self.source.visits + self.destination.visits,
        }
    }

    /// Move live states, then pop historical nodes one at a time. Append stale
    /// payloads to `out`; never clear it or invoke their destructors. Zero or
    /// asymmetric insufficient budget leaves the next step wholly untouched.
    /// Covered is repeatable with no further work. Output order is unspecified.
    pub fn advance(&mut self, budget: &mut crate::Budget, out: &mut Vec<S>) -> crate::Completion {
        while !self.stack.is_empty() || !self.source.nodes.is_empty() {
            if !budget.charge(crate::WorkLimit {
                objects: 1,
                queries: 1,
                ..Default::default()
            }) {
                return crate::Completion::BudgetExhausted;
            }
            self.source.visits += 1;
            if let Some(id) = self.stack.pop() {
                self.visited += 1;
                let node = &mut self.source.nodes[id];
                // Counts below an invalid ancestor may be stale. Never descend
                // through it, even when its descendants still claim validity.
                if node.valid == 0 {
                    continue;
                }
                if let Some(state) = node.state.take() {
                    let tile = node.tile.expect("live checkpoint leaf");
                    let version = node.version;
                    self.source
                        .invalidate(tile.0..tile.0 + 1)
                        .expect("valid tile");
                    let replaced = self.destination.insert_versioned(tile, state, version);
                    debug_assert!(replaced.is_none());
                    self.moved += 1;
                } else {
                    for child in node.children.into_iter().rev().flatten() {
                        if self.stack.len() == self.stack.capacity() {
                            self.scratch_allocations += 1;
                        }
                        self.stack.push(child.get() - 1);
                    }
                }
            } else {
                let mut node = self.source.nodes.pop().expect("remaining retired node");
                if let Some(state) = node.state.take() {
                    out.push(state);
                }
                self.retired += 1;
                if self.source.nodes.is_empty() {
                    // len is zero: release the allocation without scanning or
                    // dropping N empty Option<S> slots in a single final step.
                    self.source.nodes = Vec::new();
                }
            }
        }
        crate::Completion::Covered
    }

    /// Return the writable index only after all historical nodes are retired.
    /// An unfinished task is returned unchanged for further budgeted work.
    /// Reset any old `drain_before` arena cursor to zero after compaction.
    #[allow(clippy::result_large_err)] // Return the owned task without a new heap allocation.
    pub fn finish(mut self) -> Result<CheckpointIndex<S>, Self> {
        if !self.stack.is_empty() || !self.source.nodes.is_empty() {
            return Err(self);
        }
        self.destination.visits += self.source.visits;
        self.destination.allocations += self.source.allocations;
        Ok(self.destination)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CheckpointToken {
    index: DocumentId,
    pub(crate) tile: TileId,
    version: u64,
}
