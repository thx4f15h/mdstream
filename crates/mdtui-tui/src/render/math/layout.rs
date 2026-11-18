//! Bottom-up boxes, indexed child windows, and continued bounded-storage paint.
use super::storage::{Edge, List, Paged, StorageWork, retire_map};
use super::syntax::{Ink, Kind, Prefix, Syntax};
use mdtui3::{Budget, Span, WorkLimit};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Metrics {
    pub width: usize,
    pub height: usize,
    pub baseline: usize,
}
#[derive(Clone, Copy)]
pub(super) struct Placed {
    pub child: usize,
    pub x: usize,
    pub y: Option<usize>,
}
#[derive(Clone, Copy, Debug)]
pub(super) enum Primitive {
    Text {
        ink: Ink,
        width: usize,
        source: Span,
        x: usize,
        y: usize,
    },
    Rule {
        length: usize,
        source: Span,
        x: usize,
        y: usize,
    },
    Fence {
        ch: char,
        height: usize,
        source: Span,
        x: usize,
        y: usize,
    },
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum IndexKind {
    Small,
    X,
    Y,
}
#[derive(Clone, Copy)]
pub(super) struct BoxRecord {
    pub metrics: Metrics,
    pub has_ink: bool,
    pub edges: List,
    pub primitives: [Option<Primitive>; 3],
    pub index: IndexKind,
}
impl BoxRecord {
    fn empty(index: IndexKind) -> Self {
        Self {
            metrics: Metrics {
                width: 0,
                height: 1,
                baseline: 0,
            },
            has_ink: false,
            edges: List::default(),
            primitives: [None; 3],
            index,
        }
    }
}
#[derive(Clone, Copy, Default)]
struct Row {
    width: usize,
    base: usize,
    descent: usize,
}
impl Row {
    fn add(&mut self, m: Metrics, x: usize) {
        self.width = self.width.max(x + m.width);
        self.base = self.base.max(m.baseline);
        self.descent = self.descent.max(m.height - m.baseline)
    }
    fn metrics(self) -> Metrics {
        Metrics {
            width: self.width,
            height: self.base + self.descent.max(1),
            baseline: self.base,
        }
    }
}
#[derive(Clone, Copy)]
struct Matrix {
    columns: usize,
    fence: char,
    phase: u8,
    cell: usize,
    cell_row: Row,
    cells_end: usize,
    cell_at: usize,
    column: usize,
    total_width: usize,
    column_at: usize,
    x: usize,
    row: usize,
    row_box: usize,
    row_metrics: Row,
    height: usize,
}
#[derive(Clone, Copy)]
struct Build {
    node: usize,
    owner: usize,
    next: Option<usize>,
    row: Row,
    matrix: Option<Matrix>,
    phase: usize,
}
#[derive(Clone, Copy)]
struct RootEntry {
    ast_edge: Option<usize>,
    link: usize,
    summary: Row,
    has_ink: bool,
    ghost: bool,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct LayoutWork {
    pub steps: usize,
    pub edges: usize,
    pub boxes: usize,
    pub index_operations: usize,
    pub root_retracts: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutStep {
    Progress,
    Ready,
    NeedShape,
    BudgetExhausted,
}

pub(super) struct Layout {
    pub boxes: Paged<BoxRecord>,
    pub placed: Paged<Placed>,
    pub links: Paged<Edge>,
    node_boxes: Paged<usize>,
    pub index: BTreeMap<(usize, usize, usize), usize>,
    pub source_index: BTreeMap<(usize, usize, usize), Span>,
    pub parents: BTreeMap<usize, (usize, usize)>,
    columns: BTreeMap<(usize, usize), usize>,
    column_x: BTreeMap<(usize, usize), usize>,
    cells: Paged<usize>,
    root_entries: Paged<RootEntry>,
    next: usize,
    current: Option<Build>,
    pub root: Option<usize>,
    pub version: u64,
    root_version: Option<u64>,
    root_ghost: Option<usize>,
    pub work: LayoutWork,
}
fn pay(b: &mut Budget, bytes: usize) -> bool {
    b.charge(WorkLimit {
        queries: 1,
        objects: 1,
        bytes,
        ..WorkLimit::default()
    })
}
impl Layout {
    pub fn new() -> Self {
        Self {
            boxes: Paged::default(),
            placed: Paged::default(),
            links: Paged::default(),
            node_boxes: Paged::default(),
            index: BTreeMap::new(),
            source_index: BTreeMap::new(),
            parents: BTreeMap::new(),
            columns: BTreeMap::new(),
            column_x: BTreeMap::new(),
            cells: Paged::default(),
            root_entries: Paged::default(),
            next: 0,
            current: None,
            root: None,
            version: 0,
            root_version: None,
            root_ghost: None,
            work: LayoutWork::default(),
        }
    }
    fn new_box(&mut self, index: IndexKind) -> usize {
        self.work.boxes += 1;
        self.boxes.push(BoxRecord::empty(index))
    }
    fn edge(&mut self, owner: usize, child: usize, x: usize, y: Option<usize>) -> usize {
        let id = self.placed.push(Placed { child, x, y });
        let list = &mut self.boxes[owner].edges;
        list.push(&mut self.links, id);
        let link = list.last.unwrap();
        self.parents.insert(child, (owner, link));
        self.work.index_operations += 1;
        let coordinate = match self.boxes[owner].index {
            IndexKind::Y => y.unwrap(),
            _ => x,
        };
        if self.boxes[child].has_ink {
            self.boxes[owner].has_ink = true;
            self.index.insert((owner, coordinate, link), link);
            self.work.index_operations += 1;
        }
        self.work.edges += 1;
        link
    }
    fn finish_node(&mut self, owner: usize) {
        for at in 0..3 {
            if let Some(p) = self.boxes[owner].primitives[at] {
                let visible = match p {
                    Primitive::Text {
                        ink: Ink::Space(0), ..
                    } => false,
                    Primitive::Text { .. } => true,
                    Primitive::Rule { length, .. } => length != 0,
                    Primitive::Fence { ch, .. } => ch != '.',
                };
                if !visible {
                    continue;
                }
                self.boxes[owner].has_ink = true;
                let source = match p {
                    Primitive::Text { source, .. }
                    | Primitive::Rule { source, .. }
                    | Primitive::Fence { source, .. } => source,
                };
                if !source.is_empty() {
                    self.source_index.insert((source.start, owner, at), source);
                    self.work.index_operations += 1;
                }
            }
        }
        self.next += 1;
        self.current = None;
    }
    fn child(&self, syntax: &Syntax, node: usize, at: usize) -> usize {
        let id = syntax.nodes[node]
            .children
            .nth_small(&syntax.edges, at)
            .unwrap();
        self.node_boxes[id]
    }
    pub fn step(&mut self, syntax: &Syntax, b: &mut Budget) -> LayoutStep {
        if self.root.is_none() {
            if !pay(b, 0) {
                return LayoutStep::BudgetExhausted;
            }
            self.root = Some(self.new_box(IndexKind::X));
            self.work.steps += 1;
            return LayoutStep::Progress;
        }
        if let Some(prefix) = syntax.prefix()
            && prefix.error
        {
            if self.root_version == Some(prefix.version) {
                return LayoutStep::Ready;
            }
            if !pay(b, 0) {
                return LayoutStep::BudgetExhausted;
            }
            self.work.steps += 1;
            let root = self.root.unwrap();
            let record = &mut self.boxes[root];
            *record = BoxRecord::empty(IndexKind::Small);
            record.metrics = Metrics {
                width: 9,
                height: 1,
                baseline: 0,
            };
            record.has_ink = true;
            record.primitives[0] = Some(Primitive::Text {
                ink: Ink::Static("[invalid]"),
                width: 9,
                source: Span::new(0, 0),
                x: 0,
                y: 0,
            });
            self.root_version = Some(prefix.version);
            self.version += 1;
            return LayoutStep::Ready;
        }
        if syntax.root == Some(self.next) {
            if !pay(b, 0) {
                return LayoutStep::BudgetExhausted;
            }
            self.work.steps += 1;
            self.node_boxes.push(self.root.unwrap());
            self.next += 1;
            return LayoutStep::Progress;
        }
        if self.next < syntax.nodes.len() {
            let node = syntax.nodes[self.next];
            if node.needs_shape {
                return LayoutStep::NeedShape;
            }
            if !pay(b, 0) {
                return LayoutStep::BudgetExhausted;
            }
            self.work.steps += 1;
            if self.current.is_none() {
                let owner = self.new_box(if matches!(node.kind, Kind::Row) {
                    IndexKind::X
                } else {
                    IndexKind::Small
                });
                self.node_boxes.push(owner);
                self.current = Some(Build {
                    node: self.next,
                    owner,
                    next: node.children.first,
                    row: Row::default(),
                    matrix: if let Kind::Matrix { columns, fence } = node.kind {
                        Some(Matrix {
                            columns,
                            fence,
                            phase: 0,
                            cell: 0,
                            cell_row: Row::default(),
                            cells_end: self.cells.len(),
                            cell_at: self.cells.len(),
                            column: 0,
                            total_width: 0,
                            column_at: 0,
                            x: 0,
                            row: 0,
                            row_box: 0,
                            row_metrics: Row::default(),
                            height: 0,
                        })
                    } else {
                        None
                    },
                    phase: 0,
                });
                return LayoutStep::Progress;
            }
            let mut c = self.current.take().unwrap();
            if let Some(mut m) = c.matrix {
                match m.phase {
                    0 => {
                        m.cell = self.new_box(IndexKind::X);
                        m.phase = 1;
                    }
                    1 => {
                        if let Some(link) = c.next {
                            let ast = syntax.edges[link].child;
                            c.next = syntax.edges[link].next;
                            if matches!(syntax.nodes[ast].kind, Kind::CellEnd) {
                                self.boxes[m.cell].metrics = m.cell_row.metrics();
                                self.cells.push(m.cell);
                                m.cells_end = self.cells.len();
                                let key = (c.owner, m.column);
                                let old = *self.columns.get(&key).unwrap_or(&0);
                                let width = self.boxes[m.cell].metrics.width;
                                self.columns.insert(key, old.max(width));
                                m.total_width += old.max(width) - old;
                                self.work.index_operations += 1;
                                m.column = (m.column + 1) % m.columns;
                                m.cell_row = Row::default();
                                m.phase = if c.next.is_some() { 0 } else { 2 };
                            } else {
                                let child = self.node_boxes[ast];
                                let metric = self.boxes[child].metrics;
                                self.edge(m.cell, child, m.cell_row.width, None);
                                m.cell_row.add(metric, m.cell_row.width)
                            }
                        } else {
                            m.phase = 2
                        }
                    }
                    2 => {
                        if m.column_at < m.columns {
                            self.column_x.insert((c.owner, m.column_at), m.x);
                            m.x += self.columns[&(c.owner, m.column_at)] + 2;
                            m.column_at += 1;
                            self.work.index_operations += 1
                        } else {
                            m.phase = 3;
                            m.column = 0;
                            m.x = 0
                        }
                    }
                    3 => {
                        m.row_box = self.new_box(IndexKind::X);
                        m.row_metrics = Row::default();
                        m.phase = 4;
                    }
                    4 => {
                        if m.cell_at < m.cells_end {
                            let child = self.cells[m.cell_at];
                            let metric = self.boxes[child].metrics;
                            let mut x = self.column_x[&(c.owner, m.column)];
                            if m.fence == '=' && m.column.is_multiple_of(2) {
                                x += self.columns[&(c.owner, m.column)] - metric.width
                            }
                            self.edge(m.row_box, child, x, None);
                            m.row_metrics.add(metric, x);
                            m.cell_at += 1;
                            m.column += 1;
                            if m.column == m.columns || m.cell_at == m.cells_end {
                                self.boxes[m.row_box].metrics = m.row_metrics.metrics();
                                m.phase = 5
                            }
                        } else {
                            m.phase = 6
                        }
                    }
                    5 => {
                        let fenced = m.fence != ' ' && m.fence != '=';
                        self.edge(c.owner, m.row_box, usize::from(fenced), Some(m.height));
                        m.height += self.boxes[m.row_box].metrics.height + 1;
                        m.row += 1;
                        m.column = 0;
                        m.phase = if m.cell_at < m.cells_end { 3 } else { 6 }
                    }
                    _ => {
                        let fenced = m.fence != ' ' && m.fence != '=';
                        let height = m.height.saturating_sub(1).max(1);
                        let width = m.total_width
                            + 2 * m.columns.saturating_sub(1)
                            + 2 * usize::from(fenced);
                        let record = &mut self.boxes[c.owner];
                        record.index = IndexKind::Y;
                        record.metrics = Metrics {
                            width,
                            height,
                            baseline: height / 2,
                        };
                        if fenced {
                            record.primitives[0] = Some(Primitive::Fence {
                                ch: m.fence,
                                height,
                                source: node.origin,
                                x: 0,
                                y: 0,
                            });
                            record.primitives[1] = Some(Primitive::Fence {
                                ch: if m.fence == '(' { ')' } else { ']' },
                                height,
                                source: node.origin,
                                x: width - 1,
                                y: 0,
                            });
                        }
                        self.finish_node(c.owner);
                        return LayoutStep::Progress;
                    }
                }
                // Matrix owner was reserved before its rows. Its edge index is by Y.
                self.boxes[c.owner].index = IndexKind::Y;
                c.matrix = Some(m);
                self.current = Some(c);
                return LayoutStep::Progress;
            }
            match node.kind {
                Kind::Text {
                    ink,
                    width: Some(width),
                } => {
                    self.boxes[c.owner].metrics = Metrics {
                        width,
                        height: 1,
                        baseline: 0,
                    };
                    self.boxes[c.owner].primitives[0] = Some(Primitive::Text {
                        ink,
                        width,
                        source: node.source,
                        x: 0,
                        y: 0,
                    });
                    self.finish_node(c.owner)
                }
                Kind::Text { width: None, .. } => {
                    self.current = Some(c);
                    return LayoutStep::NeedShape;
                }
                Kind::Error => {
                    self.boxes[c.owner].metrics = Metrics {
                        width: 9,
                        height: 1,
                        baseline: 0,
                    };
                    self.boxes[c.owner].primitives[0] = Some(Primitive::Text {
                        ink: Ink::Static("[invalid]"),
                        width: 9,
                        source: node.source,
                        x: 0,
                        y: 0,
                    });
                    self.finish_node(c.owner)
                }
                Kind::CellEnd => self.finish_node(c.owner),
                Kind::Row => {
                    if let Some(link) = c.next {
                        let child = self.node_boxes[syntax.edges[link].child];
                        c.next = syntax.edges[link].next;
                        let metric = self.boxes[child].metrics;
                        self.edge(c.owner, child, c.row.width, None);
                        c.row.add(metric, c.row.width);
                        self.current = Some(c)
                    } else {
                        self.boxes[c.owner].metrics = c.row.metrics();
                        self.finish_node(c.owner)
                    }
                }
                Kind::Fraction => {
                    let a = self.child(syntax, c.node, 0);
                    let d = self.child(syntax, c.node, 1);
                    let am = self.boxes[a].metrics;
                    let dm = self.boxes[d].metrics;
                    let width = am.width.max(dm.width) + 2;
                    match c.phase {
                        0 => {
                            self.boxes[c.owner].metrics = Metrics {
                                width,
                                height: am.height + 1 + dm.height,
                                baseline: am.height,
                            };
                            self.boxes[c.owner].primitives[0] = Some(Primitive::Rule {
                                length: width,
                                source: node.origin,
                                x: 0,
                                y: am.height,
                            });
                            self.edge(c.owner, a, (width - am.width) / 2, Some(0));
                            c.phase = 1;
                            self.current = Some(c)
                        }
                        _ => {
                            self.edge(c.owner, d, (width - dm.width) / 2, Some(am.height + 1));
                            self.finish_node(c.owner)
                        }
                    }
                }
                Kind::Root => {
                    let child = self.child(syntax, c.node, 0);
                    let m = self.boxes[child].metrics;
                    self.boxes[c.owner].metrics = Metrics {
                        width: m.width + 1,
                        height: m.height + 1,
                        baseline: m.baseline + 1,
                    };
                    self.boxes[c.owner].primitives[0] = Some(Primitive::Rule {
                        length: m.width,
                        source: node.origin,
                        x: 1,
                        y: 0,
                    });
                    self.boxes[c.owner].primitives[1] = Some(Primitive::Text {
                        ink: Ink::Static("√"),
                        width: 1,
                        source: node.origin,
                        x: 0,
                        y: m.baseline + 1,
                    });
                    self.edge(c.owner, child, 1, Some(1));
                    self.finish_node(c.owner)
                }
                Kind::Script { upper, lower } => {
                    let base = self.child(syntax, c.node, 0);
                    let bm = self.boxes[base].metrics;
                    let up = upper.then(|| self.child(syntax, c.node, 1));
                    let low = lower.then(|| self.child(syntax, c.node, if upper { 2 } else { 1 }));
                    let um = up.map_or(Metrics::default(), |id| self.boxes[id].metrics);
                    let lm = low.map_or(Metrics::default(), |id| self.boxes[id].metrics);
                    let shift = if upper { um.height } else { 0 };
                    let baseline = bm.baseline + shift;
                    self.boxes[c.owner].metrics = Metrics {
                        width: bm.width + um.width.max(lm.width),
                        height: (shift + bm.height).max(if lower {
                            baseline + 1 + lm.height
                        } else {
                            0
                        }),
                        baseline,
                    };
                    let count = 1 + usize::from(upper) + usize::from(lower);
                    let (child, x, y) = if c.phase == 0 {
                        (base, 0, shift)
                    } else if c.phase == 1 && upper {
                        (up.unwrap(), bm.width, 0)
                    } else {
                        (low.unwrap(), bm.width, baseline + 1)
                    };
                    self.edge(c.owner, child, x, Some(y));
                    c.phase += 1;
                    if c.phase == count {
                        self.finish_node(c.owner)
                    } else {
                        self.current = Some(c)
                    }
                }
                Kind::Fence {
                    left,
                    right,
                    left_source,
                    right_source,
                } => {
                    let child = self.child(syntax, c.node, 0);
                    let m = self.boxes[child].metrics;
                    let l = usize::from(left != '.');
                    let r = usize::from(right != '.');
                    self.boxes[c.owner].metrics = Metrics {
                        width: m.width + l + r,
                        ..m
                    };
                    self.boxes[c.owner].primitives[0] = Some(Primitive::Fence {
                        ch: left,
                        height: m.height,
                        source: left_source,
                        x: 0,
                        y: 0,
                    });
                    self.boxes[c.owner].primitives[1] = Some(Primitive::Fence {
                        ch: right,
                        height: m.height,
                        source: right_source,
                        x: m.width + l,
                        y: 0,
                    });
                    self.edge(c.owner, child, l, Some(0));
                    self.finish_node(c.owner)
                }
                Kind::Matrix { .. } => unreachable!(),
            }
            return LayoutStep::Progress;
        }
        let Some(prefix) = syntax.prefix() else {
            return LayoutStep::Ready;
        };
        self.sync_prefix(syntax, prefix, b)
    }
    fn sync_prefix(&mut self, syntax: &Syntax, prefix: Prefix, b: &mut Budget) -> LayoutStep {
        if self.root_version == Some(prefix.version) && self.root_ghost == prefix.ghost {
            return LayoutStep::Ready;
        }
        if !pay(b, 0) {
            return LayoutStep::BudgetExhausted;
        }
        self.work.steps += 1;
        let root = self.root.unwrap();
        if let Some(last) = self.root_entries.last().copied() {
            let invalid = last.ghost || last.ast_edge.is_some_and(|id| !syntax.edges[id].attached);
            if invalid {
                let old = self.root_entries.pop().unwrap();
                let placed = self.placed[self.links[old.link].child];
                self.index.remove(&(root, placed.x, old.link));
                self.boxes[root].has_ink = self.root_entries.last().is_some_and(|r| r.has_ink);
                self.boxes[root].edges.pop(&mut self.links);
                self.boxes[root].metrics = self
                    .root_entries
                    .last()
                    .map_or(Row::default(), |r| r.summary)
                    .metrics();
                self.work.root_retracts += 1;
                self.work.index_operations += 1;
                return LayoutStep::Progress;
            }
        }
        let last = self.root_entries.last().and_then(|r| r.ast_edge);
        let next = last.map_or(prefix.list.first, |id| syntax.edges[id].next);
        let summary = self
            .root_entries
            .last()
            .map_or(Row::default(), |r| r.summary);
        if let Some(link) = next {
            let child = self.node_boxes[syntax.edges[link].child];
            let metric = self.boxes[child].metrics;
            let x = summary.width;
            let placed = self.edge(root, child, x, None);
            let mut after = summary;
            after.add(metric, x);
            self.root_entries.push(RootEntry {
                ast_edge: Some(link),
                link: placed,
                summary: after,
                has_ink: self.boxes[root].has_ink,
                ghost: false,
            });
            self.boxes[root].metrics = after.metrics();
            return LayoutStep::Progress;
        }
        if let Some(ghost) = prefix.ghost {
            let child = self.node_boxes[ghost];
            let m = self.boxes[child].metrics;
            let link = self.edge(root, child, summary.width, None);
            let mut after = summary;
            after.add(m, summary.width);
            self.root_entries.push(RootEntry {
                ast_edge: None,
                link,
                summary: after,
                has_ink: self.boxes[root].has_ink,
                ghost: true,
            });
            self.boxes[root].metrics = after.metrics();
        }
        if prefix.error {
            self.boxes[root].has_ink = true;
            self.boxes[root].primitives[0] = Some(Primitive::Text {
                ink: Ink::Static("[invalid]"),
                width: 9,
                source: Span::new(0, 0),
                x: 0,
                y: 0,
            });
        }
        self.root_version = Some(prefix.version);
        self.root_ghost = prefix.ghost;
        self.version += 1;
        LayoutStep::Ready
    }
    pub fn metrics(&self) -> Option<Metrics> {
        self.root.map(|id| self.boxes[id].metrics)
    }
    /// Refresh a joined tail glyph. A width change requires a paid layout
    /// rebuild; decoding/token/AST work is retained, never replayed.
    pub fn refresh_text(&mut self, syntax: &Syntax, node: usize, width_changed: bool) -> bool {
        if node >= self.node_boxes.len() {
            return false;
        }
        self.version += 1;
        if width_changed {
            return true;
        }
        let owner = self.node_boxes[node];
        let n = syntax.nodes[node];
        if let Some(Primitive::Text { source, .. }) = self.boxes[owner].primitives[0].as_mut() {
            self.source_index.remove(&(source.start, owner, 0));
            *source = n.source;
            self.source_index.insert((source.start, owner, 0), *source);
            self.work.index_operations += 2;
        }
        false
    }
    pub fn storage(&self) -> [StorageWork; 6] {
        [
            self.boxes.stats(),
            self.placed.stats(),
            self.links.stats(),
            self.node_boxes.stats(),
            self.cells.stats(),
            self.root_entries.stats(),
        ]
    }
    pub fn index_payload_bytes(&self) -> usize {
        self.index.len() * std::mem::size_of::<((usize, usize, usize), usize)>()
            + self.source_index.len() * std::mem::size_of::<((usize, usize, usize), Span)>()
            + self.parents.len() * std::mem::size_of::<(usize, (usize, usize))>()
            + (self.columns.len() + self.column_x.len())
                * std::mem::size_of::<((usize, usize), usize)>()
    }
    pub fn index_entries(&self) -> usize {
        self.index.len()
            + self.columns.len()
            + self.column_x.len()
            + self.source_index.len()
            + self.parents.len()
    }
    pub fn retire_one(&mut self) -> bool {
        if retire_map(&mut self.source_index) || retire_map(&mut self.parents) {
            return false;
        }
        if retire_map(&mut self.index)
            || retire_map(&mut self.columns)
            || retire_map(&mut self.column_x)
        {
            return false;
        }
        if self.root_entries.pop().is_some()
            || self.cells.pop().is_some()
            || self.links.pop().is_some()
            || self.placed.pop().is_some()
            || self.node_boxes.pop().is_some()
        {
            return false;
        }
        self.current = None;
        self.boxes.pop();
        self.boxes.is_empty()
    }
    pub fn next_link(&self, owner: usize, link: usize) -> Option<usize> {
        if self.boxes[owner].index == IndexKind::Small {
            return self.links[link].next;
        }
        let p = self.placed[self.links[link].child];
        let coordinate = if self.boxes[owner].index == IndexKind::Y {
            p.y.unwrap()
        } else {
            p.x
        };
        self.index
            .range((
                std::ops::Bound::Excluded((owner, coordinate, link)),
                std::ops::Bound::Included((owner, usize::MAX, usize::MAX)),
            ))
            .next()
            .map(|(_, &next)| next)
    }
    pub fn has_ink(&self) -> bool {
        self.root.is_some_and(|id| self.boxes[id].has_ink)
    }
    pub fn first_link(&self, owner: usize, coordinate: usize) -> Option<usize> {
        let record = self.boxes[owner];
        if record.index == IndexKind::Small {
            return record.edges.first;
        }
        if let Some((_, &link)) = self
            .index
            .range((owner, 0, 0)..(owner, coordinate, 0))
            .next_back()
        {
            let p = self.placed[self.links[link].child];
            let m = self.boxes[p.child].metrics;
            let end = if record.index == IndexKind::Y {
                p.y.unwrap() + m.height
            } else {
                p.x + m.width.saturating_add(1)
            };
            if end > coordinate {
                return Some(link);
            }
        }
        self.index
            .range((owner, coordinate, 0)..=(owner, usize::MAX, usize::MAX))
            .next()
            .map(|(_, link)| *link)
    }
}
