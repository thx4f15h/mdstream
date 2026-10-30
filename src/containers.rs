use crate::{
    ArenaStats, BlockKind, Budget, Completion, Span, SyntaxState, TaskMarker, Update, Work,
    WorkLimit,
};

/// Stable arena index, local to its Document, like BlockId.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContainerId(pub usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListMarker {
    /// One of `-`, `+`, `*`.
    Bullet(u8),
    /// `.` or `)` after one to nine decimal digits.
    Ordered(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContainerKind {
    Quote,
    List {
        marker: ListMarker,
    },
    Item {
        number: Option<u32>,
        task: Option<TaskMarker>,
    },
}

/// List spacing in the explicit-container dialect. Tight can become Loose
/// until the list is Sealed; Loose never reverts. This does not perform layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListTightness {
    Tight,
    Loose,
}

/// A current-revision snapshot. Growing ranges share a single parsed endpoint;
/// querying a parent does not traverse its descendants. Root depth is one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Container {
    pub parent: Option<ContainerId>,
    pub depth: usize,
    pub kind: ContainerKind,
    pub source: Span,
    pub state: SyntaxState,
    /// Present only on List nodes. Read the parent list instead of copying its
    /// spacing onto every Item or leaf block. See ListTightness for finality.
    pub list_tightness: Option<ListTightness>,
}

/// Borrowed ancestry walk, starting with the requested container. The borrow
/// prevents revision changes; every emitted node costs one query and one object.
pub struct ContainerAncestors<'a> {
    containers: &'a Containers,
    next: Option<ContainerId>,
}

impl ContainerAncestors<'_> {
    /// Returns None after the root, or BudgetExhausted without moving. Resume
    /// with a fresh budget; no path vector or unbounded walk is built.
    pub fn advance(
        &mut self,
        budget: &mut Budget,
    ) -> Result<Option<(ContainerId, Container)>, Completion> {
        let Some(id) = self.next else {
            return Ok(None);
        };
        if !budget.charge(WorkLimit {
            queries: 1,
            objects: 1,
            ..WorkLimit::default()
        }) {
            return Err(Completion::BudgetExhausted);
        }
        let node = self.containers.get(id).unwrap();
        self.next = node.parent;
        Ok(Some((id, node)))
    }
}

#[derive(Clone, Copy)]
enum Active {
    Quote(ContainerId),
    List {
        list: ContainerId,
        item: ContainerId,
        indent: usize,
        content_indent: usize,
        spacing: ItemSpacing,
    },
}

#[derive(Clone, Copy, Default)]
struct ItemSpacing {
    has_content: bool,
    pending_blank: bool,
}

impl Active {
    fn inner(self) -> ContainerId {
        match self {
            Self::Quote(id) => id,
            Self::List { item, .. } => item,
        }
    }
}

#[derive(Default)]
pub(crate) struct Containers {
    nodes: Vec<Container>,
    active: Vec<Active>,
    pub end: usize,
    allocations: u64,
}

pub(crate) struct ContainerOptions {
    pub complete: bool,
    pub root_markers: bool,
    pub paragraph: Option<ContainerId>,
    // Only an active nonblank inline sequence can retain unmatched frames.
    pub lazy_paragraph: Option<ContainerId>,
    pub table_header: Option<(ContainerId, usize)>,
    // Match this fence's existing path, but do not interpret its payload as
    // new containers. A missing prefix/sibling still ends that exact scope.
    pub fence: Option<ContainerId>,
}

pub(crate) struct ContainerLine {
    pub prefix: usize,
    pub kind: BlockKind,
    pub lazy: bool,
}

impl Containers {
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn starting_before(&self, offset: usize) -> usize {
        self.nodes
            .partition_point(|node| node.source.start < offset)
    }

    pub fn get(&self, id: ContainerId) -> Option<Container> {
        let mut node = *self.nodes.get(id.0)?;
        if node.state == SyntaxState::Growing {
            node.source.end = self.end;
        }
        Some(node)
    }

