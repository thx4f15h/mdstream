use crate::{
    Budget, Completion, Document, DocumentId, FoldAnchor, FoldId, FoldNavigation,
    FoldNavigationPolicy, FoldNavigationProgress, FoldView, LayoutError, NavAction, Navigator,
};

/// A document-local return position, independent of revision, heights and folds.
/// Capture the renderer's exact source, not an estimated tile start. A bookmark
/// is reusable across views of the same Document, but not a reloaded document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bookmark {
    anchor: FoldAnchor,
    target: Option<FoldId>,
    viewport_offset: u64,
}

impl Bookmark {
    /// Capture the source and its innermost fold ID without reading text.
    /// O(log blocks + log headings), O(1) space. EOF and empty input are valid.
    /// viewport_offset is the source's row below the viewport top, not a global
    /// row or tile offset. The host reapplies it after local source-to-row layout.
    pub fn new(
        document: &Document,
        source: usize,
        viewport_offset: u64,
    ) -> Result<Self, LayoutError> {
        Ok(Self {
            anchor: FoldAnchor::new(document, source)?,
            target: document
                .fold_candidate_at(source)
                .map(|candidate| candidate.id),
            viewport_offset,
        })
    }

    pub fn anchor(&self) -> FoldAnchor {
        self.anchor
    }

    /// Metadata captured with the bookmark; late syntax confirmation may change
    /// the containing object. Restoration always uses the original exact source.
    pub fn target(&self) -> Option<FoldId> {
        self.target
    }

    pub fn viewport_offset(&self) -> u64 {
        self.viewport_offset
    }

    /// Compute the requested viewport top after the renderer locates the source
    /// row. Clamp the saved offset to the resized viewport so the source remains
    /// on screen. Navigator::reconcile_source then clamps to the document end.
    pub fn viewport_top(&self, source_row: u128, viewport_rows: u64) -> u128 {
        let offset = self.viewport_offset.min(viewport_rows.saturating_sub(1));
        source_row.saturating_sub(u128::from(offset))
    }

    /// Start a fresh budgeted restore after append/resize. Only folds hiding the
    /// retained source open. The returned row is an estimate; refine locally and
    /// use viewport_top before Navigator::reconcile_source.
    pub fn navigation<S>(
        &self,
        document: &Document,
        view: &FoldView<S>,
        viewport_rows: u64,
    ) -> Result<FoldNavigation, LayoutError> {
        if self.anchor.document_id() != document.id() {
            return Err(LayoutError::WrongDocument);
        }
        FoldNavigation::new(
            document,
            view,
            &Navigator::new(),
            NavAction::Source(self.anchor.source()),
            FoldNavigationPolicy::Reveal,
            viewport_rows,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryDirection {
    Back,
    Forward,
}

/// One view's linear navigation history. Entries contain no fold-tree snapshots.
/// Refresh the current bookmark after ordinary scrolling with replace_current;
/// push a destination only after a new jump succeeds. Back/forward moves commit
/// on successful task completion, so cancellation/errors leave the cursor alone.
pub struct NavigationHistory {
    identity: DocumentId,
    version: u64,
    entries: Vec<Bookmark>,
    position: usize,
}

impl NavigationHistory {
    pub fn new(initial: Bookmark) -> Self {
        Self {
            identity: DocumentId::default(),
            version: 0,
            entries: vec![initial],
            position: 0,
        }
    }

    pub fn entries(&self) -> &[Bookmark] {
        &self.entries
    }

    pub fn position(&self) -> usize {
        self.position
    }

    pub fn current(&self) -> Bookmark {
        self.entries[self.position]
    }

    /// Refresh the actual return position without creating a jump or discarding
    /// forward history. Changed entries invalidate pending history tasks.
    pub fn replace_current(&mut self, bookmark: Bookmark) -> Result<(), LayoutError> {
        self.check_document(bookmark.anchor.document_id())?;
        if self.current() != bookmark {
            self.entries[self.position] = bookmark;
            self.version += 1;
        }
        Ok(())
    }

    /// Record a successful new jump, discarding the forward branch. An exact
    /// duplicate is a no-op, preserving that branch and pending tasks. Amortized
    /// O(1); Vec growth can copy the retained entries. No source is scanned.
    pub fn push(&mut self, bookmark: Bookmark) -> Result<bool, LayoutError> {
        self.check_document(bookmark.anchor.document_id())?;
        if self.current() == bookmark {
            return Ok(false);
        }
        self.entries.truncate(self.position + 1);
        self.entries.push(bookmark);
        self.position += 1;
        self.version += 1;
        Ok(true)
    }

    /// O(1) construction, with all fold/height queries deferred to advance.
    /// None means the requested end of history has been reached.
    pub fn navigation<S>(
        &self,
        document: &Document,
        view: &FoldView<S>,
        direction: HistoryDirection,
        viewport_rows: u64,
    ) -> Result<Option<HistoryNavigation>, LayoutError> {
        self.check_document(document.id())?;
        view.check(document)?;
        let position = match direction {
            HistoryDirection::Back => self.position.checked_sub(1),
            HistoryDirection::Forward => {
                (self.position + 1 < self.entries.len()).then_some(self.position + 1)
            }
        };
        let Some(position) = position else {
            return Ok(None);
        };
        let bookmark = self.entries[position];
        Ok(Some(HistoryNavigation {
            history: self.identity,
            version: self.version,
            position,
            bookmark,
            navigation: bookmark.navigation(document, view, viewport_rows)?,
            committed: false,
        }))
    }

    fn check_document(&self, document: DocumentId) -> Result<(), LayoutError> {
        if self.current().anchor.document_id() != document {
            return Err(LayoutError::WrongDocument);
        }
        Ok(())
    }
}

/// A pending back/forward move. Dropping it cancels the history move, but folds
/// already opened stay open, just as for FoldNavigation. No history is cloned.
pub struct HistoryNavigation {
    history: DocumentId,
    version: u64,
    position: usize,
    bookmark: Bookmark,
    navigation: FoldNavigation,
    committed: bool,
}

impl HistoryNavigation {
    /// Retained destination metadata, including its viewport offset.
    pub fn bookmark(&self) -> Bookmark {
        self.bookmark
    }

    /// Uses FoldNavigation's shared query/object budget. The final locate step
    /// also pays for an O(1) history cursor update, with no allocation or scan.
    /// Wrong histories and changed entries are rejected before spending budget
    /// or opening folds. Repeating a completed task is idempotent until stale.
    pub fn advance<S>(
        &mut self,
        document: &Document,
        view: &mut FoldView<S>,
        history: &mut NavigationHistory,
        budget: &mut Budget,
    ) -> Result<FoldNavigationProgress, LayoutError> {
        if self.history != history.identity {
            return Err(LayoutError::WrongHistory);
        }
        if self.version != history.version {
            return Err(LayoutError::StaleContinuation);
        }
        let progress = self.navigation.advance(document, view, budget)?;
        if progress.completion == Completion::Covered && !self.committed {
            debug_assert!(progress.target.is_some());
            history.position = self.position;
            history.version += 1;
            self.version = history.version;
            self.committed = true;
        }
        Ok(progress)
    }
}
