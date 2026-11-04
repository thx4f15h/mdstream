use std::collections::BTreeMap;

use crate::{
    BlockId, BlockKind, Budget, ChangeKind, Completion, Document, Span, StreamingLine, SyntaxState,
    TILE_BYTES, Update, Work, WorkLimit, block,
};

/// Maximum UTF-8 bytes in a case-sensitive alphanumeric/underscore/hyphen label.
pub const MAX_FOOTNOTE_LABEL_BYTES: usize = 999;

/// Stable document-local symbol, independent of numbering and definition order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FootnoteId(pub usize);

/// Stable source-ordered occurrence, distinct from the symbol it references.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FootnoteReferenceId(pub usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FootnoteReference {
    pub symbol: FootnoteId,
    /// Complete `[^label]` marker. Generic display keeps this literal fallback.
    pub source: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FootnoteDefinition {
    pub block: BlockId,
    /// Raw body envelope, without the opener or final LF/CRLF. Includes internal
    /// line endings, container prefixes and continuation indentation; use
    /// Document::footnote_payload to omit the latter two and map split tabs.
    pub content: Span,
    pub state: SyntaxState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Footnote {
    pub label: String,
    /// One-based first-reference order; definitions alone never allocate numbers.
    pub number: Option<usize>,
    pub first_use: Option<usize>,
    /// First recognized definition wins, including an empty definition.
    pub definition: Option<FootnoteDefinition>,
    /// Semantic version, independent of append chunk boundaries.
    pub revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FootnoteTarget {
    /// No definition yet; more source may arrive.
    Unresolved,
    /// EOF sealed without a definition.
    Missing,
    Defined(FootnoteDefinition),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FootnoteResolution {
    pub symbol: FootnoteId,
    pub number: usize,
    pub revision: u64,
    pub target: FootnoteTarget,
}

pub(crate) fn valid_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= MAX_FOOTNOTE_LABEL_BYTES
        && label
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-'))
}

#[derive(Default)]
pub(crate) struct Footnotes {
    names: BTreeMap<String, FootnoteId>,
    pub entries: Vec<Footnote>,
    pub references: Vec<FootnoteReference>,
    // Source-ordered continuation prefixes of winning definitions only.
    gaps: Vec<FootnoteGap>,
    // Only the active definition can defer a blank separating list children.
    trailing_blank: bool,
    numbered: usize,
    pub revision: u64,
}

struct FootnoteGap {
    source: Span,
    // A split final tab contributes these spaces, mapped to its single byte.
    spaces: u8,
}

impl Footnotes {
    // Callers validate bounded labels before interning. BTreeMap supplies a
    // deterministic O(log symbols) bound with a fixed maximum comparison length.
    pub fn intern(&mut self, label: &str, work: &mut Work) -> FootnoteId {
        work.footnote_steps += 1;
        work.footnote_label_bytes += label.len() as u64;
        if let Some(&id) = self.names.get(label) {
            return id;
        }
        let id = FootnoteId(self.entries.len());
        self.names.insert(label.to_owned(), id);
        self.entries.push(Footnote {
            label: label.to_owned(),
            number: None,
            first_use: None,
            definition: None,
            revision: 0,
        });
        id
    }

    pub fn reference(
        &mut self,
        symbol: FootnoteId,
        source: Span,
        update: &mut Update,
        work: &mut Work,
    ) {
        work.footnote_steps += 1;
        let entry = &mut self.entries[symbol.0];
        if entry.number.is_none() {
            self.numbered += 1;
            entry.number = Some(self.numbered);
            entry.first_use = Some(source.start);
            entry.revision += 1;
            self.revision = update.revision;
            update.footnotes_changed.push(symbol);
        }
        self.references.push(FootnoteReference { symbol, source });
    }

    pub fn define(
        &mut self,
        symbol: FootnoteId,
        definition: FootnoteDefinition,
        source: Span,
        update: &mut Update,
        work: &mut Work,
    ) {
        work.footnote_steps += 1;
        self.trailing_blank = false;
        if self.entries[symbol.0].definition.is_some() {
            return;
        }
        self.entries[symbol.0].definition = Some(definition);
        self.changed(symbol, source.len() as u64 + 1, source.end, update);
    }

    pub fn extend(
        &mut self,
        symbol: FootnoteId,
        block: BlockId,
        end: usize,
        appended: Span,
        update: &mut Update,
        work: &mut Work,
    ) {
        work.footnote_steps += 1;
        let definition = self.entries[symbol.0].definition.as_mut().unwrap();
        if definition.block != block {
            return; // Duplicate definitions remain hidden but never replace the first.
        }
        definition.content.end = end;
        self.changed(symbol, appended.len() as u64, appended.end, update);
    }

    pub fn seal(
        &mut self,
        symbol: FootnoteId,
        block: BlockId,
        source_end: usize,
        update: &mut Update,
        work: &mut Work,
    ) {
        work.footnote_steps += 1;
        let definition = self.entries[symbol.0].definition.as_mut().unwrap();
        if definition.block != block || definition.state == SyntaxState::Sealed {
            return;
        }
        definition.state = SyntaxState::Sealed;
        self.changed(symbol, 1, source_end, update);
    }

    fn changed(
        &mut self,
        symbol: FootnoteId,
        version_delta: u64,
        source_end: usize,
        update: &mut Update,
    ) {
        let entry = &mut self.entries[symbol.0];
        entry.revision += version_delta;
        self.revision = update.revision;
        update.footnotes_changed.push(symbol);
        if let Some(start) = entry.first_use {
            // Conservative propagation for object renderers; no occurrence scan.
            update.change(Span::new(start, source_end), ChangeKind::Structure);
        }
    }
}

impl Document {
    /// Match the existing explicit path before ordinary block classification.
    /// A failed probe seals the definition; its bounded prefix is then classified
    /// normally, without replaying any previously committed body.
    pub(crate) fn continue_footnote(
        &mut self,
        span: Span,
        complete: bool,
        update: &mut Update,
    ) -> bool {
        let Some(id) = self.current else {
            return false;
        };
        let BlockKind::FootnoteDefinition { symbol } = self.blocks[id.0].kind else {
            return false;
        };
        let prefix = self.containers.footnote_prefix(
            &self.source,
            span,
            self.blocks[id.0].container,
            &mut self.work,
        );
        let mut blank = false;
        let payload = prefix.and_then(|(prefix, indented)| {
            let body = &self.source.as_bytes()[prefix..block::body_end(&self.source, span)];
            blank = complete && body.iter().all(|b| matches!(b, b' ' | b'\t'));
            if let Some(payload) = indented {
                Some(payload)
            } else if blank {
                Some((prefix, 0))
            } else {
                None
            }
        });
        let Some((payload, spaces)) = payload else {
            if self.footnotes.trailing_blank {
                // Internal blank paragraphs do not loosen a surrounding list.
                // A trailing blank followed by another child/sibling does.
                self.containers
                    .observe_payload(true, span.start, &mut self.work, update);
            }
            self.close_current(update);
            return false;
        };
        self.footnotes.trailing_blank = blank;
        if payload > span.start && self.footnotes.entries[symbol.0].definition.unwrap().block == id
        {
            self.footnotes.gaps.push(FootnoteGap {
                source: Span::new(span.start, payload),
                spaces,
            });
        }
        let line = StreamingLine {
            block: id,
            inline: false,
            standalone: false,
            hidden: true,
            table: false,
        };
        self.extend_line(line, span, update);
        if !complete {
            self.streaming = Some(line);
        }
        true
    }

    pub fn footnotes(&self) -> &[Footnote] {
        &self.footnotes.entries
    }

    pub fn footnote(&self, id: FootnoteId) -> Option<&Footnote> {
        self.footnotes.entries.get(id.0)
    }

    pub fn footnote_references(&self) -> &[FootnoteReference] {
        &self.footnotes.references
    }

    /// O(1) construction; borrowed cursors cannot outlive the current revision.
    pub fn footnote_references_in(&self, range: Span) -> Option<FootnoteReferences<'_>> {
        self.text(range)?;
        Some(FootnoteReferences {
            entries: &self.footnotes.references,
            range,
            index: None,
            complete: range.is_empty(),
        })
    }

    /// One query/object/dependency, no label or body reads. Invalid IDs return None.
    pub fn resolve_footnote(
        &self,
        id: FootnoteReferenceId,
        budget: &mut Budget,
    ) -> Result<Option<FootnoteResolution>, Completion> {
        if !budget.charge(WorkLimit {
            queries: 1,
            objects: 1,
            dependencies: 1,
            ..WorkLimit::default()
        }) {
            return Err(Completion::BudgetExhausted);
        }
        let Some(reference) = self.footnotes.references.get(id.0) else {
            return Ok(None);
        };
        let entry = &self.footnotes.entries[reference.symbol.0];
        Ok(Some(FootnoteResolution {
            symbol: reference.symbol,
            number: entry.number.unwrap(),
            revision: entry.revision,
            target: entry.definition.map_or(
                if self.finished {
                    FootnoteTarget::Missing
                } else {
                    FootnoteTarget::Unresolved
                },
                FootnoteTarget::Defined,
            ),
        }))
    }

    /// O(1) borrowed raw body reader. No Markdown/TeX parsing or body allocation.
    pub fn footnote_content(&self, id: FootnoteId) -> Option<FootnoteContent<'_>> {
        Some(FootnoteContent {
            source: self.source(),
            remaining: self.footnote(id)?.definition?.content,
            gaps: &[],
            gap: None,
        })
    }

    /// O(1) mapped reader of a definition's committed body. Omits matched
    /// container prefixes and four-column continuation indentation, using
    /// physical four-column tab stops. A split final tab maps to 1..3 spaces;
    /// all other raw spelling and internal LF/CRLF are preserved. Each omitted
    /// fragment has empty text but charges source bytes, one query and one object.
    /// First successful advance locates gaps in O(log G); later steps are O(1).
    pub fn footnote_payload(&self, id: FootnoteId) -> Option<FootnoteContent<'_>> {
        let mut cursor = self.footnote_content(id)?;
        cursor.gaps = &self.footnotes.gaps;
        Some(cursor)
    }
}

