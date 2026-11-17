//! A single cooperative mutation lane. Futures own their continuations; they
//! never borrow the document, and are not cancelled by dropping a live job.
use crate::{Budget, WorkLimit};
use std::{
    cell::RefCell,
    future::Future,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll, Waker},
};

#[derive(Clone, Copy, Debug)]
pub(super) enum Op {
    Rope,
    Trie,
    Compare,
    Numeric,
    Projection,
    Normalize(usize),
    Generated(usize),
    Emit,
    Copy,
    Suffix,
    Retire,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SlugWork {
    pub source_bytes: u64,
    pub generated_input_bytes: u64,
    pub normalized_bytes: u64,
    pub copied_bytes: u64,
    pub suffix_bytes: u64,
    pub rope_steps: u64,
    pub trie_steps: u64,
    pub compared_bytes: u64,
    pub numeric_steps: u64,
    pub projection_steps: u64,
    pub retired: u64,
    pub renamed: u64,
    pub changed_bases: u64,
    pub effects: u64,
    pub metadata_steps: u64,
    pub heading_reads: u64,
    pub query_bytes: u64,
}

struct Quota {
    budget: Budget,
    work: SlugWork,
    future_bytes: usize,
    future_peak: usize,
    storage: [[usize; 3]; 9],
}

#[derive(Clone)]
pub(super) struct Cx(Rc<RefCell<Quota>>);
impl Default for Cx {
    fn default() -> Self {
        Self(Rc::new(RefCell::new(Quota {
            budget: Budget::new(WorkLimit::default()),
            work: SlugWork::default(),
            future_bytes: 0,
            future_peak: 0,
            storage: [[0; 3]; 9],
        })))
    }
}
impl Cx {
    pub fn charge(&self, op: Op) -> Charge {
        Charge {
            cx: self.clone(),
            op,
        }
    }
    pub fn storage(&self, k: usize, bytes: usize, dirs: usize, live: usize) {
        self.0.borrow_mut().storage[k] = [bytes, dirs, live];
    }
    pub fn capacity_bytes(&self) -> usize {
        std::mem::size_of::<RefCell<Quota>>()
    }
    pub fn stores(&self) -> [[usize; 3]; 9] {
        self.0.borrow().storage
    }
    pub fn work(&self) -> SlugWork {
        self.0.borrow().work
    }
    pub fn effect(&self) {
        self.0.borrow_mut().work.effects += 1;
    }
    pub fn metadata(&self) {
        self.0.borrow_mut().work.metadata_steps += 1;
    }
    pub fn heading(&self) {
        self.0.borrow_mut().work.heading_reads += 1;
    }
    pub fn query(&self, n: usize) {
        self.0.borrow_mut().work.query_bytes += n as u64;
    }
    pub fn base_changed(&self) {
        self.0.borrow_mut().work.changed_bases += 1;
    }
    pub fn changed(&self) {
        self.0.borrow_mut().work.renamed += 1;
    }
    pub fn future_bytes(&self) -> (usize, usize) {
        let q = self.0.borrow();
        (q.future_bytes, q.future_peak)
    }
    pub fn poll<T>(&self, job: Pin<&mut dyn Future<Output = T>>, budget: &mut Budget) -> Poll<T> {
        self.0.borrow_mut().budget = Budget::new(budget.remaining());
        let result = job.poll(&mut Context::from_waker(Waker::noop()));
        let used = self.0.borrow().budget.used();
        assert!(budget.charge(used));
        result
    }
}
pub(super) struct Charge {
    cx: Cx,
    op: Op,
}
impl Future for Charge {
    type Output = ();
    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<()> {
        let mut q = self.cx.0.borrow_mut();
        let mut cost = WorkLimit {
            objects: 1,
            ..Default::default()
        };
        match self.op {
            Op::Normalize(n) | Op::Generated(n) => cost.bytes = n,
            Op::Compare => cost.bytes = 2,
            Op::Emit | Op::Copy | Op::Suffix => cost.bytes = 1,
            Op::Trie | Op::Numeric | Op::Projection => cost.queries = 1,
            Op::Rope | Op::Retire => (),
        }
        if !q.budget.charge(cost) {
            return Poll::Pending;
        }
        match self.op {
            Op::Normalize(n) => q.work.source_bytes += n as u64,
            Op::Generated(n) => q.work.generated_input_bytes += n as u64,
            Op::Emit => q.work.normalized_bytes += 1,
            Op::Copy => q.work.copied_bytes += 1,
            Op::Suffix => q.work.suffix_bytes += 1,
            Op::Compare => q.work.compared_bytes += 1,
            Op::Rope => q.work.rope_steps += 1,
            Op::Trie => q.work.trie_steps += 1,
            Op::Numeric => q.work.numeric_steps += 1,
            Op::Projection => q.work.projection_steps += 1,
            Op::Retire => q.work.retired += 1,
        }
        Poll::Ready(())
    }
}

// Safe pinning, including recursive rope/AVL continuations. Both actual heap
// payload sizes are counted. Each recursive allocation follows a paid step.
pub(super) type Job<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;
struct Tracked<'a, T> {
    inner: Job<'a, T>,
    cx: Cx,
    bytes: usize,
}
impl<T> Future for Tracked<'_, T> {
    type Output = T;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        self.inner.as_mut().poll(cx)
    }
}
impl<T> Drop for Tracked<'_, T> {
    fn drop(&mut self) {
        self.cx.0.borrow_mut().future_bytes -= self.bytes;
    }
}
pub(super) fn boxed<'a, T: 'a>(cx: &Cx, future: impl Future<Output = T> + 'a) -> Job<'a, T> {
    let bytes = std::mem::size_of_val(&future) + std::mem::size_of::<Tracked<'_, T>>();
    {
        let mut q = cx.0.borrow_mut();
        q.future_bytes += bytes;
        q.future_peak = q.future_peak.max(q.future_bytes);
    }
    Box::pin(Tracked {
        inner: Box::pin(future),
        cx: cx.clone(),
        bytes,
    })
}
