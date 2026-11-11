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
    // The last descendant ends in a separator. Deliver it only when that
    // descendant closes; intervening content consumes it without ancestor work.
    trailing_blank: bool,
    pub end: usize,
    allocations: u64,
}

pub(crate) struct ContainerOptions {
    pub partial_container: bool,
    pub complete: bool,
    pub root_markers: bool,
    pub root_paragraph: bool,
    pub paragraph: Option<ContainerId>,
    // Only an active nonblank inline sequence can retain unmatched frames.
    pub lazy_paragraph: Option<ContainerId>,
    pub table_header: Option<(ContainerId, usize)>,
    // Match this fence's existing path, but do not interpret its payload as
    // new containers. Missing quotes, nonblank missing prefixes and siblings end it.
    pub fence: Option<ContainerId>,
    // Only code fences accept omitted list indentation on complete blank lines.
    pub code_fence: bool,
}

pub(crate) struct ContainerLine {
    // A read-only partial-prefix probe found a new container, sibling list,
    // or ended empty item. Route it through line() for structural mutation.
    pub new_container: bool,
    // Prefix already includes four columns of container code indentation.
    pub indented: bool,
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

    // Empty items cannot acquire their first content after a blank. Keep the
    // frame until the next classified line so same-marker siblings can still
    // reuse the List and resolve its spacing. Both blank representations are
    // already cached; no retained ancestor or historical leaf is inspected.
    fn empty_after_blank(&self, depth: usize) -> bool {
        let Active::List { item, spacing, .. } = self.active[depth] else {
            return false;
        };
        !spacing.has_content
            && (spacing.pending_blank || self.blank_from.is_some_and(|from| from <= depth))
            && matches!(
                self.nodes[item.0].kind,
                ContainerKind::Item { task: None, .. }
            )
    }

