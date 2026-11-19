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

/// Stable allocation identity, distinct from the symbol it references.
/// Active source order is provided by `Document::footnote_references_in`.
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

type ReferenceCheckpoint = (
    usize,
    usize,
    BTreeMap<FootnoteId, (Option<usize>, Option<usize>, u64)>,
);
#[derive(Default)]
pub(crate) struct Footnotes {
    extended_continuation: bool,
    continuation: Continuation,
    pub(crate) shape: crate::DisplayShapeEffects,
    pub(crate) journal: Option<Vec<Event>>,
    pub(crate) body_indices: Vec<Option<usize>>,
    pub(crate) slots: Vec<std::sync::Arc<crate::symbol_context::Symbol>>,
    pub(crate) bindings: Vec<Option<std::sync::Arc<crate::symbol_context::Symbol>>>,
    names: BTreeMap<String, FootnoteId>,
    pub(crate) first_reference: Option<usize>,
    pub(crate) literal_markers: std::collections::BTreeSet<usize>,
    pub entries: Vec<Footnote>,
    pub references: Vec<FootnoteReference>,
    active: Vec<bool>,
    states: Vec<SyntaxState>,
    pub(crate) next: Vec<Option<FootnoteReferenceId>>,
    order: BTreeMap<usize, FootnoteReferenceId>,
    checkpoint: Option<ReferenceCheckpoint>,
    undo_peak: usize,
    reuse_peak: usize,
    reuse: BTreeMap<(usize, usize, FootnoteId), FootnoteReferenceId>,
    // Source-ordered continuation prefixes of winning definitions only.
    gaps: Vec<FootnoteGap>,
    // Only the active definition can defer a blank separating list children.
    trailing_blank: bool,
    numbered: usize,
    pub revision: u64,
}

/// Input-only qualification of the current definition. Winning and losing
/// definitions share this summary; it neither parses inline syntax nor interns
/// symbols. Each NEW physical line is classified once, when LF/EOF seals it,
/// with the existing leaf/table classifiers. No old body is replayed.
#[derive(Default)]
struct Continuation {
    paragraph: bool,
    nested: bool,
    fence: Option<block::Fence>,
    header_columns: Option<usize>,
    table: bool,
    line: Option<(usize, u8)>,
}
impl Continuation {
    fn finish_line(&mut self, source: &str, end: usize, work: &mut Work) {
        let Some((start, spaces)) = self.line.take() else {
            return;
        };
        let span = Span::new(start, end);
        work.classified_bytes += span.len() as u64;
        let kind =
            block::classify_indented(source, span, true, self.fence, false, spaces, spaces % 4);
        self.paragraph = false;
        if self.fence.is_some() {
            if matches!(kind, block::Line::FenceClose) {
                self.fence = None;
            }
            self.header_columns = None;
            return;
        }
        let mut header = crate::table::HeaderSummary::default();
        header.feed(&source[span.range()], work);
        let columns = header.columns();
        let delimiter = self.header_columns.is_some_and(|count| {
            work.classified_bytes += span.len() as u64;
            crate::table::delimiter(source, span, count).is_some()
        });
        self.header_columns = None;
        match kind {
            block::Line::Blank => {
                self.nested = false;
                self.table = false;
            }
            block::Line::FenceOpen { fence, .. } => {
                self.fence = Some(fence);
                self.table = false;
            }
            block::Line::Text if delimiter || (self.table && columns.is_some()) => {
                self.table = true;
            }
            block::Line::Text => {
                self.table = false;
                if !self.nested {
                    self.paragraph = crate::containers::Containers::footnote_text(
                        source, span, true, spaces, work,
                    );
                    self.nested = !self.paragraph;
                    self.header_columns = columns;
                }
            }
            // In a body this exact marker is literal, as in the real parser.
            block::Line::FootnoteDefinition { .. } => {
                self.paragraph = !self.nested;
                self.table = false;
            }
            _ => {
                self.table = false;
            }
        }
    }
}

struct FootnoteGap {
    source: Span,
    // A split final tab contributes these spaces, mapped to its single byte.
    spaces: u8,
}

pub(crate) enum Event {
    Begin,
    Commit,
    Rollback,
    Reference(FootnoteId, Span),
}

