use crate::{
    BlockKind, Budget, Completion, Document, DocumentId, FoldCandidates, FoldChange, FoldId,
    FoldView, LayoutError, Span, WorkLimit,
};

/// Explicit commands on candidates present at the task's document revision.
/// These do not change the view's defaults for future candidates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FoldBatchAction {
    All {
        collapsed: bool,
    },
    /// Includes the root and every descendant, even below collapsed parents.
    Subtree {
        root: FoldId,
        collapsed: bool,
    },
    /// Exactly this heading level (1..=6); code and other levels keep their choices.
    AtLevel {
        level: u8,
        collapsed: bool,
    },
    /// Expand the target and its ancestors; collapse every other candidate.
    /// Descendant choices are explicitly replaced, unlike single-node toggles.
    Focus {
        target: FoldId,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FoldBatchProgress {
    pub completion: Completion,
    /// Cumulative blocks inspected, including blocks without a fold candidate.
    pub visited: usize,
    /// Cumulative candidates whose local collapsed choice changed.
    pub changed: usize,
    /// Envelope of changes made by this call, with the latest view version.
    pub change: Option<FoldChange>,
}

/// Resumable, non-atomic batch editing. Intermediate choices are immediately
/// usable by layout. Dropping/invalidating the task keeps completed changes.
/// Storage is O(1); each budgeted step visits at most one block. Appends, reset,
/// defaults changes and external fold operations invalidate the continuation.
pub struct FoldBatch {
    document: DocumentId,
    revision: u64,
    view: DocumentId,
    version: u64,
    action: FoldBatchAction,
    candidates: Option<FoldCandidates>,
    focus: usize,
    visited: usize,
    changed: usize,
}

impl FoldBatch {
    /// O(1) validation and construction. Range discovery is budgeted in advance.
    pub fn new<S>(
        document: &Document,
        view: &FoldView<S>,
        action: FoldBatchAction,
    ) -> Result<Self, LayoutError> {
        view.check(document)?;
        match action {
            FoldBatchAction::AtLevel { level, .. } if !(1..=6).contains(&level) => {
                return Err(LayoutError::InvalidRange);
            }
            FoldBatchAction::Subtree { root: id, .. } | FoldBatchAction::Focus { target: id } => {
                let valid = match id {
                    FoldId::Heading(id) => document.headings().get(id).is_some(),
                    FoldId::Code(id) => document
                        .block(id)
                        .is_some_and(|block| matches!(block.kind, BlockKind::Code { .. })),
                };
                if !valid {
                    return Err(LayoutError::InvalidRange);
                }
            }
            _ => {}
        }
        Ok(Self {
            document: document.id(),
            revision: document.revision(),
            view: view.heights().version().0,
            version: view.version(),
            action,
            candidates: None,
            focus: 0,
            visited: 0,
            changed: 0,
        })
    }

    /// One query and one object pay for initialization, or for inspecting one
    /// block and changing its candidate. Each step uses a fixed number of index
    /// operations; neither source bytes nor covered tiles are enumerated.
    pub fn advance<S>(
        &mut self,
        document: &Document,
        view: &mut FoldView<S>,
        budget: &mut Budget,
    ) -> Result<FoldBatchProgress, LayoutError> {
        if self.document != document.id() {
            return Err(LayoutError::WrongDocument);
        }
        if self.view != view.heights().version().0 {
            return Err(LayoutError::WrongView);
        }
        if self.revision != document.revision() || self.version != view.version() {
            return Err(LayoutError::StaleContinuation);
        }
        view.check(document)?;
        let mut change: Option<FoldChange> = None;
        loop {
            if self
                .candidates
                .as_ref()
                .is_some_and(FoldCandidates::is_done)
            {
                return Ok(self.progress(Completion::Covered, change));
            }
            if !budget.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            }) {
                return Ok(self.progress(Completion::BudgetExhausted, change));
            }
            let Some(candidates) = &mut self.candidates else {
                let range = match self.action {
                    FoldBatchAction::Subtree { root, .. } => {
                        document.fold_candidate(root).unwrap().source
                    }
                    _ => Span::new(0, document.source().len()),
                };
                if let FoldBatchAction::Focus { target } = self.action {
                    self.focus = document.fold_candidate(target).unwrap().source.start;
                }
                self.candidates = Some(document.fold_candidates(range)?);
                continue;
            };
            self.visited += 1;
            let Some(candidate) = candidates.next_block(document) else {
                continue;
            };
            let collapsed = match self.action {
                FoldBatchAction::All { collapsed } | FoldBatchAction::Subtree { collapsed, .. } => {
                    collapsed
                }
                FoldBatchAction::AtLevel { level, collapsed } => {
                    let FoldId::Heading(id) = candidate.id else {
                        continue;
                    };
                    if document.headings().get(id).unwrap().level != level {
                        continue;
                    }
                    collapsed
                }
                FoldBatchAction::Focus { .. } => !candidate.source.range().contains(&self.focus),
            };
            if let Some(applied) = view.set_collapsed(document, candidate.id, collapsed)? {
                self.changed += 1;
                self.version = view.version();
                change = Some(FoldChange {
                    version: applied.version,
                    source: change.map_or(applied.source, |old| {
                        Span::new(
                            old.source.start.min(applied.source.start),
                            old.source.end.max(applied.source.end),
                        )
                    }),
                });
            }
        }
    }

    fn progress(&self, completion: Completion, change: Option<FoldChange>) -> FoldBatchProgress {
        FoldBatchProgress {
            completion,
            visited: self.visited,
            changed: self.changed,
            change,
        }
    }
}
