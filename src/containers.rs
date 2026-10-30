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
    // Sorted active-frame indices: locate the first missing quote without
    // walking the list ancestors retained across an unindented blank.
    quotes: Vec<usize>,
    // An omitted-prefix blank can separate any resumed list at/after this
    // depth. Resolve only its next direct scope, never write all ancestors.
    blank_from: Option<usize>,
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
    // Unconsumed columns of the tab at prefix - 1.
    pub virtual_spaces: u8,
    pub kind: BlockKind,
    pub lazy: bool,
    pub detached_blank: bool,
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

    /// Match the entire existing path before committing partial tabs.
    /// Non-fence probes accept ATX/fence/indented-code leaves, or (when allowed)
    /// nonblank ordinary continuations. Other lines retain the original
    /// sibling/leaf classifier without side effects.
    pub fn partial_prefix(
        &self,
        source: &str,
        span: Span,
        complete: Option<bool>,
        text: bool,
        work: &mut Work,
    ) -> Option<ContainerLine> {
        if complete.is_some() {
            // Only prefix spelling can contain a consumable tab. Skip the
            // speculative path walk for ordinary space/quote continuations,
            // and stop at the first payload byte rather than reading the body.
            let mut tab = false;
            for &byte in &source.as_bytes()[span.range()] {
                work.container_bytes += 1;
                match byte {
                    b'\t' => {
                        tab = true;
                        break;
                    }
                    b' ' | b'>' => (),
                    _ => break,
                }
            }
            if !tab {
                return None;
            }
        }
        let mut prefix = Prefix::new(source, span, complete.unwrap_or(false));
        prefix.split_tabs = true;
        let mut matched = true;
        let mut quotes = 0;
        for frame in &self.active {
            work.container_steps += 1;
            matched = match *frame {
                Active::Quote(_) => {
                    quotes += 1;
                    prefix.quote()
                }
                Active::List { content_indent, .. } => prefix.consume_indent(content_indent),
            };
            if !matched {
                break;
            }
        }
        if matched && let Some(complete) = complete {
            let payload = Span::new(prefix.pos, span.end);
            matched = prefix.partial;
            if matched {
                work.classified_bytes += payload.len() as u64;
                matched = match crate::block::classify_indented(
                    source,
                    payload,
                    complete,
                    None,
                    true,
                    prefix.virtual_spaces as u8,
                ) {
                    crate::block::Line::Heading { .. }
                    | crate::block::Line::FenceOpen { .. }
                    | crate::block::Line::Indented { .. } => true,
                    crate::block::Line::Text if text => {
                        prefix.spaces() <= 3
                            && prefix.spaces_end < prefix.end
                            && prefix.bytes[prefix.spaces_end] != b'>'
                            && prefix.marker().is_none()
                    }
                    _ => false,
                };
            }
        }
        work.container_bytes += prefix.inspected;
        matched.then_some(ContainerLine {
            prefix: prefix.pos,
            virtual_spaces: prefix.virtual_spaces as u8,
            kind: if quotes == 0 {
                BlockKind::Paragraph
            } else {
                BlockKind::Quote { depth: quotes }
            },
            lazy: false,
            detached_blank: false,
        })
    }

    /// Read-only probe of the bounded physical prefix. A long separator may
    /// retain exactly the header's path, never create siblings or use lazy text.
    pub fn delimiter_prefix(
        &self,
        source: &str,
        span: Span,
        scope: Option<ContainerId>,
        work: &mut Work,
    ) -> Option<usize> {
        if scope != self.inner() {
            return None;
        }
        let mut prefix = Prefix::new(source, span, false);
        let mut matched = true;
        for frame in &self.active {
            work.container_steps += 1;
            matched = match *frame {
                Active::Quote(_) => prefix.quote(),
                Active::List { content_indent, .. } => prefix.consume_indent(content_indent),
            };
            if !matched {
                break;
            }
        }
        work.container_bytes += prefix.inspected;
        matched.then_some(prefix.pos)
    }

    pub fn stats(&self) -> ArenaStats {
        ArenaStats {
            nodes: self.nodes.len(),
            capacity_bytes: self.nodes.capacity() * std::mem::size_of::<Container>()
                + self.active.capacity() * std::mem::size_of::<Active>()
                + self.quotes.capacity() * std::mem::size_of::<usize>(),
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
        if matches!(frame, Active::Quote(_)) {
            self.allocations += u64::from(self.quotes.len() == self.quotes.capacity());
            self.quotes.push(self.active.len());
        }
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
                Active::Quote(id) => {
                    self.quotes.pop();
                    self.close(id, end, work, update);
                }
                Active::List { list, item, .. } => {
                    self.close(item, end, work, update);
                    self.close(list, end, work, update);
                }
            }
        }
        if self
            .blank_from
            .is_some_and(|from| from >= self.active.len())
        {
            self.blank_from = None;
        }
    }

    fn resolve_blank(&mut self) {
        if let Some(from) = self.blank_from.take()
            && from < self.active.len()
            && let Some(Active::List { spacing, .. }) = self.active.last_mut()
        {
            spacing.pending_blank = true;
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
        if !blank {
            self.resolve_blank();
        }
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
    /// indentation (whole bytes at four-column tab stops). At the first mismatch,
    /// lazy text retains the entire suffix in O(1); a complete blank retains
    /// lists up to the first missing quote via its depth index. Other unmatched
    /// frames pop once. Whitespace runs are scanned at most twice across levels.
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
                        if spacing.pending_blank || self.blank_from.is_some_and(|from| from <= keep)
                        {
                            self.loosen(list, work, update);
                        }
                        self.blank_from = None;
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
                            content_indent: marker.column - prefix.column,
                            spacing: ItemSpacing::default(),
                        };
                        prefix.pos = marker.end;
                        prefix.column = marker.column;
                        kind = BlockKind::ListItem {
                            indent,
                            number: marker.number,
                            task: None,
                        };
                        new_item = Some(item);
                    } else if !prefix.consume_indent(content_indent) {
                        break;
                    }
                }
            }
            keep += 1;
        }
        // Blank classification is bounded by the physical prefix window. No
        // long-line retry and no missing quote is retained. The only scan below
        // is over the new line's remaining bytes, not the unmatched stack.
        let blank = complete && new_item.is_none() && {
            prefix.inspected += (prefix.end - prefix.pos) as u64;
            prefix.bytes[prefix.pos..prefix.end]
                .iter()
                .all(u8::is_ascii_whitespace)
        };
        if blank && matches!(self.active.get(keep), Some(Active::List { .. })) {
            let next_quote = self.quotes.partition_point(|&depth| {
                work.container_steps += 1;
                depth < keep
            });
            let retained = self
                .quotes
                .get(next_quote)
                .copied()
                .unwrap_or(self.active.len());
            self.close_to(retained, span.start, work, update);
            // The last explicitly matched Item also owns this separator (the
            // pre-existing exact-scope blank rule). Its omitted child lists
            // may now survive until the next line resolves the actual scope.
            let from = if keep > 0 && matches!(self.active[keep - 1], Active::List { .. }) {
                keep - 1
            } else {
                keep
            };
            self.blank_from = Some(self.blank_from.map_or(from, |old| old.min(from)));
            self.end = span.end;
            work.container_bytes += prefix.inspected;
            return Some(ContainerLine {
                prefix: prefix.pos,
                virtual_spaces: 0,
                kind,
                lazy: false,
                detached_blank: true,
            });
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
                virtual_spaces: 0,
                kind,
                lazy: true,
                detached_blank: false,
            });
        }
        self.close_to(keep, span.start, work, update);
        if !blank {
            self.resolve_blank();
        }
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
                let text = &prefix.bytes[prefix.spaces_end..prefix.end];
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
                    content_indent: marker.column - prefix.column,
                    spacing: ItemSpacing::default(),
                });
                prefix.pos = marker.end;
                prefix.column = marker.column;
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
            virtual_spaces: 0,
            kind,
            lazy: false,
            detached_blank: false,
        })
    }
}

