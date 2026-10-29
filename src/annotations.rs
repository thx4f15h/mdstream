use crate::{LinkId, Span, Style};
use std::num::NonZeroUsize;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Mark {
    pub style: Style,
    pub link: Option<LinkId>,
    pub revision: u64,
    // Several link rewrites may occur in one append/revision.
    serial: u64,
}

impl Mark {
    fn merge(self, other: Self) -> Self {
        let link = if other.link.is_some() && other.serial >= self.serial {
            other.link
        } else {
            self.link
        };
        Self {
            style: self.style.union(other.style),
            link,
            revision: self.revision.max(other.revision),
            serial: self.serial.max(other.serial),
        }
    }
}

#[derive(Default)]
struct Node {
    children: [Option<NonZeroUsize>; 2],
    tag: Mark,
    newest: u64,
    hidden: bool,
}

impl Node {
    fn child(&self, side: usize) -> Option<usize> {
        self.children[side].map(|index| index.get() - 1)
    }
}

/// Dynamic binary segment tree over source bytes. Tags stay on their covering
/// node; updating a huge interval never visits its interior. No recursive AST.
pub(crate) struct Annotations {
    nodes: Vec<Node>,
    root: usize,
    capacity: usize,
    serial: u64,
    pub visits: u64,
    allocations: u64,
}

impl Default for Annotations {
    fn default() -> Self {
        Self {
            nodes: vec![Node::default()],
            root: 0,
            capacity: 1,
            serial: 0,
            visits: 0,
            allocations: 1,
        }
    }
}

impl Annotations {
    pub fn apply(&mut self, span: Span, style: Style, link: Option<LinkId>, revision: u64) {
        if span.is_empty() {
            return;
        }
        while self.capacity < span.end {
            let root = self.nodes.len();
            self.push_node(Node {
                children: [NonZeroUsize::new(self.root + 1), None],
                newest: self.nodes[self.root].newest,
                ..Node::default()
            });
            self.root = root;
            self.capacity *= 2;
        }
        self.serial += 1;
        let mark = Mark {
            style,
            link,
            revision,
            serial: if link.is_some() { self.serial } else { 0 },
        };
        self.update(self.root, 0, self.capacity, span, mark);
    }

    fn update(&mut self, id: usize, lo: usize, hi: usize, span: Span, mark: Mark) {
        self.visits += 1;
        self.nodes[id].newest = self.nodes[id].newest.max(mark.revision);
        if span.start <= lo && hi <= span.end {
            self.nodes[id].tag = self.nodes[id].tag.merge(mark);
            self.nodes[id].hidden |= mark.style.contains(Style::HIDDEN);
            return;
        }
        let mid = lo + (hi - lo) / 2;
        for (side, start, end) in [(0, lo, mid), (1, mid, hi)] {
            if span.start < end && start < span.end {
                let child = match self.nodes[id].child(side) {
                    Some(child) => child,
                    None => {
                        let child = self.nodes.len();
                        self.push_node(Node::default());
                        self.nodes[id].children[side] = NonZeroUsize::new(child + 1);
                        child
                    }
                };
                self.update(child, start, end, span, mark);
            }
        }
        self.nodes[id].hidden |= self.nodes[id]
            .children
            .iter()
            .all(|child| child.is_some_and(|child| self.nodes[child.get() - 1].hidden));
    }

    pub fn next_visible(&self, offset: usize, end: usize) -> Option<usize> {
        if offset >= end {
            return None;
        }
        if offset >= self.capacity {
            return Some(offset);
        }
        self.find_visible(self.root, 0, self.capacity, offset, end)
            .or_else(|| (self.capacity < end).then_some(self.capacity))
    }

    fn find_visible(
        &self,
        id: usize,
        lo: usize,
        hi: usize,
        offset: usize,
        end: usize,
    ) -> Option<usize> {
        if self.nodes[id].hidden || offset >= hi || end <= lo {
            return None;
        }
        if hi - lo == 1 {
            return Some(lo);
        }
        let mid = lo + (hi - lo) / 2;
        for (side, start, stop) in [(0, lo, mid), (1, mid, hi)] {
            if offset < stop && start < end {
                let found = match self.nodes[id].child(side) {
                    Some(child) => self.find_visible(child, start, stop, offset, end),
                    None => Some(start.max(offset)),
                };
                if found.is_some() {
                    return found;
                }
            }
        }
        None
    }

    #[cfg(test)]
    pub fn at(&self, offset: usize) -> Mark {
        if offset >= self.capacity {
            return Mark::default();
        }
        let (mut id, mut lo, mut hi) = (self.root, 0, self.capacity);
        let mut mark = Mark::default();
        loop {
            let node = &self.nodes[id];
            mark = mark.merge(node.tag);
            let mid = lo + (hi - lo) / 2;
            let side = usize::from(offset >= mid);
            match node.child(side) {
                Some(child) => {
                    id = child;
                    if side == 0 {
                        hi = mid;
                    } else {
                        lo = mid;
                    }
                }
                None => return mark,
            }
        }
    }

    pub fn newest(&self, span: Span) -> u64 {
        self.query_revision(self.root, 0, self.capacity, span)
    }

