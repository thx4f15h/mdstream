use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default)]
struct Node {
    children: [Option<usize>; 2],
    end: usize,
}

/// Sparse segment tree over source/preorder arena indices. A subtree's maximum
/// end lets lookup skip closed siblings without walking any ancestors.
/// Open leaves use usize::MAX; only close events change their endpoints.
pub(crate) struct FoldRanges {
    nodes: Vec<Node>,
    root: usize,
    capacity: usize,
    visits: AtomicU64,
}

impl Default for FoldRanges {
    fn default() -> Self {
        Self {
            nodes: vec![Node::default()],
            root: 0,
            capacity: 1,
            visits: AtomicU64::new(0),
        }
    }
}

impl FoldRanges {
    pub fn visits(&self) -> u64 {
        self.visits.load(Ordering::Relaxed)
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn set(&mut self, id: usize, end: usize) {
        while id >= self.capacity {
            let root = self.nodes.len();
            self.nodes.push(Node {
                children: [Some(self.root), None],
                end: self.nodes[self.root].end,
            });
            self.root = root;
            self.capacity *= 2;
        }
        self.update(self.root, 0, self.capacity, id, end);
    }

    fn update(&mut self, node: usize, lo: usize, hi: usize, at: usize, end: usize) {
        *self.visits.get_mut() += 1;
        if hi - lo == 1 {
            self.nodes[node].end = end;
            return;
        }
        let mid = lo + (hi - lo) / 2;
        let side = usize::from(at >= mid);
        let child = self.nodes[node].children[side].unwrap_or_else(|| {
            let child = self.nodes.len();
            self.nodes.push(Node::default());
            self.nodes[node].children[side] = Some(child);
            child
        });
        let (lo, hi) = if side == 0 { (lo, mid) } else { (mid, hi) };
        self.update(child, lo, hi, at, end);
        self.nodes[node].end = self.nodes[node]
            .children
            .iter()
            .flatten()
            .map(|&child| self.nodes[child].end)
            .max()
            .unwrap_or(0);
    }

    /// First selected ID below `before` with end > offset. Preorder chooses the
    /// outermost range even when several containers share one opener line.
    pub fn covering(&self, before: usize, offset: usize) -> Option<usize> {
        self.find(self.root, 0, self.capacity, before, offset)
    }

    fn find(
        &self,
        node: usize,
        lo: usize,
        hi: usize,
        before: usize,
        offset: usize,
    ) -> Option<usize> {
        self.visits.fetch_add(1, Ordering::Relaxed);
        if lo >= before || self.nodes[node].end <= offset {
            return None;
        }
        if hi - lo == 1 {
            return Some(lo);
        }
        let mid = lo + (hi - lo) / 2;
        for (side, lo, hi) in [(0, lo, mid), (1, mid, hi)] {
            if let Some(child) = self.nodes[node].children[side]
                && let Some(found) = self.find(child, lo, hi, before, offset)
            {
                return Some(found);
            }
        }
        None
    }
}
