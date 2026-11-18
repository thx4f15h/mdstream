//! Incremental terminal mathematics over the core's typed payload. Native
//! Markdown is never reparsed here. Query-owned records live in fixed pages.
//! Full post-TeX Unicode assembly is an explicit integration boundary; a raw
//! Unicode scalar cannot silently use a separate math width/GCB policy.
mod input;
mod layout;
mod locate;
mod paint;
mod shaping;
mod text;
mod view;
pub use locate::{LocateStep, MathPosition};
pub use text::{MathTextCursor, MathTextStep};
pub use view::{MathFocus, MathQuery, MathView, SharedMath, ViewCell};
mod storage;
mod symbols;
mod syntax;

use input::{DecodeStep, Decoder};
use layout::{Layout, LayoutStep};
pub use layout::{LayoutWork, Metrics};
use mdtui3::{Budget, Document, DocumentId, LayoutError, MathObjectId, MathObjectMeta, WorkLimit};
pub use paint::{MathCell, PaintStep, PaintWork, Rect};
pub(crate) use shaping::MathPayload;
pub use shaping::ShapingWork;
use shaping::{ShapeStep, Shaper};
use std::sync::atomic::{AtomicUsize, Ordering};
pub use storage::StorageWork;
pub use syntax::SyntaxWork;
use syntax::{Ink, Syntax, SyntaxStep};

