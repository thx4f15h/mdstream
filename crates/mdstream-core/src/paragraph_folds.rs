use crate::{
    BlockId, Budget, Completion, ContainerId, Document, DocumentId, FoldChange, FoldView,
    LayoutError, SourceFoldRegistration, Span, Update, WorkLimit,
};

/// One logical paragraph eligible for source-prefix registration at this
/// revision. This is document metadata, not a registered/view-local fold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParagraphFoldCandidate {
    /// First physical leaf; stable across ordinary continuation and closure.
    pub block: BlockId,
    pub container: Option<ContainerId>,
    /// Whole logical paragraph, including provisional trailing input when open.
    pub source: Span,
    /// Exclusive end of classified source. A registration's preview_bytes must
    /// be strictly less than classified_end - source.start, before rounding.
    /// Counts raw bytes, including syntax, container prefixes and line endings.
    pub classified_end: usize,
    pub open: bool,
}

impl Document {
    /// Logical paragraph containing an offset, including its physical container
    /// leaves and provisional open tail. Uses the existing first-leaf/end index
    /// in O(log B + log P), without reading source or walking container ancestry.
    /// Like other metadata snapshots, the result belongs to this revision.
    pub fn paragraph_at(&self, offset: usize) -> Option<ParagraphFoldCandidate> {
        let leaf = self.blocks.partition_point(|b| b.source.end <= offset);
        self.paragraph_fold_covering(offset, leaf).or_else(|| {
            let entry = self.blocks.get(leaf)?;
            (entry.source.start <= offset && offset < entry.source.end)
                .then(|| self.paragraph_fold_snapshot(BlockId(leaf)))
                .flatten()
        })
    }

    /// Current first-leaf paragraph metadata; O(log P), without reading body bytes.
    pub fn paragraph_fold_snapshot(&self, block: BlockId) -> Option<ParagraphFoldCandidate> {
        let (source, open) = self.open_paragraph_source(block)?;
        let entry = self.block(block)?;
        let classified_end = if !open {
            source.end
        } else if entry.container.is_some() {
            self.blocks[self.current?.0].source.end
        } else {
            entry.source.end
        };
        Some(ParagraphFoldCandidate {
            block,
            container: entry.container,
            source,
            classified_end,
            open,
        })
    }

    // The window can start inside any physical leaf or the unclassified tail.
    // Closed sequences use the existing first-leaf/end index; the sole active
    // sequence uses parser endpoints. Neither path walks leaves or ancestors.
    fn paragraph_fold_covering(
        &self,
        offset: usize,
        leaf: usize,
    ) -> Option<ParagraphFoldCandidate> {
        if let Some(first) = self
            .container_title
            .map(|(first, _)| first)
            .or(self.current)
            && let Some(candidate) = self.paragraph_fold_snapshot(first)
            && candidate.source.start <= offset
            && offset < candidate.source.end
        {
            return Some(candidate);
        }
        let (&first, &end) = self
            .container_paragraph_ends
            .range(..=BlockId(leaf))
            .next_back()?;
        if self.blocks[first.0].source.start <= offset && offset < end {
            self.paragraph_fold_snapshot(first)
        } else {
            None
        }
    }
}

/// Budgeted discovery of logical paragraphs intersecting a half-open source
/// window, including paragraphs starting before it. `min_bytes` filters the
/// whole classified source length, not display length or the window overlap.
/// Zero accepts every eligible nonempty paragraph. No source bytes are read.
///
/// Construction is O(1); each paid step inspects one logical paragraph or one
/// non-paragraph leaf (including rejected candidates/terminating lookups).
/// It uses O(log B + log P) indexed work, B physical blocks and P closed
/// container paragraphs. Multi-leaf paragraphs are skipped as a whole, even
/// below the threshold. Cursor storage is O(1); no parser indexes are added.
///
/// Open candidates include pending input but only classified bytes qualify
/// for the threshold/prefix. Late Setext/table confirmation can remove or
/// retract a candidate. Append/finish invalidates the cursor; restart at the
/// desired source window in the new revision, including its earlier paragraph.
/// Fold toggles, registration, preview and resize do not invalidate discovery.
/// Results include already registered/hidden paragraphs: choose and register
/// with SourceFoldRegistration separately, using its shared endpoint budget.
pub struct ParagraphFoldCandidates {
    document: DocumentId,
    revision: u64,
    next: usize,
    end: usize,
    min_bytes: usize,
}

