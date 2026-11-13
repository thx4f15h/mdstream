//! Hosted, input-stage Markdown parsing. Original source is never rewritten.
#[path = "footnote_body_query.rs"]
mod query;
pub use query::*;
#[path = "footnote_body_update.rs"]
mod updates;
use crate::{
    BlockId, Budget, ChangeKind, Completion, Document, DocumentId, FootnoteId, LayoutError, Span,
    SyntaxState, Update, WorkLimit,
};
use std::sync::Arc;
pub use updates::BodyUpdate;

/// Stable namespace of a winning footnote definition in its original document.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BodyKey {
    pub(crate) document: DocumentId,
    pub(crate) symbol: FootnoteId,
    pub(crate) definition: BlockId,
}
impl BodyKey {
    pub fn document(self) -> DocumentId {
        self.document
    }
    pub fn symbol(self) -> FootnoteId {
        self.symbol
    }
    pub fn definition(self) -> BlockId {
        self.definition
    }
}
/// Byte coordinates in one append-only, deprefixed Markdown body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BodySpan {
    pub body: BodyKey,
    pub span: Span,
}
/// Stable local object identity, explicitly qualified by its body namespace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BodyId<I> {
    pub(crate) body: BodyKey,
    pub(crate) local: I,
}
impl<I: Copy> BodyId<I> {
    pub fn body(self) -> BodyKey {
        self.body
    }
    pub fn local(self) -> I {
        self.local
    }
}
/// A selection/caret source address; projected coordinates are never root spans.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyAddress {
    Original { document: DocumentId, span: Span },
    Projected(BodySpan),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyMappingKind {
    Raw,
    Hidden,
    VirtualTab,
    Pending,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BodyMapping {
    pub kind: BodyMappingKind,
    pub original: Span,
    pub projected: BodySpan,
}
#[derive(Clone, Copy)]
struct Piece {
    kind: BodyMappingKind,
    original: Span,
    projected: Span,
}

/// Input work only; the actual reused parser counters are available from BodyView.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BodyWork {
    pub source_bytes: u64,
    pub projected_bytes: u64,
    pub mapping_steps: u64,
    pub publication_steps: u64,
    pub parser_calls: u64,
    pub mapping_records: u64,
    pub address_steps: u64,
    pub engine: crate::Work,
}
#[derive(Default)]
pub(crate) struct Bodies {
    entries: Vec<Body>,
    pub(crate) work: BodyWork,
}
struct Body {
    key: BodyKey,
    engine: Box<Document>,
    pieces: Vec<Piece>,
    processed: usize,
    tail: Option<Span>,
    link_symbols: Vec<crate::SymbolId>,
    note_symbols: Vec<FootnoteId>,
    pending_update: Option<Box<Update>>,
}
impl Body {
    fn piece(&mut self, kind: BodyMappingKind, source: Span, text: &str, work: &mut BodyWork) {
        if source.is_empty() {
            return;
        }
        let start = self.engine.source.len();
        let piece = Piece {
            kind,
            original: source,
            projected: Span::new(start, start + text.len()),
        };
        if let Some(last) = self.pieces.last_mut()
            && last.kind == kind
            && kind != BodyMappingKind::VirtualTab
            && last.original.end == source.start
            && last.projected.end == start
        {
            last.original.end = source.end;
            last.projected.end = piece.projected.end;
        } else {
            self.pieces.push(piece);
            work.mapping_records += 1;
        }
        work.mapping_steps += 1;
        if !text.is_empty() {
            work.projected_bytes += text.len() as u64;
            work.parser_calls += 1;
            let before = self.engine.work;
            let update = self.engine.append(text).expect("active body accepts input");
            self.record_update(update);
            add_engine_work(&mut work.engine, before, self.engine.work);
        }
    }
    fn record_update(&mut self, update: Update) {
        if let Some(current) = &mut self.pending_update {
            current.merge(update).expect("consecutive parser input");
        } else {
            self.pending_update = Some(Box::new(update));
        }
    }
    fn flush_tail(&mut self, source: &str, internal: bool, work: &mut BodyWork) {
        if let Some(tail) = self.tail.take() {
            let raw = &source[tail.range()];
            // Bare CR is content. A final physical LF/CRLF is not body content.
            let literal = internal || raw == "\r";
            self.piece(
                if literal {
                    BodyMappingKind::Raw
                } else {
                    BodyMappingKind::Hidden
                },
                tail,
                if literal { raw } else { "" },
                work,
            );
        }
    }
    fn prefix(&mut self, source: &str, start: usize, end: usize, spaces: u8, work: &mut BodyWork) {
        self.flush_tail(source, true, work);
        let hidden_end = end - usize::from(spaces > 0);
        self.piece(
            BodyMappingKind::Hidden,
            Span::new(start, hidden_end),
            "",
            work,
        );
        if spaces > 0 {
            self.piece(
                BodyMappingKind::VirtualTab,
                Span::new(hidden_end, end),
                &"   "[..usize::from(spaces)],
                work,
            );
        }
        self.processed = end;
    }
    fn feed(&mut self, source: &str, span: Span, work: &mut BodyWork) {
        let mut at = span.start.max(self.processed);
        work.source_bytes += span.end.saturating_sub(at) as u64;
        while at < span.end {
            if let Some(tail) = self.tail {
                if &source[tail.range()] == "\r" && source.as_bytes()[at] == b'\n' {
                    self.tail = Some(Span::new(tail.start, at + 1));
                    at += 1;
                    continue;
                }
                self.flush_tail(source, true, work);
            }
            let n = source.as_bytes()[at..span.end]
                .iter()
                .position(|b| matches!(b, b'\r' | b'\n'))
                .unwrap_or(span.end - at);
            if n > 0 {
                self.piece(
                    BodyMappingKind::Raw,
                    Span::new(at, at + n),
                    &source[at..at + n],
                    work,
                );
                at += n;
            } else {
                self.tail = Some(Span::new(at, at + 1));
                at += 1;
            }
        }
        self.processed = span.end;
    }
    // Input-stage envelope conversion: endpoints only, no body/ancestor scan.
    fn original(&self, span: Span, work: &mut BodyWork) -> Span {
        fn point(pieces: &[Piece], at: usize, end: bool, work: &mut BodyWork) -> usize {
            let i = pieces.partition_point(|p| {
                work.address_steps += 1;
                if end {
                    p.projected.end < at
                } else {
                    p.projected.end <= at
                }
            });
            let p = pieces
                .get(i)
                .or_else(|| pieces.last())
                .expect("body has a mapped source");
            if p.kind == BodyMappingKind::Raw {
                p.original.start + at.saturating_sub(p.projected.start).min(p.original.len())
            } else if end {
                p.original.end
            } else {
                p.original.start
            }
        }
        let start = point(&self.pieces, span.start, false, work);
        if span.is_empty() {
            Span::new(start, start)
        } else {
            Span::new(start, point(&self.pieces, span.end, true, work))
        }
    }
    fn title_mapping(
        &self,
        span: Span,
        local: &[Span],
        work: &mut BodyWork,
    ) -> (Vec<Span>, Vec<(Span, u8)>) {
        fn push(gaps: &mut Vec<Span>, s: Span) {
            if s.is_empty() {
                return;
            }
            if let Some(last) = gaps.last_mut()
                && s.start <= last.end
            {
                last.end = last.end.max(s.end);
            } else {
                gaps.push(s);
            }
        }
        let envelope = self.original(span, work);
        let mut gaps = Vec::new();
        let mut spaces = Vec::new();
        let mut j = 0;
        let first = self.pieces.partition_point(|p| {
            work.address_steps += 1;
            p.original.end <= envelope.start
        });
        for p in &self.pieces[first..] {
            work.address_steps += 1;
            if p.original.start >= envelope.end {
                break;
            }
            if p.kind == BodyMappingKind::Hidden {
                push(
                    &mut gaps,
                    Span::new(
                        p.original.start.max(envelope.start),
                        p.original.end.min(envelope.end),
                    ),
                );
                continue;
            }
            let a = p.projected.start.max(span.start);
            let b = p.projected.end.min(span.end);
            if a >= b {
                continue;
            }
            while j < local.len() && local[j].end <= a {
                j += 1;
            }
            let mut k = j;
            let mut omitted = 0;
            while k < local.len() && local[k].start < b {
                let x = local[k].start.max(a);
                let y = local[k].end.min(b);
                if x < y {
                    omitted += y - x;
                    if p.kind == BodyMappingKind::Raw {
                        push(
                            &mut gaps,
                            Span::new(
                                p.original.start + x - p.projected.start,
                                p.original.start + y - p.projected.start,
                            ),
                        );
                    }
                }
                k += 1;
            }
            if p.kind == BodyMappingKind::VirtualTab {
                let visible = b - a - omitted;
                if visible == 0 {
                    push(&mut gaps, p.original);
                } else {
                    spaces.push((p.original, visible as u8));
                }
            }
        }
        (gaps, spaces)
    }
}
impl Document {
    pub(crate) fn start_body(
        &mut self,
        symbol: FootnoteId,
        block: BlockId,
        source: Span,
        content: Span,
        update: &mut Update,
    ) {
        if self.footnotes.entries[symbol.0]
            .definition
            .is_none_or(|d| d.block != block)
        {
            return;
        }
        let mut engine = Box::new(Document::new());
        engine.host_clock = Some(Arc::clone(&self.clock));
        if self.shape.enabled() {
            engine
                .enable_display_shape_effects()
                .expect("body subscription precedes projected input");
        }
        engine.references.journal = Some(Vec::new());
        engine.footnotes.journal = Some(Vec::new());
        let mut body = Body {
            key: BodyKey {
                document: self.id(),
                symbol,
                definition: block,
            },
            engine,
            pieces: Vec::new(),
            processed: content.start,
            tail: None,
            link_symbols: Vec::new(),
            note_symbols: Vec::new(),
            pending_update: None,
        };
        body.piece(
            BodyMappingKind::Hidden,
            Span::new(source.start, content.start),
            "",
            &mut self.bodies.work,
        );
        let i = self.bodies.entries.len();
        self.footnotes.body_indices[symbol.0] = Some(i);
        self.bodies.entries.push(body);
        self.feed_body(symbol, source, update);
    }
    pub(crate) fn prefix_body(
        &mut self,
        symbol: FootnoteId,
        block: BlockId,
        start: usize,
        end: usize,
        spaces: u8,
        update: &mut Update,
    ) {
        let Some(i) = self.footnotes.body_indices[symbol.0] else {
            return;
        };
        if self.bodies.entries[i].key.definition != block {
            return;
        }
        self.bodies.entries[i].prefix(&self.source, start, end, spaces, &mut self.bodies.work);
        self.drain_body(i, update);
    }
    pub(crate) fn feed_body(&mut self, symbol: FootnoteId, span: Span, update: &mut Update) {
        let Some(i) = self.footnotes.body_indices[symbol.0] else {
            return;
        };
        if self.current != Some(self.bodies.entries[i].key.definition) {
            return;
        }
        self.bodies.entries[i].feed(&self.source, span, &mut self.bodies.work);
        self.drain_body(i, update);
    }
    pub(crate) fn seal_body(&mut self, symbol: FootnoteId, block: BlockId, update: &mut Update) {
        let Some(i) = self.footnotes.body_indices[symbol.0] else {
            return;
        };
        let body = &mut self.bodies.entries[i];
        if body.key.definition != block || body.engine.finished {
            return;
        }
        body.flush_tail(&self.source, false, &mut self.bodies.work);
        let before = body.engine.work;
        let local_update = body.engine.finish();
        body.record_update(local_update);
        self.bodies.work.parser_calls += 1;
        add_engine_work(&mut self.bodies.work.engine, before, body.engine.work);
        self.drain_body(i, update);
    }
    fn drain_body(&mut self, i: usize, update: &mut Update) {
        let body = &mut self.bodies.entries[i];
        if let Some(local) = body.pending_update.take() {
            let delta = BodyUpdate {
                body: body.key,
                local,
            };
            if let Some(last) = update.body_updates.last_mut()
                && last.body == body.key
            {
                last.merge(delta)
                    .expect("one body's input remains consecutive");
            } else {
                update.body_updates.push(delta);
            }
        }
        for local in body.link_symbols.len()..body.engine.references.entries.len() {
            let global = self
                .references
                .intern_key(&body.engine.references.entries[local].label);
            body.engine.references.bindings[local] =
                Some(Arc::clone(&self.references.slots[global.0]));
            body.link_symbols.push(global);
            self.bodies.work.publication_steps += 1;
        }
        for local in body.note_symbols.len()..body.engine.footnotes.entries.len() {
            let global = self
                .footnotes
                .intern(&body.engine.footnotes.entries[local].label, &mut self.work);
            body.engine.footnotes.bindings[local] =
                Some(Arc::clone(&self.footnotes.slots[global.0]));
            body.note_symbols.push(global);
            self.bodies.work.publication_steps += 1;
        }
        let journal = std::mem::take(body.engine.references.journal.as_mut().unwrap());
        for event in journal {
            self.bodies.work.publication_steps += 1;
            match event {
                crate::references::Event::Use(local, at) => {
                    let global = body.link_symbols[local.0];
                    let at = body
                        .original(Span::new(at, at), &mut self.bodies.work)
                        .start;
                    let first = &mut self.references.entries[global.0].first_use;
                    *first = Some(first.map_or(at, |old| old.min(at)));
                }
                crate::references::Event::Definition {
                    symbol,
                    position,
                    destination,
                    title,
                    gaps,
                } => {
                    let global = body.link_symbols[symbol.0];
                    let position = body
                        .original(Span::new(position, position), &mut self.bodies.work)
                        .start;
                    let (mapped_gaps, mapped_spaces) = title.map_or_else(
                        || (Vec::new(), Vec::new()),
                        |s| body.title_mapping(s, &gaps, &mut self.bodies.work),
                    );
                    if self.references.define(
                        global,
                        position,
                        body.original(destination, &mut self.bodies.work),
                        title.map(|s| body.original(s, &mut self.bodies.work)),
                        mapped_gaps,
                        self.revision,
                    ) {
                        self.references.title_spaces[global.0] = mapped_spaces;
                        update.references_changed.push(global);
                        let start = self.references.entries[global.0]
                            .first_use
                            .unwrap_or(position);
                        update.change(Span::new(start, self.source.len()), ChangeKind::Reference);
                        update.change(Span::new(start, self.source.len()), ChangeKind::Visibility);
                    }
                }
                crate::references::Event::Title {
                    symbol,
                    position,
                    title,
                    gaps,
                } => {
                    let global = body.link_symbols[symbol.0];
                    let position = body
                        .original(Span::new(position, position), &mut self.bodies.work)
                        .start;
                    if self.references.positions[global.0] == Some(position) {
                        let (mapped_gaps, mapped_spaces) =
                            body.title_mapping(title, &gaps, &mut self.bodies.work);
                        self.references.define_title(
                            global,
                            body.original(title, &mut self.bodies.work),
                            mapped_gaps,
                            self.revision,
                        );
                        self.references.title_spaces[global.0] = mapped_spaces;
                        update.references_changed.push(global);
                        let start = self.references.entries[global.0]
                            .first_use
                            .unwrap_or(position);
                        update.change(Span::new(start, self.source.len()), ChangeKind::Reference);
                    }
                }
            }
        }
        let journal = std::mem::take(body.engine.footnotes.journal.as_mut().unwrap());
        for event in journal {
            self.bodies.work.publication_steps += 1;
            match event {
                crate::footnotes::Event::Begin => self.footnotes.begin_reference_line(),
                crate::footnotes::Event::Commit => {
                    self.bodies.work.publication_steps += self.footnotes.commit_reference_line();
                }
                crate::footnotes::Event::Rollback => {
                    self.bodies.work.publication_steps +=
                        self.footnotes.rollback_reference_line(update);
                }
                crate::footnotes::Event::Reference(local, source) => {
                    let global = body.note_symbols[local.0];
                    self.footnotes.reference_symbol(
                        global,
                        body.original(source, &mut self.bodies.work),
                        update,
                    );
                }
            }
        }
    }
    /// O(1) read-only view of already parsed winning Markdown, with host-global symbols.
    pub fn footnote_body(&self, symbol: FootnoteId) -> Option<BodyView<'_>> {
        let i = self
            .footnotes
            .body_indices
            .get(symbol.0)
            .copied()
            .flatten()?;
        Some(BodyView {
            host: self,
            body: &self.bodies.entries[i],
        })
    }
    pub fn footnote_body_work(&self) -> BodyWork {
        self.bodies.work
    }
}

