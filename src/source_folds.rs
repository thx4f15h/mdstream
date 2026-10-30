use std::sync::atomic::{AtomicUsize, Ordering};

use crate::{
    BlockId, BlockKind, Budget, Completion, Document, DocumentId, FoldCandidate, FoldId, FoldView,
    LayoutError, Span, SyntaxState, WorkLimit,
};

/// Opaque, process-unique registration identity. It belongs to one FoldView,
/// survives append/reset, and is never reused after removal. Hosts can associate
/// their own selection/object keys with it without changing document syntax.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceFoldId(usize);

impl SourceFoldId {
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
    /// Pending lines, the current leaf, table probes and the entire possible
    /// container Setext title are excluded. Closing them advances this bound;
    /// finish makes all source eligible. O(1), without inspecting body bytes.
    /// This is a fold-topology boundary, not a promise of final inline styles.
    pub fn source_fold_limit(&self) -> usize {
        if self.is_finished() {
            return self.source().len();
        }
        let mut end = self.line_start;
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
    /// Present only after atomic registration. The initial choice is expanded.
    pub id: Option<FoldId>,
    /// Cumulative syntax ancestors inspected, including both endpoint paths.
    pub visited: usize,
}

/// Validate and atomically register one fixed, nonempty UTF-8 source interval.
/// Source registrations must be disjoint (including while expanded/previewed).
/// Syntax/registered-row ranges may contain it or be contained by it; crossings
/// are rejected. No syntax parents are rewritten. The source candidate's parent
/// is its innermost enclosing syntax source range at registration.
///
/// Only Document::source_fold_limit's prefix is accepted. Both anchors have
/// left affinity: later source never joins this interval, even at the old EOF.
/// Edits require removal and a new ID. Until completion no view state changes;
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
    endpoint: usize,
    started: bool,
    next: Option<FoldId>,
    parent: Option<FoldId>,
    visited: usize,
    result: Option<FoldId>,
}

impl SourceFoldRegistration {
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
        Ok(Self {
            document: document.id(),
            revision: document.revision(),
            view: view.heights().version().0,
            version: view.version(),
            source,
            hidden: source,
            paragraph: None,
            endpoint: 0,
            started: false,
            next: None,
            parent: None,
            visited: 0,
            result: None,
        })
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
    /// Setext/table candidates, container physical leaves and open paragraphs
    /// require a later registration. O(log R), no prefix/body scan or allocation.
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
            if !budget.charge(cost()) {
                return Ok(self.progress(Completion::BudgetExhausted));
            }
            if self.endpoint == 2 {
                self.result = Some(view.register_source_fold(
                    self.source,
                    self.hidden,
                    self.parent,
                    self.paragraph,
                ));
                self.version = view.version();
            } else if !self.started {
                let offset = if self.endpoint == 0 {
                    self.source.start
                } else {
                    self.source.end - 1
                };
                // Registered row ranges form a disjoint overlay. Their parents
                // remain syntax-only, so removing a row cannot orphan this ID.
                if let Some(row) = view.row_candidate_at(document, offset) {
                    validate_pair(self.source, row)?;
                }
                self.next = document.fold_candidate_at(offset).map(|c| c.id);
                self.started = true;
            } else if let Some(id) = self.next {
                let candidate = document.fold_candidate(id).expect("stable syntax identity");
                validate_pair(self.source, candidate)?;
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
/// window, including an earlier containing range. Does not enumerate syntax or
/// generate possible selections. O(1) cursor storage and O(log R) per result.
pub struct SourceFoldCandidates {
    document: DocumentId,
    revision: u64,
    view: DocumentId,
    version: u64,
    next: usize,
    end: usize,
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
        let next = if range.is_empty() {
            range.end
        } else {
            view.source_fold_at(document, range.start)?
                .map_or(range.start, |c| c.source.start)
        };
        Ok(Self {
            document: document.id(),
            revision: document.revision(),
            view: view.heights().version().0,
            version: view.version(),
            next,
            end: range.end,
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
        while self.next < self.end {
            if !budget.charge(cost()) {
                return Ok(Completion::BudgetExhausted);
            }
            let Some(candidate) = view.next_source_fold(self.next, self.end) else {
                self.next = self.end;
                break;
            };
            self.next = candidate.source.end;
            out.push(candidate);
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
