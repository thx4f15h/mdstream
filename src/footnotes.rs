use std::collections::BTreeMap;

use crate::{
    BlockId, Budget, ChangeKind, Completion, Document, Span, SyntaxState, TILE_BYTES, Update, Work,
    WorkLimit,
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
    /// Raw single-line body, without the optional one space/tab or final CRLF.
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
    numbered: usize,
    pub revision: u64,
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
        })
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
    pub source: Span,
    pub text: &'a str,
}

pub struct FootnoteContent<'a> {
    source: &'a str,
    remaining: Span,
}

impl<'a> FootnoteContent<'a> {
    pub fn source_position(&self) -> usize {
        self.remaining.start
    }

    /// At most TILE_BYTES UTF-8 bytes, one query/object per chunk. Four bytes
    /// suffice for progress; exhaustion preserves position and charges nothing.
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
        let source = Span::new(self.remaining.start, end);
        let charged = budget.charge(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: source.len(),
            ..WorkLimit::default()
        });
        debug_assert!(charged);
        self.remaining.start = end;
        Ok(Some(FootnoteChunk {
            source,
            text: &self.source[source.range()],
        }))
    }
}