    pub fn ancestors(&self, id: ContainerId) -> Option<ContainerAncestors<'_>> {
        self.nodes.get(id.0)?;
        Some(ContainerAncestors {
            containers: self,
            next: Some(id),
        })
    }

    pub fn inner(&self) -> Option<ContainerId> {
        self.active.last().map(|a| a.inner())
    }

    pub fn stats(&self) -> ArenaStats {
        ArenaStats {
            nodes: self.nodes.len(),
            capacity_bytes: self.nodes.capacity() * std::mem::size_of::<Container>()
                + self.active.capacity() * std::mem::size_of::<Active>(),
            allocation_calls: self.allocations,
        }
    }

    fn open(
        &mut self,
        kind: ContainerKind,
        parent: Option<ContainerId>,
        start: usize,
        work: &mut Work,
    ) -> ContainerId {
        let id = ContainerId(self.nodes.len());
        let depth = parent.map_or(1, |p| self.nodes[p.0].depth + 1);
        self.allocations += u64::from(self.nodes.len() == self.nodes.capacity());
        self.nodes.push(Container {
            parent,
            depth,
            kind,
            source: Span::new(start, start),
            state: SyntaxState::Growing,
            list_tightness: matches!(kind, ContainerKind::List { .. })
                .then_some(ListTightness::Tight),
        });
        work.container_steps += 1;
        id
    }

    fn push(&mut self, frame: Active) {
        self.allocations += u64::from(self.active.len() == self.active.capacity());
        self.active.push(frame);
    }

    fn close(&mut self, id: ContainerId, end: usize, work: &mut Work, update: &mut Update) {
        self.nodes[id.0].source.end = end;
        self.nodes[id.0].state = SyntaxState::Sealed;
        update.closed_containers.push(id);
        work.container_steps += 1;
    }

    pub fn close_to(&mut self, keep: usize, end: usize, work: &mut Work, update: &mut Update) {
        while self.active.len() > keep {
            match self.active.pop().unwrap() {
                Active::Quote(id) => self.close(id, end, work, update),
                Active::List { list, item, .. } => {
                    self.close(item, end, work, update);
                    self.close(list, end, work, update);
                }
            }
        }
    }

    fn loosen(&mut self, list: ContainerId, work: &mut Work, update: &mut Update) {
        if self.nodes[list.0].list_tightness == Some(ListTightness::Tight) {
            self.nodes[list.0].list_tightness = Some(ListTightness::Loose);
            update.loosened_lists.push(list);
            work.list_steps += 1;
        }
    }

    /// Observe only an item's direct payload or a newly opened direct child.
    /// Blanks inside a quote/nested list never walk or update ancestor items.
    /// A marker-only first line is not a separator; trailing blanks alone do
    /// not loosen a list. A later sibling can confirm even an empty old item.
    pub fn observe_payload(
        &mut self,
        blank: bool,
        start: usize,
        work: &mut Work,
        update: &mut Update,
    ) {
        let Some(Active::List {
            list,
            item,
            spacing,
            ..
        }) = self.active.last_mut()
        else {
            return;
        };
        work.list_steps += 1;
        if blank {
            if self.nodes[item.0].source.start != start {
                spacing.pending_blank = true;
            }
        } else {
            let loosen = spacing.has_content && spacing.pending_blank;
            spacing.has_content = true;
            spacing.pending_blank = false;
            let list = *list;
            if loosen {
                self.loosen(list, work, update);
            }
        }
    }

    /// Each matched frame consumes an explicit prefix byte or positive item
    /// indentation. At the first mismatch, lazy text retains the entire suffix
    /// in O(1); otherwise each unmatched frame pops once.
    /// Space runs are scanned once even when several list levels consume them.
    pub fn line(
        &mut self,
        source: &str,
        span: Span,
        options: ContainerOptions,
        work: &mut Work,
        update: &mut Update,
    ) -> Option<ContainerLine> {
        let ContainerOptions {
            complete,
            root_markers,
            paragraph,
            lazy_paragraph,
            table_header,
            fence,
        } = options;
        let mut prefix = Prefix::new(source, span, complete);
        let mut keep = 0;
        let mut quotes = 0;
        let mut kind = BlockKind::Paragraph;
        let mut new_item = None;
        while keep < self.active.len() {
            work.container_steps += 1;
            match self.active[keep] {
                Active::Quote(_) => {
                    if !prefix.quote() {
                        break;
                    }
                    quotes += 1;
                    kind = BlockKind::Quote { depth: quotes };
                }
                Active::List {
                    list,
                    item,
                    indent,
                    content_indent,
                    spacing,
                } => {
                    let spaces = prefix.spaces();
                    if (keep > 0 || root_markers)
                        && spaces == indent
                        && let Some(marker) = prefix.marker()
                    {
                        if self.nodes[list.0].kind
                            != (ContainerKind::List {
                                marker: marker.kind,
                            })
                        {
                            break;
                        }
                        work.list_steps += 1;
                        if spacing.pending_blank {
                            self.loosen(list, work, update);
                        }
                        self.close_to(keep + 1, span.start, work, update);
                        self.close(item, span.start, work, update);
                        let item = self.open(
                            ContainerKind::Item {
                                number: marker.number,
                                task: None,
                            },
                            Some(list),
                            span.start,
                            work,
                        );
                        self.active[keep] = Active::List {
                            list,
                            item,
                            indent,
                            content_indent: marker.end - prefix.pos,
                            spacing: ItemSpacing::default(),
                        };
                        prefix.pos = marker.end;
                        kind = BlockKind::ListItem {
                            indent,
                            number: marker.number,
                            task: None,
                        };
                        new_item = Some(item);
                    } else if spaces >= content_indent {
                        prefix.pos += content_indent;
                    } else {
                        break;
                    }
                }
            }
            keep += 1;
        }
        if keep < self.active.len()
            && lazy_paragraph.is_some()
            && lazy_paragraph == self.inner()
            && prefix.lazy_text(source, span, work)
        {
            self.end = span.end;
            work.container_bytes += prefix.inspected;
            return Some(ContainerLine {
                prefix: prefix.pos,
                kind,
                lazy: true,
            });
        }
        self.close_to(keep, span.start, work, update);
        while !self.active.is_empty() || root_markers {
            if fence.is_some() && fence == self.inner() {
                break;
            }
            // Only a delimiter in the header's exact retained scope wins over
            // a new list marker (e.g. `> - | -`). Sibling items already changed
            // identity above. Inspect at most one bounded payload per line.
            if complete
                && let Some((scope, columns)) = table_header
                && Some(scope) == self.inner()
            {
                let payload = Span::new(prefix.pos, span.end);
                prefix.inspected += payload.len() as u64;
                if crate::table::delimiter(source, payload, columns).is_some() {
                    break;
                }
            }
            let indent = prefix.spaces();
            if indent > 3 {
                break;
            }
            // A continuing paragraph's underline wins over opening an empty
            // nested list, but cannot consume a sibling item's marker.
            if complete && paragraph.is_some() && paragraph == self.inner() {
                let text = &prefix.bytes[prefix.pos + indent..prefix.end];
                prefix.inspected += text.len() as u64;
                if crate::block::setext_level(text).is_some() {
                    break;
                }
            }
            if prefix.quote() {
                self.observe_payload(false, span.start, work, update);
                let id = self.open(ContainerKind::Quote, self.inner(), span.start, work);
                self.push(Active::Quote(id));
                quotes += 1;
                kind = BlockKind::Quote { depth: quotes };
                new_item = None;
            } else if let Some(marker) = prefix.marker() {
                self.observe_payload(false, span.start, work, update);
                let list = self.open(
                    ContainerKind::List {
                        marker: marker.kind,
                    },
                    self.inner(),
                    span.start,
                    work,
                );
                let item = self.open(
                    ContainerKind::Item {
                        number: marker.number,
                        task: None,
                    },
                    Some(list),
                    span.start,
                    work,
                );
                self.push(Active::List {
                    list,
                    item,
                    indent,
                    content_indent: marker.end - prefix.pos,
                    spacing: ItemSpacing::default(),
                });
                prefix.pos = marker.end;
                kind = BlockKind::ListItem {
                    indent,
                    number: marker.number,
                    task: None,
                };
                new_item = Some(item);
            } else {
                break;
            }
        }
        if let Some(item) = new_item
            && let Some(task) = prefix.task()
        {
            let ContainerKind::Item { task: stored, .. } = &mut self.nodes[item.0].kind else {
                unreachable!()
            };
            *stored = Some(task);
            let BlockKind::ListItem { task: stored, .. } = &mut kind else {
                unreachable!()
            };
            *stored = Some(task);
        }
        self.end = span.end;
        work.container_bytes += prefix.inspected;
        self.inner().map(|_| ContainerLine {
            prefix: prefix.pos,
            kind,
            lazy: false,
        })
    }
}