struct Marker {
    kind: ListMarker,
    number: Option<u32>,
    end: usize,
    column: usize,
}

struct Prefix<'a> {
    bytes: &'a [u8],
    pos: usize,
    column: usize,
    split_tabs: bool,
    partial: bool,
    virtual_spaces: usize,
    end: usize,
    complete: bool,
    spaces_end: usize,
    spaces_column: usize,
    inspected: u64,
}

impl<'a> Prefix<'a> {
    fn new(source: &'a str, span: Span, complete: bool) -> Self {
        Self {
            bytes: source.as_bytes(),
            pos: span.start,
            column: 0,
            split_tabs: false,
            partial: false,
            virtual_spaces: 0,
            end: crate::block::body_end(source, span),
            complete,
            spaces_end: span.start,
            spaces_column: 0,
            inspected: 0,
        }
    }

    /// Remaining columns in the cached horizontal whitespace run.
    fn spaces(&mut self) -> usize {
        if self.spaces_end <= self.pos {
            self.spaces_end = self.pos;
            self.spaces_column = self.column + self.virtual_spaces;
            while self.spaces_end < self.end {
                self.inspected += 1;
                let byte = self.bytes[self.spaces_end];
                if !matches!(byte, b' ' | b'\t') {
                    break;
                }
                self.spaces_column += indent_width(byte, self.spaces_column);
                self.spaces_end += 1;
            }
        }
        self.spaces_column - self.column
    }