/// Process-local execution context. Never reused, including after retirement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct EngineOwner(usize);
impl EngineOwner {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(1);
        Self(
            NEXT.try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
                .expect("math engine identity space exhausted"),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MathKey {
    owner: EngineOwner,
    document: DocumentId,
    object: MathObjectId,
    epoch: u64,
    version: u64,
    layout: u64,
}
/// Opaque, object-generation-qualified display-byte handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MathInk {
    ink: Ink,
    key: MathKey,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MathProgress {
    Preparing,
    /// A source-stable mathematical prefix is laid out; keep incomplete tokens
    /// and groups, and resume when committed payload grows. Do not inject EOF.
    PendingInput,
    /// Needs the accepted shared POST-TeX cluster assembler, not raw-source GCB.
    NeedShape,
    Ready,
    BudgetExhausted,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct MathStats {
    pub decode_steps: usize,
    pub source_bytes: usize,
    pub payload_bytes: usize,
    pub mappings: usize,
    pub syntax: SyntaxWork,
    pub shaping: ShapingWork,
    pub shaping_storage: [StorageWork; 3],
    pub layout: LayoutWork,
    pub validation_steps: usize,
    pub payload: StorageWork,
    pub syntax_storage: [StorageWork; 3],
    pub layout_storage: [StorageWork; 6],
    pub index_entries: usize,
    /// Input-map and layout-index key/value payload; excludes BTree headers/slack.
    pub index_payload_bytes: usize,
}
/// No source copy, tokenization, AST/page reserve or layout during construction.
/// Fixed metadata lookups cost O(log M); every later continued step shares Budget.
pub struct MathEngine {
    owner: EngineOwner,
    document: DocumentId,
    decoder: Decoder,
    syntax: Syntax,
    shaper: Shaper,
    reflowing: bool,
    layout: Layout,
    turn_layout: bool,
    want_input: bool,
    ready: bool,
    validations: usize,
    waiting_version: Option<u64>,
}
impl MathEngine {
    pub fn new(document: &Document, meta: MathObjectMeta) -> Result<Self, LayoutError> {
        if meta.document != document.id() {
            return Err(LayoutError::WrongDocument);
        }
        if document.math_object(meta.id) != Some(meta) {
            return Err(LayoutError::StaleContinuation);
        }
        Ok(Self {
            owner: EngineOwner::new(),
            document: document.id(),
            decoder: Decoder::new(document, meta).ok_or(LayoutError::InvalidRange)?,
            syntax: Syntax::new(),
            shaper: Shaper::new(),
            reflowing: false,
            layout: Layout::new(),
            turn_layout: false,
            want_input: false,
            ready: false,
            validations: 0,
            waiting_version: None,
        })
    }
    pub fn parsed_bytes(&self) -> usize {
        self.syntax.position()
    }
    pub fn metadata(&self) -> MathObjectMeta {
        self.decoder.meta
    }
    /// A root-source prefix view, used by a frozen host cut. It remains a
    /// pending input boundary, never synthetic EOF or a partial-command error.
    pub fn new_at_source_cut(
        document: &Document,
        meta: MathObjectMeta,
        cut: usize,
    ) -> Result<Self, LayoutError> {
        if !document.source().is_char_boundary(cut) {
            return Err(LayoutError::InvalidRange);
        }
        let mut engine = Self::new(document, meta)?;
        engine.decoder.cut = Some(cut.max(meta.content.start));
        Ok(engine)
    }
    pub fn stats(&self) -> MathStats {
        MathStats {
            decode_steps: self.decoder.steps,
            source_bytes: self.decoder.input.source_bytes,
            payload_bytes: self.decoder.input.emitted_bytes,
            mappings: self.decoder.input.map_len(),
            syntax: self.syntax.work,
            shaping: self.shaper.work,
            shaping_storage: self
                .shaper
                .store
                .as_ref()
                .map_or([StorageWork::default(); 3], |s| s.borrow().storage()),
            layout: self.layout.work,
            validation_steps: self.validations,
            payload: self.decoder.input.stats(),
            syntax_storage: self.syntax.storage(),
            layout_storage: self.layout.storage(),
            index_entries: self.layout.index_entries()
                + self
                    .shaper
                    .store
                    .as_ref()
                    .map_or(0, |s| s.borrow().index_entries()),
            index_payload_bytes: self.layout.index_payload_bytes()
                + self.decoder.input.map_payload_bytes()
                + self
                    .shaper
                    .store
                    .as_ref()
                    .map_or(0, |s| s.borrow().index_payload_bytes()),
        }
    }
    /// Whether the intrinsic layout owns visible ink, including zero-column EGCs.
    pub fn has_ink(&self) -> bool {
        self.ready && self.layout.has_ink()
    }
    pub fn metrics(&self) -> Option<Metrics> {
        self.ready.then(|| self.layout.metrics()).flatten()
    }
    fn snapshot_key(&self) -> MathKey {
        MathKey {
            owner: self.owner,
            document: self.document,
            object: self.decoder.meta.id,
            epoch: self.decoder.meta.epoch,
            version: self.decoder.meta.version,
            layout: self.layout.version,
        }
    }
    fn validate(&self, document: &Document) -> Result<(), LayoutError> {
        if document.id() != self.document {
            return Err(LayoutError::WrongDocument);
        }
        document
            .math_object(self.decoder.meta.id)
            .filter(|m| m.epoch == self.decoder.meta.epoch)
            .ok_or(LayoutError::StaleContinuation)?;
        Ok(())
    }
    fn finish_progress(
        &mut self,
        progress: MathProgress,
        b: &mut Budget,
        start: usize,
    ) -> MathProgress {
        if b.used().queries == start {
            if !b.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..WorkLimit::default()
            }) {
                return MathProgress::BudgetExhausted;
            }
            self.validations += 1;
        }
        progress
    }
    pub fn advance(
        &mut self,
        document: &Document,
        b: &mut Budget,
    ) -> Result<MathProgress, LayoutError> {
        self.advance_limit(document, b, usize::MAX)
    }
    pub(super) fn advance_quantum(
        &mut self,
        document: &Document,
        b: &mut Budget,
    ) -> Result<MathProgress, LayoutError> {
        self.advance_limit(document, b, 1)
    }
    fn advance_limit(
        &mut self,
        document: &Document,
        b: &mut Budget,
        queries: usize,
    ) -> Result<MathProgress, LayoutError> {
        if b.remaining().queries == 0 || b.remaining().objects == 0 {
            return Ok(MathProgress::BudgetExhausted);
        }
        self.validate(document)?;
        let start = b.used().queries;
        let now = document.math_object(self.decoder.meta.id).unwrap();
        let mut blocked = self.waiting_version == Some(now.version);
        loop {
            if b.used().queries - start >= queries {
                return Ok(MathProgress::Preparing);
            }
            if b.remaining().queries == 0 || b.remaining().objects == 0 {
                return Ok(MathProgress::BudgetExhausted);
            }
            if self.reflowing {
                if !b.charge(WorkLimit {
                    queries: 1,
                    objects: 1,
                    ..WorkLimit::default()
                }) {
                    return Ok(MathProgress::BudgetExhausted);
                }
                if self.layout.retire_one() {
                    let version = self.layout.version;
                    self.layout = Layout::new();
                    self.layout.version = version;
                    self.reflowing = false;
                }
                continue;
            }
            if !self.shaper.caught_up(&self.syntax) {
                match self.shaper.step(&mut self.syntax, &self.decoder.input, b) {
                    ShapeStep::BudgetExhausted => {
                        return Ok(self.finish_progress(MathProgress::BudgetExhausted, b, start));
                    }
                    ShapeStep::Touched {
                        node,
                        width_changed,
                    } => {
                        if self.layout.refresh_text(&self.syntax, node, width_changed) {
                            self.reflowing = true;
                        }
                        self.ready = false;
                    }
                    ShapeStep::Progress | ShapeStep::Ready => {}
                }
                continue;
            }
            if self.want_input && !self.decoder.input.sealed && !blocked {
                match self.decoder.step(document, b)? {
                    DecodeStep::Progress => {
                        self.want_input = false;
                        self.turn_layout = false;
                        self.ready = false;
                        self.waiting_version = None;
                    }
                    DecodeStep::Complete => {
                        self.want_input = false;
                        self.turn_layout = false;
                    }
                    DecodeStep::PendingInput => {
                        blocked = true;
                        self.waiting_version = Some(self.decoder.meta.version);
                    }
                    DecodeStep::BudgetExhausted => {
                        return Ok(self.finish_progress(MathProgress::BudgetExhausted, b, start));
                    }
                }
                continue;
            }
            if self.turn_layout {
                match self.layout.step(&self.syntax, b) {
                    LayoutStep::Progress => {
                        self.ready = false;
                        self.turn_layout = false;
                    }
                    LayoutStep::NeedShape => {
                        self.ready = false;
                        return Ok(self.finish_progress(MathProgress::NeedShape, b, start));
                    }
                    LayoutStep::BudgetExhausted => {
                        return Ok(self.finish_progress(MathProgress::BudgetExhausted, b, start));
                    }
                    LayoutStep::Ready => {
                        self.ready = true;
                        self.turn_layout = false;
                        if self.syntax.prefix().is_some_and(|p| p.complete)
                            && self.decoder.input.sealed
                        {
                            return Ok(self.finish_progress(MathProgress::Ready, b, start));
                        }
                        if blocked && self.want_input {
                            return Ok(self.finish_progress(MathProgress::PendingInput, b, start));
                        }
                    }
                }
            } else {
                match self.syntax.step(&self.decoder.input, b) {
                    SyntaxStep::Progress => {
                        self.ready = false;
                        self.want_input = false;
                    }
                    SyntaxStep::Input => self.want_input = true,
                    SyntaxStep::Complete => {}
                    SyntaxStep::BudgetExhausted => {
                        return Ok(self.finish_progress(MathProgress::BudgetExhausted, b, start));
                    }
                }
                self.turn_layout = true;
            }
        }
    }
    /// Continued parse without attempting width/layout, for the shared shaping
    /// adapter to obtain more semantic fragments after NeedShape.
    pub fn advance_syntax(
        &mut self,
        document: &Document,
        b: &mut Budget,
    ) -> Result<MathProgress, LayoutError> {
        if b.remaining().queries == 0 || b.remaining().objects == 0 {
            return Ok(MathProgress::BudgetExhausted);
        }
        self.validate(document)?;
        let start = b.used().queries;
        loop {
            match self.syntax.step(&self.decoder.input, b) {
                SyntaxStep::Progress => self.ready = false,
                SyntaxStep::Complete => {
                    return Ok(self.finish_progress(MathProgress::Ready, b, start));
                }
                SyntaxStep::BudgetExhausted => {
                    return Ok(self.finish_progress(MathProgress::BudgetExhausted, b, start));
                }
                SyntaxStep::Input => match self.decoder.step(document, b)? {
                    DecodeStep::Progress | DecodeStep::Complete => {}
                    DecodeStep::PendingInput => {
                        return Ok(self.finish_progress(MathProgress::PendingInput, b, start));
                    }
                    DecodeStep::BudgetExhausted => {
                        return Ok(self.finish_progress(MathProgress::BudgetExhausted, b, start));
                    }
                },
            }
        }
    }
    pub fn paint(&self, rect: Rect) -> Result<MathPaint, LayoutError> {
        if !self.ready
            || rect.x.checked_add(rect.width).is_none()
            || rect.y.checked_add(rect.height).is_none()
        {
            return Err(LayoutError::InvalidRange);
        }
        Ok(MathPaint {
            inner: paint::Paint::new(
                &self.layout,
                rect,
                MathKey {
                    owner: self.owner,
                    document: self.document,
                    object: self.decoder.meta.id,
                    epoch: self.decoder.meta.epoch,
                    version: self.decoder.meta.version,
                    layout: self.layout.version,
                },
            ),
            document: self.document,
            owner: self.owner,
            epoch: self.decoder.meta.epoch,
            version: self.decoder.meta.version,
        })
    }
    pub fn locate_source(&self, source: mdtui3::Span) -> Result<MathLocator, LayoutError> {
        if !self.ready
            || source.start > source.end
            || source.start < self.decoder.meta.source.start
            || source.end > self.decoder.meta.source.end
        {
            return Err(LayoutError::InvalidRange);
        }
        Ok(MathLocator {
            owner: self.owner,
            inner: locate::Locator::new(source),
            document: self.document,
            epoch: self.decoder.meta.epoch,
            version: self.decoder.meta.version,
            layout: self.layout.version,
        })
    }

    /// Owned byte cursor for an emitted glyph/command handle. No text copy or
    /// allocation; admission failure is distinct from completion.
    pub fn ink_cursor(&self, handle: MathInk) -> Result<MathInkCursor, LayoutError> {
        if handle.key.document != self.document {
            return Err(LayoutError::WrongDocument);
        }
        if handle.key.owner != self.owner {
            return Err(LayoutError::StaleContinuation);
        }
        if handle.key.object != self.decoder.meta.id
            || handle.key.epoch != self.decoder.meta.epoch
            || handle.key.version != self.decoder.meta.version
            || handle.key.layout != self.layout.version
        {
            return Err(LayoutError::StaleContinuation);
        }
        let ink = handle.ink;
        if let Ink::Payload(span) = ink
            && (span.start > span.end || span.end > self.decoder.input.len())
        {
            return Err(LayoutError::InvalidRange);
        }
        Ok(MathInkCursor {
            owner: self.owner,
            ink,
            offset: 0,
            document: self.document,
            object: self.decoder.meta.id,
            epoch: self.decoder.meta.epoch,
            version: self.decoder.meta.version,
            layout: self.layout.version,
        })
    }
    pub(crate) fn invalidate_payloads(&self) {
        self.shaper.live.set(false);
    }
    pub(crate) fn text_payload(
        &self,
        handle: MathInk,
    ) -> Result<crate::render::payload::TextPayload, LayoutError> {
        let _ = self.ink_cursor(handle)?;
        match handle.ink {
            Ink::Glyph(id) => {
                let store = self.shaper.store.as_ref().unwrap().clone();
                let g = store.borrow().glyph(id);
                Ok(crate::render::payload::TextPayload::math(
                    MathPayload::new(store, id),
                    g.width,
                    g.kind,
                    g.first,
                    g.whitespace,
                ))
            }
            Ink::Static(text) => Ok(text.into()),
            Ink::Space(1) => Ok(" ".into()),
            _ => Err(LayoutError::InvalidRange),
        }
    }
    fn read_ink(&self, ink: Ink, offset: usize, b: &mut Budget, out: &mut [u8; 4]) -> usize {
        let len = match ink {
            Ink::Glyph(id) => {
                let store = self.shaper.store.as_ref().unwrap().borrow();
                let g = store.glyph(id);
                g.end - g.start
            }
            Ink::Glyphs(_) => unreachable!("only emitted glyph handles are readable"),
            Ink::Payload(s) => s.len(),
            Ink::Static(s) => s.len(),
            Ink::Space(n) => n,
        };
        let n = len.saturating_sub(offset).min(4).min(b.remaining().bytes);
        if n == 0
            || !b.charge(WorkLimit {
                queries: 1,
                objects: 1,
                bytes: n,
                ..WorkLimit::default()
            })
        {
            return 0;
        }
        for (at, slot) in out.iter_mut().enumerate().take(n) {
            *slot = match ink {
                Ink::Glyph(id) => {
                    let store = self.shaper.store.as_ref().unwrap().borrow();
                    store.byte(store.glyph(id).start + offset + at)
                }
                Ink::Glyphs(_) => unreachable!(),
                Ink::Payload(s) => self.decoder.input.byte(s.start + offset + at),
                Ink::Static(s) => s.as_bytes()[offset + at],
                Ink::Space(_) => b' ',
            }
        }
        n
    }
    pub fn into_retirement(self) -> MathRetirement {
        MathRetirement {
            engine: self,
            phase: 0,
            pub_steps: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InkReadStep {
    Bytes(usize),
    BudgetExhausted,
    Complete,
}
pub struct MathInkCursor {
    owner: EngineOwner,
    ink: Ink,
    offset: usize,
    document: DocumentId,
    object: MathObjectId,
    epoch: u64,
    version: u64,
    layout: u64,
}
impl MathInkCursor {
    /// Bytes may split a UTF-8 encoding. Keep the complete source atom and
    /// continue collection before passing its glyph to the shared Cell writer.
    pub fn advance(
        &mut self,
        engine: &MathEngine,
        document: &Document,
        budget: &mut Budget,
        out: &mut [u8; 4],
    ) -> Result<InkReadStep, LayoutError> {
        if self.owner != engine.owner {
            return Err(LayoutError::StaleContinuation);
        }
        if budget.remaining().queries == 0 || budget.remaining().objects == 0 {
            return Ok(InkReadStep::BudgetExhausted);
        }
        if document.id() != self.document || engine.document != self.document {
            return Err(LayoutError::WrongDocument);
        }
        let meta = document
            .math_object(self.object)
            .ok_or(LayoutError::StaleContinuation)?;
        if meta.epoch != self.epoch
            || meta.version != self.version
            || engine.decoder.meta.id != self.object
            || engine.layout.version != self.layout
        {
            return Err(LayoutError::StaleContinuation);
        }
        let len = match self.ink {
            Ink::Glyph(id) => {
                let store = engine.shaper.store.as_ref().unwrap().borrow();
                let g = store.glyph(id);
                g.end - g.start
            }
            Ink::Glyphs(_) => unreachable!(),
            Ink::Payload(span) => span.len(),
            Ink::Static(text) => text.len(),
            Ink::Space(n) => n,
        };
        if self.offset == len {
            if !budget.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..WorkLimit::default()
            }) {
                return Ok(InkReadStep::BudgetExhausted);
            }
            return Ok(InkReadStep::Complete);
        }
        let copied = engine.read_ink(self.ink, self.offset, budget, out);
        if copied == 0 {
            let charged = budget.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..WorkLimit::default()
            });
            debug_assert!(charged);
            return Ok(InkReadStep::BudgetExhausted);
        }
        self.offset += copied;
        Ok(InkReadStep::Bytes(copied))
    }
}

