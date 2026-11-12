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
/// `Document::math_payload` to omit those prefixes and expand their partial
/// tabs without decoding Markdown.
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
            gap: usize::MAX,
            spaces: &[],
            space: usize::MAX,
        })
    }

    /// Like math_block_content, omitting matched container prefixes and mapping
    /// retained partial tabs to 1..3 spaces. Other whitespace, CRLF, backslashes
    /// and Markdown spelling are preserved.
    /// Uses the same mapped chunks and shared budget as math_payload.
    pub fn math_block_payload(&self, id: BlockId) -> Option<MathContent<'_>> {
        let mut cursor = self.math_block_content(id)?;
        cursor.gaps = &self.math_gaps;
        cursor.spaces = self.display.space_mappings();
        Some(cursor)
    }

    /// Stable allocation arena. A pending late-header row can contain
    /// provisional or withdrawn candidates; use math_state/math/maths_in for
    /// current semantic objects. IDs never shift when a row is confirmed.
    pub fn maths(&self) -> &[Math] {
        &self.maths
    }

    pub fn math(&self, id: MathId) -> Option<&Math> {
        self.math_active
            .get(id.0)
            .copied()
            .filter(|active| *active)?;
        self.maths.get(id.0)
    }
    pub fn math_state(&self, id: MathId) -> Option<crate::SyntaxState> {
        self.math(id)?;
        self.math_states.get(id.0).copied()
    }

    /// O(1) construction. The first advance pays for the O(log M) range lookup;
    /// each subsequent step is O(1). The borrow prevents revision changes.
    pub fn maths_in(&self, range: Span) -> Option<MathObjects<'_>> {
        self.text(range)?;
        Some(MathObjects {
            entries: &self.maths,
            range,
            order: &self.math_index,
            cursor: None,
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
            gap: usize::MAX,
            spaces: &[],
            space: usize::MAX,
        })
    }

    /// O(1) prefix-free content cursor, preserving raw CRLF and backslashes and
    /// expanding retained partial container tabs to 1..3 spaces.
    /// Each chunk maps to its original source; omitted prefixes return empty
    /// text and still consume source bytes, a query and an object. The first
    /// successful advance pays for O(log G + log T) lookup in the G recorded
    /// gaps and T partial tabs; a tab chunk maps to its single source byte;
    /// subsequent chunks take O(1), with no ancestor walk or body allocation.
    pub fn math_payload(&self, id: MathId) -> Option<MathContent<'_>> {
        let mut cursor = self.math_content(id)?;
        cursor.gaps = &self.math_gaps;
        cursor.spaces = self.display.space_mappings();
        Some(cursor)
    }
}

/// Budgeted viewport enumeration; objects overlapping either delimiter qualify.
pub struct MathObjects<'a> {
    entries: &'a [Math],
    range: Span,
    order: &'a std::collections::BTreeMap<usize, MathId>,
    cursor: Option<std::collections::btree_map::Range<'a, usize, MathId>>,
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
        if self.cursor.is_none() {
            let start = self
                .order
                .range(..=self.range.start)
                .next_back()
                .filter(|(_, id)| self.entries[id.0].source.end > self.range.start)
                .map_or(self.range.start, |(&start, _)| start);
            self.cursor = Some(self.order.range(start..));
        }
        if let Some((_, &id)) = self.cursor.as_mut().unwrap().next() {
            let math = self.entries[id.0];
            if math.source.start < self.range.end {
                return Ok(Some((id, math)));
            }
        }
        self.complete = true;
        Ok(None)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MathChunk<'a> {
    /// Consumed original source, including omitted container prefixes.
    pub source: Span,
    /// Borrows exactly `source`, is empty for an omitted prefix fragment, or
    /// contains 1..3 static spaces mapped to a single partially consumed tab.
    pub text: &'a str,
}

/// Resumable raw or prefix-free reader with O(1) state and no output allocation.
pub struct MathContent<'a> {
    source: &'a str,
    remaining: Span,
    gaps: &'a [Span],
    // MAX means the first paid range lookup has not happened yet.
    gap: usize,
    spaces: &'a [(Span, u8)],
    space: usize,
}

impl<'a> MathContent<'a> {
    pub fn source_position(&self) -> usize {
        self.remaining.start
    }

    /// At most TILE_BYTES source UTF-8 bytes, one query and one object per chunk,
    /// including empty prefix fragments. Mapped partial tabs cost one source
    /// byte and emit at most three spaces atomically; other bytes are not decoded.
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
        let mut gap = if self.gap == usize::MAX {
            self.gaps.partition_point(|g| g.end <= self.remaining.start)
        } else {
            self.gap
        };
        let mut space = if self.space == usize::MAX {
            self.spaces
                .partition_point(|s| s.0.end <= self.remaining.start)
        } else {
            self.space
        };
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
        // Gaps and retained tab bytes are disjoint source-ordered streams.
        // Reuse display mappings directly, without copying them at closure or
        // applying unrelated entity/code normalization to raw math spelling.
        let mut columns = None;
        if let Some(&(tab, count)) = self.spaces.get(space) {
            if self.remaining.start < tab.start {
                end = end.min(tab.start);
            } else {
                debug_assert!(!omitted);
                end = tab.end;
                columns = Some(count);
                space += 1;
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
        self.gap = gap;
        self.space = space;
        Ok(Some(MathChunk {
            source,
            text: if omitted {
                ""
            } else if let Some(columns) = columns {
                &"   "[..usize::from(columns)]
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
    fn math_tabs_reuse_display_records_and_never_rewrite_history_on_closure() {
        for block in [false, true] {
            let mut doc = Document::new();
            doc.append(if block { "> $$\n" } else { "> $a\n" }).unwrap();
            for _ in 0..4096 {
                doc.append(">\tx\n").unwrap();
            }
            assert_eq!(doc.math_gaps.len(), 4096);
            assert_eq!(doc.display.space_mappings().len(), 4096);
            doc.append(&format!(">\t{}", "x".repeat(1_000_000)))
                .unwrap();
            let gaps = doc.math_gaps.clone();
            let tabs = doc.display.space_mappings().to_vec();
            let capacity = doc.math_gaps.capacity();
            let before = doc.work();
            for _ in 0..1000 {
                doc.append("x").unwrap();
            }
            assert_eq!(doc.math_gaps, gaps);
            assert_eq!(doc.math_gaps.capacity(), capacity);
            assert_eq!(doc.display.space_mappings(), tabs);
            assert_eq!(doc.work().container_steps, before.container_steps);
            assert_eq!(
                doc.work().inline_bytes - before.inline_bytes,
                if block { 0 } else { 1000 }
            );
            doc.append(if block { "\n>\t$$\n" } else { "$\n" }).unwrap();
            assert_eq!(doc.math_gaps, gaps);
            assert_eq!(doc.display.space_mappings(), tabs);
            assert!(gaps.iter().all(|gap| doc.text(*gap) == Some(">")));
            assert!(
                gaps.iter()
                    .zip(&tabs)
                    .all(|(gap, tab)| gap.end == tab.0.start)
            );
            let mut cursor = if block {
                doc.math_block_payload(crate::BlockId(0))
            } else {
                doc.math_payload(MathId(0))
            }
            .unwrap();
            let mut budget = Budget::new(WorkLimit::unlimited());
            let mut replacements = 0;
            while let Some(chunk) = cursor.advance(&mut budget).unwrap() {
                if chunk.text == "  " {
                    replacements += 1;
                    assert_eq!(doc.text(chunk.source), Some("\t"));
                }
            }
            assert_eq!(replacements, 4097);
        }
    }

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