impl ParagraphFoldCandidates {
    /// Windows are byte offsets; UTF-8 boundary alignment is unnecessary for
    /// metadata discovery. Empty windows return no containing paragraph.
    pub fn new(document: &Document, range: Span, min_bytes: usize) -> Result<Self, LayoutError> {
        if range.start > range.end || range.end > document.source().len() {
            return Err(LayoutError::InvalidRange);
        }
        Ok(Self {
            document: document.id(),
            revision: document.revision(),
            next: range.start,
            end: range.end,
            min_bytes,
        })
    }

    /// Clears/reuses out after validating document identity/revision. Charges
    /// one query and one object atomically per step, including non-matches;
    /// zero bytes/tiles/recovery. Exhaustion preserves position, completed
    /// calls are free. Output is source ordered, without duplicate leaves.
    pub fn advance(
        &mut self,
        document: &Document,
        budget: &mut Budget,
        out: &mut Vec<ParagraphFoldCandidate>,
    ) -> Result<Completion, LayoutError> {
        if self.document != document.id() {
            return Err(LayoutError::WrongDocument);
        }
        if self.revision != document.revision() {
            return Err(LayoutError::StaleContinuation);
        }
        out.clear();
        while self.next < self.end {
            if !budget.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            }) {
                return Ok(Completion::BudgetExhausted);
            }
            if let Some(candidate) = self.next_candidate(document) {
                out.push(candidate);
            }
        }
        Ok(Completion::Covered)
    }

    // One already-paid discovery step, shared by the automatic policy task.
    fn next_candidate(&mut self, document: &Document) -> Option<ParagraphFoldCandidate> {
        let leaf = document
            .blocks
            .partition_point(|b| b.source.end <= self.next);
        let candidate = document
            .paragraph_fold_covering(self.next, leaf)
            .or_else(|| {
                let entry = document.blocks.get(leaf)?;
                (entry.source.start < self.end)
                    .then(|| document.paragraph_fold_snapshot(BlockId(leaf)))
                    .flatten()
            });
        if let Some(candidate) = candidate {
            self.next = candidate.source.end;
            if candidate.classified_end > candidate.source.start
                && candidate.classified_end - candidate.source.start >= self.min_bytes
                && (candidate.open || candidate.source.end <= document.source_fold_limit())
            {
                return Some(candidate);
            }
        } else if let Some(entry) = document.blocks.get(leaf)
            && entry.source.start < self.end
        {
            self.next = entry.source.end;
        } else {
            self.next = self.end;
        }
        None
    }
}

/// Source-byte policy for automatic paragraph registration. The inclusive
/// threshold counts classified raw bytes, including syntax/prefixes/newlines.
/// A candidate must also leave a byte beyond preview_bytes before rounding.
/// Zero thresholds/prefixes are allowed; usize::MAX never overflows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParagraphFoldPolicy {
    pub min_bytes: usize,
    pub preview_bytes: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParagraphAutoFoldProgress {
    pub completion: Completion,
    /// Cumulative logical paragraphs/non-paragraph leaves inspected.
    pub inspected: usize,
    pub registered: usize,
    /// Qualifying candidates with an exclusion, existing/overlapping source
    /// registration, insufficient prefix suffix, or incompatible syntax range.
    pub skipped: usize,
    /// Envelope of new collapsed suffixes in this call, at the latest version.
    pub change: Option<FoldChange>,
}

/// Apply FoldDefaults::paragraphs to a source window with a shared Budget.
/// Each new eligible paragraph is atomically registered and initially collapsed.
/// Existing registrations (manual or automatic), choices and previews are kept.
/// Exclusions and overlaps are skipped, never removed or changed by this task.
/// min_bytes is an admission threshold, not a continuously enforced condition:
/// growth/closure preserves identity and explicit expansion; late contraction
/// keeps the registration while a suffix remains, even below the threshold.
/// Setext/table promotion or suffix removal uses FoldView::apply's usual removal.
///
/// Construction is O(1). One query/object pays for each discovery step, then
/// SourceFoldRegistration charges its two ancestor paths and final insertion.
/// The paid insertion also initializes the collapsed choice. No body scan or
/// unbudgeted ancestor walk; O(1) task storage. Completed registrations survive
/// task cancellation/invalidation; unfinished registration has no side effects.
///
/// Append/finish and external view changes invalidate this task. After applying
/// an update, use for_update to discover only affected paragraphs (including a
/// paragraph crossing the threshold). If work remains, retain remaining_range
/// and union it with the next update's window before restarting. There is no
/// hidden queue or automatic full-document scan in FoldView::apply/set_defaults.
pub struct ParagraphAutoFold {
    discovery: ParagraphFoldCandidates,
    view: DocumentId,
    version: u64,
    policy: Option<ParagraphFoldPolicy>,
    pending: Option<(usize, SourceFoldRegistration)>,
    inspected: usize,
    registered: usize,
    skipped: usize,
}

