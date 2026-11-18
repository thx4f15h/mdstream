use crate::folding::{FoldSegment, FoldState};
use crate::{
    BlockId, Budget, CheckpointToken, Document, DocumentId, FoldCandidate, FoldPlaceholder,
    FoldView, LayoutError, Span, TileId, WorkLimit,
};

/// One metadata-only step before display scanning and block preparation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FoldProjectionStep {
    /// Unfolded source, including ordinary syntax-hidden bytes. Decode it with
    /// the existing display cursor; this is not rendered or normalized text.
    Text {
        source: Span,
        tile: TileId,
    },
    /// Emitted once at the component's actual start, never for a remote start
    /// inside it. The candidate's hidden end is a frontier, not a final union.
    Placeholder(FoldPlaceholder),
    /// A paid jump through a hidden frontier. `source` is the skipped part of
    /// this request; candidate metadata retains the core's component identity.
    /// Crossing components can require several steps with the same owner.
    Hidden {
        source: Span,
        candidate: FoldCandidate,
    },
    /// State-only reset, including a start inside a paragraph's hidden suffix.
    /// Do not add height here: its placeholder owns the terminating line.
    ParagraphEnd(BlockId),
    Complete,
    BudgetExhausted,
}

/// Resumable, O(1) fold geometry for a fixed source cut. Does not read body text,
/// enumerate hidden tiles, collect events, mutate heights or publish measurements.
/// The host bounds its output window and separately budgets display/summary work.
/// Text is clipped at tile/fold/request boundaries; hidden frontiers jump directly.
pub struct FoldProjection {
    document: DocumentId,
    revision: u64,
    view: DocumentId,
    version: u64,
    requested: Span,
    cursor: usize,
    recovery_until: usize,
    checkpoint: Option<CheckpointToken>,
    folds: FoldCursor,
}

impl FoldProjection {
    /// Both cuts must be existing UTF-8 boundaries. Construction only validates
    /// fixed metadata/endpoints; all index traversal happens in `next`.
    pub fn new<S>(
        document: &Document,
        view: &FoldView<S>,
        source: Span,
    ) -> Result<Self, LayoutError> {
        view.check(document)?;
        if source.start > source.end
            || source.end > document.source().len()
            || !document.source().is_char_boundary(source.start)
            || !document.source().is_char_boundary(source.end)
        {
            return Err(LayoutError::InvalidRange);
        }
        Ok(Self {
            document: document.id(),
            revision: document.revision(),
            view: view.heights().version().0,
            version: view.version(),
            requested: source,
            cursor: source.start,
            recovery_until: source.start,
            checkpoint: None,
            folds: FoldCursor::default(),
        })
    }

    /// Replay from this view's exact input checkpoint within `max_bytes` of
    /// `source.start`. The caller restores its payload before consuming events.
    /// Events before the requested cut are recovery, not screen output; each
    /// such event pays one recovery unit. The token lookup costs one query;
    /// None means no checkpoint, too distant, or insufficient lookup budget.
    /// Invalidated/replaced tokens reject the continuation even when document
    /// and fold choices are unchanged.
    pub fn from_checkpoint<S>(
        document: &Document,
        view: &FoldView<S>,
        source: Span,
        input: TileId,
        max_bytes: usize,
        budget: &mut Budget,
    ) -> Result<Option<Self>, LayoutError> {
        let mut task = Self::new(document, view, source)?;
        let from = document.tile(input).ok_or(LayoutError::InvalidRange)?;
        if from.start > source.start {
            return Err(LayoutError::InvalidRange);
        }
        if source.start - from.start > max_bytes {
            return Ok(None);
        }
        if !budget.charge(WorkLimit {
            queries: 1,
            ..Default::default()
        }) {
            return Ok(None);
        }
        let Some(token) = view.checkpoints().token(input) else {
            return Ok(None);
        };
        task.cursor = from.start;
        task.checkpoint = Some(token);
        Ok(Some(task))
    }

    pub fn requested(&self) -> Span {
        self.requested
    }

    pub fn source_position(&self) -> usize {
        self.cursor
    }

    /// Validate an owned preparation continuation without advancing geometry.
    /// This also lets a host reject changes after it has collected its window.
    pub fn check_binding<S>(
        &self,
        document: &Document,
        view: &FoldView<S>,
    ) -> Result<(), LayoutError> {
        if self.document != document.id() {
            return Err(LayoutError::WrongDocument);
        }
        if self.view != view.heights().version().0 {
            return Err(LayoutError::WrongView);
        }
        if self.revision != document.revision() || self.version != view.version() {
            return Err(LayoutError::StaleContinuation);
        }
        view.check(document)
    }