pub struct FootnoteReferences<'a> {
    entries: &'a [FootnoteReference],
    range: Span,
    index: Option<usize>,
    complete: bool,
}

impl FootnoteReferences<'_> {
    /// First charged lookup O(log R), subsequent steps O(1); one query/object
    /// per result or terminating lookup. No progress or charges on exhaustion.
    pub fn advance(
        &mut self,
        budget: &mut Budget,
    ) -> Result<Option<(FootnoteReferenceId, FootnoteReference)>, Completion> {
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
                .partition_point(|r| r.source.end <= self.range.start)
        });
        if let Some(&reference) = self.entries.get(index)
            && reference.source.start < self.range.end
        {
            self.index = Some(index + 1);
            return Ok(Some((FootnoteReferenceId(index), reference)));
        }
        self.complete = true;
        Ok(None)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FootnoteChunk<'a> {
    /// Consumed source, including omitted prefix fragments in payload mode.
    pub source: Span,
    /// Borrows exactly source, is empty for an omitted prefix fragment, or is
    /// 1..3 static spaces mapped to one partially consumed tab byte.
    pub text: &'a str,
}

pub struct FootnoteContent<'a> {
    source: &'a str,
    remaining: Span,
    gaps: &'a [FootnoteGap],
    gap: Option<usize>,
}

