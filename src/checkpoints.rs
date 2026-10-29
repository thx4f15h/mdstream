use std::ops::Range;

use crate::{Document, LayoutError, TileId, Update};

struct Node<S> {
    children: [Option<usize>; 2],
    valid: usize,
    state: Option<S>,
}

impl<S> Default for Node<S> {
    fn default() -> Self {
        Self {
            children: [None; 2],
            valid: 0,
            state: None,
        }
    }
}

/// Sparse renderer-owned resume states, keyed by the tile they precede.
///
/// Invalidating a range only clears subtree counts; looking up a predecessor
/// skips invalid subtrees in O(log maximum tile index), even after millions of
/// cached states become stale. Invalid states are retained until overwritten or
/// this index is dropped, so invalidation never runs an unbounded number of user
/// destructors. Memory depends on positions ever cached, not invalidation count.
pub struct CheckpointIndex<S> {
    nodes: Vec<Node<S>>,
    root: usize,
    capacity: usize,
    revision: Option<u64>,
    visits: u64,
}

impl<S> Default for CheckpointIndex<S> {
    fn default() -> Self {
        Self {
            nodes: vec![Node::default()],
            root: 0,
            capacity: 1,
            revision: None,
            visits: 0,
        }
    }
}

impl<S> CheckpointIndex<S> {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn len(&self) -> usize {
        self.nodes[self.root].valid
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }
    pub fn mutation_visits(&self) -> u64 {
        self.visits
    }

    pub fn insert(&mut self, tile: TileId, state: S) -> Result<Option<S>, LayoutError> {
        if tile.0 >= isize::MAX as usize {
            return Err(LayoutError::InvalidRange);
        }
        while self.capacity <= tile.0 {
            let root = self.nodes.len();
            self.nodes.push(Node {
                children: [Some(self.root), None],
                valid: self.nodes[self.root].valid,
                state: None,
            });
            self.root = root;
            self.capacity *= 2;
        }
        Ok(self.set(self.root, 0, self.capacity, tile.0, state))
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
    }

    /// Conservative propagation of layout dependencies: all checkpoints at or
    /// after the first changed tile become invalid. Existing prefixes survive.
    /// Use this together with HeightIndex::apply after every append/finish.
    pub fn apply(&mut self, document: &Document, update: &Update) -> Result<(), LayoutError> {
        let previous = self.revision.unwrap_or(0);
        if update.revision != document.revision()
            || update.revision < previous
            || update.revision > previous + 1
        {
            return Err(LayoutError::OutOfOrderUpdate);
        }
        if !update.references_changed.is_empty() {
            self.clear();
        } else if let Some(start) = update.dirty.iter().map(|s| s.start).min()
            && let Some(tile) = document.tile_at(start)
            && tile.0 < self.capacity
        {
            self.invalidate(tile.0..self.capacity)?;
        }
        self.revision = Some(update.revision);
        Ok(())
    }

    fn set(&mut self, id: usize, lo: usize, hi: usize, tile: usize, state: S) -> Option<S> {
        self.visits += 1;
        if hi - lo == 1 {
            self.nodes[id].valid = 1;
            return self.nodes[id].state.replace(state);
        }
        // A cleared parent hides potentially stale children. Clear their counts
        // before reviving a single path, or insertion would resurrect old states.
        if self.nodes[id].valid == 0 {
            for child in self.nodes[id].children.into_iter().flatten() {
                self.nodes[child].valid = 0;
            }
        }
        let mid = lo + (hi - lo) / 2;
        let side = usize::from(tile >= mid);
        let child = match self.nodes[id].children[side] {
            Some(child) => child,
            None => {
                let child = self.nodes.len();
                self.nodes.push(Node::default());
                self.nodes[id].children[side] = Some(child);
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
            .map(|child| self.nodes[child].valid)
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
            if let Some(child) = self.nodes[id].children[side] {
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
        if let Some(right) = node.children[1]
            && let Some(found) = self.find(right, mid, hi, at)
        {
            return Some(found);
        }
        node.children[0].and_then(|left| self.find(left, lo, mid, at))
    }
}
