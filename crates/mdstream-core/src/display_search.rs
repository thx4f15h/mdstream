use std::ops::Range;

use crate::{
    Budget, Completion, DisplayCursor, DisplayStep, Document, DocumentId, LayoutError, Span, Style,
    TILE_BYTES, WorkLimit, search::failure_table,
};

/// One literal match in a revision's concatenated, non-HIDDEN display runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayMatch {
    /// UTF-8 byte offsets in the display stream, not source or screen coordinates.
    pub display: Range<usize>,
    /// Smallest source envelope containing the contributing display bytes.
    /// Includes hidden syntax between them and whole replacements at either end.
    /// Distinct display matches can map to the same source (e.g. `&fjlig;`).
    pub source: Span,
}

/// Budgeted, literal, case-sensitive search of one document revision.
/// Matches can overlap and cross runs, hidden syntax, and source tiles. No
/// separators, case folding, or Unicode normalization are added to display text.
///
/// Any revision change invalidates this task AND its previously emitted matches.
/// The caller must discard those matches and explicitly start a new task; this
/// never restarts a whole-document scan implicitly on append or late closure.
pub struct DisplaySearch {
    document: DocumentId,
    revision: u64,
    cursor: DisplayCursor,
    pattern: Vec<u8>,
    failure: Vec<usize>,
    matched: usize,
    display_position: usize,
    // Ring of the last pattern.len() consumed bytes' source mappings. KMP
    // fallback only changes an index; it never copies a matched prefix.
    sources: Vec<Span>,
    next_source: usize,
    // Keep a bounded run across budget boundaries without borrowing Document.
    pending: [u8; TILE_BYTES],
    pending_len: usize,
    pending_position: usize,
    pending_source: Span,
    pending_replacement: Option<Span>,
}

impl DisplaySearch {
    /// O(query bytes) preprocessing and storage, plus one fixed-size run buffer.
    /// Construction does not scan source or copy the document's display text.
    pub fn new(document: &Document, query: &str) -> Result<Self, LayoutError> {
        if query.is_empty() {
            return Err(LayoutError::InvalidRange);
        }
        let pattern = query.as_bytes().to_vec();
        let failure = failure_table(&pattern);
        Ok(Self {
            document: document.id(),
            revision: document.revision(),
            cursor: document
                .display_cursor(Span::new(0, document.source().len()))
                .unwrap(),
            sources: vec![Span::default(); pattern.len()],
            pattern,
            failure,
            matched: 0,
            display_position: 0,
            next_source: 0,
            pending: [0; TILE_BYTES],
            pending_len: 0,
            pending_position: 0,
            pending_source: Span::default(),
            pending_replacement: None,
        })
    }

    /// Move the selection memo out before dropping/replacing an old revision.
    pub fn take_link_resolver(&mut self) -> crate::LinkResolver {
        self.cursor.take_link_resolver()
    }
    pub fn swap_link_resolver(
        &mut self,
        resolver: &mut crate::LinkResolver,
    ) -> Result<(), LayoutError> {
        self.cursor.swap_link_resolver(resolver)
    }
    pub fn retire(&mut self, budget: &mut Budget) -> Completion {
        self.cursor.retire(budget)
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Source read so far. A bounded run may still await matching, so reaching
    /// the source end alone does not mean the search is complete.
    pub fn source_position(&self) -> usize {
        self.cursor.source_position()
    }

    /// Display bytes consumed by the matcher; may pause within a UTF-8 scalar.
    pub fn display_position(&self) -> usize {
        self.display_position
    }

    /// Append matches to caller storage. Reading uses DisplayCursor's byte,
    /// query and object charges; each comparison (including KMP fallback) costs
    /// one more byte, and emitting a match costs one object. A failed charge
    /// preserves the pending byte. Six bytes, one query and one object per call
    /// suffice to make progress, possibly reading and matching in separate calls.
    ///
    /// EndOfDocument means this revision has been searched, even before finish.
    /// Repeated calls then do no work. Wrong/stale documents are rejected before
    /// consuming budget, pending bytes, or modifying output.
    pub fn advance(
        &mut self,
        document: &Document,
        budget: &mut Budget,
        out: &mut Vec<DisplayMatch>,
    ) -> Result<Completion, LayoutError> {
        self.advance_with(document, budget, |hit| out.push(hit))
    }

    pub(crate) fn validate(&self, document: &Document) -> Result<(), LayoutError> {
        if document.id() != self.document {
            return Err(LayoutError::WrongDocument);
        }
        if document.revision() != self.revision {
            return Err(LayoutError::StaleContinuation);
        }
        self.cursor.validate(document)
    }

    /// Reuse the prepared pattern for another independent range in the same
    /// revision. Old ring entries need no clearing: a new full match overwrites
    /// all of them before reading its source start. The caller validates first.
    pub(crate) fn reset(&mut self, mut cursor: DisplayCursor) {
        let mut resolver = self.cursor.take_link_resolver();
        cursor.swap_link_resolver(&mut resolver).unwrap();
        self.cursor = cursor;
        self.matched = 0;
        self.display_position = 0;
        self.next_source = 0;
        self.pending_len = 0;
        self.pending_position = 0;
    }

    pub(crate) fn advance_with(
        &mut self,
        document: &Document,
        budget: &mut Budget,
        mut emit: impl FnMut(DisplayMatch),
    ) -> Result<Completion, LayoutError> {
        self.validate(document)?;
        loop {
            if self.pending_position == self.pending_len {
                match self.cursor.next(document, budget)? {
                    DisplayStep::Run(run) => {
                        if run.style.contains(Style::HIDDEN) || run.text.is_empty() {
                            continue;
                        }
                        self.pending_len = run.text.len();
                        self.pending[..self.pending_len].copy_from_slice(run.text.as_bytes());
                        self.pending_position = 0;
                        self.pending_source = run.source;
                        self.pending_replacement = run.replacement;
                    }
                    DisplayStep::Complete => return Ok(Completion::EndOfDocument),
                    DisplayStep::BudgetExhausted => return Ok(Completion::BudgetExhausted),
                }
            }
            let byte = self.pending[self.pending_position];
            let equal = byte == self.pattern[self.matched];
            let complete = equal && self.matched + 1 == self.pattern.len();
            if !budget.charge(WorkLimit {
                bytes: 1,
                objects: usize::from(complete),
                ..Default::default()
            }) {
                return Ok(Completion::BudgetExhausted);
            }
            if !equal && self.matched > 0 {
                self.matched = self.failure[self.matched - 1];
                continue;
            }
            let source = self.pending_replacement.unwrap_or_else(|| {
                let start = self.pending_source.start + self.pending_position;
                Span::new(start, start + 1)
            });
            self.sources[self.next_source] = source;
            self.next_source = (self.next_source + 1) % self.sources.len();
            self.pending_position += 1;
            self.display_position += 1;
            if equal {
                self.matched += 1;
                if complete {
                    emit(DisplayMatch {
                        display: self.display_position - self.pattern.len()..self.display_position,
                        source: Span::new(self.sources[self.next_source].start, source.end),
                    });
                    self.matched = self.failure[self.matched - 1];
                }
            }
        }
    }
}