struct Marker {
    kind: ListMarker,
    number: Option<u32>,
    end: usize,
}

struct Prefix<'a> {
    bytes: &'a [u8],
    pos: usize,
    end: usize,
    complete: bool,
    spaces_end: usize,
    inspected: u64,
}

impl<'a> Prefix<'a> {
    fn new(source: &'a str, span: Span, complete: bool) -> Self {
        Self {
            bytes: source.as_bytes(),
            pos: span.start,
            end: crate::block::body_end(source, span),
            complete,
            spaces_end: span.start,
            inspected: 0,
        }
    }

    fn spaces(&mut self) -> usize {
        if self.spaces_end <= self.pos {
            self.spaces_end = self.pos;
            while self.spaces_end < self.end {
                self.inspected += 1;
                if self.bytes[self.spaces_end] != b' ' {
                    break;
                }
                self.spaces_end += 1;
            }
        }
        self.spaces_end - self.pos
    }

    fn lazy_text(&mut self, source: &str, span: Span, work: &mut Work) -> bool {
        // Only the bounded, as-yet-unconsumed payload is examined. No probe
        // mutates the path or consumes text that must remain visible.
        let payload = Span::new(self.pos, span.end);
        self.inspected += payload.len() as u64;
        if self.bytes[self.pos..self.end]
            .iter()
            .all(u8::is_ascii_whitespace)
        {
            return false;
        }
        let spaces = self.spaces();
        if spaces <= 3
            && (self.bytes.get(self.pos + spaces) == Some(&b'>') || self.marker().is_some())
        {
            return false;
        }
        work.classified_bytes += payload.len() as u64;
        match crate::block::classify(source, payload, self.complete, None, false) {
            crate::block::Line::Text | crate::block::Line::Indented { .. } => true,
            crate::block::Line::Definition { label, .. } => {
                // Match References::intern's label rejection without interning
                // a symbol or allocating a normalized candidate.
                self.inspected += label.len() as u64;
                label.len() > crate::references::MAX_LABEL_BYTES
                    || source[label.range()].chars().all(char::is_whitespace)
            }
            _ => false,
        }
    }

