use std::{
    ops::Bound,
    sync::atomic::{AtomicUsize, Ordering},
};

use crate::{
    BlockId, BlockKind, Budget, Completion, Document, DocumentId, FoldCandidate, FoldId, FoldView,
    LayoutError, Span, SyntaxState, WorkLimit,
};

/// Opaque, process-unique registration identity. It belongs to one FoldView,
/// remains stable while registered (including append/reset), and is never reused
/// after removal. Hosts can associate their own selection/object keys with it
/// without changing document syntax.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceFoldId(pub(crate) usize);

impl SourceFoldId {
    /// Stable diagnostic/UI identity; this does not construct or rebind an ID.
    pub fn index(self) -> usize {
        self.0
    }

    pub(crate) fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        Self(
            NEXT.try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
                .expect("source fold identity space exhausted"),
        )
    }
}

impl Document {
    /// Conservative end of the source prefix eligible for fixed source folds.
    /// Pending lines, definitions awaiting a possible title, the current leaf,
    /// table probes and the entire possible container Setext title are excluded.
    /// Closing them advances this bound;
    /// finish makes all source eligible. O(1), without inspecting body bytes.
    /// This is a fold-topology boundary, not a promise of final inline styles.
    pub fn source_fold_limit(&self) -> usize {
        if self.is_finished() {
            return self.source().len();
        }
        let mut end = self.line_start;
        if let Some(candidate) = &self.definition_candidate {
            end = end.min(self.blocks[candidate.block.0].source.start);
        }
        if let Some(candidate) = &self.definition_title_candidate {
            end = end.min(self.blocks[candidate.block.0].source.start);
        }
        if let Some(id) = self.current {
            end = end.min(self.blocks[id.0].source.start);
        }
        if let Some((id, _)) = self.container_title {
            end = end.min(self.blocks[id.0].source.start);
        }
        if let Some(header) = &self.table_header {
            end = end.min(header.source.start);
        }
        if let Some(header) = &self.streaming_header {
            end = end.min(header.header.source.start);
        }
        end
    }

    // Freeze one logical container paragraph, never its individual leaves.
    // Setext promotion invalidates the first leaf; a table retracts the last.
    pub(crate) fn close_container_paragraph(&mut self, end: usize) {
        let Some((first, _)) = self.container_title.take() else {
            return;
        };
        let block = &self.blocks[first.0];
        if matches!(
            block.kind,
            BlockKind::Paragraph | BlockKind::Quote { .. } | BlockKind::ListItem { .. }
        ) && block.source.start < end
        {
            self.container_paragraph_ends.insert(first, end);
            self.work.paragraph_closures += 1;
        }
    }

    // A single logical paragraph can grow or change kind. Container physical
    // Sealed states do not close it. Pending input provisionally joins its tail.
    pub(crate) fn open_paragraph_source(&self, block: BlockId) -> Option<(Span, bool)> {
        let entry = self.block(block)?;
        if entry.container.is_some() {
            if self
                .container_title
                .is_some_and(|(first, _)| first == block)
            {
                return Some((Span::new(entry.source.start, self.source().len()), true));
            }
            let end = *self.container_paragraph_ends.get(&block)?;
            return Some((Span::new(entry.source.start, end), false));
        }
        if entry.kind != BlockKind::Paragraph {
            return None;
        }
        let open = entry.state != SyntaxState::Sealed && !self.is_finished();
        Some((
            Span::new(
                entry.source.start,
                if open {
                    self.source().len()
                } else {
                    entry.source.end
                },
            ),
            open,
        ))
    }
}

fn contains(outer: Span, inner: Span) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
}

