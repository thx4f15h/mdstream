use crate::{BlockId, BlockKind, Budget, Completion, Document, Span, TILE_BYTES, WorkLimit};

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
/// Content is a raw source envelope, including any container prefixes. Read
/// `Document::math_payload` to omit those prefixes without decoding Markdown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Math {
    pub kind: MathKind,
    /// Includes both delimiters.
    pub source: Span,
    /// Excludes both delimiters; may include whitespace and physical newlines.
    pub content: Span,
}

impl Document {
    /// O(1) reader of the committed raw content of an independent math block.
    /// Pending, unclassified line prefixes are not yet part of the content.
    /// BlockKind::Math holds the current range/closed flag; Block::state and
    /// Update::{new_blocks, changed_blocks} report growth and sealing.
    pub fn math_block_content(&self, id: BlockId) -> Option<MathContent<'_>> {
        let BlockKind::Math { content, .. } = self.block(id)?.kind else {
            return None;
        };
        Some(MathContent {
            source: self.source(),
            remaining: content,
            gaps: &[],
            gap: None,
        })
    }

    /// Like math_block_content, omitting only matched container prefixes.
    /// Raw whitespace, CRLF, backslashes and Markdown spelling are preserved.
    /// Uses the same mapped chunks and shared budget as math_payload.
    pub fn math_block_payload(&self, id: BlockId) -> Option<MathContent<'_>> {
        let mut cursor = self.math_block_content(id)?;
        cursor.gaps = &self.math_gaps;
        Some(cursor)
    }

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
            gaps: &[],
            gap: None,
        })
    }

    /// O(1) prefix-free content cursor, preserving raw CRLF and backslashes.
    /// Each chunk maps to its original source; omitted prefixes return empty
    /// text and still consume source bytes, a query and an object. The first
    /// successful advance pays for O(log G) lookup in the G recorded gaps;
    /// subsequent chunks take O(1), with no ancestor walk or body allocation.
    pub fn math_payload(&self, id: MathId) -> Option<MathContent<'_>> {
        Some(MathContent {
            source: self.source(),
            remaining: self.math(id)?.content,
            gaps: &self.math_gaps,
            gap: None,
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
    /// Consumed original source, including omitted container prefixes.
    pub source: Span,
    /// Borrows exactly `source`, or is empty for an omitted prefix fragment.
    pub text: &'a str,
}

/// Resumable raw or prefix-free reader with O(1) state and no output allocation.
pub struct MathContent<'a> {
    source: &'a str,
    remaining: Span,
    gaps: &'a [Span],
    gap: Option<usize>,
}

impl<'a> MathContent<'a> {
    pub fn source_position(&self) -> usize {
        self.remaining.start
    }

    /// At most TILE_BYTES source UTF-8 bytes, one query and one object per chunk,
    /// including empty prefix fragments. No Markdown or whitespace decoding.
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
        // All remaining paths make progress, so the lookup is paid even when
        // the next gap splits the otherwise available inspection window.
        let mut gap = self
            .gap
            .unwrap_or_else(|| self.gaps.partition_point(|g| g.end <= self.remaining.start));
        let mut omitted = false;
        if let Some(g) = self.gaps.get(gap) {
            if self.remaining.start < g.start {
                end = end.min(g.start);
            } else {
                end = end.min(g.end);
                omitted = true;
                if end == g.end {
                    gap += 1;
                }
            }
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
        self.gap = Some(gap);
        Ok(Some(MathChunk {
            source,
            text: if omitted {
                ""
            } else {
                &self.source[source.range()]
            },
        }))
    }
}

#[cfg(test)]
mod tests {
    use crate::{Budget, Document, MathId, WorkLimit};