    /// Match the entire existing path for leaves, or the ancestors of a
    /// sibling marker, before committing partial tabs. Empty different markers
    /// still obey the retained scope's paragraph-interruption rule in line().
    /// Non-fence probes also accept Item code with whole-byte path boundaries.
    /// Partial paths accept headings/fence/rule/indented-code leaves, complete
    /// blanks and ordinary text, including new paragraphs. List/quote candidates
    /// and ended empty items are handed to the structural classifier; other
    /// lines retain the original sibling/leaf classifier without side effects.
    /// Active prose may omit a path suffix after a tab was partially consumed;
    /// the existing lazy predicate decides eligibility without visiting that suffix.
    pub fn partial_prefix(
        &self,
        source: &str,
        span: Span,
        complete: Option<bool>,
        text: bool,
        paragraph: bool,
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
        let mut new_container = false;
        for (depth, frame) in self.active.iter().enumerate() {
            work.container_steps += 1;
            matched = match *frame {
                Active::Quote(_) => {
                    let matched = prefix.quote();
                    quotes += usize::from(matched);
                    matched
                }
                Active::List { content_indent, .. } => {
                    // A sibling needs only its ancestors, not the old Item's
                    // content indentation. Defer all mutations and priorities
                    // to line(); fence probes still require the entire path.
                    if complete.is_some()
                        && prefix.partial
                        && prefix.spaces() <= 3
                        && prefix.spaces() < content_indent
                        && prefix.marker().is_some()
                    {
                        new_container = true;
                        break;
                    }
                    if self.empty_after_blank(depth) {
                        // A split ancestor must reach line() atomically even
                        // when the next payload is ordinary text or code.
                        new_container = prefix.partial;
                        matched = new_container;
                        break;
                    }
                    prefix.consume_indent(content_indent)
                }
            };
            if !matched {
                break;
            }
        }
        // Active prose or a pending header may retain an omitted path suffix.
        // The header falls back to prose because lazy lines cannot confirm a
        // table. Sibling markers already defer to line() above. Commit the
        // consumed tab columns without walking the missing frames
        // or treating this lazy line as a new table/leaf candidate. A failed
        // lazy predicate leaves the original structural classifier untouched.
        let lazy = !matched
            && complete.is_some()
            && (text || paragraph)
            && prefix.partial
            && prefix.lazy_text(source, span, false, work);
        if lazy {
            matched = true;
        }
        let mut indented = false;
        if matched
            && !lazy
            && !new_container
            && let Some(complete) = complete
        {
            // A fully matched path without active prose may consume list code
            // indentation even when its boundary did not split a tab. Failed
            // probes leave the original leaf/lazy classifier untouched.
            if !text && matches!(self.active.last(), Some(Active::List { .. })) {
                let mut probe = prefix;
                probe.inspected = 0;
                indented = probe.consume_indent(4) && (!complete || probe.spaces_end < probe.end);
                prefix.inspected += probe.inspected;
                if indented {
                    probe.inspected = prefix.inspected;
                    prefix = probe;
                }
            }
            let payload = Span::new(prefix.pos, span.end);
            matched = prefix.partial || indented;
            if matched && !indented {
                work.classified_bytes += payload.len() as u64;
                matched = match crate::block::classify_indented(
                    source,
                    payload,
                    complete,
                    None,
                    paragraph,
                    prefix.virtual_spaces as u8,
                    ((prefix.column + prefix.virtual_spaces) % 4) as u8,
                ) {
                    crate::block::Line::Blank
                    | crate::block::Line::Heading { .. }
                    | crate::block::Line::Definition { .. }
                    | crate::block::Line::DefinitionStart { .. }
                    | crate::block::Line::FootnoteDefinition { .. }
                    | crate::block::Line::Setext(_)
                    | crate::block::Line::FenceOpen { .. }
                    | crate::block::Line::Indented { .. }
                    | crate::block::Line::Rule => true,
                    crate::block::Line::Text
                        if prefix.spaces() <= 3 && prefix.spaces_end < prefix.end =>
                    {
                        new_container =
                            prefix.bytes[prefix.spaces_end] == b'>' || prefix.marker().is_some();
                        // A pending table header also admits ordinary text:
                        // its short delimiter can confirm in the same scope,
                        // or fall back with the original virtual-space map.
                        true
                    }
                    _ => false,
                };
            }
        }
        work.container_bytes += prefix.inspected;
        matched.then_some(ContainerLine {
            new_container,
            indented,
            prefix: prefix.pos,
            virtual_spaces: prefix.virtual_spaces as u8,
            kind: if quotes == 0 {
                BlockKind::Paragraph
            } else {
                BlockKind::Quote { depth: quotes }
            },
            lazy,
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
        if let Ok(prefix) = self.delimiter_cursor(source, span, scope, false, work) {
            return Some(prefix.pos);
        }
        // Extend only paths rejected by the original byte-aligned probe.
        // Match atomically; four residual columns still belong to code. Both
        // walks inspect this bounded physical prefix, never retained ancestors
        // without input or the growing separator tail.
        let mut prefix = self
            .delimiter_cursor(source, span, scope, true, work)
            .ok()?;
        let inspected = prefix.inspected;
        let accepted = prefix.partial && prefix.spaces() <= 3;
        work.container_bytes += prefix.inspected - inspected;
        accepted.then_some(prefix.pos)
    }

    /// Match the same explicit path for a reference target continuation. Unlike
    /// table delimiters, its payload may have four or more indentation columns.
    /// New containers and leaves that interrupt the candidate's paragraph win.
    pub fn definition_prefix(
        &self,
        source: &str,
        span: Span,
        scope: Option<ContainerId>,
        work: &mut Work,
    ) -> Option<usize> {
        let prefix = self
            .delimiter_cursor(source, span, scope, false, work)
            .or_else(|_| self.delimiter_cursor(source, span, scope, true, work))
            .ok()?;
        // Structural classification admits a complete partial-tab path even
        // when the original byte-aligned spelling also matched. Keep that
        // spelling for the target, but do not let it conceal a new structure.
        let mut structure = if prefix.split_tabs {
            prefix
        } else {
            match self.delimiter_cursor(source, span, scope, true, work) {
                Ok(structure) => structure,
                Err((mut missing, Some(Active::List { list, .. }))) if missing.partial => {
                    // A partial ancestor can expose a sibling before the old
                    // Item's indentation. That changes identity even for an
                    // ordered start other than 1. Same-marker empty siblings
                    // also replace the Item; other empty markers stay lazy.
                    let inspected = missing.inspected;
                    missing.complete = true;
                    let sibling = missing.spaces() <= 3
                        && missing.marker().is_some_and(|marker| {
                            marker.end < missing.end
                                || self.nodes[list.0].kind
                                    == (ContainerKind::List {
                                        marker: marker.kind,
                                    })
                        });
                    work.container_bytes += missing.inspected - inspected;
                    if sibling {
                        return None;
                    }
                    prefix
                }
                Err(_) => prefix,
            }
        };
        let inspected = structure.inspected;
        // The caller only confirms complete bounded physical lines. Matching
        // the old path alone does not exclude a new nested Quote/List. Reuse
        // the same marker and paragraph interruption rules as line(), without
        // mutating containers or classifying any old candidate text.
        structure.complete = true;
        let new_container = structure.spaces() <= 3
            && (structure.quote()
                || structure
                    .marker()
                    .is_some_and(|marker| marker.interrupts_paragraph(true, structure.end)));
        work.container_bytes += structure.inspected - inspected;
        if new_container {
            return None;
        }
        // Use the same partial-tab boundary for leaf priority as partial_prefix.
        // The byte-aligned target cursor can overstate indentation and conceal
        // a heading, Setext underline, rule or fence within three columns.
        let payload = Span::new(structure.pos, span.end);
        work.classified_bytes += payload.len() as u64;
        matches!(
            crate::block::classify_indented(
                source,
                payload,
                true,
                None,
                true,
                structure.virtual_spaces as u8,
                ((structure.column + structure.virtual_spaces) % 4) as u8,
            ),
            crate::block::Line::Text | crate::block::Line::Indented { .. }
        )
        .then_some(prefix.pos)
    }

    /// A footnote path and its four-column continuation indent may split tabs.
    /// Return the final byte endpoint and its residual columns; failed partial
    /// probes preserve the original path/blank spelling.
    pub fn footnote_prefix(
        &self,
        source: &str,
        span: Span,
        scope: Option<ContainerId>,
        work: &mut Work,
    ) -> Option<(usize, Option<(usize, u8)>)> {
        for split_tabs in [true, false] {
            let mut prefix = match self.delimiter_cursor(source, span, scope, split_tabs, work) {
                Ok(prefix) => prefix,
                Err((prefix, _)) if prefix.partial => continue,
                Err(_) => return None,
            };
            let start = prefix.pos;
            let inspected = prefix.inspected;
            let payload = prefix
                .consume_indent(4)
                .then_some((prefix.pos, prefix.virtual_spaces as u8));
            work.container_bytes += prefix.inspected - inspected;
            if payload.is_some() || !prefix.partial {
                return Some((start, payload));
            }
        }
        None
    }

    // On a path mismatch, retain the cursor and first missing frame. A split
    // tab can differ from the old spelling and expose a sibling list marker.
    fn delimiter_cursor<'a>(
        &self,
        source: &'a str,
        span: Span,
        scope: Option<ContainerId>,
        split_tabs: bool,
        work: &mut Work,
    ) -> Result<Prefix<'a>, (Prefix<'a>, Option<Active>)> {
        let mut prefix = Prefix::new(source, span, false);
        if scope != self.inner() {
            return Err((prefix, None));
        }
        prefix.split_tabs = split_tabs;
        let mut missing = None;
        for frame in &self.active {
            work.container_steps += 1;
            let matched = match *frame {
                Active::Quote(_) => prefix.quote(),
                Active::List { content_indent, .. } => prefix.consume_indent(content_indent),
            };
            if !matched {
                missing = Some(*frame);
                break;
            }
        }
        work.container_bytes += prefix.inspected;
        if missing.is_some() {
            Err((prefix, missing))
        } else {
            Ok(prefix)
        }
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
        let popped = self.active.len() > keep;
        while self.active.len() > keep {
            match self.active.pop().unwrap() {
                Active::Quote(id) => {
                    self.quotes.pop();
                    self.close(id, end, work, update);
                    // A quote's internal blank does not separate blocks or
                    // sibling items outside that quote, even if it ends a list.
                    self.trailing_blank = false;
                }
                Active::List { list, item, .. } => {
                    self.close(item, end, work, update);
                    self.close(list, end, work, update);
                }
            }
        }
        if self.active.is_empty() {
            self.trailing_blank = false;
        } else if popped
            && self.trailing_blank
            && let Some(Active::List { spacing, .. }) = self.active.last_mut()
        {
            // Trailing list descendants deliver one pending separator, unless
            // a quote boundary consumed it. Closure alone never confirms Loose.
            spacing.pending_blank = true;
            work.list_steps += 1;
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
    /// Blanks inside a quote/nested list never walk or update ancestor items;
    /// only a trailing list separator survives closure to a resumed parent
    /// scope; closing a quote consumes its internal separator.
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
        self.trailing_blank = blank
            && self
                .inner()
                .is_some_and(|id| self.nodes[id.0].source.start != start);
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
            partial_container,
            complete,
            root_markers,
            root_paragraph,
            paragraph,
            lazy_paragraph,
            table_header,
            fence,
            code_fence,
        } = options;
        let mut prefix = Prefix::new(source, span, complete);
        // A single bounded reverse scan makes rule priority O(1) per frame,
        // even for long chains of failed `* ` list prefixes.
        let rule_suffix = if complete { prefix.rule_suffix() } else { None };
        let mut keep = 0;
        let mut quotes = 0;
        let mut kind = BlockKind::Paragraph;
        let mut new_item = None;
        let mut indented = false;
        let mut sibling_marker = false;
        // Commit a new list/quote, sibling marker, or ended empty item
        // after matching its ancestors. Failed probes leave the original path intact.
        // Each matched frame consumes positive columns in this bounded prefix;
        // the whitespace endpoint stays cached across all list ancestors.
        if partial_container {
            let mut probe = prefix;
            probe.split_tabs = true;
            probe.inspected = 0;
            let mut matched = true;
            let mut sibling = false;
            let mut retained = 0;
            let mut retained_quotes = 0;
            let mut ended_empty = false;
            for frame in &self.active {
                work.container_steps += 1;
                matched = match *frame {
                    Active::Quote(_) => {
                        retained_quotes += 1;
                        probe.quote()
                    }
                    Active::List { content_indent, .. } => {
                        if probe.partial
                            && probe.spaces() <= 3
                            && probe.spaces() < content_indent
                            && probe.marker().is_some()
                        {
                            sibling = true;
                            break;
                        }
                        if self.empty_after_blank(retained) {
                            ended_empty = true;
                            break;
                        }
                        probe.consume_indent(content_indent)
                    }
                };
                if !matched {
                    break;
                }
                retained += 1;
            }
            let mut accept = matched
                && probe.partial
                && (ended_empty
                    || (probe.spaces() <= 3
                        && !probe.is_rule(rule_suffix)
                        && (probe.bytes.get(probe.spaces_end) == Some(&b'>')
                            || probe.marker().is_some())));
            if accept
                && complete
                && !sibling
                && !ended_empty
                && paragraph.is_some()
                && paragraph == self.inner()
            {
                let text = &probe.bytes[probe.spaces_end..probe.end];
                probe.inspected += text.len() as u64;
                accept = crate::block::setext_level(text).is_none();
            }
            // Keep an exact-scope table delimiter on this matched path.
            // The new-container loop below gives it priority over a list
            // marker, just as for whole-byte indentation boundaries.
            work.container_bytes += probe.inspected;
            if accept {
                probe.inspected = prefix.inspected;
                probe.split_tabs = false;
                prefix = probe;
                keep = retained;
                quotes = retained_quotes;
                // A lazy empty marker can retain this prefix without opening
                // a new node; preserve the explicitly matched quote kind too.
                if quotes > 0 {
                    kind = BlockKind::Quote { depth: quotes };
                }
            }
        }
        while keep < self.active.len() {
            work.container_steps += 1;
            match self.active[keep] {
                Active::Quote(_) => {
                    let code = if keep + 1 == self.active.len() && fence.is_none() {
                        prefix.quote_code()
                    } else {
                        None
                    };
                    if let Some(code) = code {
                        prefix = code;
                        indented = true;
                    } else if !prefix.quote() {
                        break;
                    }
                    quotes += 1;
                    kind = BlockKind::Quote { depth: quotes };
                }
                Active::List {
                    list,
                    item,
                    content_indent,
                    ..
                } => {
                    let spaces = prefix.spaces();
                    // Below the current Item's body indentation, a legal marker
                    // starts a sibling even when its indentation changed. At or
                    // beyond the body boundary it belongs inside the old Item.
                    if (keep > 0 || root_markers)
                        && spaces <= 3
                        && spaces < content_indent
                        && !prefix.is_rule(rule_suffix)
                        && let Some(marker) = prefix.marker()
                    {
                        if self.nodes[list.0].kind
                            != (ContainerKind::List {
                                marker: marker.kind,
                            })
                        {
                            sibling_marker = true;
                            break;
                        }
                        // A child's tail must reach the old Item before the
                        // sibling decides spacing and replaces that Item.
                        self.close_to(keep + 1, span.start, work, update);
                        let Active::List { spacing, .. } = self.active[keep] else {
                            unreachable!()
                        };
                        work.list_steps += 1;
                        if spacing.pending_blank || self.blank_from.is_some_and(|from| from <= keep)
                        {
                            self.loosen(list, work, update);
                        }
                        self.blank_from = None;
                        self.trailing_blank = false;
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
                            content_indent: marker.column - prefix.column,
                            spacing: ItemSpacing::default(),
                        };
                        prefix.pos = marker.end;
                        prefix.column = marker.column;
                        prefix.virtual_spaces = 0;
                        if let Some(spaces) = marker.code_spaces {
                            prefix.column += 4;
                            prefix.virtual_spaces = usize::from(spaces);
                            indented = true;
                        }
                        kind = BlockKind::ListItem {
                            indent: spaces,
                            number: marker.number,
                            task: None,
                        };
                        new_item = Some(item);
                    } else if self.empty_after_blank(keep) || !prefix.consume_indent(content_indent)
                    {
                        break;
                    }
                }
            }
            keep += 1;
            if indented {
                break;
            }
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
            if code_fence && fence == self.inner() {
                prefix.spaces();
                if prefix.spaces_end == prefix.end {
                    // All quotes matched, and only list indentation is missing.
                    // Its insufficient whitespace is structural; keep the line
                    // ending as literal code, without recording a list separator.
                    self.end = span.end;
                    work.container_bytes += prefix.inspected;
                    return Some(ContainerLine {
                        new_container: false,
                        indented: false,
                        prefix: prefix.end,
                        virtual_spaces: 0,
                        kind,
                        lazy: false,
                        detached_blank: false,
                    });
                }
            }
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
                new_container: false,
                indented: false,
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
            && prefix.lazy_text(source, span, sibling_marker, work)
        {
            self.end = span.end;
            work.container_bytes += prefix.inspected;
            return Some(ContainerLine {
                new_container: false,
                indented: false,
                prefix: prefix.pos,
                virtual_spaces: prefix.virtual_spaces as u8,
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
            if indented {
                break;
            }
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
            // Rules win over new lists; the leaf classifier still gives an
            // exact-scope paragraph's Setext underline priority over a rule.
            if prefix.is_rule(rule_suffix) {
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
            let code = prefix.quote_code();
            let leaf = if code.is_none() {
                prefix.quote_leaf(source, span, rule_suffix, work)
            } else {
                None
            };
            if code.is_some() || leaf.is_some() || prefix.quote() {
                if let Some(code) = code {
                    prefix = code;
                    indented = true;
                } else if let Some(leaf) = leaf {
                    prefix = leaf;
                }
                self.observe_payload(false, span.start, work, update);
                let id = self.open(ContainerKind::Quote, self.inner(), span.start, work);
                self.push(Active::Quote(id));
                quotes += 1;
                kind = BlockKind::Quote { depth: quotes };
                new_item = None;
            } else if let Some(marker) = prefix.marker() {
                // Empty markers and ordered starts other than 1 cannot
                // interrupt prose in this exact retained scope. Only complete
                // bounded lines prove emptiness; the parsed number is already
                // final even for a long line. Siblings replaced their Item
                // above, and new containers have a different identity.
                if !marker.interrupts_paragraph(complete, prefix.end)
                    && ((paragraph.is_some() && paragraph == self.inner())
                        || (root_paragraph && self.inner().is_none()))
                {
                    break;
                }
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
                    content_indent: marker.column - prefix.column,
                    spacing: ItemSpacing::default(),
                });
                prefix.pos = marker.end;
                prefix.column = marker.column;
                prefix.virtual_spaces = 0;
                if let Some(spaces) = marker.code_spaces {
                    prefix.column += 4;
                    prefix.virtual_spaces = usize::from(spaces);
                    indented = true;
                }
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
            && !indented
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
            new_container: false,
            indented,
            prefix: prefix.pos,
            virtual_spaces: prefix.virtual_spaces as u8,
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
    code_spaces: Option<u8>,
}

impl Marker {
    fn interrupts_paragraph(&self, complete: bool, end: usize) -> bool {
        (!complete || self.end < end) && self.number.is_none_or(|number| number == 1)
    }
}

#[derive(Clone, Copy)]
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

    /// First and third-from-last marker of the maximal homogeneous rule
    /// suffix. Any marker between these endpoints starts a valid rule; other
    /// prefixes cannot. Keep only offsets, not a suffix copy or ancestor state.
    fn rule_suffix(&mut self) -> Option<(usize, usize)> {
        let mut marker = None;
        let mut first = self.end;
        let mut third = None;
        let mut count = 0;
        for at in (self.pos..self.end).rev() {
            self.inspected += 1;
            let byte = self.bytes[at];
            if matches!(byte, b' ' | b'\t') {
                continue;
            }
            if !matches!(byte, b'*' | b'-' | b'_') || marker.is_some_and(|old| old != byte) {
                break;
            }
            marker = Some(byte);
            first = at;
            count += 1;
            if count == 3 {
                third = Some(at);
            }
        }
        Some((first, third?))
    }

    fn is_rule(&mut self, suffix: Option<(usize, usize)>) -> bool {
        let Some((first, third)) = suffix else {
            return false;
        };
        // Use the same physical columns (including partial tab residue) as
        // leaf classification, so a rule cannot become a nested list first.
        self.spaces() <= 3 && (first..=third).contains(&self.spaces_end)
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

    fn lazy_text(
        &mut self,
        source: &str,
        span: Span,
        sibling_marker: bool,
        work: &mut Work,
    ) -> bool {
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
            && (self.bytes.get(self.spaces_end) == Some(&b'>')
                || self.marker().is_some_and(|marker| {
                    // Like explicit paragraph continuations, lazy text is
                    // interrupted only by an ordered start of 1. A different
                    // sibling marker still replaces the old list; matching
                    // siblings were already handled on the old path.
                    (!self.complete || marker.end < self.end)
                        && (sibling_marker || marker.number.is_none_or(|number| number == 1))
                }))
        {
            return false;
        }
        work.classified_bytes += payload.len() as u64;
        match crate::block::classify(source, payload, self.complete, None, false) {
            // Callers have an existing nonempty paragraph in the retained
            // scope. A definition candidate cannot interrupt it just because
            // some ancestor prefixes are omitted; lazy lines become Text.
            // Keep the classifier's nonparagraph mode for Setext/rule priority.
            crate::block::Line::Text
            | crate::block::Line::Indented { .. }
            | crate::block::Line::Definition { .. } => true,
            crate::block::Line::DefinitionStart { .. } => true,
            _ => false,
        }
    }

    /// Atomically accept an explicit quote followed by tab-containing code
    /// indentation. Only inspect this bounded prefix; failed probes preserve
    /// the old quote/leaf spelling. Carry the final tab's unused columns.
    fn quote_code(&mut self) -> Option<Self> {
        let mut probe = *self;
        probe.inspected = 0;
        probe.split_tabs = true;
        let start = probe.pos;
        let accepted = probe.quote()
            && probe.consume_indent(4)
            && (!probe.complete || probe.spaces_end < probe.end);
        let has_tab = if accepted {
            probe.inspected += (probe.pos - start) as u64;
            probe.bytes[start..probe.pos].contains(&b'\t')
        } else {
            false
        };
        self.inspected += probe.inspected;
        probe.inspected = self.inspected;
        (accepted && has_tab).then_some(probe)
    }

    /// A new quote may split its tab separator for a confirmed code/math fence,
    /// ATX, rule, valid definition, nested quote/list, complete blank or text.
    /// Text retains unused separator columns through the existing display map;
    /// Incomplete blanks and candidates outside the window keep the old spelling.
    fn quote_leaf(
        &mut self,
        source: &str,
        span: Span,
        rule_suffix: Option<(usize, usize)>,
        work: &mut Work,
    ) -> Option<Self> {
        if self.spaces() > 3 {
            return None;
        }
        let at = self.spaces_end;
        self.inspected += 2.min(self.end - at) as u64;
        if self.bytes.get(at..self.end)?.get(..2) != Some(b">\t")
            || indent_width(b'\t', self.spaces_column + 1) == 1
        {
            return None;
        }
        let mut probe = *self;
        probe.inspected = 0;
        probe.split_tabs = true;
        let quoted = probe.quote();
        debug_assert!(quoted);
        // The next explicit marker proves a nested quote without classifying
        // its suffix. Repeated `>\t >` prefixes therefore stay linear.
        let nested_quote = probe.spaces() <= 3 && probe.spaces_end < probe.end && {
            probe.inspected += 1;
            probe.bytes[probe.spaces_end] == b'>'
        };
        let candidate = probe.spaces() <= 3 && probe.spaces_end < probe.end;
        // The suffix already proves a complete homogeneous rule. Reuse it
        // instead of rescanning the remaining payload at every new quote.
        let accepted = nested_quote
            // spaces() above already found the horizontal whitespace endpoint.
            // A complete blank needs no indentation cap or payload rescan.
            || probe.complete && probe.spaces_end == probe.end
            || probe.is_rule(rule_suffix)
            // Marker parsing is bounded (at most nine digits plus following
            // whitespace). On success the next iteration consumes that same
            // marker; failed markers may still classify as ordinary text below.
            || probe.spaces() <= 3 && probe.marker().is_some()
            || candidate && {
                let payload = Span::new(probe.pos, span.end);
                work.classified_bytes += payload.len() as u64;
                match crate::block::classify_indented(
                    source,
                    payload,
                    probe.complete,
                    None,
                    false,
                    probe.virtual_spaces as u8,
                    ((probe.column + probe.virtual_spaces) % 4) as u8,
                ) {
                    crate::block::Line::Text
                    | crate::block::Line::FenceOpen { .. }
                    | crate::block::Line::Heading { .. }
                    | crate::block::Line::FootnoteDefinition { .. } => true,
                    crate::block::Line::Definition { label, .. }
                    | crate::block::Line::DefinitionStart { label } => {
                        // Match intern's rejection without allocating or
                        // mutating symbols in this speculative prefix probe.
                        probe.inspected += label.len() as u64;
                        label.len() <= crate::references::MAX_LABEL_BYTES
                            && source[label.range()].chars().any(|c| !c.is_whitespace())
                    }
                    _ => false,
                }
            };
        self.inspected += probe.inspected;
        probe.inspected = self.inspected;
        accepted.then_some(probe)
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
            // A complete empty marker has one (possibly implicit) separator
            // column for continuation, independent of trailing whitespace.
            return Some(Marker {
                kind,
                number,
                end: at,
                column: column + 1,
                code_spaces: None,
            });
        }
        self.inspected += 1;
        if !matches!(self.bytes[at], b' ' | b'\t') {
            return None;
        }
        let whitespace = at;
        let marker_column = column;
        while at < self.end && matches!(self.bytes[at], b' ' | b'\t') {
            self.inspected += 1;
            column += indent_width(self.bytes[at], column);
            at += 1;
        }
        if self.complete && at == self.end {
            return Some(Marker {
                kind,
                number,
                end: at,
                column: marker_column + 1,
                code_spaces: None,
            });
        }
        // A bounded first-line prefix with at least five whitespace columns uses
        // one column as separator and four as code indentation. Reuse the scanned
        // whitespace endpoint; advancing the probe consumes at most five bytes.
        // Incomplete prefixes retain the existing dialect.
        if column - marker_column >= 5 && (!self.complete || at < self.end) {
            let mut probe = *self;
            probe.pos = whitespace;
            probe.column = marker_column;
            probe.virtual_spaces = 0;
            probe.split_tabs = true;
            probe.spaces_end = at;
            probe.spaces_column = column;
            probe.inspected = 0;
            let consumed = probe.consume_indent(5);
            debug_assert!(consumed);
            self.inspected += probe.inspected;
            return Some(Marker {
                kind,
                number,
                end: probe.pos,
                column: marker_column + 1,
                code_spaces: Some(probe.virtual_spaces as u8),
            });
        }
        Some(Marker {
            kind,
            number,
            end: at,
            column,
            code_spaces: None,
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