impl ParagraphAutoFold {
    pub fn new<S>(
        document: &Document,
        view: &FoldView<S>,
        range: Span,
    ) -> Result<Self, LayoutError> {
        view.check(document)?;
        let policy = view.defaults().paragraphs;
        let mut discovery =
            ParagraphFoldCandidates::new(document, range, policy.map_or(0, |p| p.min_bytes))?;
        if policy.is_none() {
            discovery.next = discovery.end;
        }
        Ok(Self {
            discovery,
            view: view.heights().version().0,
            version: view.version(),
            policy,
            pending: None,
            inspected: 0,
            registered: 0,
            skipped: 0,
        })
    }

    /// Call after view.apply. O(1) window from the first changed block through
    /// EOF, including finish with no appended bytes and merged notifications.
    /// Reference/style changes to unrelated history do not widen this window.
    /// Prior unfinished windows must be retained by the caller; replay is safe.
    pub fn for_update<S>(
        document: &Document,
        view: &FoldView<S>,
        update: &Update,
    ) -> Result<Self, LayoutError> {
        view.check(document)?;
        update.validate(document, Some(document.id()), Some(document.revision()))?;
        let start = if update.base_revision == update.revision {
            document.source().len()
        } else {
            document
                .blocks()
                .get(update.changed_blocks.start)
                .filter(|_| !update.changed_blocks.is_empty())
                .map_or(update.appended.start, |block| {
                    block.source.start.min(update.appended.start)
                })
        };
        Self::new(document, view, Span::new(start, document.source().len()))
    }

    /// Source window still to process, including any unfinished registration.
    /// Save before dropping a stale task; union with new work, not old history.
    pub fn remaining_range(&self) -> Span {
        Span::new(
            self.pending
                .as_ref()
                .map_or(self.discovery.next.min(self.discovery.end), |p| p.0),
            self.discovery.end,
        )
    }

    pub fn advance<S>(
        &mut self,
        document: &Document,
        view: &mut FoldView<S>,
        budget: &mut Budget,
    ) -> Result<ParagraphAutoFoldProgress, LayoutError> {
        if self.discovery.document != document.id() {
            return Err(LayoutError::WrongDocument);
        }
        if self.view != view.heights().version().0 {
            return Err(LayoutError::WrongView);
        }
        if self.discovery.revision != document.revision() || self.version != view.version() {
            return Err(LayoutError::StaleContinuation);
        }
        view.check(document)?;
        let mut change: Option<FoldChange> = None;
        loop {
            if let Some((_, task)) = &mut self.pending {
                match task.advance(document, view, budget) {
                    Ok(progress) => {
                        let Some(id) = progress.id else {
                            return Ok(self.progress(Completion::BudgetExhausted, change));
                        };
                        let applied = view.set_collapsed(document, id, true)?.unwrap();
                        self.version = view.version();
                        self.registered += 1;
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
                    // Syntax/row crossings are a skipped candidate. Endpoint
                    // validation has not changed the view or allocated an ID.
                    Err(LayoutError::InvalidRange) => self.skipped += 1,
                    Err(error) => return Err(error),
                }
                self.pending = None;
            }
            if self.discovery.next >= self.discovery.end {
                return Ok(self.progress(Completion::Covered, change));
            }
            if !budget.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            }) {
                return Ok(self.progress(Completion::BudgetExhausted, change));
            }
            self.inspected += 1;
            let Some(candidate) = self.discovery.next_candidate(document) else {
                continue;
            };
            let preview = self.policy.unwrap().preview_bytes;
            if preview >= candidate.classified_end - candidate.source.start
                || view.paragraph_auto_excluded.contains(&candidate.block)
            {
                self.skipped += 1;
                continue;
            }
            let task = if candidate.container.is_some() {
                SourceFoldRegistration::container_paragraph(
                    document,
                    view,
                    candidate.block,
                    preview,
                )
            } else if candidate.open {
                SourceFoldRegistration::open_paragraph(document, view, candidate.block, preview)
            } else {
                SourceFoldRegistration::paragraph(document, view, candidate.block, preview)
            };
            match task {
                Ok(task) => self.pending = Some((candidate.source.start, task)),
                Err(LayoutError::InvalidRange) => self.skipped += 1,
                Err(error) => return Err(error),
            }
        }
    }

    fn progress(
        &self,
        completion: Completion,
        change: Option<FoldChange>,
    ) -> ParagraphAutoFoldProgress {
        ParagraphAutoFoldProgress {
            completion,
            inspected: self.inspected,
            registered: self.registered,
            skipped: self.skipped,
            change,
        }
    }
}
