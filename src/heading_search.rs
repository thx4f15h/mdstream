use std::ops::Range;

use crate::{Budget, Completion, DisplaySearch, Document, HeadingId, LayoutError, Span, WorkLimit};

/// One occurrence in a heading's non-HIDDEN display text. Repeated names and
/// overlapping occurrences remain distinct; no match crosses heading boundaries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeadingMatch {
    pub heading: HeadingId,
    /// UTF-8 byte offsets relative to this heading's displayed content.
    pub display: Range<usize>,
    /// Absolute source envelope, including whole replacements at either end.
    pub source: Span,
}

/// Revision-bound literal, case-sensitive search of indexed heading content.
/// The six level indexes skip excluded headings and body text. Fold visibility
/// does not affect search; use the returned ID/source for explicit navigation.
///
/// Construction prepares the query once in O(query bytes) time/space. Switching
/// headings reuses that storage in O(1); candidate lookup costs O(log(H + 1)).
/// Reads and matching share DisplaySearch's bounded, resumable work contract.
pub struct HeadingSearch {
    search: DisplaySearch,
    max_level: u8,
    next_heading: usize,
    current: Option<HeadingId>,
    complete: bool,
}

impl HeadingSearch {
    /// Select levels <= max_level, clamped to 6; zero selects none, just like
    /// HeadingIndex::next. Empty queries return InvalidRange. Does not read titles.
    pub fn new(document: &Document, query: &str, max_level: u8) -> Result<Self, LayoutError> {
        Ok(Self {
            search: DisplaySearch::new(document, query)?,
            max_level: max_level.min(6),
            next_heading: 0,
            current: None,
            complete: max_level == 0 || document.headings().entries().is_empty(),
        })
    }

    pub fn revision(&self) -> u64 {
        self.search.revision()
    }

    /// Heading currently being read/matched, or None between candidates/at EOF.
    pub fn current_heading(&self) -> Option<HeadingId> {
        self.current
    }

    /// Absolute read position within current_heading, possibly ahead of pending
    /// matching. None between headings or after completion; not a screen position.
    pub fn source_position(&self) -> Option<usize> {
        self.current.map(|_| self.search.source_position())
    }

    /// Append occurrences in heading/source order. Each candidate lookup costs
    /// one query and one object, including a lookup finding no eligible heading.
    /// Reading, comparisons and match emission use DisplaySearch's charges. Six
    /// bytes, one query and one object per call suffice for eventual progress.
    ///
    /// EndOfDocument means all selected headings in this revision were searched,
    /// even before finish. Any revision change invalidates the task AND returned
    /// matches. Wrong/stale documents are rejected before budget/output changes;
    /// rebuilding a task and discarding old matches is the caller's responsibility.
    pub fn advance(
        &mut self,
        document: &Document,
        budget: &mut Budget,
        out: &mut Vec<HeadingMatch>,
    ) -> Result<Completion, LayoutError> {
        self.search.validate(document)?;
        loop {
            if self.complete {
                return Ok(Completion::EndOfDocument);
            }
            let Some(heading) = self.current else {
                if !budget.charge(WorkLimit {
                    queries: 1,
                    objects: 1,
                    ..Default::default()
                }) {
                    return Ok(Completion::BudgetExhausted);
                }
                let next = (1..=self.max_level)
                    .filter_map(|level| {
                        let ids = document.headings().at_level(level);
                        ids.get(ids.partition_point(|id| id.0 < self.next_heading))
                            .copied()
                    })
                    .min();
                if let Some(id) = next {
                    let content = document.headings().get(id).unwrap().content;
                    self.search.reset(document.display_cursor(content).unwrap());
                    self.current = Some(id);
                } else {
                    self.complete = true;
                }
                continue;
            };
            let completion = self.search.advance_with(document, budget, |hit| {
                out.push(HeadingMatch {
                    heading,
                    display: hit.display,
                    source: hit.source,
                });
            })?;
            if completion == Completion::BudgetExhausted {
                return Ok(completion);
            }
            self.current = None;
            self.next_heading = heading.0 + 1;
            self.complete = self.next_heading == document.headings().entries().len();
        }
    }
}