pub struct MathPaint {
    owner: EngineOwner,
    inner: paint::Paint,
    document: DocumentId,
    epoch: u64,
    version: u64,
}
impl MathPaint {
    pub fn advance(
        &mut self,
        engine: &MathEngine,
        document: &Document,
        b: &mut Budget,
    ) -> Result<PaintStep, LayoutError> {
        if self.owner != engine.owner {
            return Err(LayoutError::StaleContinuation);
        }
        if b.remaining().queries == 0 || b.remaining().objects == 0 {
            return Ok(PaintStep::BudgetExhausted);
        }
        if self.document != document.id() || self.document != engine.document {
            return Err(LayoutError::WrongDocument);
        }
        let now = document
            .math_object(engine.decoder.meta.id)
            .ok_or(LayoutError::StaleContinuation)?;
        if !engine.ready
            || now.epoch != self.epoch
            || now.version != self.version
            || self.inner.version != engine.layout.version
        {
            return Err(LayoutError::StaleContinuation);
        }
        let before = b.used().queries;
        let step = self.inner.step(
            &engine.layout,
            &engine.decoder.input,
            engine.shaper.store.as_ref(),
            b,
        );
        if b.used().queries == before {
            let charged = b.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..WorkLimit::default()
            });
            debug_assert!(charged);
            self.inner.work.validation_steps += 1;
        }
        Ok(match step {
            PaintStep::Cell(mut cell) => {
                if cell.source.is_empty() {
                    cell.source = now.content
                }
                PaintStep::Cell(cell)
            }
            other => other,
        })
    }
    pub fn work(&self) -> PaintWork {
        self.inner.work
    }
    pub fn storage(&self) -> StorageWork {
        self.inner.storage()
    }
    pub fn retire_step(&mut self, b: &mut Budget) -> bool {
        self.inner.retire_step(b)
    }
}
/// Dropping a large task is an explicit non-frame boundary. Frame hosts move
/// ownership here and drain pages/index entries under the SAME shared budget.
pub struct MathRetirement {
    engine: MathEngine,
    phase: u8,
    pub pub_steps: usize,
}
impl MathRetirement {
    pub fn advance(&mut self, b: &mut Budget) -> bool {
        if self.phase == 4 {
            return true;
        }
        if !b.charge(WorkLimit {
            queries: 1,
            objects: 1,
            ..WorkLimit::default()
        }) {
            return false;
        }
        self.pub_steps += 1;
        let complete = match self.phase {
            0 => self.engine.layout.retire_one(),
            1 => self.engine.syntax.retire_one(),
            2 => self.engine.decoder.retire_one(),
            _ => self.engine.shaper.retire_one(),
        };
        if complete {
            self.phase += 1
        }
        self.phase == 4
    }
    pub fn stats(&self) -> MathStats {
        self.engine.stats()
    }
}

