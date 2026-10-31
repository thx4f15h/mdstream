use crate::{
    Budget, DisplayExcerpt, DisplayRun, Document, DocumentId, ExcerptEnd, ExcerptLimits,
    ExcerptStep, LayoutError, TableId, WorkLimit,
};

/// Lifetime caps shared across all preview rows, not reset for each row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TablePreviewLimits {
    /// Maximum indexed rows, including the header (row zero).
    pub rows: usize,
    /// Source bytes including hidden syntax within rows; excludes the delimiter.
    pub source_bytes: usize,
    /// Emitted UTF-8 bytes; never splits a scalar.
    pub display_bytes: usize,
}

/// The first detected stopping condition, without scanning for visible tail.
/// As with DisplayExcerpt, coincident source/display limits may report either
/// reason depending on paid read windows; the emitted prefix stays the same.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TablePreviewEnd {
    /// All currently indexed rows were consumed, possibly including an open row.
    RowsEnd,
    RowLimit,
    SourceLimit,
    DisplayLimit,
}

#[derive(Debug, PartialEq, Eq)]
pub enum TablePreviewStep<'a> {
    /// Row identity plus the original display/source mapping. No synthesized
    /// separators, padding, ellipsis or implicit empty cells are emitted.
    Run {
        row: usize,
        run: DisplayRun<'a>,
    },
    Complete(TablePreviewEnd),
    BudgetExhausted,
}

/// Bounded first-row summary for an entire folded table. Reads independently of
/// FoldView: preserves collapse intent, temporary preview, version and heights,
/// even beneath a collapsed ancestor. The host decides whether to show it.
/// Does not measure cells or change the fold's hidden range/placeholder height.
///
/// Construction and storage are O(1). Row lookup costs one query/object; display
/// work uses DisplayExcerpt's shared budget. No traversal of columns, cells or
/// omitted rows occurs. With R visited rows and S inspected source bytes, work
/// is O(R + (S + 1) log(n + 1)); output storage belongs to the caller.
///
/// Task and output are revision snapshots. Append/first finish invalidates even
/// completed tasks; recreate explicitly. Unclassified pending input is excluded;
/// indexed incomplete rows are included with their current source extent.
pub struct TablePreview {
    document: DocumentId,
    revision: u64,
    table: TableId,
    limits: TablePreviewLimits,
    row: usize,
    excerpt: Option<DisplayExcerpt>,
    complete: Option<TablePreviewEnd>,
}

impl TablePreview {
    pub fn new(
        document: &Document,
        table: TableId,
        limits: TablePreviewLimits,
    ) -> Result<Self, LayoutError> {
        document.table(table).ok_or(LayoutError::InvalidRange)?;
        Ok(Self {
            document: document.id(),
            revision: document.revision(),
            table,
            limits,
            row: 0,
            excerpt: None,
            complete: None,
        })
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Wrong/stale documents fail before mutation or charging. Completion is
    /// repeatable for free. Six bytes and one query/object per frame suffice
    /// for progress (row setup may consume a separate frame).
    pub fn next<'a>(
        &mut self,
        document: &'a Document,
        budget: &mut Budget,
    ) -> Result<TablePreviewStep<'a>, LayoutError> {
        if document.id() != self.document {
            return Err(LayoutError::WrongDocument);
        }
        if document.revision() != self.revision {
            return Err(LayoutError::StaleContinuation);
        }
        loop {
            if let Some(end) = self.complete {
                return Ok(TablePreviewStep::Complete(end));
            }
            if self.excerpt.is_none() {
                let table = document.table(self.table).unwrap();
                self.complete = if self.row == table.rows().len() {
                    Some(TablePreviewEnd::RowsEnd)
                } else if self.row == self.limits.rows {
                    Some(TablePreviewEnd::RowLimit)
                } else if self.limits.source_bytes == 0 {
                    Some(TablePreviewEnd::SourceLimit)
                } else if self.limits.display_bytes == 0 {
                    Some(TablePreviewEnd::DisplayLimit)
                } else {
                    None
                };
                if self.complete.is_some() {
                    continue;
                }
                if !budget.charge(WorkLimit {
                    queries: 1,
                    objects: 1,
                    ..Default::default()
                }) {
                    return Ok(TablePreviewStep::BudgetExhausted);
                }
                let source = table.row(self.row).unwrap().source;
                self.excerpt = Some(DisplayExcerpt::new(
                    document,
                    source,
                    ExcerptLimits {
                        source_bytes: self.limits.source_bytes,
                        display_bytes: self.limits.display_bytes,
                        lines: usize::MAX,
                    },
                )?);
                self.limits.source_bytes -= source.len().min(self.limits.source_bytes);
            }
            match self.excerpt.as_mut().unwrap().next(document, budget)? {
                ExcerptStep::Run(run) => {
                    self.limits.display_bytes -= run.text.len();
                    return Ok(TablePreviewStep::Run { row: self.row, run });
                }
                ExcerptStep::BudgetExhausted => return Ok(TablePreviewStep::BudgetExhausted),
                ExcerptStep::Complete(ExcerptEnd::SourceEnd) => {
                    self.excerpt = None;
                    self.row += 1;
                }
                ExcerptStep::Complete(end) => {
                    self.complete = Some(match end {
                        ExcerptEnd::SourceLimit => TablePreviewEnd::SourceLimit,
                        ExcerptEnd::DisplayLimit => TablePreviewEnd::DisplayLimit,
                        _ => unreachable!("row excerpt has unlimited logical lines"),
                    });
                }
            }
        }
    }
}