    fn quote(&mut self) -> bool {
        let spaces = self.spaces();
        let at = self.pos + spaces;
        self.inspected += u64::from(at < self.end);
        if spaces > 3 || at == self.end || self.bytes[at] != b'>' {
            return false;
        }
        self.pos = at + 1;
        if self.pos < self.end {
            self.inspected += 1;
            if self.bytes[self.pos] == b' ' {
                self.pos += 1;
            }
        }
        true
    }

    fn marker(&mut self) -> Option<Marker> {
        let mut at = self.pos + self.spaces();
        if at == self.end {
            return None;
        }
        self.inspected += 1;
        let (kind, number) = if matches!(self.bytes[at], b'-' | b'+' | b'*') {
            let kind = ListMarker::Bullet(self.bytes[at]);
            at += 1;
            (kind, None)
        } else {
            let start = at;
            let mut value = 0;
            while at < self.end && at - start < 9 {
                self.inspected += 1;
                if !self.bytes[at].is_ascii_digit() {
                    break;
                }
                value = value * 10 + u32::from(self.bytes[at] - b'0');
                at += 1;
            }
            if at == start || at == self.end {
                return None;
            }
            self.inspected += 1;
            if !matches!(self.bytes[at], b'.' | b')') {
                return None;
            }
            let kind = ListMarker::Ordered(self.bytes[at]);
            at += 1;
            (kind, Some(value))
        };
        if at == self.end {
            if !self.complete {
                return None;
            }
            // Empty items still require at least one indentation space for continuation.
            return Some(Marker {
                kind,
                number,
                end: at,
            });
        }
        self.inspected += 1;
        if !matches!(self.bytes[at], b' ' | b'\t') {
            return None;
        }
        while at < self.end && matches!(self.bytes[at], b' ' | b'\t') {
            self.inspected += 1;
            at += 1;
        }
        Some(Marker {
            kind,
            number,
            end: at,
        })
    }

    fn task(&mut self) -> Option<TaskMarker> {
        if self.end - self.pos < 3 {
            return None;
        }
        self.inspected += 4.min(self.end - self.pos) as u64;
        let at = self.pos;
        if self.bytes[at] != b'['
            || !matches!(self.bytes[at + 1], b' ' | b'x' | b'X')
            || self.bytes[at + 2] != b']'
            || (at + 3 == self.end && !self.complete)
            || (at + 3 < self.end && !matches!(self.bytes[at + 3], b' ' | b'\t'))
        {
            return None;
        }
        self.pos += 3;
        while self.pos < self.end && matches!(self.bytes[self.pos], b' ' | b'\t') {
            self.inspected += 1;
            self.pos += 1;
        }
        Some(TaskMarker {
            checked: self.bytes[at + 1] != b' ',
            source: Span::new(at, at + 3),
        })
    }
}