    fn query_revision(&self, id: usize, lo: usize, hi: usize, span: Span) -> u64 {
        if span.start >= hi || span.end <= lo {
            return 0;
        }
        let node = &self.nodes[id];
        if span.start <= lo && hi <= span.end {
            return node.newest;
        }
        let mid = lo + (hi - lo) / 2;
        let mut revision = node.tag.revision;
        for (side, start, end) in [(0, lo, mid), (1, mid, hi)] {
            if let Some(child) = node.child(side) {
                revision = revision.max(self.query_revision(child, start, end, span));
            }
        }
        revision
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

    pub fn segments(&self, span: Span) -> Segments<'_> {
        let mut stack = Vec::new();
        if span.end > self.capacity && !span.is_empty() {
            stack.push(Frame {
                node: None,
                span: Span::new(span.start.max(self.capacity), span.end),
                inherited: Mark::default(),
            });
        }
        if span.start < self.capacity && !span.is_empty() {
            stack.push(Frame {
                node: Some(self.root),
                span: Span::new(0, self.capacity),
                inherited: Mark::default(),
            });
        }
        Segments {
            tree: self,
            query: span,
            stack,
            visits: 0,
        }
    }
}

struct Frame {
    node: Option<usize>,
    span: Span,
    inherited: Mark,
}

/// Ordered constant-mark intervals. Each relevant tree node is visited once,
/// including absent children (uniform inherited tags), with O(log n) state.
/// Internal intervals may split UTF-8; the public run iterator coalesces them.
pub(crate) struct Segments<'a> {
    tree: &'a Annotations,
    query: Span,
    stack: Vec<Frame>,
    pub visits: u64,
}

impl Iterator for Segments<'_> {
    type Item = (Span, Mark);

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(frame) = self.stack.pop() {
            self.visits += 1;
            let mut mark = frame.inherited;
            if let Some(id) = frame.node {
                let node = &self.tree.nodes[id];
                mark = mark.merge(node.tag);
                if node.children != [None; 2] {
                    let mid = frame.span.start + frame.span.len() / 2;
                    for (side, start, end) in [(1, mid, frame.span.end), (0, frame.span.start, mid)]
                    {
                        if start < self.query.end && self.query.start < end {
                            self.stack.push(Frame {
                                node: node.child(side),
                                span: Span::new(start, end),
                                inherited: mark,
                            });
                        }
                    }
                    continue;
                }
            }
            return Some((
                Span::new(
                    frame.span.start.max(self.query.start),
                    frame.span.end.min(self.query.end),
                ),
                mark,
            ));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlapping_tags_and_revisions_match_a_naive_array() {
        let mut tree = Annotations::default();
        let mut model = vec![Mark::default(); 1024];
        let mut seed = 17_u64;
        let mut random = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed as usize
        };
        for serial in 1..3000 {
            let a = random() % 1024;
            let b = random() % 1024;
            let span = Span::new(a.min(b), a.max(b) + 1);
            let style = Style(1 << (random() % 7));
            let link = (random() % 3 == 0).then_some(LinkId(serial as usize));
            // Many writes share a document revision, including link rewrites.
            let revision = serial / 7 + 1;
            tree.apply(span, style, link, revision);
            for point in &mut model[span.range()] {
                point.style = point.style.union(style);
                if link.is_some() {
                    point.link = link;
                }
                point.revision = revision;
            }
            for _ in 0..8 {
                let at = random() % 1024;
                let actual = tree.at(at);
                assert_eq!(actual.style, model[at].style);
                assert_eq!(actual.link, model[at].link);
                assert_eq!(actual.revision, model[at].revision);
                let visible = (at..1024).find(|&i| !model[i].style.contains(Style::HIDDEN));
                assert_eq!(tree.next_visible(at, 1024), visible);
            }
            let lo = random() % 1024;
            let hi = random() % 1024;
            let query = Span::new(lo.min(hi), lo.max(hi) + 1);
            assert_eq!(
                tree.newest(query),
                model[query.range()]
                    .iter()
                    .map(|m| m.revision)
                    .max()
                    .unwrap()
            );
            if serial % 31 == 0 {
                let mut cursor = query.start;
                let mut segments = tree.segments(query);
                for (span, mark) in segments.by_ref() {
                    assert_eq!(span.start, cursor);
                    assert!(span.end > span.start);
                    for &expected in &model[span.range()] {
                        assert_eq!(mark.style, expected.style);
                        assert_eq!(mark.link, expected.link);
                        assert_eq!(mark.revision, expected.revision);
                    }
                    cursor = span.end;
                }
                assert_eq!(cursor, query.end);
                assert!(segments.visits <= 2 * query.len() as u64 + 32);
            }
        }
    }

    #[test]
    fn visibility_can_jump_past_capacity_and_respects_the_query_end() {
        let mut tree = Annotations::default();
        tree.apply(Span::new(0, 8), Style::HIDDEN, None, 1);
        assert_eq!(tree.next_visible(0, 12), Some(8));
        assert_eq!(tree.next_visible(0, 8), None);
        tree.apply(Span::new(9, 12), Style::HIDDEN, None, 2);
        assert_eq!(tree.next_visible(9, 11), None);
        assert_eq!(tree.next_visible(9, 13), Some(12));
    }

    #[test]
    fn segments_cover_unallocated_tail_and_empty_ranges() {
        let mut tree = Annotations::default();
        tree.apply(Span::new(0, 8), Style::STRONG, None, 1);
        let segments: Vec<_> = tree.segments(Span::new(5, 13)).collect();
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].0, Span::new(5, 8));
        assert_eq!(segments[0].1.style, Style::STRONG);
        assert_eq!(segments[1].0, Span::new(8, 13));
        assert_eq!(segments[1].1, Mark::default());
        assert_eq!(tree.segments(Span::new(13, 13)).count(), 0);
    }
}