/// A borrowed body view. It never parses, finishes, or clones global tables.
#[derive(Clone, Copy)]
pub struct BodyView<'a> {
    host: &'a Document,
    body: &'a Body,
}
impl<'a> BodyView<'a> {
    pub fn key(self) -> BodyKey {
        self.body.key
    }
    pub fn range(self) -> BodySpan {
        BodySpan {
            body: self.key(),
            span: Span::new(0, self.body.engine.source.len()),
        }
    }
    pub fn state(self) -> SyntaxState {
        if self.body.engine.finished {
            SyntaxState::Sealed
        } else {
            SyntaxState::Growing
        }
    }
    pub fn work(self) -> crate::Work {
        self.body.engine.work()
    }
    pub fn mapping_count(self) -> usize {
        self.body.pieces.len()
    }
    fn check(self, key: BodyKey) -> Result<(), LayoutError> {
        if key.document != self.host.id() {
            Err(LayoutError::WrongDocument)
        } else if key != self.key() {
            Err(LayoutError::InvalidRange)
        } else {
            Ok(())
        }
    }
    pub fn display_cursor(self) -> BodyDisplayCursor {
        self.display_cursor_in(self.range())
            .expect("body range is valid")
    }
    pub fn display_cursor_in(self, span: BodySpan) -> Result<BodyDisplayCursor, LayoutError> {
        Ok(BodyDisplayCursor {
            key: self.key(),
            inner: self.body.engine.display_cursor(self.span(span)?).unwrap(),
        })
    }
}
pub struct BodyDisplayCursor {
    key: BodyKey,
    inner: crate::DisplayCursor,
}
impl BodyDisplayCursor {
    pub fn next<'a>(
        &mut self,
        view: BodyView<'a>,
        budget: &mut Budget,
    ) -> Result<BodyDisplayStep<'a>, LayoutError> {
        view.check(self.key)?;
        self.inner
            .next(&view.body.engine, budget)
            .map(|step| BodyDisplayStep::from_step(self.key, step))
    }
    pub fn take_link_resolver(&mut self) -> crate::LinkResolver {
        self.inner.take_link_resolver()
    }
    pub fn retire(&mut self, budget: &mut Budget) -> Completion {
        self.inner.retire(budget)
    }
}

