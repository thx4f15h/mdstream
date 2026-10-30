use crate::{
    BlockId, Budget, Completion, ContainerId, Document, DocumentId, LayoutError, Span, WorkLimit,
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
    fn paragraph_fold_snapshot(&self, block: BlockId) -> Option<ParagraphFoldCandidate> {
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
                    out.push(candidate);
                }
            } else if let Some(entry) = document.blocks.get(leaf)
                && entry.source.start < self.end
            {
                self.next = entry.source.end;
            } else {
                self.next = self.end;
            }
        }
        Ok(Completion::Covered)
    }
}