impl Footnotes {
    pub(crate) fn semantic_revision(&self, id: FootnoteId) -> u64 {
        self.bindings[id.0]
            .as_ref()
            .map_or(self.entries[id.0].revision, |s| s.revision())
    }
    pub(crate) fn defined(&self, id: FootnoteId) -> bool {
        self.bindings[id.0]
            .as_ref()
            .map_or(self.entries[id.0].definition.is_some(), |s| s.resolved())
    }
    fn publish(&self, id: FootnoteId) {
        let entry = &self.entries[id.0];
        self.slots[id.0].publish_number(entry.number);
        self.slots[id.0].publish(entry.definition.is_some(), entry.revision);
    }

    pub(crate) fn confirmation_peak(&self) -> usize {
        self.undo_peak
            .max(self.checkpoint.as_ref().map_or(0, |c| c.2.len()))
            + self.reuse_peak.max(self.reuse.len())
    }

    pub(crate) fn begin_reference_line(&mut self) {
        self.shape.emit(crate::DisplayShapeEffectKind::Begin(
            crate::DisplayShapeDomain::FootnoteReferences,
        ));
        assert!(self.checkpoint.is_none());
        if let Some(journal) = &mut self.journal {
            journal.push(Event::Begin);
        }
        self.reuse.clear();
        self.checkpoint = Some((self.references.len(), self.numbered, BTreeMap::new()));
    }
    pub(crate) fn commit_reference_line(&mut self) -> u64 {
        if self.checkpoint.is_some() {
            self.shape.emit(crate::DisplayShapeEffectKind::Commit(
                crate::DisplayShapeDomain::FootnoteReferences,
            ));
        }
        let Some((base, _, saved)) = self.checkpoint.take() else {
            return 0;
        };
        if let Some(journal) = &mut self.journal {
            journal.push(Event::Commit);
        }
        self.undo_peak = self.undo_peak.max(saved.len());
        for id in base..self.references.len() {
            if self.active[id] {
                self.states[id] = SyntaxState::Sealed;
                self.shape
                    .emit(crate::DisplayShapeEffectKind::FootnoteReference {
                        id: FootnoteReferenceId(id),
                        reference: self.references[id],
                        state: Some(SyntaxState::Sealed),
                    });
            }
        }
        (self.references.len() - base + saved.len()) as u64
    }
    pub(crate) fn rollback_reference_line(&mut self, update: &mut Update) -> u64 {
        self.shape.emit(crate::DisplayShapeEffectKind::Rollback(
            crate::DisplayShapeDomain::FootnoteReferences,
        ));
        let (base, numbered, saved) = self.checkpoint.take().unwrap();
        if let Some(journal) = &mut self.journal {
            journal.push(Event::Rollback);
        }
        self.undo_peak = self.undo_peak.max(saved.len());
        let count = self.references.len() - base + saved.len();
        for i in base..self.references.len() {
            assert_ne!(self.states[i], SyntaxState::Sealed);
            let r = self.references[i];
            self.reuse.insert(
                (r.source.start, r.source.end, r.symbol),
                FootnoteReferenceId(i),
            );
            if self.active[i] {
                self.deactivate(FootnoteReferenceId(i));
            }
        }
        for (id, (number, first_use, revision)) in saved {
            let entry = &mut self.entries[id.0];
            // A hosted definition may grow while its body's line is pending.
            // Undo only this line's numbering effect, preserving that growth.
            if number.is_none() && entry.number.is_some() {
                entry.revision -= 1;
            }
            debug_assert!(entry.revision >= revision);
            entry.number = number;
            entry.first_use = first_use;
            self.shape
                .emit(crate::DisplayShapeEffectKind::Footnote { id });
            self.publish(id);
            update.footnotes_changed.push(id);
        }
        self.reuse_peak = self.reuse_peak.max(self.reuse.len());
        self.numbered = numbered;
        count as u64
    }
    pub(crate) fn clear_reference_reuse(&mut self) -> u64 {
        let n = self.reuse.len();
        self.reuse.clear();
        n as u64
    }
    fn activate(&mut self, id: FootnoteReferenceId) {
        let start = self.references[id.0].source.start;
        let next = self
            .order
            .range((std::ops::Bound::Excluded(start), std::ops::Bound::Unbounded))
            .next()
            .map(|(_, id)| *id);
        if let Some((_, &previous)) = self.order.range(..start).next_back() {
            self.next[previous.0] = Some(id);
        }
        self.next[id.0] = next;
        self.active[id.0] = true;
        self.order.insert(start, id);
    }
    fn deactivate(&mut self, id: FootnoteReferenceId) {
        self.shape
            .emit(crate::DisplayShapeEffectKind::FootnoteReference {
                id,
                reference: self.references[id.0],
                state: None,
            });
        let start = self.references[id.0].source.start;
        if let Some((_, &previous)) = self.order.range(..start).next_back() {
            self.next[previous.0] = self.next[id.0];
        }
        self.order.remove(&start);
        self.active[id.0] = false;
        self.next[id.0] = None;
    }
    pub(crate) fn first_reference(&self, start: usize) -> Option<FootnoteReferenceId> {
        self.order
            .range(..=start)
            .next_back()
            .map(|(_, id)| *id)
            .filter(|id| self.references[id.0].source.end > start)
            .or_else(|| self.order.range(start..).next().map(|(_, id)| *id))
    }

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
        self.body_indices.push(None);
        self.slots.push(Default::default());
        self.bindings.push(None);
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
        if let Some(journal) = &mut self.journal {
            journal.push(Event::Reference(symbol, source));
        }
        self.reference_symbol(symbol, source, update);
        let id = if let Some(id) = self.reuse.remove(&(source.start, source.end, symbol)) {
            id
        } else {
            let id = FootnoteReferenceId(self.references.len());
            self.references.push(FootnoteReference { symbol, source });
            self.active.push(false);
            self.states.push(SyntaxState::Provisional);
            self.next.push(None);
            id
        };
        self.states[id.0] = if self.checkpoint.is_some() {
            SyntaxState::Provisional
        } else {
            SyntaxState::Sealed
        };
        self.activate(id);
        self.shape
            .emit(crate::DisplayShapeEffectKind::FootnoteReference {
                id,
                reference: self.references[id.0],
                state: Some(self.states[id.0]),
            });
    }

    pub(crate) fn reference_symbol(
        &mut self,
        symbol: FootnoteId,
        source: Span,
        update: &mut Update,
    ) {
        self.first_reference = Some(
            self.first_reference
                .map_or(source.start, |old| old.min(source.start)),
        );
        let entry = &mut self.entries[symbol.0];
        if let Some((_, _, saved)) = &mut self.checkpoint {
            saved
                .entry(symbol)
                .or_insert((entry.number, entry.first_use, entry.revision));
        }
        if entry.number.is_none() {
            self.numbered += 1;
            entry.number = Some(self.numbered);
            entry.first_use = Some(source.start);
            entry.revision += 1;
            self.revision = update.revision;
            self.shape
                .emit(crate::DisplayShapeEffectKind::Footnote { id: symbol });
            update.footnotes_changed.push(symbol);
        }
        self.publish(symbol);
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
        self.shape
            .emit(crate::DisplayShapeEffectKind::Footnote { id: symbol });
        let first_use = entry.first_use;
        self.publish(symbol);
        update.footnotes_changed.push(symbol);
        if let Some(start) = first_use {
            // Conservative propagation for object renderers; no occurrence scan.
            update.change(Span::new(start, source_end), ChangeKind::Structure);
        }
    }
}