/// Bias at omitted prefixes and inside an expanded single-byte tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyBias {
    Left,
    Right,
}
/// One paid mapping step, a pause, or a completed range/caret query.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyMapStep {
    Mapping(BodyMapping),
    BudgetExhausted,
    Complete,
}
/// Owned, revision-bound mapping task. Each seek comparison and output is a
/// separate charged metadata step. No source text or ancestor is scanned.
pub struct BodyMapTask {
    key: BodyKey,
    revision: u64,
    range: Span,
    reverse: bool,
    bias: Option<BodyBias>,
    lo: usize,
    hi: usize,
    at: Option<usize>,
    done: bool,
}
impl BodyView<'_> {
    fn piece(self, i: usize) -> Option<Piece> {
        self.body.pieces.get(i).copied().or_else(|| {
            (i == self.body.pieces.len())
                .then(|| {
                    self.body.tail.map(|original| Piece {
                        kind: BodyMappingKind::Pending,
                        original,
                        projected: Span::new(
                            self.body.engine.source.len(),
                            self.body.engine.source.len(),
                        ),
                    })
                })
                .flatten()
        })
    }
    /// Construct a range relation task without traversing indexes or evaluating symbols.
    pub fn map(self, address: BodyAddress) -> Result<BodyMapTask, LayoutError> {
        self.map_task(address, None)
    }
    /// Map an empty caret address using an explicit bias, without decoding UTF-8 columns.
    pub fn map_caret(
        self,
        address: BodyAddress,
        bias: BodyBias,
    ) -> Result<BodyMapTask, LayoutError> {
        self.map_task(address, Some(bias))
    }
    fn map_task(
        self,
        address: BodyAddress,
        bias: Option<BodyBias>,
    ) -> Result<BodyMapTask, LayoutError> {
        let (range, reverse) = match address {
            BodyAddress::Original { document, span } => {
                if document != self.host.id() {
                    return Err(LayoutError::WrongDocument);
                }
                self.host.text(span).ok_or(LayoutError::InvalidRange)?;
                let start = self.host.blocks[self.body.key.definition.0].source.start;
                if span.start < start || span.end > self.body.processed {
                    return Err(LayoutError::InvalidRange);
                }
                (span, true)
            }
            BodyAddress::Projected(span) => {
                self.check(span.body)?;
                self.body
                    .engine
                    .text(span.span)
                    .ok_or(LayoutError::InvalidRange)?;
                (span.span, false)
            }
        };
        if bias.is_some() && !range.is_empty() {
            return Err(LayoutError::InvalidRange);
        }
        Ok(BodyMapTask {
            key: self.key(),
            revision: self.host.revision(),
            range,
            reverse,
            bias,
            lo: 0,
            hi: self.body.pieces.len() + usize::from(self.body.tail.is_some()),
            at: None,
            done: range.is_empty() && bias.is_none(),
        })
    }
}
impl BodyMapTask {
    pub fn advance(
        &mut self,
        view: BodyView<'_>,
        budget: &mut Budget,
    ) -> Result<BodyMapStep, LayoutError> {
        view.check(self.key)?;
        if view.host.revision() != self.revision {
            return Err(LayoutError::StaleContinuation);
        }
        loop {
            if self.done {
                return Ok(BodyMapStep::Complete);
            }
            if !budget.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            }) {
                return Ok(BodyMapStep::BudgetExhausted);
            }
            if self.at.is_none() {
                if self.lo < self.hi {
                    let mid = (self.lo + self.hi) / 2;
                    let p = view.piece(mid).unwrap();
                    let s = if self.reverse {
                        p.original
                    } else {
                        p.projected
                    };
                    let before = if self.bias == Some(BodyBias::Left) {
                        s.end < self.range.start
                    } else {
                        s.end <= self.range.start
                    };
                    if before {
                        self.lo = mid + 1;
                    } else {
                        self.hi = mid;
                    }
                } else {
                    self.at = Some(self.lo);
                }
                continue;
            }
            let i = self.at.unwrap();
            let p = if let Some(p) = view.piece(i) {
                p
            } else if self.bias.is_some() {
                view.piece(i.saturating_sub(1))
                    .ok_or(LayoutError::InvalidRange)?
            } else {
                self.done = true;
                return Ok(BodyMapStep::Complete);
            };
            let domain = if self.reverse {
                p.original
            } else {
                p.projected
            };
            if let Some(bias) = self.bias {
                let point = self.range.start;
                let projected;
                let original;
                if self.reverse {
                    original = Span::new(point, point);
                    let at = if p.kind == BodyMappingKind::Raw {
                        p.projected.start
                            + point.saturating_sub(p.original.start).min(p.original.len())
                    } else if point >= p.original.end || bias == BodyBias::Right {
                        p.projected.end
                    } else {
                        p.projected.start
                    };
                    projected = Span::new(at, at);
                } else {
                    projected = Span::new(point, point);
                    let at = if p.kind == BodyMappingKind::Raw {
                        p.original.start
                            + point
                                .saturating_sub(p.projected.start)
                                .min(p.projected.len())
                    } else if bias == BodyBias::Right {
                        p.original.end
                    } else {
                        p.original.start
                    };
                    original = Span::new(at, at);
                }
                self.done = true;
                return Ok(BodyMapStep::Mapping(BodyMapping {
                    kind: p.kind,
                    original,
                    projected: BodySpan {
                        body: self.key,
                        span: projected,
                    },
                }));
            }
            if domain.start >= self.range.end {
                self.done = true;
                return Ok(BodyMapStep::Complete);
            }
            self.at = Some(i + 1);
            let a = domain.start.max(self.range.start);
            let b = domain.end.min(self.range.end);
            if a >= b {
                continue;
            }
            let (original, projected) = if p.kind == BodyMappingKind::Raw {
                if self.reverse {
                    (
                        Span::new(a, b),
                        Span::new(
                            p.projected.start + a - p.original.start,
                            p.projected.start + b - p.original.start,
                        ),
                    )
                } else {
                    (
                        Span::new(
                            p.original.start + a - p.projected.start,
                            p.original.start + b - p.projected.start,
                        ),
                        Span::new(a, b),
                    )
                }
            } else if self.reverse {
                (Span::new(a, b), p.projected)
            } else {
                (p.original, Span::new(a, b))
            };
            return Ok(BodyMapStep::Mapping(BodyMapping {
                kind: p.kind,
                original,
                projected: BodySpan {
                    body: self.key,
                    span: projected,
                },
            }));
        }
    }
}