pub struct MathLocator {
    owner: EngineOwner,
    inner: locate::Locator,
    document: DocumentId,
    epoch: u64,
    version: u64,
    layout: u64,
}
impl MathLocator {
    pub fn advance(
        &mut self,
        engine: &MathEngine,
        document: &Document,
        b: &mut Budget,
    ) -> Result<LocateStep, LayoutError> {
        if self.owner != engine.owner {
            return Err(LayoutError::StaleContinuation);
        }
        if b.remaining().queries == 0 || b.remaining().objects == 0 {
            return Ok(LocateStep::BudgetExhausted);
        }
        if document.id() != self.document || engine.document != self.document {
            return Err(LayoutError::WrongDocument);
        }
        let meta = document
            .math_object(engine.decoder.meta.id)
            .ok_or(LayoutError::StaleContinuation)?;
        if !engine.ready
            || meta.epoch != self.epoch
            || meta.version != self.version
            || engine.layout.version != self.layout
        {
            return Err(LayoutError::StaleContinuation);
        }
        Ok(match self.inner.step(&engine.layout, b) {
            LocateStep::Position(mut position) => {
                if position.cell_source.is_empty() {
                    position.cell_source = meta.content;
                }
                LocateStep::Position(position)
            }
            other => other,
        })
    }
    pub fn steps(&self) -> usize {
        self.inner.steps
    }
}
