use crate::{Budget, Completion, Document, Span, TILE_BYTES, WorkLimit};

/// Stable, document-local index of a confirmed mathematical span.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MathId(pub usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MathKind {
    /// Exactly one unescaped dollar at each end.
    Inline,
    /// Exactly two unescaped dollars at each end; a presentation hint, not a block.
    Display,
}

/// Confirmed, non-nesting syntax. Both ranges remain stable across appends.
/// Content is raw source, with no TeX validation or Markdown decoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Math {
    pub kind: MathKind,
    /// Includes both delimiters.
    pub source: Span,
    /// Excludes both delimiters; may include whitespace and physical newlines.
    pub content: Span,
}

impl Document {
    /// Source-ordered, non-overlapping confirmed objects. No provisional entries.
    pub fn maths(&self) -> &[Math] {
        &self.maths
    }

    pub fn math(&self, id: MathId) -> Option<&Math> {
        self.maths.get(id.0)
    }

    /// O(1) construction. The first advance pays for the O(log M) range lookup;
    /// each subsequent step is O(1). The borrow prevents revision changes.
    pub fn maths_in(&self, range: Span) -> Option<MathObjects<'_>> {
        self.text(range)?;
        Some(MathObjects {
            entries: &self.maths,
            range,
            index: None,
            complete: range.is_empty(),
        })
    }

    /// O(1) raw-content cursor. Constructing it does not read or copy the body.
    /// The borrow prevents revision changes during budgeted consumption.
    pub fn math_content(&self, id: MathId) -> Option<MathContent<'_>> {
        Some(MathContent {
            source: self.source(),
            remaining: self.math(id)?.content,
        })
    }
}

/// Budgeted viewport enumeration; objects overlapping either delimiter qualify.
pub struct MathObjects<'a> {
    entries: &'a [Math],
    range: Span,
    index: Option<usize>,
    complete: bool,
}

impl MathObjects<'_> {
    /// One query/object per inspected entry, including the terminating lookup.
    /// Failure consumes nothing and preserves position; completed calls are free.
    pub fn advance(&mut self, budget: &mut Budget) -> Result<Option<(MathId, Math)>, Completion> {
        if self.complete {
            return Ok(None);
        }
        if !budget.charge(WorkLimit {
            queries: 1,
            objects: 1,
            ..WorkLimit::default()
        }) {
            return Err(Completion::BudgetExhausted);
        }
        let index = self.index.unwrap_or_else(|| {
            self.entries
                .partition_point(|math| math.source.end <= self.range.start)
        });
        if let Some(&math) = self.entries.get(index)
            && math.source.start < self.range.end
        {
            self.index = Some(index + 1);
            return Ok(Some((MathId(index), math)));
        }
        self.complete = true;
        Ok(None)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MathChunk<'a> {
    pub source: Span,
    pub text: &'a str,
}

/// Resumable raw-source reader with O(1) state and no output allocation.
pub struct MathContent<'a> {
    source: &'a str,
    remaining: Span,
}

impl<'a> MathContent<'a> {
    pub fn source_position(&self) -> usize {
        self.remaining.start
    }

    /// At most TILE_BYTES raw UTF-8 bytes, one query and one object per chunk.
    /// Four byte units suffice for any scalar. Too-small budgets do not advance;
    /// the caller can cancel simply by dropping this cursor.
    pub fn advance(&mut self, budget: &mut Budget) -> Result<Option<MathChunk<'a>>, Completion> {
        if self.remaining.is_empty() {
            return Ok(None);
        }
        let available = budget.remaining();
        if available.queries == 0 || available.objects == 0 || available.bytes == 0 {
            return Err(Completion::BudgetExhausted);
        }
        let mut end = self
            .remaining
            .start
            .saturating_add(available.bytes.min(TILE_BYTES))
            .min(self.remaining.end);
        while !self.source.is_char_boundary(end) {
            end -= 1;
        }
        if end == self.remaining.start {
            return Err(Completion::BudgetExhausted);
        }
        let source = Span::new(self.remaining.start, end);
        let charged = budget.charge(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: source.len(),
            ..WorkLimit::default()
        });
        debug_assert!(charged);
        self.remaining.start = end;
        Ok(Some(MathChunk {
            source,
            text: &self.source[source.range()],
        }))
    }
}