impl<'a> FootnoteContent<'a> {
    pub fn source_position(&self) -> usize {
        self.remaining.start
    }

    /// At most TILE_BYTES source UTF-8 bytes, one query/object per chunk,
    /// including empty prefix fragments. Charges the maximum of source/text
    /// bytes; split tabs are atomic. Four bytes suffice for progress;
    /// exhaustion preserves position and charges nothing.
    pub fn advance(
        &mut self,
        budget: &mut Budget,
    ) -> Result<Option<FootnoteChunk<'a>>, Completion> {
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
        // Locate the next prefix after checking basic progress. A split tab
        // additionally requires its full output-byte budget before advancing.
        let mut gap = self.gap.unwrap_or_else(|| {
            self.gaps
                .partition_point(|g| g.source.end <= self.remaining.start)
        });
        let mut omitted = false;
        let mut spaces = 0;
        if let Some(g) = self.gaps.get(gap) {
            if self.remaining.start < g.source.start {
                end = end.min(g.source.start);
            } else {
                let hidden_end = g.source.end - usize::from(g.spaces > 0);
                if self.remaining.start < hidden_end {
                    end = end.min(hidden_end);
                } else {
                    spaces = usize::from(g.spaces);
                    if available.bytes < spaces {
                        return Err(Completion::BudgetExhausted);
                    }
                    end = g.source.end;
                }
                omitted = true;
                if end == g.source.end {
                    gap += 1;
                }
            }
        }
        let source = Span::new(self.remaining.start, end);
        let charged = budget.charge(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: source.len().max(spaces),
            ..WorkLimit::default()
        });
        debug_assert!(charged);
        self.remaining.start = end;
        self.gap = Some(gap);
        Ok(Some(FootnoteChunk {
            source,
            text: if omitted {
                &"   "[..spaces]
            } else {
                &self.source[source.range()]
            },
        }))
    }
}

