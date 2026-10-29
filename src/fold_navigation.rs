use crate::folding::FoldSegment;
use crate::{
    Budget, Completion, Document, DocumentId, FoldChange, FoldId, FoldView, LayoutError, NavAction,
    Navigator, ScrollTarget, Span, WorkLimit,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FoldNavigationPolicy {
    /// Open only folds hiding the destination. A heading's own body stays folded.
    #[default]
    Reveal,
    /// Sequential next/previous (including exact-level) actions skip whole hidden
    /// ranges. Other source/heading actions return no target if it is hidden.
    VisibleOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FoldNavigationProgress {
    /// Covered means the action finished, including when there was no target.
    pub completion: Completion,
    /// Estimated row and optional exact source, only on successful completion.
    pub target: Option<ScrollTarget>,
    /// Envelope of folds opened by this call, with the latest view version.
    pub change: Option<FoldChange>,
}

enum Step {
    Start,
    Inspect(usize),
    Locate(usize),
    Done(Option<ScrollTarget>),
}

/// Navigation from a snapshot of a Navigator, without changing that navigator.
/// Apply the completed target with Navigator::apply_target. Drop the task to
/// cancel; folds already opened remain open. No fold-tree snapshots are stored.
/// Document/view changes invalidate the task, except its own path expansions
/// and height measurements. Final positioning uses the current measured heights;
/// repeated calls after completion return the saved result.
pub struct FoldNavigation {
    document: DocumentId,
    revision: u64,
    view: DocumentId,
    version: u64,
    origin: Navigator,
    action: NavAction,
    policy: FoldNavigationPolicy,
    viewport_rows: u64,
    step: Step,
}

impl FoldNavigation {
    /// O(1) construction; heading, visibility and height queries are deferred
    /// until advance so they share the caller's frame budget.
    pub fn new<S>(
        document: &Document,
        view: &FoldView<S>,
        navigator: &Navigator,
        action: NavAction,
        policy: FoldNavigationPolicy,
        viewport_rows: u64,
    ) -> Result<Self, LayoutError> {
        view.check(document)?;
        if let NavAction::Source(source) = action
            && !document.source().is_char_boundary(source)
        {
            return Err(LayoutError::InvalidRange);
        }
        Ok(Self {
            document: document.id(),
            revision: document.revision(),
            view: view.heights().version().0,
            version: view.version(),
            origin: navigator.clone(),
            action,
            policy,
            viewport_rows,
            step: Step::Start,
        })
    }

    /// Each bounded logical step costs one query and one object: resolving an
    /// action, checking visibility and opening/skipping one enclosing fold, or
    /// locating the final estimated row. No body scan or tile layout is performed;
    /// source boundary validation only needs a constant-time UTF-8 boundary check.
    /// At most six heading ancestors and one code fold can hide a destination.
    /// Visible-only traversal resumes after each skipped range, not at its start.
    pub fn advance<S>(
        &mut self,
        document: &Document,
        view: &mut FoldView<S>,
        budget: &mut Budget,
    ) -> Result<FoldNavigationProgress, LayoutError> {
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
            if let Step::Done(target) = self.step {
                return Ok(FoldNavigationProgress {
                    completion: Completion::Covered,
                    target,
                    change,
                });
            }
            if !budget.charge(navigation_cost()) {
                return Ok(FoldNavigationProgress {
                    completion: Completion::BudgetExhausted,
                    target: None,
                    change,
                });
            }
            self.step = match self.step {
                Step::Start => {
                    let target = self.origin.navigate(
                        document,
                        view.heights(),
                        self.action,
                        self.viewport_rows,
                    );
                    match target.and_then(|t| t.source) {
                        Some(source) => Step::Inspect(source),
                        None => Step::Done(target),
                    }
                }
                Step::Inspect(source) => match view.state.segment(document, source) {
                    FoldSegment::Text { .. } => Step::Locate(source),
                    FoldSegment::Hidden(candidate) => match self.policy {
                        FoldNavigationPolicy::Reveal => {
                            let opened = view
                                .set_collapsed(document, candidate.id, false)?
                                .expect("hidden fold is collapsed");
                            self.version = view.version();
                            change = Some(FoldChange {
                                version: opened.version,
                                source: change.map_or(opened.source, |old| {
                                    Span::new(
                                        old.source.start.min(opened.source.start),
                                        old.source.end.max(opened.source.end),
                                    )
                                }),
                            });
                            Step::Inspect(source)
                        }
                        FoldNavigationPolicy::VisibleOnly => {
                            // next() is strictly after its bound; include a
                            // heading starting exactly at the hidden range end.
                            let before_end = candidate.hidden.end - 1;
                            let headings = document.headings();
                            let next = match self.action {
                                NavAction::NextHeading { max_level } => {
                                    headings.next(before_end, max_level)
                                }
                                NavAction::PreviousHeading { max_level } => {
                                    headings.previous(candidate.hidden.start, max_level)
                                }
                                NavAction::NextAtLevel { level } => {
                                    headings.next_at_level(before_end, level)
                                }
                                NavAction::PreviousAtLevel { level } => {
                                    headings.previous_at_level(candidate.hidden.start, level)
                                }
                                _ => None,
                            };
                            next.map_or(Step::Done(None), |id| {
                                Step::Inspect(headings.get(id).unwrap().source.start)
                            })
                        }
                    },
                },
                Step::Locate(source) => Step::Done(self.origin.navigate(
                    document,
                    view.heights(),
                    NavAction::Source(source),
                    self.viewport_rows,
                )),
                Step::Done(_) => unreachable!(),
            };
        }
    }
}

/// A document-bound source position retained while its visible projection moves
/// to a fold placeholder. Resolve again after expansion/append/resize to recover
/// the original source. At an appended boundary this has left affinity: its byte
/// offset stays fixed. It stores no layout or fold-tree history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FoldAnchor {
    document: DocumentId,
    source: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FoldAnchorTarget {
    /// Placeholder source when hidden, otherwise the retained original source.
    pub target: ScrollTarget,
    pub hidden_by: Option<FoldId>,
}

impl FoldAnchor {
    pub fn new(document: &Document, source: usize) -> Result<Self, LayoutError> {
        if !document.source().is_char_boundary(source) {
            return Err(LayoutError::InvalidRange);
        }
        Ok(Self {
            document: document.id(),
            source,
        })
    }

    pub fn source(&self) -> usize {
        self.source
    }

    /// None means budget exhaustion with no side effects. One query and one
    /// object cover the bounded ancestor lookup and estimated height lookup.
    /// The renderer refines the returned source/placeholder row locally.
    pub fn resolve<S>(
        &self,
        document: &Document,
        view: &FoldView<S>,
        viewport_rows: u64,
        budget: &mut Budget,
    ) -> Result<Option<FoldAnchorTarget>, LayoutError> {
        if self.document != document.id() {
            return Err(LayoutError::WrongDocument);
        }
        view.check(document)?;
        if !budget.charge(navigation_cost()) {
            return Ok(None);
        }
        let (source, hidden_by) = match view.state.segment(document, self.source) {
            FoldSegment::Hidden(candidate) => (candidate.hidden.start, Some(candidate.id)),
            FoldSegment::Text { .. } => (self.source, None),
        };
        let target = Navigator::new()
            .navigate(
                document,
                view.heights(),
                NavAction::Source(source),
                viewport_rows,
            )
            .expect("validated source or placeholder boundary");
        Ok(Some(FoldAnchorTarget { target, hidden_by }))
    }
}

fn navigation_cost() -> WorkLimit {
    WorkLimit {
        queries: 1,
        objects: 1,
        ..Default::default()
    }
}
