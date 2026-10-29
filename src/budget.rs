/// Independent limits for one frame or explicit background task. Reuse the same
/// Budget through queries, recovery, objects, measurement and correction loops.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorkLimit {
    pub queries: usize,
    pub tiles: usize,
    pub bytes: usize,
    pub objects: usize,
    pub recovery: usize,
    pub dependencies: usize,
}

impl WorkLimit {
    pub const fn unlimited() -> Self {
        Self {
            queries: usize::MAX,
            tiles: usize::MAX,
            bytes: usize::MAX,
            objects: usize::MAX,
            recovery: usize::MAX,
            dependencies: usize::MAX,
        }
    }
    fn values(self) -> [usize; 6] {
        [
            self.queries,
            self.tiles,
            self.bytes,
            self.objects,
            self.recovery,
            self.dependencies,
        ]
    }
    fn from_values(v: [usize; 6]) -> Self {
        Self {
            queries: v[0],
            tiles: v[1],
            bytes: v[2],
            objects: v[3],
            recovery: v[4],
            dependencies: v[5],
        }
    }
}

#[derive(Debug)]
pub struct Budget {
    limit: WorkLimit,
    used: WorkLimit,
}

impl Budget {
    pub fn new(limit: WorkLimit) -> Self {
        Self {
            limit,
            used: WorkLimit::default(),
        }
    }
    pub fn used(&self) -> WorkLimit {
        self.used
    }
    pub fn remaining(&self) -> WorkLimit {
        let used = self.used.values();
        WorkLimit::from_values(std::array::from_fn(|i| self.limit.values()[i] - used[i]))
    }
    /// Atomic consumption: a failed step consumes nothing.
    pub fn charge(&mut self, cost: WorkLimit) -> bool {
        let remaining = self.remaining().values();
        let costs = cost.values();
        if (0..6).any(|i| costs[i] > remaining[i]) {
            return false;
        }
        let used = self.used.values();
        self.used = WorkLimit::from_values(std::array::from_fn(|i| used[i] + costs[i]));
        true
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Completion {
    Covered,
    EndOfDocument,
    BudgetExhausted,
}
