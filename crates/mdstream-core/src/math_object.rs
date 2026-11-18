//! Current mathematical objects, independent of their allocation arena.
use std::collections::BTreeMap;

use crate::{
    BlockId, Budget, Completion, Document, DocumentId, LayoutError, MathChunk, MathContent, MathId,
    MathKind, Span, SyntaxState, Update, Work, WorkLimit,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MathDelimiter {
    Dollar,
    DoubleDollar,
    Paren,
    Bracket,
}

impl MathDelimiter {
    pub(crate) fn kind(self) -> MathKind {
        match self {
            Self::Dollar | Self::Paren => MathKind::Inline,
            Self::DoubleDollar | Self::Bracket => MathKind::Display,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MathObjectId {
    Inline(MathId),
    Block(BlockId),
}

/// Current metadata. `epoch` changes on withdrawal/republication; `version`
/// also changes on append/seal. Appends preserve the entire old payload prefix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MathObjectMeta {
    pub document: DocumentId,
    pub id: MathObjectId,
    pub source: Span,
    pub content: Span,
    pub kind: MathKind,
    pub delimiter: MathDelimiter,
    pub state: SyntaxState,
    pub closed: bool,
    pub epoch: u64,
    pub version: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MathChangeKind {
    Publish,
    Append,
    Seal,
    Withdraw,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MathChange {
    pub kind: MathChangeKind,
    pub id: MathObjectId,
    pub before: Option<MathObjectMeta>,
    pub after: Option<MathObjectMeta>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MathObjectStats {
    pub active: usize,
    pub retained: usize,
    /// Key/value payload only; excludes BTree allocation headers/slack.
    pub index_payload_bytes: usize,
}

#[derive(Default)]
pub(crate) struct MathIndex {
    records: BTreeMap<MathObjectId, MathObjectMeta>,
    order: BTreeMap<usize, MathObjectId>,
    serial: u64,
}

impl MathIndex {
    pub(crate) fn next_anchor(&self, offset: usize) -> Option<usize> {
        if self
            .order
            .range(..=offset)
            .next_back()
            .is_some_and(|(_, id)| self.records[id].source.end > offset)
        {
            return Some(offset);
        }
        self.order.range(offset..).next().map(|(&start, _)| start)
    }
    pub(crate) fn get(&self, id: MathObjectId) -> Option<MathObjectMeta> {
        let meta = *self.records.get(&id)?;
        (self.order.get(&meta.source.start) == Some(&id)).then_some(meta)
    }

    pub(crate) fn publish(
        &mut self,
        mut meta: MathObjectMeta,
        update: &mut Update,
        work: &mut Work,
    ) {
        let before = self.get(meta.id);
        if let Some(old) = before {
            meta.epoch = old.epoch;
            meta.version = old.version;
            if meta == old {
                return;
            }
        }
        self.serial += 1;
        let prefix = before.is_some_and(|old| {
            old.source.start == meta.source.start
                && old.content.start == meta.content.start
                && old.content.end <= meta.content.end
                && old.delimiter == meta.delimiter
        });
        meta.epoch = if prefix {
            before.unwrap().epoch
        } else {
            self.serial
        };
        meta.version = self.serial;
        let kind = match before {
            Some(old) if prefix && old.content.end < meta.content.end => MathChangeKind::Append,
            Some(_) if prefix => MathChangeKind::Seal,
            _ => MathChangeKind::Publish,
        };
        self.records.insert(meta.id, meta);
        self.order.insert(meta.source.start, meta.id);
        update.math_changes.push(MathChange {
            kind,
            id: meta.id,
            before,
            after: Some(meta),
        });
        work.math_object_steps += 1;
    }

    pub(crate) fn withdraw(&mut self, id: MathObjectId, update: &mut Update, work: &mut Work) {
        if let Some(before) = self.get(id) {
            self.order.remove(&before.source.start);
            update.math_changes.push(MathChange {
                kind: MathChangeKind::Withdraw,
                id,
                before: Some(before),
                after: None,
            });
            work.math_object_steps += 1;
        }
    }
}

impl Document {
    pub fn math_object(&self, id: MathObjectId) -> Option<MathObjectMeta> {
        self.math_objects.get(id)
    }

    pub fn math_delimiter(&self, id: MathId) -> Option<MathDelimiter> {
        self.math_object(MathObjectId::Inline(id))
            .map(|m| m.delimiter)
    }

    pub fn math_object_stats(&self) -> MathObjectStats {
        MathObjectStats {
            active: self.math_objects.order.len(),
            retained: self.math_objects.records.len(),
            index_payload_bytes: self.math_objects.order.len()
                * (std::mem::size_of::<usize>() + std::mem::size_of::<MathObjectId>())
                + self.math_objects.records.len()
                    * (std::mem::size_of::<MathObjectId>() + std::mem::size_of::<MathObjectMeta>()),
        }
    }

    /// O(1) construction, no allocation. Each advance pays one query/object
    /// for O(log M) current-source predecessor/successor lookup; no body read.
    pub fn math_window(&self, source: Span) -> Result<MathWindow, LayoutError> {
        self.text(source).ok_or(LayoutError::InvalidRange)?;
        Ok(MathWindow {
            document: self.id(),
            revision: self.revision(),
            source,
            next: None,
            complete: source.is_empty(),
        })
    }

    /// O(log M) metadata lookup, no payload copy or scan. This cursor survives unrelated
    /// appends and real prefix-preserving growth. A withdrawn/reused ID rejects.
    pub fn math_payload_cursor(&self, id: MathObjectId) -> Option<MathPayloadCursor> {
        let meta = self.math_object(id)?;
        Some(MathPayloadCursor {
            document: self.id(),
            id,
            epoch: meta.epoch,
            position: meta.content.start,
            gap: usize::MAX,
            space: usize::MAX,
        })
    }

    pub(crate) fn refresh_math_block(&mut self, id: BlockId, update: &mut Update) {
        let Some(old) = self.math_objects.get(MathObjectId::Block(id)) else {
            return;
        };
        let block = &self.blocks[id.0];
        let crate::BlockKind::Math { content, closed } = block.kind else {
            return;
        };
        self.math_objects.publish(
            MathObjectMeta {
                source: block.source,
                content,
                closed,
                state: block.state,
                ..old
            },
            update,
            &mut self.work,
        );
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MathWindowStep {
    Object(MathObjectMeta),
    BudgetExhausted,
    Complete,
}

pub struct MathWindow {
    document: DocumentId,
    revision: u64,
    source: Span,
    next: Option<usize>,
    complete: bool,
}

impl MathWindow {
    pub fn advance(
        &mut self,
        document: &Document,
        budget: &mut Budget,
    ) -> Result<MathWindowStep, LayoutError> {
        if self.document != document.id() {
            return Err(LayoutError::WrongDocument);
        }
        if self.revision != document.revision() {
            return Err(LayoutError::StaleContinuation);
        }
        if self.complete {
            return Ok(MathWindowStep::Complete);
        }
        if !budget.charge(WorkLimit {
            queries: 1,
            objects: 1,
            ..WorkLimit::default()
        }) {
            return Ok(MathWindowStep::BudgetExhausted);
        }
        let index = &document.math_objects;
        let start = self.next.unwrap_or_else(|| {
            index
                .order
                .range(..=self.source.start)
                .next_back()
                .filter(|(_, id)| index.records[id].source.end > self.source.start)
                .map_or(self.source.start, |(&at, _)| at)
        });
        if let Some((&at, id)) = index.order.range(start..self.source.end).next() {
            self.next = Some(at + 1);
            return Ok(MathWindowStep::Object(index.records[id]));
        }
        self.complete = true;
        Ok(MathWindowStep::Complete)
    }
}

#[derive(Debug)]
pub struct MathPayloadCursor {
    document: DocumentId,
    id: MathObjectId,
    epoch: u64,
    position: usize,
    gap: usize,
    space: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MathPayloadStep<'a> {
    Chunk(MathChunk<'a>),
    /// No new committed payload yet; preserve lexer/AST prefix, do not send EOF.
    Pending,
    BudgetExhausted,
    Complete,
}

impl MathPayloadCursor {
    pub fn source_position(&self) -> usize {
        self.position
    }

    pub fn advance<'a>(
        &mut self,
        document: &'a Document,
        budget: &mut Budget,
    ) -> Result<MathPayloadStep<'a>, LayoutError> {
        if document.id() != self.document {
            return Err(LayoutError::WrongDocument);
        }
        if budget.remaining().queries == 0 || budget.remaining().objects == 0 {
            return Ok(MathPayloadStep::BudgetExhausted);
        }
        let meta = document
            .math_object(self.id)
            .filter(|m| m.epoch == self.epoch)
            .ok_or(LayoutError::StaleContinuation)?;
        if self.position == meta.content.end {
            let charged = budget.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..WorkLimit::default()
            });
            debug_assert!(charged);
            return Ok(if meta.state == SyntaxState::Sealed {
                MathPayloadStep::Complete
            } else {
                MathPayloadStep::Pending
            });
        }
        let mut inner = MathContent {
            source: document.source(),
            remaining: Span::new(self.position, meta.content.end),
            gaps: &document.math_gaps,
            gap: self.gap,
            spaces: document.display.space_mappings(),
            space: self.space,
        };
        let chunk = match inner.advance(budget) {
            Ok(Some(chunk)) => chunk,
            Err(Completion::BudgetExhausted) => {
                // The current-object lookup above still cost a query even
                // when the next scalar/source fragment has no byte admission.
                let charged = budget.charge(WorkLimit {
                    queries: 1,
                    objects: 1,
                    ..WorkLimit::default()
                });
                debug_assert!(charged);
                return Ok(MathPayloadStep::BudgetExhausted);
            }
            _ => return Err(LayoutError::InvalidRange),
        };
        self.position = inner.remaining.start;
        self.gap = inner.gap;
        self.space = inner.space;
        Ok(MathPayloadStep::Chunk(chunk))
    }
}