#[cfg(test)]
mod tests {
    use crate::Document;

    #[test]
    fn partial_tab_storage_is_per_winning_line_not_body_or_reference() {
        let mut doc = Document::new();
        doc.append(&format!("- [^n]: first\n\t\t{}", "x".repeat(2048)))
            .unwrap();
        assert_eq!(doc.footnotes.gaps.len(), 1);
        assert_eq!(doc.footnotes.gaps[0].spaces, 2);
        let capacity = doc.footnotes.gaps.capacity();
        for _ in 0..1000 {
            doc.append("x").unwrap();
        }
        assert_eq!(doc.footnotes.gaps.len(), 1);
        assert_eq!(doc.footnotes.gaps.capacity(), capacity);
        doc.append("\n- [^n]: duplicate\n\t\tignored\n- [^m]: next\n\t\tlast\n")
            .unwrap();
        assert_eq!(doc.footnotes.gaps.len(), 2);
        assert!(doc.footnotes.gaps.iter().all(|g| g.spaces == 2));
        assert!(doc.footnotes.gaps[0].source.end <= doc.footnotes.gaps[1].source.start);
    }

    #[test]
    fn prefix_storage_tracks_lines_without_copying_body_or_duplicate_definitions() {
        let mut doc = Document::new();
        doc.append(&format!("> [^n]: first\n>     {}", "x".repeat(1_000_000)))
            .unwrap();
        assert_eq!(doc.footnotes.gaps.len(), 1);
        let gaps = doc.footnotes.gaps.capacity();
        let blocks = doc.blocks.capacity();
        let symbols = doc.footnotes.entries.capacity();
        for _ in 0..1000 {
            doc.append("x").unwrap();
        }
        assert_eq!(doc.footnotes.gaps.len(), 1);
        assert_eq!(doc.footnotes.gaps.capacity(), gaps);
        assert_eq!(doc.blocks.capacity(), blocks);
        assert_eq!(doc.footnotes.entries.capacity(), symbols);
        doc.append("\n>\n>     last\n> [^n]: duplicate\n>     ignored\n> [^m]: new\n>     last\n")
            .unwrap();
        assert_eq!(doc.footnotes.gaps.len(), 4);
        assert!(
            doc.footnotes
                .gaps
                .windows(2)
                .all(|g| g[0].source.end <= g[1].source.start)
        );
        assert_eq!(doc.blocks.len(), 3);
        assert_eq!(doc.footnotes.entries.len(), 2);
    }
}