impl Document {
    /// Canonical display number for an active occurrence in this arena. A
    /// hosted body reads its fixed shared symbol fact; no global table copy or
    /// old-occurrence visit is involved. Definition addresses still require
    /// BodyView::resolve_footnote, never an unqualified native body ID.
    pub fn footnote_display_number(
        &self,
        id: FootnoteReferenceId,
        budget: &mut Budget,
    ) -> Result<Option<usize>, Completion> {
        if !budget.charge(WorkLimit {
            queries: 1,
            objects: 1,
            dependencies: 1,
            ..Default::default()
        }) {
            return Err(Completion::BudgetExhausted);
        }
        if self.footnote_reference_state(id).is_none() {
            return Ok(None);
        }
        let symbol = self.footnotes.references[id.0].symbol;
        Ok(self.footnotes.bindings[symbol.0]
            .as_ref()
            .map_or(self.footnotes.entries[symbol.0].number, |s| s.number()))
    }
    /// Opt into the finite paragraph-lazy footnote continuation dialect before
    /// input. The default retains the original strict-four-column core API.
    pub fn enable_footnote_continuation(&mut self) -> Result<(), crate::LayoutError> {
        if !self.source.is_empty() || self.finished {
            return Err(crate::LayoutError::InvalidRange);
        }
        self.footnotes.extended_continuation = true;
        Ok(())
    }
    pub(crate) fn begin_footnote_continuation(&mut self, start: usize) {
        if self.footnotes.extended_continuation {
            self.footnotes.continuation = Continuation {
                line: Some((start, 0)),
                ..Default::default()
            };
        }
    }
    pub(crate) fn finish_footnote_line(&mut self, end: usize) {
        if self.footnotes.extended_continuation {
            self.footnotes
                .continuation
                .finish_line(&self.source, end, &mut self.work);
        }
    }
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
        let mut payload = prefix.and_then(|(prefix, indented)| {
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
        if payload.is_none()
            && self.footnotes.extended_continuation
            && self.footnotes.continuation.paragraph
        {
            payload = self.containers.footnote_lazy_prefix(
                &self.source,
                span,
                self.blocks[id.0].container,
                complete,
                &mut self.work,
            );
            if let (Some((start, _)), Some(columns)) =
                (payload, self.footnotes.continuation.header_columns)
            {
                // A possible new table delimiter belongs to the host, even if
                // its last cell has not reached LF yet. Reuse the real streaming
                // delimiter grammar; do not append and later retract body bytes.
                let body = Span::new(start, block::body_end(&self.source, span));
                let mut delimiter = crate::table::DelimiterCandidate::new(body, columns);
                if delimiter.feed(&self.source[body.range()], &mut self.work) {
                    payload = None;
                }
            }
        }
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
        if self.footnotes.extended_continuation {
            self.footnotes.continuation.line = Some((payload, spaces));
        }
        self.prefix_body(symbol, id, span.start, payload, spaces, update);
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
        if complete {
            self.finish_footnote_line(span.end);
        }
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

    /// Stable allocation arena; pending late-header rows can contain
    /// provisional/withdrawn occurrences. Enumerate footnote_references_in or
    /// use footnote_reference_state before consuming an occurrence.
    pub fn footnote_references(&self) -> &[FootnoteReference] {
        &self.footnotes.references
    }

    /// Number of currently active occurrences, excluding withdrawn arena slots.
    /// Constant-time metadata; no reference or source traversal.
    pub fn active_footnote_reference_count(&self) -> usize {
        self.footnotes.order.len()
    }

    pub fn footnote_reference_state(&self, id: FootnoteReferenceId) -> Option<SyntaxState> {
        self.footnotes
            .active
            .get(id.0)
            .copied()
            .filter(|active| *active)?;
        self.footnotes.states.get(id.0).copied()
    }
    /// Exact marker lookup, O(log R), with no source read or occurrence scan.
    /// Like block_at, the caller accounts for this metadata operation.
    pub fn footnote_reference_at(&self, source: Span) -> Option<FootnoteReferenceId> {
        let id = *self.footnotes.order.get(&source.start)?;
        (self.footnotes.references[id.0].source == source).then_some(id)
    }

    /// Active source-order successor in O(1), independent of allocation order.
    /// Obtain the first ID with `footnote_references_in`; resumable consumers
    /// can retain that ID and follow this link after paying to resolve it.
    /// Withdrawn or invalid IDs have no successor.
    pub fn next_footnote_reference(&self, id: FootnoteReferenceId) -> Option<FootnoteReferenceId> {
        self.footnote_reference_state(id)?;
        self.footnotes.next[id.0]
    }

    /// O(1) construction; borrowed cursors cannot outlive the current revision.
    pub fn footnote_references_in(&self, range: Span) -> Option<FootnoteReferences<'_>> {
        self.text(range)?;
        Some(FootnoteReferences {
            footnotes: &self.footnotes,
            range,
            current: None,
            started: false,
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
        if self.footnote_reference_state(id).is_none() {
            return Ok(None);
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
    footnotes: &'a Footnotes,
    range: Span,
    current: Option<FootnoteReferenceId>,
    started: bool,
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
        if !self.started {
            self.current = self.footnotes.first_reference(self.range.start);
            self.started = true;
        }
        if let Some(id) = self.current {
            let reference = self.footnotes.references[id.0];
            if reference.source.start < self.range.end {
                self.current = self.footnotes.next[id.0];
                return Ok(Some((id, reference)));
            }
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