impl<'a> BodyView<'a> {
    /// Current child-parser blank ownership, qualified into this body's source.
    pub fn indented_code_pending(self) -> Option<BodyObject<crate::PendingCodeBlanks>> {
        self.body
            .engine
            .indented_code_pending()
            .map(|value| BodyObject {
                body: self.key(),
                value,
            })
    }

    pub fn code_blank_storage(self) -> crate::CodeBlankStorage {
        self.body.engine.code_blank_storage()
    }

    fn span(self, span: BodySpan) -> Result<Span, LayoutError> {
        self.check(span.body)?;
        self.body
            .engine
            .text(span.span)
            .ok_or(LayoutError::InvalidRange)?;
        Ok(span.span)
    }
    pub fn qualify<I: Copy>(self, local: I) -> BodyId<I> {
        BodyId {
            body: self.key(),
            local,
        }
    }
    pub fn block(
        self,
        id: BodyId<BlockId>,
    ) -> Result<Option<BodyObject<&'a crate::Block>>, LayoutError> {
        self.check(id.body)?;
        Ok(self.body.engine.block(id.local).map(|value| BodyObject {
            body: self.key(),
            value,
        }))
    }
    pub fn container(
        self,
        id: BodyId<crate::ContainerId>,
    ) -> Result<Option<BodyObject<crate::Container>>, LayoutError> {
        self.check(id.body)?;
        Ok(self
            .body
            .engine
            .container(id.local)
            .map(|value| BodyObject {
                body: self.key(),
                value,
            }))
    }
    pub fn heading(
        self,
        id: BodyId<crate::HeadingId>,
    ) -> Result<Option<BodyObject<&'a crate::Heading>>, LayoutError> {
        self.check(id.body)?;
        Ok(self
            .body
            .engine
            .headings()
            .get(id.local)
            .map(|value| BodyObject {
                body: self.key(),
                value,
            }))
    }
    pub fn table(
        self,
        id: BodyId<crate::TableId>,
    ) -> Result<Option<BodyObject<&'a crate::Table>>, LayoutError> {
        self.check(id.body)?;
        Ok(self
            .body
            .engine
            .tables()
            .get(id.local.0)
            .map(|value| BodyObject {
                body: self.key(),
                value,
            }))
    }
    pub fn block_count(self) -> usize {
        self.body.engine.blocks.len()
    }
    pub fn container_count(self) -> usize {
        self.body.engine.container_count()
    }
    pub fn heading_count(self) -> usize {
        self.body.engine.headings().entries().len()
    }
    pub fn table_count(self) -> usize {
        self.body.engine.tables.len()
    }
    pub fn link_resolver(self) -> crate::LinkResolver {
        self.body.engine.link_resolver()
    }
    pub fn link_info(
        self,
        id: BodyId<crate::LinkId>,
    ) -> Result<Option<BodyObject<&'a crate::LinkInfo>>, LayoutError> {
        self.check(id.body)?;
        Ok(self
            .body
            .engine
            .link_info(id.local)
            .map(|value| BodyObject {
                body: self.key(),
                value,
            }))
    }
    pub fn link_window(self, span: BodySpan) -> Result<BodyLinkWindow, LayoutError> {
        Ok(BodyLinkWindow {
            key: self.key(),
            inner: self.body.engine.link_window(self.span(span)?),
        })
    }
    pub fn link_activity(
        self,
        id: BodyId<crate::LinkId>,
    ) -> Result<Option<BodyLinkActivity>, LayoutError> {
        self.check(id.body)?;
        Ok(self
            .body
            .engine
            .link_activity(id.local)
            .map(|inner| BodyLinkActivity {
                key: self.key(),
                inner,
            }))
    }
    pub fn style_path(self, at: BodySpan) -> Result<Option<BodyStylePath>, LayoutError> {
        let span = self.span(at)?;
        Ok(self
            .body
            .engine
            .style_path(span.start)
            .map(|inner| BodyStylePath {
                key: self.key(),
                inner,
            }))
    }
    pub fn math(
        self,
        id: BodyId<crate::MathId>,
    ) -> Result<Option<BodyObject<&'a crate::Math>>, LayoutError> {
        self.check(id.body)?;
        Ok(self.body.engine.math(id.local).map(|value| BodyObject {
            body: self.key(),
            value,
        }))
    }
    pub fn maths_in(self, span: BodySpan) -> Result<BodyMathObjects<'a>, LayoutError> {
        Ok(BodyMathObjects {
            key: self.key(),
            inner: self.body.engine.maths_in(self.span(span)?).unwrap(),
        })
    }
    pub fn math_payload(
        self,
        id: BodyId<crate::MathId>,
    ) -> Result<Option<BodyMathPayload<'a>>, LayoutError> {
        self.check(id.body)?;
        Ok(self
            .body
            .engine
            .math_payload(id.local)
            .map(|inner| BodyMathPayload {
                key: self.key(),
                inner,
            }))
    }
    pub fn math_block_payload(
        self,
        id: BodyId<BlockId>,
    ) -> Result<Option<BodyMathPayload<'a>>, LayoutError> {
        self.check(id.body)?;
        Ok(self
            .body
            .engine
            .math_block_payload(id.local)
            .map(|inner| BodyMathPayload {
                key: self.key(),
                inner,
            }))
    }
    pub fn code_info(self, id: BodyId<BlockId>) -> Result<Option<BodyTextCursor<'a>>, LayoutError> {
        self.check(id.body)?;
        Ok(self
            .body
            .engine
            .code_info(id.local)
            .map(|inner| BodyTextCursor::projected(self.key(), self.host.id(), inner)))
    }
    pub fn footnote_references_in(
        self,
        span: BodySpan,
    ) -> Result<BodyFootnoteReferences<'a>, LayoutError> {
        Ok(BodyFootnoteReferences {
            key: self.key(),
            bindings: &self.body.note_symbols,
            inner: self
                .body
                .engine
                .footnote_references_in(self.span(span)?)
                .unwrap(),
        })
    }
    pub fn resolve_footnote(
        self,
        id: BodyId<crate::FootnoteReferenceId>,
        budget: &mut Budget,
    ) -> Result<BodyResolveStep, LayoutError> {
        self.check(id.body)?;
        if !budget.charge(WorkLimit {
            queries: 1,
            objects: 1,
            dependencies: 1,
            ..Default::default()
        }) {
            return Ok(BodyResolveStep::BudgetExhausted);
        }
        if self
            .body
            .engine
            .footnote_reference_state(id.local)
            .is_none()
        {
            return Ok(BodyResolveStep::Resolved(None));
        }
        let local = self.body.engine.footnotes.references[id.local.0].symbol;
        let symbol = self.body.note_symbols[local.0];
        let entry = &self.host.footnotes.entries[symbol.0];
        Ok(BodyResolveStep::Resolved(Some(crate::FootnoteResolution {
            symbol,
            number: entry.number.unwrap(),
            revision: entry.revision,
            target: entry.definition.map_or(
                if self.host.finished {
                    crate::FootnoteTarget::Missing
                } else {
                    crate::FootnoteTarget::Unresolved
                },
                crate::FootnoteTarget::Defined,
            ),
        })))
    }
    pub fn footnote_reference_state(
        self,
        id: BodyId<crate::FootnoteReferenceId>,
    ) -> Result<Option<SyntaxState>, LayoutError> {
        self.check(id.body)?;
        Ok(self.body.engine.footnote_reference_state(id.local))
    }
    pub fn next_footnote_reference(
        self,
        id: BodyId<crate::FootnoteReferenceId>,
    ) -> Result<Option<BodyId<crate::FootnoteReferenceId>>, LayoutError> {
        self.check(id.body)?;
        Ok(self
            .body
            .engine
            .next_footnote_reference(id.local)
            .map(|local| self.qualify(local)))
    }
}

