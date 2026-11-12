//! Fixed-size shared symbol facts for hosted Markdown arenas. A body binds only
//! the symbols it actually uses; definitions never visit old body arenas.
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

#[derive(Default)]
pub(crate) struct Symbol {
    resolved: AtomicBool,
    revision: AtomicU64,
}
impl Symbol {
    pub(crate) fn publish(&self, resolved: bool, revision: u64) {
        self.resolved.store(resolved, Ordering::Relaxed);
        self.revision.store(revision, Ordering::Relaxed);
    }
    pub(crate) fn resolved(&self) -> bool {
        self.resolved.load(Ordering::Relaxed)
    }
    pub(crate) fn revision(&self) -> u64 {
        self.revision.load(Ordering::Relaxed)
    }
}

#[derive(Default)]
pub(crate) struct Clock {
    pub(crate) revision: AtomicU64,
    pub(crate) finished: AtomicBool,
}