/// Source envelopes and effective hidden intervals must both remain laminar.
/// Straddling a growing range's start would allow a later crossing, so reject it.
pub(crate) fn validate_pair(source: Span, candidate: FoldCandidate) -> Result<(), LayoutError> {
    for span in [candidate.source, candidate.hidden] {
        if !span.is_empty()
            && source.start < span.end
            && span.start < source.end
            && !contains(source, span)
            && !contains(span, source)
        {
            return Err(LayoutError::InvalidRange);
        }
        // The visible title can lie outside hidden. A selection crossing the
        // hidden start may currently contain the entire body at EOF, but that
        // body's future growth would cross the selection's fixed end. Equal
        // starts are safe: subsequent growth simply contains the selection.
        if candidate.open && source.start < span.start && span.start < source.end {
            return Err(LayoutError::InvalidRange);
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceFoldProgress {
    pub completion: Completion,
    /// Present only after atomic registration or seal. New registrations start expanded.
    pub id: Option<FoldId>,
    /// Cumulative syntax ancestors inspected, including both endpoint paths.
    pub visited: usize,
}

#[derive(Clone, Copy)]
enum SourceAction {
    Register,
    RootTail,
    Stream,
    SealTail(SourceFoldId),
}

/// Validate and atomically register one nonempty UTF-8 source interval.
/// `new`, `tail` and the original paragraph constructors retain strict disjoint
/// registrations and reject syntax crossings. `overlapping`, `stream` and
/// `paragraph_overlay` opt into independent nested/crossing source overlays.
/// Syntax parents are never rewritten. Source parent lookup resolves containing
/// manual envelopes lazily, with the captured syntax host taking precedence
/// when it lies inside that envelope. No descendant metadata is rewritten.
/// Active registrations per view are capped at the document's source byte count
/// (InvalidRange at publication), keeping view storage O(n) even for duplicates.
///
/// `new` and `paragraph` only accept Document::source_fold_limit's prefix;
/// their anchors have left affinity, including at the old EOF. `open_paragraph`
/// follows the active paragraph and may be removed on late structural changes.
/// `container_paragraph` accepts open/closed multi-leaf container paragraphs.
/// Changing the prefix requires removal and a new ID. Until completion no view state changes;
/// cancellation, an error or stale document/view leaves no partial registration.
/// O(1) task storage; two ancestor paths, never all candidates or covered bytes.
pub struct SourceFoldRegistration {
    document: DocumentId,
    revision: u64,
    view: DocumentId,
    version: u64,
    source: Span,
    hidden: Span,
    paragraph: Option<BlockId>,
    open: bool,
    endpoint: usize,
    started: bool,
    next: Option<FoldId>,
    parent: Option<FoldId>,
    visited: usize,
    result: Option<FoldId>,
    action: SourceAction,
    flexible: bool,
    restored_choice: Option<bool>,
}

impl SourceFoldRegistration {
    pub(crate) fn restore_choice(&mut self, collapsed: bool) {
        self.restored_choice = Some(collapsed);
    }

    /// Fixed metadata/index checks, O(log R); ancestor validation is deferred.
    pub fn new<S>(
        document: &Document,
        view: &FoldView<S>,
        source: Span,
    ) -> Result<Self, LayoutError> {
        view.check(document)?;
        if source.start >= source.end
            || source.end > document.source_fold_limit()
            || !document.source().is_char_boundary(source.start)
            || !document.source().is_char_boundary(source.end)
        {
            return Err(LayoutError::InvalidRange);
        }
        view.validate_source_span(source)?;
        Ok(Self::unvalidated(document, view, source))
    }

    fn unvalidated<S>(document: &Document, view: &FoldView<S>, source: Span) -> Self {
        Self {
            document: document.id(),
            revision: document.revision(),
            view: view.heights().version().0,
            version: view.version(),
            source,
            hidden: source,
            paragraph: None,
            open: false,
            endpoint: 0,
            started: false,
            next: None,
            parent: None,
            visited: 0,
            result: None,
            action: SourceAction::Register,
            flexible: false,
            restored_choice: None,
        }
    }

    /// Register a live root tail from a stable UTF-8 cut to the real source EOF.
    /// The start must be strictly before source_fold_limit and outside every
    /// syntax fold envelope. Root validation and atomic insertion each cost one
    /// query/object in advance. Initially expanded; apply grows the same ID,
    /// including pending input, and finish closes it automatically.
    /// Manual registrations remain disjoint even while expanded or previewed.
    pub fn tail<S>(
        document: &Document,
        view: &FoldView<S>,
        start: usize,
    ) -> Result<Self, LayoutError> {
        view.check(document)?;
        if document.is_finished()
            || start >= document.source_fold_limit()
            || !document.source().is_char_boundary(start)
        {
            return Err(LayoutError::InvalidRange);
        }
        let source = Span::new(start, document.source().len());
        view.validate_source_span(source)?;
        let mut task = Self::unvalidated(document, view, source);
        task.open = true;
        task.action = SourceAction::RootTail;
        Ok(task)
    }

    /// Register a fixed source envelope with independent nested/crossing intent.
    /// Intersecting collapsed intervals share one placeholder; adjacent intervals
    /// keep separate placeholders. Syntax crossings are supported too. Fixed
    /// endpoints may lie in pending input; late syntax classification does not
    /// move the cuts or transfer identity. No source bytes are changed.
    pub fn overlapping<S>(
        document: &Document,
        view: &FoldView<S>,
        source: Span,
    ) -> Result<Self, LayoutError> {
        view.check(document)?;
        if source.start >= source.end
            || source.end > document.source().len()
            || !document.source().is_char_boundary(source.start)
            || !document.source().is_char_boundary(source.end)
        {
            return Err(LayoutError::InvalidRange);
        }
        let mut task = Self::unvalidated(document, view, source);
        task.flexible = true;
        Ok(task)
    }

    /// A source-anchored tail from any existing UTF-8 cut, including pending
    /// input and syntax hosts. The cut stays fixed across late classification;
    /// all tails share the real EOF, without visiting registrations on append.
    /// A syntax host's closure does not close this document-tail range. Finish
    /// or explicit seal closes it; local paragraph lifetime uses the paragraph
    /// constructors instead. Nested and crossing overlays keep independent IDs.
    pub fn stream<S>(
        document: &Document,
        view: &FoldView<S>,
        start: usize,
    ) -> Result<Self, LayoutError> {
        view.check(document)?;
        if document.is_finished()
            || start >= document.source().len()
            || !document.source().is_char_boundary(start)
        {
            return Err(LayoutError::InvalidRange);
        }
        let mut task = Self::unvalidated(document, view, Span::new(start, document.source().len()));
        task.open = true;
        task.flexible = true;
        task.action = SourceAction::Stream;
        Ok(task)
    }

    /// Close this view's live tail without changing its ID or saved choice.
    /// Strict root tails require stable EOF and reject future syntax crossings.
    /// Stream overlays accept the current UTF-8 EOF even while pending, retaining
    /// a fixed cut through subsequent classification. Both endpoint paths share
    /// the budget. Exhaustion
    /// is resumable and no view state changes before the final paid step.
    pub fn seal_tail<S>(
        document: &Document,
        view: &FoldView<S>,
        id: SourceFoldId,
    ) -> Result<Self, LayoutError> {
        view.check(document)?;
        let candidate = view
            .live_source_tail(document, id)
            .ok_or(LayoutError::InvalidRange)?;
        if candidate.source.end > document.source_fold_limit() && !view.source_fold_is_flexible(id)
        {
            return Err(LayoutError::InvalidRange);
        }
        // Reuse this registration directly. Strict or overlay policy is kept;
        // exempting its own identity needs no history enumeration.
        let mut task = Self::unvalidated(document, view, candidate.source);
        task.action = SourceAction::SealTail(id);
        task.flexible = view.source_fold_is_flexible(id);
        Ok(task)
    }

    /// Register a bounded source-prefix preview of a closed top-level paragraph.
    /// The candidate's source covers the whole block, summary is the retained
    /// prefix, and hidden is the suffix. The resulting SourceFoldId is view-local;
    /// its paragraph BlockId and fixed boundaries survive append and reset.
    ///
    /// `preview_bytes` includes hidden syntax and is rounded down to UTF-8 and
    /// CRLF boundaries. Zero is allowed; a prefix covering the whole block is
    /// rejected (there would be no suffix). This is not a display-line, grapheme
    /// or terminal-width limit. Inline styles/mappings are not reparsed at the
    /// cut. Summary display can separately use DisplayExcerpt.
    ///
    /// The entire paragraph reserves one disjoint source registration, including
    /// its visible prefix. Remove it before changing the limit. Only sealed,
    /// top-level Paragraph blocks inside source_fold_limit are accepted; pending
    /// Setext/table candidates and open paragraphs can use `open_paragraph`;
    /// container sequences use [`Self::container_paragraph`]. O(log R), no body scan.
    pub fn paragraph<S>(
        document: &Document,
        view: &FoldView<S>,
        block: BlockId,
        preview_bytes: usize,
    ) -> Result<Self, LayoutError> {
        view.check(document)?;
        let entry = document.block(block).ok_or(LayoutError::InvalidRange)?;
        if entry.kind != BlockKind::Paragraph
            || entry.container.is_some()
            || entry.state != SyntaxState::Sealed
            || preview_bytes >= entry.source.len()
        {
            return Err(LayoutError::InvalidRange);
        }
        let mut task = Self::new(document, view, entry.source)?;
        let mut cut = entry.source.start + preview_bytes;
        while !document.source().is_char_boundary(cut) {
            cut -= 1;
        }
        let bytes = document.source().as_bytes();
        if cut > entry.source.start && bytes[cut - 1] == b'\r' && bytes[cut] == b'\n' {
            cut -= 1;
        }
        task.hidden.start = cut;
        task.paragraph = Some(block);
        Ok(task)
    }

    /// Register the current top-level paragraph with a fixed source-byte prefix
    /// and a suffix that follows append. The prefix must leave at least one
    /// already classified byte; UTF-8/CRLF rounding matches [`Self::paragraph`].
    /// Unclassified trailing input provisionally joins the hidden suffix.
    ///
    /// `FoldView::apply` keeps the ID and choice on growth/ordinary closure. A
    /// late table may retract the suffix; if it removes the entire suffix, or
    /// the block becomes a Setext heading/table, apply removes the registration,
    /// choice and preview. It does not transfer them to the new syntax object.
    /// Query `FoldView::fold_candidate` after apply to observe removal. Closed
    /// paragraphs use [`Self::paragraph`]; container sequences use
    /// [`Self::container_paragraph`].
    ///
    /// One active registration is reconciled per update, including while
    /// expanded/previewed. No historical registrations, body bytes or ancestors
    /// are revisited. The whole provisional source range reserves the interval;
    /// manual registrations remain disjoint. Registration itself shares the
    /// usual endpoint-path budget and is atomic/revision-bound.
    pub fn open_paragraph<S>(
        document: &Document,
        view: &FoldView<S>,
        block: BlockId,
        preview_bytes: usize,
    ) -> Result<Self, LayoutError> {
        view.check(document)?;
        let (source, open) = document
            .open_paragraph_source(block)
            .ok_or(LayoutError::InvalidRange)?;
        let entry = document.block(block).unwrap();
        if entry.container.is_some()
            || !open
            || document.current != Some(block)
            || preview_bytes >= entry.source.len()
        {
            return Err(LayoutError::InvalidRange);
        }
        Self::paragraph_range(document, view, block, preview_bytes, source, open, false)
    }

    /// Register a logical paragraph in one exact container scope, using its
    /// first physical leaf's BlockId. Explicit and lazy continuations share
    /// this identity; later leaves, blank markers and non-text blocks fail.
    /// Both open and closed paragraphs are accepted. The source-byte prefix
    /// includes physical container prefixes and must leave classified content.
    /// UTF-8/CRLF rounding and layout end callbacks match [`Self::paragraph`].
    ///
    /// Open ranges include pending input. Apply freezes them at a scope/block
    /// boundary, retracts a late table header, or removes them after Setext or
    /// whole-table promotion (also when retraction removes the hidden suffix).
    /// Physical leaves and inline objects are never merged or copied. The
    /// registration's parent is its innermost syntax ancestor; the two endpoint
    /// paths consume budget even for arbitrarily deep containers.
    ///
    /// Construction uses O(log P + log R) index work, no leaf/body scan. Each
    /// closed paragraph stores one end; open growth uses the existing parser
    /// endpoints. One active registration per view is reconciled on apply.
    pub fn container_paragraph<S>(
        document: &Document,
        view: &FoldView<S>,
        first: BlockId,
        preview_bytes: usize,
    ) -> Result<Self, LayoutError> {
        view.check(document)?;
        let entry = document.block(first).ok_or(LayoutError::InvalidRange)?;
        if entry.container.is_none() {
            return Err(LayoutError::InvalidRange);
        }
        let (source, open) = document
            .open_paragraph_source(first)
            .ok_or(LayoutError::InvalidRange)?;
        let classified_end = if open {
            document.blocks[document.current.unwrap().0].source.end
        } else {
            source.end
        };
        if preview_bytes >= classified_end - source.start
            || (!open && source.end > document.source_fold_limit())
        {
            return Err(LayoutError::InvalidRange);
        }
        Self::paragraph_range(document, view, first, preview_bytes, source, open, false)
    }

    /// Paragraph prefix preview using the same nested/crossing overlay policy
    /// as `overlapping`. Accepts an open/closed root or container paragraph.
    /// One active logical paragraph preview per view; document-tail overlays
    /// can be nested around it in either registration order.
    pub fn paragraph_overlay<S>(
        document: &Document,
        view: &FoldView<S>,
        first: BlockId,
        preview_bytes: usize,
    ) -> Result<Self, LayoutError> {
        view.check(document)?;
        let (source, open) = document
            .open_paragraph_source(first)
            .ok_or(LayoutError::InvalidRange)?;
        let end = if open && document.block(first).unwrap().container.is_some() {
            document
                .block(document.current.ok_or(LayoutError::InvalidRange)?)
                .unwrap()
                .source
                .end
        } else {
            document.block(first).unwrap().source.end
        };
        if preview_bytes >= end - source.start
            || (!open && source.end > document.source_fold_limit())
            || (open && view.has_open_paragraph())
        {
            return Err(LayoutError::InvalidRange);
        }
        Self::paragraph_range(document, view, first, preview_bytes, source, open, true)
    }

    fn paragraph_range<S>(
        document: &Document,
        view: &FoldView<S>,
        block: BlockId,
        preview_bytes: usize,
        source: Span,
        open: bool,
        flexible: bool,
    ) -> Result<Self, LayoutError> {
        if !flexible {
            view.validate_source_span(source)?;
        }
        let mut cut = source.start + preview_bytes;
        while !document.source().is_char_boundary(cut) {
            cut -= 1;
        }
        let bytes = document.source().as_bytes();
        if cut > source.start && bytes[cut - 1] == b'\r' && bytes[cut] == b'\n' {
            cut -= 1;
        }
        Ok(Self {
            document: document.id(),
            revision: document.revision(),
            view: view.heights().version().0,
            version: view.version(),
            source,
            hidden: Span::new(cut, source.end),
            paragraph: Some(block),
            open,
            endpoint: 0,
            started: false,
            next: None,
            parent: None,
            visited: 0,
            result: None,
            action: SourceAction::Register,
            flexible,
            restored_choice: None,
        })
    }

    /// One query/object per endpoint lookup, ancestor, path end or final insertion. Each
    /// step performs a fixed number of indexed operations and reads zero body
    /// bytes. Exhaustion preserves the cursor; success invalidates other view
    /// tasks. Appends/external view changes require a new registration task.
    pub fn advance<S>(
        &mut self,
        document: &Document,
        view: &mut FoldView<S>,
        budget: &mut Budget,
    ) -> Result<SourceFoldProgress, LayoutError> {
        check(
            document,
            view,
            self.document,
            self.revision,
            self.view,
            self.version,
        )?;
        while self.result.is_none() {
            let mut step_cost = cost();
            if self.endpoint == 2 && self.restored_choice.is_some() {
                step_cost.recovery = 1;
            }
            if !budget.charge(step_cost) {
                return Ok(self.progress(Completion::BudgetExhausted));
            }
            if matches!(self.action, SourceAction::RootTail) && self.endpoint == 0 {
                if document.fold_candidate_at(self.source.start).is_some()
                    || view.row_candidate_at(document, self.source.start).is_some()
                {
                    return Err(LayoutError::InvalidRange);
                }
                self.endpoint = 2;
            } else if self.endpoint == 2 {
                if !matches!(self.action, SourceAction::SealTail(_))
                    && view.source_fold_count() >= document.source().len()
                {
                    return Err(LayoutError::InvalidRange);
                }
                self.result = Some(match self.action {
                    SourceAction::SealTail(id) => view.seal_source_tail(document, id)?,
                    SourceAction::Register | SourceAction::RootTail | SourceAction::Stream => view
                        .register_source_fold(
                            self.source,
                            self.hidden,
                            self.parent,
                            self.paragraph,
                            self.open,
                            self.flexible,
                        ),
                });
                if let Some(choice) = self.restored_choice {
                    view.set_collapsed(document, self.result.unwrap(), choice)?;
                }
                self.version = view.version();
            } else if !self.started {
                let offset = if self.endpoint == 0 {
                    self.source.start
                } else {
                    self.source.end - 1
                };
                // Registered row ranges form a disjoint overlay. Their parents
                // remain syntax-only, so removing a row cannot orphan this ID.
                if !self.flexible
                    && let Some(row) = view.row_candidate_at(document, offset)
                {
                    validate_pair(self.source, row)?;
                }
                self.next = document.fold_candidate_at(offset).map(|c| c.id);
                self.started = true;
            } else if let Some(id) = self.next {
                let candidate = document.fold_candidate(id).expect("stable syntax identity");
                if !self.flexible {
                    validate_pair(self.source, candidate)?;
                }
                self.visited += 1;
                if self.endpoint == 0
                    && self.parent.is_none()
                    && contains(candidate.source, self.source)
                {
                    self.parent = Some(id);
                }
                self.next = candidate.parent;
            } else {
                self.endpoint += 1;
                self.started = false;
            }
        }
        Ok(self.progress(Completion::Covered))
    }

    fn progress(&self, completion: Completion) -> SourceFoldProgress {
        SourceFoldProgress {
            completion,
            id: self.result,
            visited: self.visited,
        }
    }
}

/// Budgeted enumeration of registered source ranges intersecting a source
/// window, including all earlier containing/crossing ranges. Does not enumerate syntax or
/// generate possible selections. O(1) cursor storage and O(log R) per result.
pub struct SourceFoldCandidates {
    document: DocumentId,
    revision: u64,
    view: DocumentId,
    version: u64,
    after: Bound<crate::fold_intervals::Key>,
    range: Span,
    done: bool,
}

impl SourceFoldCandidates {
    pub fn new<S>(
        document: &Document,
        view: &FoldView<S>,
        range: Span,
    ) -> Result<Self, LayoutError> {
        view.check(document)?;
        if range.start > range.end || range.end > document.source().len() {
            return Err(LayoutError::InvalidRange);
        }
        Ok(Self {
            document: document.id(),
            revision: document.revision(),
            view: view.heights().version().0,
            version: view.version(),
            after: Bound::Unbounded,
            range,
            done: range.is_empty(),
        })
    }

    pub fn advance<S>(
        &mut self,
        document: &Document,
        view: &FoldView<S>,
        budget: &mut Budget,
        out: &mut Vec<FoldCandidate>,
    ) -> Result<Completion, LayoutError> {
        check(
            document,
            view,
            self.document,
            self.revision,
            self.view,
            self.version,
        )?;
        out.clear();
        while !self.done {
            if !budget.charge(cost()) {
                return Ok(Completion::BudgetExhausted);
            }
            let Some(candidate) =
                view.next_source_fold(document, self.after, self.range.start, self.range.end)
            else {
                self.done = true;
                break;
            };
            self.after = Bound::Excluded(view.source_cursor(candidate.id));
            out.push(candidate);
            self.done = view
                .next_source_fold(document, self.after, self.range.start, self.range.end)
                .is_none();
        }
        Ok(Completion::Covered)
    }
}

fn cost() -> WorkLimit {
    WorkLimit {
        queries: 1,
        objects: 1,
        ..Default::default()
    }
}

fn check<S>(
    document: &Document,
    view: &FoldView<S>,
    expected_document: DocumentId,
    revision: u64,
    expected_view: DocumentId,
    version: u64,
) -> Result<(), LayoutError> {
    if document.id() != expected_document {
        return Err(LayoutError::WrongDocument);
    }
    if view.heights().version().0 != expected_view {
        return Err(LayoutError::WrongView);
    }
    if document.revision() != revision || view.version() != version {
        return Err(LayoutError::StaleContinuation);
    }
    view.check(document)
}
