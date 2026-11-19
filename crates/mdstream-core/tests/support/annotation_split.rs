//! Private, synchronous layout experiment. Never compiled into the library API.
//! The parser is used only to capture inputs; the array oracle interprets writes.
use super::{Annotations, Mark, Node};
use crate::{LinkId, Span, Style, rewind::UndoLog};
use std::{
    cell::Cell,
    hint::black_box,
    mem::{align_of, size_of},
    num::NonZeroUsize,
};

#[path = "storage_input.rs"]
mod input;

#[derive(Clone, Copy, Debug)]
pub(super) enum Op {
    Apply(Span, Mark),
    Begin,
    Commit,
    Rollback,
    Boundary(usize),
}

#[derive(Default)]
pub(super) struct Probe {
    trace: Option<Vec<Op>>,
    trace_requests: u64,
    pub lookup: Cell<u64>,
    pub revision: Cell<u64>,
    pub tag_writes: u64,
    pub undo_requests: u64,
}
impl Probe {
    pub fn record(&mut self, op: Op) {
        if let Some(trace) = &mut self.trace {
            self.trace_requests += u64::from(trace.len() == trace.capacity());
            trace.push(op);
        }
    }
}

// Exactly the current Mark's semantic fields. Serial is semantic precedence,
// not a revision: moving it to a max-only revision tree would lose rewrites.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Semantic {
    style: Style,
    base_hidden: bool,
    delimiter_hidden: bool,
    emphasis_depth: usize,
    strong_depth: usize,
    link: Option<LinkId>,
    hide_when_target_available: Option<NonZeroUsize>,
    hide_alternative: Option<NonZeroUsize>,
    link_syntax: bool,
    serial: u64,
}
impl From<Mark> for Semantic {
    fn from(m: Mark) -> Self {
        Self {
            style: m.style,
            base_hidden: m.base_hidden,
            delimiter_hidden: m.delimiter_hidden,
            emphasis_depth: m.emphasis_depth,
            strong_depth: m.strong_depth,
            link: m.link,
            hide_when_target_available: m.hide_when_target_available,
            hide_alternative: m.hide_alternative,
            link_syntax: m.link_syntax,
            serial: m.serial,
        }
    }
}
impl Semantic {
    fn mark(self, revision: u64) -> Mark {
        Mark {
            style: self.style,
            base_hidden: self.base_hidden,
            delimiter_hidden: self.delimiter_hidden,
            emphasis_depth: self.emphasis_depth,
            strong_depth: self.strong_depth,
            link: self.link,
            hide_when_target_available: self.hide_when_target_available,
            hide_alternative: self.hide_alternative,
            link_syntax: self.link_syntax,
            serial: self.serial,
            revision,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct RevisionNode {
    saved_epoch: u64,
    children: [Option<NonZeroUsize>; 2],
    semantic: Option<NonZeroUsize>,
    tag_revision: u64,
    newest: u64,
    hidden: bool,
}
type Undo = (usize, RevisionNode, Option<Semantic>);
struct Checkpoint {
    nodes: usize,
    semantics: usize,
    root: usize,
    extent: usize,
    serial: u64,
    undo: UndoLog<Undo>,
}

struct Split {
    revisions: Vec<RevisionNode>,
    semantics: Vec<Semantic>,
    root: usize,
    extent: usize,
    serial: u64,
    epoch: u64,
    checkpoint: Option<Checkpoint>,
    revision_requests: u64,
    semantic_requests: u64,
    undo_requests: u64,
    update_visits: u64,
    tag_writes: u64,
    semantic_writes: u64,
    merge_reads: u64,
    undo_records: u64,
    commit_records: u64,
    restore_semantic_stores: u64,
    undo_peak: crate::rewind::UndoStats,
    peak: usize,
    lookup: Cell<u64>,
    lookup_semantics: Cell<u64>,
    revision: Cell<u64>,
}
impl Default for Split {
    fn default() -> Self {
        Self {
            revisions: vec![RevisionNode::default()],
            semantics: Vec::new(),
            root: 0,
            extent: 1,
            serial: 0,
            epoch: 0,
            checkpoint: None,
            revision_requests: 1,
            semantic_requests: 0,
            undo_requests: 0,
            update_visits: 0,
            tag_writes: 0,
            semantic_writes: 0,
            merge_reads: 0,
            undo_records: 0,
            commit_records: 0,
            restore_semantic_stores: 0,
            undo_peak: crate::rewind::UndoStats::default(),
            peak: size_of::<RevisionNode>(),
            lookup: Cell::new(0),
            lookup_semantics: Cell::new(0),
            revision: Cell::new(0),
        }
    }
}
impl Split {
    fn tag(&self, id: usize) -> Mark {
        let n = self.revisions[id];
        n.semantic
            .map_or_else(Semantic::default, |s| self.semantics[s.get() - 1])
            .mark(n.tag_revision)
    }
    fn push_node(&mut self, node: RevisionNode) {
        self.revision_requests += u64::from(self.revisions.len() == self.revisions.capacity());
        self.revisions.push(node);
    }
    fn step(&mut self, op: Op) {
        match op {
            Op::Apply(span, mark) => {
                assert!(!span.is_empty());
                while self.extent < span.end {
                    let root = self.revisions.len();
                    self.push_node(RevisionNode {
                        children: [NonZeroUsize::new(self.root + 1), None],
                        newest: self.revisions[self.root].newest,
                        ..RevisionNode::default()
                    });
                    self.root = root;
                    self.extent *= 2;
                }
                self.serial += 1;
                assert_eq!(
                    mark.serial,
                    if mark.link.is_some() { self.serial } else { 0 }
                );
                self.update(self.root, 0, self.extent, span, mark);
            }
            Op::Begin => {
                assert!(self.checkpoint.is_none());
                self.epoch += 1;
                self.checkpoint = Some(Checkpoint {
                    nodes: self.revisions.len(),
                    semantics: self.semantics.len(),
                    root: self.root,
                    extent: self.extent,
                    serial: self.serial,
                    undo: UndoLog::default(),
                });
            }
            Op::Commit => {
                if let Some(c) = self.checkpoint.take() {
                    self.commit_records += c.undo.len() as u64;
                    self.undo_peak = self.undo_peak.max(c.undo.stats());
                }
            }
            Op::Rollback => {
                let mut c = self.checkpoint.take().unwrap();
                self.undo_peak = self.undo_peak.max(c.undo.stats());
                while let Some((id, node, semantic)) = c.undo.pop() {
                    self.undo_records += 1;
                    self.revisions[id] = node;
                    if let Some(slot) = node.semantic {
                        self.semantics[slot.get() - 1] = semantic.unwrap();
                        self.restore_semantic_stores += 1;
                    }
                }
                self.revisions.truncate(c.nodes);
                self.semantics.truncate(c.semantics);
                self.root = c.root;
                self.extent = c.extent;
                self.serial = c.serial;
            }
            Op::Boundary(_) => {}
        }
        self.peak = self
            .peak
            .max(self.capacity_bytes() + self.active_undo_bytes());
    }
    fn update(&mut self, id: usize, lo: usize, hi: usize, span: Span, mark: Mark) {
        self.update_visits += 1;
        let n = self.revisions[id];
        if let Some(c) = &mut self.checkpoint
            && id < c.nodes
            && n.saved_epoch != self.epoch
        {
            let old_bytes = c.undo.stats().bytes;
            c.undo
                .push((id, n, n.semantic.map(|s| self.semantics[s.get() - 1])));
            self.undo_requests += u64::from(c.undo.stats().bytes > old_bytes);
            self.revisions[id].saved_epoch = self.epoch;
        }
        self.revisions[id].newest = n.newest.max(mark.revision);
        if span.start <= lo && hi <= span.end {
            self.tag_writes += 1;
            self.revisions[id].tag_revision = n.tag_revision.max(mark.revision);
            if Semantic::from(mark) != Semantic::default() {
                self.semantic_writes += 1;
                if let Some(slot) = n.semantic {
                    self.merge_reads += 1;
                    self.semantics[slot.get() - 1] =
                        self.semantics[slot.get() - 1].mark(0).merge(mark).into();
                } else {
                    self.semantic_requests +=
                        u64::from(self.semantics.len() == self.semantics.capacity());
                    self.semantics.push(mark.into());
                    self.revisions[id].semantic = NonZeroUsize::new(self.semantics.len());
                }
            }
            self.revisions[id].hidden |= mark.style.contains(Style::HIDDEN);
            return;
        }
        let mid = lo + (hi - lo) / 2;
        for (side, start, end) in [(0, lo, mid), (1, mid, hi)] {
            if span.start < end && start < span.end {
                let child = if let Some(child) = self.revisions[id].children[side] {
                    child.get() - 1
                } else {
                    let child = self.revisions.len();
                    self.push_node(RevisionNode::default());
                    self.revisions[id].children[side] = NonZeroUsize::new(child + 1);
                    child
                };
                self.update(child, start, end, span, mark);
            }
        }
        self.revisions[id].hidden |= self.revisions[id]
            .children
            .iter()
            .all(|c| c.is_some_and(|c| self.revisions[c.get() - 1].hidden));
    }
    fn at(&self, offset: usize) -> Mark {
        if offset >= self.extent {
            return Mark::default();
        }
        let (mut id, mut lo, mut hi) = (self.root, 0, self.extent);
        let mut mark = Mark::default();
        loop {
            self.lookup.set(self.lookup.get() + 1);
            let n = self.revisions[id];
            self.lookup_semantics
                .set(self.lookup_semantics.get() + u64::from(n.semantic.is_some()));
            mark = mark.merge(self.tag(id));
            let mid = lo + (hi - lo) / 2;
            let side = usize::from(offset >= mid);
            if let Some(child) = n.children[side] {
                id = child.get() - 1;
                if side == 0 {
                    hi = mid;
                } else {
                    lo = mid;
                }
            } else {
                return mark;
            }
        }
    }
    fn newest(&self, span: Span) -> u64 {
        self.query(self.root, 0, self.extent, span)
    }
    fn query(&self, id: usize, lo: usize, hi: usize, span: Span) -> u64 {
        if span.start >= hi || span.end <= lo {
            return 0;
        }
        self.revision.set(self.revision.get() + 1);
        let n = self.revisions[id];
        if span.start <= lo && hi <= span.end {
            return n.newest;
        }
        let mid = lo + (hi - lo) / 2;
        let mut revision = n.tag_revision;
        for (side, start, end) in [(0, lo, mid), (1, mid, hi)] {
            if let Some(child) = n.children[side] {
                revision = revision.max(self.query(child.get() - 1, start, end, span));
            }
        }
        revision
    }
    fn capacity_bytes(&self) -> usize {
        self.revisions.capacity() * size_of::<RevisionNode>()
            + self.semantics.capacity() * size_of::<Semantic>()
    }
    fn active_undo_bytes(&self) -> usize {
        self.checkpoint.as_ref().map_or(0, |c| c.undo.stats().bytes)
    }
}

// A plain byte array with direct field interpretation. Never calls Mark::merge,
// reads a tree, or derives expected values from either measured implementation.
struct Model {
    bytes: Vec<Mark>,
    undo: Option<std::collections::BTreeMap<usize, Mark>>,
    serial: u64,
    saved_serial: u64,
}
impl Model {
    fn new(len: usize) -> Self {
        Self {
            bytes: vec![Mark::default(); len],
            undo: None,
            serial: 0,
            saved_serial: 0,
        }
    }
    fn step(&mut self, op: Op) {
        match op {
            Op::Apply(span, m) => {
                self.serial += 1;
                assert_eq!(m.serial, if m.link.is_some() { self.serial } else { 0 });
                for at in span.start..span.end {
                    let a = &mut self.bytes[at];
                    if let Some(undo) = &mut self.undo {
                        undo.entry(at).or_insert(*a);
                    }
                    let replace = match (a.link, m.link) {
                        (_, None) => false,
                        (old, Some(new)) if a.link_syntax && m.link_syntax => {
                            Some(new) < old || (Some(new) == old && m.serial >= a.serial)
                        }
                        _ => m.serial >= a.serial,
                    };
                    if replace {
                        a.link = m.link;
                        a.link_syntax = m.link_syntax;
                    }
                    a.style = a.style.union(m.style);
                    a.base_hidden |= m.base_hidden;
                    a.delimiter_hidden |= m.delimiter_hidden;
                    a.emphasis_depth += m.emphasis_depth;
                    a.strong_depth += m.strong_depth;
                    for (old, new) in [
                        (
                            &mut a.hide_when_target_available,
                            m.hide_when_target_available,
                        ),
                        (&mut a.hide_alternative, m.hide_alternative),
                    ] {
                        if let Some(new) = new {
                            if let Some(old) = *old {
                                assert_eq!(old, new);
                            }
                            *old = Some(new);
                        }
                    }
                    a.revision = a.revision.max(m.revision);
                    a.serial = a.serial.max(m.serial);
                }
            }
            Op::Begin => {
                assert!(self.undo.is_none());
                self.saved_serial = self.serial;
                self.undo = Some(Default::default());
            }
            Op::Commit => {
                self.undo = None;
            }
            Op::Rollback => {
                for (at, m) in self.undo.take().unwrap() {
                    self.bytes[at] = m;
                }
                self.serial = self.saved_serial;
            }
            Op::Boundary(_) => {}
        }
    }
    fn newest(&self, span: Span) -> u64 {
        self.bytes[span.start..span.end]
            .iter()
            .map(|m| m.revision)
            .max()
            .unwrap_or(0)
    }
}
fn baseline_step(a: &mut Annotations, op: Op) -> u64 {
    match op {
        Op::Apply(span, m) => {
            a.apply_mark(span, m);
            0
        }
        Op::Begin => {
            a.begin_line();
            0
        }
        Op::Commit => a.commit_line(),
        Op::Rollback => a.rollback_line(),
        Op::Boundary(_) => 0,
    }
}
fn check(a: &Annotations, s: &Split, m: &Model, len: usize) {
    for at in 0..len {
        assert_eq!(a.at(at), m.bytes[at], "baseline byte {at}");
        assert_eq!(s.at(at), m.bytes[at], "split byte {at}");
    }
    for i in 0..64 {
        let start = (len / 2 + i % 8).min(len.saturating_sub(1));
        let span = Span::new(start, (start + 256).min(len));
        assert_eq!(a.newest(span), m.newest(span));
        assert_eq!(s.newest(span), m.newest(span));
    }
    assert_eq!(a.nodes.len(), s.revisions.len());
    assert_eq!((a.root, a.capacity, a.serial), (s.root, s.extent, s.serial));
    for (id, n) in a.nodes.iter().enumerate() {
        let r = s.revisions[id];
        assert_eq!(
            (n.children, n.newest, n.hidden, n.saved_epoch),
            (r.children, r.newest, r.hidden, r.saved_epoch)
        );
        assert_eq!(n.tag, s.tag(id));
    }
}
fn capture(case: &str, size: usize, chunk: usize) -> (crate::Document, Vec<Op>, String) {
    let source = match case {
        "table_rollback" => {
            // An inherited inline scope and a late pipe require the real
            // annotation transaction; an early leading pipe does not.
            let unit = format!("*prior\nend* [a](b) {}|b\n-|-\n\n", "x".repeat(2048));
            unit.repeat(size.div_ceil(unit.len()))
        }
        "tag_saturated" => input::source("plain", size),
        _ => input::source(case, size),
    };
    let mut doc = crate::Document::new();
    doc.annotations.experiment.trace = Some(Vec::new());
    let mut update = crate::Update::default();
    for bytes in source.as_bytes().chunks(chunk) {
        doc.append_into(std::str::from_utf8(bytes).unwrap(), &mut update)
            .unwrap();
        doc.annotations
            .experiment
            .record(Op::Boundary(doc.source().len()));
    }
    doc.finish_into(&mut update);
    if case == "tag_saturated" {
        assert!(size.is_power_of_two());
        // Supplemental storage stress, not parser output: put a semantic tag
        // on every reachable node, then rewrite each byte twice per revision.
        let mut width = 1;
        while width <= size {
            for start in (0..size).step_by(width) {
                doc.annotations
                    .apply(Span::new(start, start + width), Style::STRONG, None, 100);
            }
            width *= 2;
        }
        for cycle in 0..8 {
            doc.annotations.begin_line();
            for at in 0..size {
                for id in [7, 9] {
                    doc.annotations.apply(
                        Span::new(at, at + 1),
                        Style::EMPHASIS,
                        Some(LinkId(id)),
                        101 + cycle,
                    );
                }
            }
            if cycle % 2 == 0 {
                doc.annotations.rollback_line();
            } else {
                doc.annotations.commit_line();
            }
        }
    }
    doc.annotations
        .experiment
        .record(Op::Boundary(source.len()));
    let trace = doc.annotations.experiment.trace.take().unwrap();
    if case == "table_rollback" {
        assert!(trace.iter().any(|op| matches!(op, Op::Begin)));
        assert!(trace.iter().any(|op| matches!(op, Op::Rollback)));
    }
    (doc, trace, source)
}

#[test]
fn storage_split_chunked_model() {
    for case in ["plain", "dense", "containers", "table_rollback"] {
        for chunk in [1, 7, 31, 4096] {
            let (doc, trace, source) = capture(case, 256, chunk);
            let (mut a, mut s, mut m) = (
                Annotations::default(),
                Split::default(),
                Model::new(source.len()),
            );
            for op in trace {
                baseline_step(&mut a, op);
                s.step(op);
                m.step(op);
                if let Op::Boundary(len) = op {
                    check(&a, &s, &m, len);
                }
            }
            for at in 0..source.len() {
                assert_eq!(doc.annotations.at(at), m.bytes[at]);
            }
        }
    }
}

#[test]
fn storage_split_rewrites_ids_and_rollback_model() {
    let mut producer = Annotations::default();
    producer.experiment.trace = Some(Vec::new());
    producer.apply(Span::new(0, 32), Style::STRONG, None, 1);
    producer.apply_object(Span::new(4, 20), Style::default(), LinkId(9), 2);
    producer.apply_object(Span::new(4, 20), Style::default(), LinkId(4), 2);
    producer.apply_object(Span::new(4, 20), Style::default(), LinkId(4), 2);
    producer.hide_when_either(Span::new(4, 20), LinkId(9), LinkId(10), 2);
    let mut seed = 0x4141_u64;
    for cycle in 0..40 {
        producer.begin_line();
        for _ in 0..17 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let start = (seed as usize) % 240;
            let end = (start + 1 + ((seed >> 32) as usize) % 16).min(256);
            producer.apply(
                Span::new(start, end),
                Style::EMPHASIS,
                Some(LinkId(cycle % 7)),
                3 + cycle as u64,
            );
        }
        if cycle % 2 == 0 {
            producer.rollback_line();
        } else {
            producer.commit_line();
        }
    }
    let trace = producer.experiment.trace.take().unwrap();
    let (mut a, mut s, mut m) = (Annotations::default(), Split::default(), Model::new(256));
    let mut saved = None;
    for op in trace {
        let old_slots: Vec<_> = s.revisions.iter().map(|n| n.semantic).collect();
        if matches!(op, Op::Begin) {
            saved = Some((s.revisions.clone(), s.semantics.clone()));
        }
        baseline_step(&mut a, op);
        s.step(op);
        m.step(op);
        if matches!(op, Op::Apply(_, mark) if mark.serial == 4) {
            assert_eq!(m.bytes[4].link, Some(LinkId(4)));
            assert_eq!(m.bytes[4].revision, 2);
            assert_eq!(m.bytes[4].serial, 4);
        }
        if matches!(op, Op::Apply(..)) {
            for (id, old) in old_slots.iter().enumerate() {
                if old.is_some() {
                    assert_eq!(*old, s.revisions[id].semantic);
                }
            }
        }
        if matches!(op, Op::Rollback) {
            let (nodes, semantics) = saved.take().unwrap();
            assert_eq!(s.revisions, nodes);
            assert_eq!(s.semantics, semantics);
        }
        check(&a, &s, &m, 256);
        for start in 0..16 {
            for end in start + 1..=32 {
                let span = Span::new(start, end);
                assert_eq!(a.newest(span), m.newest(span));
                assert_eq!(s.newest(span), m.newest(span));
            }
        }
    }
    // No revision bump was needed to preserve the smaller syntax ID's rewrite.
    assert_eq!(m.bytes[4].hide_owner(), Some(LinkId(9)));
    assert!(s.undo_records > 0 && s.restore_semantic_stores > 0);
}

// CSV is deliberately dependency-free. Allocation counts mean Vec allocation /
// growth requests, not global allocator invocations or resident memory.
struct Report {
    file: std::fs::File,
}
impl Report {
    fn new(path: &std::path::Path) -> Self {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)
            .unwrap();
        writeln!(file, "case,scale,chunk,variant,phase,metric,value").unwrap();
        Self { file }
    }
    fn row(
        &mut self,
        group: (&str, usize, usize),
        metric: (&str, &str, &str),
        value: impl TryInto<u64>,
    ) {
        use std::io::Write;
        let (case, scale, chunk) = group;
        let (variant, phase, key) = metric;
        let value = value.try_into().ok().unwrap();
        writeln!(
            self.file,
            "{case},{scale},{chunk},{variant},{phase},{key},{value}"
        )
        .unwrap();
    }
    fn arena<T>(
        &mut self,
        group: (&str, usize, usize),
        variant: &str,
        phase: &str,
        v: &Vec<T>,
        requests: u64,
    ) {
        for (key, value) in [
            ("len", v.len()),
            ("capacity", v.capacity()),
            ("used_bytes", v.len() * size_of::<T>()),
            ("capacity_bytes", v.capacity() * size_of::<T>()),
            ("slack_bytes", (v.capacity() - v.len()) * size_of::<T>()),
        ] {
            self.row(group, (variant, phase, key), value);
        }
        self.row(group, (variant, phase, "growth_requests"), requests);
    }
}

#[test]
#[ignore = "explicit release measurement; MDSTREAM_SPLIT_OUT must name a new CSV"]
fn storage_split_measurement() {
    let path = std::env::var_os("MDSTREAM_SPLIT_OUT").expect("set MDSTREAM_SPLIT_OUT");
    let mut report = Report::new(std::path::Path::new(&path));
    for (name, bytes, align) in [
        ("Mark", size_of::<Mark>(), align_of::<Mark>()),
        ("Node", size_of::<Node>(), align_of::<Node>()),
        ("Semantic", size_of::<Semantic>(), align_of::<Semantic>()),
        (
            "RevisionNode",
            size_of::<RevisionNode>(),
            align_of::<RevisionNode>(),
        ),
        (
            "baseline_undo_cell",
            size_of::<((usize, Node), usize)>(),
            align_of::<((usize, Node), usize)>(),
        ),
        (
            "split_undo_cell",
            size_of::<(Undo, usize)>(),
            align_of::<(Undo, usize)>(),
        ),
    ] {
        report.row(("layout", 0, 0), (name, "layout", "size"), bytes);
        report.row(("layout", 0, 0), (name, "layout", "alignment"), align);
    }
    for size in [4096, 16384, 65536] {
        for case in ["plain", "dense", "containers"] {
            measure(&mut report, case, size, 4096);
        }
    }
    for case in ["plain", "dense", "containers", "table_rollback"] {
        for chunk in [1, 7, 31, 4096] {
            measure(&mut report, case, 256, chunk);
        }
    }
    measure(&mut report, "table_rollback", 4096, 4096);
    measure(&mut report, "tag_saturated", 4096, 4096);
}
fn measure(report: &mut Report, case: &str, size: usize, chunk: usize) {
    use std::time::Instant;
    let start = Instant::now();
    let (doc, trace, source) = capture(case, size, chunk);
    let capture_ns = start.elapsed().as_nanos();
    let mut a = Annotations::default();
    let (mut peak, mut rollback, mut commit) = (a.arena_stats().capacity_bytes, 0_u64, 0_u64);
    let mut baseline_undo = (0, 0);
    let start = Instant::now();
    for &op in &trace {
        let n = baseline_step(&mut a, op);
        if matches!(op, Op::Rollback) {
            rollback += n;
        }
        if matches!(op, Op::Commit) {
            commit += n;
        }
        if let Some(c) = &a.checkpoint {
            baseline_undo = baseline_undo.max((c.4.stats().bytes, c.4.len()));
        }
        peak = peak.max(
            a.arena_stats().capacity_bytes + a.checkpoint.as_ref().map_or(0, |c| c.4.stats().bytes),
        );
    }
    let baseline_ns = start.elapsed().as_nanos();
    let mut s = Split::default();
    let mut split_undo = (0, 0);
    let start = Instant::now();
    for &op in &trace {
        s.step(op);
        if let Some(c) = &s.checkpoint {
            split_undo = split_undo.max((c.undo.stats().bytes, c.undo.len()));
        }
    }
    let split_ns = start.elapsed().as_nanos();
    let mut m = Model::new(source.len());
    for &op in &trace {
        m.step(op);
    }
    check(&a, &s, &m, source.len());
    for at in 0..source.len() {
        assert_eq!(doc.annotations.at(at), m.bytes[at]);
    }
    assert_eq!(doc.annotation_arena_stats(), a.arena_stats());
    assert_eq!(a.visits, s.update_visits);
    assert_eq!(a.experiment.tag_writes, s.tag_writes);
    assert_eq!(rollback, s.undo_records);
    assert_eq!(commit, s.commit_records);
    for (key, value) in [
        ("source_bytes", source.len() as u64),
        ("source_fnv1a", input::fingerprint(source.as_bytes())),
        ("ops", trace.len() as u64),
        (
            "append_boundaries",
            trace
                .iter()
                .filter(|op| matches!(op, Op::Boundary(_)))
                .count() as u64,
        ),
        (
            "begin",
            trace.iter().filter(|op| matches!(op, Op::Begin)).count() as u64,
        ),
        (
            "rollback",
            trace.iter().filter(|op| matches!(op, Op::Rollback)).count() as u64,
        ),
        ("capture_ns", capture_ns as u64),
    ] {
        report.row((case, size, chunk), ("input", "trace", key), value);
    }
    report.arena(
        (case, size, chunk),
        "baseline",
        "nodes",
        &a.nodes,
        a.allocations,
    );
    report.arena(
        (case, size, chunk),
        "split",
        "revisions",
        &s.revisions,
        s.revision_requests,
    );
    report.arena(
        (case, size, chunk),
        "split",
        "semantics",
        &s.semantics,
        s.semantic_requests,
    );
    report.arena(
        (case, size, chunk),
        "input",
        "trace_buffer",
        &trace,
        doc.annotations.experiment.trace_requests,
    );
    // The model's transient BTreeMap/other Document allocations are excluded.
    report.arena((case, size, chunk), "oracle", "byte_array", &m.bytes, 1);
    for (variant, (bytes, len), cell_bytes) in [
        (
            "baseline",
            baseline_undo,
            size_of::<((usize, Node), usize)>(),
        ),
        ("split", split_undo, size_of::<(Undo, usize)>()),
    ] {
        for (key, value) in [
            ("capacity_bytes", bytes),
            ("used_bytes", len * cell_bytes),
            ("slack_bytes", bytes - len * cell_bytes),
            ("len", len),
            ("capacity", bytes / cell_bytes),
        ] {
            report.row(
                (case, size, chunk),
                (variant, "undo_peak_observed", key),
                value,
            );
        }
    }
    for (variant, bytes, len, cell_bytes) in [
        (
            "baseline",
            a.checkpoint.as_ref().map_or(0, |c| c.4.stats().bytes),
            a.checkpoint.as_ref().map_or(0, |c| c.4.len()),
            size_of::<((usize, Node), usize)>(),
        ),
        (
            "split",
            s.active_undo_bytes(),
            s.checkpoint.as_ref().map_or(0, |c| c.undo.len()),
            size_of::<(Undo, usize)>(),
        ),
    ] {
        for (key, value) in [
            ("capacity_bytes", bytes),
            ("used_bytes", len * cell_bytes),
            ("slack_bytes", bytes - len * cell_bytes),
            ("len", len),
        ] {
            report.row((case, size, chunk), (variant, "undo_current", key), value);
        }
    }
    report.row(
        (case, size, chunk),
        ("input", "capture", "retained_annotation_arena_bytes"),
        doc.annotation_arena_stats().capacity_bytes,
    );
    report.row(
        (case, size, chunk),
        ("baseline", "replay", "vec_descriptor_bytes"),
        size_of::<Vec<Node>>(),
    );
    report.row(
        (case, size, chunk),
        ("split", "replay", "vec_descriptor_bytes"),
        size_of::<Vec<RevisionNode>>() + size_of::<Vec<Semantic>>(),
    );
    for (variant, key, value) in [
        ("baseline", "observed_peak_arena_plus_undo", peak as u64),
        ("split", "observed_peak_arena_plus_undo", s.peak as u64),
        (
            "baseline",
            "current_arena_plus_undo",
            (a.arena_stats().capacity_bytes
                + a.checkpoint.as_ref().map_or(0, |c| c.4.stats().bytes)) as u64,
        ),
        (
            "split",
            "current_arena_plus_undo",
            (s.capacity_bytes() + s.active_undo_bytes()) as u64,
        ),
        ("baseline", "undo_peak_bytes", a.undo_stats().bytes as u64),
        (
            "split",
            "undo_peak_bytes",
            s.undo_peak
                .max(
                    s.checkpoint
                        .as_ref()
                        .map_or(Default::default(), |c| c.undo.stats()),
                )
                .bytes as u64,
        ),
        ("baseline", "undo_requests", a.experiment.undo_requests),
        ("split", "undo_requests", s.undo_requests),
        ("baseline", "update_visits", a.visits),
        ("split", "update_visits", s.update_visits),
        ("baseline", "tag_writes", a.experiment.tag_writes),
        ("split", "tag_writes", s.tag_writes),
        ("split", "semantic_writes", s.semantic_writes),
        ("split", "semantic_merge_reads", s.merge_reads),
        (
            "split",
            "directory_used_bytes",
            (s.revisions.len() * size_of::<Option<NonZeroUsize>>()) as u64,
        ),
        (
            "split",
            "directory_capacity_bytes",
            (s.revisions.capacity() * size_of::<Option<NonZeroUsize>>()) as u64,
        ),
        ("baseline", "rollback_node_stores", rollback),
        ("split", "rollback_revision_stores", s.undo_records),
        (
            "split",
            "rollback_semantic_stores",
            s.restore_semantic_stores,
        ),
        ("baseline", "commit_records_dropped", commit),
        ("split", "commit_records_dropped", s.commit_records),
        ("baseline", "replay_ns", baseline_ns as u64),
        ("split", "replay_ns", split_ns as u64),
    ] {
        report.row((case, size, chunk), (variant, "replay", key), value);
    }
    a.experiment.lookup.set(0);
    a.experiment.revision.set(0);
    s.lookup.set(0);
    s.lookup_semantics.set(0);
    s.revision.set(0);
    let start = Instant::now();
    for i in 0..64 {
        let at = source.len() / 2 + i % 8;
        black_box(a.at(at));
    }
    let a_lookup = start.elapsed().as_nanos();
    let start = Instant::now();
    for i in 0..64 {
        let at = source.len() / 2 + i % 8;
        black_box(s.at(at));
    }
    let s_lookup = start.elapsed().as_nanos();
    let start = Instant::now();
    for i in 0..64 {
        let at = source.len() / 2 + i % 8;
        black_box(a.newest(Span::new(at, (at + 256).min(source.len()))));
    }
    let a_revision = start.elapsed().as_nanos();
    let start = Instant::now();
    for i in 0..64 {
        let at = source.len() / 2 + i % 8;
        black_box(s.newest(Span::new(at, (at + 256).min(source.len()))));
    }
    let s_revision = start.elapsed().as_nanos();
    assert_eq!(a.experiment.lookup.get(), s.lookup.get());
    assert_eq!(a.experiment.revision.get(), s.revision.get());
    for (variant, key, value) in [
        ("baseline", "lookup_node_reads", a.experiment.lookup.get()),
        ("split", "lookup_directory_reads", s.lookup.get()),
        ("split", "lookup_semantic_reads", s.lookup_semantics.get()),
        (
            "baseline",
            "range_revision_node_reads",
            a.experiment.revision.get(),
        ),
        ("split", "range_revision_node_reads", s.revision.get()),
        ("split", "range_revision_semantic_reads", 0),
        ("baseline", "lookup_ns", a_lookup as u64),
        ("split", "lookup_ns", s_lookup as u64),
        ("baseline", "range_revision_ns", a_revision as u64),
        ("split", "range_revision_ns", s_revision as u64),
    ] {
        report.row((case, size, chunk), (variant, "queries64", key), value);
    }
}