    #[test]
    fn math_blocks_share_ordered_gaps_without_per_append_or_body_storage() {
        let mut doc = Document::new();
        doc.append("> $inline\n> end$\n> $$\n> ").unwrap();
        doc.append(&"x".repeat(1_000_000)).unwrap();
        assert_eq!(doc.math_gaps.len(), 2);
        let capacity = doc.math_gaps.capacity();
        let blocks = doc.blocks.capacity();
        for _ in 0..1000 {
            doc.append("x").unwrap();
        }
        assert_eq!(doc.math_gaps.len(), 2);
        assert_eq!(doc.math_gaps.capacity(), capacity);
        assert_eq!(doc.blocks.capacity(), blocks);
        doc.append("\n> $$\n> $next\n> end$\n").unwrap();
        assert_eq!(doc.math_gaps.len(), 3);
        assert!(doc.math_gaps.windows(2).all(|g| g[0].end <= g[1].start));
        for (id, expected) in [(MathId(0), "inline\nend"), (MathId(1), "next\nend")] {
            let mut cursor = doc.math_payload(id).unwrap();
            let mut budget = Budget::new(WorkLimit::unlimited());
            let mut text = String::new();
            while let Some(chunk) = cursor.advance(&mut budget).unwrap() {
                text.push_str(chunk.text);
            }
            assert_eq!(text, expected);
        }
        let id = crate::BlockId(2);
        assert_eq!(
            doc.math_block_payload(id).unwrap().source_position(),
            doc.blocks[id.0].source.start + 5
        );
    }

    #[test]
    fn math_gap_storage_tracks_only_discontinuous_feeds_and_never_copies_body() {
        let mut doc = Document::new();
        doc.append(&format!("> $a\n> {}", "x".repeat(1_000_000)))
            .unwrap();
        assert_eq!(doc.math_gaps.len(), 1);
        let capacity = doc.math_gaps.capacity();
        for _ in 0..1000 {
            doc.append("x").unwrap();
        }
        assert_eq!(doc.math_gaps.len(), 1);
        assert_eq!(doc.math_gaps.capacity(), capacity);
        doc.append("\n").unwrap();
        for _ in 0..100 {
            doc.append("lazy\n").unwrap();
        }
        assert_eq!(doc.math_gaps.len(), 1);
        for _ in 0..100 {
            doc.append("> explicit\n").unwrap();
        }
        assert_eq!(doc.math_gaps.len(), 101);
        doc.append("> end$\n").unwrap();
        assert_eq!(doc.math_gaps.len(), 102);
        assert!(doc.math_gaps.windows(2).all(|g| g[0].end <= g[1].start));
        assert!(doc.math_gaps.iter().all(|g| doc.text(*g) == Some("> ")));
        assert!(std::mem::size_of::<super::MathContent<'_>>() <= 10 * std::mem::size_of::<usize>());
    }

    #[test]
    fn abandoned_candidate_gaps_do_not_change_later_objects_or_raw_reads() {
        let mut doc = Document::new();
        for _ in 0..4096 {
            doc.append("> $failed\n> still open\n>\n").unwrap();
        }
        assert_eq!(doc.math_gaps.len(), 4096);
        assert!(doc.maths().is_empty());
        doc.append("> # $unclosed #\n").unwrap();
        assert_eq!(doc.math_gaps.len(), 4096);
        doc.append("> $中\n> 🙂$\n").unwrap();
        assert_eq!(doc.math_gaps.len(), 4097);
        let mut cursor = doc.math_payload(MathId(0)).unwrap();
        let mut budget = Budget::new(WorkLimit::unlimited());
        let mut payload = String::new();
        while let Some(chunk) = cursor.advance(&mut budget).unwrap() {
            payload.push_str(chunk.text);
        }
        assert_eq!(payload, "中\n🙂");
        assert_eq!(budget.used().queries, 3);
        assert_eq!(budget.used().bytes, doc.maths()[0].content.len());
        assert_eq!(
            doc.math_content(MathId(0))
                .unwrap()
                .advance(&mut budget)
                .unwrap()
                .unwrap()
                .text,
            "中\n> 🙂"
        );
    }
}
