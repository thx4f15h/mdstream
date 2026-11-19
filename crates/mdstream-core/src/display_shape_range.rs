//! Budgeted current semantic metadata for one source range. Unlike the complete
//! adoption seed, this cursor does not supply an incremental rollback baseline.
use crate::{
    Budget, Completion, DisplayShapeSeedItem as Item, DisplayShapeSeedWork, Document, DocumentId,
    FootnoteReferenceId, LayoutError, LinkStep, LinkWindow, Span, WorkLimit,
};

/// Revision-bound current metadata for a caller-selected source range.
///
/// This supplies no rollback baseline and cannot initialize an incremental
/// renderer. Marks are clipped byte-tree intervals; replacements and objects
/// retain their full intersecting source extents. The conservative Candidate
/// covers the requested range; resolve styles with the native style indexes.
/// HTML classification and opener qualification are separate metadata queries.
pub struct DisplayShapeRangeSeed {
    document: DocumentId,
    revision: u64,
    source: Span,
    phase: u8,
    annotations: crate::annotations::ShapeRange,
    at: usize,
    links: LinkWindow,
    footnote: Option<FootnoteReferenceId>,
    footnote_started: bool,
    steps: usize,
}
impl Document {
    /// A current-state metadata range, with no source decoding or prefix walk.
    /// Hold the revision fixed until completion. This is a read-only range seed,
    /// not the full-seed contract for consuming later incremental effects.
    /// Cost follows the requested metadata range, not the viewport dimensions;
    /// selecting a full document still requests a full metadata traversal.
    pub fn display_shape_range_seed(
        &self,
        source: Span,
    ) -> Result<DisplayShapeRangeSeed, LayoutError> {
        self.text(source).ok_or(LayoutError::InvalidRange)?;
        Ok(DisplayShapeRangeSeed {
            document: self.id(),
            revision: self.revision(),
            source,
            phase: if source.is_empty() { 7 } else { 0 },
            annotations: crate::annotations::ShapeRange::new(),
            at: 0,
            links: self.link_window(source),
            footnote: None,
            footnote_started: false,
            steps: 0,
        })
    }
}
impl Document {
    /// Qualify an already input-classified HTML opener using current native
    /// annotation/replacement indexes, including context before a query range.
    pub fn display_shape_html_qualified(
        &self,
        at: usize,
        budget: &mut Budget,
    ) -> Result<Result<bool, Completion>, LayoutError> {
        if at >= self.source().len() || !self.source().is_char_boundary(at) {
            return Err(LayoutError::InvalidRange);
        }
        if !budget.charge(WorkLimit {
            queries: 1,
            objects: 1,
            ..Default::default()
        }) {
            return Ok(Err(Completion::BudgetExhausted));
        }
        let mark = self.annotations.current_leaf(at, at + 1).1;
        let replacement = self
            .display
            .shape_seed_entry(self.display.shape_seed_start(at))
            .is_some_and(|(s, _)| s.start <= at && at < s.end);
        Ok(Ok(!replacement
            && !mark.style.contains(crate::Style::CODE)
            && !mark.base_hidden
            && mark.hide_owner().is_none()))
    }
}

impl DisplayShapeRangeSeed {
    /// Actual annotation traversal and charged cursor steps, including the
    /// delegated link/footnote cursor work; no source decoding is performed.
    pub fn work(&self) -> DisplayShapeSeedWork {
        let mut work = self.annotations.work;
        work.steps += self.steps;
        work
    }
    /// Resume on the same document revision, sharing the caller's Budget.
    /// An empty valid range completes without work, after identity checks.
    pub fn advance(
        &mut self,
        document: &Document,
        budget: &mut Budget,
    ) -> Result<Result<Option<Item>, Completion>, LayoutError> {
        if self.document != document.id() {
            return Err(LayoutError::WrongDocument);
        }
        if self.revision != document.revision() {
            return Err(LayoutError::StaleContinuation);
        }
        loop {
            if self.phase == 0 {
                match self
                    .annotations
                    .next(&document.annotations, self.source, budget)
                {
                    Ok(Some(item)) => return Ok(Ok(Some(item))),
                    Err(completion) => return Ok(Err(completion)),
                    Ok(None) => {
                        self.phase = 1;
                    }
                }
            }
            if self.phase == 5 {
                let before = budget.used();
                let next = self.links.advance(document, budget)?;
                let after = budget.used();
                self.steps += after.queries - before.queries + after.objects - before.objects;
                match next {
                    LinkStep::Object(id, info) => {
                        return Ok(Ok(Some(Item::Link {
                            id,
                            info: *info,
                            active: document.link_order.active(id),
                        })));
                    }
                    LinkStep::BudgetExhausted => return Ok(Err(Completion::BudgetExhausted)),
                    LinkStep::Complete => {
                        self.phase = 6;
                    }
                }
            }
            if self.phase == 6 && !self.footnote_started {
                let mut refs = document.footnote_references_in(self.source).unwrap();
                let next = refs.advance(budget);
                if next.is_ok() {
                    self.steps += 1;
                }
                match next {
                    Ok(Some((id, reference))) => {
                        self.footnote_started = true;
                        self.footnote = document.next_footnote_reference(id);
                        return Ok(Ok(Some(Item::FootnoteReference {
                            id,
                            source: reference.source,
                        })));
                    }
                    Ok(None) => {
                        self.phase = 7;
                    }
                    Err(completion) => return Ok(Err(completion)),
                }
            }
            if self.phase == 7 {
                return Ok(Ok(None));
            }
            if !budget.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            }) {
                return Ok(Err(Completion::BudgetExhausted));
            }
            self.steps += 1;
            match self.phase {
                1 => {
                    self.at = document.display.shape_seed_start(self.source.start);
                    self.phase = 2;
                }
                2 => {
                    if let Some((source, replacement)) = document.display.shape_seed_entry(self.at)
                        && source.start < self.source.end
                    {
                        self.at += 1;
                        return Ok(Ok(Some(Item::Replacement {
                            source,
                            replacement,
                        })));
                    }
                    self.at = document
                        .display
                        .space_mappings()
                        .partition_point(|(s, _)| s.end <= self.source.start);
                    self.phase = 3;
                }
                3 => {
                    if let Some(&(source, columns)) = document.display.space_mappings().get(self.at)
                        && source.start < self.source.end
                    {
                        self.at += 1;
                        return Ok(Ok(Some(Item::Replacement {
                            source,
                            replacement: crate::DisplayShapeReplacement::Spaces { columns },
                        })));
                    }
                    self.phase = 4;
                }
                4 => {
                    self.phase = 5;
                    // Existing native style queries carry preceding-pair and
                    // conditional-scope state; never enumerate old style events.
                    return Ok(Ok(Some(Item::Candidate {
                        source: self.source,
                        conditional_scope: None,
                    })));
                }
                6 => {
                    if let Some(id) = self.footnote {
                        let reference = document.footnote_references()[id.0];
                        if reference.source.start < self.source.end {
                            self.footnote = document.next_footnote_reference(id);
                            return Ok(Ok(Some(Item::FootnoteReference {
                                id,
                                source: reference.source,
                            })));
                        }
                        self.phase = 7;
                    } else {
                        self.phase = 7;
                    }
                }
                _ => unreachable!(),
            }
        }
    }
}
