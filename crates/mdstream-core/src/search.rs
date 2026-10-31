use crate::{
    BlockId, BlockKind, Budget, Completion, Document, DocumentId, LayoutError, Span, WorkLimit,
};
use std::collections::BTreeMap;

/// Fixed-query incremental source search (literal, case-sensitive, overlapping).
/// Changing a query creates a new explicit task; old source is only visited under
/// its budget. KMP failure steps also consume bytes, bounding adversarial retries.
pub struct SourceSearch {
    document: DocumentId,
    pattern: Vec<u8>,
    failure: Vec<usize>,
    cursor: usize,
    matched: usize,
}

impl SourceSearch {
    /// Query preprocessing is O(query bytes), performed once outside frame layout.
    pub fn new(document: &Document, query: &str) -> Result<Self, LayoutError> {
        if query.is_empty() {
            return Err(LayoutError::InvalidRange);
        }
        let pattern = query.as_bytes().to_vec();
        let failure = failure_table(&pattern);
        Ok(Self {
            document: document.id(),
            pattern,
            failure,
            cursor: 0,
            matched: 0,
        })
    }
    pub fn position(&self) -> usize {
        self.cursor
    }
    /// Append new matches to caller storage. Repeated calls at the current tail
    /// are no-ops; a later append resumes with any partial match still intact.
    pub fn advance(
        &mut self,
        document: &Document,
        budget: &mut Budget,
        out: &mut Vec<Span>,
    ) -> Result<Completion, LayoutError> {
        if document.id() != self.document {
            return Err(LayoutError::WrongDocument);
        }
        while self.cursor < document.source().len() {
            let byte = document.source().as_bytes()[self.cursor];
            let complete =
                byte == self.pattern[self.matched] && self.matched + 1 == self.pattern.len();
            if !budget.charge(WorkLimit {
                bytes: 1,
                objects: usize::from(complete),
                ..Default::default()
            }) {
                return Ok(Completion::BudgetExhausted);
            }
            if byte == self.pattern[self.matched] {
                self.matched += 1;
                self.cursor += 1;
                if complete {
                    out.push(Span::new(self.cursor - self.pattern.len(), self.cursor));
                    self.matched = self.failure[self.matched - 1];
                }
            } else if self.matched > 0 {
                self.matched = self.failure[self.matched - 1];
            } else {
                self.cursor += 1;
            }
        }
        Ok(Completion::EndOfDocument)
    }
}

pub(crate) fn failure_table(pattern: &[u8]) -> Vec<usize> {
    let mut failure = vec![0; pattern.len()];
    let mut matched = 0;
    for i in 1..pattern.len() {
        while matched > 0 && pattern[i] != pattern[matched] {
            matched = failure[matched - 1];
        }
        if pattern[i] == pattern[matched] {
            matched += 1;
        }
        failure[i] = matched;
    }
    failure
}

/// Independent task checkbox overrides; source and parser facts stay immutable.
pub struct TaskState {
    document: DocumentId,
    checked: BTreeMap<BlockId, bool>,
}
impl TaskState {
    pub fn new(document: &Document) -> Self {
        Self {
            document: document.id(),
            checked: BTreeMap::new(),
        }
    }
    pub fn checked(&self, document: &Document, block: BlockId) -> Result<bool, LayoutError> {
        if document.id() != self.document {
            return Err(LayoutError::WrongDocument);
        }
        let Some(BlockKind::ListItem {
            task: Some(task), ..
        }) = document.block(block).map(|b| &b.kind)
        else {
            return Err(LayoutError::InvalidRange);
        };
        Ok(self.checked.get(&block).copied().unwrap_or(task.checked))
    }
    pub fn toggle(&mut self, document: &Document, block: BlockId) -> Result<bool, LayoutError> {
        let value = !self.checked(document, block)?;
        self.checked.insert(block, value);
        Ok(value)
    }
    pub fn reset(&mut self, block: BlockId) {
        self.checked.remove(&block);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub document: DocumentId,
    pub source: Span,
}
impl Selection {
    pub fn new(document: &Document, source: Span) -> Result<Self, LayoutError> {
        document.text(source).ok_or(LayoutError::InvalidRange)?;
        Ok(Self {
            document: document.id(),
            source,
        })
    }
    pub fn text(self, document: &Document) -> Result<&str, LayoutError> {
        if document.id() != self.document {
            return Err(LayoutError::WrongDocument);
        }
        document.text(self.source).ok_or(LayoutError::InvalidRange)
    }
    pub fn intersection(self, source: Span) -> Option<Span> {
        let span = Span::new(
            self.source.start.max(source.start),
            self.source.end.min(source.end),
        );
        (span.start < span.end).then_some(span)
    }
}