    /// Partial tabs are opt-in for an atomic existing-path probe. Otherwise a
    /// failed match leaves the entire prefix untouched for the original dialect.
    /// Cache the run endpoint across nested items; advancing visits each consumed
    /// byte once more, never rescans the remaining run for every ancestor.
    fn consume_indent(&mut self, columns: usize) -> bool {
        if self.spaces() < columns {
            return false;
        }
        let target = self.column + columns;
        if target == self.spaces_column {
            self.pos = self.spaces_end;
            self.column = target;
            self.virtual_spaces = 0;
            return true;
        }
        if self.virtual_spaces == 0
            && self.spaces_column - self.column == self.spaces_end - self.pos
        {
            self.pos += columns;
            self.column = target;
            return true;
        }
        let mut at = self.pos;
        let mut column = self.column + self.virtual_spaces;
        while column < target {
            self.inspected += 1;
            column += indent_width(self.bytes[at], column);
            at += 1;
        }
        if column != target && !self.split_tabs {
            return false;
        }
        self.pos = at;
        self.column = target;
        self.virtual_spaces = column - target;
        self.partial |= self.virtual_spaces > 0;
        true
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
            && (self.bytes.get(self.spaces_end) == Some(&b'>') || self.marker().is_some())
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
        let at = self.spaces_end;
        self.inspected += u64::from(at < self.end);
        if spaces > 3 || at == self.end || self.bytes[at] != b'>' {
            return false;
        }
        self.pos = at + 1;
        self.column = self.spaces_column + 1;
        self.virtual_spaces = 0;
        if self.pos < self.end {
            self.inspected += 1;
            if self.bytes[self.pos] == b' '
                || (self.bytes[self.pos] == b'\t'
                    && (self.split_tabs || indent_width(b'\t', self.column) == 1))
            {
                self.virtual_spaces = indent_width(self.bytes[self.pos], self.column) - 1;
                self.partial |= self.virtual_spaces > 0;
                self.pos += 1;
                self.column += 1;
            }
        }
        true
    }

    fn marker(&mut self) -> Option<Marker> {
        self.spaces();
        let mut at = self.spaces_end;
        let start = at;
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
        let mut column = self.spaces_column + at - start;
        if at == self.end {
            if !self.complete {
                return None;
            }
            // Empty items still require at least one indentation space for continuation.
            return Some(Marker {
                kind,
                number,
                end: at,
                column,
            });
        }
        self.inspected += 1;
        if !matches!(self.bytes[at], b' ' | b'\t') {
            return None;
        }
        while at < self.end && matches!(self.bytes[at], b' ' | b'\t') {
            self.inspected += 1;
            column += indent_width(self.bytes[at], column);
            at += 1;
        }
        Some(Marker {
            kind,
            number,
            end: at,
            column,
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
        self.column += 3;
        while self.pos < self.end && matches!(self.bytes[self.pos], b' ' | b'\t') {
            self.inspected += 1;
            self.column += indent_width(self.bytes[self.pos], self.column);
            self.pos += 1;
        }
        Some(TaskMarker {
            checked: self.bytes[at + 1] != b' ',
            source: Span::new(at, at + 3),
        })
    }
}

fn indent_width(byte: u8, column: usize) -> usize {
    if byte == b'\t' { 4 - column % 4 } else { 1 }
}
