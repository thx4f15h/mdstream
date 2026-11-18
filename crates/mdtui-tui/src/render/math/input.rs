//! Owned, append-only TeX payload pages and exact logical/root byte mapping.
use super::storage::{Paged, StorageWork, retire_map};
use mdtui3::{
    Budget, Document, LayoutError, MathObjectMeta, MathPayloadCursor, MathPayloadStep, Span,
    TILE_BYTES, WorkLimit,
};
use std::collections::BTreeMap;

#[derive(Clone, Copy)]
struct Mapping {
    end: usize,
    root: Span,
    raw: bool,
}
#[derive(Default)]
pub(super) struct Input {
    pub origin: usize,
    bytes: Paged<u8>,
    map: BTreeMap<usize, Mapping>,
    pub sealed: bool,
    pub source_bytes: usize,
    pub emitted_bytes: usize,
    pub mapping_steps: usize,
}
impl Input {
    pub fn len(&self) -> usize {
        self.bytes.len()
    }
    pub fn byte(&self, at: usize) -> u8 {
        self.bytes[at]
    }
    pub fn root(&self, logical: Span) -> Span {
        if logical.is_empty() {
            return Span::new(self.origin, self.origin);
        }
        let (&a, first) = self
            .map
            .range(..=logical.start)
            .next_back()
            .expect("mapped payload start");
        let (&z, last) = self
            .map
            .range(..logical.end)
            .next_back()
            .expect("mapped payload end");
        debug_assert!(logical.start < first.end && logical.end <= last.end);
        Span::new(
            if first.raw {
                first.root.start + logical.start - a
            } else {
                first.root.start
            },
            if last.raw {
                last.root.start + logical.end - z
            } else {
                last.root.end
            },
        )
    }
    pub fn stats(&self) -> StorageWork {
        self.bytes.stats()
    }
    pub fn map_payload_bytes(&self) -> usize {
        self.map.len() * (std::mem::size_of::<usize>() + std::mem::size_of::<Mapping>())
    }
    pub fn map_len(&self) -> usize {
        self.map.len()
    }
    pub fn retire_one(&mut self) -> bool {
        if retire_map(&mut self.map) {
            return false;
        }
        self.bytes.pop();
        self.bytes.is_empty()
    }
}
#[derive(Clone, Copy)]
struct Chunk {
    bytes: [u8; TILE_BYTES],
    len: usize,
    at: usize,
    source: Span,
    mapped: bool,
}
pub(super) struct Decoder {
    cursor: MathPayloadCursor,
    pending: Option<Chunk>,
    pub input: Input,
    pub cut: Option<usize>,
    stopped: bool,
    pub meta: MathObjectMeta,
    pub steps: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeStep {
    Progress,
    PendingInput,
    Complete,
    BudgetExhausted,
}
fn charge(b: &mut Budget, bytes: usize) -> bool {
    b.charge(WorkLimit {
        queries: 1,
        objects: 1,
        bytes,
        ..WorkLimit::default()
    })
}
impl Decoder {
    pub fn new(document: &Document, meta: MathObjectMeta) -> Option<Self> {
        Some(Self {
            cursor: document.math_payload_cursor(meta.id)?,
            cut: None,
            stopped: false,
            pending: None,
            input: Input {
                origin: meta.content.start,
                ..Input::default()
            },
            meta,
            steps: 0,
        })
    }
    pub fn step(&mut self, document: &Document, b: &mut Budget) -> Result<DecodeStep, LayoutError> {
        if b.remaining().queries == 0 || b.remaining().objects == 0 {
            return Ok(DecodeStep::BudgetExhausted);
        }
        let Some(now) = document
            .math_object(self.meta.id)
            .filter(|m| m.epoch == self.meta.epoch)
        else {
            return Err(LayoutError::StaleContinuation);
        };
        self.meta = now;
        if let Some(mut c) = self.pending {
            if !c.mapped {
                if !charge(b, 0) {
                    return Ok(DecodeStep::BudgetExhausted);
                }
                if c.len > 0 {
                    self.input.map.insert(
                        self.input.len(),
                        Mapping {
                            end: self.input.len() + c.len,
                            root: c.source,
                            raw: c.len == c.source.len(),
                        },
                    );
                    self.input.mapping_steps += 1;
                }
                c.mapped = true;
                self.pending = Some(c);
                self.steps += 1;
                return Ok(DecodeStep::Progress);
            }
            if c.at < c.len {
                if !charge(b, 1) {
                    return Ok(DecodeStep::BudgetExhausted);
                }
                self.input.bytes.push(c.bytes[c.at]);
                self.input.emitted_bytes += 1;
                c.at += 1;
                self.steps += 1;
                self.pending = if c.at == c.len { None } else { Some(c) };
                return Ok(DecodeStep::Progress);
            }
            self.pending = None;
        }
        if self
            .cut
            .is_some_and(|cut| self.cursor.source_position() >= cut)
        {
            self.stopped = true;
        }
        if self.stopped {
            return Ok(if charge(b, 0) {
                DecodeStep::PendingInput
            } else {
                DecodeStep::BudgetExhausted
            });
        }
        match self.cursor.advance(document, b)? {
            MathPayloadStep::Chunk(c) => {
                let mut source = c.source;
                let mut text = c.text;
                if let Some(cut) = self.cut {
                    if source.start >= cut {
                        self.stopped = true;
                        return Ok(DecodeStep::PendingInput);
                    }
                    if source.end >= cut {
                        if source.len() == text.len() {
                            text = &text[..cut - source.start];
                            source.end = cut;
                        }
                        self.stopped = true;
                    }
                }
                let mut bytes = [0; TILE_BYTES];
                bytes[..text.len()].copy_from_slice(text.as_bytes());
                self.pending = Some(Chunk {
                    bytes,
                    len: text.len(),
                    at: 0,
                    source,
                    mapped: false,
                });
                self.input.source_bytes += c.source.len();
                self.steps += 1;
                Ok(DecodeStep::Progress)
            }
            MathPayloadStep::Pending => Ok(DecodeStep::PendingInput),
            MathPayloadStep::Complete => {
                self.input.sealed = true;
                Ok(DecodeStep::Complete)
            }
            MathPayloadStep::BudgetExhausted => Ok(DecodeStep::BudgetExhausted),
        }
    }
    pub fn retire_one(&mut self) -> bool {
        self.pending = None;
        self.input.retire_one()
    }
}