pub struct BodyLinkWindow {
    key: BodyKey,
    inner: crate::LinkWindow,
}
#[derive(Debug)]
pub enum BodyLinkStep<'a> {
    Object(BodyId<crate::LinkId>, BodyObject<&'a crate::LinkInfo>),
    BudgetExhausted,
    Complete,
}
impl BodyLinkWindow {
    pub fn advance<'a>(
        &mut self,
        view: BodyView<'a>,
        budget: &mut Budget,
    ) -> Result<BodyLinkStep<'a>, LayoutError> {
        view.check(self.key)?;
        Ok(match self.inner.advance(&view.body.engine, budget)? {
            crate::LinkStep::Object(id, info) => BodyLinkStep::Object(
                view.qualify(id),
                BodyObject {
                    body: self.key,
                    value: info,
                },
            ),
            crate::LinkStep::BudgetExhausted => BodyLinkStep::BudgetExhausted,
            crate::LinkStep::Complete => BodyLinkStep::Complete,
        })
    }
}
pub struct BodyLinkActivity {
    key: BodyKey,
    inner: crate::LinkActivity,
}
impl BodyLinkActivity {
    pub fn advance(
        &mut self,
        view: BodyView<'_>,
        resolver: &mut crate::LinkResolver,
        budget: &mut Budget,
    ) -> Result<Option<crate::LinkDisposition>, LayoutError> {
        view.check(self.key)?;
        self.inner.advance_with(&view.body.engine, resolver, budget)
    }
}
pub struct BodyStylePath {
    key: BodyKey,
    inner: crate::StylePath,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyStyleStep {
    Pair(BodyId<crate::StylePairId>),
    BudgetExhausted,
    Complete,
}
impl BodyStylePath {
    pub fn advance(
        &mut self,
        view: BodyView<'_>,
        resolver: &mut crate::LinkResolver,
        budget: &mut Budget,
    ) -> Result<BodyStyleStep, LayoutError> {
        view.check(self.key)?;
        Ok(
            match self.inner.advance(&view.body.engine, resolver, budget)? {
                crate::StylePathStep::Pair(pair) => BodyStyleStep::Pair(view.qualify(pair)),
                crate::StylePathStep::BudgetExhausted => BodyStyleStep::BudgetExhausted,
                crate::StylePathStep::Complete => BodyStyleStep::Complete,
            },
        )
    }
    pub fn retire(&mut self, budget: &mut Budget) -> Completion {
        self.inner.retire(budget)
    }
}
pub type BodyMathObject = (BodyId<crate::MathId>, BodyObject<crate::Math>);
pub struct BodyMathObjects<'a> {
    key: BodyKey,
    inner: crate::MathObjects<'a>,
}
impl<'a> BodyMathObjects<'a> {
    pub fn advance(&mut self, budget: &mut Budget) -> Result<Option<BodyMathObject>, Completion> {
        Ok(self.inner.advance(budget)?.map(|(id, math)| {
            (
                BodyId {
                    body: self.key,
                    local: id,
                },
                BodyObject {
                    body: self.key,
                    value: math,
                },
            )
        }))
    }
}
pub struct BodyFootnoteReferences<'a> {
    key: BodyKey,
    bindings: &'a [FootnoteId],
    inner: crate::FootnoteReferences<'a>,
}
impl BodyFootnoteReferences<'_> {
    pub fn advance(
        &mut self,
        budget: &mut Budget,
    ) -> Result<Option<(BodyId<crate::FootnoteReferenceId>, BodyFootnoteReference)>, Completion>
    {
        Ok(self.inner.advance(budget)?.map(|(id, r)| {
            (
                BodyId {
                    body: self.key,
                    local: id,
                },
                BodyFootnoteReference {
                    symbol: self.bindings[r.symbol.0],
                    source: BodySpan {
                        body: self.key,
                        span: r.source,
                    },
                },
            )
        }))
    }
}
impl BodyDisplayCursor {
    pub fn next_with_resolver<'a>(
        &mut self,
        view: BodyView<'a>,
        resolver: &mut crate::LinkResolver,
        budget: &mut Budget,
    ) -> Result<BodyDisplayStep<'a>, LayoutError> {
        view.check(self.key)?;
        self.inner
            .next_with_resolver(&view.body.engine, resolver, budget)
            .map(|step| BodyDisplayStep::from_step(self.key, step))
    }
    pub fn swap_link_resolver(
        &mut self,
        view: BodyView<'_>,
        resolver: &mut crate::LinkResolver,
    ) -> Result<(), LayoutError> {
        view.check(self.key)?;
        self.inner.swap_link_resolver(resolver)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyResolveStep {
    Resolved(Option<crate::FootnoteResolution>),
    BudgetExhausted,
}

fn add_engine_work(total: &mut crate::Work, before: crate::Work, after: crate::Work) {
    total.code_blank_probes += after.code_blank_probes - before.code_blank_probes;
    total.code_blank_bytes += after.code_blank_bytes - before.code_blank_bytes;
    total.code_blank_admissions += after.code_blank_admissions - before.code_blank_admissions;
    total.code_blank_retirements += after.code_blank_retirements - before.code_blank_retirements;
    total.code_blank_decisions += after.code_blank_decisions - before.code_blank_decisions;
    total.inline_undo_peak += after
        .inline_undo_peak
        .saturating_sub(before.inline_undo_peak);
    total.inline_undo_bytes_peak += after
        .inline_undo_bytes_peak
        .saturating_sub(before.inline_undo_bytes_peak);
    total.eligibility_undo_peak += after
        .eligibility_undo_peak
        .saturating_sub(before.eligibility_undo_peak);
    total.eligibility_undo_bytes_peak += after
        .eligibility_undo_bytes_peak
        .saturating_sub(before.eligibility_undo_bytes_peak);
    total.style_events += after.style_events.saturating_sub(before.style_events);
    total.style_scope_steps += after
        .style_scope_steps
        .saturating_sub(before.style_scope_steps);
    total.style_index_steps += after
        .style_index_steps
        .saturating_sub(before.style_index_steps);
    total.style_suffix_bytes += after
        .style_suffix_bytes
        .saturating_sub(before.style_suffix_bytes);
    total.style_suffix_steps += after
        .style_suffix_steps
        .saturating_sub(before.style_suffix_steps);
    total.inline_checkpoint_steps += after
        .inline_checkpoint_steps
        .saturating_sub(before.inline_checkpoint_steps);
    total.inline_rollback_steps += after
        .inline_rollback_steps
        .saturating_sub(before.inline_rollback_steps);
    total.header_effect_steps += after
        .header_effect_steps
        .saturating_sub(before.header_effect_steps);
    total.header_replay_bytes += after
        .header_replay_bytes
        .saturating_sub(before.header_replay_bytes);
    total.paragraph_closures += after
        .paragraph_closures
        .saturating_sub(before.paragraph_closures);
    total.link_label_steps += after
        .link_label_steps
        .saturating_sub(before.link_label_steps);
    total.link_index_steps += after
        .link_index_steps
        .saturating_sub(before.link_index_steps);
    total.link_gate_steps += after.link_gate_steps.saturating_sub(before.link_gate_steps);
    total.input_bytes += after.input_bytes.saturating_sub(before.input_bytes);
    total.classified_bytes += after
        .classified_bytes
        .saturating_sub(before.classified_bytes);
    total.structural_prefix_bytes += after
        .structural_prefix_bytes
        .saturating_sub(before.structural_prefix_bytes);
    total.structural_prefix_steps += after
        .structural_prefix_steps
        .saturating_sub(before.structural_prefix_steps);
    total.fence_candidate_bytes += after
        .fence_candidate_bytes
        .saturating_sub(before.fence_candidate_bytes);
    total.fence_candidate_steps += after
        .fence_candidate_steps
        .saturating_sub(before.fence_candidate_steps);
    total.atx_bytes += after.atx_bytes.saturating_sub(before.atx_bytes);
    total.inline_bytes += after.inline_bytes.saturating_sub(before.inline_bytes);
    total.root_eligibility_bytes += after
        .root_eligibility_bytes
        .saturating_sub(before.root_eligibility_bytes);
    total.root_eligibility_steps += after
        .root_eligibility_steps
        .saturating_sub(before.root_eligibility_steps);
    total.container_eligibility_bytes += after
        .container_eligibility_bytes
        .saturating_sub(before.container_eligibility_bytes);
    total.container_eligibility_steps += after
        .container_eligibility_steps
        .saturating_sub(before.container_eligibility_steps);
    total.link_title_gap_steps += after
        .link_title_gap_steps
        .saturating_sub(before.link_title_gap_steps);
    total.link_fallback_bytes += after
        .link_fallback_bytes
        .saturating_sub(before.link_fallback_bytes);
    total.link_fallback_steps += after
        .link_fallback_steps
        .saturating_sub(before.link_fallback_steps);
    total.delimiter_steps += after.delimiter_steps.saturating_sub(before.delimiter_steps);
    total.code_fallback_bytes += after
        .code_fallback_bytes
        .saturating_sub(before.code_fallback_bytes);
    total.code_fallback_steps += after
        .code_fallback_steps
        .saturating_sub(before.code_fallback_steps);
    total.code_fallback_comparisons += after
        .code_fallback_comparisons
        .saturating_sub(before.code_fallback_comparisons);
    total.code_fallback_eligibility_bytes += after
        .code_fallback_eligibility_bytes
        .saturating_sub(before.code_fallback_eligibility_bytes);
    total.code_fallback_eligibility_steps += after
        .code_fallback_eligibility_steps
        .saturating_sub(before.code_fallback_eligibility_steps);
    total.delimiter_search_steps += after
        .delimiter_search_steps
        .saturating_sub(before.delimiter_search_steps);
    total.flanking_steps += after.flanking_steps.saturating_sub(before.flanking_steps);
    total.label_bytes += after.label_bytes.saturating_sub(before.label_bytes);
    total.label_scalars += after.label_scalars.saturating_sub(before.label_scalars);
    total.label_fold_steps += after
        .label_fold_steps
        .saturating_sub(before.label_fold_steps);
    total.label_key_bytes += after.label_key_bytes.saturating_sub(before.label_key_bytes);
    total.tree_visits += after.tree_visits.saturating_sub(before.tree_visits);
    total.table_bytes += after.table_bytes.saturating_sub(before.table_bytes);
    total.table_header_bytes += after
        .table_header_bytes
        .saturating_sub(before.table_header_bytes);
    total.table_candidate_bytes += after
        .table_candidate_bytes
        .saturating_sub(before.table_candidate_bytes);
    total.autolink_bytes += after.autolink_bytes.saturating_sub(before.autolink_bytes);
    total.bare_autolink_bytes += after
        .bare_autolink_bytes
        .saturating_sub(before.bare_autolink_bytes);
    total.bare_eligibility_bytes += after
        .bare_eligibility_bytes
        .saturating_sub(before.bare_eligibility_bytes);
    total.entity_bytes += after.entity_bytes.saturating_sub(before.entity_bytes);
    total.container_bytes += after.container_bytes.saturating_sub(before.container_bytes);
    total.container_steps += after.container_steps.saturating_sub(before.container_steps);
    total.list_steps += after.list_steps.saturating_sub(before.list_steps);
    total.heading_steps += after.heading_steps.saturating_sub(before.heading_steps);
    total.math_steps += after.math_steps.saturating_sub(before.math_steps);
    total.footnote_steps += after.footnote_steps.saturating_sub(before.footnote_steps);
    total.footnote_label_bytes += after
        .footnote_label_bytes
        .saturating_sub(before.footnote_label_bytes);
}

/// Observed owned capacities. This does not claim allocator overhead or the
/// complete heap of the reused engine's private indexes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BodyStorage {
    pub parser_fixed_bytes: usize,
    pub projection_bytes: usize,
    pub projection_capacity: usize,
    pub mapping_entries: usize,
    pub mapping_capacity_bytes: usize,
    pub binding_entries: usize,
    pub binding_capacity_bytes: usize,
    pub annotation_capacity_bytes: usize,
    pub container_capacity_bytes: usize,
}
impl BodyView<'_> {
    pub fn storage(self) -> BodyStorage {
        BodyStorage {
            parser_fixed_bytes: std::mem::size_of::<Document>(),
            projection_bytes: self.body.engine.source.len(),
            projection_capacity: self.body.engine.source.capacity(),
            mapping_entries: self.body.pieces.len(),
            mapping_capacity_bytes: self.body.pieces.capacity() * std::mem::size_of::<Piece>(),
            binding_entries: self.body.link_symbols.len() + self.body.note_symbols.len(),
            binding_capacity_bytes: self.body.link_symbols.capacity()
                * std::mem::size_of::<crate::SymbolId>()
                + self.body.note_symbols.capacity() * std::mem::size_of::<FootnoteId>(),
            annotation_capacity_bytes: self.body.engine.annotation_arena_stats().capacity_bytes,
            container_capacity_bytes: self.body.engine.container_arena_stats().capacity_bytes,
        }
    }
}
impl Document {
    pub fn footnote_body_count(&self) -> usize {
        self.bodies.entries.len()
    }
}

/// A body occurrence points to the canonical host symbol; its source remains
/// in the explicitly qualified body projection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BodyFootnoteReference {
    pub symbol: FootnoteId,
    pub source: BodySpan,
}