    /// Share one frame Budget with queries, display scanning and measurement.
    /// A segment lookup costs 1 query; each event costs 1 query/1 object, and a
    /// Text event also costs 1 tile. Recovery steps additionally cost 1 recovery.
    /// Bytes are zero: no body run or summary is read. A one-query frame can
    /// retain the lookup and emit the event on a subsequent frame. Complete is
    /// free for an unrestored task. Restored calls first pay one query for
    /// token validation plus their fixed geometry lookup/emission, including
    /// Complete or a stale-token error; they do not pay a second query. Document,
    /// view and fold-version errors are rejected before consuming budget.
    pub fn next<S>(
        &mut self,
        document: &Document,
        view: &FoldView<S>,
        budget: &mut Budget,
    ) -> Result<FoldProjectionStep, LayoutError> {
        self.check_binding(document, view)?;
        let queries = if let Some(token) = self.checkpoint {
            if !budget.charge(WorkLimit {
                queries: 1,
                ..Default::default()
            }) {
                return Ok(FoldProjectionStep::BudgetExhausted);
            }
            if !view.checkpoints().validates(token) {
                return Err(LayoutError::StaleContinuation);
            }
            0
        } else {
            1
        };
        let recovery = usize::from(self.cursor < self.recovery_until);
        let cost = WorkLimit {
            queries,
            objects: 1,
            recovery,
            ..Default::default()
        };
        match self.folds.paragraph_end(budget, cost) {
            Err(()) => return Ok(FoldProjectionStep::BudgetExhausted),
            Ok(Some(block)) => return Ok(FoldProjectionStep::ParagraphEnd(block)),
            Ok(None) => {}
        }
        if self.cursor == self.requested.end {
            return Ok(FoldProjectionStep::Complete);
        }
        let Ok(segment) = self.folds.query(
            document,
            &view.state,
            self.cursor,
            budget,
            WorkLimit {
                queries,
                // A restored query already paid for its fixed metadata work.
                recovery: if self.checkpoint.is_some() {
                    0
                } else {
                    recovery
                },
                ..Default::default()
            },
        ) else {
            return Ok(FoldProjectionStep::BudgetExhausted);
        };
        match segment {
            FoldSegment::Text { end } => {
                let tile = document.tile_at(self.cursor).unwrap();
                let end = end
                    .min(document.tile(tile).unwrap().end)
                    .min(self.requested.end)
                    .min(if recovery > 0 {
                        self.recovery_until
                    } else {
                        self.requested.end
                    });
                if !budget.charge(WorkLimit { tiles: 1, ..cost }) {
                    return Ok(FoldProjectionStep::BudgetExhausted);
                }
                let source = Span::new(self.cursor, end);
                self.cursor = end;
                Ok(FoldProjectionStep::Text { source, tile })
            }
            FoldSegment::Hidden(candidate) => {
                let placeholder = FoldPlaceholder {
                    candidate,
                    owner: document.tile_at(candidate.hidden.start).unwrap(),
                    paragraph: view.state.paragraph(candidate.id),
                };
                match self
                    .folds
                    .placeholder(self.cursor, placeholder, budget, cost)
                {
                    Err(()) => return Ok(FoldProjectionStep::BudgetExhausted),
                    Ok(true) => return Ok(FoldProjectionStep::Placeholder(placeholder)),
                    Ok(false) => {}
                }
                if !budget.charge(cost) {
                    return Ok(FoldProjectionStep::BudgetExhausted);
                }
                let end = candidate
                    .hidden
                    .end
                    .min(self.requested.end)
                    .min(if recovery > 0 {
                        self.recovery_until
                    } else {
                        self.requested.end
                    });
                let source = Span::new(self.cursor, end);
                self.cursor = end;
                self.folds.skipped(&view.state, candidate, end);
                Ok(FoldProjectionStep::Hidden { source, candidate })
            }
        }
    }
}

// Shared by the metadata projection and the real LayoutPass. All component
// lookup, placeholder delivery and once-only paragraph recovery use this state;
// neither consumer implements its own union/ownership or paragraph-end rules.
#[derive(Default)]
pub(crate) struct FoldCursor {
    pub(crate) segment: Option<FoldSegment>,
    placeholder: Option<usize>,
    pending_end: Option<BlockId>,
    ended: Option<BlockId>,
}

impl FoldCursor {
    pub(crate) fn query(
        &mut self,
        document: &Document,
        folds: &FoldState,
        source: usize,
        budget: &mut Budget,
        cost: WorkLimit,
    ) -> Result<FoldSegment, ()> {
        if self.segment.is_none_or(|s| source >= s.end()) {
            if !budget.charge(cost) {
                return Err(());
            }
            self.segment = Some(folds.segment(document, source));
        }
        Ok(self.segment.unwrap())
    }

    pub(crate) fn placeholder(
        &mut self,
        source: usize,
        placeholder: FoldPlaceholder,
        budget: &mut Budget,
        cost: WorkLimit,
    ) -> Result<bool, ()> {
        let start = placeholder.candidate.hidden.start;
        if source != start || self.placeholder == Some(start) {
            return Ok(false);
        }
        if !budget.charge(cost) {
            return Err(());
        }
        self.placeholder = Some(start);
        Ok(true)
    }

    pub(crate) fn skipped(&mut self, folds: &FoldState, candidate: FoldCandidate, end: usize) {
        if end == candidate.hidden.end {
            self.pending_end = folds.paragraph(candidate.id);
        }
    }

    pub(crate) fn paragraph_end(
        &mut self,
        budget: &mut Budget,
        cost: WorkLimit,
    ) -> Result<Option<BlockId>, ()> {
        let Some(block) = self.pending_end else {
            return Ok(None);
        };
        if self.ended == Some(block) {
            self.pending_end = None;
            return Ok(None);
        }
        if !budget.charge(cost) {
            return Err(());
        }
        self.pending_end = None;
        self.ended = Some(block);
        Ok(Some(block))
    }
}
